use crate::loading_screen::HeightField;
use crate::window::GpuContext;

pub struct LoadingDissolveTextures {
    _initial_texture: wgpu::Texture,
    initial_view: wgpu::TextureView,
    _final_texture: wgpu::Texture,
    final_view: wgpu::TextureView,
    _mask_texture: wgpu::Texture,
    mask_view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

pub(crate) fn create_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("loading dissolve bgl"),
        entries: &[
            texture_entry(0),
            texture_entry(1),
            texture_entry(2),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

pub(crate) fn build_pipeline(
    device: &wgpu::Device,
    bgl_screen: &wgpu::BindGroupLayout,
    bgl_loading_dissolve: &wgpu::BindGroupLayout,
    output_format: wgpu::TextureFormat,
    quad_vertex_stride: u64,
    depth_stencil_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("loading_dissolve.wgsl"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/loading_dissolve.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("loading dissolve layout"),
        bind_group_layouts: &[Some(bgl_screen), Some(bgl_loading_dissolve)],
        immediate_size: 0,
    });
    let vertex_buffers = [wgpu::VertexBufferLayout {
        array_stride: quad_vertex_stride,
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
        label: Some("quad/loading_dissolve"),
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
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_stencil_format,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

pub(crate) fn upload_textures(
    gpu: &GpuContext,
    width: u32,
    height: u32,
    initial_pixels: &[u16],
    final_pixels: &[u16],
    height_field: &HeightField,
) -> Option<LoadingDissolveTextures> {
    let expected = width as usize * height as usize;
    if initial_pixels.len() != expected
        || final_pixels.len() != expected
        || height_field.data.len() != expected
        || height_field.width != width
        || height_field.height != height
    {
        return None;
    }

    let initial_rgba = rgb565_to_rgba_opaque(initial_pixels);
    let final_rgba = rgb565_to_rgba_opaque(final_pixels);
    let mut mask_rgba = Vec::with_capacity(expected * 4);
    for &h in &height_field.data {
        mask_rgba.extend_from_slice(&[h, h, h, 255]);
    }

    let (initial_texture, initial_view) =
        upload_rgba_texture(gpu, &initial_rgba, width, height, "loading initial");
    let (final_texture, final_view) =
        upload_rgba_texture(gpu, &final_rgba, width, height, "loading final");
    let (mask_texture, mask_view) =
        upload_rgba_texture(gpu, &mask_rgba, width, height, "loading mask");

    Some(LoadingDissolveTextures {
        _initial_texture: initial_texture,
        initial_view,
        _final_texture: final_texture,
        final_view,
        _mask_texture: mask_texture,
        mask_view,
        width,
        height,
    })
}

pub(crate) fn create_frame_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    textures: &LoadingDissolveTextures,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("loading dissolve bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&textures.initial_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&textures.final_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&textures.mask_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn rgb565_to_rgba_opaque(pixels: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len() * 4);
    for &px in pixels {
        let r5 = ((px >> 11) & 0x1F) as u8;
        let g6 = ((px >> 5) & 0x3F) as u8;
        let b5 = (px & 0x1F) as u8;
        out.push((r5 << 3) | (r5 >> 2));
        out.push((g6 << 2) | (g6 >> 4));
        out.push((b5 << 3) | (b5 >> 2));
        out.push(255);
    }
    out
}

fn upload_rgba_texture(
    gpu: &GpuContext,
    rgba: &[u8],
    width: u32,
    height: u32,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
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
            texture: &texture,
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
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}
