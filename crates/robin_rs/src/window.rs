//! winit + wgpu window/event/GPU bootstrap, async-driven.
//!
//! Cross-target architecture (single-threaded on wasm, dual-threaded
//! on native):
//!
//! * The **main thread** owns winit's [`EventLoop`] and runs the
//!   [`AppHandler`] (an [`ApplicationHandler`]).  The handler creates
//!   the window + wgpu context inside `resumed()` and forwards every
//!   [`WindowEvent`] into an [`async_channel::Sender`].
//!
//! * The **game** runs as a `Future` consuming the matching
//!   [`async_channel::Receiver`].  On native the future is driven by
//!   `pollster::block_on` on a dedicated [`std::thread`]; on wasm it's
//!   driven by `wasm_bindgen_futures::spawn_local` on the same main
//!   JS thread that hosts winit.
//!
//! * [`GameWindow::poll_events`] is **synchronous** — it drains
//!   whatever the handler has buffered without awaiting.  The yield
//!   point lives in [`yield_to_runtime`] / [`sleep_ms`] (used by every
//!   per-frame pacing sleep), which the game calls inside its main
//!   loop. On wasm the yield races `setTimeout` with a page-lifecycle wake,
//!   so input can fire while visibility changes can trigger capture before
//!   hidden-page timer throttling; on native the game runs on its own thread
//!   and the yield is a no-op.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{
    ElementState, KeyEvent, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::gfx_types::{GameEvent, Keycode};
use crate::touch_input::{TouchClassifier, TouchOutput};
use robin_engine::graphic_config::GraphicConfig;

#[cfg(not(target_arch = "wasm32"))]
const GAME_THREAD_STACK_SIZE: usize = 8 * 1024 * 1024;

static NATIVE_REFRESH_PRESENTATION: AtomicBool = AtomicBool::new(true);

/// Wall-clock-ish millis since process start. Wraps at ~49 days,
/// which is fine for game pacing (used as a delta between frames).
pub fn process_uptime_ms() -> u32 {
    process_uptime().as_millis() as u32
}

/// Monotonic process time at microsecond precision for presentation pacing.
/// Simulation and replay timestamps intentionally remain millisecond based.
pub fn process_uptime_us() -> u64 {
    process_uptime().as_micros() as u64
}

fn process_uptime() -> web_time::Duration {
    static START: std::sync::OnceLock<web_time::Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(web_time::Instant::now);
    start.elapsed()
}

// ---------------------------------------------------------------------
// Async pacing helpers — the only yield points in the game loop.
// ---------------------------------------------------------------------

/// Yield once to the runtime so the [`AppHandler`] (running on the
/// main thread on wasm) gets a chance to drain pending JS events into
/// the game's event channel.  No-op on native — the game runs on a
/// dedicated thread and the [`ApplicationHandler`] runs on the main
/// thread, so they don't need cooperative scheduling.
pub async fn yield_to_runtime() {
    #[cfg(target_arch = "wasm32")]
    browser_wait_for_timer_or_lifecycle(0).await;
}

/// Wait for the browser compositor's next display refresh. Native wgpu FIFO
/// presentation applies equivalent back-pressure in `get_current_texture`, so
/// no additional host-thread delay is needed there.
pub async fn yield_to_display_refresh() {
    #[cfg(target_arch = "wasm32")]
    {
        use futures::future::select;

        let (sender, receiver) = futures::channel::oneshot::channel();
        let _frame = gloo_render::request_animation_frame(move |_| {
            let _ = sender.send(());
        });
        let fallback = gloo_timers::future::TimeoutFuture::new(20);
        futures::pin_mut!(receiver, fallback);
        // Browsers suspend requestAnimationFrame for hidden tabs. Gameplay,
        // especially multiplayer, must keep its fixed-step future alive; the
        // timer wins in that state and dropping `_frame` cancels the request.
        let _ = select(receiver, fallback).await;
    }
}

/// Async sleep used by every per-frame pacing point in the game loop.
/// Native: blocks the dedicated game thread via [`std::thread::sleep`].
/// Wasm: yields via `setTimeout(<ms>)`, unless a lifecycle autosave edge wakes
/// it first.
pub async fn sleep_ms(ms: u64) {
    #[cfg(target_arch = "wasm32")]
    {
        let ms_u32 = ms.min(u32::MAX as u64) as u32;
        browser_wait_for_timer_or_lifecycle(ms_u32).await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::sleep(Duration::from_millis(ms));
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static BROWSER_LIFECYCLE_WAKE_RX: std::cell::RefCell<Option<async_channel::Receiver<()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(target_arch = "wasm32")]
async fn browser_wait_for_timer_or_lifecycle(ms: u32) {
    use futures::{FutureExt as _, pin_mut, select};
    let wake = BROWSER_LIFECYCLE_WAKE_RX.with(|slot| slot.borrow().clone());
    let Some(wake) = wake else {
        gloo_timers::future::TimeoutFuture::new(ms).await;
        return;
    };
    let timer = gloo_timers::future::TimeoutFuture::new(ms).fuse();
    let lifecycle = wake.recv().fuse();
    pin_mut!(timer, lifecycle);
    select! {
        _ = timer => {},
        _ = lifecycle => {},
    }
}

#[cfg(target_arch = "wasm32")]
fn install_browser_lifecycle_autosave(requested: Arc<AtomicBool>) -> Result<(), String> {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;

    let window = web_sys::window().ok_or("browser window is unavailable")?;
    let document = window.document().ok_or("browser document is unavailable")?;
    let (wake_tx, wake_rx) = async_channel::bounded(1);
    BROWSER_LIFECYCLE_WAKE_RX.with(|slot| {
        if slot.borrow().is_some() {
            return Err("browser lifecycle autosave listener was installed twice".to_owned());
        }
        *slot.borrow_mut() = Some(wake_rx);
        Ok(())
    })?;

    let visibility_requested = requested.clone();
    let visibility_wake = wake_tx.clone();
    let visibility = Closure::<dyn FnMut()>::new(move || {
        let hidden = web_sys::window()
            .and_then(|window| window.document())
            .is_some_and(|document| document.hidden());
        if hidden {
            visibility_requested.store(true, Ordering::Release);
            let _ = visibility_wake.try_send(());
        }
    });
    document
        .add_event_listener_with_callback("visibilitychange", visibility.as_ref().unchecked_ref())
        .map_err(|error| format!("installing visibilitychange autosave listener: {error:?}"))?;
    visibility.forget();

    let pagehide_requested = requested.clone();
    let pagehide = Closure::<dyn FnMut()>::new(move || {
        pagehide_requested.store(true, Ordering::Release);
        let _ = wake_tx.try_send(());
    });
    window
        .add_event_listener_with_callback("pagehide", pagehide.as_ref().unchecked_ref())
        .map_err(|error| format!("installing pagehide autosave listener: {error:?}"))?;
    pagehide.forget();

    if document.hidden() {
        requested.store(true, Ordering::Release);
    }
    Ok(())
}

/// Pace a UI-owned render loop. Vsync presentation is itself the clock when
/// native-refresh mode is enabled; the legacy/off path retains the historical
/// 16 ms sleep.
pub async fn sleep_ui_frame() {
    if NATIVE_REFRESH_PRESENTATION.load(Ordering::Relaxed) {
        yield_to_display_refresh().await;
    } else {
        sleep_ms(16).await;
    }
}

// ---------------------------------------------------------------------
// GPU context shared by the renderer and the upscale pipeline.
// ---------------------------------------------------------------------

/// All wgpu plumbing the rest of the renderer needs. Cheaply cloneable
/// via the inner `Arc`s.
#[derive(Clone)]
pub struct GpuContext {
    pub instance: Arc<wgpu::Instance>,
    pub adapter: Arc<wgpu::Adapter>,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub surface_format: wgpu::TextureFormat,
}

/// Shared, replaceable swapchain surface.
///
/// Android can destroy and recreate the native window while keeping
/// the Rust game thread and renderers alive. wgpu surfaces are bound
/// to that native window, so renderers must see the replacement
/// surface without being rebuilt.
#[derive(Clone)]
pub struct SharedSurface {
    inner: Arc<std::sync::Mutex<Option<wgpu::Surface<'static>>>>,
}

impl SharedSurface {
    fn new(surface: wgpu::Surface<'static>) -> Self {
        Self {
            inner: Arc::new(std::sync::Mutex::new(Some(surface))),
        }
    }

    pub fn configure(&self, device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) {
        self.inner
            .lock()
            .expect("surface mutex poisoned")
            .as_ref()
            .expect("surface missing")
            .configure(device, config);
    }

    pub fn get_current_texture(&self) -> wgpu::CurrentSurfaceTexture {
        self.inner
            .lock()
            .expect("surface mutex poisoned")
            .as_ref()
            .expect("surface missing")
            .get_current_texture()
    }

    fn replace(&self, surface: wgpu::Surface<'static>) {
        let mut guard = self.inner.lock().expect("surface mutex poisoned");
        #[cfg(target_os = "android")]
        if let Some(old_surface) = guard.take() {
            // Android/wgpu 29.0.1: dropping a Vulkan surface after the
            // ANativeWindow has been destroyed can still abort inside
            // wgpu-core's Surface::drop -> surface_drop path. Keep this
            // deliberate leak until wgpu/Android exposes a destructor path
            // that is safe after winit's suspended/resumed window churn.
            std::mem::forget(old_surface);
        }
        *guard = Some(surface);
    }
}

#[cfg(target_os = "android")]
impl Drop for SharedSurface {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1
            && let Some(surface) = self.inner.lock().expect("surface mutex poisoned").take()
        {
            // Android/wgpu 29.0.1: see `replace`. During process
            // shutdown this is preferable to a destructor abort while the
            // Java Activity/native window is already being torn down.
            std::mem::forget(surface);
        }
    }
}

// ---------------------------------------------------------------------
// Channel messages between the AppHandler (main thread) and the game.
// ---------------------------------------------------------------------

/// Messages flowing from the main-thread [`AppHandler`] into the game.
enum HostMsg {
    /// A regular input event the game should consume.
    Event(GameEvent),
    /// Touch taps are recognized on physical release, but widgets need to
    /// observe a pushed frame before the release frame. Queue their matching
    /// mouse-up for the following game-side poll.
    DeferredEvent(GameEvent),
    /// Window resized — both the new physical size and a new
    /// [`SurfaceConfiguration`] are computed on the main thread and
    /// pushed through.  The game side calls `surface.configure` to
    /// apply.
    Resized {
        width: u32,
        height: u32,
    },
    SurfaceReady {
        window: Arc<Window>,
    },
    /// Native window focus loss is an autosave boundary. Browser page
    /// lifecycle events signal the same atomic directly because a throttled
    /// page may not run another winit event drain first.
    LifecycleAutosave,
}

#[cfg(target_os = "android")]
static ANDROID_BACK_TX: std::sync::OnceLock<
    std::sync::Mutex<Option<async_channel::Sender<HostMsg>>>,
> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
fn android_back_tx() -> &'static std::sync::Mutex<Option<async_channel::Sender<HostMsg>>> {
    ANDROID_BACK_TX.get_or_init(|| std::sync::Mutex::new(None))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_phiresky_robinhood_RobinHoodActivity_nativeOnBackPressed(
    _env: *mut std::ffi::c_void,
    _this: *mut std::ffi::c_void,
) {
    tracing::info!("Android Back pressed");
    if let Some(tx) = android_back_tx()
        .lock()
        .expect("android back tx poisoned")
        .as_ref()
    {
        let _ = tx.try_send(HostMsg::Event(GameEvent::MenuToggleRequested));
    }
}

/// Process-wide handle on the live winit [`Window`].  Populated when
/// the OS window is created so the game thread can reach the window
/// for fire-and-forget calls like [`Window::reset_dead_keys`] without
/// round-tripping through the [`HostCmd`] queue.  The cmd queue is
/// only drained at `about_to_wait`, which is too late for dead-key
/// resets — by the time it runs, the next keypress has already been
/// composed.
static GAME_WINDOW: std::sync::OnceLock<std::sync::Mutex<Option<Arc<Window>>>> =
    std::sync::OnceLock::new();

fn game_window_slot() -> &'static std::sync::Mutex<Option<Arc<Window>>> {
    GAME_WINDOW.get_or_init(|| std::sync::Mutex::new(None))
}

fn set_game_window(window: Arc<Window>) {
    *game_window_slot().lock().expect("game window poisoned") = Some(window);
}

fn with_game_window<F: FnOnce(&Window)>(f: F) {
    if let Some(w) = game_window_slot()
        .lock()
        .expect("game window poisoned")
        .as_ref()
    {
        f(w);
    }
}

/// Commands flowing from the game out to the [`AppHandler`] / window.
/// Picked up on the next `about_to_wait` / `new_events` callback.
pub(crate) enum HostCmd {
    GrabMouse(bool),
    Exit,
}

// ---------------------------------------------------------------------
// GameWindow — the handle the game-side code holds.
// ---------------------------------------------------------------------

/// Owns the wgpu device/surface and the receiving end of the event
/// channel.  Created on `resumed()` and handed off into the game
/// future via the closure passed to [`run_with_game`].
pub struct GameWindow {
    /// Current logical game-canvas size. Physical swapchain dimensions live
    /// in [`Self::surface_config`] and may be much larger.
    pub width: u32,
    pub height: u32,
    pub gpu: GpuContext,
    pub surface: SharedSurface,
    pub surface_config: wgpu::SurfaceConfiguration,
    #[cfg(feature = "gamepad")]
    pub gamepads: Option<gilrs::Gilrs>,
    pub active_gamepad: Option<u32>,
    pub close_requested: bool,
    cursor_x: i32,
    cursor_y: i32,
    logical_w: u32,
    logical_h: u32,
    logical_resolution_policy: Option<GraphicConfig>,
    last_emitted_cursor: Option<(i32, i32)>,
    events_rx: async_channel::Receiver<HostMsg>,
    cmd_tx: async_channel::Sender<HostCmd>,
    lifecycle_autosave_requested: Arc<AtomicBool>,
    /// Ordered polling batches separated by synthetic touch releases. A tap
    /// release is held until the next poll, while later input may join that
    /// release's batch only until the next release barrier.
    deferred_event_batches: VecDeque<Vec<GameEvent>>,
}

/// Aspect-preserving placement of a logical canvas inside a physical surface.
///
/// Presentation and pointer conversion must use the exact same geometry or a
/// cursor near a letterbox edge can disagree with the pixel under it. Keep the
/// calculation here and share it with the renderer instead of maintaining two
/// subtly different copies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PresentationRect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl PresentationRect {
    pub(crate) fn aspect_fit(
        logical_width: u32,
        logical_height: u32,
        surface_width: u32,
        surface_height: u32,
    ) -> Self {
        assert!(
            logical_width > 0,
            "logical presentation width must be non-zero"
        );
        assert!(
            logical_height > 0,
            "logical presentation height must be non-zero"
        );
        assert!(
            surface_width > 0,
            "surface presentation width must be non-zero"
        );
        assert!(
            surface_height > 0,
            "surface presentation height must be non-zero"
        );

        let logical_width = logical_width as f32;
        let logical_height = logical_height as f32;
        let surface_width = surface_width as f32;
        let surface_height = surface_height as f32;
        let logical_aspect = logical_width / logical_height;
        let surface_aspect = surface_width / surface_height;
        let (width, height) = if surface_aspect >= logical_aspect {
            (surface_height * logical_aspect, surface_height)
        } else {
            (surface_width, surface_width / logical_aspect)
        };
        Self {
            x: (surface_width - width) * 0.5,
            y: (surface_height - height) * 0.5,
            width,
            height,
        }
    }
}

impl GameWindow {
    /// Peek without consuming: a transient save/load boundary must not lose a
    /// browser background request before a snapshot can safely be captured.
    pub(crate) fn lifecycle_autosave_requested(&self) -> bool {
        self.lifecycle_autosave_requested.load(Ordering::Acquire)
    }

    /// Acknowledge only after the snapshot was accepted by persistence, or
    /// after policy deliberately excludes this session from autosaving.
    pub(crate) fn acknowledge_lifecycle_autosave_request(&self) {
        self.lifecycle_autosave_requested
            .store(false, Ordering::Release);
    }

    /// Clear and present the swapchain surface directly, without the
    /// logical renderer. Used by pre-engine wait loops that need to keep the
    /// native/browser window visibly painted before a Renderer exists.
    pub fn clear_to_color(&mut self, color: wgpu::Color) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            status => {
                tracing::warn!("window clear: get_current_texture: {status:?}");
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("window clear"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("window clear pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
        }
        self.gpu.queue.submit(Some(encoder.finish()));
        self.gpu.queue.present(frame);
    }

    /// Drain pending events that the [`AppHandler`] has buffered into
    /// the channel since the last call.  Synchronous — no `.await`.
    /// The corresponding yield point is [`sleep_ms`] / [`yield_to_runtime`],
    /// which the game's main loop calls every frame.
    pub fn poll_events(&mut self) -> Vec<GameEvent> {
        let mut events = self.deferred_event_batches.pop_front().unwrap_or_default();
        let mut defer_following_events = !self.deferred_event_batches.is_empty();
        while let Ok(msg) = self.events_rx.try_recv() {
            match msg {
                HostMsg::Event(e) => {
                    if matches!(&e, GameEvent::Quit) {
                        self.close_requested = true;
                    }
                    if defer_following_events {
                        self.deferred_event_batches
                            .back_mut()
                            .expect("deferred event batch missing after barrier")
                            .push(e);
                    } else {
                        events.push(e);
                    }
                }
                HostMsg::Resized { width, height } => {
                    self.surface_config.width = width.max(1);
                    self.surface_config.height = height.max(1);
                    self.surface
                        .configure(&self.gpu.device, &self.surface_config);
                    // A minimized native window commonly reports 0x0. Keep
                    // the last usable logical canvas until it is restored;
                    // the 1x1 swapchain is only a presentation placeholder.
                    if width > 0 && height > 0 {
                        self.recompute_logical_resolution();
                    }
                    let event = GameEvent::Resized(width, height);
                    if defer_following_events {
                        self.deferred_event_batches
                            .back_mut()
                            .expect("deferred resize batch missing after barrier")
                            .push(event);
                    } else {
                        events.push(event);
                    }
                }
                HostMsg::SurfaceReady { window } => {
                    match create_surface_any_thread(&self.gpu.instance, window) {
                        Ok(surface) => {
                            self.surface.replace(surface);
                            self.surface
                                .configure(&self.gpu.device, &self.surface_config);
                            tracing::info!("wgpu surface recreated after resume");
                        }
                        Err(e) => tracing::error!("recreate surface: {e}"),
                    }
                }
                HostMsg::LifecycleAutosave => {
                    self.lifecycle_autosave_requested
                        .store(true, Ordering::Release);
                }
                HostMsg::DeferredEvent(event) => {
                    // Preserve channel order after the synthetic tap-up
                    // barrier. A subsequent fast tap must not overtake the
                    // first release merely because both arrived in one poll.
                    defer_following_events = true;
                    self.deferred_event_batches.push_back(vec![event]);
                }
            }
        }

        // Carry cursor-position deltas across drains so MouseDown/Up
        // events that didn't include explicit coords can still find
        // the latest sampled position.
        for ev in events.iter_mut() {
            match ev {
                GameEvent::MouseMove { x, y, xrel, yrel } => {
                    tracing::trace!("game-side MouseMove drained: ({x}, {y})");
                    let prev = self
                        .last_emitted_cursor
                        .unwrap_or((self.cursor_x, self.cursor_y));
                    let raw_xrel = *x - prev.0;
                    let raw_yrel = *y - prev.1;
                    self.last_emitted_cursor = Some((*x, *y));
                    self.cursor_x = *x;
                    self.cursor_y = *y;
                    let scale = self.window_pixel_to_logical_scale();
                    let (lx, ly) = self.window_to_logical(*x, *y);
                    *x = lx;
                    *y = ly;
                    *xrel = (raw_xrel as f32 * scale.0) as i32;
                    *yrel = (raw_yrel as f32 * scale.1) as i32;
                }
                GameEvent::MouseDown(x, y, _, _) | GameEvent::MouseUp(x, y, _) => {
                    let (lx, ly) = self.window_to_logical(*x, *y);
                    *x = lx;
                    *y = ly;
                }
                GameEvent::ViewportPan { xrel, yrel } => {
                    let scale = self.window_pixel_to_logical_scale();
                    *xrel = (*xrel as f32 * scale.0) as i32;
                    *yrel = (*yrel as f32 * scale.1) as i32;
                }
                GameEvent::TouchTransformStart {
                    first_x,
                    first_y,
                    second_x,
                    second_y,
                } => {
                    (*first_x, *first_y) = self.window_to_logical_f32(*first_x, *first_y);
                    (*second_x, *second_y) = self.window_to_logical_f32(*second_x, *second_y);
                }
                GameEvent::TouchTransform {
                    centroid_x,
                    centroid_y,
                    pan_x,
                    pan_y,
                    velocity_x,
                    velocity_y,
                    ..
                } => {
                    (*centroid_x, *centroid_y) =
                        self.window_to_logical_f32(*centroid_x, *centroid_y);
                    let scale = self.window_pixel_to_logical_scale();
                    *pan_x *= scale.0;
                    *pan_y *= scale.1;
                    *velocity_x *= scale.0;
                    *velocity_y *= scale.1;
                }
                GameEvent::TouchTransformEnd {
                    velocity_x,
                    velocity_y,
                    ..
                } => {
                    let scale = self.window_pixel_to_logical_scale();
                    *velocity_x *= scale.0;
                    *velocity_y *= scale.1;
                }
                _ => {}
            }
        }

        // Drain gilrs events to GameEvent::Gamepad{Added,Removed,Button,Axis}.
        #[cfg(feature = "gamepad")]
        if let Some(gilrs) = &mut self.gamepads {
            while let Some(gilrs::Event { id, event, .. }) = gilrs.next_event() {
                let which = usize::from(id) as u32;
                match event {
                    gilrs::EventType::Connected => {
                        if self.active_gamepad.is_none() {
                            self.active_gamepad = Some(which);
                        }
                        events.push(GameEvent::GamepadAdded { which });
                    }
                    gilrs::EventType::Disconnected => {
                        if self.active_gamepad == Some(which) {
                            self.active_gamepad = None;
                        }
                        events.push(GameEvent::GamepadRemoved { which });
                    }
                    gilrs::EventType::ButtonPressed(btn, _) => {
                        if let Some(b) = gilrs_button_to_index(btn) {
                            events.push(GameEvent::GamepadButton {
                                which,
                                button: b,
                                pressed: true,
                            });
                        }
                    }
                    gilrs::EventType::ButtonReleased(btn, _) => {
                        if let Some(b) = gilrs_button_to_index(btn) {
                            events.push(GameEvent::GamepadButton {
                                which,
                                button: b,
                                pressed: false,
                            });
                        }
                    }
                    gilrs::EventType::AxisChanged(axis, value, _) => {
                        if let Some(a) = gilrs_axis_to_index(axis) {
                            let v = (value * 32767.0).clamp(-32768.0, 32767.0) as i16;
                            events.push(GameEvent::GamepadAxis {
                                which,
                                axis: a,
                                value: v,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }

        events
    }

    pub fn grab_mouse(&mut self, grab: bool) {
        let _ = self.cmd_tx.try_send(HostCmd::GrabMouse(grab));
    }

    pub fn cursor_pos(&self) -> (i32, i32) {
        self.window_to_logical(self.cursor_x, self.cursor_y)
    }

    pub fn set_logical_size(&mut self, w: u32, h: u32) {
        self.logical_w = w.max(1);
        self.logical_h = h.max(1);
        self.width = self.logical_w;
        self.height = self.logical_h;
    }

    /// Install a profile-backed logical-resolution policy and immediately
    /// fit it to the current physical surface. Calling this after a graphics
    /// setting change keeps input conversion and rendering on one canvas.
    pub fn set_logical_resolution_policy(&mut self, config: &GraphicConfig) {
        self.logical_resolution_policy = Some(config.clone());
        self.recompute_logical_resolution();
    }

    /// Current physical swapchain size, independent of the logical game
    /// canvas returned by [`Self::logical_size`].
    pub fn surface_size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    pub fn logical_size(&self) -> (u32, u32) {
        (self.logical_w, self.logical_h)
    }

    fn recompute_logical_resolution(&mut self) {
        let Some(config) = self.logical_resolution_policy.as_ref() else {
            return;
        };
        let (width, height) = config
            .logical_resolution_for_surface(self.surface_config.width, self.surface_config.height);
        self.logical_w = u32::from(width);
        self.logical_h = u32::from(height);
        self.width = self.logical_w;
        self.height = self.logical_h;
    }

    pub fn set_native_refresh_presentation(&mut self, enabled: bool) {
        NATIVE_REFRESH_PRESENTATION.store(enabled, Ordering::Relaxed);
        let present_mode = if enabled {
            wgpu::PresentMode::Fifo
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        if self.surface_config.present_mode == present_mode {
            return;
        }
        self.surface_config.present_mode = present_mode;
        self.surface
            .configure(&self.gpu.device, &self.surface_config);
        tracing::info!(?present_mode, "updated swapchain presentation mode");
    }

    pub fn window_to_logical(&self, x: i32, y: i32) -> (i32, i32) {
        let (lx, ly) = self.window_to_logical_f32(x as f32, y as f32);
        (lx as i32, ly as i32)
    }

    fn window_to_logical_f32(&self, x: f32, y: f32) -> (f32, f32) {
        let rect = PresentationRect::aspect_fit(
            self.logical_w,
            self.logical_h,
            self.surface_config.width.max(1),
            self.surface_config.height.max(1),
        );
        let lx = (x - rect.x) / rect.width * self.logical_w as f32;
        let ly = (y - rect.y) / rect.height * self.logical_h as f32;
        (lx, ly)
    }

    fn window_pixel_to_logical_scale(&self) -> (f32, f32) {
        let rect = PresentationRect::aspect_fit(
            self.logical_w,
            self.logical_h,
            self.surface_config.width.max(1),
            self.surface_config.height.max(1),
        );
        (
            self.logical_w as f32 / rect.width,
            self.logical_h as f32 / rect.height,
        )
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::PresentationRect;

    fn assert_rect_close(actual: PresentationRect, expected: PresentationRect) {
        for (actual, expected) in [
            (actual.x, expected.x),
            (actual.y, expected.y),
            (actual.width, expected.width),
            (actual.height, expected.height),
        ] {
            assert!(
                (actual - expected).abs() < 0.01,
                "presentation coordinate {actual} differs from {expected}"
            );
        }
    }

    #[test]
    fn aspect_fit_letterboxes_wider_surfaces() {
        let rect = PresentationRect::aspect_fit(1280, 720, 3440, 1440);
        assert_rect_close(
            rect,
            PresentationRect {
                x: 440.0,
                y: 0.0,
                width: 2560.0,
                height: 1440.0,
            },
        );
    }

    #[test]
    fn aspect_fit_letterboxes_taller_surfaces() {
        let rect = PresentationRect::aspect_fit(1024, 768, 1920, 1080);
        assert_rect_close(
            rect,
            PresentationRect {
                x: 240.0,
                y: 0.0,
                width: 1440.0,
                height: 1080.0,
            },
        );
    }
}

// ---------------------------------------------------------------------
// AppHandler — winit ApplicationHandler driving the event channel.
// ---------------------------------------------------------------------

type WindowReadyFn = Box<dyn FnMut(Arc<Window>) + 'static>;

/// Create a wgpu surface for `window` from the game thread.
///
/// On Windows, winit only hands out the window handle on the event-loop
/// thread, so the plain `create_surface` fails there. Use winit's
/// documented any-thread escape hatch and build the surface from the raw
/// handles; the `Arc<Window>` held by `GameWindow` keeps them valid for
/// the surface's lifetime.
#[cfg(target_os = "windows")]
fn create_surface_any_thread(
    instance: &wgpu::Instance,
    window: Arc<Window>,
) -> Result<wgpu::Surface<'static>, wgpu::CreateSurfaceError> {
    use winit::platform::windows::WindowExtWindows;
    unsafe {
        let window_handle = match window.window_handle_any_thread() {
            Ok(handle) => handle.as_raw(),
            Err(e) => {
                // The zero-window sentinel never occurs for a live window,
                // and a dead window means we're shutting down anyway.
                panic!("window_handle_any_thread failed: {e}");
            }
        };
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(winit::raw_window_handle::RawDisplayHandle::Windows(
                winit::raw_window_handle::WindowsDisplayHandle::new(),
            )),
            raw_window_handle: window_handle,
        })
    }
}

#[cfg(not(target_os = "windows"))]
fn create_surface_any_thread(
    instance: &wgpu::Instance,
    window: Arc<Window>,
) -> Result<wgpu::Surface<'static>, wgpu::CreateSurfaceError> {
    instance.create_surface(window)
}

/// Async wgpu bring-up: runs on the game side after `resumed()` ships
/// us the bare winit window.  `request_adapter` and `request_device`
/// genuinely yield on wasm, so they have to live on the async path
/// (not behind `pollster::block_on`).
async fn build_game_window_async(
    window: Arc<Window>,
    logical_w: u32,
    logical_h: u32,
    events_rx: async_channel::Receiver<HostMsg>,
    cmd_tx: async_channel::Sender<HostCmd>,
    lifecycle_autosave_requested: Arc<AtomicBool>,
) -> Result<GameWindow, String> {
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
    let instance = wgpu::Instance::new(instance_descriptor);

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

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("robin device"),
            required_features,
            required_limits,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        })
        .await
        .map_err(|e| format!("request_device: {e}"))?;

    let surface_caps = surface.get_capabilities(&adapter);
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
        desired_maximum_frame_latency: 2,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
    };
    surface.configure(&device, &surface_config);

    tracing::info!(
        "window: requested={}x{} actual_inner={}x{} surface={}x{} format={:?}",
        logical_w,
        logical_h,
        actual.width,
        actual.height,
        surface_config.width,
        surface_config.height,
        surface_format,
    );

    let gpu = GpuContext {
        instance: Arc::new(instance),
        adapter: Arc::new(adapter),
        device: Arc::new(device),
        queue: Arc::new(queue),
        surface_format,
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
        active_gamepad: None,
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

/// Wall-clock window for two presses to register as a double-click.
/// 15 frames at ~60fps → ~250ms.  winit does not surface a multi-click
/// count, so the handler emits `clicks=2` itself when the second press
/// of the same button arrives within this window.
const DOUBLE_CLICK_INTERVAL_MS: u128 = 250;

pub struct AppHandler {
    title: String,
    width: u32,
    height: u32,
    visible: bool,
    /// Sender the handler pushes events into.
    events_tx: async_channel::Sender<HostMsg>,
    cmd_rx: async_channel::Receiver<HostCmd>,
    /// User callback that gets the bare winit `Window` once the OS
    /// window is up.  All wgpu init happens on the game side, async.
    on_window_ready: WindowReadyFn,
    window: Option<Arc<Window>>,
    last_cursor: (i32, i32),
    touch: TouchClassifier,
    #[cfg(target_os = "android")]
    resize_refresh_frames: u8,
    /// Per-button (button code, press timestamp) of the most recent press
    /// — used to detect double-clicks since winit doesn't surface a
    /// multi-click count.  Each entry is consumed (cleared) when it
    /// produces a double-click so a triple-press doesn't chain.
    last_press: Option<(u8, web_time::Instant)>,
}

fn touch_output_needs_deferred_up(output: &[TouchOutput]) -> bool {
    output
        .iter()
        .any(|event| matches!(event, TouchOutput::PointerDown { .. }))
}

impl AppHandler {
    #[cfg(target_os = "android")]
    fn send_menu_toggle_request(&self) {
        let _ = self
            .events_tx
            .try_send(HostMsg::Event(GameEvent::MenuToggleRequested));
    }

    fn send_pause_request(&self) {
        let _ = self
            .events_tx
            .try_send(HostMsg::Event(GameEvent::PauseRequested));
    }

    fn emit_touch_outputs(&mut self, output: Vec<TouchOutput>) {
        // Only a release-classified interaction can emit Down and Up in the
        // same winit callback. Give that synthetic press one game-side poll
        // of lifetime for widget/input state. An ordinary drag already
        // emitted Down on an earlier move, so its release remains immediate.
        let defer_pointer_up = touch_output_needs_deferred_up(&output);
        for event in output {
            let event = match event {
                TouchOutput::MotionStop => GameEvent::TouchMotionStop,
                TouchOutput::PointerMove { x, y } => {
                    self.last_cursor = (x as i32, y as i32);
                    GameEvent::MouseMove {
                        x: x as i32,
                        y: y as i32,
                        xrel: 0,
                        yrel: 0,
                    }
                }
                TouchOutput::PointerDown { x, y, clicks } => {
                    self.last_cursor = (x as i32, y as i32);
                    GameEvent::MouseDown(x as i32, y as i32, 1, clicks)
                }
                TouchOutput::PointerUp { x, y } => {
                    self.last_cursor = (x as i32, y as i32);
                    let mouse_up = GameEvent::MouseUp(x as i32, y as i32, 1);
                    let _ = if defer_pointer_up {
                        self.events_tx.try_send(HostMsg::DeferredEvent(mouse_up))
                    } else {
                        self.events_tx.try_send(HostMsg::Event(mouse_up))
                    };
                    continue;
                }
                TouchOutput::PointerCancel => GameEvent::PointerCancel,
                TouchOutput::TransformStart {
                    first_x,
                    first_y,
                    second_x,
                    second_y,
                } => GameEvent::TouchTransformStart {
                    first_x: first_x as f32,
                    first_y: first_y as f32,
                    second_x: second_x as f32,
                    second_y: second_y as f32,
                },
                TouchOutput::TransformUpdate {
                    centroid_x,
                    centroid_y,
                    pan_x,
                    pan_y,
                    scale,
                    velocity_x,
                    velocity_y,
                } => GameEvent::TouchTransform {
                    centroid_x: centroid_x as f32,
                    centroid_y: centroid_y as f32,
                    pan_x: pan_x as f32,
                    pan_y: pan_y as f32,
                    scale: scale as f32,
                    velocity_x: velocity_x as f32,
                    velocity_y: velocity_y as f32,
                },
                TouchOutput::TransformEnd {
                    velocity_x,
                    velocity_y,
                    cancelled,
                } => GameEvent::TouchTransformEnd {
                    velocity_x: velocity_x as f32,
                    velocity_y: velocity_y as f32,
                    cancelled,
                },
            };
            let _ = self.events_tx.try_send(HostMsg::Event(event));
        }
    }

    fn process_cmds(&mut self) {
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            match cmd {
                HostCmd::GrabMouse(grab) => {
                    if let Some(w) = &self.window {
                        let _ = w.set_cursor_grab(if grab {
                            winit::window::CursorGrabMode::Confined
                        } else {
                            winit::window::CursorGrabMode::None
                        });
                    }
                }
                HostCmd::Exit => {
                    // Handled in `about_to_wait` via the ActiveEventLoop.
                    // Mark by closing the events channel so the game
                    // wakes; the actual exit() needs the loop ref.
                    // Best effort here — the loop will pick this up
                    // when about_to_wait next fires.
                    self.events_tx.close();
                }
            }
        }
    }
}

impl ApplicationHandler for AppHandler {
    fn user_event(&mut self, event_loop: &ActiveEventLoop, (): ()) {
        self.process_cmds();
        if self.events_tx.is_closed() {
            event_loop.exit();
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            #[cfg(target_os = "android")]
            window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
            #[cfg(target_os = "android")]
            {
                // Android can suspend the native window while the game
                // thread is still building the initial wgpu surface. If
                // that happens, the init loop waits for a fresh resumed
                // window on the same channel used for first creation.
                (self.on_window_ready)(window.clone());
            }
            let _ = self.events_tx.try_send(HostMsg::SurfaceReady {
                window: window.clone(),
            });
            let PhysicalSize { width, height } = window.inner_size();
            let _ = self.events_tx.try_send(HostMsg::Resized { width, height });
            let _ = self
                .events_tx
                .try_send(HostMsg::Event(GameEvent::WindowFocusChanged(true)));
            #[cfg(target_os = "android")]
            {
                self.resize_refresh_frames = 30;
            }
            set_game_control_flow(event_loop);
            return;
        }
        let attrs = winit::window::Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(PhysicalSize::new(self.width, self.height))
            .with_visible(self.visible)
            .with_resizable(true);

        // On wasm we need to attach the canvas to the document.  On
        // native, the OS window is created directly.
        #[cfg(target_arch = "wasm32")]
        let attrs = {
            use wasm_bindgen::JsCast;
            use winit::platform::web::WindowAttributesExtWebSys;
            let document = web_sys::window()
                .and_then(|w| w.document())
                .expect("no document");
            let canvas = document
                .get_element_by_id("canvas")
                .expect("no #canvas element");
            let canvas: web_sys::HtmlCanvasElement =
                canvas.dyn_into().expect("#canvas is not a <canvas>");
            attrs.with_canvas(Some(canvas))
        };

        let window = match event_loop.create_window(attrs) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("Window create: {e}");
                event_loop.exit();
                return;
            }
        };
        #[cfg(target_os = "android")]
        window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
        window.set_cursor_visible(false);
        let window = Arc::new(window);
        self.touch.set_scale_factor(window.scale_factor());
        self.window = Some(window.clone());
        set_game_window(window.clone());

        // Hand the bare window to the game future.  All wgpu init
        // (`request_adapter`, `request_device`) happens *async* on the
        // game side: on wasm those futures genuinely yield to the JS
        // event loop, and `pollster::block_on` would deadlock on the
        // condvar wait.  Native runs the same async init on its
        // dedicated game thread — `pollster::block_on` driving the
        // future is fine because the thread can sleep.
        (self.on_window_ready)(window);

        set_game_control_flow(event_loop);
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        let output = self.touch.cancel_all();
        self.emit_touch_outputs(output);
        self.send_pause_request();
        let _ = self.events_tx.try_send(HostMsg::LifecycleAutosave);
        let _ = self
            .events_tx
            .try_send(HostMsg::Event(GameEvent::WindowFocusChanged(false)));
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        #[cfg(target_os = "android")]
        let _ = event_loop;
        match event {
            WindowEvent::CloseRequested => {
                #[cfg(target_os = "android")]
                {
                    self.send_menu_toggle_request();
                }
                #[cfg(not(target_os = "android"))]
                {
                    let _ = self.events_tx.try_send(HostMsg::Event(GameEvent::Quit));
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(PhysicalSize { width, height }) => {
                let _ = self.events_tx.try_send(HostMsg::Resized { width, height });
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.touch.set_scale_factor(scale_factor);
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key,
                        logical_key,
                        state,
                        repeat,
                        text,
                        ..
                    },
                ..
            } => {
                let (keycode, physical_key) = if is_android_back_key(&logical_key, physical_key) {
                    (Keycode::Escape, Some(KeyCode::Escape))
                } else {
                    (
                        physical_key_to_keycode(physical_key),
                        physical_key_to_key_code(physical_key),
                    )
                };
                match state {
                    ElementState::Pressed => {
                        if !repeat {
                            let _ = self.events_tx.try_send(HostMsg::Event(GameEvent::KeyDown {
                                keycode,
                                physical_key,
                            }));
                        }
                        if let Some(text) = text
                            && !text.chars().any(|c| c.is_control())
                        {
                            let _ = self
                                .events_tx
                                .try_send(HostMsg::Event(GameEvent::TextInput {
                                    text: text.to_string(),
                                }));
                        }
                    }
                    ElementState::Released => {
                        let _ = self.events_tx.try_send(HostMsg::Event(GameEvent::KeyUp {
                            keycode,
                            physical_key,
                        }));
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let x = position.x as i32;
                let y = position.y as i32;
                tracing::trace!("winit CursorMoved: ({x}, {y})");
                self.last_cursor = (x, y);
                let _ = self
                    .events_tx
                    .try_send(HostMsg::Event(GameEvent::MouseMove {
                        x,
                        y,
                        xrel: 0,
                        yrel: 0,
                    }));
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let (x, y) = self.last_cursor;
                let btn = match button {
                    MouseButton::Left => 1,
                    MouseButton::Middle => 2,
                    MouseButton::Right => 3,
                    MouseButton::Back => 4,
                    MouseButton::Forward => 5,
                    MouseButton::Other(n) => n as u8,
                };
                let event = match state {
                    ElementState::Pressed => {
                        let now = web_time::Instant::now();
                        let clicks = match self.last_press {
                            Some((prev_btn, prev_t))
                                if prev_btn == btn
                                    && now.duration_since(prev_t).as_millis()
                                        <= DOUBLE_CLICK_INTERVAL_MS =>
                            {
                                self.last_press = None;
                                2
                            }
                            _ => {
                                self.last_press = Some((btn, now));
                                1
                            }
                        };
                        GameEvent::MouseDown(x, y, btn, clicks)
                    }
                    ElementState::Released => GameEvent::MouseUp(x, y, btn),
                };
                let _ = self.events_tx.try_send(HostMsg::Event(event));
            }
            WindowEvent::Touch(touch) => {
                let x = touch.location.x;
                let y = touch.location.y;
                let now_ms = process_uptime_ms();
                let output = match touch.phase {
                    TouchPhase::Started => self.touch.started(touch.id, x, y, now_ms),
                    TouchPhase::Moved => self.touch.moved(touch.id, x, y, now_ms),
                    TouchPhase::Ended => self.touch.ended(touch.id, x, y, now_ms, false),
                    TouchPhase::Cancelled => self.touch.ended(touch.id, x, y, now_ms, true),
                };
                self.emit_touch_outputs(output);
            }
            WindowEvent::Focused(focused) => {
                if !focused {
                    let _ = self.events_tx.try_send(HostMsg::LifecycleAutosave);
                    let output = self.touch.cancel_all();
                    self.emit_touch_outputs(output);
                }
                let _ = self
                    .events_tx
                    .try_send(HostMsg::Event(GameEvent::WindowFocusChanged(focused)));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as i32,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 32.0) as i32,
                };
                if y != 0 {
                    let _ = self
                        .events_tx
                        .try_send(HostMsg::Event(GameEvent::MouseWheel(y)));
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.process_cmds();
        if self.events_tx.is_closed() {
            event_loop.exit();
        }
        #[cfg(target_os = "android")]
        if self.resize_refresh_frames > 0 {
            self.resize_refresh_frames -= 1;
            if let Some(window) = &self.window {
                let PhysicalSize { width, height } = window.inner_size();
                if width > 0 && height > 0 {
                    let _ = self.events_tx.try_send(HostMsg::Resized { width, height });
                    window.request_redraw();
                }
            }
        }
    }
}

fn set_game_control_flow(event_loop: &impl EventLoopControlFlow) {
    event_loop.set_control_flow(ControlFlow::Wait);
}

trait EventLoopControlFlow {
    fn set_control_flow(&self, control_flow: ControlFlow);
}

impl EventLoopControlFlow for ActiveEventLoop {
    fn set_control_flow(&self, control_flow: ControlFlow) {
        ActiveEventLoop::set_control_flow(self, control_flow);
    }
}

impl<T> EventLoopControlFlow for EventLoop<T> {
    fn set_control_flow(&self, control_flow: ControlFlow) {
        EventLoop::set_control_flow(self, control_flow);
    }
}

// ---------------------------------------------------------------------
// Public entry: run a game future under a winit ApplicationHandler.
// ---------------------------------------------------------------------

/// Start the EventLoop and the game future together.
///
/// Native: spawns the game on a dedicated `std::thread` driven by
/// `pollster::block_on`, then runs winit on the calling thread.
/// Wasm: spawns the game via `wasm_bindgen_futures::spawn_local` and
/// hands control to winit's web backend (`spawn_app`), which never
/// returns.
///
/// `game_main` receives the constructed [`GameWindow`] once the OS
/// window + wgpu context are up.  Its return value (game exit code)
/// is ignored on wasm (where the page just stops rendering); on
/// native it's returned via the outer `Result`.
pub fn run_with_game<F, Fut>(
    title: &str,
    width: u32,
    height: u32,
    game_main: F,
) -> Result<i32, String>
where
    F: FnOnce(GameWindow) -> Fut + Send + 'static,
    Fut: Future<Output = i32> + 'static,
{
    run_with_game_visibility(title, width, height, true, game_main)
}

/// Run with a visible or hidden GPU-backed native window. Hidden mode still
/// creates a render surface, unlike the engine's no-render `--headless` mode.
pub fn run_with_game_visibility<F, Fut>(
    title: &str,
    width: u32,
    height: u32,
    visible: bool,
    game_main: F,
) -> Result<i32, String>
where
    F: FnOnce(GameWindow) -> Fut + Send + 'static,
    Fut: Future<Output = i32> + 'static,
{
    run_with_game_impl(
        title,
        width,
        height,
        visible,
        game_main,
        #[cfg(target_os = "android")]
        None,
    )
}

#[cfg(target_os = "android")]
pub fn run_with_android_game<F, Fut>(
    app: winit::platform::android::activity::AndroidApp,
    title: &str,
    width: u32,
    height: u32,
    game_main: F,
) -> Result<i32, String>
where
    F: FnOnce(GameWindow) -> Fut + Send + 'static,
    Fut: Future<Output = i32> + 'static,
{
    run_with_game_impl(title, width, height, true, game_main, Some(app))
}

fn run_with_game_impl<F, Fut>(
    title: &str,
    width: u32,
    height: u32,
    visible: bool,
    game_main: F,
    #[cfg(target_os = "android")] android_app: Option<
        winit::platform::android::activity::AndroidApp,
    >,
) -> Result<i32, String>
where
    F: FnOnce(GameWindow) -> Fut + Send + 'static,
    Fut: Future<Output = i32> + 'static,
{
    let event_loop = make_event_loop(
        #[cfg(target_os = "android")]
        android_app,
    )?;
    set_game_control_flow(&event_loop);

    let (events_tx, events_rx) = async_channel::unbounded::<HostMsg>();
    let (cmd_tx, cmd_rx) = async_channel::unbounded::<HostCmd>();
    let lifecycle_autosave_requested = Arc::new(AtomicBool::new(false));
    #[cfg(target_arch = "wasm32")]
    install_browser_lifecycle_autosave(lifecycle_autosave_requested.clone())?;
    #[cfg(target_os = "android")]
    {
        *android_back_tx().lock().expect("android back tx poisoned") = Some(events_tx.clone());
    }

    // The game future receives the bare winit window through this
    // oneshot-style channel.  All wgpu init (instance / surface /
    // adapter / device) happens *async* on the game side so the
    // wasm executor can yield while `request_adapter` etc. resolve.
    let (window_tx, window_rx) = async_channel::unbounded::<Arc<Window>>();
    let event_loop_proxy = event_loop.create_proxy();

    let on_ready: WindowReadyFn = Box::new(move |w: Arc<Window>| {
        let _ = window_tx.try_send(w);
    });

    let logical_w = width;
    let logical_h = height;
    let events_rx_for_game = events_rx.clone();
    let cmd_tx_for_game = cmd_tx.clone();
    let cmd_tx_for_exit = cmd_tx.clone();
    let (exit_code_tx, _exit_code_rx) = std::sync::mpsc::channel::<i32>();
    let lifecycle_for_game = lifecycle_autosave_requested.clone();

    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut handler = AppHandler {
        title: title.to_string(),
        width,
        height,
        visible,
        events_tx,
        cmd_rx,
        on_window_ready: on_ready,
        window: None,
        last_cursor: (0, 0),
        touch: TouchClassifier::default(),
        #[cfg(target_os = "android")]
        resize_refresh_frames: 0,
        last_press: None,
    };

    // Spawn the game.
    //
    // The closure constructs the future on the destination thread/task,
    // which means the future itself does NOT need to be `Send`: the
    // engine's `Rc`/`Cell`/`RefCell` state held across `.await` points
    // never crosses a thread boundary.  Only the `game_main` closure
    // (capturing plain data like `Campaign`, `Args`) needs `Send`.
    spawn_game_runtime(move || async move {
        #[cfg(target_os = "android")]
        let game_window = loop {
            let window = match window_rx.recv().await {
                Ok(w) => w,
                Err(_) => {
                    tracing::error!("event loop exited before window was ready");
                    let _ = exit_code_tx.send(1);
                    return;
                }
            };
            match build_game_window_async(
                window,
                logical_w,
                logical_h,
                events_rx_for_game.clone(),
                cmd_tx_for_game.clone(),
                lifecycle_for_game.clone(),
            )
            .await
            {
                Ok(gw) => break gw,
                Err(e) if e.contains("underlying handle is not available") => {
                    tracing::warn!(
                        "Android native window vanished during wgpu init; waiting for resume"
                    );
                }
                Err(e) => {
                    tracing::error!("wgpu init failed: {e}");
                    let _ = exit_code_tx.send(1);
                    let _ = cmd_tx_for_exit.try_send(HostCmd::Exit);
                    let _ = event_loop_proxy.send_event(());
                    return;
                }
            }
        };
        #[cfg(not(target_os = "android"))]
        let game_window = {
            // Wait for `resumed()` to ship us the bare winit window.  On
            // native this blocks the dedicated thread; on wasm this
            // `.await`s on the channel, yielding back to the JS event loop
            // until winit fires resumed().
            let window = match window_rx.recv().await {
                Ok(w) => w,
                Err(_) => {
                    tracing::error!("event loop exited before window was ready");
                    let _ = exit_code_tx.send(1);
                    return;
                }
            };
            match build_game_window_async(
                window,
                logical_w,
                logical_h,
                events_rx_for_game,
                cmd_tx_for_game,
                lifecycle_for_game,
            )
            .await
            {
                Ok(gw) => gw,
                Err(e) => {
                    tracing::error!("wgpu init failed: {e}");
                    let _ = exit_code_tx.send(1);
                    let _ = cmd_tx_for_exit.try_send(HostCmd::Exit);
                    let _ = event_loop_proxy.send_event(());
                    return;
                }
            }
        };
        let exit_code = game_main(game_window).await;
        tracing::info!("game future returned, exit_code={exit_code}");
        let _ = exit_code_tx.send(exit_code);
        let _ = cmd_tx_for_exit.try_send(HostCmd::Exit);
        let _ = event_loop_proxy.send_event(());
    });

    // Run winit on the calling thread.
    #[cfg(not(target_arch = "wasm32"))]
    {
        event_loop
            .run_app(&mut handler)
            .map_err(|e| format!("EventLoop::run_app: {e}"))?;
        receive_game_exit_code(&_exit_code_rx)
    }
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        // spawn_app takes ownership and never returns on web.
        event_loop.spawn_app(handler);
        Ok(0)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn receive_game_exit_code(receiver: &std::sync::mpsc::Receiver<i32>) -> Result<i32, String> {
    // Parity TODO: `original-code/launcher.cpp:679` runs the game and event
    // loop in one `main` thread, so the Rust native split runtime has no
    // Original thread-result analogue. Treat absence as a runtime failure,
    // never as the successful process exit code zero.
    receiver.try_recv().map_err(|err| match err {
        std::sync::mpsc::TryRecvError::Empty => {
            "game event loop exited before the game thread published its exit code".to_owned()
        }
        std::sync::mpsc::TryRecvError::Disconnected => {
            "game thread terminated without publishing an exit code".to_owned()
        }
    })
}

fn make_event_loop(
    #[cfg(target_os = "android")] android_app: Option<
        winit::platform::android::activity::AndroidApp,
    >,
) -> Result<EventLoop<()>, String> {
    let mut builder = EventLoop::builder();
    #[cfg(target_os = "android")]
    {
        use winit::platform::android::EventLoopBuilderExtAndroid;
        let app = android_app.ok_or("Android EventLoop requires AndroidApp from android_main")?;
        builder.with_android_app(app);
    }
    builder
        .build()
        .map_err(|e| format!("EventLoop::build: {e}"))
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_game_runtime<F, Fut>(make_fut: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()>,
{
    std::thread::Builder::new()
        .name("robin-game".into())
        .stack_size(GAME_THREAD_STACK_SIZE)
        .spawn(move || pollster::block_on(make_fut()))
        .expect("spawn game thread");
}

#[cfg(target_arch = "wasm32")]
fn spawn_game_runtime<F, Fut>(make_fut: F)
where
    F: FnOnce() -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    wasm_bindgen_futures::spawn_local(make_fut());
}

// ---------------------------------------------------------------------
// Text-input toggles (no-ops under winit).
// ---------------------------------------------------------------------

/// IME helpers. Winit delivers `GameEvent::TextInput` events
/// whether or not we've explicitly "started" it, so the start/stop
/// pair is mostly bookkeeping.  `start_text_input` additionally clears
/// any pending dead-key composition: when the player opens a text
/// surface (e.g. the dev console) using a key bound to a dead key
/// (`^`, `~`, etc.), the OS would otherwise compose that mark into
/// the next typed character.  Called directly on the [`Window`]
/// (rather than via the [`HostCmd`] queue) so the reset lands before
/// the next [`WindowEvent::KeyboardInput`] is processed on the main
/// thread.  On Linux winit's implementation is just an atomic-flag
/// store, safe from any thread.
pub fn start_text_input() {
    with_game_window(|w| w.reset_dead_keys());
}
pub fn stop_text_input() {}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{receive_game_exit_code, touch_output_needs_deferred_up};
    use crate::touch_input::TouchOutput;

    #[test]
    fn game_exit_code_is_forwarded() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(17).unwrap();

        assert_eq!(receive_game_exit_code(&rx), Ok(17));
    }

    #[test]
    fn missing_game_exit_code_is_an_error() {
        let (_tx, rx) = std::sync::mpsc::channel();

        assert_eq!(
            receive_game_exit_code(&rx),
            Err("game event loop exited before the game thread published its exit code".to_owned())
        );
    }

    #[test]
    fn disconnected_game_thread_is_an_error() {
        let (tx, rx) = std::sync::mpsc::channel::<i32>();
        drop(tx);

        assert_eq!(
            receive_game_exit_code(&rx),
            Err("game thread terminated without publishing an exit code".to_owned())
        );
    }

    #[test]
    fn only_release_classified_pointer_sequences_defer_their_up() {
        assert!(touch_output_needs_deferred_up(&[
            TouchOutput::PointerDown {
                x: 10.0,
                y: 20.0,
                clicks: 1,
            },
            TouchOutput::PointerUp { x: 10.0, y: 20.0 },
        ]));
        assert!(!touch_output_needs_deferred_up(&[
            TouchOutput::PointerMove { x: 30.0, y: 40.0 },
            TouchOutput::PointerUp { x: 30.0, y: 40.0 },
        ]));
    }
}

// ---------------------------------------------------------------------
// Key mapping (unchanged from the pump-events implementation).
// ---------------------------------------------------------------------

fn physical_key_to_key_code(key: PhysicalKey) -> Option<KeyCode> {
    match key {
        PhysicalKey::Code(c) => Some(c),
        PhysicalKey::Unidentified(_) => None,
    }
}

fn is_android_back_key(logical_key: &Key, physical_key: PhysicalKey) -> bool {
    matches!(
        logical_key,
        Key::Named(NamedKey::BrowserBack | NamedKey::GoBack)
    ) || matches!(physical_key, PhysicalKey::Code(KeyCode::BrowserBack))
}

fn physical_key_to_keycode(key: PhysicalKey) -> Keycode {
    use Keycode as K;
    let code = match key {
        PhysicalKey::Code(c) => c,
        PhysicalKey::Unidentified(_) => return K::Unknown,
    };
    match code {
        KeyCode::Escape => K::Escape,
        KeyCode::Enter => K::Return,
        KeyCode::NumpadEnter => K::KpEnter,
        KeyCode::Tab => K::Tab,
        KeyCode::Space => K::Space,
        KeyCode::Backspace => K::Backspace,
        KeyCode::Delete => K::Delete,
        KeyCode::Insert => K::Insert,
        KeyCode::ArrowUp => K::Up,
        KeyCode::ArrowDown => K::Down,
        KeyCode::ArrowLeft => K::Left,
        KeyCode::ArrowRight => K::Right,
        KeyCode::Home => K::Home,
        KeyCode::End => K::End,
        KeyCode::PageUp => K::PageUp,
        KeyCode::PageDown => K::PageDown,
        KeyCode::F1 => K::F1,
        KeyCode::F2 => K::F2,
        KeyCode::F3 => K::F3,
        KeyCode::F4 => K::F4,
        KeyCode::F5 => K::F5,
        KeyCode::F6 => K::F6,
        KeyCode::F7 => K::F7,
        KeyCode::F8 => K::F8,
        KeyCode::F9 => K::F9,
        KeyCode::F10 => K::F10,
        KeyCode::F11 => K::F11,
        KeyCode::F12 => K::F12,
        KeyCode::ShiftLeft => K::LShift,
        KeyCode::ShiftRight => K::RShift,
        KeyCode::ControlLeft => K::LCtrl,
        KeyCode::ControlRight => K::RCtrl,
        KeyCode::AltLeft => K::LAlt,
        KeyCode::AltRight => K::RAlt,
        KeyCode::KeyA => K::Char(b'a'),
        KeyCode::KeyB => K::Char(b'b'),
        KeyCode::KeyC => K::Char(b'c'),
        KeyCode::KeyD => K::Char(b'd'),
        KeyCode::KeyE => K::Char(b'e'),
        KeyCode::KeyF => K::Char(b'f'),
        KeyCode::KeyG => K::Char(b'g'),
        KeyCode::KeyH => K::Char(b'h'),
        KeyCode::KeyI => K::Char(b'i'),
        KeyCode::KeyJ => K::Char(b'j'),
        KeyCode::KeyK => K::Char(b'k'),
        KeyCode::KeyL => K::Char(b'l'),
        KeyCode::KeyM => K::Char(b'm'),
        KeyCode::KeyN => K::Char(b'n'),
        KeyCode::KeyO => K::Char(b'o'),
        KeyCode::KeyP => K::Char(b'p'),
        KeyCode::KeyQ => K::Char(b'q'),
        KeyCode::KeyR => K::Char(b'r'),
        KeyCode::KeyS => K::Char(b's'),
        KeyCode::KeyT => K::Char(b't'),
        KeyCode::KeyU => K::Char(b'u'),
        KeyCode::KeyV => K::Char(b'v'),
        KeyCode::KeyW => K::Char(b'w'),
        KeyCode::KeyX => K::Char(b'x'),
        KeyCode::KeyY => K::Char(b'y'),
        KeyCode::KeyZ => K::Char(b'z'),
        KeyCode::Digit0 => K::Char(b'0'),
        KeyCode::Digit1 => K::Char(b'1'),
        KeyCode::Digit2 => K::Char(b'2'),
        KeyCode::Digit3 => K::Char(b'3'),
        KeyCode::Digit4 => K::Char(b'4'),
        KeyCode::Digit5 => K::Char(b'5'),
        KeyCode::Digit6 => K::Char(b'6'),
        KeyCode::Digit7 => K::Char(b'7'),
        KeyCode::Digit8 => K::Char(b'8'),
        KeyCode::Digit9 => K::Char(b'9'),
        _ => K::Unknown,
    }
}

#[cfg(feature = "gamepad")]
fn gilrs_button_to_index(b: gilrs::Button) -> Option<u8> {
    use gilrs::Button as B;
    Some(match b {
        B::South => 0,
        B::East => 1,
        B::West => 2,
        B::North => 3,
        B::Select => 4,
        B::Mode => 5,
        B::Start => 6,
        B::LeftThumb => 7,
        B::RightThumb => 8,
        B::LeftTrigger => 9,
        B::RightTrigger => 10,
        B::DPadUp => 11,
        B::DPadDown => 12,
        B::DPadLeft => 13,
        B::DPadRight => 14,
        _ => return None,
    })
}

#[cfg(feature = "gamepad")]
fn gilrs_axis_to_index(a: gilrs::Axis) -> Option<u8> {
    use gilrs::Axis as A;
    Some(match a {
        A::LeftStickX => 0,
        A::LeftStickY => 1,
        A::RightStickX => 2,
        A::RightStickY => 3,
        A::LeftZ => 4,
        A::RightZ => 5,
        _ => return None,
    })
}
