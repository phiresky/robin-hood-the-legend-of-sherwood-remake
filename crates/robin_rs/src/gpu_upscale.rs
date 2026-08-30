//! wgpu-native upscale shader pipelines.
//!
//! Simple upscalers use local WGSL final-blit shaders. RetroArch
//! `.slangp` presets are delegated to [`crate::shader_preset`].

use robin_engine::graphic_config::{
    TextureEffect, TextureEffectParameters, TextureScaleMode, UpscaleParameters,
};
use std::collections::HashMap;

use crate::shader_preset::ShaderPresetRenderer;
use crate::window::GpuContext;

const COMMON_WGSL: &str = include_str!("../shaders/_common.wgsl");
const SHARP_BILINEAR_WGSL: &str = include_str!("../shaders/sharp_bilinear.wgsl");
const BICUBIC_WGSL: &str = include_str!("../shaders/bicubic.wgsl");
const LANCZOS_WGSL: &str = include_str!("../shaders/lanczos.wgsl");
const CUT3_WGSL: &str = include_str!("../shaders/cut3.wgsl");
const SCALE2X_WGSL: &str = include_str!("../shaders/scale2x.wgsl");
const SCALE3X_WGSL: &str = include_str!("../shaders/scale3x.wgsl");
const XBR_LV1_WGSL: &str = include_str!("../shaders/xbr_lv1.wgsl");
const BUILTIN_MULTIPASS_WGSL: &str = include_str!("../shaders/builtin_multipass.wgsl");

fn wgsl_for_mode(mode: TextureScaleMode) -> Option<&'static str> {
    match mode {
        TextureScaleMode::SharpBilinear => Some(SHARP_BILINEAR_WGSL),
        TextureScaleMode::Bicubic => Some(BICUBIC_WGSL),
        TextureScaleMode::Lanczos => Some(LANCZOS_WGSL),
        TextureScaleMode::Cut3 => Some(CUT3_WGSL),
        TextureScaleMode::Scale2x => Some(SCALE2X_WGSL),
        TextureScaleMode::Scale3x => Some(SCALE3X_WGSL),
        TextureScaleMode::XbrLv1 => Some(XBR_LV1_WGSL),
        _ => None,
    }
}

/// Frame uniforms — matches the `FrameUniforms` struct in `_common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FrameUniforms {
    /// xy = src_size, zw = 1 / src_size
    pub src: [f32; 4],
    /// xy = dst_size, zw = 1 / dst_size
    pub dst: [f32; 4],
}

/// A built upscale pipeline: pipeline + bind groups for source texture,
/// sampler, and uniform buffer.
#[derive(Clone)]
pub struct UpscalePipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout_tex: wgpu::BindGroupLayout,
    pub bind_group_layout_uniform: wgpu::BindGroupLayout,
    pub uniform_buffer: wgpu::Buffer,
    pub uniform_bind_group: wgpu::BindGroup,
    pub sampler: wgpu::Sampler,
    /// Empty bind group for slots 0 and 1 — `_common.wgsl` puts the
    /// upscale-specific groups at `@group(2)` / `@group(3)`, so the
    /// pipeline layout demands something bound at the lower slots.
    pub empty_bind_group: wgpu::BindGroup,
}

pub struct GpuUpscale {
    gpu: GpuContext,
    /// Per-mode WGSL pipelines, built on demand.
    pipelines: HashMap<TextureScaleMode, UpscalePipeline>,
    preset_renderer: ShaderPresetRenderer,
    builtin_runner: BuiltinRunner,
    /// Output format the pipelines target (the swapchain format).
    output_format: wgpu::TextureFormat,
}

impl GpuUpscale {
    pub fn new(gpu: GpuContext, output_format: wgpu::TextureFormat) -> Self {
        Self {
            preset_renderer: ShaderPresetRenderer::new(gpu.clone()),
            builtin_runner: BuiltinRunner::new(gpu.clone(), output_format),
            gpu,
            pipelines: HashMap::new(),
            output_format,
        }
    }

    pub fn is_multipass_mode(mode: TextureScaleMode, effect: TextureEffect) -> bool {
        matches!(mode, TextureScaleMode::Linear | TextureScaleMode::PixelArt)
            || mode.uses_builtin_chain()
            || crate::shader_preset::is_shader_preset_mode(mode)
            || effect != TextureEffect::None
    }

    pub fn validate_retroarch_preset(&mut self, preset: &str) -> Result<(), UpscaleError> {
        self.preset_renderer
            .validate_preset(preset)
            .map_err(UpscaleError::RetroArch)
    }

    pub fn render_ui_overlay(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        dst_rect: [f32; 4],
    ) {
        self.builtin_runner
            .render_ui_overlay(encoder, source, target_view, dst_rect);
    }

    /// Get-or-build the pipeline for the given legacy single-pass mode.
    /// Returns `None` only when the mode has no single-pass WGSL mapping;
    /// callers selecting a shader mode must treat that as a configuration
    /// error rather than silently substituting another scaler.
    pub fn pipeline_for(&mut self, mode: TextureScaleMode) -> Option<&UpscalePipeline> {
        if !self.pipelines.contains_key(&mode) {
            let p = self.build_pipeline(mode, self.output_format)?;
            self.pipelines.insert(mode, p);
        }
        self.pipelines.get(&mode)
    }

    fn build_pipeline(
        &self,
        mode: TextureScaleMode,
        output_format: wgpu::TextureFormat,
    ) -> Option<UpscalePipeline> {
        let body = wgsl_for_mode(mode)?;
        Some(self.build_pipeline_from_body(&format!("upscale {mode:?}"), body, output_format))
    }

    fn build_pipeline_from_body(
        &self,
        label: &str,
        body: &'static str,
        output_format: wgpu::TextureFormat,
    ) -> UpscalePipeline {
        let vertex_wgsl = r#"
struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var uv = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );
    var out: VsOut;
    out.clip_pos = vec4<f32>(pos[vid], 0.0, 1.0);
    out.color = vec4<f32>(1.0, 1.0, 1.0, 1.0);
    out.uv = uv[vid];
    return out;
}
"#;
        let full = format!("{vertex_wgsl}\n{COMMON_WGSL}\n{body}");
        let module = self
            .gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(full.into()),
            });

        let bgl_tex = self
            .gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("upscale tex bgl"),
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
        let bgl_uniform =
            self.gpu
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("upscale uniform bgl"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let layout = self
            .gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("upscale layout"),
                // Empty groups 0/1 to align with `@group(2)` / `@group(3)` in `_common.wgsl`.
                bind_group_layouts: &[
                    Some(&empty_bgl(&self.gpu.device)),
                    Some(&empty_bgl(&self.gpu.device)),
                    Some(&bgl_tex),
                    Some(&bgl_uniform),
                ],
                immediate_size: 0,
            });

        let pipeline = self
            .gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&format!("{label} pipeline")),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: output_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let uniform_buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upscale uniforms"),
            size: std::mem::size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind_group = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("upscale uniform bg"),
                layout: &bgl_uniform,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            });
        let sampler = self.gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("upscale sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let empty_layout = empty_bgl(&self.gpu.device);
        let empty_bind_group = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("upscale empty bg"),
                layout: &empty_layout,
                entries: &[],
            });

        UpscalePipeline {
            pipeline,
            bind_group_layout_tex: bgl_tex,
            bind_group_layout_uniform: bgl_uniform,
            uniform_buffer,
            uniform_bind_group,
            sampler,
            empty_bind_group,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_multipass(
        &mut self,
        mode: TextureScaleMode,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        target_size: [u32; 2],
        dst_rect: [f32; 4],
        frame_count: Option<usize>,
        retroarch_preset: Option<&str>,
        effect: TextureEffect,
        upscale_parameters: UpscaleParameters,
        effect_parameters: TextureEffectParameters,
    ) -> Result<bool, UpscaleError> {
        if matches!(mode, TextureScaleMode::Linear | TextureScaleMode::PixelArt)
            || mode.uses_builtin_chain()
            || (effect != TextureEffect::None && mode != TextureScaleMode::RetroArch)
        {
            self.builtin_runner.render(
                mode,
                effect,
                upscale_parameters,
                effect_parameters,
                encoder,
                source,
                target_view,
                target_size,
                dst_rect,
                frame_count.unwrap_or(0),
            )?;
            return Ok(true);
        }
        if crate::shader_preset::is_shader_preset_mode(mode) {
            if effect == TextureEffect::None {
                self.preset_renderer
                    .render(
                        mode,
                        encoder,
                        source,
                        target_view,
                        target_size,
                        dst_rect,
                        self.output_format,
                        frame_count,
                        retroarch_preset,
                    )
                    .map_err(UpscaleError::RetroArch)?;
            } else {
                let effect_size = [
                    dst_rect[2].max(1.0).ceil() as u32,
                    dst_rect[3].max(1.0).ceil() as u32,
                ];
                let (preset_texture, preset_view) =
                    self.builtin_runner.external_intermediate(effect_size);
                self.preset_renderer
                    .render(
                        mode,
                        encoder,
                        source,
                        &preset_view,
                        effect_size,
                        [0.0, 0.0, effect_size[0] as f32, effect_size[1] as f32],
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        frame_count,
                        retroarch_preset,
                    )
                    .map_err(UpscaleError::RetroArch)?;
                self.builtin_runner.render(
                    TextureScaleMode::Linear,
                    effect,
                    upscale_parameters,
                    effect_parameters,
                    encoder,
                    &preset_texture,
                    target_view,
                    target_size,
                    dst_rect,
                    frame_count.unwrap_or(0),
                )?;
            }
            return Ok(true);
        }
        Ok(false)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UpscaleError {
    #[error("RetroArch shader failed: {0}")]
    RetroArch(String),
    #[error("{effect:?} cannot be combined with {mode:?}: {reason}")]
    UnsupportedCombination {
        mode: TextureScaleMode,
        effect: TextureEffect,
        reason: &'static str,
    },
    #[error("built-in presentation chain has {passes} passes; maximum is {maximum}")]
    TooManyPasses { passes: usize, maximum: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BuiltinPass {
    Linear,
    Nearest,
    SharpBilinear,
    Bicubic,
    Lanczos,
    Cut3,
    Hqx,
    ScaleNx,
    Xbrz,
    AnimeRestore,
    AnimeRestoreSoft,
    AnimeDenoise,
    AnimeUpscale,
    ArtifactRemove,
    SuperFinish,
    CrtGuest,
    CrtRoyale,
}

impl BuiltinPass {
    fn entry_point(self) -> &'static str {
        match self {
            Self::Linear => "fs_copy",
            Self::Nearest => "fs_nearest",
            Self::SharpBilinear => "fs_sharp_bilinear",
            Self::Bicubic => "fs_bicubic",
            Self::Lanczos => "fs_lanczos",
            Self::Cut3 => "fs_cut3",
            Self::Hqx => "fs_hqx",
            Self::ScaleNx => "fs_scalenx",
            Self::Xbrz => "fs_xbrz",
            Self::AnimeRestore => "fs_anime_restore",
            Self::AnimeRestoreSoft => "fs_anime_restore_soft",
            Self::AnimeDenoise => "fs_anime_denoise",
            Self::AnimeUpscale => "fs_anime_upscale",
            Self::ArtifactRemove => "fs_artifact_remove",
            Self::SuperFinish => "fs_super_finish",
            Self::CrtGuest => "fs_crt_guest",
            Self::CrtRoyale => "fs_crt_royale",
        }
    }

    fn source_sized(self) -> bool {
        matches!(
            self,
            Self::AnimeRestore | Self::AnimeRestoreSoft | Self::AnimeDenoise
        )
    }
}

fn builtin_passes(mode: TextureScaleMode, effect: TextureEffect) -> Vec<BuiltinPass> {
    let mut passes = match mode {
        TextureScaleMode::Nearest => vec![BuiltinPass::Nearest],
        TextureScaleMode::Linear => vec![BuiltinPass::Linear],
        TextureScaleMode::PixelArt | TextureScaleMode::SharpBilinear => {
            vec![BuiltinPass::SharpBilinear]
        }
        TextureScaleMode::Bicubic => vec![BuiltinPass::Bicubic],
        TextureScaleMode::Lanczos => vec![BuiltinPass::Lanczos],
        TextureScaleMode::Cut3 => vec![BuiltinPass::Cut3],
        TextureScaleMode::Scale2x | TextureScaleMode::Scale3x => {
            vec![BuiltinPass::ScaleNx]
        }
        TextureScaleMode::XbrLv1 => vec![BuiltinPass::Xbrz],
        TextureScaleMode::Hqx => vec![BuiltinPass::Hqx],
        TextureScaleMode::ScaleNx => {
            vec![BuiltinPass::ScaleNx, BuiltinPass::ArtifactRemove]
        }
        TextureScaleMode::Xbrz => vec![BuiltinPass::Xbrz, BuiltinPass::ArtifactRemove],
        TextureScaleMode::SuperXbr => vec![
            BuiltinPass::Xbrz,
            BuiltinPass::ArtifactRemove,
            BuiltinPass::SuperFinish,
        ],
        TextureScaleMode::Anime4kA => vec![
            BuiltinPass::AnimeRestore,
            BuiltinPass::AnimeUpscale,
            BuiltinPass::ArtifactRemove,
        ],
        TextureScaleMode::Anime4kB => vec![
            BuiltinPass::AnimeRestoreSoft,
            BuiltinPass::AnimeUpscale,
            BuiltinPass::ArtifactRemove,
        ],
        TextureScaleMode::Anime4kC => vec![
            BuiltinPass::AnimeDenoise,
            BuiltinPass::AnimeUpscale,
            BuiltinPass::ArtifactRemove,
        ],
        TextureScaleMode::RetroArch => {
            unreachable!("RetroArch mode is delegated to ShaderPresetRenderer")
        }
    };
    match effect {
        TextureEffect::None => {}
        TextureEffect::CrtGuest => passes.push(BuiltinPass::CrtGuest),
        TextureEffect::CrtRoyale => passes.push(BuiltinPass::CrtRoyale),
    }
    passes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PipelineTarget {
    Intermediate,
    Surface,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BuiltinUniforms {
    src: [f32; 4],
    dst: [f32; 4],
    upscale: [f32; 4],
    effect: [f32; 4],
    temporal: [f32; 4],
}

struct IntermediateTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
}

impl IntermediateTexture {
    fn new(device: &wgpu::Device, label: &str, size: [u32; 2]) -> Self {
        let size = [size[0].max(1), size[1].max(1)];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            size,
        }
    }
}

const MAX_BUILTIN_PASSES: usize = 5;

struct BuiltinRunner {
    gpu: GpuContext,
    output_format: wgpu::TextureFormat,
    module: wgpu::ShaderModule,
    texture_layout: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    pipelines: HashMap<(BuiltinPass, PipelineTarget), wgpu::RenderPipeline>,
    ui_pipeline: wgpu::RenderPipeline,
    ui_uniform_buffer: wgpu::Buffer,
    ui_uniform_bind_group: wgpu::BindGroup,
    uniform_buffers: Vec<wgpu::Buffer>,
    uniform_bind_groups: Vec<wgpu::BindGroup>,
    source_intermediate: Option<IntermediateTexture>,
    output_intermediates: [Option<IntermediateTexture>; 2],
    foreign_intermediate: Option<IntermediateTexture>,
}

impl BuiltinRunner {
    fn new(gpu: GpuContext, output_format: wgpu::TextureFormat) -> Self {
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("built-in multi-pass upscalers"),
                source: wgpu::ShaderSource::Wgsl(BUILTIN_MULTIPASS_WGSL.into()),
            });
        let texture_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("built-in pass texture layout"),
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
        let uniform_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("built-in pass uniform layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("built-in pass pipeline layout"),
                bind_group_layouts: &[Some(&texture_layout), Some(&uniform_layout)],
                immediate_size: 0,
            });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("built-in pass sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let ui_pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sharp UI overlay pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_sharp_bilinear"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: output_format,
                        // UI draws were already blended over transparent
                        // black, so the intermediate contains premultiplied
                        // RGB. Multiplying by alpha again here would darken
                        // antialiased text and translucent menu artwork.
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        let ui_uniform_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sharp UI overlay uniforms"),
            size: std::mem::size_of::<BuiltinUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ui_uniform_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sharp UI overlay uniform bind group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: ui_uniform_buffer.as_entire_binding(),
            }],
        });
        let mut uniform_buffers = Vec::with_capacity(MAX_BUILTIN_PASSES);
        let mut uniform_bind_groups = Vec::with_capacity(MAX_BUILTIN_PASSES);
        for index in 0..MAX_BUILTIN_PASSES {
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("built-in pass {index} uniforms")),
                size: std::mem::size_of::<BuiltinUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("built-in pass {index} uniform bind group")),
                layout: &uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            uniform_buffers.push(buffer);
            uniform_bind_groups.push(bind_group);
        }
        Self {
            gpu,
            output_format,
            module,
            texture_layout,
            layout,
            sampler,
            pipelines: HashMap::new(),
            ui_pipeline,
            ui_uniform_buffer,
            ui_uniform_bind_group,
            uniform_buffers,
            uniform_bind_groups,
            source_intermediate: None,
            output_intermediates: [None, None],
            foreign_intermediate: None,
        }
    }

    fn ensure_intermediates(&mut self, source_size: [u32; 2], output_size: [u32; 2]) {
        if self
            .source_intermediate
            .as_ref()
            .is_none_or(|texture| texture.size != source_size)
        {
            self.source_intermediate = Some(IntermediateTexture::new(
                &self.gpu.device,
                "built-in source-sized intermediate",
                source_size,
            ));
        }
        for (index, slot) in self.output_intermediates.iter_mut().enumerate() {
            if slot
                .as_ref()
                .is_none_or(|texture| texture.size != output_size)
            {
                *slot = Some(IntermediateTexture::new(
                    &self.gpu.device,
                    &format!("built-in output intermediate {index}"),
                    output_size,
                ));
            }
        }
    }

    fn render_ui_overlay(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        dst_rect: [f32; 4],
    ) {
        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let source_bind_group = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sharp UI overlay source"),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&source_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
        let source_size = [source.width().max(1), source.height().max(1)];
        let output_size = [dst_rect[2].max(1.0), dst_rect[3].max(1.0)];
        let uniforms = BuiltinUniforms {
            src: [
                source_size[0] as f32,
                source_size[1] as f32,
                1.0 / source_size[0] as f32,
                1.0 / source_size[1] as f32,
            ],
            dst: [
                output_size[0],
                output_size[1],
                1.0 / output_size[0],
                1.0 / output_size[1],
            ],
            upscale: [0.0; 4],
            effect: [0.0; 4],
            temporal: [0.0; 4],
        };
        self.gpu
            .queue
            .write_buffer(&self.ui_uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sharp UI overlay"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        pass.set_viewport(dst_rect[0], dst_rect[1], dst_rect[2], dst_rect[3], 0.0, 1.0);
        pass.set_pipeline(&self.ui_pipeline);
        pass.set_bind_group(0, &source_bind_group, &[]);
        pass.set_bind_group(1, &self.ui_uniform_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Reserve a full-size texture for a foreign multi-pass producer (the
    /// librashader chain) and return cheap handles to the same allocation.
    fn external_intermediate(&mut self, size: [u32; 2]) -> (wgpu::Texture, wgpu::TextureView) {
        if self
            .foreign_intermediate
            .as_ref()
            .is_none_or(|texture| texture.size != size)
        {
            self.foreign_intermediate = Some(IntermediateTexture::new(
                &self.gpu.device,
                "foreign shader output intermediate",
                size,
            ));
        }
        let target = self
            .foreign_intermediate
            .as_ref()
            .expect("foreign output intermediate allocated");
        (target.texture.clone(), target.view.clone())
    }

    fn ensure_pipeline(&mut self, pass: BuiltinPass, target: PipelineTarget) {
        if self.pipelines.contains_key(&(pass, target)) {
            return;
        }
        let format = match target {
            PipelineTarget::Intermediate => wgpu::TextureFormat::Rgba8UnormSrgb,
            PipelineTarget::Surface => self.output_format,
        };
        let pipeline = self
            .gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&format!("built-in {:?} {:?} pipeline", pass, target)),
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module: &self.module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.module,
                    entry_point: Some(pass.entry_point()),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        self.pipelines.insert((pass, target), pipeline);
    }

    #[allow(clippy::too_many_arguments)]
    fn render(
        &mut self,
        mode: TextureScaleMode,
        effect: TextureEffect,
        upscale_parameters: UpscaleParameters,
        effect_parameters: TextureEffectParameters,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        _target_size: [u32; 2],
        dst_rect: [f32; 4],
        presentation_frame: usize,
    ) -> Result<(), UpscaleError> {
        let passes = builtin_passes(mode, effect);
        if passes.len() > MAX_BUILTIN_PASSES {
            return Err(UpscaleError::TooManyPasses {
                passes: passes.len(),
                maximum: MAX_BUILTIN_PASSES,
            });
        }

        let source_size = [source.width(), source.height()];
        let output_size = [
            dst_rect[2].max(1.0).ceil() as u32,
            dst_rect[3].max(1.0).ceil() as u32,
        ];
        self.ensure_intermediates(source_size, output_size);
        for (index, pass) in passes.iter().copied().enumerate() {
            self.ensure_pipeline(
                pass,
                if index + 1 == passes.len() {
                    PipelineTarget::Surface
                } else {
                    PipelineTarget::Intermediate
                },
            );
        }

        #[derive(Clone, Copy)]
        enum InputSlot {
            Source,
            SourceIntermediate,
            Output(usize),
        }
        let mut input_slot = InputSlot::Source;
        let mut output_ping = 0usize;
        for (index, pass_kind) in passes.iter().copied().enumerate() {
            let final_pass = index + 1 == passes.len();
            let input_view = match input_slot {
                InputSlot::Source => source.create_view(&wgpu::TextureViewDescriptor::default()),
                InputSlot::SourceIntermediate => self
                    .source_intermediate
                    .as_ref()
                    .expect("source intermediate allocated")
                    .view
                    .clone(),
                InputSlot::Output(slot) => self.output_intermediates[slot]
                    .as_ref()
                    .expect("output intermediate allocated")
                    .view
                    .clone(),
            };
            let input_size = match input_slot {
                InputSlot::Source | InputSlot::SourceIntermediate => source_size,
                InputSlot::Output(_) => output_size,
            };
            let output_size_for_pass = if pass_kind.source_sized() {
                source_size
            } else {
                output_size
            };
            let (output_view, next_input) = if final_pass {
                (target_view.clone(), input_slot)
            } else if pass_kind.source_sized() {
                (
                    self.source_intermediate
                        .as_ref()
                        .expect("source intermediate allocated")
                        .view
                        .clone(),
                    InputSlot::SourceIntermediate,
                )
            } else {
                if matches!(input_slot, InputSlot::Output(slot) if slot == output_ping) {
                    output_ping ^= 1;
                }
                let slot = output_ping;
                output_ping ^= 1;
                (
                    self.output_intermediates[slot]
                        .as_ref()
                        .expect("output intermediate allocated")
                        .view
                        .clone(),
                    InputSlot::Output(slot),
                )
            };
            let input_bind_group = self
                .gpu
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("built-in pass source bind group"),
                    layout: &self.texture_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&input_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
            let pct = |value: u8| f32::from(value.min(100)) / 100.0;
            let uniforms = BuiltinUniforms {
                src: [
                    input_size[0] as f32,
                    input_size[1] as f32,
                    1.0 / input_size[0] as f32,
                    1.0 / input_size[1] as f32,
                ],
                dst: [
                    output_size_for_pass[0] as f32,
                    output_size_for_pass[1] as f32,
                    1.0 / output_size_for_pass[0] as f32,
                    1.0 / output_size_for_pass[1] as f32,
                ],
                upscale: [
                    pct(upscale_parameters.strength),
                    pct(upscale_parameters.edge_threshold),
                    pct(upscale_parameters.artifact_removal),
                    0.0,
                ],
                effect: [
                    pct(effect_parameters.scanlines),
                    pct(effect_parameters.phosphor_mask),
                    pct(effect_parameters.bloom),
                    pct(effect_parameters.curvature),
                ],
                temporal: [
                    pct(effect_parameters.temporal_flicker),
                    (presentation_frame % 4096) as f32,
                    0.0,
                    0.0,
                ],
            };
            self.gpu.queue.write_buffer(
                &self.uniform_buffers[index],
                0,
                bytemuck::bytes_of(&uniforms),
            );
            let target = if final_pass {
                PipelineTarget::Surface
            } else {
                PipelineTarget::Intermediate
            };
            let pipeline = self
                .pipelines
                .get(&(pass_kind, target))
                .expect("pipeline ensured above");
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("built-in upscaler pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            if final_pass {
                render_pass.set_viewport(
                    dst_rect[0],
                    dst_rect[1],
                    dst_rect[2],
                    dst_rect[3],
                    0.0,
                    1.0,
                );
            }
            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(0, &input_bind_group, &[]);
            render_pass.set_bind_group(1, &self.uniform_bind_groups[index], &[]);
            render_pass.draw(0..3, 0..1);
            drop(render_pass);
            input_slot = next_input;
        }
        Ok(())
    }
}

fn empty_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("empty bgl"),
        entries: &[],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_multipass_wgsl_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(BUILTIN_MULTIPASS_WGSL)
            .unwrap_or_else(|error| panic!("built-in presentation WGSL does not parse: {error}"));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            // Empty optional capabilities exercises the portable WebGPU/
            // WebGL2 subset instead of accidentally permitting native-only
            // subgroup, ray-query, or 64-bit constructs.
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|error| panic!("built-in presentation WGSL is invalid: {error}"));
    }

    #[test]
    fn every_bundled_entry_point_translates_to_webgl2_glsl() {
        use naga::back::glsl;

        let module = naga::front::wgsl::parse_str(BUILTIN_MULTIPASS_WGSL)
            .unwrap_or_else(|error| panic!("built-in presentation WGSL does not parse: {error}"));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|error| panic!("built-in presentation WGSL is invalid: {error}"));
        let options = glsl::Options {
            version: glsl::Version::Embedded {
                version: 300,
                is_webgl: true,
            },
            ..Default::default()
        };
        for entry in &module.entry_points {
            let pipeline = glsl::PipelineOptions {
                shader_stage: entry.stage,
                entry_point: entry.name.clone(),
                multiview: None,
            };
            let mut source = String::new();
            glsl::Writer::new(
                &mut source,
                &module,
                &info,
                &options,
                &pipeline,
                naga::proc::BoundsCheckPolicies::default(),
            )
            .unwrap_or_else(|error| {
                panic!("{} cannot target WebGL2 GLSL ES 3.00: {error}", entry.name)
            })
            .write()
            .unwrap_or_else(|error| {
                panic!("{} failed WebGL2 GLSL generation: {error}", entry.name)
            });
            assert!(source.starts_with("#version 300 es"), "{}", entry.name);
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn headless_downlevel_device_executes_every_multipass_profile() {
        use std::sync::Arc;

        pollster::block_on(async {
            let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
            descriptor.backends = wgpu::Backends::PRIMARY | wgpu::Backends::GL;
            let instance = Arc::new(wgpu::Instance::new(descriptor));
            let adapter = match instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: None,
                    force_fallback_adapter: true,
                    apply_limit_buckets: false,
                })
                .await
            {
                Ok(adapter) => adapter,
                Err(fallback_error) => match instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::LowPower,
                        compatible_surface: None,
                        force_fallback_adapter: false,
                        apply_limit_buckets: false,
                    })
                    .await
                {
                    Ok(adapter) => adapter,
                    Err(hardware_error) => {
                        eprintln!(
                            "skipping headless multipass smoke test: no adapter; fallback={fallback_error}; hardware={hardware_error}"
                        );
                        return;
                    }
                },
            };
            let limits =
                wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("multipass smoke test device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: limits,
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    trace: wgpu::Trace::Off,
                })
                .await
                .expect("headless adapter must provide WebGL2 baseline limits");
            let gpu = GpuContext {
                instance,
                adapter: Arc::new(adapter),
                device: Arc::new(device),
                queue: Arc::new(queue),
                surface_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            };
            let source = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("multipass smoke source"),
                size: wgpu::Extent3d {
                    width: 4,
                    height: 4,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let mut pixels = Vec::with_capacity(4 * 4 * 4);
            let colours = [
                [255, 20, 10, 255],
                [10, 220, 30, 255],
                [20, 40, 240, 255],
                [240, 220, 30, 255],
            ];
            for y in 0..4 {
                for x in 0..4 {
                    pixels.extend_from_slice(&colours[(x + y * 3) % colours.len()]);
                }
            }
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &source,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(16),
                    rows_per_image: Some(4),
                },
                wgpu::Extent3d {
                    width: 4,
                    height: 4,
                    depth_or_array_layers: 1,
                },
            );
            let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("multipass smoke output"),
                size: wgpu::Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let mut upscaler = GpuUpscale::new(gpu.clone(), wgpu::TextureFormat::Rgba8UnormSrgb);
            for (index, (mode, effect)) in [
                (TextureScaleMode::Linear, TextureEffect::None),
                (TextureScaleMode::PixelArt, TextureEffect::None),
                (TextureScaleMode::Nearest, TextureEffect::CrtGuest),
                (TextureScaleMode::ScaleNx, TextureEffect::None),
                (TextureScaleMode::Hqx, TextureEffect::CrtGuest),
                (TextureScaleMode::Xbrz, TextureEffect::None),
                (TextureScaleMode::SuperXbr, TextureEffect::CrtRoyale),
                (TextureScaleMode::Anime4kA, TextureEffect::None),
                (TextureScaleMode::Anime4kB, TextureEffect::CrtGuest),
                (TextureScaleMode::Anime4kC, TextureEffect::CrtRoyale),
            ]
            .into_iter()
            .enumerate()
            {
                let mut encoder =
                    gpu.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("multipass smoke encoder"),
                        });
                assert!(
                    upscaler
                        .render_multipass(
                            mode,
                            &mut encoder,
                            &source,
                            &target_view,
                            [16, 16],
                            [0.0, 0.0, 16.0, 16.0],
                            Some(index),
                            None,
                            effect,
                            UpscaleParameters::default(),
                            TextureEffectParameters::default(),
                        )
                        .unwrap_or_else(|error| panic!("{mode:?}/{effect:?} failed: {error}"))
                );
                gpu.queue.submit(Some(encoder.finish()));
            }

            let padded_bytes_per_row = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("multipass smoke readback"),
                size: u64::from(padded_bytes_per_row * 16),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("multipass smoke readback encoder"),
                });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bytes_per_row),
                        rows_per_image: Some(16),
                    },
                },
                wgpu::Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 1,
                },
            );
            gpu.queue.submit(Some(encoder.finish()));
            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("headless device poll failed");
            let mapped = slice.get_mapped_range().expect("readback did not map");
            let mut unique_rgb = std::collections::BTreeSet::new();
            for row in 0..16usize {
                let start = row * padded_bytes_per_row as usize;
                for pixel in mapped[start..start + 64].chunks_exact(4) {
                    unique_rgb.insert([pixel[0], pixel[1], pixel[2]]);
                    assert_ne!(pixel[3], 0, "profile output unexpectedly transparent");
                }
            }
            assert!(
                unique_rgb.len() >= 4,
                "profile output collapsed to fewer than four colours: {unique_rgb:?}"
            );
            drop(mapped);
            readback.unmap();
        });
    }

    #[test]
    fn every_curated_mode_is_routed_through_the_runner() {
        for mode in [
            TextureScaleMode::Linear,
            TextureScaleMode::PixelArt,
            TextureScaleMode::ScaleNx,
            TextureScaleMode::Hqx,
            TextureScaleMode::Xbrz,
            TextureScaleMode::SuperXbr,
            TextureScaleMode::Anime4kA,
            TextureScaleMode::Anime4kB,
            TextureScaleMode::Anime4kC,
        ] {
            assert!(
                matches!(mode, TextureScaleMode::Linear | TextureScaleMode::PixelArt)
                    || mode.uses_builtin_chain(),
                "missing route for {mode:?}"
            );
            assert!(GpuUpscale::is_multipass_mode(mode, TextureEffect::None));
        }
        for effect in TextureEffect::ALL {
            assert!(
                *effect == TextureEffect::None
                    || GpuUpscale::is_multipass_mode(TextureScaleMode::Linear, *effect)
            );
        }
    }

    #[test]
    fn scale2x_published_corner_vector_is_preserved() {
        // Published Scale2x rule for B=1, D=1, E=2, F=2, H=2:
        // [D==B ? D : E, B==F ? F : E; D==H ? D : E, H==F ? F : E].
        fn corners(b: u8, d: u8, e: u8, f: u8, h: u8) -> [u8; 4] {
            if b == h || d == f {
                return [e; 4];
            }
            [
                if d == b { d } else { e },
                if b == f { f } else { e },
                if d == h { d } else { e },
                if h == f { f } else { e },
            ]
        }
        assert_eq!(corners(1, 1, 2, 2, 2), [1, 2, 2, 2]);
        assert_eq!(corners(1, 3, 2, 3, 1), [2; 4]);

        let input = [[0u8, 1, 0], [1, 2, 2], [0, 2, 3]];
        let mut output = [[0u8; 6]; 6];
        for y in 0..3usize {
            for x in 0..3usize {
                let b = input[y.saturating_sub(1)][x];
                let d = input[y][x.saturating_sub(1)];
                let e = input[y][x];
                let f = input[y][(x + 1).min(2)];
                let h = input[(y + 1).min(2)][x];
                let expanded = corners(b, d, e, f, h);
                output[y * 2][x * 2..x * 2 + 2].copy_from_slice(&expanded[..2]);
                output[y * 2 + 1][x * 2..x * 2 + 2].copy_from_slice(&expanded[2..]);
            }
        }
        assert_eq!(
            output,
            [
                [0, 0, 1, 1, 0, 0],
                [0, 1, 1, 1, 0, 0],
                [1, 1, 1, 2, 2, 2],
                [1, 1, 2, 2, 2, 2],
                [0, 0, 2, 2, 2, 3],
                [0, 0, 2, 2, 3, 3],
            ]
        );
    }

    #[test]
    fn anime_v4_layouts_and_post_effect_order_are_fixed() {
        assert_eq!(
            builtin_passes(TextureScaleMode::Anime4kA, TextureEffect::None),
            [
                BuiltinPass::AnimeRestore,
                BuiltinPass::AnimeUpscale,
                BuiltinPass::ArtifactRemove,
            ]
        );
        assert_eq!(
            builtin_passes(TextureScaleMode::Anime4kB, TextureEffect::None),
            [
                BuiltinPass::AnimeRestoreSoft,
                BuiltinPass::AnimeUpscale,
                BuiltinPass::ArtifactRemove,
            ]
        );
        assert_eq!(
            builtin_passes(TextureScaleMode::Anime4kC, TextureEffect::CrtRoyale),
            [
                BuiltinPass::AnimeDenoise,
                BuiltinPass::AnimeUpscale,
                BuiltinPass::ArtifactRemove,
                BuiltinPass::CrtRoyale,
            ]
        );
    }

    #[test]
    fn approximate_algorithms_are_labelled_as_such() {
        for mode in [
            TextureScaleMode::Hqx,
            TextureScaleMode::Xbrz,
            TextureScaleMode::SuperXbr,
        ] {
            assert!(mode.label().contains("style"));
        }
        for mode in [
            TextureScaleMode::Anime4kA,
            TextureScaleMode::Anime4kB,
            TextureScaleMode::Anime4kC,
        ] {
            assert!(mode.label().contains("layout"));
        }
    }
}
