//! wgpu-native upscale shader pipelines.
//!
//! Simple upscalers use local WGSL final-blit shaders. RetroArch
//! `.slangp` presets are delegated to [`crate::shader_preset`].

use robin_engine::graphic_config::TextureScaleMode;
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
    /// Output format the pipelines target (the swapchain format).
    output_format: wgpu::TextureFormat,
}

impl GpuUpscale {
    pub fn new(gpu: GpuContext, output_format: wgpu::TextureFormat) -> Self {
        Self {
            preset_renderer: ShaderPresetRenderer::new(gpu.clone()),
            gpu,
            pipelines: HashMap::new(),
            output_format,
        }
    }

    pub fn is_multipass_mode(mode: TextureScaleMode) -> bool {
        crate::shader_preset::is_shader_preset_mode(mode)
    }

    /// Get-or-build the pipeline for the given scale mode.
    /// Returns `None` for non-shader modes (Nearest/Linear/PixelArt) or
    /// if shader compilation fails.
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
    ) -> Option<()> {
        self.preset_renderer.render(
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
    }
}

fn empty_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("empty bgl"),
        entries: &[],
    })
}
