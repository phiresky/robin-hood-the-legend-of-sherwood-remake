use crate::loading_screen::HeightField;
use crate::renderer::{rgb565_to_rgb8, upload_rgba_texture};
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
    let expected = match validate_upload(width, height, initial_pixels, final_pixels, height_field)
    {
        Ok(expected) => expected,
        Err(reason) => {
            tracing::warn!(width, height, reason, "loading dissolve upload rejected");
            return None;
        }
    };

    // Each upload copies its bytes into GPU-owned storage. Reuse one conversion
    // buffer for both images and the mask rather than allocating each separately.
    let mut rgba = Vec::with_capacity(expected * 4);
    rgb565_to_rgba_opaque(initial_pixels, &mut rgba);
    let (initial_texture, initial_view) =
        upload_rgba_texture(gpu, &rgba, width, height, "loading initial");
    rgb565_to_rgba_opaque(final_pixels, &mut rgba);
    let (final_texture, final_view) =
        upload_rgba_texture(gpu, &rgba, width, height, "loading final");
    rgba.clear();
    for &h in &height_field.data {
        rgba.extend_from_slice(&[h, h, h, 255]);
    }

    let (mask_texture, mask_view) = upload_rgba_texture(gpu, &rgba, width, height, "loading mask");

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

fn validate_upload(
    width: u32,
    height: u32,
    initial_pixels: &[u16],
    final_pixels: &[u16],
    height_field: &HeightField,
) -> Result<usize, &'static str> {
    if width == 0 || height == 0 {
        return Err("dimensions must be nonzero");
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .filter(|count| count.checked_mul(4).is_some())
        .ok_or("RGBA image size overflow")?;
    if initial_pixels.len() != expected
        || final_pixels.len() != expected
        || height_field.data.len() != expected
        || height_field.width != width
        || height_field.height != height
    {
        return Err("image and mask dimensions or pixel counts do not match");
    }
    Ok(expected)
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

/// RGB565 → opaque RGBA8 using the renderer's truncating channel
/// expansion, so the dissolve screens match how every other 565
/// surface (backgrounds, sprites, managed surfaces) is expanded.
/// No colour-key or shadow-key handling: these are literal screen
/// captures.
fn rgb565_to_rgba_opaque(pixels: &[u16], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(pixels.len() * 4);
    for &px in pixels {
        let (r, g, b) = rgb565_to_rgb8(px);
        out.extend_from_slice(&[r, g, b, 255]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_conversion_reuses_storage_and_discards_previous_image_bytes() {
        let mut rgba = Vec::with_capacity(16);
        let allocation = rgba.as_ptr();
        for pixels in [&[0xFFFF, 0xF800, 0x07E0][..], &[0], &[], &[0x001F, 0xFFFF]] {
            rgb565_to_rgba_opaque(pixels, &mut rgba);
            assert_eq!(rgba.as_ptr(), allocation);
            assert_eq!(rgba.len(), pixels.len() * 4);
            for (&pixel, actual) in pixels.iter().zip(rgba.as_chunks::<4>().0) {
                let (r, g, b) = rgb565_to_rgb8(pixel);
                assert_eq!(*actual, [r, g, b, 255]);
            }
        }
    }

    #[test]
    fn upload_validation_rejects_invalid_shapes_before_allocation() {
        let mask = HeightField {
            data: vec![0; 6],
            width: 3,
            height: 2,
        };
        assert_eq!(validate_upload(3, 2, &[0; 6], &[0; 6], &mask), Ok(6));
        assert!(validate_upload(0, 2, &[], &[], &mask).is_err());
        assert!(validate_upload(3, 0, &[], &[], &mask).is_err());
        assert_eq!(
            validate_upload(u32::MAX, u32::MAX, &[], &[], &mask),
            Err("RGBA image size overflow")
        );
        assert!(validate_upload(3, 2, &[0; 5], &[0; 6], &mask).is_err());
        assert!(validate_upload(3, 2, &[0; 6], &[0; 7], &mask).is_err());
        assert!(validate_upload(2, 3, &[0; 6], &[0; 6], &mask).is_err());
        let short_mask = HeightField {
            data: vec![0; 5],
            ..mask
        };
        assert!(validate_upload(3, 2, &[0; 6], &[0; 6], &short_mask).is_err());
    }

    #[test]
    fn opaque_conversion_preserves_every_rgb565_value() {
        let pixels: Vec<_> = (0..=u16::MAX).collect();
        let mut rgba = Vec::new();
        rgb565_to_rgba_opaque(&pixels, &mut rgba);
        assert_eq!(rgba.len(), pixels.len() * 4);
        for (pixel, color) in pixels.into_iter().zip(rgba.chunks_exact(4)) {
            assert_eq!(
                color,
                [
                    ((pixel >> 11) << 3) as u8,
                    (((pixel >> 5) & 63) << 2) as u8,
                    ((pixel & 31) << 3) as u8,
                    255,
                ]
            );
        }
    }
}
