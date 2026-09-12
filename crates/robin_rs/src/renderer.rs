//! GPU renderer built on wgpu.
//!
//! The level background is a persistent GPU texture. Sprites, UI elements,
//! patch-effect background decals, and overlays render on top via a
//! textured-quad pipeline with selectable blend modes.
//!
//! Decompressed sprite frames are cached as GPU textures keyed by
//! `(bank_id, variant, shadow_color)` so that unchanged frames skip
//! decompression entirely on subsequent renders.
//!
//! Legacy surface ids point at uploaded GPU textures plus hit masks;
//!   runtime drawing is queued as GPU quads and submitted in `present()`.
//! - Upscale shaders run as native WGSL pipelines (see [`crate::gpu_upscale`]).

use robin_engine::graphic_config::TextureScaleMode;
use robin_engine::sprite::BBox;
use std::collections::HashMap;

use robin_assets::frame_holder::{FrameHolder, SHADOW_KEY, SpriteVariant};

use crate::font::TrueTypeFont;
use crate::gfx_types::{BlendMode, Color, Rect};
use crate::presentation::{
    PresentationFrameId, ZoomPresentation, ZoomPresentationState, ZoomPresentationUnavailable,
    ZoomPresentationUpdate,
};
use crate::ui::AlphaMask;
use crate::window::{GpuContext, SharedSurface};
use crate::zoom_hud::ZoomTooltipTracker;

mod atlas;
mod frame;
mod pipelines;
mod readback;
mod resources;

use atlas::AtlasSlot;
use frame::FrameState;
use pipelines::PipelineStore;
pub use readback::{CaptureError, CapturedFrame, PendingCapture};
use resources::GpuResources;

/// Borrowed identity minted by one renderer. Deserialization never restores authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SurfaceHandle {
    id: u32,
    #[serde(skip)]
    renderer: u64,
}

impl SurfaceHandle {
    pub fn legacy_id(self) -> u32 {
        self.id
    }
}

/// Screen targets are not uploads and can never be adopted or retired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SurfaceTarget {
    Screen,
    Upload(SurfaceHandle),
}

/// Unique retirement authority. Borrowed draw handles cannot delete an upload.
/// Explicit retirement requires the originating renderer; Drop does not touch GPU state.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct OwnedSurface {
    handle: SurfaceHandle,
}

impl OwnedSurface {
    pub fn handle(&self) -> SurfaceHandle {
        self.handle
    }

    #[cfg(test)]
    pub(crate) fn synthetic(id: u32) -> Self {
        Self {
            handle: SurfaceHandle { id, renderer: 0 },
        }
    }
}

static NEXT_RENDERER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Debug, thiserror::Error, serde::Serialize, serde::Deserialize)]
#[error("renderer surface {0:?} is missing or has been deleted")]
pub struct MissingSurface(pub SurfaceHandle);

#[derive(Debug, thiserror::Error, serde::Serialize, serde::Deserialize)]
pub enum SurfaceOwnershipError {
    #[error(transparent)]
    Missing(#[from] MissingSurface),
    #[error("surface {0} cannot have multiple owners")]
    AlreadyOwned(u32),
    #[error("surface {0} has no retirement owner")]
    NotOwned(u32),
}

/// Session-scoped atlas residency. Entries include outlines and baked shadow
/// variants; distinct frames count only bank/variant identity. No entry-only
/// eviction is offered because it would not release atlas layer memory.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct SpriteResidencyStats {
    pub resident_bytes: u64,
    pub layers: usize,
    pub entries: usize,
    pub distinct_frames: usize,
    pub occupied_texels: u64,
    pub committed_texels: u64,
}

// ---------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------

pub const BLIT_SOURCE_TRANSPARENT: u32 = 0x01;
pub const TRANSPARENT_COLOR_KEY_16: u16 = 0x07C0;
pub const TRANSPARENT_COLOR_KEY_15: u16 = 0x03E0;
pub const OUTLINE_PAD: usize = 2;
const OUTLINE_CACHE_TAG: u32 = 0x0001_0000;

pub use robin_util::color::rgb565_to_rgb8;

// ---------------------------------------------------------------------
// Sprite/texture caches — wgpu::Texture-backed.
// ---------------------------------------------------------------------

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
struct SpriteCacheKey {
    bank_id: u32,
    variant: SpriteVariant,
    shadow_color: u32,
    shadow_alpha: u8,
}

/// One decoded sprite frame's GPU residency: a sub-rect of a shared
/// [`atlas`] layer.
///
/// This was briefly an enum with a `Legacy` one-texture-per-sprite arm
/// behind `ROBIN_SPRITE_ATLAS=0`, so a single binary could render both
/// halves of an A/B comparison with data, build, driver and scene held
/// fixed. That comparison is done — eight full-map captures across three
/// missions and two datadirs came back byte-identical — so the arm and
/// its flag are gone.
struct SpriteResidency(AtlasSlot);

impl SpriteResidency {
    fn dimensions(&self) -> (u16, u16) {
        (self.0.width, self.0.height)
    }
}

#[derive(Default)]
struct SpriteTextureCache {
    entries: HashMap<SpriteCacheKey, SpriteResidency>,
}

#[inline]
fn outline_cache_key(bank_id: u32, variant: SpriteVariant, shadow_color: u16) -> SpriteCacheKey {
    SpriteCacheKey {
        bank_id,
        variant,
        shadow_color: OUTLINE_CACHE_TAG | shadow_color as u32,
        shadow_alpha: 0,
    }
}

/// Per-mask static GPU state. The binary bitmap is uploaded once as R8 and
/// rasterized into the stencil buffer immediately before a masked sprite.
struct MaskAlpha {
    /// Held alive for the bind group's lifetime.
    _texture: wgpu::Texture,
    _view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
    atlas: Option<MaskAtlasBounds>,
}

/// Mask-only vertex metadata, shared with sprite_mask_stencil.wgsl. Each axis
/// packs origin + size * 4096 into an integer below 2^24, exactly representable
/// in f32. Pages are at most 2048 square. Tint blue/alpha are unused by the
/// standalone binary, depth and stencil-clear paths; negative green selects
/// this encoding without adding attributes to every sprite vertex.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
struct MaskAtlasBounds {
    origin: [u32; 2],
    size: [u32; 2],
}

impl MaskAtlasBounds {
    fn tint(self) -> [f32; 4] {
        assert!(self.origin.into_iter().all(|v| v < 2048));
        assert!(self.size.into_iter().all(|v| v > 0 && v <= 2048));
        [
            0.5,
            -1.0,
            (self.origin[0] + self.size[0] * 4096) as f32,
            (self.origin[1] + self.size[1] * 4096) as f32,
        ]
    }
}

/// `MaskIndex` is a `NonMaxU32`, so this cannot collide with a legacy mask.
const OCCLUSION_DEPTH_TEXTURE_INDEX: u32 = u32::MAX;

struct BackgroundTexture {
    _view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

struct ManagedSurface {
    width: u16,
    height: u16,
    pixels: ManagedSurfacePixels,
    alpha_mask: AlphaMask,
    shadow_alpha: u8,
}

enum ManagedSurfacePixels {
    Pending(Box<[u16]>),
    Resident(ManagedSurfaceTextures),
}

struct ManagedSurfaceTextures {
    _opaque_texture: wgpu::Texture,
    _opaque_view: wgpu::TextureView,
    opaque_bg: wgpu::BindGroup,
    _color_texture: wgpu::Texture,
    _color_view: wgpu::TextureView,
    color_bg: wgpu::BindGroup,
    _shadow_texture: Option<wgpu::Texture>,
    _shadow_view: Option<wgpu::TextureView>,
    shadow_bg: Option<wgpu::BindGroup>,
}

impl ManagedSurface {
    fn textures(&self) -> &ManagedSurfaceTextures {
        match &self.pixels {
            ManagedSurfacePixels::Resident(textures) => textures,
            ManagedSurfacePixels::Pending(_) => {
                panic!("managed surface must be uploaded before drawing")
            }
        }
    }
}

/// Persistent GPU texture for decoded RGB565 assets that never need the
/// managed-surface compatibility API.
pub struct GpuImage {
    _texture: wgpu::Texture,
    _view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    width: u16,
    height: u16,
}

// ---------------------------------------------------------------------
// Per-frame draw queue — accumulated overlays drawn in `present()`.
// ---------------------------------------------------------------------

/// One queued overlay draw. All draws go through the same textured-quad
/// pipeline; solid-color draws use a 1×1 white texture and rely on the
/// `tint` to colourize.
#[derive(Clone)]
struct QueuedDraw {
    /// Pixel-space destination rectangle (top-left origin, +y down).
    /// Ignored when `corners` is `Some(_)`.
    dst: Rect,
    /// Optional explicit four-corner positions (TL, TR, BL, BR) in
    /// pixel space — used by `render_gpu_line` (rotated thin quad
    /// for diagonal lines) and `render_gpu_triangle` (degenerate
    /// quad with `BR == BL`). When `None`, vertices are derived
    /// from `dst`.
    corners: Option<[(f32, f32); 4]>,
    /// `(u0, v0, u1, v1)` in 0..1 source-texture coords. Solid-color
    /// draws use the full white texture so the values are `(0,0,1,1)`.
    uv: [f32; 4],
    /// RGBA in linear 0..1, multiplied with the sampled texel.
    /// `DrawOperation::ColorizeFrozen` repurposes this as
    /// `(hue/360, scale, _, _)` — the `fs_colorize` shader in
    /// `shaders/quad.wgsl` reads it.
    tint: [f32; 4],
    /// Operation owns its valid texture/blend combination. The remaining
    /// fields are already packed shader inputs, not application preferences.
    operation: DrawOperation,
}

/// Actual texture identity, independent of pipeline selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextureSource {
    White,
    FrozenScene,
    Framebuffer,
    Frame(u32),
    MaskAlpha(u32),
    LoadingDissolve(u32),
}

/// Texture sources accepted by the ordinary textured-quad layout.
#[derive(Clone, Copy)]
enum QuadTexture {
    White,
    FrozenScene,
    Frame(u32),
}

#[derive(Clone, Copy)]
enum DrawOperation {
    Quad {
        texture: QuadTexture,
        blend: BlendMode,
    },
    Masked {
        texture: u32,
        blend: BlendMode,
    },
    ColorizeFrozen,
    FramebufferAlpha,
    ViewConeGradient,
    MaskAlpha(u32),
    StencilClear,
    LoadingDissolve(u32),
}

impl DrawOperation {
    fn texture(self) -> TextureSource {
        match self {
            Self::Quad { texture, .. } => match texture {
                QuadTexture::White => TextureSource::White,
                QuadTexture::FrozenScene => TextureSource::FrozenScene,
                QuadTexture::Frame(index) => TextureSource::Frame(index),
            },
            Self::Masked { texture, .. } => TextureSource::Frame(texture),
            Self::ColorizeFrozen => TextureSource::FrozenScene,
            Self::FramebufferAlpha => TextureSource::Framebuffer,
            Self::ViewConeGradient | Self::StencilClear => TextureSource::White,
            Self::MaskAlpha(index) => TextureSource::MaskAlpha(index),
            Self::LoadingDissolve(index) => TextureSource::LoadingDissolve(index),
        }
    }
}

/// Vertex layout consumed by `shaders/quad.wgsl`. 32 bytes per vertex.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct QuadVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    tint: [f32; 4],
}

/// Screen-size uniform consumed by `shaders/quad.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ScreenUniform {
    screen_size: [f32; 2],
    _pad: [f32; 2],
}

// ---------------------------------------------------------------------
// Renderer
// ---------------------------------------------------------------------

/// wgpu-backed renderer. Owns everything needed to draw a frame:
/// uploaded legacy resource surfaces, GPU sprite cache, the swapchain
/// config, and the upscale pipelines.
///
/// All resources are `Arc`-shared internally; `Renderer` itself owns
/// no borrows.
pub struct Renderer {
    identity: u64,
    owned_surfaces: std::collections::HashSet<u32>,
    /// Shared GPU context (device/queue/surface format).
    pub(crate) gpu: GpuContext,
    resources: GpuResources,
    pipelines: PipelineStore,
    frame: FrameState,
    // Keep the optional second frame out of frontend construction futures.
    // Its allocation is reused for both the held live frame and cached target.
    capture_frame: Option<Box<FrameState>>,
    screen_layout: wgpu::BindGroupLayout,
    /// Update-owned zoom HUD data. Kept separate from GPU ownership because
    /// throwaway screenshot and thumbnail passes must not advance it.
    zoom_presentation: ZoomPresentationState,
    fog_mask: Option<FogMaskTexture>,
}

struct FontAtlas {
    font_lifetime: std::sync::Weak<()>,
    _texture: wgpu::Texture,
    _view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

/// A scoped selection of the renderer's cached capture target, not a second
/// asset-owning renderer. Drop restores the untouched live frame even when a
/// draw/readback returns early (or unwinds).
pub(crate) struct CaptureTarget<'a> {
    renderer: &'a mut Renderer,
    live: Option<Box<FrameState>>,
}

impl serde::Serialize for CaptureTarget<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_unit_struct("CaptureTarget")
    }
}

impl<'de> serde::Deserialize<'de> for CaptureTarget<'_> {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "capture target authority must be borrowed from a renderer",
        ))
    }
}

impl std::ops::Deref for CaptureTarget<'_> {
    type Target = Renderer;
    fn deref(&self) -> &Renderer {
        self.renderer
    }
}

impl std::ops::DerefMut for CaptureTarget<'_> {
    fn deref_mut(&mut self) -> &mut Renderer {
        self.renderer
    }
}

impl Drop for CaptureTarget<'_> {
    fn drop(&mut self) {
        let mut live = self
            .live
            .take()
            .expect("capture scope owns its live target");
        std::mem::swap(&mut self.renderer.frame, &mut *live);
        self.renderer.capture_frame = Some(live);
    }
}

struct FogMaskTexture {
    _texture: wgpu::Texture,
    _view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
    generation: u64,
}

fn make_tex_bg(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    label: &str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn make_alpha_source(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    width: u16,
    height: u16,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("alpha source"),
        size: wgpu::Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = make_tex_bg(device, layout, &view, sampler, "alpha source bg");
    (texture, view, bind_group)
}

fn mark_draws_stencil_tested(draws: &mut [QueuedDraw]) {
    for draw in draws {
        draw.operation = match draw.operation {
            DrawOperation::Quad {
                texture: QuadTexture::Frame(texture),
                blend,
            }
            | DrawOperation::Masked { texture, blend } => DrawOperation::Masked { texture, blend },
            _ => panic!("non-textured draw queued inside a sprite mask region"),
        };
    }
}

impl Renderer {
    /// Build a fresh renderer borrowing GPU resources from `window`.
    /// `gpu` and `surface` are `Arc`-shared so cloning is cheap; the
    /// renderer keeps its own clone for the lifetime of the session.
    pub fn new(
        window: &crate::window::GameWindow,
        width: u16,
        height: u16,
        scale_mode: TextureScaleMode,
    ) -> Self {
        Self::with_gpu(
            window.gpu.clone(),
            window.surface.clone(),
            Some(window.surface_config.clone()),
            width,
            height,
            scale_mode,
        )
    }

    /// Lower-level constructor accepting the wgpu context + surface
    /// directly. Used by callers that don't have a `GameWindow` handy
    /// (the WASM bootstrap, tests).
    pub fn with_gpu(
        gpu: GpuContext,
        surface: SharedSurface,
        surface_config: Option<wgpu::SurfaceConfiguration>,
        width: u16,
        height: u16,
        scale_mode: TextureScaleMode,
    ) -> Self {
        Self::with_optional_surface(
            gpu,
            Some(surface),
            surface_config,
            width,
            height,
            scale_mode,
        )
    }

    /// Render into a GPU texture without creating a window or swapchain.
    /// Use try_capture_frame_rgba to read the result.
    pub fn offscreen(gpu: GpuContext, width: u16, height: u16) -> Self {
        assert!(
            width > 0 && height > 0,
            "offscreen dimensions must be positive"
        );
        Self::with_optional_surface(gpu, None, None, width, height, TextureScaleMode::Nearest)
    }

    fn with_optional_surface(
        gpu: GpuContext,
        surface: Option<SharedSurface>,
        surface_config: Option<wgpu::SurfaceConfiguration>,
        width: u16,
        height: u16,
        scale_mode: TextureScaleMode,
    ) -> Self {
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("quad sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let linear_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fog mask linear sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        // 1×1 white texture used for solid-color draws (so the same
        // pipeline handles textured + solid).
        let white_texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("white 1x1"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &white_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255u8, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let white_view = white_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bgl_screen = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("quad screen bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let bgl_tex = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("quad tex bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let screen_uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad screen uniform"),
            size: std::mem::size_of::<ScreenUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let initial_screen = ScreenUniform {
            screen_size: [width as f32, height as f32],
            _pad: [0.0; 2],
        };
        gpu.queue
            .write_buffer(&screen_uniform, 0, bytemuck::bytes_of(&initial_screen));
        let screen_bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quad screen bg"),
            layout: &bgl_screen,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen_uniform.as_entire_binding(),
            }],
        });

        let swap_screen_uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("swap screen uniform"),
            size: std::mem::size_of::<ScreenUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let swap_screen_bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("swap screen bg"),
            layout: &bgl_screen,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: swap_screen_uniform.as_entire_binding(),
            }],
        });

        let white_bg = make_tex_bg(&gpu.device, &bgl_tex, &white_view, &sampler, "white bg");
        let resources = GpuResources::new(sampler, linear_sampler, bgl_tex, white_view, white_bg);
        let pipelines = PipelineStore::new(&gpu, &bgl_screen, &resources.bgl_tex, scale_mode);
        let frame = FrameState::new(
            &gpu,
            &resources,
            screen_uniform,
            screen_bg,
            swap_screen_uniform,
            swap_screen_bg,
            surface,
            surface_config,
            width,
            height,
        );

        Renderer {
            identity: NEXT_RENDERER_ID
                .try_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |id| id.checked_add(1),
                )
                .expect("renderer identity exhausted"),
            owned_surfaces: Default::default(),
            gpu,
            resources,
            pipelines,
            frame,
            capture_frame: None,
            screen_layout: bgl_screen,
            zoom_presentation: ZoomPresentationState::default(),
            fog_mask: None,
        }
    }

    /// Retain compiled pipelines while releasing loading-screen frame state.
    pub(crate) fn finish_loading_screen(&mut self) {
        self.frame.finish_loading_screen(&self.gpu);
        self.pipelines.gpu_upscale.finish_loading_screen();
        self.clear_font_atlas_cache();
    }

    /// Select an independent reusable logical target while sharing uploaded
    /// assets and pipelines. The held live frame (including pending draws and
    /// modal snapshot) is never resized, flushed or otherwise modified.
    pub(crate) fn capture_target(&mut self, width: u16, height: u16) -> CaptureTarget<'_> {
        assert!(
            width > 0 && height > 0,
            "capture dimensions must be positive"
        );
        let mut frame = self.capture_frame.take().unwrap_or_else(|| {
            Box::new(FrameState::offscreen(
                &self.gpu,
                &self.resources,
                &self.screen_layout,
                width,
                height,
            ))
        });
        frame.resize(&self.gpu, &self.resources, width, height);
        frame.clear_frozen_scene();
        frame.clear_recording();
        std::mem::swap(&mut self.frame, &mut *frame);
        CaptureTarget {
            renderer: self,
            live: Some(frame),
        }
    }

    // ----- accessors that stayed compatible -----

    pub fn screen_width(&self) -> u16 {
        self.frame.dimensions().0
    }

    pub fn screen_height(&self) -> u16 {
        self.frame.dimensions().1
    }

    pub fn transparent_color(&self) -> u16 {
        if self.resources.bit_depth == 15 {
            TRANSPARENT_COLOR_KEY_15
        } else {
            TRANSPARENT_COLOR_KEY_16
        }
    }

    pub fn scale_mode(&self) -> TextureScaleMode {
        self.pipelines.scale_mode()
    }

    pub fn set_scale_mode(&mut self, mode: TextureScaleMode) {
        self.pipelines.set_scale_mode(mode);
    }

    pub fn set_shader_preset(&mut self, preset: impl Into<String>) {
        self.pipelines.set_shader_preset(preset);
    }

    /// Apply the complete persisted upscaler/effect configuration atomically.
    /// This avoids one-frame combinations of old parameters with a new mode
    /// while the Graphics screen is being accepted.
    pub fn apply_upscale_config(&mut self, config: &robin_engine::graphic_config::GraphicConfig) {
        self.pipelines.apply_upscale_config(config);
    }

    pub fn validate_retroarch_preset(
        &mut self,
        preset: &str,
    ) -> Result<(), crate::gpu_upscale::UpscaleError> {
        self.pipelines.validate_retroarch_preset(preset)
    }

    pub(crate) fn update_zoom_presentation(
        &mut self,
        frame_id: PresentationFrameId,
        input: ZoomPresentationUpdate,
        tooltip_tracker: &mut ZoomTooltipTracker,
    ) {
        self.zoom_presentation
            .update(frame_id, input, tooltip_tracker);
    }

    pub(crate) fn zoom_presentation(
        &self,
        frame_id: PresentationFrameId,
    ) -> Result<&ZoomPresentation, ZoomPresentationUnavailable> {
        self.zoom_presentation.presentation(frame_id)
    }

    pub fn is_gpu_phase(&self) -> bool {
        self.frame.is_gpu_phase()
    }

    pub fn create_color_16(r: u8, g: u8, b: u8) -> u16 {
        robin_util::color::rgb565(r, g, b)
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn assert_legacy_adoption_rejected(&mut self, handle: SurfaceHandle) {
        assert!(self.try_adopt_surface(handle.id).is_err());
    }

    /// Enter the GPU overlay phase. The old renderer uploaded CPU
    /// framebuffer pixels here; the current renderer draws the level
    /// background and all overlays as GPU quads.
    pub fn flush_base_layer(&mut self) {
        self.frame.enter_gpu_phase();
    }

    /// Mark subsequent draws as screen-space UI. Presentation effects are
    /// applied to the preceding world/video layer; this layer is composited
    /// afterwards through a sharp fractional-scale filter.
    pub fn begin_ui_layer(&mut self) {
        self.frame.begin_ui_layer();
    }

    /// Present this whole logical frame with the sharp UI scaler, bypassing
    /// gameplay upscalers and post-effects. Top-level menus use this instead
    /// of splitting a transparent UI layer so their rendered frame remains
    /// available to the modal-freeze path.
    pub fn begin_ui_only_frame(&mut self) {
        self.frame.begin_ui_only_frame();
    }

    /// Snapshot the offscreen render target into a held texture so a
    /// modal menu can overlay dim/tint + widgets on top of the previous
    /// gameplay frame. Idempotent — subsequent calls while a freeze is
    /// already held are no-ops, so menu render paths can call this on
    /// every frame they're up. `clear_frozen_scene` drops the snapshot
    /// when the gameplay path resumes.
    ///
    /// Unlike the old `flush_base_layer` snapshot, this captures every GPU
    /// sprite drawn over the software background layer.
    pub fn freeze_scene_for_modal(&mut self) {
        self.frame.freeze_scene(&self.gpu, &self.resources);
    }

    /// Drop the modal-snapshot texture. Called by the gameplay render
    /// path on every frame so the snapshot is alive only while a modal
    /// is up.
    pub fn clear_frozen_scene(&mut self) {
        self.frame.clear_frozen_scene();
    }

    /// Submit all queued draws and present to the swapchain.
    ///
    /// Order: clear → optional frozen-scene quad → all `queued` draws
    /// in submission order, switching pipeline per blend-mode and
    /// rebinding the texture group per texture source.
    pub fn present(&mut self) {
        let _ = self.try_present();
    }

    /// True only when a swapchain texture was acquired and submitted.
    /// This is not a physical display presentation timestamp.
    pub fn try_present(&mut self) -> bool {
        self.frame
            .present(&self.gpu, &mut self.pipelines, &self.resources)
    }

    /// Re-present the last completed logical frame. Unlike [`Self::present`],
    /// this performs no pass-1 composition and therefore cannot repeat game,
    /// UI, tooltip, fade, cursor, or post-render side effects.
    pub fn present_cached(&mut self) -> bool {
        self.frame
            .present_cached(&self.gpu, &mut self.pipelines, &self.resources)
    }

    pub fn configure_surface_size(&mut self, width: u32, height: u32) {
        self.frame.configure_surface_size(&self.gpu, width, height);
    }

    /// Synchronize both presentation and logical render-target dimensions
    /// with a [`GameWindow`](crate::window::GameWindow).
    ///
    /// `GameWindow` owns the profile-backed aspect policy and transforms raw
    /// pointer input. Keeping this operation here ensures the renderer's
    /// swapchain bookkeeping and offscreen target change as one unit.
    /// Returns `true` when the logical canvas was resized.
    pub fn sync_window_size(&mut self, window: &crate::window::GameWindow) -> bool {
        let (surface_width, surface_height) = window.surface_size();
        self.configure_surface_size(surface_width, surface_height);
        let (logical_width, logical_height) = window.logical_size();
        let logical_width = logical_width.min(u32::from(u16::MAX)) as u16;
        let logical_height = logical_height.min(u32::from(u16::MAX)) as u16;
        let changed =
            self.screen_width() != logical_width || self.screen_height() != logical_height;
        if changed {
            self.resize(logical_width, logical_height);
        }
        changed
    }

    pub fn configure_native_refresh_presentation(
        &mut self,
        enabled: bool,
        surface_width: u32,
        surface_height: u32,
    ) {
        self.frame
            .configure_present_mode(&self.gpu, enabled, surface_width, surface_height);
    }

    /// Cross the flush boundary and present in one shot. Loading/menu screens
    /// queue their own GPU draws before calling this.
    pub fn flip(&mut self) {
        self.flush_base_layer();
        self.present();
    }

    /// Resize the renderer's logical canvas. Pure presentation-size changes
    /// only reconfigure the swapchain, while an aspect-policy change reaches
    /// this through [`Self::sync_window_size`]. Graphics preset changes may
    /// also call it indirectly through that synchronization path.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.frame.resize(&self.gpu, &self.resources, width, height);
    }

    pub fn upload_background_texture(&mut self, width: u32, height: u32, pixels: &[u16]) -> bool {
        self.resources
            .upload_background_texture(&self.gpu, width, height, pixels)
    }

    pub fn render_background_texture(
        &mut self,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
    ) -> bool {
        let Some(bg) = self.resources.background_texture.as_ref() else {
            return false;
        };
        let bg_bind_group = bg.bind_group.clone();
        let (dst, uv) = src_dst_uv(src_rect, dst_rect, bg.width as f32, bg.height as f32);
        let tex_idx = self.queue_cached_bg(bg_bind_group);
        self.frame.queued.push(QueuedDraw {
            dst,
            corners: None,
            uv,
            tint: [1.0, 1.0, 1.0, 1.0],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend: BlendMode::Blend,
            },
        });
        true
    }

    /// Read access to the wgpu context — needed by callers that want
    /// to allocate / write textures directly (titbit_renderer keeps
    /// its frame textures across game frames).
    pub fn gpu(&self) -> &GpuContext {
        &self.gpu
    }

    /// On-demand diagnostic; scans cache keys, so sample at reporting cadence.
    pub fn sprite_residency_stats(&self) -> SpriteResidencyStats {
        self.resources.sprite_residency_stats()
    }

    /// Synchronous capture for native tools and GPU tests. Runtime callers use
    /// owned asynchronous readbacks so completion never blocks the session loop.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn try_capture_frame_rgba(&mut self) -> Result<CapturedFrame, CaptureError> {
        readback::capture_frame_rgba(&self.gpu, &self.pipelines, &self.resources, &mut self.frame)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn try_capture_presented_frame_rgba(&self) -> Result<CapturedFrame, CaptureError> {
        readback::capture_presented_frame_rgba(&self.gpu, &self.frame)
    }

    /// Submit a snapshot now; the owned completion can run after the renderer
    /// has moved on to another frame without reading that later frame.
    pub fn begin_capture_frame_rgba(&mut self) -> PendingCapture {
        readback::begin_capture_frame_rgba(
            &self.gpu,
            &self.pipelines,
            &self.resources,
            &mut self.frame,
        )
    }

    /// Submit the already-presented logical target without borrowing it while mapping.
    pub fn begin_capture_presented_frame_rgba(&self) -> PendingCapture {
        readback::begin_capture_presented_frame_rgba(&self.gpu, &self.frame)
    }

    pub async fn capture_frame_rgba_async(&mut self) -> Result<CapturedFrame, CaptureError> {
        readback::capture_frame_rgba_async(
            &self.gpu,
            &self.pipelines,
            &self.resources,
            &mut self.frame,
        )
        .await
    }

    pub async fn capture_presented_frame_rgba_async(&self) -> Result<CapturedFrame, CaptureError> {
        readback::capture_presented_frame_rgba_async(&self.gpu, &self.frame).await
    }
}

// ---------------------------------------------------------------------
// Per-frame texture / cache helpers
// ---------------------------------------------------------------------

impl Renderer {
    /// Bind a one-shot texture view as a per-frame bind group, return
    /// its index into `frame_texture_bgs`. The view is captured by
    /// the bind group; `present()` clears the vec at frame end so
    /// views with a shorter lifetime than the renderer (one-shot
    /// uploads) are safe to use as long as the same call site queues
    /// + presents in one frame.
    fn queue_frame_texture(&mut self, view: &wgpu::TextureView) -> u32 {
        self.frame
            .queue_frame_texture(&self.gpu, &self.resources, view)
    }

    /// Reuse an already-built bind group (sprite cache, mask cache,
    /// managed-surface cache). `wgpu::BindGroup` is Arc-internal so
    /// `.clone()` is cheap — much cheaper than rebuilding the bg
    /// every frame inside `queue_frame_texture`.
    fn queue_cached_bg(&mut self, bg: wgpu::BindGroup) -> u32 {
        self.frame.queue_cached_bg(bg)
    }

    /// Resolve a cached sprite to `(per-frame bind-group index, uv)`,
    /// or `None` when it was never cached: the layer's shared bind group
    /// and the sprite's sub-rect within it.
    fn queue_sprite_texture(&mut self, key: &SpriteCacheKey) -> Option<(u32, [f32; 4])> {
        // Copy the slot out first so the immutable borrow of
        // `self.resources` ends before the `&mut self` queue call.
        let slot = self.resources.sprite_cache.entries.get(key)?.0;
        Some((self.queue_atlas_layer(slot.layer), slot.uv))
    }

    /// Resolve an atlas layer to a per-frame bind-group index,
    /// memoized for the frame.
    ///
    /// The memo is what turns the atlas into actual batching: the
    /// draw encoder elides `set_bind_group` when consecutive draws
    /// carry the same `TextureSource::Frame(idx)`, so every sprite from a
    /// layer has to resolve to *one* index rather than a fresh one per
    /// draw.
    fn queue_atlas_layer(&mut self, layer: u32) -> u32 {
        if let Some(idx) = self.frame.atlas_bg_slot(layer) {
            return idx;
        }
        let bg = self.resources.sprite_atlas.bind_group(layer).clone();
        let idx = self.frame.queue_cached_bg(bg);
        self.frame.remember_atlas_bg_slot(layer, idx);
        idx
    }
}

/// Shadow opacity for `FrameHolder::global_shadow()` = 40, gamma-compensated
/// for blending into the wgpu sRGB render target.
///
/// The reference path darkens 16-bit RGB values directly.  A literal 40%
/// black alpha in wgpu blends in linear light and looks too bright, so we
/// bake the alpha that makes white land at 60% sRGB after linear blending.
pub const DEFAULT_SHADOW_ALPHA: u8 = 174;

/// Shadow opacity for the menu-button 50% intensity variant, using the same
/// sRGB-compensated alpha as [`DEFAULT_SHADOW_ALPHA`].
pub const MENU_BUTTON_SHADOW_ALPHA: u8 = 200;

#[inline]
fn shadow_alpha_from_level(shadow_level: u16) -> u8 {
    let retain_srgb = (100 - shadow_level.min(100)) as f32 / 100.0;
    let retain_linear = srgb_to_linear(retain_srgb);
    ((1.0 - retain_linear) * 255.0).round() as u8
}

#[inline]
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// RGB565 → RGBA8, baking the two reserved colour keys:
/// - `color_key` (= `TRANSPARENT_COLOR_KEY_16` = 0x07C0, pure green)
///   → alpha = 0
/// - `SHADOW_KEY` (= 0x001F, pure blue) → black at `shadow_alpha` so
///   a `BlendMode::Blend` draw multiplies the destination by
///   `(1 - shadow/255)`, matching the MMX shadow blit
/// - `shadow_color` (when present) receives the same treatment. Sprite
///   decompression applies ArnoLaw before upload and rewrites shadow
///   pixels from `SHADOW_KEY` to the current ambience colour.
fn rgb565_to_rgba_with_key(
    src: &[u16],
    w: usize,
    h: usize,
    color_key: u16,
    shadow_alpha: u8,
    shadow_color: Option<u16>,
) -> Vec<u8> {
    let n = w * h;
    // Pre-allocate the full Vec once, then write 4 bytes (one u32) per
    // pixel via a bytemuck cast. ~5× faster than the
    // `extend_from_slice` per pixel form, especially in debug builds.
    let mut out = vec![0u8; n * 4];
    let out_u32: &mut [u32] = bytemuck::cast_slice_mut(&mut out);
    let shadow_pixel = (shadow_alpha as u32) << 24;
    for (dst, &px) in out_u32.iter_mut().zip(&src[..n]) {
        *dst = if px == color_key {
            0
        } else if px == SHADOW_KEY || shadow_color == Some(px) {
            shadow_pixel
        } else {
            // RGB565 → 8-bit per channel, packed into wgpu RGBA u32
            // little-endian byte order = [R, G, B, A].
            let (r, g, b) = rgb565_to_rgb8(px);
            u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16) | 0xFF00_0000
        };
    }
    out
}

fn sprite_rgba_for_upload(
    frame_holder: &FrameHolder,
    bank_id: u32,
    variant: SpriteVariant,
    shadow_color: u16,
    shadow_alpha: u8,
    bit_depth: u16,
) -> std::borrow::Cow<'_, [u8]> {
    if let Some(rgba) = frame_holder.rgba_data(bank_id) {
        return std::borrow::Cow::Borrowed(rgba);
    }

    let w = frame_holder.sprite_width(bank_id);
    let h = frame_holder.sprite_height(bank_id);
    let mut rgb565 = vec![TRANSPARENT_COLOR_KEY_16; w as usize * h as usize];
    frame_holder.uncompress_frame(
        &mut rgb565,
        w as usize,
        bank_id,
        variant,
        shadow_color,
        bit_depth,
    );
    std::borrow::Cow::Owned(rgb565_to_rgba_with_key(
        &rgb565,
        w as usize,
        h as usize,
        TRANSPARENT_COLOR_KEY_16,
        shadow_alpha,
        Some(shadow_color),
    ))
}

/// Build the outside-edge outline texture: transparent surface, two
/// coloured pixels outside each horizontal opaque run. Shadow pixels are
/// excluded from the body edge during edge-map generation.
fn sprite_outline_rgba(
    src: &[u16],
    w: usize,
    h: usize,
    out_w: usize,
    color_key: u16,
    shadow_color: u16,
) -> Vec<u8> {
    let n = w * h;
    assert!(
        src.len() >= n,
        "sprite_outline_rgba source too small: {} < {}",
        src.len(),
        n
    );
    assert!(
        out_w >= w + OUTLINE_PAD * 2,
        "sprite outline target width {out_w} cannot hold {w}px sprite plus padding"
    );

    let mut out = vec![0u8; out_w * h * 4];
    let out_u32: &mut [u32] = bytemuck::cast_slice_mut(&mut out);
    let outline_pixel = 0xFFFF_FFFFu32;

    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        let dst_row = &mut out_u32[y * out_w..(y + 1) * out_w];
        let mut inside = false;
        for (x, &px) in row.iter().enumerate() {
            let solid = px != color_key && px != SHADOW_KEY && px != shadow_color;
            if !inside && solid {
                // Entering a solid run. The reference algorithm writes
                // `pos - 2` and `pos - 1` into a surface shifted right
                // by thickness.
                for dx in 0..OUTLINE_PAD {
                    dst_row[x + dx] = outline_pixel;
                }
                inside = true;
            } else if inside && !solid {
                // Leaving a solid run. The reference stores `x - 1` as
                // the edge position, then writes `edge + 1` and
                // `edge + 2`.
                for dx in 0..OUTLINE_PAD {
                    dst_row[x + OUTLINE_PAD + dx] = outline_pixel;
                }
                inside = false;
            }
        }
        if inside {
            for dx in 0..OUTLINE_PAD {
                dst_row[w + OUTLINE_PAD + dx] = outline_pixel;
            }
        }
    }

    out
}

/// RGB565 → RGBA8 with no green-key handling — every pixel opaque
/// except `SHADOW_KEY`. Used by `build_managed_surface`,
/// `create_rgb565_gpu_image` and `upload_background_texture`, where
/// the caller wants the literal source contents and green pixels (if
/// any) should appear as green, not as transparent gaps.
fn rgb565_to_rgba_opaque(src: &[u16], w: usize, h: usize) -> Vec<u8> {
    let n = w * h;
    let mut out = vec![0u8; n * 4];
    let out_u32: &mut [u32] = bytemuck::cast_slice_mut(&mut out);
    let shadow_pixel = (DEFAULT_SHADOW_ALPHA as u32) << 24;
    for (dst, &px) in out_u32.iter_mut().zip(&src[..n]) {
        *dst = if px == SHADOW_KEY {
            shadow_pixel
        } else {
            let (r, g, b) = rgb565_to_rgb8(px);
            u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16) | 0xFF00_0000
        };
    }
    out
}

fn rgb565_to_color_shadow_rgba(src: &[u16], color_key: u16) -> (Vec<u8>, Option<Vec<u8>>) {
    let byte_len = src
        .len()
        .checked_mul(4)
        .expect("managed surface RGBA size overflow");
    let mut color = Vec::with_capacity(byte_len);
    let mut shadow: Option<Vec<u8>> = None;
    for (index, &px) in src.iter().enumerate() {
        if px == color_key {
            color.extend_from_slice(&[0, 0, 0, 0]);
        } else if px == SHADOW_KEY {
            color.extend_from_slice(&[0, 0, 0, 0]);
            // Allocate only on the first shadow pixel. All other mask bytes
            // remain transparent, including the prefix already visited.
            let mask = shadow.get_or_insert_with(|| vec![0; byte_len]);
            mask[index * 4 + 3] = 255;
        } else {
            let (r, g, b) = rgb565_to_rgb8(px);
            color.extend_from_slice(&[r, g, b, 255]);
        }
    }
    (color, shadow)
}

/// Allocate a `Rgba8UnormSrgb` 2D texture and upload `rgba` into it.
fn upload_rgba_texture(
    gpu: &GpuContext,
    uploads: &diagnostics::UploadCounters,
    rgba: &[u8],
    width: u32,
    height: u32,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    uploads.inc(label);
    let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Resolve the dst+src rect pair from a textured draw's optional
/// arguments. `src_rect=None` means full source; `dst_rect=None` means
/// the source is positioned at `(0,0)` on the screen.
fn src_dst_uv(
    src_rect: Option<&BBox>,
    dst_rect: Option<&BBox>,
    src_w: f32,
    src_h: f32,
) -> (Rect, [f32; 4]) {
    let (sx, sy, sw, sh) = match src_rect {
        Some(r) => (
            r.min.x,
            r.min.y,
            (r.max.x - r.min.x).max(0.0),
            (r.max.y - r.min.y).max(0.0),
        ),
        None => (0.0, 0.0, src_w, src_h),
    };
    let dst = match dst_rect {
        Some(r) => Rect {
            x: r.min.x as i32,
            y: r.min.y as i32,
            w: (r.max.x - r.min.x) as i32,
            h: (r.max.y - r.min.y) as i32,
        },
        None => Rect {
            x: 0,
            y: 0,
            w: sw as i32,
            h: sh as i32,
        },
    };
    let uv = [sx / src_w, sy / src_h, (sx + sw) / src_w, (sy + sh) / src_h];
    (dst, uv)
}

fn decode_rgb565_pixels(width: u16, height: u16, bytes: &[u8]) -> Option<Vec<u16>> {
    let expected = usize::from(width)
        .checked_mul(usize::from(height))?
        .checked_mul(2)?;
    if expected == 0 || bytes.len() != expected {
        return None;
    }
    Some(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pixel| u16::from_le_bytes(*pixel))
            .collect(),
    )
}

/// Clip `dst` against `clip` and compute the matching uv sub-rect
/// (assuming the original uv is `[0,0,1,1]` over the full `dst`).
/// Returns `None` if fully clipped away.
fn clip_dst_to_uv(dst: Rect, clip: Rect) -> Option<(Rect, [f32; 4])> {
    let clipped = dst.intersection(clip)?;
    let x0 = i64::from(clipped.x);
    let y0 = i64::from(clipped.y);
    let x1 = x0 + i64::from(clipped.w);
    let y1 = y0 + i64::from(clipped.h);
    // A nonempty intersection proves both destination dimensions are positive.
    let dw = dst.w as f32;
    let dh = dst.h as f32;
    let u0 = (x0 - i64::from(dst.x)) as f32 / dw;
    let v0 = (y0 - i64::from(dst.y)) as f32 / dh;
    let u1 = (x1 - i64::from(dst.x)) as f32 / dw;
    let v1 = (y1 - i64::from(dst.y)) as f32 / dh;
    Some((clipped, [u0, v0, u1, v1]))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod gpu_contract_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) use gpu_contract_tests::verify_offscreen_gpu_contract;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_target_diagnostics_cannot_restore_gpu_authority() {
        assert!(serde_json::from_str::<CaptureTarget<'_>>("null").is_err());
    }

    #[test]
    fn mask_atlas_vertex_encoding_is_exact_at_page_extremes() {
        for origin in [0, 1, 2046, 2047] {
            for size in [1, 2, 2046, 2048] {
                let tint = MaskAtlasBounds {
                    origin: [origin; 2],
                    size: [size; 2],
                }
                .tint();
                for encoded in [tint[2], tint[3]] {
                    assert!(encoded < (1 << 24) as f32);
                    assert_eq!((encoded as u32) % 4096, origin);
                    assert_eq!((encoded as u32) / 4096, size);
                }
            }
        }
    }

    #[test]
    fn surface_diagnostics_never_restore_renderer_authority() {
        let handle = SurfaceHandle {
            id: 42,
            renderer: 7,
        };
        let json = serde_json::to_value(handle).unwrap();
        assert_eq!(json, serde_json::json!({"id": 42}));
        let restored: SurfaceHandle = serde_json::from_value(json).unwrap();
        assert_eq!(restored.legacy_id(), 42);
        assert_eq!(restored.renderer, 0);
        assert_ne!(restored, handle);
        let forged: SurfaceHandle =
            serde_json::from_value(serde_json::json!({"id": 42, "renderer": 7})).unwrap();
        assert_eq!(forged.renderer, 0);
    }

    fn queued_frame(index: u32) -> QueuedDraw {
        QueuedDraw {
            dst: Rect::new(0, 0, 1, 1),
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0; 4],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(index),
                blend: BlendMode::Blend,
            },
        }
    }

    #[test]
    fn src_dst_uv_full_source_defaults() {
        // No src rect, no dst rect: full texture positioned at (0,0),
        // uv spanning the whole texture.
        let (dst, uv) = src_dst_uv(None, None, 64.0, 32.0);
        assert_eq!(dst, Rect::new(0, 0, 64, 32));
        assert_eq!(uv, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn src_dst_uv_sub_rect_source() {
        // A 16x8 sub-rect at (16,8) of a 64x32 source, no dst rect:
        // positioned at (0,0) with the sub-rect's size, uv covering
        // only the sub-rect.
        let src = BBox::from_coords(16.0, 8.0, 32.0, 16.0);
        let (dst, uv) = src_dst_uv(Some(&src), None, 64.0, 32.0);
        assert_eq!(dst, Rect::new(0, 0, 16, 8));
        assert_eq!(uv, [0.25, 0.25, 0.5, 0.5]);
    }

    #[test]
    fn src_dst_uv_explicit_dst_rect() {
        // dst rect places (and stretches) the quad; uv still comes
        // from the src rect alone.
        let src = BBox::from_coords(0.0, 0.0, 32.0, 32.0);
        let dst_rect = BBox::from_coords(10.0, 20.0, 74.0, 52.0);
        let (dst, uv) = src_dst_uv(Some(&src), Some(&dst_rect), 64.0, 32.0);
        assert_eq!(dst, Rect::new(10, 20, 64, 32));
        assert_eq!(uv, [0.0, 0.0, 0.5, 1.0]);
    }

    #[test]
    fn src_dst_uv_inverted_src_rect_clamps_to_zero_size() {
        // A degenerate (max < min) src rect must not produce negative
        // sizes.
        let src = BBox::from_coords(16.0, 16.0, 8.0, 8.0);
        let (dst, uv) = src_dst_uv(Some(&src), None, 64.0, 32.0);
        assert_eq!(dst, Rect::new(0, 0, 0, 0));
        assert_eq!(uv, [0.25, 0.5, 0.25, 0.5]);
    }

    #[test]
    fn clipping_preserves_uvs_at_integer_coordinate_boundaries() {
        for origin in [i32::MIN, 0, i32::MAX - 5] {
            let dst = Rect::new(origin, origin, 10, 10);
            let clip = Rect::new(origin + 2, origin + 2, 4, 4);
            let (clipped, uv) = clip_dst_to_uv(dst, clip).unwrap();
            assert_eq!(clipped, clip);
            assert_eq!(uv, [0.2, 0.2, 0.6, 0.6]);
        }
        let invalid = Rect {
            x: i32::MIN,
            y: i32::MIN,
            w: -1,
            h: -1,
        };
        assert!(clip_dst_to_uv(invalid, invalid).is_none());
    }

    #[test]
    fn rgb565_byte_decoding_requires_an_exact_nonempty_layout() {
        assert_eq!(
            decode_rgb565_pixels(2, 1, &[0, 0, 0x34, 0x12]),
            Some(vec![0, 0x1234])
        );
        assert_eq!(
            decode_rgb565_pixels(1, 2, &[0, 0, 0xff, 0xff]),
            Some(vec![0, 0xffff])
        );
        for len in [0, 1, 2, 3, 5, 6] {
            assert_eq!(decode_rgb565_pixels(2, 1, &vec![0; len]), None);
        }
        for (width, height) in [(0, 0), (0, 1), (1, 0), (u16::MAX, u16::MAX)] {
            assert_eq!(decode_rgb565_pixels(width, height, &[]), None);
        }
    }

    #[test]
    fn clip_dst_to_uv_no_clip_is_identity() {
        let dst = Rect::new(10, 10, 20, 20);
        let clip = Rect::new(0, 0, 100, 100);
        let (clipped, uv) = clip_dst_to_uv(dst, clip).expect("unclipped rect must survive");
        assert_eq!(clipped, dst);
        assert_eq!(uv, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn clip_dst_to_uv_partial_clip_scales_uv() {
        // dst is 100..200 in x, clip cuts it to 150..200: right half
        // survives, uv.x starts at 0.5.
        let dst = Rect::new(100, 0, 100, 50);
        let clip = Rect::new(150, 0, 200, 50);
        let (clipped, uv) = clip_dst_to_uv(dst, clip).expect("half the rect survives");
        assert_eq!(clipped, Rect::new(150, 0, 50, 50));
        assert_eq!(uv, [0.5, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn clip_dst_to_uv_fully_clipped_away_is_none() {
        let dst = Rect::new(0, 0, 10, 10);
        let clip = Rect::new(50, 50, 10, 10);
        assert!(clip_dst_to_uv(dst, clip).is_none());
    }

    #[test]
    fn clip_dst_to_uv_zero_size_dst_does_not_divide_by_zero() {
        // A zero-sized dst is always fully clipped away (x1 <= x0), so
        // the `.max(1)` guard never produces NaN/inf uv values.
        let dst = Rect::new(5, 5, 0, 0);
        let clip = Rect::new(0, 0, 100, 100);
        assert!(clip_dst_to_uv(dst, clip).is_none());

        // 1x1 dst inside the clip exercises the divide with the
        // smallest legal size.
        let dst = Rect::new(5, 5, 1, 1);
        let (clipped, uv) = clip_dst_to_uv(dst, clip).expect("1x1 rect inside clip survives");
        assert_eq!(clipped, dst);
        assert_eq!(uv, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn sprite_mask_marks_every_component_draw_for_stencil_testing() {
        let mut draws = [queued_frame(7), queued_frame(11)];

        mark_draws_stencil_tested(&mut draws);

        assert!(matches!(
            draws[0].operation,
            DrawOperation::Masked { texture: 7, .. }
        ));
        assert!(matches!(
            draws[1].operation,
            DrawOperation::Masked { texture: 11, .. }
        ));
    }

    #[test]
    #[should_panic(expected = "non-textured draw queued inside a sprite mask region")]
    fn sprite_mask_rejects_unexpected_draw_kinds() {
        let mut draw = queued_frame(7);
        draw.operation = DrawOperation::Quad {
            texture: QuadTexture::White,
            blend: BlendMode::Blend,
        };
        mark_draws_stencil_tested(std::slice::from_mut(&mut draw));
    }

    #[test]
    fn managed_surface_shadow_masks_are_optional_and_preserve_pixel_positions() {
        let transparent = TRANSPARENT_COLOR_KEY_16;
        for pixels in [vec![], vec![transparent, 0xF800, 0x07E0]] {
            let (_, shadow) = rgb565_to_color_shadow_rgba(&pixels, transparent);
            assert!(shadow.is_none());
        }
        for shadow_index in 0..4 {
            let mut pixels = [0xF800, transparent, 0x07E0, 0xFFFF];
            pixels[shadow_index] = SHADOW_KEY;
            let (color, shadow) = rgb565_to_color_shadow_rgba(&pixels, transparent);
            let shadow = shadow.expect("one shadow pixel must produce a mask");
            for (index, &pixel) in pixels.iter().enumerate() {
                let offset = index * 4;
                if index == shadow_index {
                    assert_eq!(&color[offset..offset + 4], &[0, 0, 0, 0]);
                    assert_eq!(&shadow[offset..offset + 4], &[0, 0, 0, 255]);
                } else {
                    assert_eq!(&shadow[offset..offset + 4], &[0, 0, 0, 0]);
                    if pixel == transparent {
                        assert_eq!(&color[offset..offset + 4], &[0, 0, 0, 0]);
                    } else {
                        let (r, g, b) = rgb565_to_rgb8(pixel);
                        assert_eq!(&color[offset..offset + 4], &[r, g, b, 255]);
                    }
                }
            }
        }
        // Transparency takes precedence even if the caller chooses the shadow key.
        let (color, shadow) = rgb565_to_color_shadow_rgba(&[SHADOW_KEY], SHADOW_KEY);
        assert_eq!(color, [0, 0, 0, 0]);
        assert!(shadow.is_none());
    }

    #[test]
    fn rgba_upload_bakes_raw_and_arno_shadow_pixels_to_black_alpha() {
        let ambient_shadow = 0x2964;
        let normal_blue_gray = ambient_shadow;
        let pixels = [TRANSPARENT_COLOR_KEY_16, SHADOW_KEY, ambient_shadow, 0xF800];

        let shadow_alpha = shadow_alpha_from_level(40);
        assert_eq!(shadow_alpha, DEFAULT_SHADOW_ALPHA);

        let sprite_rgba = rgb565_to_rgba_with_key(
            &pixels,
            4,
            1,
            TRANSPARENT_COLOR_KEY_16,
            shadow_alpha,
            Some(ambient_shadow),
        );
        assert_eq!(&sprite_rgba[0..4], &[0, 0, 0, 0]);
        assert_eq!(&sprite_rgba[4..8], &[0, 0, 0, shadow_alpha]);
        assert_eq!(&sprite_rgba[8..12], &[0, 0, 0, shadow_alpha]);
        assert_eq!(&sprite_rgba[12..16], &[248, 0, 0, 255]);

        let surface_rgba = rgb565_to_rgba_with_key(
            &[normal_blue_gray],
            1,
            1,
            TRANSPARENT_COLOR_KEY_16,
            shadow_alpha,
            None,
        );
        assert_eq!(&surface_rgba[0..4], &[40, 44, 32, 255]);
    }

    #[test]
    fn runtime_rgba_sprites_upload_source_alpha_without_quantizing() {
        let mut holder = FrameHolder::default();
        let rgba = [
            255, 0, 0, 255, //
            8, 8, 8, 132, //
            0, 0, 0, 64, //
            0, 0, 0, 0,
        ];
        let bank_id = holder.append_rgba_sprite(2, 2, &rgba);

        let uploaded = sprite_rgba_for_upload(
            &holder,
            bank_id,
            SpriteVariant::Day,
            SHADOW_KEY,
            DEFAULT_SHADOW_ALPHA,
            16,
        );

        assert!(matches!(uploaded, std::borrow::Cow::Borrowed(_)));
        assert_eq!(uploaded.as_ref(), rgba);
        assert_eq!(
            uploaded.as_ptr(),
            holder.rgba_data(bank_id).unwrap().as_ptr()
        );
    }

    #[test]
    fn legacy_sprite_upload_owns_decoded_pixels_and_preserves_keys() {
        let mut holder = FrameHolder::default();
        let bank_id = holder.append_legacy_keyed_rgba_sprite(
            3,
            1,
            &[0, 250, 0, 255, 0, 0, 255, 255, 255, 0, 0, 255],
        );
        let uploaded = sprite_rgba_for_upload(
            &holder,
            bank_id,
            SpriteVariant::Day,
            SHADOW_KEY,
            DEFAULT_SHADOW_ALPHA,
            16,
        );
        assert!(matches!(uploaded, std::borrow::Cow::Owned(_)));
        assert_eq!(
            uploaded.as_ref(),
            &[0, 0, 0, 0, 0, 0, 0, DEFAULT_SHADOW_ALPHA, 248, 0, 0, 255]
        );
    }

    #[test]
    fn sprite_outline_rgba_marks_only_outside_horizontal_edges() {
        let solid = 0xF800;
        let pixels = [
            TRANSPARENT_COLOR_KEY_16,
            solid,
            solid,
            TRANSPARENT_COLOR_KEY_16,
            SHADOW_KEY,
            solid,
        ];
        let out_w = 6 + OUTLINE_PAD * 2;
        let outline = sprite_outline_rgba(&pixels, 6, 1, out_w, TRANSPARENT_COLOR_KEY_16, 0x2964);
        let rgba: &[u32] = bytemuck::cast_slice(&outline);

        let mut expected = vec![0u32; out_w];
        expected[1] = 0xFFFF_FFFF;
        expected[2] = 0xFFFF_FFFF;
        expected[5] = 0xFFFF_FFFF;
        expected[6] = 0xFFFF_FFFF;
        expected[8] = 0xFFFF_FFFF;
        expected[9] = 0xFFFF_FFFF;
        assert_eq!(rgba, expected.as_slice());
    }

    #[test]
    fn sprite_outline_rgba_treats_ambient_shadow_as_transparent() {
        let solid = 0xF800;
        let ambient_shadow = 0x2964;
        let pixels = [solid, ambient_shadow, ambient_shadow];
        let out_w = 3 + OUTLINE_PAD * 2;
        let outline = sprite_outline_rgba(
            &pixels,
            3,
            1,
            out_w,
            TRANSPARENT_COLOR_KEY_16,
            ambient_shadow,
        );
        let rgba: &[u32] = bytemuck::cast_slice(&outline);

        let opaque_count = rgba.iter().filter(|&&px| px == 0xFFFF_FFFF).count();
        assert_eq!(opaque_count, 4);
        assert_eq!(rgba[2], 0);
    }
}

mod diagnostics;

mod draw;
mod masks;
mod surfaces;
