//! Synchronous logical-render-target readback facilities.

use crate::window::GpuContext;

use super::frame::FrameState;
use super::pipelines::PipelineStore;
use super::resources::GpuResources;

pub(super) fn capture_frame_rgba(
    gpu: &GpuContext,
    pipelines: &PipelineStore,
    resources: &GpuResources,
    frame: &mut FrameState,
) -> Option<(u32, u32, Vec<u8>)> {
    let (width, height) = frame.dimensions();
    let w = width as u32;
    let h = height as u32;
    if w == 0 || h == 0 {
        return None;
    }

    frame.push_implicit_base_quad();
    frame.upload_queue_geometry(gpu);

    let bytes_per_pixel = 4u32;
    let unpadded_bpr = w * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bpr = unpadded_bpr.div_ceil(align) * align;
    let buffer_size = padded_bpr as u64 * h as u64;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rt readback"),
        size: buffer_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("capture frame"),
        });
    frame.encode_pass1_to_rt(&mut encoder, pipelines, resources);
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &frame.render_target_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(Some(encoder.finish()));

    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    if gpu
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .is_err()
    {
        tracing::warn!("capture_frame_rgba: device.poll(Wait) failed");
        frame.clear_recording();
        return None;
    }
    let mapped = match slice.get_mapped_range() {
        Ok(mapped) => mapped,
        Err(error) => {
            tracing::warn!(%error, "capture_frame_rgba: failed to access mapped buffer");
            buffer.unmap();
            frame.clear_recording();
            return None;
        }
    };
    let mut rgba = Vec::with_capacity((w * h * bytes_per_pixel) as usize);
    for row in 0..h {
        let start = (row * padded_bpr) as usize;
        let end = start + unpadded_bpr as usize;
        rgba.extend_from_slice(&mapped[start..end]);
    }
    drop(mapped);
    buffer.unmap();
    frame.clear_recording();

    Some((w, h, rgba))
}
