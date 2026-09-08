//! Logical-target readback with explicit submission, mapping and completion.

use super::{frame::FrameState, pipelines::PipelineStore, resources::GpuResources};
use crate::window::GpuContext;

pub type CapturedFrame = (u32, u32, Vec<u8>);

#[derive(Debug, thiserror::Error, serde::Serialize, serde::Deserialize)]
pub enum CaptureError {
    #[error("cannot capture a zero-sized render target")]
    EmptyTarget,
    #[error("capture dimensions exceed the supported byte layout")]
    InvalidLayout,
    #[error("GPU capture polling failed: {0}")]
    Poll(String),
    #[error("GPU readback did not complete within 30 seconds")]
    Timeout,
    #[error("GPU readback mapping failed: {0}")]
    Map(String),
    #[error("GPU readback mapping callback was dropped")]
    CompletionLost,
    #[error("browser capture requires the asynchronous capture API")]
    AsyncRequired,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct ReadbackLayout {
    width: u32,
    height: u32,
    row_bytes: u32,
    padded_row_bytes: u32,
}

impl ReadbackLayout {
    fn new(width: u32, height: u32) -> Result<Self, CaptureError> {
        if width == 0 || height == 0 {
            return Err(CaptureError::EmptyTarget);
        }
        let row_bytes = width.checked_mul(4).ok_or(CaptureError::InvalidLayout)?;
        let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_row_bytes = row_bytes
            .checked_add(alignment - 1)
            .ok_or(CaptureError::InvalidLayout)?
            / alignment
            * alignment;
        usize::try_from(u64::from(padded_row_bytes) * u64::from(height))
            .map_err(|_| CaptureError::InvalidLayout)?;
        Ok(Self {
            width,
            height,
            row_bytes,
            padded_row_bytes,
        })
    }

    fn unpack(self, mapped: &[u8]) -> Result<CapturedFrame, CaptureError> {
        let size = u64::from(self.padded_row_bytes) * u64::from(self.height);
        if mapped.len() as u64 != size {
            return Err(CaptureError::InvalidLayout);
        }
        let mut rgba = Vec::with_capacity(self.row_bytes as usize * self.height as usize);
        for row in mapped.chunks_exact(self.padded_row_bytes as usize) {
            rgba.extend_from_slice(&row[..self.row_bytes as usize]);
        }
        Ok((self.width, self.height, rgba))
    }
}

fn submit(
    gpu: &GpuContext,
    frame: &FrameState,
    mut encoder: wgpu::CommandEncoder,
) -> Result<(wgpu::Buffer, ReadbackLayout), CaptureError> {
    let (width, height) = frame.dimensions();
    let layout = ReadbackLayout::new(u32::from(width), u32::from(height))?;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("frame readback"),
        size: u64::from(layout.padded_row_bytes) * u64::from(layout.height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
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
                bytes_per_row: Some(layout.padded_row_bytes),
                rows_per_image: Some(layout.height),
            },
        },
        wgpu::Extent3d {
            width: layout.width,
            height: layout.height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(Some(encoder.finish()));
    Ok((buffer, layout))
}

/// Browser GL fences only progress after returning to the event loop. Polling
/// with Wait on that thread can time out even though the submitted work is valid.
#[cfg(any(target_arch = "wasm32", test))]
async fn poll_mapping<T, Y: std::future::Future<Output = ()>>(
    mut receiver: futures::channel::oneshot::Receiver<T>,
    mut poll: impl FnMut() -> Result<(), CaptureError>,
    mut yield_turn: impl FnMut() -> Y,
) -> Result<T, CaptureError> {
    loop {
        poll()?;
        if let Some(result) = receiver
            .try_recv()
            .map_err(|_| CaptureError::CompletionLost)?
        {
            return Ok(result);
        }
        yield_turn().await;
    }
}

async fn complete(
    gpu: &GpuContext,
    buffer: wgpu::Buffer,
    layout: ReadbackLayout,
) -> Result<CapturedFrame, CaptureError> {
    let (sender, receiver) = futures::channel::oneshot::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    #[cfg(not(target_arch = "wasm32"))]
    let mapped = {
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| CaptureError::Poll(error.to_string()))?;
        receiver.await.map_err(|_| CaptureError::CompletionLost)?
    };
    #[cfg(target_arch = "wasm32")]
    let mapped = {
        use futures::{future::Either, pin_mut};
        let completion = poll_mapping(
            receiver,
            || {
                gpu.device
                    .poll(wgpu::PollType::Poll)
                    .map(|_| ())
                    .map_err(|error| CaptureError::Poll(error.to_string()))
            },
            || gloo_timers::future::TimeoutFuture::new(1),
        );
        // A lost device normally reports through map_async. Also bound the wait
        // if a browser/driver never delivers that callback, without busy spinning.
        let timeout = gloo_timers::future::TimeoutFuture::new(30_000);
        pin_mut!(completion, timeout);
        match futures::future::select(completion, timeout).await {
            Either::Left((result, _)) => result?,
            Either::Right(_) => return Err(CaptureError::Timeout),
        }
    };
    mapped.map_err(|error| CaptureError::Map(error.to_string()))?;
    let result = {
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|error| CaptureError::Map(error.to_string()))?;
        layout.unpack(&mapped)
    };
    buffer.unmap();
    result
}

pub(super) async fn capture_frame_rgba_async(
    gpu: &GpuContext,
    pipelines: &PipelineStore,
    resources: &GpuResources,
    frame: &mut FrameState,
) -> Result<CapturedFrame, CaptureError> {
    frame.push_implicit_base_quad();
    frame.upload_queue_geometry(gpu);
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("capture frame"),
        });
    frame.encode_pass1_to_rt(&mut encoder, pipelines, resources);
    tracing::info!(target: "fps", residency = ?resources.sprite_residency_stats(),
        quads = frame.queued.len(), drawcalls = super::bind_counter::take_draw_calls(),
        binds = super::bind_counter::take_count(), "capture residency");
    let submitted = submit(gpu, frame, encoder);
    // Submission consumes commands even when subsequent mapping fails.
    frame.clear_recording();
    let (buffer, layout) = submitted?;
    complete(gpu, buffer, layout).await
}

/// Reads the logical target from the last presentation without consuming
/// queued commands. This is deliberately not the postprocessed swapchain.
pub(super) async fn capture_presented_frame_rgba_async(
    gpu: &GpuContext,
    frame: &FrameState,
) -> Result<CapturedFrame, CaptureError> {
    let encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("capture presented frame"),
        });
    let (buffer, layout) = submit(gpu, frame, encoder)?;
    complete(gpu, buffer, layout).await
}

pub(super) fn capture_frame_rgba(
    gpu: &GpuContext,
    pipelines: &PipelineStore,
    resources: &GpuResources,
    frame: &mut FrameState,
) -> Result<CapturedFrame, CaptureError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        pollster::block_on(capture_frame_rgba_async(gpu, pipelines, resources, frame))
    }
    #[cfg(target_arch = "wasm32")]
    {
        use futures::FutureExt;
        // Retain immediate-completion captures, but never block the browser
        // waiting for a GL fence. Dropping a pending future drops its buffer;
        // wgpu retains submitted resources, and the callback tolerates a dropped
        // receiver. No mapped view has been obtained at this point.
        // TODO: migrate browser screenshot consumers to async so they can also
        // capture when mapping requires another browser event-loop turn.
        capture_frame_rgba_async(gpu, pipelines, resources, frame)
            .now_or_never()
            .ok_or(CaptureError::AsyncRequired)?
    }
}

pub(super) fn capture_presented_frame_rgba(
    gpu: &GpuContext,
    frame: &FrameState,
) -> Result<CapturedFrame, CaptureError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        pollster::block_on(capture_presented_frame_rgba_async(gpu, frame))
    }
    #[cfg(target_arch = "wasm32")]
    {
        use futures::FutureExt;
        capture_presented_frame_rgba_async(gpu, frame)
            .now_or_never()
            .ok_or(CaptureError::AsyncRequired)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_polls_again_after_yielding_and_preserves_callback_result() {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let mut sender = Some(sender);
        let polls = std::cell::Cell::new(0);
        let yields = std::cell::Cell::new(0);
        let result = pollster::block_on(poll_mapping(
            receiver,
            || {
                polls.set(polls.get() + 1);
                assert_eq!(polls.get(), yields.get() + 1);
                if polls.get() == 3 {
                    sender.take().unwrap().send(42).unwrap();
                }
                Ok(())
            },
            || {
                yields.set(yields.get() + 1);
                std::future::ready(())
            },
        ));
        assert_eq!(result.unwrap(), 42);
        assert_eq!(yields.get(), 2);
    }

    #[test]
    fn mapping_reports_poll_failure_and_dropped_callback() {
        let (_sender, receiver) = futures::channel::oneshot::channel::<()>();
        assert!(matches!(
            pollster::block_on(poll_mapping(
                receiver,
                || Err(CaptureError::Poll("device lost".into())),
                || async { panic!("poll errors must not wait") },
            )),
            Err(CaptureError::Poll(_))
        ));
        let (sender, receiver) = futures::channel::oneshot::channel::<()>();
        drop(sender);
        assert!(matches!(
            pollster::block_on(poll_mapping(
                receiver,
                || Ok(()),
                || async { panic!("dropped callbacks must not wait") }
            )),
            Err(CaptureError::CompletionLost)
        ));
    }

    #[test]
    fn odd_width_rows_exclude_gpu_padding() {
        let layout = ReadbackLayout::new(3, 2).unwrap();
        let mut mapped = vec![99; layout.padded_row_bytes as usize * 2];
        mapped[..12].fill(1);
        mapped[256..268].fill(2);
        assert_eq!(
            layout.unpack(&mapped).unwrap(),
            (3, 2, [vec![1; 12], vec![2; 12]].concat())
        );
        assert!(matches!(
            layout.unpack(&mapped[..267]),
            Err(CaptureError::InvalidLayout)
        ));
    }

    #[test]
    fn rejects_empty_and_overflowing_targets() {
        assert!(matches!(
            ReadbackLayout::new(0, 4),
            Err(CaptureError::EmptyTarget)
        ));
        assert!(matches!(
            ReadbackLayout::new(u32::MAX, 1),
            Err(CaptureError::InvalidLayout)
        ));
    }
}
