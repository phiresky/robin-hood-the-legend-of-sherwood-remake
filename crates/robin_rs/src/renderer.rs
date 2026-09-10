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

#[inline]
pub fn rgb565_to_rgb8(px: u16) -> (u8, u8, u8) {
    (
        ((px >> 8) & 0xF8) as u8,
        ((px >> 3) & 0xFC) as u8,
        ((px << 3) & 0xF8) as u8,
    )
}

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

    /// Resolve the "surface 1 means surface 0" alias.
    fn resolve_id(&self, id: u32) -> u32 {
        GpuResources::resolve_surface_id(id)
    }

    pub fn create_color_16(r: u8, g: u8, b: u8) -> u16 {
        robin_util::color::rgb565(r, g, b)
    }

    /// Upload decoded pixels and return their unique retirement authority.
    pub fn upload_rgb565(
        &mut self,
        width: u16,
        height: u16,
        pixels: &[u16],
    ) -> Option<OwnedSurface> {
        let id = self.create_surface_from_rgb565(width, height, pixels)?;
        Some(self.adopt_surface(id))
    }

    /// Upload tightly packed little-endian RGB565 bytes without discarding a
    /// partial pixel. Invalid layouts are rejected before allocating pixels.
    pub(crate) fn upload_rgb565_bytes(
        &mut self,
        width: u16,
        height: u16,
        bytes: &[u8],
    ) -> Option<OwnedSurface> {
        let Some(pixels) = decode_rgb565_pixels(width, height, bytes) else {
            tracing::warn!(
                "invalid RGB565 byte layout: {width}x{height}, {} bytes",
                bytes.len()
            );
            return None;
        };
        self.upload_rgb565(width, height, &pixels)
    }

    pub(crate) fn upload_deferred_rgb565(
        &mut self,
        width: u16,
        height: u16,
        pixels: Box<[u16]>,
    ) -> OwnedSurface {
        let id = self.create_deferred_surface_from_rgb565(width, height, pixels);
        self.adopt_surface(id)
    }

    fn create_surface_from_rgb565(
        &mut self,
        width: u16,
        height: u16,
        pixels: &[u16],
    ) -> Option<u32> {
        let expected = width as usize * height as usize;
        if expected == 0 || pixels.len() != expected {
            tracing::warn!(
                "create_surface_from_rgb565: invalid dimensions/data: {}x{}, {} pixels",
                width,
                height,
                pixels.len()
            );
            return None;
        }
        let surface = self.build_managed_surface(width, height, pixels, DEFAULT_SHADOW_ALPHA)?;
        Some(self.resources.insert_managed_surface(surface))
    }

    fn build_managed_surface(
        &self,
        width: u16,
        height: u16,
        pixels: &[u16],
        shadow_alpha: u8,
    ) -> Option<ManagedSurface> {
        let w = width as usize;
        let h = height as usize;
        if w == 0 || h == 0 || pixels.len() != w * h {
            return None;
        }

        Some(ManagedSurface {
            width,
            height,
            pixels: ManagedSurfacePixels::Resident(
                self.upload_managed_surface(width, height, pixels),
            ),
            alpha_mask: AlphaMask::from_pixels(
                width,
                height,
                width as u32,
                pixels,
                TRANSPARENT_COLOR_KEY_16,
            ),
            shadow_alpha,
        })
    }

    /// Validate and register a surface immediately, retaining its RGB565 pixels
    /// until its first draw. Hit testing and dimensions are available before
    /// GPU residency, so unopened menus need no texture conversion or upload.
    fn create_deferred_surface_from_rgb565(
        &mut self,
        width: u16,
        height: u16,
        pixels: Box<[u16]>,
    ) -> u32 {
        assert!(
            width > 0 && height > 0,
            "deferred surface requires nonzero dimensions"
        );
        assert_eq!(
            pixels.len(),
            width as usize * height as usize,
            "deferred surface RGB565 payload must match dimensions"
        );
        let max_dimension = self.gpu.device.limits().max_texture_dimension_2d;
        assert!(
            u32::from(width) <= max_dimension && u32::from(height) <= max_dimension,
            "deferred surface dimensions exceed GPU texture limit {max_dimension}"
        );
        let alpha_mask = AlphaMask::from_pixels(
            width,
            height,
            width as u32,
            &pixels,
            TRANSPARENT_COLOR_KEY_16,
        );
        self.resources.insert_managed_surface(ManagedSurface {
            width,
            height,
            pixels: ManagedSurfacePixels::Pending(pixels),
            alpha_mask,
            shadow_alpha: DEFAULT_SHADOW_ALPHA,
        })
    }

    fn ensure_managed_surface_resident(&mut self, id: u32) {
        let Some(surface) = self.resources.managed_surfaces.get(&id) else {
            return;
        };
        let ManagedSurfacePixels::Pending(pixels) = &surface.pixels else {
            return;
        };
        let textures = self.upload_managed_surface(surface.width, surface.height, pixels);
        self.resources
            .managed_surfaces
            .get_mut(&id)
            .expect("surface remains registered during upload")
            .pixels = ManagedSurfacePixels::Resident(textures);
    }

    fn upload_managed_surface(
        &self,
        width: u16,
        height: u16,
        pixels: &[u16],
    ) -> ManagedSurfaceTextures {
        let opaque_rgba = rgb565_to_rgba_opaque(pixels, width as usize, height as usize);
        let (color_rgba, shadow_rgba) =
            rgb565_to_color_shadow_rgba(pixels, TRANSPARENT_COLOR_KEY_16);
        let (opaque_texture, opaque_view) = upload_rgba_texture(
            &self.gpu,
            &opaque_rgba,
            width as u32,
            height as u32,
            "managed surface opaque",
        );
        let opaque_bg = make_tex_bg(
            &self.gpu.device,
            &self.resources.bgl_tex,
            &opaque_view,
            &self.resources.sampler,
            "managed surface opaque bg",
        );
        let (color_texture, color_view) = upload_rgba_texture(
            &self.gpu,
            &color_rgba,
            width as u32,
            height as u32,
            "managed surface color",
        );
        let color_bg = make_tex_bg(
            &self.gpu.device,
            &self.resources.bgl_tex,
            &color_view,
            &self.resources.sampler,
            "managed surface color bg",
        );
        let (shadow_texture, shadow_view, shadow_bg) = if let Some(shadow_rgba) = shadow_rgba {
            let (texture, view) = upload_rgba_texture(
                &self.gpu,
                &shadow_rgba,
                width as u32,
                height as u32,
                "managed surface shadow",
            );
            let bg = make_tex_bg(
                &self.gpu.device,
                &self.resources.bgl_tex,
                &view,
                &self.resources.sampler,
                "managed surface shadow bg",
            );
            (Some(texture), Some(view), Some(bg))
        } else {
            (None, None, None)
        };

        ManagedSurfaceTextures {
            _opaque_texture: opaque_texture,
            _opaque_view: opaque_view,
            opaque_bg,
            _color_texture: color_texture,
            _color_view: color_view,
            color_bg,
            _shadow_texture: shadow_texture,
            _shadow_view: shadow_view,
            shadow_bg,
        }
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    fn delete_surface(&mut self, id: u32) -> bool {
        self.try_delete_legacy_surface(id)
            .expect("owned upload must be retired with its ownership token")
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn try_delete_legacy_surface(
        &mut self,
        id: u32,
    ) -> Result<bool, SurfaceOwnershipError> {
        if self.owned_surfaces.contains(&id) {
            return Err(SurfaceOwnershipError::AlreadyOwned(id));
        }
        Ok(self.resources.delete_managed_surface(id))
    }

    /// Compatibility boundary: resolve legacy screen aliases or mint a local upload reference.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    fn legacy_surface_target(&self, id: u32) -> Result<SurfaceTarget, MissingSurface> {
        if id <= 1 {
            Ok(SurfaceTarget::Screen)
        } else {
            self.surface_handle(id).map(SurfaceTarget::Upload)
        }
    }

    /// Validate an existing compatibility upload ID. Screen IDs are rejected.
    fn surface_handle(&self, id: u32) -> Result<SurfaceHandle, MissingSurface> {
        let handle = SurfaceHandle {
            id,
            renderer: self.identity,
        };
        self.surface_dimensions(handle)?;
        Ok(handle)
    }

    fn adopt_surface(&mut self, id: u32) -> OwnedSurface {
        self.try_adopt_surface(id)
            .expect("ownership requires a live unowned upload")
    }

    fn try_adopt_surface(&mut self, id: u32) -> Result<OwnedSurface, SurfaceOwnershipError> {
        self.validate_surface_adoption(id)?;
        let handle = self.surface_handle(id)?;
        self.owned_surfaces.insert(id);
        Ok(OwnedSurface { handle })
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn assert_legacy_adoption_rejected(&mut self, handle: SurfaceHandle) {
        assert!(self.try_adopt_surface(handle.id).is_err());
    }

    fn validate_surface_adoption(&self, id: u32) -> Result<(), SurfaceOwnershipError> {
        self.surface_handle(id)?;
        if self.owned_surfaces.contains(&id) {
            return Err(SurfaceOwnershipError::AlreadyOwned(id));
        }
        Ok(())
    }

    pub fn retire_surface(&mut self, surface: OwnedSurface) {
        self.try_retire_surface(surface)
            .expect("retirement requires the originating renderer and a live owned upload");
    }

    pub(crate) fn identity(&self) -> u64 {
        self.identity
    }

    /// Preflight a whole owner bank before retiring any member.
    pub(crate) fn validate_surface_retirement(
        &self,
        surface: &OwnedSurface,
    ) -> Result<(), SurfaceOwnershipError> {
        self.surface_dimensions(surface.handle)?;
        if !self.owned_surfaces.contains(&surface.handle.id) {
            return Err(SurfaceOwnershipError::NotOwned(surface.handle.id));
        }
        Ok(())
    }

    /// On rejection the caller retains its token and can retire it with the correct renderer.
    pub fn try_retire_surface(
        &mut self,
        surface: OwnedSurface,
    ) -> Result<(), (SurfaceOwnershipError, OwnedSurface)> {
        if let Err(error) = self.surface_dimensions(surface.handle) {
            return Err((error.into(), surface));
        }
        if !self.owned_surfaces.remove(&surface.handle.id) {
            return Err((SurfaceOwnershipError::NotOwned(surface.handle.id), surface));
        }
        assert!(
            self.resources.delete_managed_surface(surface.handle.id),
            "owned surface disappeared during retirement"
        );
        Ok(())
    }

    pub fn target_dimensions(&self, target: SurfaceTarget) -> Result<(u16, u16), MissingSurface> {
        match target {
            SurfaceTarget::Screen => Ok(self.frame.dimensions()),
            SurfaceTarget::Upload(handle) => self.surface_dimensions(handle),
        }
    }

    pub fn surface_dimensions(&self, handle: SurfaceHandle) -> Result<(u16, u16), MissingSurface> {
        if handle.renderer != self.identity || handle.id <= 1 {
            return Err(MissingSurface(handle));
        }
        self.resources
            .surface_dimensions(handle.id)
            .ok_or(MissingSurface(handle))
    }

    /// Build an `AlphaMask` from a managed surface — one bit per pixel,
    /// flagging non-transparent (`pixel != color_key`) pixels. Used by
    /// the UI hit-test path (`RendererBase::is_real_point`) so widget
    /// clicks on visually-transparent corners of round/non-rectangular
    /// sprites get rejected via a viewport pixel sample.
    pub fn build_alpha_mask(&self, id: u32) -> Option<AlphaMask> {
        self.resources.alpha_mask(id)
    }

    /// A foreign or retired handle is an error, not an absent optional mask.
    pub fn surface_alpha_mask(&self, handle: SurfaceHandle) -> Result<AlphaMask, MissingSurface> {
        self.surface_dimensions(handle)?;
        self.resources
            .alpha_mask(handle.id)
            .ok_or(MissingSurface(handle))
    }

    /// Override the shadow alpha baked into `SHADOW_KEY` pixels at
    /// upload time. Set to `MENU_BUTTON_SHADOW_ALPHA` (50%) for
    /// menu-button packs, leave at the default `DEFAULT_SHADOW_ALPHA`
    /// (40%) for everything else.
    pub fn set_surface_shadow_alpha(
        &mut self,
        handle: SurfaceHandle,
        shadow_alpha: u8,
    ) -> Result<(), MissingSurface> {
        self.surface_dimensions(handle)?;
        self.set_shadow_alpha(handle.id, shadow_alpha);
        Ok(())
    }

    fn set_shadow_alpha(&mut self, id: u32, shadow_alpha: u8) {
        self.resources.set_shadow_alpha(id, shadow_alpha);
    }

    pub fn create_loading_dissolve_textures(
        &self,
        width: u32,
        height: u32,
        initial_pixels: &[u16],
        final_pixels: &[u16],
        height_field: &crate::loading_screen::HeightField,
    ) -> Option<crate::loading_dissolve_gpu::LoadingDissolveTextures> {
        crate::loading_dissolve_gpu::upload_textures(
            &self.gpu,
            width,
            height,
            initial_pixels,
            final_pixels,
            height_field,
        )
    }

    /// Queue the sand-dissolve composite of the loading artwork into `dst`
    /// (logical canvas pixels). The caller picks the rectangle so the
    /// artwork can be aspect-fitted and centred on a canvas whose shape
    /// differs from the pictures' native 4:3.
    pub fn render_loading_dissolve(
        &mut self,
        textures: &crate::loading_dissolve_gpu::LoadingDissolveTextures,
        threshold: u32,
        dst: Rect,
    ) {
        if textures.width == 0 || textures.height == 0 || dst.w <= 0 || dst.h <= 0 {
            return;
        }
        let bind_group = crate::loading_dissolve_gpu::create_frame_bind_group(
            &self.gpu.device,
            &self.pipelines.bgl_loading_dissolve,
            textures,
            &self.resources.sampler,
        );
        let frame_idx = self.frame.frame_texture_bgs.len() as u32;
        self.frame.frame_texture_bgs.push(bind_group);
        self.frame.queued.push(QueuedDraw {
            dst,
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            // The shader compares `height > threshold`; threshold can be 256
            // at progress 0, so keep the normalized value slightly above 1.0.
            tint: [threshold as f32 / 255.0, 0.0, 0.0, 1.0],
            operation: DrawOperation::LoadingDissolve(frame_idx),
        });
    }

    pub fn create_rgb565_gpu_image(
        &self,
        width: u16,
        height: u16,
        pixels: &[u16],
        transparent: bool,
        label: &str,
    ) -> Option<GpuImage> {
        let expected = width as usize * height as usize;
        if expected == 0 || pixels.len() != expected {
            tracing::warn!(
                "create_rgb565_gpu_image: invalid dimensions/data for {label}: {}x{}, {} pixels",
                width,
                height,
                pixels.len()
            );
            return None;
        }
        let rgba = if transparent {
            rgb565_to_rgba_with_key(
                pixels,
                width as usize,
                height as usize,
                self.transparent_color(),
                0,
                None,
            )
        } else {
            rgb565_to_rgba_opaque(pixels, width as usize, height as usize)
        };
        let (texture, view) =
            upload_rgba_texture(&self.gpu, &rgba, width as u32, height as u32, label);
        let bind_group = make_tex_bg(
            &self.gpu.device,
            &self.resources.bgl_tex,
            &view,
            &self.resources.sampler,
            "gpu image bg",
        );
        Some(GpuImage {
            _texture: texture,
            _view: view,
            bind_group,
            width,
            height,
        })
    }

    pub fn create_rgba_gpu_image(
        &self,
        width: u16,
        height: u16,
        rgba: &[u8],
        label: &str,
    ) -> Option<GpuImage> {
        let expected = width as usize * height as usize * 4;
        if expected == 0 || rgba.len() != expected {
            tracing::warn!(
                "create_rgba_gpu_image: invalid dimensions/data for {label}: {}x{}, {} bytes",
                width,
                height,
                rgba.len()
            );
            return None;
        }
        let (texture, view) =
            upload_rgba_texture(&self.gpu, rgba, width as u32, height as u32, label);
        let bind_group = make_tex_bg(
            &self.gpu.device,
            &self.resources.bgl_tex,
            &view,
            &self.resources.sampler,
            "gpu image bg",
        );
        Some(GpuImage {
            _texture: texture,
            _view: view,
            bind_group,
            width,
            height,
        })
    }

    pub fn render_gpu_image(
        &mut self,
        image: &GpuImage,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        blend: BlendMode,
    ) {
        self.render_gpu_image_tinted(image, src_rect, dst_rect, blend, [1.0, 1.0, 1.0, 1.0]);
    }

    pub fn render_gpu_image_tinted(
        &mut self,
        image: &GpuImage,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        blend: BlendMode,
        tint: [f32; 4],
    ) {
        if image.width == 0 || image.height == 0 {
            return;
        }
        let (dst, uv) = src_dst_uv(src_rect, dst_rect, image.width as f32, image.height as f32);
        let tex_idx = self.queue_cached_bg(image.bind_group.clone());
        self.frame.queued.push(QueuedDraw {
            dst,
            corners: None,
            uv,
            tint,
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend,
            },
        });
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

    /// Outline a rect on the GPU overlay layer. Color is RGB565 to match the
    /// rest of the legacy rendering API.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_rect_outline_screen(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: u16) {
        let (r, g, b) = rgb565_to_rgb8(color);
        self.render_gpu_line(x1, y1, x2, y1, r, g, b);
        self.render_gpu_line(x2, y1, x2, y2, r, g, b);
        self.render_gpu_line(x2, y2, x1, y2, r, g, b);
        self.render_gpu_line(x1, y2, x1, y1, r, g, b);
    }

    /// Draw a line on the GPU overlay layer. RGB565 color in.
    pub fn draw_line_screen(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: u16) {
        let (r, g, b) = rgb565_to_rgb8(color);
        self.render_gpu_line(x1, y1, x2, y2, r, g, b);
    }

    /// Fill a rect on the GPU overlay layer. `rect=None` fills the
    /// whole logical screen. RGB565 color in.
    pub fn fill_screen(&mut self, rect: Option<&BBox>, color: u16) -> bool {
        let (r, g, b) = rgb565_to_rgb8(color);
        let (x, y, w, h) = match rect {
            Some(r) => (
                r.min.x as i32,
                r.min.y as i32,
                (r.max.x - r.min.x) as i32,
                (r.max.y - r.min.y) as i32,
            ),
            None => (0, 0, self.frame.width as i32, self.frame.height as i32),
        };
        self.render_gpu_rect(x, y, w, h, r, g, b, 255);
        true
    }

    /// Start a GPU-only frame with the render target's normal black clear and
    /// no legacy framebuffer upload. Used by menus/loading states whose
    /// background is fully drawn by queued GPU quads.
    pub fn begin_gpu_frame_clear(&mut self) {
        self.frame.enter_gpu_phase();
    }

    /// Blit a sprite from a managed RGB565 surface to the screen,
    /// multiply-darkening pixels matching `SHADOW_KEY` by
    /// `(100 - shadow_level) / 100`. Routes the MMX-style alpha-keying
    /// shadow blit through the GPU overlay queue.
    #[allow(clippy::too_many_arguments)]
    fn blit_with_shadow(
        &mut self,
        src_id: u32,
        src_rect: Option<&BBox>,
        dst_id: u32,
        dst_rect: Option<&BBox>,
        _shadow_color: u16,
        shadow_level: u16,
        flags: u32,
    ) -> bool {
        let src_id = self.resolve_id(src_id);
        let dst_id = self.resolve_id(dst_id);
        if dst_id != 0 {
            tracing::warn!("blit_with_shadow: GPU path requires screen destination");
            return false;
        }
        self.frame.enter_gpu_phase();

        // Snapshot src-side data with a single immutable borrow.
        let (blit_w, blit_h, src_x, src_y) = {
            let src_info = match self.resources.managed_surfaces.get(&src_id) {
                Some(i) => i,
                None => return false,
            };
            let (sx, sy, w, h) = if let Some(r) = src_rect {
                (
                    r.min.x as usize,
                    r.min.y as usize,
                    (r.max.x - r.min.x) as usize,
                    (r.max.y - r.min.y) as usize,
                )
            } else {
                (0, 0, src_info.width as usize, src_info.height as usize)
            };
            (w, h, sx, sy)
        };
        if blit_w == 0 || blit_h == 0 {
            return false;
        }
        // shadow_alpha = shadow_level * 255 / 100 → multiply-darken at
        // (1 - shadow_alpha/255) under standard alpha blending.
        let shadow_alpha = (shadow_level.min(100) as u32 * 255 / 100) as u8;

        // Determine the dst rect (default = source size at origin 0).
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
                w: blit_w as i32,
                h: blit_h as i32,
            },
        };

        if flags & BLIT_SOURCE_TRANSPARENT == 0 {
            return self.blit_to_screen(src_id, src_rect, dst_rect, flags);
        }
        self.ensure_managed_surface_resident(src_id);
        let Some(src_surface) = self.resources.managed_surfaces.get(&src_id) else {
            return false;
        };
        let sw = src_surface.width as f32;
        let sh = src_surface.height as f32;
        let uv = [
            src_x as f32 / sw,
            src_y as f32 / sh,
            (src_x + blit_w) as f32 / sw,
            (src_y + blit_h) as f32 / sh,
        ];
        self.queue_transparent_managed_bgs(
            src_surface.textures().color_bg.clone(),
            src_surface.textures().shadow_bg.clone(),
            shadow_alpha,
            dst,
            uv,
            1.0,
        );
        true
    }

    /// Queue a borrowed upload only after checking its renderer and lifetime.
    pub fn draw_surface(
        &mut self,
        handle: SurfaceHandle,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        flags: u32,
    ) -> Result<(), MissingSurface> {
        self.surface_dimensions(handle)?;
        assert!(self.blit_to_screen(handle.id, src_rect, dst_rect, flags));
        Ok(())
    }

    /// Rotate a borrowed UI surface counterclockwise inside its destination.
    pub(crate) fn draw_surface_rotated_ccw(
        &mut self,
        handle: SurfaceHandle,
        dst: &BBox,
    ) -> Result<(), MissingSurface> {
        let first = self.frame.queued.len();
        self.draw_surface(handle, None, Some(dst), BLIT_SOURCE_TRANSPARENT)?;
        for draw in &mut self.frame.queued[first..] {
            draw.corners = Some([
                (dst.min.x, dst.max.y),
                (dst.min.x, dst.min.y),
                (dst.max.x, dst.max.y),
                (dst.max.x, dst.min.y),
            ]);
        }
        Ok(())
    }

    /// Alpha draw with the same renderer/lifetime checks as ordinary typed draws.
    pub fn draw_surface_alpha(
        &mut self,
        handle: SurfaceHandle,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        alpha_level: u16,
        flags: u32,
    ) -> Result<(), MissingSurface> {
        self.surface_dimensions(handle)?;
        assert!(self.blit_to_screen_alpha(handle.id, src_rect, dst_rect, alpha_level, flags));
        Ok(())
    }

    /// Shadow draw for a borrowed upload, always targeting this renderer's screen.
    pub fn draw_surface_with_shadow(
        &mut self,
        handle: SurfaceHandle,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        shadow_color: u16,
        shadow_level: u16,
        flags: u32,
    ) -> Result<(), MissingSurface> {
        self.surface_dimensions(handle)?;
        assert!(self.blit_with_shadow(
            handle.id,
            src_rect,
            0,
            dst_rect,
            shadow_color,
            shadow_level,
            flags
        ));
        Ok(())
    }

    /// Legacy compatibility entry point; new resource owners should retain typed handles.
    /// Submit a managed surface as a GPU overlay quad. Lazy-uploads
    /// the surface to a wgpu texture (cached, invalidated on surface
    /// mutation) and queues a textured-quad draw at `dst_rect`.
    fn blit_to_screen(
        &mut self,
        src_id: u32,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        flags: u32,
    ) -> bool {
        let id = self.resolve_id(src_id);
        let transparent = flags & BLIT_SOURCE_TRANSPARENT != 0;
        self.ensure_managed_surface_resident(id);
        let Some(surface) = self.resources.managed_surfaces.get(&id) else {
            return false;
        };
        let (sw, sh) = (surface.width as f32, surface.height as f32);
        let (sub_dst, sub_uv) = src_dst_uv(src_rect, dst_rect, sw, sh);
        if transparent {
            self.queue_transparent_managed_bgs(
                surface.textures().color_bg.clone(),
                surface.textures().shadow_bg.clone(),
                surface.shadow_alpha,
                sub_dst,
                sub_uv,
                1.0,
            );
        } else {
            let tex_idx = self.queue_cached_bg(surface.textures().opaque_bg.clone());
            self.frame.queued.push(QueuedDraw {
                dst: sub_dst,
                corners: None,
                uv: sub_uv,
                tint: [1.0, 1.0, 1.0, 1.0],
                operation: DrawOperation::Quad {
                    texture: QuadTexture::Frame(tex_idx),
                    blend: BlendMode::Blend,
                },
            });
        }
        true
    }

    /// `blit_to_screen` with a per-frame alpha applied to the whole
    /// quad (used by the fade-in / fade-out transitions).
    fn blit_to_screen_alpha(
        &mut self,
        src_id: u32,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        alpha_level: u16,
        flags: u32,
    ) -> bool {
        let id = self.resolve_id(src_id);
        let transparent = flags & BLIT_SOURCE_TRANSPARENT != 0;
        self.ensure_managed_surface_resident(id);
        let Some(surface) = self.resources.managed_surfaces.get(&id) else {
            return false;
        };
        let (sw, sh) = (surface.width as f32, surface.height as f32);
        let (sub_dst, sub_uv) = src_dst_uv(src_rect, dst_rect, sw, sh);
        // alpha_level: 0 = fully opaque, 100 = fully transparent.
        // Convert to a 0..1 multiplier.
        let alpha = ((100u16.saturating_sub(alpha_level)) as f32 / 100.0).clamp(0.0, 1.0);
        if transparent {
            self.queue_transparent_managed_bgs(
                surface.textures().color_bg.clone(),
                surface.textures().shadow_bg.clone(),
                surface.shadow_alpha,
                sub_dst,
                sub_uv,
                alpha,
            );
        } else {
            let tex_idx = self.queue_cached_bg(surface.textures().opaque_bg.clone());
            self.frame.queued.push(QueuedDraw {
                dst: sub_dst,
                corners: None,
                uv: sub_uv,
                tint: [1.0, 1.0, 1.0, alpha],
                operation: DrawOperation::Quad {
                    texture: QuadTexture::Frame(tex_idx),
                    blend: BlendMode::Blend,
                },
            });
        }
        true
    }

    fn queue_transparent_managed_bgs(
        &mut self,
        color_bg: wgpu::BindGroup,
        shadow_bg: Option<wgpu::BindGroup>,
        shadow_alpha: u8,
        dst: Rect,
        uv: [f32; 4],
        opacity: f32,
    ) {
        if let Some(shadow_bg) = shadow_bg {
            let tex_idx = self.queue_cached_bg(shadow_bg);
            self.frame.queued.push(QueuedDraw {
                dst,
                corners: None,
                uv,
                tint: [
                    1.0,
                    1.0,
                    1.0,
                    (shadow_alpha as f32 / 255.0 * opacity).clamp(0.0, 1.0),
                ],
                operation: DrawOperation::Quad {
                    texture: QuadTexture::Frame(tex_idx),
                    blend: BlendMode::Blend,
                },
            });
        }
        let tex_idx = self.queue_cached_bg(color_bg);
        self.frame.queued.push(QueuedDraw {
            dst,
            corners: None,
            uv,
            tint: [1.0, 1.0, 1.0, opacity.clamp(0.0, 1.0)],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend: BlendMode::Blend,
            },
        });
    }

    /// Draw a 1-pixel-thick line from (x1,y1) to (x2,y2). Implemented
    /// as a rotated thin quad — 4 corners offset by ±0.5 perpendicular
    /// to the line direction — so diagonal lines render as a real line
    /// rather than the bounding-box outline placeholder. Used by the
    /// view-cone outlines and debug overlays.
    #[allow(clippy::too_many_arguments)]
    pub fn render_gpu_line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, r: u8, g: u8, b: u8) {
        let tint = Color::rgb(r, g, b).to_f32_srgb();
        // Axis-aligned single-pixel strips stay on the rect path to avoid
        // half-pixel rounding from the perpendicular offset.
        if y1 == y2 {
            let lx = x1.min(x2);
            let rx = x1.max(x2);
            self.frame.queued.push(QueuedDraw {
                dst: Rect {
                    x: lx,
                    y: y1,
                    w: (rx - lx).max(1),
                    h: 1,
                },
                corners: None,
                uv: [0.0, 0.0, 1.0, 1.0],
                tint,
                operation: DrawOperation::Quad {
                    texture: QuadTexture::White,
                    blend: BlendMode::None,
                },
            });
            return;
        }
        if x1 == x2 {
            let ty = y1.min(y2);
            let by = y1.max(y2);
            self.frame.queued.push(QueuedDraw {
                dst: Rect {
                    x: x1,
                    y: ty,
                    w: 1,
                    h: (by - ty).max(1),
                },
                corners: None,
                uv: [0.0, 0.0, 1.0, 1.0],
                tint,
                operation: DrawOperation::Quad {
                    texture: QuadTexture::White,
                    blend: BlendMode::None,
                },
            });
            return;
        }
        // Diagonal — build a thin rotated quad covering the line.
        // Endpoints sit on the half-pixel centre; the perpendicular
        // offset of ±0.5 gives a 1-pixel-thick strip oriented along
        // the line direction.
        let p1 = (x1 as f32 + 0.5, y1 as f32 + 0.5);
        let p2 = (x2 as f32 + 0.5, y2 as f32 + 0.5);
        let dx = p2.0 - p1.0;
        let dy = p2.1 - p1.1;
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        let nx = -dy / len * 0.5;
        let ny = dx / len * 0.5;
        let corners = [
            (p1.0 + nx, p1.1 + ny), // TL
            (p2.0 + nx, p2.1 + ny), // TR
            (p1.0 - nx, p1.1 - ny), // BL
            (p2.0 - nx, p2.1 - ny), // BR
        ];
        self.frame.queued.push(QueuedDraw {
            dst: Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            corners: Some(corners),
            uv: [0.0, 0.0, 1.0, 1.0],
            tint,
            operation: DrawOperation::Quad {
                texture: QuadTexture::White,
                blend: BlendMode::None,
            },
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_gpu_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: u8, g: u8, b: u8, a: u8) {
        self.frame.queued.push(QueuedDraw {
            dst: Rect { x, y, w, h },
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: Color::rgba(r, g, b, a).to_f32_srgb(),
            operation: DrawOperation::Quad {
                texture: QuadTexture::White,
                blend: BlendMode::Blend,
            },
        });
    }

    /// Draw the persistent, linearly sampled fog mask. The CPU only uploads
    /// new texels when deterministic fog or an exact PC reveal circle changes;
    /// panning and zooming reuse the texture with a different UV rectangle.
    #[allow(clippy::too_many_arguments)]
    pub fn render_fog_mask(
        &mut self,
        width: u32,
        height: u32,
        generation: u64,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        uv: [f32; 4],
        build_rgba: impl FnOnce() -> Vec<u8>,
    ) {
        if w <= 0 || h <= 0 {
            return;
        }
        let recreate = self
            .fog_mask
            .as_ref()
            .is_none_or(|mask| mask.width != width || mask.height != height);
        let update = self
            .fog_mask
            .as_ref()
            .is_some_and(|mask| mask.generation != generation);
        if recreate || update {
            let rgba = build_rgba();
            let expected = width
                .checked_mul(height)
                .and_then(|pixels| pixels.checked_mul(4))
                .unwrap_or_else(|| panic!("fog mask dimensions overflow: {width}x{height}"));
            assert_eq!(
                rgba.len(),
                expected as usize,
                "fog mask pixel count does not match {width}x{height}"
            );
            if !recreate {
                let mask = self
                    .fog_mask
                    .as_mut()
                    .expect("fog mask disappeared while updating it");
                self.gpu.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &mask._texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &rgba,
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
                mask.generation = generation;
            } else {
                let (texture, view) =
                    upload_rgba_texture(&self.gpu, &rgba, width, height, "smooth fog mask");
                let bind_group = make_tex_bg(
                    &self.gpu.device,
                    &self.resources.bgl_tex,
                    &view,
                    &self.resources.linear_sampler,
                    "smooth fog mask bg",
                );
                self.fog_mask = Some(FogMaskTexture {
                    _texture: texture,
                    _view: view,
                    bind_group,
                    width,
                    height,
                    generation,
                });
            }
        }

        let bind_group = self
            .fog_mask
            .as_ref()
            .expect("fog mask was not created")
            .bind_group
            .clone();
        let tex_idx = self.queue_cached_bg(bind_group);
        self.frame.queued.push(QueuedDraw {
            dst: Rect { x, y, w, h },
            corners: None,
            uv,
            tint: [1.0; 4],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend: BlendMode::Blend,
            },
        });
    }

    /// Filled triangle on the GPU overlay. Submitted as a degenerate
    /// quad: corner layout is `[A, B, C, C]` so the 6-vertex
    /// expansion in `upload_queue_geometry` (`[TL, TR, BL, BL, TR,
    /// BR]`) emits one real triangle `(A, B, C)` followed by a
    /// zero-area triangle `(C, B, C)`. Used by the debug shadow /
    /// view-cone overlays.
    pub fn render_gpu_triangle(&mut self, pts: [(f32, f32); 3], r: u8, g: u8, b: u8, a: u8) {
        self.frame.queued.push(QueuedDraw {
            dst: Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            corners: Some([pts[0], pts[1], pts[2], pts[2]]),
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: Color::rgba(r, g, b, a).to_f32_srgb(),
            operation: DrawOperation::Quad {
                texture: QuadTexture::White,
                blend: BlendMode::Blend,
            },
        });
    }

    /// Tint the current frame toward `desired_hue` with `scale`
    /// intensity (0..1). Used by the pause menu to dim + colour-shift
    /// the in-game image while the menu is up.
    ///
    /// Per-pixel HSV-replace using the desired hue with the source
    /// pixel's saturation and value (after a `(2*scale, scale, scale)`
    /// pre-scale that gives the dim a warm bias). Implemented as a
    /// fullscreen quad through the `fs_colorize` pipeline that samples
    /// the modal-snapshot texture (`freeze_scene_for_modal`) and writes
    /// the recoloured result into the offscreen RT, replacing the
    /// unmodified frozen-scene quad that would otherwise have been
    /// pushed by `present()`.
    pub fn colorize_framebuffer(&mut self, desired_hue: f32, scale: f32) {
        if self.frame.frozen_scene.is_none() {
            // No snapshot to recolour — colorize is a no-op outside
            // the modal flow. The recolour runs against the scene
            // snapshot captured by `freeze_scene_for_modal`.
            return;
        }
        let scale = scale.clamp(0.0, 1.0);
        let hue = desired_hue.rem_euclid(360.0) / 360.0;
        self.frame.queued.push(QueuedDraw {
            dst: Rect {
                x: 0,
                y: 0,
                w: self.frame.width as i32,
                h: self.frame.height as i32,
            },
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: [hue, scale, 0.0, 0.0],
            operation: DrawOperation::ColorizeFrozen,
        });
    }

    /// Decompress sprite frame `(bank_id, variant)` into the GPU cache,
    /// converting RGB565 → RGBA8 with the shadow key baked into alpha.
    /// Returns `Some((width, height))` of the cached frame on success.
    pub fn ensure_sprite_cached(
        &mut self,
        frame_holder: &FrameHolder,
        bank_id: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        shadow_level: u16,
    ) -> Option<(u16, u16)> {
        self.resources.ensure_sprite_cached(
            &self.gpu,
            frame_holder,
            bank_id,
            variant,
            shadow_color,
            shadow_level,
        )
    }

    /// Build the GPU cache for the edge-map outline used by the
    /// selection / mouse-over highlights. The texture is transparent
    /// except for the two outside pixels written at each horizontal
    /// sprite-body edge; the draw-time tint supplies the actual colour.
    pub fn ensure_outline_cached(
        &mut self,
        frame_holder: &FrameHolder,
        bank_id: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        shadow_level: u16,
    ) -> Option<(u16, u16)> {
        self.resources.ensure_outline_cached(
            &self.gpu,
            frame_holder,
            bank_id,
            variant,
            shadow_color,
            shadow_level,
        )
    }

    /// Upload the static binary alpha for a sprite-occlusion mask. It is
    /// rasterized into stencil for each affected sprite, matching the
    /// original engine's temporary-sprite transparency operation.
    /// Upload mission masks in shared R8 pages. Standalone uploads remain
    /// available for oversized masks and the profiling override.
    pub fn upload_mask_alphas<'a>(
        &mut self,
        masks: impl IntoIterator<Item = (u32, &'a [u8], u16, u16)>,
    ) -> Result<(), String> {
        self.resources.upload_mask_alphas(&self.gpu, masks)
    }

    pub fn upload_mask_alpha(
        &mut self,
        mask_index: u32,
        bitmap: &[u8],
        mask_w: u16,
        mask_h: u16,
    ) -> bool {
        self.resources
            .upload_mask_alpha(&self.gpu, mask_index, bitmap, mask_w, mask_h)
    }

    /// Upload a map-sized R16 ground-depth field. Each texel stores the
    /// map-ground Y of the visible reconstructed surface at that pixel.
    pub fn upload_occlusion_depth(&mut self, depth: &[u16], width: u16, height: u16) -> bool {
        self.resources.upload_occlusion_depth(
            &self.gpu,
            OCCLUSION_DEPTH_TEXTURE_INDEX,
            depth,
            width,
            height,
        )
    }

    /// Return a stable insertion point immediately before the next sprite or
    /// ground-mark draw that may need building occlusion.
    pub(crate) fn draw_queue_checkpoint(&self) -> usize {
        self.frame.queued.len()
    }

    /// Apply the union of `masks` to draws queued since `checkpoint`.
    ///
    /// Each mask is written into stencil, the affected draws are switched to
    /// stencil-tested pipelines, and the clipped sprite rectangle is cleared
    /// afterward. Masked fragments therefore never touch the framebuffer,
    /// exactly like the original temporary-sprite masking path.
    pub(crate) fn mask_queued_draws(
        &mut self,
        checkpoint: usize,
        masks: &[(u32, Rect)],
        clip_rect: Rect,
    ) {
        self.mask_queued_draws_impl(checkpoint, masks, clip_rect, None);
    }

    /// Apply legacy masks plus the continuous reconstructed-geometry depth
    /// field to one grounded sprite. `actor_ground_y` is world Y (not the
    /// projected map Y), so elevated and inclined paths compare correctly.
    pub(crate) fn mask_queued_draws_with_depth(
        &mut self,
        checkpoint: usize,
        masks: &[(u32, Rect)],
        clip_rect: Rect,
        view_x: f32,
        view_y: f32,
        zoom: f32,
        actor_ground_y: f32,
    ) {
        let depth = self
            .resources
            .mask_alpha_cache
            .get(&OCCLUSION_DEPTH_TEXTURE_INDEX)
            .map(|mask| {
                let rect = Rect::new(
                    (-view_x * zoom).round() as i32,
                    (-view_y * zoom).round() as i32,
                    (mask.width as f32 * zoom).round() as u32,
                    (mask.height as f32 * zoom).round() as u32,
                );
                // A small bias prevents the character's own ground surface
                // from flickering into stencil through quantization/filtering.
                let threshold = (actor_ground_y / mask.height as f32 + 2.0 / mask.height as f32)
                    .clamp(0.0, 1.0);
                (rect, threshold)
            });
        self.mask_queued_draws_impl(checkpoint, masks, clip_rect, depth);
    }

    fn mask_queued_draws_impl(
        &mut self,
        checkpoint: usize,
        masks: &[(u32, Rect)],
        clip_rect: Rect,
        depth: Option<(Rect, f32)>,
    ) {
        assert!(
            checkpoint <= self.frame.queued.len(),
            "sprite mask checkpoint {checkpoint} exceeds draw queue length {}",
            self.frame.queued.len()
        );
        if masks.is_empty() && depth.is_none() {
            return;
        }
        assert!(
            checkpoint < self.frame.queued.len(),
            "sprite masks require at least one queued draw"
        );

        let mut stencil_draws = Vec::with_capacity(masks.len() + usize::from(depth.is_some()));
        for &(mask_index, mask_rect) in masks {
            assert!(
                self.resources.mask_alpha_cache.contains_key(&mask_index),
                "sprite mask {mask_index} was not uploaded"
            );
            let Some((dst, uv)) = clip_dst_to_uv(mask_rect, clip_rect) else {
                continue;
            };
            stencil_draws.push(QueuedDraw {
                dst,
                corners: None,
                uv,
                tint: self.resources.mask_alpha_cache[&mask_index]
                    .atlas
                    .map_or([0.5, 0.0, 1.0, 1.0], MaskAtlasBounds::tint),
                operation: DrawOperation::MaskAlpha(mask_index),
            });
        }
        if let Some((depth_rect, threshold)) = depth
            && let Some((dst, uv)) = clip_dst_to_uv(depth_rect, clip_rect)
        {
            stencil_draws.push(QueuedDraw {
                dst,
                corners: None,
                uv,
                // Green selects 16-bit high/low reconstruction in the
                // shared mask-stencil shader.
                tint: [threshold, 1.0, 1.0, 1.0],
                operation: DrawOperation::MaskAlpha(OCCLUSION_DEPTH_TEXTURE_INDEX),
            });
        }
        if stencil_draws.is_empty() {
            return;
        }

        mark_draws_stencil_tested(&mut self.frame.queued[checkpoint..]);
        self.frame
            .queued
            .splice(checkpoint..checkpoint, stencil_draws);
        self.frame.queued.push(QueuedDraw {
            dst: clip_rect,
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: [0.5, 0.0, 1.0, 1.0],
            operation: DrawOperation::StencilClear,
        });
    }

    /// Queue a cached sprite as an alpha-blended GPU overlay quad.
    /// `pub(crate)` to match the original visibility so game_render
    /// can still reach it.
    pub(crate) fn render_cached_sprite(
        &mut self,
        bank_id: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        shadow_level: u16,
        dst_rect: Rect,
    ) -> bool {
        let key = SpriteCacheKey {
            bank_id,
            variant,
            shadow_color: shadow_color as u32,
            shadow_alpha: shadow_alpha_from_level(shadow_level),
        };
        let Some((tex_idx, uv)) = self.queue_sprite_texture(&key) else {
            return false;
        };
        self.frame.queued.push(QueuedDraw {
            dst: dst_rect,
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

    /// Like [`render_cached_sprite`] but applies a per-frame alpha to
    /// the whole quad (used by the fade-out / damage-flash paths).
    pub(crate) fn render_cached_sprite_alpha(
        &mut self,
        bank_id: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        shadow_level: u16,
        dst_rect: Rect,
        alpha: u8,
    ) -> bool {
        let key = SpriteCacheKey {
            bank_id,
            variant,
            shadow_color: shadow_color as u32,
            shadow_alpha: shadow_alpha_from_level(shadow_level),
        };
        let Some((tex_idx, uv)) = self.queue_sprite_texture(&key) else {
            return false;
        };
        self.frame.queued.push(QueuedDraw {
            dst: dst_rect,
            corners: None,
            uv,
            tint: [1.0, 1.0, 1.0, alpha as f32 / 255.0],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend: BlendMode::Blend,
            },
        });
        true
    }

    /// Queue the cached edge-map outline tinted by `rgb * alpha`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_cached_outline(
        &mut self,
        bank_id: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        _shadow_level: u16,
        dst_rect: Rect,
        rgb: (u8, u8, u8),
        alpha: u8,
    ) -> bool {
        let key = outline_cache_key(bank_id, variant, shadow_color);
        let Some((tex_idx, uv)) = self.queue_sprite_texture(&key) else {
            return false;
        };
        self.frame.queued.push(QueuedDraw {
            dst: dst_rect,
            corners: None,
            uv,
            tint: [
                rgb.0 as f32 / 255.0,
                rgb.1 as f32 / 255.0,
                rgb.2 as f32 / 255.0,
                alpha as f32 / 255.0,
            ],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend: BlendMode::Blend,
            },
        });
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_hidden_mask_outline(
        &mut self,
        frame_holder: &FrameHolder,
        bank_id: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        mask_bitmap: &[u8],
        mask_w: u16,
        mask_h: u16,
        mask_rect: Rect,
        sprite_rect: Rect,
        rgb: (u8, u8, u8),
    ) -> bool {
        if mask_w == 0 || mask_h == 0 || mask_rect.w <= 0 || mask_rect.h <= 0 {
            return true;
        }
        if sprite_rect.w <= 0 || sprite_rect.h <= 0 {
            return true;
        }
        let expected_mask = mask_w as usize * mask_h as usize;
        if mask_bitmap.len() < expected_mask {
            return false;
        }

        let viewport = Rect::new(0, 0, self.frame.width.into(), self.frame.height.into());
        let Some(overlap) = mask_rect
            .intersection(sprite_rect)
            .and_then(|overlap| overlap.intersection(viewport))
        else {
            return true;
        };
        let left = overlap.left();
        let top = overlap.top();
        let right = overlap.right();
        let bottom = overlap.bottom();

        let sw = frame_holder.sprite_width(bank_id) as usize;
        let sh = frame_holder.sprite_height(bank_id) as usize;
        if sw == 0 || sh == 0 {
            return false;
        }

        let mut sprite = vec![TRANSPARENT_COLOR_KEY_16; sw * sh];
        frame_holder.uncompress_frame(
            &mut sprite,
            sw,
            bank_id,
            variant,
            shadow_color,
            self.resources.bit_depth,
        );

        let out_w = (right - left) as usize;
        let out_h = (bottom - top) as usize;
        let mut rgba = vec![0u8; out_w * out_h * 4];
        let mut any = false;
        let mask_scale_x = mask_w as f32 / mask_rect.w as f32;
        let mask_scale_y = mask_h as f32 / mask_rect.h as f32;
        let sprite_scale_x = sw as f32 / sprite_rect.w as f32;
        let sprite_scale_y = sh as f32 / sprite_rect.h as f32;

        for y in top..bottom {
            let mask_y = ((y - mask_rect.y) as f32 * mask_scale_y).floor() as usize;
            let sy = ((y - sprite_rect.y) as f32 * sprite_scale_y).floor() as usize;
            if mask_y >= mask_h as usize || sy >= sh {
                continue;
            }
            for x in left..right {
                let mask_x = ((x - mask_rect.x) as f32 * mask_scale_x).floor() as usize;
                if mask_x >= mask_w as usize {
                    continue;
                }
                if mask_bitmap[mask_y * mask_w as usize + mask_x] == 0 {
                    continue;
                }

                let sx = ((x - sprite_rect.x) as f32 * sprite_scale_x).floor() as usize;
                if sx + 1 >= sw {
                    continue;
                }
                let row = sy * sw;
                let mut p1 = sprite[row + sx];
                let mut p2 = sprite[row + sx + 1];
                if p1 == shadow_color {
                    p1 = TRANSPARENT_COLOR_KEY_16;
                }
                if p2 == shadow_color {
                    p2 = TRANSPARENT_COLOR_KEY_16;
                }
                if p1 != p2 && (p1 == TRANSPARENT_COLOR_KEY_16 || p2 == TRANSPARENT_COLOR_KEY_16) {
                    let out_idx = (((y - top) as usize * out_w) + (x - left) as usize) * 4;
                    rgba[out_idx] = rgb.0;
                    rgba[out_idx + 1] = rgb.1;
                    rgba[out_idx + 2] = rgb.2;
                    rgba[out_idx + 3] = 255;
                    any = true;
                }
            }
        }

        if !any {
            return true;
        }

        let (_texture, view) = upload_rgba_texture(
            &self.gpu,
            &rgba,
            out_w as u32,
            out_h as u32,
            "hidden mask outline",
        );
        let tex_idx = self.queue_frame_texture(&view);
        self.frame.queued.push(QueuedDraw {
            dst: Rect::new(left, top, out_w as u32, out_h as u32),
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 1.0],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend: BlendMode::Blend,
            },
        });
        true
    }

    pub fn clear_mask_alpha_cache(&mut self) {
        self.resources.clear_mask_alpha_cache();
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

    pub fn render_framebuffer_alpha_rect(
        &mut self,
        dst_rect: Rect,
        uv: [f32; 4],
        color: u32,
        alpha_256: u32,
    ) -> bool {
        if dst_rect.w <= 0 || dst_rect.h <= 0 {
            return false;
        }
        let r = ((color >> 16) & 0xFF) as f32 / 255.0;
        let g = ((color >> 8) & 0xFF) as f32 / 255.0;
        let b = (color & 0xFF) as f32 / 255.0;
        let a = alpha_256.min(256) as f32 / 256.0;
        self.frame.queued.push(QueuedDraw {
            dst: dst_rect,
            corners: None,
            uv,
            tint: [r, g, b, a],
            operation: DrawOperation::FramebufferAlpha,
        });
        true
    }

    pub fn render_view_cone_span(
        &mut self,
        dst_rect: Rect,
        tint: (u8, u8, u8),
        alpha_left: u8,
        alpha_right: u8,
    ) {
        if dst_rect.w <= 0 || dst_rect.h <= 0 {
            return;
        }
        self.frame.queued.push(QueuedDraw {
            dst: dst_rect,
            corners: None,
            uv: [
                alpha_left as f32 / 255.0,
                0.0,
                alpha_right as f32 / 255.0,
                0.0,
            ],
            tint: [
                tint.0 as f32 / 255.0,
                tint.1 as f32 / 255.0,
                tint.2 as f32 / 255.0,
                1.0,
            ],
            operation: DrawOperation::ViewConeGradient,
        });
    }

    /// Render a string of native-font text by emitting one quad per
    /// glyph against the font's cached atlas texture.
    ///
    /// The atlas (built once per `NativeFont` via
    /// `NativeFont::build_rgba_atlas`) holds every glyph laid out in
    /// a single horizontal strip; alpha comes from the font's
    /// alpha-channel picture. Per-string layout uses
    /// `NativeFont::layout_quads` for the same spacing rules as the
    /// CPU-path glyph rasterization. Result: zero per-string upload —
    /// dynamic labels (counters, FPS overlay, dialogue) cost only
    /// `len(text)` quads in the GPU queue.
    pub fn render_text_argb(
        &mut self,
        font: &crate::native_font::NativeFont,
        text: &str,
        x: i32,
        y: i32,
    ) {
        if text.is_empty() || font.height() == 0 {
            return;
        }
        let font_id = (font as *const crate::native_font::NativeFont) as usize as u64;
        let atlas_bg = self.ensure_font_atlas(font_id, font);
        let tex_idx = self.queue_cached_bg(atlas_bg);
        for q in font.layout_quads(text, x, y) {
            self.frame.queued.push(QueuedDraw {
                dst: Rect {
                    x: q.dst_x,
                    y: q.dst_y,
                    w: q.dst_w as i32,
                    h: q.dst_h as i32,
                },
                corners: None,
                uv: [q.u0, q.v0, q.u1, q.v1],
                tint: [1.0, 1.0, 1.0, 1.0],
                operation: DrawOperation::Quad {
                    texture: QuadTexture::Frame(tex_idx),
                    blend: BlendMode::Blend,
                },
            });
        }
    }

    /// Render a string with a `.tfn`-backed TrueType font. Equivalent
    /// to a `TTF_RenderUNICODE_Solid` call that produces a per-string
    /// ARGB surface and blits it.
    ///
    /// `ab_glyph` doesn't ship a glyph atlas (and the .tfn font set is
    /// only used for list views, so per-string upload cost is trivial),
    /// so we rasterise into a temporary RGBA buffer sized by
    /// `font.total_pixel_height()` and upload as a one-shot wgpu
    /// texture, then queue the same blended quad the native-font path uses.
    pub fn render_text_truetype(&mut self, font: &TrueTypeFont, text: &str, x: i32, y: i32) {
        if text.is_empty() || !font.is_valid() {
            return;
        }
        let raw_w = font.get_string_width_total(text.chars().map(u32::from));
        if raw_w <= 0 {
            return;
        }
        // Pad horizontally — italic / wide glyphs occasionally extend
        // past the cumulative h_advance (especially the final glyph's
        // right side). The TTF_RenderUNICODE_Solid surface includes the
        // same overshoot.
        let overhang_pad = (text.chars().count() as u32).saturating_mul(2).min(128);
        let w = raw_w as u32 + 16 + overhang_pad;
        let h = font.total_pixel_height();
        if w == 0 || h == 0 {
            return;
        }
        let pitch = (w as usize) * 4;
        let mut rgba = vec![0u8; pitch * h as usize];
        font.render_to_rgba(&mut rgba, w as i32, h as i32, pitch, text, 0, 0);
        let (_tex, view) = upload_rgba_texture(&self.gpu, &rgba, w, h, "tt scratch");
        let tex_idx = self.queue_frame_texture(&view);
        self.frame.queued.push(QueuedDraw {
            dst: Rect {
                x,
                y,
                w: w as i32,
                h: h as i32,
            },
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 1.0],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend: BlendMode::Blend,
            },
        });
    }

    /// Get-or-build the GPU font-atlas bind group for `font`.
    /// Identity is the font's pointer — stable for the duration of
    /// the level since `Host` owns the font.
    fn ensure_font_atlas(
        &mut self,
        font_id: u64,
        font: &crate::native_font::NativeFont,
    ) -> wgpu::BindGroup {
        self.resources.ensure_font_atlas(&self.gpu, font_id, font)
    }

    /// Invalidate pointer-keyed bitmap font atlases before replacing eager
    /// font owners. Allocators may reuse an old address for a different
    /// locale's font; retaining that atlas would render stale glyphs.
    pub fn clear_font_atlas_cache(&mut self) {
        self.resources.clear_font_atlas_cache();
    }

    /// Public helper for callers that own their own wgpu textures
    /// (titbit_renderer, campaign_map background) and want to enqueue
    /// them through the renderer's draw queue. The caller is
    /// responsible for keeping the texture alive until `present()`
    /// runs at end of frame.
    pub fn enqueue_external_texture(
        &mut self,
        view: &wgpu::TextureView,
        dst: Rect,
        uv: [f32; 4],
        tint: [f32; 4],
        blend: BlendMode,
    ) {
        let tex_idx = self.queue_frame_texture(view);
        self.frame.queued.push(QueuedDraw {
            dst,
            corners: None,
            uv,
            tint,
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend,
            },
        });
    }

    /// Convenience wrapper around [`upload_rgba_texture`] for callers
    /// that build their own pixel buffers and want a texture+view they
    /// can hold across frames. The texture isn't tracked by the
    /// renderer — the caller manages its lifetime.
    pub fn create_static_rgba_texture(
        &self,
        rgba: &[u8],
        width: u32,
        height: u32,
        label: &str,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        upload_rgba_texture(&self.gpu, rgba, width, height, label)
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
            let r = ((px >> 8) & 0xF8) as u32;
            let g = ((px >> 3) & 0xFC) as u32;
            let b = ((px << 3) & 0xF8) as u32;
            r | (g << 8) | (b << 16) | 0xFF00_0000
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
            let r = ((px >> 8) & 0xF8) as u32;
            let g = ((px >> 3) & 0xFC) as u32;
            let b = ((px << 3) & 0xF8) as u32;
            r | (g << 8) | (b << 16) | 0xFF00_0000
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
pub(crate) fn upload_rgba_texture(
    gpu: &GpuContext,
    rgba: &[u8],
    width: u32,
    height: u32,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    upload_counter::inc(label);
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

/// Resolve the dst+src rect pair from `blit_to_screen`'s optional
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

/// Per-second FPS counter + per-frame draw / upload counts logged at
/// debug level. One mutex take per `present`; residency keys are scanned only
/// at reporting cadence. Run with `RUST_LOG=fps=debug`.
fn log_fps(
    draws_this_frame: usize,
    uploads_this_frame: usize,
    binds_this_frame: usize,
    draw_calls_this_frame: usize,
    resources: &GpuResources,
) {
    use std::sync::OnceLock;
    static STATE: OnceLock<std::sync::Mutex<FpsState>> = OnceLock::new();
    struct FpsState {
        frames: u32,
        draws_total: usize,
        uploads_total: usize,
        binds_total: usize,
        draw_calls_total: usize,
        last: web_time::Instant,
    }
    let m = STATE.get_or_init(|| {
        std::sync::Mutex::new(FpsState {
            frames: 0,
            draws_total: 0,
            uploads_total: 0,
            binds_total: 0,
            draw_calls_total: 0,
            last: web_time::Instant::now(),
        })
    });
    let mut g = m.lock().unwrap();
    g.frames += 1;
    g.draws_total += draws_this_frame;
    g.uploads_total += uploads_this_frame;
    g.binds_total += binds_this_frame;
    g.draw_calls_total += draw_calls_this_frame;
    if g.last.elapsed().as_secs() >= 1 {
        let atlas = resources.sprite_atlas.stats();
        let residency = resources.sprite_residency_stats();
        let avg_draws = g.draws_total / g.frames as usize;
        let avg_uploads = g.uploads_total / g.frames as usize;
        let avg_binds = g.binds_total / g.frames as usize;
        let avg_draw_calls = g.draw_calls_total / g.frames as usize;
        let (present_avg_us, _) = present_time::take_avg();
        let upload_labels = upload_counter::take_labels();
        tracing::debug!(
            target: "fps",
            sprite_cache_entries = residency.entries,
            distinct_sprite_frames = residency.distinct_frames,
            resident_sprite_bytes = residency.resident_bytes,
            atlas_packing_efficiency = atlas.packing_efficiency(),
            "{} fps  quads/f={}  drawcalls/f={}  binds/f={}  uploads/f={}  \
             present={:.2}ms  atlas={}L/{:.0}MiB/{:.0}%occ/{}spr  upload_labels={}",
            g.frames, avg_draws, avg_draw_calls, avg_binds, avg_uploads,
            present_avg_us as f32 / 1000.0,
            atlas.layers,
            atlas.bytes() as f32 / (1024.0 * 1024.0),
            atlas.occupancy() * 100.0,
            atlas.sprites,
            upload_labels,
        );
        g.frames = 0;
        g.draws_total = 0;
        g.uploads_total = 0;
        g.binds_total = 0;
        g.draw_calls_total = 0;
        g.last = web_time::Instant::now();
    }
}

/// Average per-frame `present()` wall time, summed over the FPS
/// window. Surfaced on the same log line as the FPS count.
mod present_time {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SUM_US: AtomicU64 = AtomicU64::new(0);
    static N: AtomicU64 = AtomicU64::new(0);
    pub fn record(us: u64) {
        SUM_US.fetch_add(us, Ordering::Relaxed);
        N.fetch_add(1, Ordering::Relaxed);
    }
    /// Returns `(avg_us, samples)` and resets.
    pub fn take_avg() -> (u64, u64) {
        let s = SUM_US.swap(0, Ordering::Relaxed);
        let n = N.swap(0, Ordering::Relaxed);
        let avg = s.checked_div(n).unwrap_or(0);
        (avg, n)
    }
}

fn present_time_record(us: u64) {
    present_time::record(us);
}

/// Per-frame upload counter. `upload_rgba_texture` bumps it; `present`
/// drains it into the `fps` log line so we can see the rate of
/// fresh GPU texture allocations.
mod upload_counter {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, OnceLock};

    static N: AtomicUsize = AtomicUsize::new(0);
    static LABELS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();

    pub fn inc(label: &str) {
        N.fetch_add(1, Ordering::Relaxed);
        let labels = LABELS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut labels = labels.lock().unwrap();
        *labels.entry(label.to_string()).or_default() += 1;
    }

    pub fn take_count() -> usize {
        N.swap(0, Ordering::Relaxed)
    }

    pub fn take_labels() -> String {
        let Some(labels) = LABELS.get() else {
            return "-".to_string();
        };
        let mut labels = labels.lock().unwrap();
        if labels.is_empty() {
            return "-".to_string();
        }
        let mut entries: Vec<_> = labels.drain().collect();
        entries.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        entries
            .into_iter()
            .take(6)
            .map(|(label, count)| format!("{label}:{count}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Counts `set_bind_group(1, …)` calls issued while encoding the scene
/// pass.
///
/// This is the number the sprite atlas is meant to move: before it,
/// every sprite owned a texture and so forced its own texture bind, and
/// `binds/f` tracked `draws/f` almost exactly. Packed into shared
/// layers, a run of sprites from one layer costs a single bind.
/// …and the `draw` calls actually recorded, which is not the same as
/// the number of queued quads once consecutive same-state draws are
/// coalesced into one contiguous vertex range.
mod bind_counter {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static BINDS: AtomicUsize = AtomicUsize::new(0);
    static DRAW_CALLS: AtomicUsize = AtomicUsize::new(0);

    pub fn inc() {
        BINDS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_draw_call() {
        DRAW_CALLS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn take_count() -> usize {
        BINDS.swap(0, Ordering::Relaxed)
    }

    pub fn take_draw_calls() -> usize {
        DRAW_CALLS.swap(0, Ordering::Relaxed)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn verify_offscreen_gpu_contract(gpu: GpuContext) {
    verify_mask_atlas_pixels(gpu.clone());
    let mut other_renderer =
        Renderer::with_optional_surface(gpu.clone(), None, None, 3, 2, TextureScaleMode::Nearest);
    let mut renderer =
        Renderer::with_optional_surface(gpu, None, None, 3, 2, TextureScaleMode::Nearest);
    assert_eq!(
        renderer.target_dimensions(SurfaceTarget::Screen).unwrap(),
        (3, 2)
    );
    assert_eq!(
        renderer.legacy_surface_target(0).unwrap(),
        renderer.legacy_surface_target(1).unwrap()
    );
    assert!(renderer.surface_handle(0).is_err());
    assert!(renderer.surface_handle(1).is_err());
    let local_id = renderer
        .create_surface_from_rgb565(1, 1, &[0xffff])
        .unwrap();
    let other_id = other_renderer
        .create_surface_from_rgb565(1, 1, &[0xffff])
        .unwrap();
    assert_eq!(local_id, other_id);
    let owned = renderer.adopt_surface(local_id);
    assert!(other_renderer.surface_dimensions(owned.handle()).is_err());
    assert!(
        other_renderer
            .draw_surface(owned.handle(), None, None, 0)
            .is_err()
    );
    let before_foreign_draw = other_renderer.draw_queue_checkpoint();
    assert!(
        other_renderer
            .draw_surface_alpha(owned.handle(), None, None, 25, 0)
            .is_err()
    );
    assert!(
        other_renderer
            .draw_surface_with_shadow(owned.handle(), None, None, 0, 40, BLIT_SOURCE_TRANSPARENT)
            .is_err()
    );
    assert_eq!(other_renderer.draw_queue_checkpoint(), before_foreign_draw);
    // Widget alpha mixing validates both borrowed uploads before queuing
    // either draw: a foreign/decoded input cannot partially render a fade.
    let mut alpha_widget = crate::ui::RendererAlphaConstant::default();
    assert!(!alpha_widget.render(&mut other_renderer, 0, Some(owned.handle()), None));
    assert_eq!(other_renderer.draw_queue_checkpoint(), before_foreign_draw);
    alpha_widget.mixing_in_progress = true;
    let local_other = other_renderer.surface_handle(other_id).unwrap();
    assert!(!alpha_widget.render(
        &mut other_renderer,
        0,
        Some(local_other),
        Some(owned.handle())
    ));
    assert_eq!(other_renderer.draw_queue_checkpoint(), before_foreign_draw);
    let decoded: SurfaceHandle =
        serde_json::from_value(serde_json::to_value(local_other).unwrap()).unwrap();
    assert!(!alpha_widget.render(&mut other_renderer, 0, Some(local_other), Some(decoded)));
    assert_eq!(other_renderer.draw_queue_checkpoint(), before_foreign_draw);
    assert!(other_renderer.surface_alpha_mask(owned.handle()).is_err());
    let restored: OwnedSurface =
        serde_json::from_str(&serde_json::to_string(&owned).unwrap()).unwrap();
    assert!(renderer.surface_dimensions(restored.handle()).is_err());
    assert!(renderer.try_retire_surface(restored).is_err());
    assert!(renderer.surface_dimensions(owned.handle()).is_ok());
    let (_, owned) = other_renderer.try_retire_surface(owned).unwrap_err();
    assert!(renderer.surface_dimensions(owned.handle()).is_ok());
    let mut mission = crate::mission_render_resources::MissionRenderResources::default();
    let other_owned = other_renderer.adopt_surface(other_id);
    mission.replace_map(&mut other_renderer, other_owned);
    assert!(mission.try_retire(&mut renderer).is_err());
    assert_eq!(
        mission.map(),
        Some(other_renderer.surface_handle(other_id).unwrap())
    );
    let borrowed_map = mission.map().unwrap();
    assert!(renderer.draw_surface(borrowed_map, None, None, 0).is_err());
    assert!(
        renderer
            .draw_surface_alpha(borrowed_map, None, None, 0, 0)
            .is_err()
    );
    assert!(other_renderer.surface_handle(other_id).is_ok());
    let (_, owned) = mission
        .try_replace_map(&mut other_renderer, owned)
        .unwrap_err();
    assert!(renderer.surface_dimensions(owned.handle()).is_ok());
    assert!(
        mission
            .try_replace_map(&mut other_renderer, OwnedSurface::synthetic(u32::MAX))
            .is_err()
    );
    assert_eq!(
        mission.map(),
        Some(other_renderer.surface_handle(other_id).unwrap())
    );
    assert!(renderer.try_adopt_surface(local_id).is_err());
    assert!(renderer.try_delete_legacy_surface(local_id).is_err());
    assert!(other_renderer.surface_handle(other_id).is_ok());
    renderer.retire_surface(owned);
    assert!(renderer.surface_handle(local_id).is_err());
    assert!(other_renderer.surface_handle(other_id).is_ok());
    mission.retire(&mut other_renderer);
    assert!(
        other_renderer
            .draw_surface(borrowed_map, None, None, 0)
            .is_err()
    );
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255,
        0, 255,
    ];
    let image = renderer
        .create_rgba_gpu_image(3, 2, &pixels, "capture contract")
        .unwrap();
    renderer
        .upload_mask_alphas([(7, &[2, 0, 0, 255, 0, 0][..], 3, 2)])
        .unwrap();
    let checkpoint = renderer.draw_queue_checkpoint();
    renderer.render_gpu_image(&image, None, None, BlendMode::None);
    renderer.mask_queued_draws(
        checkpoint,
        &[(7, Rect::new(0, 0, 3, 2))],
        Rect::new(0, 0, 3, 2),
    );
    // Zero-alpha overlay still exercises framebuffer snapshot/pass ordering.
    renderer.render_framebuffer_alpha_rect(Rect::new(0, 0, 3, 2), [0.0, 0.0, 1.0, 1.0], 0, 0);
    renderer.begin_ui_layer();
    renderer.render_gpu_rect(2, 0, 1, 1, 255, 255, 255, 255);
    let expected = vec![
        0, 0, 0, 255, 0, 255, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0,
        255,
    ];
    assert_eq!(
        renderer.try_capture_frame_rgba().unwrap(),
        (3, 2, expected.clone())
    );
    assert_eq!(renderer.draw_queue_checkpoint(), 0);
    renderer.freeze_scene_for_modal();
    assert_eq!(renderer.try_capture_frame_rgba().unwrap().2, expected);
    let queued_image = renderer
        .create_rgba_gpu_image(3, 2, &[255, 0, 0, 255].repeat(6), "pending live frame")
        .unwrap();
    renderer.render_gpu_image(&queued_image, None, None, BlendMode::None);
    let queued = renderer.draw_queue_checkpoint();
    let live_texture = renderer.frame.render_target_texture.clone();
    let live_backdrop = renderer.frame.frozen_scene.as_ref().unwrap().0.clone();
    let mut capture_texture = None;
    for fail in [false, true, false] {
        let result = (|| -> Result<(), CaptureError> {
            let mut target = renderer.capture_target(7, 5);
            if let Some(previous) = &capture_texture {
                assert_eq!(
                    &target.frame.render_target_texture, previous,
                    "same-size captures must reuse their target allocation"
                );
            } else {
                capture_texture = Some(target.frame.render_target_texture.clone());
            }
            assert_ne!(target.frame.render_target_texture, live_texture);
            assert!(target.frame.frozen_scene.is_none());
            let image = target
                .create_rgba_gpu_image(7, 5, &[0, 0, 255, 255].repeat(35), "capture frame")
                .unwrap();
            target.render_gpu_image(&image, None, None, BlendMode::None);
            target.begin_ui_layer();
            target.render_gpu_rect(0, 0, 1, 1, 0, 255, 0, 255);
            if fail {
                // Exercise the same early-return path as failed mapping,
                // including unsubmitted capture-only world/UI commands.
                return Err(CaptureError::CompletionLost);
            }
            let (w, h, pixels) = target.try_capture_frame_rgba()?;
            assert_eq!((w, h), (7, 5));
            assert_eq!(&pixels[..4], &[0, 255, 0, 255]);
            assert_eq!(&pixels[4..], &[0, 0, 255, 255].repeat(34));
            target.freeze_scene_for_modal();
            Ok(())
        })();
        assert_eq!(result.is_err(), fail);
        assert_eq!((renderer.screen_width(), renderer.screen_height()), (3, 2));
        assert_eq!(renderer.frame.render_target_texture, live_texture);
        assert_eq!(
            renderer.frame.frozen_scene.as_ref().unwrap().0,
            live_backdrop
        );
        assert_eq!(renderer.draw_queue_checkpoint(), queued);
        assert_eq!(
            renderer.try_capture_presented_frame_rgba().unwrap().2,
            expected
        );
    }
    assert_eq!(
        renderer.try_capture_presented_frame_rgba().unwrap().2,
        expected
    );
    assert_eq!(
        renderer.draw_queue_checkpoint(),
        queued,
        "presented capture must not consume pending commands"
    );
    assert_eq!(
        renderer.try_capture_frame_rgba().unwrap().2,
        [255, 0, 0, 255].repeat(6)
    );
    // Detaching a submitted capture must preserve the old frame, even when
    // another frame overwrites the logical target before mapping starts.
    renderer.render_gpu_rect(0, 0, 3, 2, 0, 255, 0, 255);
    let pending_capture = renderer.begin_capture_frame_rgba();
    assert_eq!(renderer.draw_queue_checkpoint(), 0);
    renderer.render_gpu_rect(0, 0, 3, 2, 0, 0, 255, 255);
    assert_eq!(
        renderer.try_capture_frame_rgba().unwrap().2,
        [0, 0, 255, 255].repeat(6)
    );
    assert_eq!(
        pollster::block_on(pending_capture).unwrap().2,
        [0, 255, 0, 255].repeat(6)
    );
    let id = renderer
        .create_surface_from_rgb565(1, 1, &[0xffff])
        .unwrap();
    let handle = renderer.surface_handle(id).unwrap();
    assert_eq!(renderer.surface_dimensions(handle).unwrap(), (1, 1));
    assert!(renderer.delete_surface(id));
    assert!(renderer.surface_dimensions(handle).is_err());
    let replacement = renderer
        .create_surface_from_rgb565(1, 1, &[0xffff])
        .unwrap();
    assert_ne!(id, replacement, "deleted surface IDs must not be reused");
    crate::mission_render_resources::verify_gpu_lifecycle(&mut renderer);
    crate::corner_hud::verify_gpu_ownership(&mut renderer);
    crate::zoom_hud::verify_gpu_ownership(&mut renderer);
    crate::stature_hud::verify_gpu_ownership(&mut renderer);
    crate::sherwood_hud::verify_gpu_ownership(&mut renderer);
    crate::main_menu::credits::verify_gpu_retirement(&mut renderer);
    let mut portrait_renderer = Renderer::with_optional_surface(
        renderer.gpu.clone(),
        None,
        None,
        3,
        2,
        TextureScaleMode::Nearest,
    );
    let mut portrait_peer = Renderer::with_optional_surface(
        renderer.gpu.clone(),
        None,
        None,
        3,
        2,
        TextureScaleMode::Nearest,
    );
    crate::ui_panel::verify_portrait_gpu_ownership(&mut portrait_renderer, &mut portrait_peer);
    let mut menu_renderer = Renderer::with_optional_surface(
        renderer.gpu.clone(),
        None,
        None,
        3,
        2,
        TextureScaleMode::Nearest,
    );
    let mut menu_peer = Renderer::with_optional_surface(
        renderer.gpu.clone(),
        None,
        None,
        3,
        2,
        TextureScaleMode::Nearest,
    );
    crate::ingame_menu::resources::verify_menu_gpu_ownership(&mut menu_renderer, &mut menu_peer);
    verify_deferred_menu_surfaces(&mut renderer);
}

#[cfg(all(test, not(target_arch = "wasm32")))]
fn verify_deferred_menu_surfaces(renderer: &mut Renderer) {
    let pixels = [
        0xf800,
        TRANSPARENT_COLOR_KEY_16,
        SHADOW_KEY,
        0x07e0,
        0x001f,
        0xffff,
    ];
    // Every rendering entry point must realize a deferred surface, including
    // the shadow override and whole-widget fade paths used by modal menus.
    for mode in 0..5 {
        let eager = renderer.create_surface_from_rgb565(3, 2, &pixels).unwrap();
        let deferred = renderer.create_deferred_surface_from_rgb565(3, 2, Box::new(pixels));
        let owned = renderer.adopt_surface(deferred);
        let handle = owned.handle();
        assert_eq!(renderer.surface_dimensions(handle).unwrap(), (3, 2));
        let mask = renderer.build_alpha_mask(deferred).unwrap();
        assert!(mask.is_opaque(0, 0));
        assert!(!mask.is_opaque(1, 0));
        assert!(
            matches!(
                renderer.resources.managed_surfaces[&deferred].pixels,
                ManagedSurfacePixels::Pending(_)
            ),
            "metadata access must not upload unopened menus"
        );
        renderer.set_shadow_alpha(eager, MENU_BUTTON_SHADOW_ALPHA);
        renderer.set_shadow_alpha(deferred, MENU_BUTTON_SHADOW_ALPHA);
        let mut captured = Vec::new();
        for id in [eager, deferred, deferred] {
            renderer.finish_loading_screen();
            renderer.render_gpu_rect(0, 0, 3, 2, 255, 255, 255, 255);
            let drawn = match mode {
                0 => renderer.blit_to_screen(id, None, None, 0),
                1 => renderer.blit_to_screen(id, None, None, BLIT_SOURCE_TRANSPARENT),
                2 => renderer.blit_to_screen_alpha(id, None, None, 35, 0),
                3 => renderer.blit_to_screen_alpha(id, None, None, 35, BLIT_SOURCE_TRANSPARENT),
                4 => renderer.blit_with_shadow(id, None, 0, None, 0, 50, BLIT_SOURCE_TRANSPARENT),
                _ => unreachable!(),
            };
            assert!(drawn);
            captured.push(renderer.try_capture_frame_rgba().unwrap());
        }
        assert_eq!(
            captured[0], captured[1],
            "deferred first draw differs for mode {mode}"
        );
        assert_eq!(
            captured[1], captured[2],
            "reopening menu differs for mode {mode}"
        );
        assert!(matches!(
            renderer.resources.managed_surfaces[&deferred].pixels,
            ManagedSurfacePixels::Resident(_)
        ));
        renderer.retire_surface(owned);
        assert!(renderer.surface_dimensions(handle).is_err());
        assert!(renderer.delete_surface(eager));
    }
    let never_opened = renderer.create_deferred_surface_from_rgb565(1, 1, Box::new([0xffff]));
    let owned = renderer.adopt_surface(never_opened);
    renderer.retire_surface(owned);
    assert!(
        !renderer
            .resources
            .managed_surfaces
            .contains_key(&never_opened)
    );

    // The loading renderer may have queued geometry, a frozen scene and a
    // UI-only presentation policy. Handoff must not carry its pixels into the
    // first mission frame, even when logical dimensions do not change.
    renderer.freeze_scene_for_modal();
    renderer.begin_ui_only_frame();
    renderer.render_gpu_rect(0, 0, 3, 2, 255, 0, 0, 255);
    let identity = renderer.identity;
    renderer.finish_loading_screen();
    assert_eq!(
        renderer.identity, identity,
        "handoff must retain renderer ownership"
    );
    assert_eq!(renderer.draw_queue_checkpoint(), 0);
    assert!(renderer.frame.frozen_scene.is_none());
    // Lost-Sherwood can open its debriefing before the first world render.
    renderer.freeze_scene_for_modal();
    assert_eq!(
        renderer.try_capture_presented_frame_rgba().unwrap().2,
        vec![0; 3 * 2 * 4],
        "handoff must not expose loading pixels before the first composition"
    );
    assert_eq!(
        renderer.try_capture_frame_rgba().unwrap().2,
        vec![0; 3 * 2 * 4],
        "an immediate modal must freeze the cleared target"
    );
    renderer.clear_frozen_scene();
    assert_eq!(
        renderer.try_capture_frame_rgba().unwrap().2,
        [0, 0, 0, 255].repeat(6)
    );

    // Reproduce normal presentation's split world/UI composition before a
    // pause snapshot. Captures normally draw the full queue and would hide
    // the regression where only the world survived in the logical target.
    renderer.render_gpu_rect(0, 0, 3, 2, 0, 255, 0, 255);
    renderer.begin_ui_layer();
    renderer.render_gpu_rect(2, 0, 1, 2, 255, 0, 0, 255);
    renderer.frame.push_implicit_base_quad();
    renderer.frame.upload_queue_geometry(&renderer.gpu);
    let mut encoder = renderer
        .gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("split UI modal regression"),
        });
    renderer
        .frame
        .encode_scene_to_rt(&mut encoder, &renderer.pipelines, &renderer.resources);
    renderer.frame.encode_ui_to_logical_frame(
        &mut encoder,
        &renderer.pipelines,
        &renderer.resources,
    );
    renderer.gpu.queue.submit(Some(encoder.finish()));
    renderer.frame.clear_recording();
    renderer.freeze_scene_for_modal();
    let expected = [0, 255, 0, 255, 0, 255, 0, 255, 255, 0, 0, 255].repeat(2);
    assert_eq!(renderer.try_capture_frame_rgba().unwrap().2, expected);
    renderer.colorize_framebuffer(210.0, 0.35);
    let tinted = renderer.try_capture_frame_rgba().unwrap().2;
    for pixel in tinted.as_chunks::<4>().0 {
        assert!(
            pixel[2] > pixel[0],
            "pause tint must include world and portrait: {pixel:?}"
        );
        assert_eq!(pixel[3], 255);
    }
    renderer.clear_frozen_scene();
}

#[cfg(all(test, not(target_arch = "wasm32")))]
fn verify_mask_atlas_pixels(gpu: GpuContext) {
    let mut renderer =
        Renderer::with_optional_surface(gpu, None, None, 31, 19, TextureScaleMode::Nearest);
    let image = renderer
        .create_rgba_gpu_image(31, 19, &[255; 31 * 19 * 4], "atlas parity")
        .unwrap();
    let filler = vec![1; 2040 * 8];
    let narrow = [0, 2, 0, 255, 1, 0, 1];
    let pattern = [0, 1, 0, 1, 0, 1];
    let masks = [
        (10, filler.as_slice(), 2040, 8),
        (11, narrow.as_slice(), 1, 7),
        (12, pattern.as_slice(), 3, 2),
    ];
    renderer.upload_mask_alphas(masks).unwrap();
    assert!(
        renderer.resources.mask_alpha_cache[&11]
            .atlas
            .unwrap()
            .origin[0]
            > 2000
    );
    assert!(renderer.upload_mask_alpha(21, &narrow, 1, 7));
    assert!(renderer.upload_mask_alpha(22, &pattern, 3, 2));
    assert!(renderer.upload_occlusion_depth(&[0, 255, 256, 65535, 32767, 32768], 3, 2));
    for depth in [None, Some((Rect::new(0, 0, 31, 19), 0.4))] {
        for (atlas_id, standalone_id) in [(11, 21), (12, 22)] {
            for uv in [
                [0.0, 0.0, 1.0, 1.0],
                [-9.0, -3.0, 5.0, 8.0],
                [0.125, 0.13, 0.91, 0.87],
            ] {
                let mut captures = Vec::new();
                for id in [standalone_id, atlas_id] {
                    renderer.render_gpu_rect(0, 0, 31, 19, 0, 0, 0, 255);
                    let checkpoint = renderer.draw_queue_checkpoint();
                    renderer.render_gpu_image(&image, None, None, BlendMode::None);
                    renderer.mask_queued_draws_impl(
                        checkpoint,
                        &[(id, Rect::new(0, 0, 31, 19))],
                        Rect::new(0, 0, 31, 19),
                        depth,
                    );
                    for draw in &mut renderer.frame.queued[checkpoint..] {
                        if matches!(draw.operation, DrawOperation::MaskAlpha(index) if index == id)
                        {
                            draw.uv = uv;
                            draw.corners =
                                Some([(0.25, 0.75), (30.5, 0.75), (0.25, 18.25), (30.5, 18.25)]);
                        }
                    }
                    captures.push(renderer.try_capture_frame_rgba().unwrap().2);
                }
                assert_eq!(captures[0], captures[1], "atlas {atlas_id} UV {uv:?}");
                assert!(captures[0].as_chunks::<4>().0.iter().any(|p| p[0] == 255));
                assert!(captures[0].as_chunks::<4>().0.iter().any(|p| p[0] == 0));
            }
        }
    }
    assert!(
        renderer.resources.mask_alpha_cache[&OCCLUSION_DEPTH_TEXTURE_INDEX]
            .atlas
            .is_none()
    );
    assert!(renderer.upload_mask_alphas([(50, &[][..], 0, 0)]).is_err());
    assert!(renderer.upload_mask_alphas([(50, &[1][..], 2, 2)]).is_err());
    let page = vec![1; 2046 * 2046];
    let oversized = vec![1; 2048];
    renderer
        .upload_mask_alphas([
            (60, page.as_slice(), 2046, 2046),
            (61, page.as_slice(), 2046, 2046),
            (62, oversized.as_slice(), 2048, 1),
        ])
        .unwrap();
    assert!(renderer.resources.mask_alpha_cache[&60].atlas.is_some());
    assert!(renderer.resources.mask_alpha_cache[&61].atlas.is_some());
    assert!(renderer.resources.mask_alpha_cache[&62].atlas.is_none());
    assert_ne!(
        renderer.resources.mask_alpha_cache[&60]._texture,
        renderer.resources.mask_alpha_cache[&61]._texture
    );
    renderer.clear_mask_alpha_cache();
    renderer.upload_mask_alphas(std::iter::empty()).unwrap();
    assert!(renderer.resources.mask_alpha_cache.is_empty());
}

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
