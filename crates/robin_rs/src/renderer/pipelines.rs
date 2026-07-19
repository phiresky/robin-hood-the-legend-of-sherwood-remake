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
    pub(super) pipelines: [Option<wgpu::RenderPipeline>; 4],
    pub(super) masked_pipelines: [Option<wgpu::RenderPipeline>; 4],
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
        let pipelines =
            build_quad_pipelines(&gpu.device, bgl_screen, bgl_tex, output_format, true, false);
        let masked_pipelines =
            build_quad_pipelines(&gpu.device, bgl_screen, bgl_tex, output_format, true, true);
        let blit_pipeline = build_quad_pipelines(
            &gpu.device,
            bgl_screen,
            bgl_tex,
            gpu.surface_format,
            false,
            false,
        )[blend_index(BlendMode::None)]
        .clone()
        .expect("blit pipeline");
        let colorize_pipeline =
            build_colorize_pipeline(&gpu.device, bgl_screen, bgl_tex, output_format);
        let bg_alpha_pipeline =
            build_bg_alpha_pipeline(&gpu.device, bgl_screen, bgl_tex, output_format);
        let view_cone_pipeline =
            build_view_cone_pipeline(&gpu.device, bgl_screen, bgl_tex, output_format);
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
    let vertex_buffers = [wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<QuadVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
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
        ],
    }];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quad/sprite_mask_stencil"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[Some(vertex_buffers[0].clone())],
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
    bgl_screen: &wgpu::BindGroupLayout,
    bgl_tex: &wgpu::BindGroupLayout,
    output_format: wgpu::TextureFormat,
    stencil_attachment: bool,
    masked: bool,
) -> [Option<wgpu::RenderPipeline>; 4] {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("quad.wgsl"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/quad.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("quad layout"),
        bind_group_layouts: &[Some(bgl_screen), Some(bgl_tex)],
        immediate_size: 0,
    });
    let vertex_buffers = [wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<QuadVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
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
        ],
    }];

    let mut out: [Option<wgpu::RenderPipeline>; 4] = [None, None, None, None];
    for &(blend, idx, normal_label, masked_label) in &[
        (BlendMode::None, 0usize, "quad/none", "quad/masked_none"),
        (BlendMode::Blend, 1, "quad/blend", "quad/masked_blend"),
        (BlendMode::Add, 2, "quad/add", "quad/masked_add"),
        (BlendMode::Mod, 3, "quad/mod", "quad/masked_mod"),
    ] {
        let label = if masked { masked_label } else { normal_label };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[Some(vertex_buffers[0].clone())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
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
        });
        out[idx] = Some(pipeline);
    }
    out
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

fn build_colorize_pipeline(
    device: &wgpu::Device,
    bgl_screen: &wgpu::BindGroupLayout,
    bgl_tex: &wgpu::BindGroupLayout,
    output_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("quad.wgsl (colorize)"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/quad.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("colorize layout"),
        bind_group_layouts: &[Some(bgl_screen), Some(bgl_tex)],
        immediate_size: 0,
    });
    let vertex_buffers = [wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<QuadVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
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
        ],
    }];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quad/colorize"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[Some(vertex_buffers[0].clone())],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_colorize"),
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: None,
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

/// Build the background-sampled alpha polygon pipeline used by
/// `DrawManager::draw_alpha_polygon`.
fn build_bg_alpha_pipeline(
    device: &wgpu::Device,
    bgl_screen: &wgpu::BindGroupLayout,
    bgl_tex: &wgpu::BindGroupLayout,
    output_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("quad.wgsl (bg alpha polygon)"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/quad.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("bg alpha polygon layout"),
        bind_group_layouts: &[Some(bgl_screen), Some(bgl_tex)],
        immediate_size: 0,
    });
    let vertex_buffers = [wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<QuadVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
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
        ],
    }];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quad/bg_alpha_polygon"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[Some(vertex_buffers[0].clone())],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_bg_alpha_polygon"),
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: None,
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

/// Build the view-cone gradient span pipeline used by
/// `shadow_polygon::render_darken_inside`.
fn build_view_cone_pipeline(
    device: &wgpu::Device,
    bgl_screen: &wgpu::BindGroupLayout,
    bgl_tex: &wgpu::BindGroupLayout,
    output_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("quad.wgsl (view cone gradient)"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/quad.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("view cone gradient layout"),
        bind_group_layouts: &[Some(bgl_screen), Some(bgl_tex)],
        immediate_size: 0,
    });
    let vertex_buffers = [wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<QuadVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
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
        ],
    }];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quad/view_cone_gradient"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[Some(vertex_buffers[0].clone())],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_view_cone_gradient"),
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: BlendMode::Blend.to_wgpu(),
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
