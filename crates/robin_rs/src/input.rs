//! Low-level input system.
//!
//! Captures winit keyboard/mouse events into a persistent keyboard state,
//! absolute mouse position, and per-frame wheel accumulator that the
//! game loop reads.  SDL polling is driven by `GameWindow` in `sdl.rs`
//! rather than a blocking polling thread; per-device input-manager
//! bookkeeping is unused and has been removed.
//!
//! Related concerns live in other modules:
//! - Software-cursor rendering, shadow / jerky / pulsating / sniper
//!   effects, and the mouse-background save/restore stack → `cursor.rs`.
//! - Mouse button edge detection (click / double-click / down flags) →
//!   `ingame_menu::widget_bridge::ModalInputState`.
//! - Gamepad/joystick state → `gamepad.rs`.

use robin_engine::coordinates::ScreenPoint;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use crate::geo2d::BBox2D;
use crate::gfx_types::{GameEvent, Keycode};

/// Mouse button identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Keyboard state keyed by winit physical key location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyboardState {
    pub keys: BTreeSet<KeyCode>,
}

impl Default for KeyboardState {
    fn default() -> Self {
        Self {
            keys: BTreeSet::new(),
        }
    }
}

impl KeyboardState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check whether a physical key is currently pressed.
    #[inline]
    pub fn is_pressed(&self, key: KeyCode) -> bool {
        self.keys.contains(&key)
    }
}

// ─── ThreadedInput ───────────────────────────────────────────────────

/// Per-frame input state sink.
///
/// Populated from SDL events via [`feed_sdl_events`](Self::feed_sdl_events):
/// persistent keyboard state, absolute mouse position clipped to a
/// window rect, per-frame wheel accumulator, and a quit flag.  Also
/// hosts [`reach_position`](Self::reach_position) for the gamepad's
/// cursor-seeking path.
#[derive(Debug)]
pub struct ThreadedInput {
    keyboard_state: KeyboardState,
    wheel_delta: i16,
    position: ScreenPoint,
    has_position: bool,
    clipping: BBox2D,
    ended: bool,
    /// When `false`, [`feed_sdl_events`](Self::feed_sdl_events) drops
    /// mouse-motion / button events so cinematics, mission briefings,
    /// and movie playback don't leak input through to the game.
    /// Defaults to `true`.
    enabled: bool,
    synthetic_events: Vec<GameEvent>,
}

impl Default for ThreadedInput {
    fn default() -> Self {
        Self {
            keyboard_state: KeyboardState::default(),
            wheel_delta: 0,
            position: ScreenPoint::default(),
            has_position: false,
            clipping: BBox2D::default(),
            ended: false,
            enabled: true,
            synthetic_events: Vec::new(),
        }
    }
}

impl ThreadedInput {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Mouse position ──

    pub fn position(&self) -> ScreenPoint {
        self.position
    }

    pub fn has_position(&self) -> bool {
        self.has_position
    }

    pub fn clear_position(&mut self) {
        self.has_position = false;
    }

    pub fn set_clipping(&mut self, clip: BBox2D) {
        self.clipping = clip;
        // Re-clip immediately after assigning the box, so a cursor that
        // was outside the new rect snaps inside on the same call rather
        // than waiting for the next motion event.
        self.clip_position();
    }

    /// Clamp cursor position to the clipping box.
    fn clip_position(&mut self) {
        if let Some(rect) = self.clipping.0 {
            let min = rect.min();
            let max = rect.max();
            self.position.x = self.position.x.clamp(min.x, max.x - 1.0);
            self.position.y = self.position.y.clamp(min.y, max.y - 1.0);
        }
    }

    pub fn is_ended(&self) -> bool {
        self.ended
    }

    /// Toggle mouse input.  On the false→true edge, drop any pending
    /// synthetic events so a queue built up while disabled doesn't fire
    /// the moment input resumes.
    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled && !self.enabled {
            self.synthetic_events.clear();
        }
        self.enabled = enabled;
    }

    /// Snap the cursor to `target`, queuing one synthetic mouse-motion
    /// event for downstream consumers.  Rejects out-of-clip targets
    /// (returns `false`), otherwise teleports `position` to `target`,
    /// enqueues a `MouseMove` event with the full delta as `xrel/yrel`,
    /// and returns `true`.
    pub fn reach_position(&mut self, target: ScreenPoint) -> bool {
        if self.clipping.is_somewhere() && !self.clipping.contains_point(target.to_geo()) {
            return false;
        }

        let old_pos = self.rounded_position();
        let new_pos = (target.x.round() as i32, target.y.round() as i32);
        self.position = target;
        self.has_position = true;
        self.synthetic_events.push(GameEvent::MouseMove {
            x: new_pos.0,
            y: new_pos.1,
            xrel: new_pos.0 - old_pos.0,
            yrel: new_pos.1 - old_pos.1,
        });
        true
    }

    /// Queue a synthetic `MouseMove` at the current cursor position
    /// with zero relative deltas, fired after a modal dialog closes.
    /// Lets HUD widgets / portraits / buttons that the cursor is parked
    /// on rebuild their hover highlight on the first frame after a
    /// modal closes, without waiting for the user to nudge the mouse.
    pub fn queue_mouse_motion_resync(&mut self) {
        if !self.has_position {
            return;
        }
        let (x, y) = self.rounded_position();
        self.synthetic_events.push(GameEvent::MouseMove {
            x,
            y,
            xrel: 0,
            yrel: 0,
        });
    }

    fn event_button(button: MouseButton) -> u8 {
        match button {
            MouseButton::Left => 1,
            MouseButton::Middle => 2,
            MouseButton::Right => 3,
        }
    }

    fn rounded_position(&self) -> (i32, i32) {
        (
            self.position.x.round() as i32,
            self.position.y.round() as i32,
        )
    }

    /// Simulate a mouse-button press (e.g. gamepad-as-mouse).
    pub fn push_button(&mut self, button: MouseButton) {
        let (x, y) = self.rounded_position();
        self.synthetic_events
            .push(GameEvent::MouseDown(x, y, Self::event_button(button), 1));
    }

    /// Simulate a mouse-button release.  See [`push_button`](Self::push_button).
    pub fn release_button(&mut self, button: MouseButton) {
        let (x, y) = self.rounded_position();
        self.synthetic_events
            .push(GameEvent::MouseUp(x, y, Self::event_button(button)));
    }

    /// Simulate a key press by enqueuing a synthetic `KeyDown` event.
    /// No in-game caller exists today; kept so any future synthetic-key
    /// path doesn't have to re-introduce the queue.
    pub fn push_key(&mut self, physical_key: KeyCode) {
        self.synthetic_events.push(GameEvent::KeyDown {
            physical_key: Some(physical_key),
            keycode: Keycode::Unknown,
        });
    }

    /// Simulate a key release.  See [`push_key`](Self::push_key).
    pub fn release_key(&mut self, physical_key: KeyCode) {
        self.synthetic_events.push(GameEvent::KeyUp {
            physical_key: Some(physical_key),
            keycode: Keycode::Unknown,
        });
    }

    /// Drain synthetic events queued by in-process input producers.
    ///
    /// Mirrors the original queued-input handoff for generated mouse
    /// motion/buttons and keyboard presses; hardware SDL events still
    /// enter through the direct per-frame `GameEvent` pipeline.
    pub fn drain_synthetic_events(&mut self) -> Vec<GameEvent> {
        std::mem::take(&mut self.synthetic_events)
    }

    // ── SDL event bridge ──

    /// Feed SDL events into the input system.
    ///
    /// Updates the persistent keyboard state array, mouse position
    /// (absolute, from SDL — the game loop reads absolute window
    /// coordinates via [`position`](Self::position) for edge-scroll and
    /// UI hit tests), and per-frame wheel accumulator.  Quit flips
    /// [`ended`](Self::is_ended).
    ///
    /// Call once per frame with the output of
    /// [`GameWindow::poll_events`](crate::window::GameWindow::poll_events).
    pub fn feed_sdl_events(&mut self, events: &[GameEvent]) {
        if self.ended {
            return;
        }

        self.wheel_delta = 0;

        for event in events {
            match event {
                GameEvent::KeyDown {
                    physical_key: Some(physical_key),
                    ..
                } => {
                    self.keyboard_state.keys.insert(*physical_key);
                }
                GameEvent::KeyUp {
                    physical_key: Some(physical_key),
                    ..
                } => {
                    self.keyboard_state.keys.remove(physical_key);
                }
                GameEvent::KeyDown {
                    physical_key: None, ..
                }
                | GameEvent::KeyUp {
                    physical_key: None, ..
                } => {}
                GameEvent::MouseMove { x, y, .. } => {
                    // Skip the entire mouse-event branch when disabled
                    // so cinematics / movie playback / mission briefings
                    // don't leak motion through to the game.
                    if !self.enabled {
                        continue;
                    }
                    self.position.x = *x as f32;
                    self.position.y = *y as f32;
                    self.has_position = true;
                    self.clip_position();
                }
                GameEvent::MouseDown(x, y, _btn, _clicks) => {
                    if !self.enabled {
                        continue;
                    }
                    self.position.x = *x as f32;
                    self.position.y = *y as f32;
                    self.has_position = true;
                    self.clip_position();
                }
                GameEvent::MouseUp(x, y, _btn) => {
                    if !self.enabled {
                        continue;
                    }
                    self.position.x = *x as f32;
                    self.position.y = *y as f32;
                    self.has_position = true;
                    self.clip_position();
                }
                GameEvent::MouseWheel(y) => {
                    if !self.enabled {
                        continue;
                    }
                    // Accumulate per-frame so `translate_mouse` sees the
                    // net delta when several wheel events arrive in one
                    // tick (each SDL event carries a ±1 step).
                    self.wheel_delta += *y as i16;
                }
                GameEvent::Quit => {
                    self.ended = true;
                }
                GameEvent::WindowFocusChanged(false) => {
                    self.clear_position();
                }
                GameEvent::Resized(..)
                | GameEvent::TextInput { .. }
                | GameEvent::ViewportPan { .. }
                | GameEvent::MenuToggleRequested
                | GameEvent::PauseRequested
                | GameEvent::GamepadAdded { .. }
                | GameEvent::GamepadRemoved { .. }
                | GameEvent::GamepadAxis { .. }
                | GameEvent::GamepadButton { .. }
                | GameEvent::WindowFocusChanged(true) => {
                    // Handled elsewhere: Resized in the game loop;
                    // TextInput in the widget input-field path;
                    // Gamepad* in `game_session` via `GamePadState`;
                    // WindowFocusChanged in the loading-screen pause
                    // path (and reserved for future game-loop pause).
                }
            }
        }
    }

    /// Current persistent keyboard state.
    pub fn keyboard_state(&self) -> &KeyboardState {
        &self.keyboard_state
    }

    /// Clear cached pressed-key state and drop any pending wheel delta.
    ///
    /// Run after a modal dialog/popup closes so held-key edges queued
    /// during the modal don't re-fire as actions the instant control
    /// returns to the gameplay loop.
    pub fn reset_input_state(&mut self) {
        self.keyboard_state.keys.clear();
        self.wheel_delta = 0;
        self.synthetic_events.clear();
    }

    /// Accumulated mouse wheel delta for the current frame.
    pub fn wheel_delta(&self) -> i16 {
        self.wheel_delta
    }
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_state_default_all_released() {
        let ks = KeyboardState::default();
        assert!(!ks.is_pressed(KeyCode::KeyA));
        assert!(!ks.is_pressed(KeyCode::Backspace));
        assert!(!ks.is_pressed(KeyCode::F12));
    }

    #[test]
    fn keyboard_state_press() {
        let mut ks = KeyboardState::default();
        ks.keys.insert(KeyCode::Backspace);
        assert!(ks.is_pressed(KeyCode::Backspace));
        assert!(!ks.is_pressed(KeyCode::Tab));
    }

    #[test]
    fn feed_sdl_key_updates_persistent_state() {
        let mut ti = ThreadedInput::new();
        ti.feed_sdl_events(&[GameEvent::KeyDown {
            keycode: Keycode::Char(b'a'),
            physical_key: Some(KeyCode::KeyA),
        }]);
        assert!(ti.keyboard_state().is_pressed(KeyCode::KeyA));

        ti.feed_sdl_events(&[GameEvent::KeyUp {
            keycode: Keycode::Char(b'a'),
            physical_key: Some(KeyCode::KeyA),
        }]);
        assert!(!ti.keyboard_state().is_pressed(KeyCode::KeyA));
    }

    #[test]
    fn feed_sdl_mouse_move_updates_position() {
        let mut ti = ThreadedInput::new();
        ti.set_clipping(BBox2D::from_coords(0.0, 0.0, 800.0, 600.0));

        ti.feed_sdl_events(&[GameEvent::MouseMove {
            x: 400,
            y: 300,
            xrel: 10,
            yrel: 5,
        }]);
        assert_eq!(ti.position().x, 400.0);
        assert_eq!(ti.position().y, 300.0);

        // Out-of-clip coordinates clamp to (max - 1).
        ti.feed_sdl_events(&[GameEvent::MouseMove {
            x: 1000,
            y: 700,
            xrel: 0,
            yrel: 0,
        }]);
        assert_eq!(ti.position().x, 799.0);
        assert_eq!(ti.position().y, 599.0);
    }

    #[test]
    fn feed_sdl_wheel_accumulates_per_frame_and_resets() {
        let mut ti = ThreadedInput::new();
        ti.feed_sdl_events(&[GameEvent::MouseWheel(2), GameEvent::MouseWheel(-1)]);
        assert_eq!(ti.wheel_delta(), 1);

        // A fresh frame resets the accumulator even when no wheel arrives.
        ti.feed_sdl_events(&[]);
        assert_eq!(ti.wheel_delta(), 0);
    }

    #[test]
    fn feed_sdl_quit_sets_ended_and_stops_further_processing() {
        let mut ti = ThreadedInput::new();
        ti.feed_sdl_events(&[GameEvent::Quit]);
        assert!(ti.is_ended());

        // After Quit, subsequent events are ignored.
        ti.feed_sdl_events(&[GameEvent::KeyDown {
            keycode: Keycode::Char(b'a'),
            physical_key: Some(KeyCode::KeyA),
        }]);
        assert!(!ti.keyboard_state().is_pressed(KeyCode::KeyA));
    }

    #[test]
    fn reach_position_teleports_within_clip_and_queues_motion_event() {
        let mut ti = ThreadedInput::new();
        ti.set_clipping(BBox2D::from_coords(0.0, 0.0, 800.0, 600.0));

        // Inside clip — teleport, returns true.
        assert!(ti.reach_position(ScreenPoint::new(500.0, 400.0)));
        assert_eq!(ti.position().x, 500.0);
        assert_eq!(ti.position().y, 400.0);
        let evs = ti.drain_synthetic_events();
        assert!(matches!(
            evs[0],
            GameEvent::MouseMove {
                x: 500,
                y: 400,
                xrel: 500,
                yrel: 400
            }
        ));

        // Outside clip — refuses, position unchanged.
        assert!(!ti.reach_position(ScreenPoint::new(900.0, 400.0)));
        assert_eq!(ti.position().x, 500.0);
        assert!(ti.drain_synthetic_events().is_empty());
    }

    #[test]
    fn mouse_events_dropped_when_disabled() {
        let mut ti = ThreadedInput::new();
        ti.set_clipping(BBox2D::from_coords(0.0, 0.0, 800.0, 600.0));
        ti.feed_sdl_events(&[GameEvent::MouseMove {
            x: 100,
            y: 100,
            xrel: 0,
            yrel: 0,
        }]);
        assert_eq!(ti.position().x, 100.0);

        ti.set_enabled(false);
        ti.feed_sdl_events(&[GameEvent::MouseMove {
            x: 200,
            y: 200,
            xrel: 0,
            yrel: 0,
        }]);
        // Position frozen.
        assert_eq!(ti.position().x, 100.0);

        // Re-enabling clears any queued synthetic events.
        ti.push_button(MouseButton::Left);
        ti.set_enabled(true);
        assert!(ti.drain_synthetic_events().is_empty());
    }

    #[test]
    fn synthetic_keys_drain_as_game_events() {
        let mut ti = ThreadedInput::new();
        ti.push_key(KeyCode::Backspace);
        ti.release_key(KeyCode::Backspace);
        let evs = ti.drain_synthetic_events();
        assert!(matches!(
            evs[0],
            GameEvent::KeyDown {
                physical_key: Some(KeyCode::Backspace),
                ..
            }
        ));
        assert!(matches!(
            evs[1],
            GameEvent::KeyUp {
                physical_key: Some(KeyCode::Backspace),
                ..
            }
        ));
    }

    #[test]
    fn synthetic_mouse_buttons_drain_as_game_events_at_current_position() {
        let mut ti = ThreadedInput::new();
        ti.feed_sdl_events(&[GameEvent::MouseMove {
            x: 123,
            y: 45,
            xrel: 0,
            yrel: 0,
        }]);

        ti.push_button(MouseButton::Left);
        ti.release_button(MouseButton::Left);

        let events = ti.drain_synthetic_events();
        assert!(matches!(events[0], GameEvent::MouseDown(123, 45, 1, 1)));
        assert!(matches!(events[1], GameEvent::MouseUp(123, 45, 1)));
        assert!(ti.drain_synthetic_events().is_empty());
    }
}
