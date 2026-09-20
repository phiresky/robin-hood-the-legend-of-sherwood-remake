//! wgpu bring-up for the game window: surface creation, adapter/device
//! negotiation and the initial swapchain configuration.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use winit::window::Window;

use super::{GameWindow, GpuContext, HostCmd, HostMsg, SharedSurface};

/// Create a wgpu surface for `window` from the game thread.
///
/// On Windows, winit only hands out the window handle on the event-loop
/// thread, so the plain `create_surface` fails there. Use winit's
/// documented any-thread escape hatch and build the surface from the raw
/// handles instead. Every other platform uses the safe owning path.
pub(super) fn create_surface_any_thread(
    instance: &wgpu::Instance,
    window: Arc<Window>,
) -> Result<wgpu::Surface<'static>, wgpu::CreateSurfaceError> {
    #[cfg(not(target_os = "windows"))]
    {
        instance.create_surface(window)
    }
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::WindowExtWindows;
        // SAFETY: the handle is only passed to wgpu to create a swapchain
        // surface; wgpu never sends window messages through it, which is the
        // cross-thread hazard `window_handle_any_thread` guards against.
        let window_handle = match unsafe { window.window_handle_any_thread() } {
            Ok(handle) => handle.as_raw(),
            Err(e) => {
                // The zero-window sentinel never occurs for a live window,
                // and a dead window means we're shutting down anyway.
                panic!("window_handle_any_thread failed: {e}");
            }
        };
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(winit::raw_window_handle::RawDisplayHandle::Windows(
                winit::raw_window_handle::WindowsDisplayHandle::new(),
            )),
            raw_window_handle: window_handle,
        };
        // SAFETY: the HWND stays valid for the surface's lifetime because the
        // `Arc<Window>` is retained by the `AppHandler` and the process-wide
        // `GAME_WINDOW` slot until the event loop exits.
        unsafe { instance.create_surface_unsafe(target) }
    }
}

/// Backend selection per target.
fn instance_descriptor() -> wgpu::InstanceDescriptor {
    let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    // Native: PRIMARY (Vulkan / Metal / DX12).  Wasm: WebGPU + WebGL2
    // — WebGL2 is the fallback when the browser doesn't expose WebGPU
    // (most non-Chrome desktop browsers as of 2026).
    #[cfg(not(target_arch = "wasm32"))]
    {
        instance_descriptor.backends = wgpu::Backends::PRIMARY;
    }
    // The statically linked DXC shader compiler only exists for MSVC
    // targets, and DX12's FXC fallback cannot compile our binding_array
    // shaders (they need shader model 5.1+). Windows-gnu builds (used for
    // local Wine testing) therefore go through Vulkan instead of DX12.
    #[cfg(all(windows, target_env = "gnu"))]
    {
        instance_descriptor.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
    }
    #[cfg(target_arch = "wasm32")]
    {
        // wgpu 30 has a bug where mixing BROWSER_WEBGPU + GL causes
        // the WebGPU backend's `request_adapter` error to claim
        // `supported_backends = BROWSER_WEBGPU` only — masking the
        // GL backend even when wgpu-core/gles is compiled in (see
        // `wgpu-30.0.0/src/backend/webgpu.rs:1022`, where upstream still
        // notes that supported_backends should include compiled
        // wgpu-core backends). Pin to GL (= WebGL2 on wasm) for now
        // until that adapter-discovery path is fixed upstream.
        instance_descriptor.backends = wgpu::Backends::GL;
    }
    instance_descriptor
}

fn log_adapter_info(adapter: &wgpu::Adapter) {
    let info = adapter.get_info();
    tracing::info!(
        "wgpu adapter: {:?} backend={:?} type={:?} driver={:?}",
        info.name,
        info.backend,
        info.device_type,
        info.driver,
    );
    if info.device_type == wgpu::DeviceType::Cpu {
        tracing::warn!("wgpu picked a CPU (software) adapter — no real GPU acceleration");
    }
}

/// Negotiate limits/features for the adapter's backend and open the device.
async fn request_device(
    adapter: &wgpu::Adapter,
    surface: &wgpu::Surface<'_>,
) -> Result<(wgpu::Device, wgpu::Queue), String> {
    // WebGL2 lacks compute shaders, storage buffers, etc., so the
    // default `Limits` would fail `request_device` on the GL backend.
    // Drop to the WebGL2 baseline.  Native runs with full
    // `Limits::default()` and gets every feature the adapter
    // advertises.
    let required_limits = if adapter.get_info().backend == wgpu::Backend::Gl {
        wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits())
    } else {
        wgpu::Limits::default()
    };

    let mut required_features = wgpu::Features::empty();
    if adapter.get_info().backend != wgpu::Backend::Gl {
        let adapter_features = adapter.features();
        for feature in [
            wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER,
            wgpu::Features::PIPELINE_CACHE,
            wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
            wgpu::Features::FLOAT32_FILTERABLE,
        ] {
            if adapter_features.contains(feature) {
                required_features |= feature;
            } else {
                tracing::warn!(
                    "wgpu adapter does not expose {feature:?}; some shader presets may fail"
                );
            }
        }
    }

    let desc = wgpu::DeviceDescriptor {
        label: Some("robin device"),
        required_features,
        required_limits,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    };
    #[cfg(target_os = "linux")]
    match crate::vulkan_presentation::request_device(adapter, surface, &desc) {
        Ok(Some(device)) => return Ok(device),
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(%error, "Vulkan presentation profiling unavailable; opening standard device")
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = surface;
    adapter
        .request_device(&desc)
        .await
        .map_err(|e| format!("request_device: {e}"))
}

/// Pick the swapchain format and configure the surface at the window's
/// current physical size.
fn configure_initial_surface(
    window: &Window,
    surface: &wgpu::Surface<'static>,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    logical_w: u32,
    logical_h: u32,
) -> wgpu::SurfaceConfiguration {
    let surface_caps = surface.get_capabilities(adapter);
    let surface_format = surface_caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(surface_caps.formats[0]);

    let actual = window.inner_size();
    let surface_config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: actual.width.max(1),
        height: actual.height.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 1,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
    };
    #[cfg(target_os = "linux")]
    crate::vulkan_presentation::configure(surface, device, &surface_config);
    #[cfg(not(target_os = "linux"))]
    surface.configure(device, &surface_config);

    tracing::info!(
        present_mode = ?surface_config.present_mode,
        requested_maximum_frame_latency = surface_config.desired_maximum_frame_latency,
        "window: requested={}x{} actual_inner={}x{} surface={}x{} format={:?}",
        logical_w,
        logical_h,
        actual.width,
        actual.height,
        surface_config.width,
        surface_config.height,
        surface_format,
    );
    surface_config
}

/// Async wgpu bring-up: runs on the game side after `resumed()` ships
/// us the bare winit window.  `request_adapter` and `request_device`
/// genuinely yield on wasm, so they have to live on the async path
/// (not behind `pollster::block_on`).
pub(super) async fn build_game_window_async(
    window: Arc<Window>,
    logical_w: u32,
    logical_h: u32,
    events_rx: async_channel::Receiver<HostMsg>,
    cmd_tx: async_channel::Sender<HostCmd>,
    lifecycle_autosave_requested: Arc<AtomicBool>,
) -> Result<GameWindow, String> {
    let instance = wgpu::Instance::new(instance_descriptor());

    let surface = create_surface_any_thread(&instance, window.clone())
        .map_err(|e| format!("create_surface: {e}"))?;

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        })
        .await
        .map_err(|e| format!("request_adapter: {e}"))?;
    log_adapter_info(&adapter);

    let (device, queue) = request_device(&adapter, &surface).await?;

    let surface_config =
        configure_initial_surface(&window, &surface, &adapter, &device, logical_w, logical_h);

    let gpu = GpuContext {
        instance: Arc::new(instance),
        adapter: Arc::new(adapter),
        device: Arc::new(device),
        queue: Arc::new(queue),
        surface_format: surface_config.format,
    };

    #[cfg(feature = "gamepad")]
    let gamepads = match gilrs::Gilrs::new() {
        Ok(g) => Some(g),
        Err(e) => {
            tracing::warn!("gilrs init failed: {e:?}; gamepad input disabled");
            None
        }
    };

    Ok(GameWindow {
        width: logical_w,
        height: logical_h,
        gpu,
        surface: SharedSurface::new(surface),
        surface_config,
        #[cfg(feature = "gamepad")]
        gamepads,
        gamepad_input: Default::default(),
        local_players: Default::default(),
        close_requested: false,
        cursor_x: 0,
        cursor_y: 0,
        logical_w,
        logical_h,
        logical_resolution_policy: None,
        last_emitted_cursor: None,
        events_rx,
        cmd_tx,
        lifecycle_autosave_requested,
        deferred_event_batches: VecDeque::new(),
    })
}
