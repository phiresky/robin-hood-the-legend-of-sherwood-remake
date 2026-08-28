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
    let queued_draws = frame.queued.len();
    frame.encode_pass1_to_rt(&mut encoder, pipelines, resources);
    // `present` reports these per second, but a screenshot/offscreen
    // capture never goes through `present` — and for the full-map
    // exporter this pass *is* the whole scene, so it is the one place
    // the real sprite draw/bind counts can be observed.
    let atlas = resources.sprite_atlas.stats();
    // Distinct bank frames vs. cache entries: the gap is the cost of
    // keying the cache on (shadow_color, shadow_alpha) as well as the
    // frame, i.e. the same art baked twice at two shadow levels. It is
    // the only duplication a shader-side shadow resolve could remove,
    // so it is worth seeing next to the occupancy.
    let entries = resources.sprite_cache.entries.len();
    let distinct_frames = resources
        .sprite_cache
        .entries
        .keys()
        .map(|k| (k.bank_id, k.variant))
        .collect::<std::collections::HashSet<_>>()
        .len();
    tracing::info!(
        target: "fps",
        "capture {w}x{h}  quads={queued_draws}  drawcalls={}  binds={}  \
         atlas={}L/{:.1}MiB/{:.0}%occ/{:.0}%pack/{}spr  cache={entries}e/{distinct_frames}f",
        super::bind_counter::take_draw_calls(),
        super::bind_counter::take_count(),
        atlas.layers,
        atlas.bytes() as f32 / (1024.0 * 1024.0),
        atlas.occupancy() * 100.0,
        atlas.packing_efficiency() * 100.0,
        atlas.sprites,
    );
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

/// Read the render target left by the most recent `present()` without
/// submitting a new draw pass. This is used for screenshots of modal/UI
/// surfaces whose draw queue has already been consumed by presentation.
pub(super) fn capture_presented_frame_rgba(
    gpu: &GpuContext,
    frame: &FrameState,
) -> Option<(u32, u32, Vec<u8>)> {
    let (width, height) = frame.dimensions();
    let w = width as u32;
    let h = height as u32;
    if w == 0 || h == 0 {
        return None;
    }

    let bytes_per_pixel = 4u32;
    let unpadded_bpr = w * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bpr = unpadded_bpr.div_ceil(align) * align;
    let buffer_size = padded_bpr as u64 * h as u64;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("presented rt readback"),
        size: buffer_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("capture presented frame"),
        });
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
        tracing::warn!("capture_presented_frame_rgba: device.poll(Wait) failed");
        return None;
    }
    let mapped = match slice.get_mapped_range() {
        Ok(mapped) => mapped,
        Err(error) => {
            tracing::warn!(%error, "capture_presented_frame_rgba: failed to map buffer");
            buffer.unmap();
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
    Some((w, h, rgba))
}
