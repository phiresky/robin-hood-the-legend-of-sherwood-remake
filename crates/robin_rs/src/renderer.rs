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

mod frame;
mod pipelines;
mod readback;
mod resources;

use frame::FrameState;
use pipelines::PipelineStore;
use resources::GpuResources;

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

#[inline]
fn rgb8_to_rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0xF8) << 8) | ((g as u16 & 0xFC) << 3) | ((b as u16) >> 3)
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

struct CachedSprite {
    /// Held alive for the bind group's lifetime; the renderer only
    /// touches the bind group on draws.
    _texture: wgpu::Texture,
    _view: wgpu::TextureView,
    /// Cached `(texture, sampler)` bind group so per-frame draws of
    /// this sprite don't rebuild it.
    bind_group: wgpu::BindGroup,
    width: u16,
    height: u16,
}

#[derive(Default)]
struct SpriteTextureCache {
    entries: HashMap<SpriteCacheKey, CachedSprite>,
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
}

struct BackgroundTexture {
    _view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

struct ManagedSurface {
    width: u16,
    height: u16,
    _opaque_texture: wgpu::Texture,
    _opaque_view: wgpu::TextureView,
    opaque_bg: wgpu::BindGroup,
    _color_texture: wgpu::Texture,
    _color_view: wgpu::TextureView,
    color_bg: wgpu::BindGroup,
    _shadow_texture: Option<wgpu::Texture>,
    _shadow_view: Option<wgpu::TextureView>,
    shadow_bg: Option<wgpu::BindGroup>,
    alpha_mask: AlphaMask,
    shadow_alpha: u8,
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
    /// `TextureRef::ColorizeFromFrozen` repurposes this as
    /// `(hue/360, scale, _, _)` — the `fs_colorize` shader in
    /// `shaders/quad.wgsl` reads it.
    tint: [f32; 4],
    /// Texture to sample from. Solid-color draws use the renderer's
    /// 1×1 white texture (`TextureRef::White`); cached resources use
    /// `TextureRef::Frame`.
    /// `TextureRef::ColorizeFromFrozen` routes through the HSV-replace
    /// pipeline (ignores `blend`).
    tex: TextureRef,
    /// Blend mode — selects which of the 4 textured-quad pipelines
    /// to use. Ignored for `TextureRef::ColorizeFromFrozen`.
    blend: BlendMode,
}

#[derive(Clone, Copy)]
enum TextureRef {
    /// 1×1 white sampler-friendly texture for solid-color draws.
    White,
    /// Snapshot of the offscreen render target at the moment a modal
    /// menu opened — pause-menu / options / etc. dim and overlay
    /// widgets on top of this. See `freeze_scene_for_modal`.
    FrozenScene,
    /// HSV-replace colorize draw — samples the frozen-scene texture
    /// through the `fs_colorize` pipeline. Hue+scale come from the
    /// `tint` field. Used by `colorize_framebuffer` / `dim_screen`.
    ColorizeFromFrozen,
    /// Door/patch hover alpha polygon over the current composited
    /// frame. `present()` snapshots the render target before drawing
    /// this queue run, then feeds that snapshot to the RGB565 alpha
    /// shader.
    FramebufferAlpha,
    /// View-cone overlay span. Uses the white texture bind group only to
    /// satisfy the shared quad layout; `fs_view_cone_gradient` reads the
    /// interpolated alpha from `uv.x` and the alert colour from `tint.rgb`.
    ViewConeGradient,
    /// One of the per-frame texture views queued by sprite / surface /
    /// mask draws — index into `Renderer::frame_textures`.
    Frame(u32),
    /// Regular textured quad rejected wherever stencil is non-zero.
    MaskedFrame(u32),
    /// Static building mask rasterized into stencil with reference one.
    MaskAlpha(u32),
    /// Solid quad that resets the sprite's stencil region to zero.
    StencilClear,
    /// Loading-screen initial/final/mask dissolve. Bind group at
    /// `frame_texture_bgs[idx]` uses `bgl_loading_dissolve`.
    LoadingDissolveFrame(u32),
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
    /// Shared GPU context (device/queue/surface format).
    pub(crate) gpu: GpuContext,
    resources: GpuResources,
    pipelines: PipelineStore,
    frame: FrameState,
    /// Update-owned zoom HUD data. Kept separate from GPU ownership because
    /// throwaway screenshot and thumbnail passes must not advance it.
    zoom_presentation: ZoomPresentationState,
}

struct FontAtlas {
    _texture: wgpu::Texture,
    _view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
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
        label: Some("cxx alpha source"),
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
    let bind_group = make_tex_bg(device, layout, &view, sampler, "cxx alpha source bg");
    (texture, view, bind_group)
}

fn mark_draws_stencil_tested(draws: &mut [QueuedDraw]) {
    for draw in draws {
        draw.tex = match draw.tex {
            TextureRef::Frame(index) => TextureRef::MaskedFrame(index),
            TextureRef::MaskedFrame(index) => TextureRef::MaskedFrame(index),
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
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("quad sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
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
        let resources = GpuResources::new(sampler, bgl_tex, white_view, white_bg);
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
            gpu,
            resources,
            pipelines,
            frame,
            zoom_presentation: ZoomPresentationState::default(),
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

    pub fn set_shader_frame_count(&mut self, frame_count: Option<usize>) {
        self.frame.set_shader_frame_count(frame_count);
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
        rgb8_to_rgb565(r, g, b)
    }

    /// Create a managed RGB565 surface from decoded asset pixels.
    /// This is the compatibility entry point for older widget/minimap
    /// surfaces that still need a renderer surface ID; callers should not
    /// mutate these surfaces after creation.
    pub fn create_surface_from_rgb565(
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

        let opaque_rgba = rgb565_to_rgba_opaque(pixels, w, h);
        let (color_rgba, shadow_rgba, has_shadow) =
            rgb565_to_color_shadow_rgba(pixels, TRANSPARENT_COLOR_KEY_16);
        let alpha_mask = AlphaMask::from_pixels(
            width,
            height,
            width as u32,
            pixels,
            TRANSPARENT_COLOR_KEY_16,
        );

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
        let (shadow_texture, shadow_view, shadow_bg) = if has_shadow {
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

        Some(ManagedSurface {
            width,
            height,
            _opaque_texture: opaque_texture,
            _opaque_view: opaque_view,
            opaque_bg,
            _color_texture: color_texture,
            _color_view: color_view,
            color_bg,
            _shadow_texture: shadow_texture,
            _shadow_view: shadow_view,
            shadow_bg,
            alpha_mask,
            shadow_alpha,
        })
    }

    pub fn delete_surface(&mut self, id: u32) -> bool {
        self.resources.delete_managed_surface(id)
    }

    pub fn surface_width(&self, id: u32) -> u16 {
        self.resources
            .surface_dimensions(id)
            .map_or(0, |(width, _)| width)
    }

    pub fn surface_height(&self, id: u32) -> u16 {
        self.resources
            .surface_dimensions(id)
            .map_or(0, |(_, height)| height)
    }

    /// Build an `AlphaMask` from a managed surface — one bit per pixel,
    /// flagging non-transparent (`pixel != color_key`) pixels. Used by
    /// the UI hit-test path (`RendererBase::is_real_point`) so widget
    /// clicks on visually-transparent corners of round/non-rectangular
    /// sprites get rejected via a viewport pixel sample.
    pub fn build_alpha_mask(&self, id: u32) -> Option<AlphaMask> {
        self.resources.alpha_mask(id)
    }

    /// Override the shadow alpha baked into `SHADOW_KEY` pixels at
    /// upload time. Set to `MENU_BUTTON_SHADOW_ALPHA` (50%) for
    /// menu-button packs, leave at the default `DEFAULT_SHADOW_ALPHA`
    /// (40%) for everything else.
    pub fn set_shadow_alpha(&mut self, id: u32, shadow_alpha: u8) {
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

    pub fn render_loading_dissolve(
        &mut self,
        textures: &crate::loading_dissolve_gpu::LoadingDissolveTextures,
        threshold: u32,
    ) {
        if textures.width == 0 || textures.height == 0 {
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
            dst: Rect {
                x: 0,
                y: 0,
                w: textures.width as i32,
                h: textures.height as i32,
            },
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            // The shader compares `height > threshold`; threshold can be 256
            // at progress 0, so keep the normalized value slightly above 1.0.
            tint: [threshold as f32 / 255.0, 0.0, 0.0, 1.0],
            tex: TextureRef::LoadingDissolveFrame(frame_idx),
            blend: BlendMode::None,
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
            tex: TextureRef::Frame(tex_idx),
            blend,
        });
    }

    /// Enter the GPU overlay phase. The old renderer uploaded CPU
    /// framebuffer pixels here; the current renderer draws the level
    /// background and all overlays as GPU quads.
    pub fn flush_base_layer(&mut self) {
        self.frame.enter_gpu_phase();
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
    /// rebinding the texture group per `TextureRef`.
    pub fn present(&mut self) {
        self.frame
            .present(&self.gpu, &mut self.pipelines, &self.resources);
    }

    pub fn configure_surface_size(&mut self, width: u32, height: u32) {
        self.frame.configure_surface_size(&self.gpu, width, height);
    }

    /// Cross the flush boundary and present in one shot. Loading/menu screens
    /// queue their own GPU draws before calling this.
    pub fn flip(&mut self) {
        self.flush_base_layer();
        self.present();
    }

    /// Resize the renderer's logical resolution. Window-size changes
    /// don't call this — the swapchain owns its own size and the
    /// letterbox in `present()` adapts. Only call this when the game
    /// genuinely wants to render at a new logical resolution (the
    /// graphics-options menu, for example).
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
    pub fn blit_with_shadow(
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
            src_surface.color_bg.clone(),
            src_surface.shadow_bg.clone(),
            shadow_alpha,
            dst,
            uv,
            1.0,
        );
        true
    }

    /// Submit a managed surface as a GPU overlay quad. Lazy-uploads
    /// the surface to a wgpu texture (cached, invalidated on surface
    /// mutation) and queues a textured-quad draw at `dst_rect`.
    pub fn blit_to_screen(
        &mut self,
        src_id: u32,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        flags: u32,
    ) -> bool {
        let id = self.resolve_id(src_id);
        let transparent = flags & BLIT_SOURCE_TRANSPARENT != 0;
        let Some(surface) = self.resources.managed_surfaces.get(&id) else {
            return false;
        };
        let (sw, sh) = (surface.width as f32, surface.height as f32);
        let (sub_dst, sub_uv) = src_dst_uv(src_rect, dst_rect, sw, sh);
        if transparent {
            self.queue_transparent_managed_bgs(
                surface.color_bg.clone(),
                surface.shadow_bg.clone(),
                surface.shadow_alpha,
                sub_dst,
                sub_uv,
                1.0,
            );
        } else {
            let tex_idx = self.queue_cached_bg(surface.opaque_bg.clone());
            self.frame.queued.push(QueuedDraw {
                dst: sub_dst,
                corners: None,
                uv: sub_uv,
                tint: [1.0, 1.0, 1.0, 1.0],
                tex: TextureRef::Frame(tex_idx),
                blend: BlendMode::Blend,
            });
        }
        true
    }

    /// `blit_to_screen` with a per-frame alpha applied to the whole
    /// quad (used by the fade-in / fade-out transitions).
    pub fn blit_to_screen_alpha(
        &mut self,
        src_id: u32,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        alpha_level: u16,
        flags: u32,
    ) -> bool {
        let id = self.resolve_id(src_id);
        let transparent = flags & BLIT_SOURCE_TRANSPARENT != 0;
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
                surface.color_bg.clone(),
                surface.shadow_bg.clone(),
                surface.shadow_alpha,
                sub_dst,
                sub_uv,
                alpha,
            );
        } else {
            let tex_idx = self.queue_cached_bg(surface.opaque_bg.clone());
            self.frame.queued.push(QueuedDraw {
                dst: sub_dst,
                corners: None,
                uv: sub_uv,
                tint: [1.0, 1.0, 1.0, alpha],
                tex: TextureRef::Frame(tex_idx),
                blend: BlendMode::Blend,
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
                tex: TextureRef::Frame(tex_idx),
                blend: BlendMode::Blend,
            });
        }
        let tex_idx = self.queue_cached_bg(color_bg);
        self.frame.queued.push(QueuedDraw {
            dst,
            corners: None,
            uv,
            tint: [1.0, 1.0, 1.0, opacity.clamp(0.0, 1.0)],
            tex: TextureRef::Frame(tex_idx),
            blend: BlendMode::Blend,
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
                tex: TextureRef::White,
                blend: BlendMode::None,
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
                tex: TextureRef::White,
                blend: BlendMode::None,
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
            tex: TextureRef::White,
            blend: BlendMode::None,
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_gpu_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: u8, g: u8, b: u8, a: u8) {
        self.frame.queued.push(QueuedDraw {
            dst: Rect { x, y, w, h },
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: Color::rgba(r, g, b, a).to_f32_srgb(),
            tex: TextureRef::White,
            blend: BlendMode::Blend,
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
            tex: TextureRef::White,
            blend: BlendMode::Blend,
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
            tex: TextureRef::ColorizeFromFrozen,
            blend: BlendMode::None,
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
        assert!(
            checkpoint <= self.frame.queued.len(),
            "sprite mask checkpoint {checkpoint} exceeds draw queue length {}",
            self.frame.queued.len()
        );
        if masks.is_empty() {
            return;
        }
        assert!(
            checkpoint < self.frame.queued.len(),
            "sprite masks require at least one queued draw"
        );

        let mut stencil_draws = Vec::with_capacity(masks.len());
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
                tint: [1.0; 4],
                tex: TextureRef::MaskAlpha(mask_index),
                blend: BlendMode::None,
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
            tint: [1.0; 4],
            tex: TextureRef::StencilClear,
            blend: BlendMode::None,
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
        let bg = match self.resources.sprite_cache.entries.get(&key) {
            Some(c) => c.bind_group.clone(),
            None => return false,
        };
        let tex_idx = self.queue_cached_bg(bg);
        self.frame.queued.push(QueuedDraw {
            dst: dst_rect,
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 1.0],
            tex: TextureRef::Frame(tex_idx),
            blend: BlendMode::Blend,
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
        let bg = match self.resources.sprite_cache.entries.get(&key) {
            Some(c) => c.bind_group.clone(),
            None => return false,
        };
        let tex_idx = self.queue_cached_bg(bg);
        self.frame.queued.push(QueuedDraw {
            dst: dst_rect,
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, alpha as f32 / 255.0],
            tex: TextureRef::Frame(tex_idx),
            blend: BlendMode::Blend,
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
        let bg = match self.resources.sprite_cache.entries.get(&key) {
            Some(c) => c.bind_group.clone(),
            None => return false,
        };
        let tex_idx = self.queue_cached_bg(bg);
        self.frame.queued.push(QueuedDraw {
            dst: dst_rect,
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: [
                rgb.0 as f32 / 255.0,
                rgb.1 as f32 / 255.0,
                rgb.2 as f32 / 255.0,
                alpha as f32 / 255.0,
            ],
            tex: TextureRef::Frame(tex_idx),
            blend: BlendMode::Blend,
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

        let left = mask_rect.left().max(sprite_rect.left()).max(0);
        let top = mask_rect.top().max(sprite_rect.top()).max(0);
        let right = mask_rect
            .right()
            .min(sprite_rect.right())
            .min(self.frame.width as i32);
        let bottom = mask_rect
            .bottom()
            .min(sprite_rect.bottom())
            .min(self.frame.height as i32);
        if left >= right || top >= bottom {
            return true;
        }

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
            tex: TextureRef::Frame(tex_idx),
            blend: BlendMode::Blend,
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
            tex: TextureRef::Frame(tex_idx),
            blend: BlendMode::Blend,
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
            tex: TextureRef::FramebufferAlpha,
            blend: BlendMode::None,
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
            tex: TextureRef::ViewConeGradient,
            blend: BlendMode::Blend,
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
    /// CPU-path `render_to_argb`. Result: zero per-string upload —
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
                tex: TextureRef::Frame(tex_idx),
                blend: BlendMode::Blend,
            });
        }
    }

    /// Render a string with a `.tfn`-backed TrueType font. Equivalent
    /// to a `TTF_RenderUNICODE_Solid` call that produces a per-string
    /// ARGB surface and blits it.
    ///
    /// `ab_glyph` doesn't ship a glyph atlas (and the .tfn font set is
    /// only used for list views, so per-string upload cost is trivial),
    /// so we rasterise into a temporary ARGB buffer sized by
    /// `font.total_pixel_height()` and upload as a one-shot wgpu
    /// texture, then queue the same blended quad the native-font path uses.
    pub fn render_text_truetype(&mut self, font: &TrueTypeFont, text: &str, x: i32, y: i32) {
        if text.is_empty() || !font.is_valid() {
            return;
        }
        let chars: Vec<u32> = text.chars().map(|c| c as u32).collect();
        let raw_w = font.get_string_width_total(&chars);
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
        let mut argb = vec![0u8; pitch * h as usize];
        font.render_to_argb(&mut argb, w as i32, h as i32, pitch, text, 0, 0);

        // ARGB8888 LE = [B, G, R, A] in memory → [R, G, B, A] for wgpu.
        let mut rgba = Vec::with_capacity(argb.len());
        for px in argb.as_chunks::<4>().0 {
            rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
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
            tex: TextureRef::Frame(tex_idx),
            blend: BlendMode::Blend,
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
            tex: TextureRef::Frame(tex_idx),
            blend,
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

    /// Capture the next-to-be-presented composite frame as RGBA8.
    ///
    /// Executes the queued draws against the offscreen render target
    /// and reads it back via `copy_texture_to_buffer` + `map_async`.
    /// The queue is consumed (cleared like `present()` does) so the
    /// next live render starts fresh; the swapchain is untouched.
    /// Used by the `/screenshot` HTTP endpoint, the `PrintScreen`
    /// hotkey path, and the savegame thumbnail.
    pub fn capture_frame_rgba(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        readback::capture_frame_rgba(&self.gpu, &self.pipelines, &self.resources, &mut self.frame)
    }

    /// No-op because wgpu has no target stack: `capture_frame_rgba` already
    /// clears the queue and the next `present()` re-clears the render target.
    pub fn reset_render_target(&mut self) {}
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
) -> Vec<u8> {
    if let Some(rgba) = frame_holder.rgba_data(bank_id) {
        return rgba.to_vec();
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
    rgb565_to_rgba_with_key(
        &rgb565,
        w as usize,
        h as usize,
        TRANSPARENT_COLOR_KEY_16,
        shadow_alpha,
        Some(shadow_color),
    )
}

/// Build the outside-edge outline texture: transparent surface, two
/// coloured pixels outside each horizontal opaque run. Shadow pixels are
/// excluded from the body edge, matching `GenerateEdgeMap`.
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

fn rgb565_to_color_shadow_rgba(src: &[u16], color_key: u16) -> (Vec<u8>, Vec<u8>, bool) {
    let mut color = Vec::with_capacity(src.len() * 4);
    let mut shadow = Vec::with_capacity(src.len() * 4);
    let mut has_shadow = false;
    for &px in src {
        if px == color_key {
            color.extend_from_slice(&[0, 0, 0, 0]);
            shadow.extend_from_slice(&[0, 0, 0, 0]);
        } else if px == SHADOW_KEY {
            color.extend_from_slice(&[0, 0, 0, 0]);
            shadow.extend_from_slice(&[0, 0, 0, 255]);
            has_shadow = true;
        } else {
            let (r, g, b) = rgb565_to_rgb8(px);
            color.extend_from_slice(&[r, g, b, 255]);
            shadow.extend_from_slice(&[0, 0, 0, 0]);
        }
    }
    (color, shadow, has_shadow)
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

/// Clip `dst` against `clip` and compute the matching uv sub-rect
/// (assuming the original uv is `[0,0,1,1]` over the full `dst`).
/// Returns `None` if fully clipped away.
fn clip_dst_to_uv(dst: Rect, clip: Rect) -> Option<(Rect, [f32; 4])> {
    let x0 = dst.x.max(clip.x);
    let y0 = dst.y.max(clip.y);
    let x1 = (dst.x + dst.w).min(clip.x + clip.w);
    let y1 = (dst.y + dst.h).min(clip.y + clip.h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let dw = dst.w.max(1) as f32;
    let dh = dst.h.max(1) as f32;
    let u0 = (x0 - dst.x) as f32 / dw;
    let v0 = (y0 - dst.y) as f32 / dh;
    let u1 = (x1 - dst.x) as f32 / dw;
    let v1 = (y1 - dst.y) as f32 / dh;
    Some((
        Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        },
        [u0, v0, u1, v1],
    ))
}

/// Per-second FPS counter + per-frame draw / upload counts logged at
/// info level. Cheap — one mutex take per `present`. Run with
/// `RUST_LOG=fps=info`.
fn log_fps(draws_this_frame: usize, uploads_this_frame: usize) {
    use std::sync::OnceLock;
    static STATE: OnceLock<std::sync::Mutex<FpsState>> = OnceLock::new();
    struct FpsState {
        frames: u32,
        draws_total: usize,
        uploads_total: usize,
        last: web_time::Instant,
    }
    let m = STATE.get_or_init(|| {
        std::sync::Mutex::new(FpsState {
            frames: 0,
            draws_total: 0,
            uploads_total: 0,
            last: web_time::Instant::now(),
        })
    });
    let mut g = m.lock().unwrap();
    g.frames += 1;
    g.draws_total += draws_this_frame;
    g.uploads_total += uploads_this_frame;
    if g.last.elapsed().as_secs() >= 1 {
        let avg_draws = g.draws_total / g.frames as usize;
        let avg_uploads = g.uploads_total / g.frames as usize;
        let (present_avg_us, _) = present_time::take_avg();
        let upload_labels = upload_counter::take_labels();
        tracing::debug!(
            target: "fps",
            "{} fps  draws/f={}  uploads/f={}  present={:.2}ms  upload_labels={}",
            g.frames, avg_draws, avg_uploads,
            present_avg_us as f32 / 1000.0,
            upload_labels,
        );
        g.frames = 0;
        g.draws_total = 0;
        g.uploads_total = 0;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn queued_frame(index: u32) -> QueuedDraw {
        QueuedDraw {
            dst: Rect::new(0, 0, 1, 1),
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0; 4],
            tex: TextureRef::Frame(index),
            blend: BlendMode::Blend,
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

        assert!(matches!(draws[0].tex, TextureRef::MaskedFrame(7)));
        assert!(matches!(draws[1].tex, TextureRef::MaskedFrame(11)));
    }

    #[test]
    #[should_panic(expected = "non-textured draw queued inside a sprite mask region")]
    fn sprite_mask_rejects_unexpected_draw_kinds() {
        let mut draw = queued_frame(7);
        draw.tex = TextureRef::White;
        mark_draws_stencil_tested(std::slice::from_mut(&mut draw));
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

        assert_eq!(uploaded, rgba);
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
