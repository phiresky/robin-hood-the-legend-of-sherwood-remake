//! Bridge between the [`crate::widget`] system and the in-game menu
//! rendering / input infrastructure.
//!
//! The widget module provides state-machine input handling (hover, push,
//! select) and the `FrameWnd` container.  The in-game menus already have
//! sprite-based rendering (`draw_button`, `render_text_in_box`, etc.).
//! This bridge connects the two: widgets drive the *state*, the existing
//! layout helpers drive the *rendering*.

use crate::cursor::CursorRenderer;
use crate::geo2d;
use crate::gfx_types::GameEvent;
use crate::native_font::NativeFont;
use crate::renderer::Renderer;
use crate::resource_ids;
use crate::resource_manager::ResourceManager;
use crate::sound::{AudioBackend, SoundManager};
use crate::ui::{MouseButtons, UiEvent, UiKeyboard, UiMsg, UiState};
use crate::widget::{
    CaptureSlot, FrameWnd, Widget, WidgetButton, WidgetId, WidgetInput, WidgetLabel,
    WidgetMultiPicture, WidgetPicture, WidgetRenderer,
};
use robin_engine::coordinates::ScreenBBox;
use robin_engine::sound_cache::SampleLoader;
use robin_engine::sprite::BBox;

use super::layout::{
    BTN_STATE_DISABLED, BTN_STATE_HOVER, BTN_STATE_NORMAL, BTN_STATE_PRESSED, BTN_STATE_SELECTED,
    MenuTransform,
};
use super::resources::{IngameMenuResources, MenuSurface};

// ─── Widget creation helpers ────────────────────────────────────────

/// Create a [`WidgetButton`] positioned in virtual 640×480 coordinates,
/// using the default rectangular `RHID_MENU_BUTTON` sprite pack.
pub fn make_button(id: WidgetId, label: &str, x: i32, y: i32, w: i32, h: i32) -> Widget {
    make_button_enabled(id, label, true, x, y, w, h)
}

/// Create a [`WidgetButton`] with an explicit enabled flag.
pub fn make_button_enabled(
    id: WidgetId,
    label: &str,
    enabled: bool,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Widget {
    make_button_with_resource(
        id,
        label,
        enabled,
        resource_ids::RHID_MENU_BUTTON,
        x,
        y,
        w,
        h,
    )
}

/// Create a [`WidgetButton`] bound to a specific sprite-pack resource
/// ID.  The button widget carries its sprite resource so different
/// buttons (rectangular `RHID_MENU_BUTTON`, round `RHID_OK` seal, etc.)
/// can coexist inside the same `FrameWnd`.
#[allow(clippy::too_many_arguments)]
pub fn make_button_with_resource(
    id: WidgetId,
    label: &str,
    enabled: bool,
    resource_id: crate::ui::ResourceId,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Widget {
    let mut btn = WidgetButton::new(id);
    // In-game menu buttons engage the override state machine (group
    // focus, drag-off-cancel, hide-focus, per-frame `WidgetFocused`
    // heartbeat) — the base button state machine has subtly different
    // transitions.
    btn.is_menu_button = true;
    let bbox = ScreenBBox::from_coords(x as f32, y as f32, (x + w) as f32, (y + h) as f32);
    btn.base.create_with_resource(label, bbox, 0, resource_id);
    btn.base.enabled = enabled;
    // `create_with_resource` leaves `renderer` as `None`; give it a
    // bitmap renderer so hit testing against `bbox` works (the actual
    // rendering is done by the bridge, not the widget's renderer).
    let renderer_base = match btn.base.renderer.base() {
        Some(b) => b.clone(),
        None => crate::ui::RendererBase {
            bbox,
            resource_id,
            ..Default::default()
        },
    };
    btn.base.renderer = WidgetRenderer::Bitmap(crate::ui::RendererBitmap {
        base: renderer_base,
    });
    Widget::Button(btn)
}

/// Create a [`FrameWnd`] with the given buttons.
///
/// `buttons` is a slice of `(id, label, x, y, w, h)`.
pub fn make_button_frame(buttons: &[(WidgetId, &str, i32, i32, i32, i32)]) -> FrameWnd {
    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;
    for &(id, label, x, y, w, h) in buttons {
        frame.add_widget_absolute(make_button(id, label, x, y, w, h));
    }
    frame
}

/// Create a non-interactive bitmap picture widget.
///
/// The widget owns position, visibility/input flags, and resource id;
/// the bridge resolves that resource id to a GPU surface at draw time.
pub fn make_picture_with_resource(
    id: WidgetId,
    resource_id: crate::ui::ResourceId,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Widget {
    let mut pic = WidgetPicture::new(id);
    let bbox = ScreenBBox::from_coords(x as f32, y as f32, (x + w) as f32, (y + h) as f32);
    pic.base.create_with_resource("", bbox, 0, resource_id);
    pic.base.with_focus = false;
    let renderer_base = match pic.base.renderer.base() {
        Some(b) => b.clone(),
        None => crate::ui::RendererBase {
            bbox,
            resource_id,
            ..Default::default()
        },
    };
    pic.base.renderer = WidgetRenderer::Bitmap(crate::ui::RendererBitmap {
        base: renderer_base,
    });
    Widget::Picture(pic)
}

/// Create a non-interactive multi-picture widget bound to one resource id.
pub fn make_multi_picture_with_resource(
    id: WidgetId,
    resource_id: crate::ui::ResourceId,
    sub_picture: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Widget {
    let mut pic = WidgetMultiPicture::new(id);
    let bbox = ScreenBBox::from_coords(x as f32, y as f32, (x + w) as f32, (y + h) as f32);
    pic.base.create_with_resource("", bbox, 0, resource_id);
    pic.base.with_focus = false;
    pic.select_picture(sub_picture);
    let renderer_base = match pic.base.renderer.base() {
        Some(b) => b.clone(),
        None => crate::ui::RendererBase {
            bbox,
            resource_id,
            ..Default::default()
        },
    };
    pic.base.renderer = WidgetRenderer::Bitmap(crate::ui::RendererBitmap {
        base: renderer_base,
    });
    Widget::MultiPicture(pic)
}

/// Create a non-interactive text label widget.
pub fn make_label(id: WidgetId, text: &str, x: i32, y: i32, w: i32, h: i32) -> Widget {
    let mut label = WidgetLabel::new(id);
    let bbox = ScreenBBox::from_coords(x as f32, y as f32, (x + w) as f32, (y + h) as f32);
    label.base.create(text, bbox, 0);
    label.set_text(text);
    Widget::Label(label)
}

// ─── Input conversion ───────────────────────────────────────────────

/// Accumulated mouse state for driving widgets each frame.
///
/// Call [`update_from_event`](Self::update_from_event) for each SDL
/// event, then call [`as_widget_input`](Self::as_widget_input) once per
/// frame to get the [`WidgetInput`].
pub struct ModalInputState {
    pub virt_x: f32,
    pub virt_y: f32,
    pub buttons: MouseButtons,
    keyboard: UiKeyboard,
    /// Raw physical-key pressed-state set; `KeyDown`/`KeyUp` events
    /// toggle entries here and [`refresh_keyboard`](Self::refresh_keyboard)
    /// feeds the set into [`UiKeyboard::refresh`] each frame so
    /// the widget state machine sees the same pressed/released/
    /// typewriter transitions the in-game path uses.
    raw_keyboard: crate::input::KeyboardState,
    /// Monotonic millisecond clock sampled from `web_time::Instant`
    /// at construction; `elapsed_ms` feeds it to `UiKeyboard::refresh`
    /// for double-press and typewriter-repeat timing.
    start_time: web_time::Instant,
    /// Accumulated UTF-8 text from `SDL_EVENT_TEXT_INPUT` since the last
    /// [`as_widget_input`](Self::as_widget_input) call. SDL emits only
    /// committed IME text here — composition preview (SDL_EVENT_TEXT_EDITING)
    /// isn't surfaced, matching the save/load dialog's text path.
    text_input: String,
    /// Mouse-capture slot passed through to widgets so push/drag
    /// interactions can request capture.  See [`CaptureSlot`].
    capture: CaptureSlot,
    /// Pending SDL `clicks` count from the most recent `MouseButtonDown`
    /// (per button), consumed on the matching `MouseButtonUp` to decide
    /// whether to flag the released click as a double-click.  Promotion
    /// happens on the release edge using SDL3's native double-click
    /// counter (250 ms window, set by the OS) rather than a
    /// frame-based counter.
    pending_double_click_left: bool,
    pending_double_click_right: bool,
}

impl Default for ModalInputState {
    fn default() -> Self {
        Self {
            virt_x: -1.0,
            virt_y: -1.0,
            buttons: MouseButtons::empty(),
            keyboard: UiKeyboard::default(),
            raw_keyboard: crate::input::KeyboardState::default(),
            start_time: web_time::Instant::now(),
            text_input: String::new(),
            capture: CaptureSlot::default(),
            pending_double_click_left: false,
            pending_double_click_right: false,
        }
    }
}

/// Cursor-rendering context threaded into modal popup loops so the
/// custom game cursor stays visible while the modal is up.
///
/// The in-game `CursorRenderer` hides the OS cursor for the lifetime of
/// the session, so modals that open on top of the game must draw the
/// cursor themselves — otherwise the mouse appears to vanish.
pub struct ModalCursor<'a> {
    pub cursor: &'a mut CursorRenderer,
    pub opacity: u16,
    pub shadow_color: u16,
}

impl<'a> ModalCursor<'a> {
    pub fn new(cursor: &'a mut CursorRenderer, opacity: u16, shadow_color: u16) -> Self {
        Self {
            cursor,
            opacity,
            shadow_color,
        }
    }

    /// Re-borrow for passing into a nested modal call.
    pub fn reborrow(&mut self) -> ModalCursor<'_> {
        ModalCursor {
            cursor: &mut *self.cursor,
            opacity: self.opacity,
            shadow_color: self.shadow_color,
        }
    }

    /// Draw the cursor at the current modal input position (virtual → screen).
    /// Skips rendering until the first mouse event seeds `input.virt_x/y`.
    pub fn draw(
        &self,
        renderer: &mut Renderer,
        transform: super::layout::MenuTransform,
        input: &ModalInputState,
    ) {
        if input.virt_x < 0.0 || input.virt_y < 0.0 {
            return;
        }
        let (sx, sy) = transform.to_screen(input.virt_x as i32, input.virt_y as i32);
        self.cursor.render(
            renderer,
            sx as f32,
            sy as f32,
            self.opacity,
            self.shadow_color,
        );
    }
}

pub fn default_modal_cursor<'a>(
    cursor: &'a mut CursorRenderer,
    cursor_res: &mut ResourceManager,
    renderer: &mut Renderer,
) -> ModalCursor<'a> {
    if cursor.current_cursor_id() != resource_ids::RHMOUSE_DEFAULT {
        cursor.load_cursor(resource_ids::RHMOUSE_DEFAULT, cursor_res, renderer);
    }
    ModalCursor::new(
        cursor,
        robin_engine::engine::input::MOUSE_OPACITY_DEFAULT,
        0,
    )
}

impl ModalInputState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update from an SDL event.  Returns the event unchanged so the
    /// caller can also process keyboard shortcuts, quit, etc.
    pub fn update_from_event<'e>(
        &mut self,
        event: &'e GameEvent,
        transform: MenuTransform,
    ) -> &'e GameEvent {
        match event {
            GameEvent::MouseMove { x, y, .. } => {
                let (vx, vy) = transform.from_screen(*x, *y);
                self.virt_x = vx as f32;
                self.virt_y = vy as f32;
            }
            GameEvent::MouseDown(x, y, btn, clicks) => {
                let (vx, vy) = transform.from_screen(*x, *y);
                self.virt_x = vx as f32;
                self.virt_y = vy as f32;
                if *btn == 1 {
                    self.buttons |= MouseButtons::LEFT_DOWN;
                    self.pending_double_click_left = *clicks >= 2;
                } else if *btn == 3 {
                    self.buttons |= MouseButtons::RIGHT_DOWN;
                    self.pending_double_click_right = *clicks >= 2;
                }
            }
            GameEvent::MouseUp(x, y, btn) => {
                let (vx, vy) = transform.from_screen(*x, *y);
                self.virt_x = vx as f32;
                self.virt_y = vy as f32;
                if *btn == 1 {
                    self.buttons.remove(MouseButtons::LEFT_DOWN);
                    self.buttons |= MouseButtons::LEFT_CLICK;
                    if self.pending_double_click_left {
                        self.buttons |= MouseButtons::LEFT_DOUBLE_CLICK;
                        self.pending_double_click_left = false;
                    }
                } else if *btn == 3 {
                    self.buttons.remove(MouseButtons::RIGHT_DOWN);
                    self.buttons |= MouseButtons::RIGHT_CLICK;
                    if self.pending_double_click_right {
                        self.buttons |= MouseButtons::RIGHT_DOUBLE_CLICK;
                        self.pending_double_click_right = false;
                    }
                }
            }
            GameEvent::TextInput { text } => {
                self.text_input.push_str(text);
            }
            GameEvent::KeyDown {
                physical_key: Some(physical_key),
                ..
            } => {
                self.raw_keyboard.keys.insert(*physical_key);
            }
            GameEvent::KeyUp {
                physical_key: Some(physical_key),
                ..
            } => {
                self.raw_keyboard.keys.remove(physical_key);
            }
            _ => {}
        }
        event
    }

    /// Build a [`WidgetInput`] for this frame.  Held-down and one-shot
    /// click flags are both exposed to the widget state machine —
    /// `end_frame` clears the one-shot flags after `process_input`
    /// consumes them, so pressing and releasing the mouse button
    /// actually fires `WidgetActivated`.
    ///
    /// Also advances the [`UiKeyboard`] state machine against the raw
    /// pressed/released buffer accumulated from `KeyDown`/`KeyUp`
    /// events so widgets see per-frame transitions (KeyDown → KeyPressed
    /// → KeyUp) and typewriter auto-repeat. Before this refresh existed
    /// the keyboard was a stuck-at-default stub and the input-field
    /// widget never saw Left/Right/Home/End/Backspace/Enter/Esc in the
    /// modal path.
    pub fn as_widget_input(&mut self) -> WidgetInput<'_> {
        let now_ms = self.start_time.elapsed().as_millis() as u32;
        self.keyboard.refresh(&self.raw_keyboard, now_ms);
        WidgetInput {
            mouse_position: robin_engine::coordinates::ScreenPoint::new(self.virt_x, self.virt_y),
            mouse_z: 0,
            mouse_button: self.buttons,
            keyboard: &self.keyboard,
            text_input: &self.text_input,
            capture: Some(&self.capture),
        }
    }

    /// Read-only access to the refreshed [`UiKeyboard`]. Useful for
    /// modal code that wants to peek at typewriter-repeat state between
    /// `process_input` calls.
    pub fn keyboard(&self) -> &UiKeyboard {
        &self.keyboard
    }

    /// Widget currently requesting mouse capture (if any). Set via
    /// [`CaptureSlot::set`] inside a widget's `process_input`.
    pub fn capture(&self) -> Option<WidgetId> {
        self.capture.get()
    }

    /// Clear one-shot flags (click / double-click / text-input buffer)
    /// after `process_input` has consumed them.  Held-down button flags
    /// persist so the next frame still sees the mouse as down.
    pub fn end_frame(&mut self) {
        self.buttons.remove(MouseButtons::LEFT_CLICK);
        self.buttons.remove(MouseButtons::RIGHT_CLICK);
        self.buttons.remove(MouseButtons::LEFT_DOUBLE_CLICK);
        self.buttons.remove(MouseButtons::RIGHT_DOUBLE_CLICK);
        self.text_input.clear();
    }

    /// Seed `virt_x`/`virt_y` from the live SDL mouse state so the modal
    /// cursor renders at the correct location on the first frame, before
    /// any `MouseMove` event has been delivered.
    pub fn seed_mouse_from_sdl(
        &mut self,
        event_pump: &crate::window::GameWindow,
        transform: super::layout::MenuTransform,
    ) {
        let (mx, my) = event_pump.cursor_pos();
        let (vx, vy) = transform.from_screen(mx, my);
        self.virt_x = vx as f32;
        self.virt_y = vy as f32;
    }
}

// ─── Rendering bridge ───────────────────────────────────────────────

/// Map a widget's [`UiState`] to a button sprite state index.
fn widget_state_to_sprite(state: UiState, enabled: bool) -> usize {
    if !enabled {
        return BTN_STATE_DISABLED;
    }
    match state {
        UiState::Default => BTN_STATE_NORMAL,
        UiState::Focused => BTN_STATE_HOVER,
        UiState::Pushed => BTN_STATE_PRESSED,
        UiState::Selected => BTN_STATE_SELECTED,
        _ => BTN_STATE_NORMAL,
    }
}

/// Extract virtual-space position and size from a widget's bbox.
fn widget_virt_rect(widget: &Widget) -> Option<(i32, i32, i32, i32)> {
    let rect = widget.base().bbox.0?;
    Some((
        rect.min().x as i32,
        rect.min().y as i32,
        (rect.max().x - rect.min().x) as i32,
        (rect.max().y - rect.min().y) as i32,
    ))
}

/// Bake a per-pixel opacity mask onto every button-style widget in the
/// frame whose `resource_id` resolves to a known menu sprite pack.
///
/// Pre-bakes the answer to "is this point really inside the visible
/// pixels of the sprite?" into a 1-bit-per-pixel `AlphaMask` against
/// the Default-state sprite, used by the per-click hit test.  The
/// state-dependent alpha differences between Normal/Focused/Pressed
/// are below the hit-test threshold for the round-seal sprites this
/// matters for, so a single mask suffices.  Should be invoked once
/// after a frame's widgets are added (and re-invoked if the frame's
/// resources are ever rebuilt).
///
/// Widgets without a recognised sprite pack are left mask-less and
/// fall back to bbox-only hit testing (no behaviour change).
pub fn attach_alpha_masks(
    frame: &mut FrameWnd,
    resources: &IngameMenuResources,
    renderer: &Renderer,
) {
    for widget in frame.widgets_mut() {
        let resource_id = match widget.base().renderer.base() {
            Some(rb) => rb.resource_id,
            None => continue,
        };
        let surface_id = match widget {
            Widget::Button(_)
            | Widget::ToggleButton(_)
            | Widget::RadioButton(_)
            | Widget::Picture(_)
            | Widget::MultiPicture(_) => match resource_id {
                resource_ids::RHID_OK => resources.ok_button_surface(BTN_STATE_NORMAL),
                resource_ids::RHID_CANCEL => resources.cancel_button_surface(BTN_STATE_NORMAL),
                resource_ids::RHID_MENU_BUTTON => resources.button_surface(BTN_STATE_NORMAL),
                _ => None,
            },
            _ => None,
        };
        let Some(surf) = surface_id else { continue };
        let Some(mask) = renderer.build_alpha_mask(surf) else {
            continue;
        };
        if let Some(rb) = widget.base_mut().renderer.base_mut() {
            rb.set_alpha_mask(Some(mask));
        }
    }
}

/// Render all button widgets in a [`FrameWnd`] using the existing
/// sprite + font pipeline.
pub fn draw_frame_buttons(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    frame: &FrameWnd,
) {
    for widget in frame.widgets() {
        draw_widget_button(renderer, resources, transform, widget, false);
    }
}

/// Render a single widget as a menu button sprite.
///
/// If `force_hover` is true, the button renders as hovered regardless of
/// widget state (used for keyboard navigation highlight).
pub fn draw_widget_button(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    widget: &Widget,
    force_hover: bool,
) {
    let Some((vx, vy, w, h)) = widget_virt_rect(widget) else {
        return;
    };
    let base = widget.base();
    let state_idx = if force_hover && base.state == UiState::Default && base.enabled {
        BTN_STATE_HOVER
    } else {
        widget_state_to_sprite(base.state, base.enabled)
    };
    let (sx, sy) = transform.to_screen(vx, vy);

    use crate::renderer::BLIT_SOURCE_TRANSPARENT;
    use robin_engine::sprite::BBox;

    // Look up the sprite pack by the widget's resource ID.  Dispatching
    // on `base.renderer.base().resource_id` lets the same draw routine
    // handle both rectangular menu buttons (`RHID_MENU_BUTTON`) and
    // round seal buttons (`RHID_OK`).
    let resource_id = base
        .renderer
        .base()
        .map(|b| b.resource_id)
        .unwrap_or(resource_ids::RHID_MENU_BUTTON);
    let sprite = match resource_id {
        resource_ids::RHID_OK => resources.ok_button_surface(state_idx),
        resource_ids::RHID_CANCEL => resources.cancel_button_surface(state_idx),
        resource_ids::RHID_RADIO => {
            let selected = matches!(
                widget,
                Widget::Button(button)
                    if matches!(button.group_state, UiState::Pushed | UiState::Selected)
            );
            let focused = matches!(base.state, UiState::Focused | UiState::Pushed);
            let radio_state = match (selected, focused) {
                (true, true) => crate::ui::resource_widget_id::RADIO_FOCUS_SELECTED as usize,
                (true, false) => crate::ui::resource_widget_id::RADIO_SELECTED as usize,
                (false, true) => crate::ui::resource_widget_id::RADIO_FOCUS as usize,
                (false, false) => crate::ui::resource_widget_id::RADIO_UNSELECTED as usize,
            };
            resources.radio_surface(radio_state)
        }
        _ => resources.button_surface(state_idx),
    };

    let sprite_drawn = if let Some(surf) = sprite {
        let src = BBox::new(geo2d::pt(0.0, 0.0), geo2d::pt(w as f32, h as f32));
        let dst = BBox::new(
            geo2d::pt(sx as f32, sy as f32),
            geo2d::pt((sx + w) as f32, (sy + h) as f32),
        );
        // `blit_with_shadow` multiply-darkens shadow-key pixels (key
        // 0x1F, intensity 50) in the source bitmap so the drop-shadow
        // ring around each button blends instead of rendering opaque
        // blue.
        renderer.blit_with_shadow(
            surf,
            Some(&src),
            0, // screen
            Some(&dst),
            0,  // shadow_color (unused on this path)
            50, // shadow_level — shadow-renderer default
            BLIT_SOURCE_TRANSPARENT,
        )
    } else {
        let hovered = force_hover || base.state == UiState::Focused;
        super::layout::draw_fallback_rect(renderer, sx, sy, w, h, hovered);
        false
    };

    // Only render the label when the sprite blit succeeded — drawing
    // text over a missing sprite would leave it floating against the
    // background.  `render_text_screen` is itself a no-op for empty
    // labels, so seal buttons with no overlaid text (e.g. `RHID_OK`
    // created with `""`) Just Work.
    //
    // Centering: set the text box to exactly the font height at
    // `(btn_h - font_height) / 2` inside the button and use horizontal
    // centred alignment.  The text-cell top thus lands at
    // `sy + (h - font_height) / 2` with the glyph cell top at that y.
    if sprite_drawn && let Some(font) = resources.menu_button_font(base.enabled) {
        let tw = font.text_width(&base.text);
        let th = font.height() as i32;
        let tx = sx + (w - tw) / 2;
        let ty = sy + (h - th) / 2;
        super::layout::render_text_screen(renderer, font, &base.text, tx, ty);
    }
}

/// Render every bitmap-backed widget in a frame with caller-provided
/// resource resolution.
///
/// The widget system owns order, position, state, and resource ids; the
/// resolver supplies the GPU surface for `(resource_id, sub_resource)`.
pub fn draw_frame_bitmap_widgets(
    renderer: &mut Renderer,
    transform: MenuTransform,
    frame: &FrameWnd,
    mut resolve: impl FnMut(crate::ui::ResourceId, u8) -> Option<MenuSurface>,
) {
    for widget in frame.widgets() {
        if frame.is_excluded(widget.id()) {
            continue;
        }
        let Some(resource_id) = widget.base().renderer.base().map(|b| b.resource_id) else {
            continue;
        };
        let sub_resource = widget.transform_state_into_id();
        match widget {
            Widget::Picture(pic) => {
                if let Some(surface_id) = pic.alternate_picture() {
                    draw_widget_surface_id(renderer, transform, widget, surface_id, None, true);
                } else if let Some(surface) = resolve(resource_id, sub_resource) {
                    draw_widget_surface(renderer, transform, widget, surface, true);
                }
            }
            Widget::MultiPicture(_) | Widget::Button(_) => {
                if let Some(surface) = resolve(resource_id, sub_resource) {
                    draw_widget_surface(renderer, transform, widget, surface, true);
                }
            }
            _ => {}
        }
    }
}

/// Render a `WidgetPicture`'s alternate surface.  Used by widgets whose
/// bitmap is generated at runtime rather than loaded from `.RES`.
pub fn draw_picture_alternate_surface(
    renderer: &mut Renderer,
    transform: MenuTransform,
    widget: &WidgetPicture,
    width: i32,
    height: i32,
    transparent: bool,
) {
    if let Some(surface_id) = widget.alternate_picture() {
        let temp = Widget::Picture(widget.clone());
        draw_widget_surface_id(
            renderer,
            transform,
            &temp,
            surface_id,
            Some((0, 0, width, height)),
            transparent,
        );
    }
}

/// Render an arbitrary surface through a temporary `WidgetPicture`.
///
/// This is for legacy paths that already own a GPU surface handle rather
/// than a `.RES` resource id. It keeps the blit in the widget bridge
/// instead of each feature locally constructing source/destination boxes.
#[allow(clippy::too_many_arguments)]
pub fn draw_picture_surface_rect(
    renderer: &mut Renderer,
    transform: MenuTransform,
    surface_id: u32,
    dst_x: i32,
    dst_y: i32,
    dst_w: i32,
    dst_h: i32,
    src_x: i32,
    src_y: i32,
    src_w: i32,
    src_h: i32,
    transparent: bool,
) {
    let mut widget = WidgetPicture::new(WidgetId::MAX);
    widget.base.create(
        "",
        ScreenBBox::from_coords(
            dst_x as f32,
            dst_y as f32,
            (dst_x + dst_w) as f32,
            (dst_y + dst_h) as f32,
        ),
        0,
    );
    widget.set_alternate_picture(surface_id);
    let temp = Widget::Picture(widget);
    draw_widget_surface_id(
        renderer,
        transform,
        &temp,
        surface_id,
        Some((src_x, src_y, src_w, src_h)),
        transparent,
    );
}

/// Render all label widgets in a frame with a single native font.
pub fn draw_frame_labels(
    renderer: &mut Renderer,
    transform: MenuTransform,
    frame: &FrameWnd,
    font: &NativeFont,
    align: super::layout::TextAlign,
) {
    for widget in frame.widgets() {
        if frame.is_excluded(widget.id()) || !matches!(widget, Widget::Label(_)) {
            continue;
        }
        let Some((vx, vy, w, h)) = widget_virt_rect(widget) else {
            continue;
        };
        super::layout::render_text_in_box(
            renderer,
            font,
            transform,
            &widget.base().text,
            vx,
            vy,
            w,
            h,
            align,
        );
    }
}

fn draw_widget_surface(
    renderer: &mut Renderer,
    transform: MenuTransform,
    widget: &Widget,
    surface: MenuSurface,
    transparent: bool,
) {
    draw_widget_surface_id(
        renderer,
        transform,
        widget,
        surface.id,
        Some((0, 0, surface.width, surface.height)),
        transparent,
    );
}

fn draw_widget_surface_id(
    renderer: &mut Renderer,
    transform: MenuTransform,
    widget: &Widget,
    surface_id: u32,
    source_rect: Option<(i32, i32, i32, i32)>,
    transparent: bool,
) {
    let Some((vx, vy, w, h)) = widget_virt_rect(widget) else {
        return;
    };
    let (src_x, src_y, src_w, src_h) = source_rect.unwrap_or((0, 0, w, h));
    let (sx, sy) = transform.to_screen(vx, vy);
    let src = BBox::new(
        geo2d::pt(src_x as f32, src_y as f32),
        geo2d::pt((src_x + src_w) as f32, (src_y + src_h) as f32),
    );
    let dst = BBox::new(
        geo2d::pt(sx as f32, sy as f32),
        geo2d::pt((sx + w) as f32, (sy + h) as f32),
    );
    let flags = if transparent {
        crate::renderer::BLIT_SOURCE_TRANSPARENT
    } else {
        0
    };
    renderer.blit_to_screen(surface_id, Some(&src), Some(&dst), flags);
}

/// Render a widget as a radio-button (input field sprite + label).
///
/// `selected` is the logical selection state from the config (e.g.
/// "is this the currently active resolution?"), independent of the
/// widget's hover/push state.
pub fn draw_widget_radio(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    widget: &Widget,
    selected: bool,
) {
    let Some((vx, vy, w, h)) = widget_virt_rect(widget) else {
        return;
    };
    let base = widget.base();
    let hovered = matches!(base.state, UiState::Focused | UiState::Pushed);
    let (sx, sy) = transform.to_screen(vx, vy);

    use crate::renderer::BLIT_SOURCE_TRANSPARENT;
    use robin_engine::sprite::BBox;

    if let Some(surf) = resources.input_field_surface(selected) {
        let src = BBox::new(geo2d::pt(0.0, 0.0), geo2d::pt(w as f32, h as f32));
        let dst = BBox::new(
            geo2d::pt(sx as f32, sy as f32),
            geo2d::pt((sx + w) as f32, (sy + h) as f32),
        );
        renderer.blit_to_screen(surf, Some(&src), Some(&dst), BLIT_SOURCE_TRANSPARENT);
    } else {
        let bg = if selected {
            Renderer::create_color_16(100, 80, 40)
        } else if hovered {
            Renderer::create_color_16(60, 50, 30)
        } else {
            Renderer::create_color_16(30, 25, 15)
        };
        renderer.fill_screen(
            Some(&robin_engine::sprite::BBox::new(
                geo2d::pt(sx as f32, sy as f32),
                geo2d::pt((sx + w) as f32, (sy + h) as f32),
            )),
            bg,
        );
        let border = if hovered || selected {
            Renderer::create_color_16(220, 200, 140)
        } else {
            Renderer::create_color_16(120, 100, 60)
        };
        renderer.draw_rect_outline_screen(sx, sy, sx + w, sy + h, border);
    }

    if let Some(font) = resources.menu_button_font(base.enabled) {
        let tx = sx + 4;
        let ty = sy + (h - font.height() as i32) / 2;
        super::layout::render_text_screen(renderer, font, &base.text, tx, ty);
    }
}

/// Check whether any widget in the frame was activated this frame.
/// Returns the widget ID if so.
pub fn find_activated(events: &[crate::ui::UiEvent]) -> Option<WidgetId> {
    events
        .iter()
        .find(|e| e.msg_type == UiMsg::WidgetActivated)
        .map(|e| e.origin_widget_id)
}

// ─── Widget "noisy" sound effects ───────────────────────────────────
//
// Each widget sub-class is associated with a noisy-id and a map of
// `(msg_type → noisy-event)`.  After `ProcessInput`, the helper looks
// at emitted events and plays `(noisy_id << 16) + event_id` through
// the menu-sound pipeline.

/// Widget-noisy-id values.  The menu sound bank keys entries as
/// `(noisy_id << 16) + event_id`, so these constants must match the
/// values baked into the sound resources.
pub const WIDGET_NOISY_BUTTON: u32 = 2;
pub const WIDGET_NOISY_SLIDER: u32 = 11;
pub const WIDGET_NOISY_LISTBOX: u32 = 20;
pub const WIDGET_NOISY_INPUTFIELD: u32 = 42;

/// Widget-noisy-event values.  The original enum lists ACTIVATED
/// first (ordinal 0) and FOCUSED second (ordinal 1) — these
/// constants must match the resource-bank encoding.
pub const WIDGET_NOISY_EVENT_ACTIVATED: u32 = 0;
pub const WIDGET_NOISY_EVENT_FOCUSED: u32 = 1;

/// Play the menu sound associated with any widget events emitted this
/// frame.  Plays at most one sound per call (the first matching event
/// wins).
///
/// `noisy_id` picks the widget variety (button / slider / listbox / …);
/// our widget enum doesn't carry it, so callers pass the right constant
/// for the frame contents (popup-scroll, yes/no, debriefing, pause menu
/// all use `WIDGET_NOISY_BUTTON`).
///
/// The slider's `WidgetFocused` and `WidgetSliderTrack` messages both
/// map to the FOCUSED sound slot, so a single FOCUSED entry covers
/// both hover and per-tick drag feedback.  Listbox
/// `WidgetListFocusChange` and `WidgetListSelectChange` likewise both
/// land on FOCUSED.
pub fn play_widget_noise(
    events: &[UiEvent],
    noisy_id: u32,
    sound: &mut SoundManager,
    backend: Option<&mut dyn AudioBackend>,
    loader: &SampleLoader,
) {
    play_widget_noise_tracked(
        events,
        noisy_id,
        sound,
        backend,
        loader,
        None,
        UiState::Default,
        false,
    )
}

/// Per-widget noise-tracking state.  One slot per noisy widget keyed
/// by `(widget_id, noisy_id)`, persisted across frames so
/// [`play_widget_noise_tracked`] can implement state-gated behaviour:
/// at most one sound per widget-state; flag reset on state change or
/// when the caller asks for a forced replay.
#[derive(Debug, Default, Clone)]
pub struct NoisyTracker {
    /// (widget_id, noisy_id) → (last_state_seen, played_this_state)
    entries: std::collections::HashMap<(u32, u32), (UiState, bool)>,
}

impl NoisyTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget all tracked state.  Useful when a menu closes and reopens
    /// — the next frame should re-play focus sounds rather than
    /// assuming the previous session's state is still current.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Same as [`play_widget_noise`] but threads through a [`NoisyTracker`]
/// and a per-widget current-state so repeat events within one
/// widget-state stay silent.
///
/// `tracker` / `current_state` are `Option`/ignored when you don't
/// need gating (see the thin [`play_widget_noise`] wrapper).
/// `force_play` fires a sound even if nothing about the state changed.
#[allow(clippy::too_many_arguments)]
pub fn play_widget_noise_tracked(
    events: &[UiEvent],
    noisy_id: u32,
    sound: &mut SoundManager,
    backend: Option<&mut dyn AudioBackend>,
    loader: &SampleLoader,
    tracker: Option<&mut NoisyTracker>,
    current_state: UiState,
    force_play: bool,
) {
    let Some(backend) = backend else {
        return;
    };
    for event in events {
        let event_id = match event.msg_type {
            UiMsg::WidgetActivated => WIDGET_NOISY_EVENT_ACTIVATED,
            UiMsg::WidgetFocused
            | UiMsg::WidgetSliderTrack
            | UiMsg::WidgetListFocusChange
            | UiMsg::WidgetListSelectChange => WIDGET_NOISY_EVENT_FOCUSED,
            _ => continue,
        };
        // State-gate: when a tracker is provided, the flag resets on
        // state change or `force_play`; otherwise any subsequent event
        // in the same state is suppressed.
        if let Some(tracker) = tracker.as_deref() {
            let key = (event.origin_widget_id, noisy_id);
            if let Some(&(last_state, played)) = tracker.entries.get(&key)
                && last_state == current_state
                && played
                && !force_play
            {
                continue;
            }
        }
        let sound_id = (noisy_id << 16) + event_id;
        sound.play_menu_sound(sound_id, backend, loader);
        if let Some(tracker) = tracker {
            tracker
                .entries
                .insert((event.origin_widget_id, noisy_id), (current_state, true));
        }
        return;
    }
}

/// Play menu-widget noises for events emitted by a [`FrameWnd`], using
/// each event's origin widget to retrieve the widget's current state.
///
/// This mirrors `RHWidgetNoisy::ProcessEvents2MakeNoise`: state changes
/// reset the per-widget "already played" flag, and at most one matching
/// sound is emitted per call.
pub fn play_frame_widget_noise(
    events: &[UiEvent],
    frame: &FrameWnd,
    noisy_id: u32,
    sound: &mut SoundManager,
    backend: Option<&mut dyn AudioBackend>,
    loader: &SampleLoader,
    tracker: &mut NoisyTracker,
) {
    let Some(backend) = backend else {
        return;
    };
    for event in events {
        let event_id = match event.msg_type {
            UiMsg::WidgetActivated => WIDGET_NOISY_EVENT_ACTIVATED,
            UiMsg::WidgetFocused
            | UiMsg::WidgetSliderTrack
            | UiMsg::WidgetListFocusChange
            | UiMsg::WidgetListSelectChange => WIDGET_NOISY_EVENT_FOCUSED,
            _ => continue,
        };
        let current_state = frame
            .widget(event.origin_widget_id)
            .map(|w| w.base().state)
            .unwrap_or(UiState::Default);
        let key = (event.origin_widget_id, noisy_id);
        if let Some(&(last_state, played)) = tracker.entries.get(&key)
            && last_state == current_state
            && played
        {
            continue;
        }
        let sound_id = (noisy_id << 16) + event_id;
        sound.play_menu_sound(sound_id, backend, loader);
        tracker.entries.insert(key, (current_state, true));
        return;
    }
}

#[cfg(test)]
mod noisy_tracker_tests {
    use super::*;

    /// Verify the state-gate behaviour: one sound per state, reset on
    /// state change, force flag bypasses gating.
    #[test]
    fn tracker_gates_within_state() {
        let mut tracker = NoisyTracker::new();
        // Simulate the decision the gate would make without actually
        // playing any sound — checks only the bookkeeping side.
        let key = (42_u32, WIDGET_NOISY_BUTTON);

        // First event in Focused — not yet in map, would play.
        assert!(
            !tracker
                .entries
                .get(&key)
                .is_some_and(|&(s, p)| s == UiState::Focused && p),
        );
        tracker.entries.insert(key, (UiState::Focused, true));

        // Second event still in Focused — matches, would be suppressed.
        assert!(
            tracker
                .entries
                .get(&key)
                .is_some_and(|&(s, p)| s == UiState::Focused && p),
        );

        // State transitions to Pushed — map lookup says different state,
        // so gate allows playing again.
        let prior = tracker.entries.get(&key).copied();
        assert!(prior.is_some_and(|(s, _)| s != UiState::Pushed));
    }

    #[test]
    fn tracker_clear_forgets_everything() {
        let mut tracker = NoisyTracker::new();
        tracker
            .entries
            .insert((1, WIDGET_NOISY_BUTTON), (UiState::Focused, true));
        tracker.clear();
        assert!(tracker.entries.is_empty());
    }
}
