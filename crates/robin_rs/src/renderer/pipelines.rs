//! Render pipelines and the lazy upscale-pipeline cache.

use robin_engine::graphic_config::TextureScaleMode;

use crate::gpu_upscale::GpuUpscale;
use crate::window::GpuContext;

use super::QuadVertex;
use crate::gfx_types::BlendMode;

pub(super) struct PipelineStore {
    pub(super) scale_mode: TextureScaleMode,
    pub(super) shader_preset: String,
    pub(super) gpu_upscale: GpuUpscale,
    pub(super) pipelines: [wgpu::RenderPipeline; 4],
    pub(super) masked_pipelines: [wgpu::RenderPipeline; 4],
    pub(super) blit_pipeline: wgpu::RenderPipeline,
    pub(super) colorize_pipeline: wgpu::RenderPipeline,
    pub(super) bg_alpha_pipeline: wgpu::RenderPipeline,
    pub(super) view_cone_pipeline: wgpu::RenderPipeline,
    pub(super) mask_stencil_pipeline: wgpu::RenderPipeline,
    pub(super) loading_dissolve_pipeline: wgpu::RenderPipeline,
    pub(super) bgl_loading_dissolve: wgpu::BindGroupLayout,
}

impl PipelineStore {
    pub(super) fn new(
        gpu: &GpuContext,
        bgl_screen: &wgpu::BindGroupLayout,
        bgl_tex: &wgpu::BindGroupLayout,
        scale_mode: TextureScaleMode,
    ) -> Self {
        let bgl_loading_dissolve =
            crate::loading_dissolve_gpu::create_bind_group_layout(&gpu.device);
        let output_format = wgpu::TextureFormat::Rgba8UnormSrgb;

        // One shared quad.wgsl shader module + pipeline layout: every
        // quad-family pipeline below (blend variants, masked variants,
        // blit, colorize, bg-alpha, view-cone) uses the same module and
        // bind-group layouts, so compile the shader once.
        let quad_module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("quad.wgsl"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/quad.wgsl").into()),
            });
        let quad_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("quad layout"),
                bind_group_layouts: &[Some(bgl_screen), Some(bgl_tex)],
                immediate_size: 0,
            });

        let pipelines = build_quad_pipelines(
            &gpu.device,
            &quad_module,
            &quad_layout,
            output_format,
            true,
            false,
        );
        let masked_pipelines = build_quad_pipelines(
            &gpu.device,
            &quad_module,
            &quad_layout,
            output_format,
            true,
            true,
        );
        let blit_pipeline = build_quad_pipelines(
            &gpu.device,
            &quad_module,
            &quad_layout,
            gpu.surface_format,
            false,
            false,
        )[blend_index(BlendMode::None)]
        .clone();
        let colorize_pipeline = build_single_quad_pipeline(
            &gpu.device,
            &quad_module,
            &quad_layout,
            "quad/colorize",
            "fs_colorize",
            None,
            output_format,
        );
        let bg_alpha_pipeline = build_single_quad_pipeline(
            &gpu.device,
            &quad_module,
            &quad_layout,
            "quad/bg_alpha_polygon",
            "fs_bg_alpha_polygon",
            None,
            output_format,
        );
        let view_cone_pipeline = build_single_quad_pipeline(
            &gpu.device,
            &quad_module,
            &quad_layout,
            "quad/view_cone_gradient",
            "fs_view_cone_gradient",
            BlendMode::Blend.to_wgpu(),
            output_format,
        );
        let mask_stencil_pipeline =
            build_mask_stencil_pipeline(&gpu.device, bgl_screen, bgl_tex, output_format);
        let loading_dissolve_pipeline = crate::loading_dissolve_gpu::build_pipeline(
            &gpu.device,
            bgl_screen,
            &bgl_loading_dissolve,
            output_format,
            std::mem::size_of::<QuadVertex>() as u64,
            SPRITE_STENCIL_FORMAT,
        );

        Self {
            scale_mode,
            shader_preset: String::new(),
            gpu_upscale: GpuUpscale::new(gpu.clone(), gpu.surface_format),
            pipelines,
            masked_pipelines,
            blit_pipeline,
            colorize_pipeline,
            bg_alpha_pipeline,
            view_cone_pipeline,
            mask_stencil_pipeline,
            loading_dissolve_pipeline,
            bgl_loading_dissolve,
        }
    }

    pub(super) fn scale_mode(&self) -> TextureScaleMode {
        self.scale_mode
    }

    pub(super) fn set_scale_mode(&mut self, mode: TextureScaleMode) {
        self.scale_mode = mode;
    }

    pub(super) fn set_shader_preset(&mut self, preset: impl Into<String>) {
        self.shader_preset = preset.into();
    }
}

pub(super) const SPRITE_STENCIL_FORMAT: wgpu::TextureFormat =
    wgpu::TextureFormat::Depth24PlusStencil8;

const QUAD_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 8,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 16,
        shader_location: 2,
    },
];

fn quad_vertex_buffer_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<QuadVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &QUAD_VERTEX_ATTRIBUTES,
    }
}

fn sprite_stencil_state(
    compare: wgpu::CompareFunction,
    pass_op: wgpu::StencilOperation,
) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: SPRITE_STENCIL_FORMAT,
        depth_write_enabled: Some(false),
        depth_compare: Some(wgpu::CompareFunction::Always),
        stencil: wgpu::StencilState {
            front: wgpu::StencilFaceState {
                compare,
                fail_op: wgpu::StencilOperation::Keep,
                depth_fail_op: wgpu::StencilOperation::Keep,
                pass_op,
            },
            back: wgpu::StencilFaceState {
                compare,
                fail_op: wgpu::StencilOperation::Keep,
                depth_fail_op: wgpu::StencilOperation::Keep,
                pass_op,
            },
            read_mask: 0xFF,
            write_mask: if pass_op == wgpu::StencilOperation::Replace {
                0xFF
            } else {
                0
            },
        },
        bias: wgpu::DepthBiasState::default(),
    }
}

fn build_mask_stencil_pipeline(
    device: &wgpu::Device,
    bgl_screen: &wgpu::BindGroupLayout,
    bgl_tex: &wgpu::BindGroupLayout,
    output_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sprite_mask_stencil.wgsl"),
        source: wgpu::ShaderSource::Wgsl(
            include_str!("../../shaders/sprite_mask_stencil.wgsl").into(),
        ),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sprite mask stencil layout"),
        bind_group_layouts: &[Some(bgl_screen), Some(bgl_tex)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quad/sprite_mask_stencil"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[Some(quad_vertex_buffer_layout())],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: None,
                write_mask: wgpu::ColorWrites::empty(),
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(sprite_stencil_state(
            wgpu::CompareFunction::Always,
            wgpu::StencilOperation::Replace,
        )),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn build_quad_pipelines(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    output_format: wgpu::TextureFormat,
    stencil_attachment: bool,
    masked: bool,
) -> [wgpu::RenderPipeline; 4] {
    [
        (BlendMode::None, "quad/none", "quad/masked_none"),
        (BlendMode::Blend, "quad/blend", "quad/masked_blend"),
        (BlendMode::Add, "quad/add", "quad/masked_add"),
        (BlendMode::Mod, "quad/mod", "quad/masked_mod"),
    ]
    .map(|(blend, normal_label, masked_label)| {
        let label = if masked { masked_label } else { normal_label };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_main"),
                buffers: &[Some(quad_vertex_buffer_layout())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: blend.to_wgpu(),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: if masked {
                Some(sprite_stencil_state(
                    wgpu::CompareFunction::Equal,
                    wgpu::StencilOperation::Keep,
                ))
            } else if stencil_attachment {
                Some(sprite_stencil_state(
                    wgpu::CompareFunction::Always,
                    wgpu::StencilOperation::Keep,
                ))
            } else {
                None
            },
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    })
}

#[inline]
pub(super) fn blend_index(b: BlendMode) -> usize {
    match b {
        BlendMode::None => 0,
        BlendMode::Blend => 1,
        BlendMode::Add => 2,
        BlendMode::Mod => 3,
    }
}

/// Build one quad-family pipeline that differs from the standard
/// textured quad only in its fragment entry point, blend state and
/// label: `fs_colorize` (menu dim/colorize), `fs_bg_alpha_polygon`
/// (door/patch hover highlights) and `fs_view_cone_gradient`
/// (view-cone span fills).
fn build_single_quad_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    label: &str,
    fragment_entry: &str,
    blend: Option<wgpu::BlendState>,
    output_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            buffers: &[Some(quad_vertex_buffer_layout())],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(sprite_stencil_state(
            wgpu::CompareFunction::Always,
            wgpu::StencilOperation::Keep,
        )),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
