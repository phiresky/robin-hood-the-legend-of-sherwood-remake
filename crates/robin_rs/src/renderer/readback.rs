//! Logical-target readback with explicit submission, mapping and completion.

use super::{frame::FrameState, pipelines::PipelineStore, resources::GpuResources};
use crate::window::GpuContext;

pub type CapturedFrame = (u32, u32, Vec<u8>);

/// Owns the submitted buffer; completing it never borrows the live renderer.
pub type PendingCapture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<CapturedFrame, CaptureError>>>>;

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
    let started = web_time::Instant::now();
    let mapped = poll_mapping(
        receiver,
        || {
            if started.elapsed().as_secs() >= 30 {
                return Err(CaptureError::Timeout);
            }
            gpu.device
                .poll(wgpu::PollType::Poll)
                .map(|_| ())
                .map_err(|error| CaptureError::Poll(error.to_string()))
        },
        yield_mapping,
    )
    .await?;
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

async fn yield_mapping() {
    #[cfg(target_arch = "wasm32")]
    gloo_timers::future::TimeoutFuture::new(1).await;
    #[cfg(not(target_arch = "wasm32"))]
    {
        // The session polls each pending capture once per loop turn. Standalone
        // native tooling can still drive the same future with block_on.
        let mut yielded = false;
        futures::future::poll_fn(|cx| {
            if std::mem::replace(&mut yielded, true) {
                std::task::Poll::Ready(())
            } else {
                cx.waker().wake_by_ref();
                std::task::Poll::Pending
            }
        })
        .await;
    }
}

pub(super) fn begin_capture_frame_rgba(
    gpu: &GpuContext,
    pipelines: &PipelineStore,
    resources: &GpuResources,
    frame: &mut FrameState,
) -> PendingCapture {
    let submitted_at = web_time::Instant::now();
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
    tracing::debug!(
        elapsed_ms = submitted_at.elapsed().as_secs_f64() * 1000.0,
        "capture GPU: encode and submit"
    );
    let submitted_at = web_time::Instant::now();
    let gpu = gpu.clone();
    Box::pin(async move {
        let (buffer, layout) = submitted?;
        let captured = complete(&gpu, buffer, layout).await;
        tracing::debug!(
            elapsed_ms = submitted_at.elapsed().as_secs_f64() * 1000.0,
            "capture GPU: map and unpack"
        );
        captured
    })
}

pub(super) async fn capture_frame_rgba_async(
    gpu: &GpuContext,
    pipelines: &PipelineStore,
    resources: &GpuResources,
    frame: &mut FrameState,
) -> Result<CapturedFrame, CaptureError> {
    begin_capture_frame_rgba(gpu, pipelines, resources, frame).await
}

/// Reads the logical target from the last presentation without consuming
/// queued commands. This is deliberately not the postprocessed swapchain.
pub(super) async fn capture_presented_frame_rgba_async(
    gpu: &GpuContext,
    frame: &FrameState,
) -> Result<CapturedFrame, CaptureError> {
    begin_capture_presented_frame_rgba(gpu, frame).await
}

pub(super) fn begin_capture_presented_frame_rgba(
    gpu: &GpuContext,
    frame: &FrameState,
) -> PendingCapture {
    let encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("capture presented frame"),
        });
    let submitted = submit(gpu, frame, encoder);
    let gpu = gpu.clone();
    Box::pin(async move {
        let (buffer, layout) = submitted?;
        complete(&gpu, buffer, layout).await
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn capture_frame_rgba(
    gpu: &GpuContext,
    pipelines: &PipelineStore,
    resources: &GpuResources,
    frame: &mut FrameState,
) -> Result<CapturedFrame, CaptureError> {
    pollster::block_on(capture_frame_rgba_async(gpu, pipelines, resources, frame))
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn capture_presented_frame_rgba(
    gpu: &GpuContext,
    frame: &FrameState,
) -> Result<CapturedFrame, CaptureError> {
    pollster::block_on(capture_presented_frame_rgba_async(gpu, frame))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen_test::wasm_bindgen_test]
    #[ignore = "requires browser WebGL2; run capture tests with wasm-bindgen-test-runner --include-ignored"]
    async fn browser_gpu_captures_keep_submitted_pixels_across_event_loop_turns() {
        use std::sync::Arc;
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::GL;
        let instance = Arc::new(wgpu::Instance::new(descriptor));
        // Browser GL discovers its context through a canvas surface even when
        // the renderer itself only renders into an offscreen logical target.
        let canvas = web_sys::OffscreenCanvas::new(2, 2).expect("browser canvas");
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::OffscreenCanvas(canvas))
            .expect("browser GL surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect("browser must provide WebGL2 adapter");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("async capture browser test"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                    .using_resolution(adapter.limits()),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("browser WebGL2 device");
        let gpu = GpuContext {
            instance,
            adapter: Arc::new(adapter),
            device: Arc::new(device),
            queue: Arc::new(queue),
            surface_format: wgpu::TextureFormat::Rgba8UnormSrgb,
        };
        let mut renderer = crate::renderer::Renderer::offscreen(gpu, 2, 2);
        renderer.render_gpu_rect(0, 0, 2, 2, 255, 0, 0, 255);
        let red = renderer.begin_capture_frame_rgba();
        let presented_red = renderer.begin_capture_presented_frame_rgba();
        renderer.render_gpu_rect(0, 0, 2, 2, 0, 255, 0, 255);
        let green = renderer.begin_capture_frame_rgba();
        // All three submissions precede completion and reuse the logical
        // target. Returning to JS must not change any submitted frame.
        gloo_timers::future::TimeoutFuture::new(1).await;
        assert_eq!(red.await.unwrap(), (2, 2, [255, 0, 0, 255].repeat(4)));
        assert_eq!(
            presented_red.await.unwrap(),
            (2, 2, [255, 0, 0, 255].repeat(4))
        );
        assert_eq!(green.await.unwrap(), (2, 2, [0, 255, 0, 255].repeat(4)));
    }

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
