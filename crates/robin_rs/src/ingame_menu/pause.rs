//! In-game pause menu.
//!
//! A full-screen 640x480 menu with six vertical buttons (Continue /
//! Load / Save / Options / Restart / Quit Game) aligned bottom-right,
//! and the short-briefings list shown on the left half of the window
//! at `(2,2)..(440,480)`.
//!
//! Unlike the other in-game menus the pause menu is driven from the
//! main game loop as a non-blocking state machine: events from the
//! same input queue the rest of the game consumes are fed into
//! [`PauseMenu::handle_event`], and [`PauseMenu::render`] is called
//! once per frame in the GPU phase on top of the dimmed game view.
//!
//! Side menus (Options hub, confirmation dialogs) are still launched
//! through blocking calls from `game_session.rs`.
//!
//! Buttons are driven by the [`crate::widget`] system via the
//! [`super::widget_bridge`].

use crate::gfx_types::Keycode;
use robin_engine::sound_cache::SampleLoader;

use crate::gfx_types::GameEvent;
use crate::renderer::Renderer;
use crate::short_briefings::ShortBriefings;
use crate::sound::{AudioBackend, SoundManager};
use crate::ui::UiState;
use crate::widget::FrameWnd;

use super::briefings::draw_short_briefings;
use super::layout::{MenuRect, MenuTransform, align_bottom_right, dim_screen};
use super::resources::{
    IngameMenuResources, MT_BTN_CONTINUE, MT_BTN_LOAD, MT_BTN_OPTIONS, MT_BTN_QUIT_GAME,
    MT_BTN_RESTART, MT_BTN_SAVE,
};
use super::widget_bridge::{self, ModalInputState};

/// Button indices / widget IDs.  Order is the widget creation order so
/// `align_bottom_right` produces the canonical vertical stack.
pub const PAUSE_BTN_CONTINUE: u32 = 0;
pub const PAUSE_BTN_LOAD: u32 = 1;
pub const PAUSE_BTN_SAVE: u32 = 2;
pub const PAUSE_BTN_OPTIONS: u32 = 3;
pub const PAUSE_BTN_RESTART: u32 = 4;
pub const PAUSE_BTN_QUIT: u32 = 5;

/// Short briefings area inside the window: `(2, 2)..(440, 480)`.
const BRIEFINGS_RECT: MenuRect = MenuRect {
    x: 2,
    y: 2,
    w: 438,
    h: 478,
};

/// The player's interaction outcome for the current frame.
///
/// `Pending` means nothing actionable happened.  `Quit` and `Restart`
/// mean the player chose the corresponding action; confirmation dialogs
/// are handled by the caller inside `game_session.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseMenuOutcome {
    Pending,
    /// Resume the mission (Continue button or Escape).
    Continue,
    /// Open the Options hub.  The caller should run it and continue
    /// displaying the pause menu afterwards.
    OpenOptions,
    /// Open the Load slot picker.  The caller runs it, optionally
    /// emits a `SaveLoadRequest::Load`, then returns to this menu.
    OpenLoad,
    /// Open the Save slot picker.  The caller runs it, optionally
    /// emits a `SaveLoadRequest::Save`, then returns to this menu.
    OpenSave,
    /// Restart the mission.
    Restart,
    /// Quit to the main menu.
    Quit,
}

/// State machine for the in-game pause menu.
pub struct PauseMenu {
    frame: FrameWnd,
    input_state: ModalInputState,
    noisy_tracker: widget_bridge::NoisyTracker,
    keyboard_selection: u32,
    outcome: PauseMenuOutcome,
}

impl PauseMenu {
    /// Build the pause menu with the standard button layout.
    ///
    /// `restart_allowed` is `false` in Sherwood (the hub level has no
    /// mission to restart).
    pub fn new(resources: &IngameMenuResources, restart_allowed: bool) -> Self {
        let (btn_w, btn_h) = resources.button_dimensions();

        // Localised button labels.
        let continue_txt = resources.menu_text.get(MT_BTN_CONTINUE);
        let load_txt = resources.menu_text.get(MT_BTN_LOAD);
        let save_txt = resources.menu_text.get(MT_BTN_SAVE);
        let options_txt = resources.menu_text.get(MT_BTN_OPTIONS);
        let restart_txt = resources.menu_text.get(MT_BTN_RESTART);
        let quit_txt = resources.menu_text.get(MT_BTN_QUIT_GAME);
        let labels: &[(&str, bool)] = &[
            (&continue_txt, true),
            (&load_txt, true),
            (&save_txt, true),
            (&options_txt, true),
            (&restart_txt, restart_allowed),
            (&quit_txt, true),
        ];

        let menu_buttons = align_bottom_right(labels, btn_w, btn_h);

        // Build a FrameWnd with widget buttons matching the layout.
        let mut frame = FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        for (i, mb) in menu_buttons.iter().enumerate() {
            frame.add_widget_absolute(widget_bridge::make_button_enabled(
                i as u32, &mb.label, mb.enabled, mb.x, mb.y, mb.w, mb.h,
            ));
        }

        Self {
            frame,
            input_state: ModalInputState::new(),
            noisy_tracker: widget_bridge::NoisyTracker::new(),
            keyboard_selection: PAUSE_BTN_CONTINUE,
            outcome: PauseMenuOutcome::Pending,
        }
    }

    pub fn outcome(&self) -> PauseMenuOutcome {
        self.outcome
    }

    /// Reset transient input state after a blocking side-menu
    /// (Load/Save/Options/YesNo) returns.  The side-menu owned the
    /// input pump while it was up, so the pause menu's last-seen
    /// button flags and the widget hover/pressed states are stale by
    /// the time control returns.  Clearing them prevents the
    /// Save/Load/Options button from appearing stuck in "Pressed" on
    /// the first post-return frame and prevents a still-set
    /// `LEFT_DOWN` from synthesising a phantom drag.
    ///
    /// Callers should follow this with
    /// [`ModalInputState::seed_mouse_from_window`] via
    /// [`Self::seed_mouse_from_window`] so the cursor reads the current
    /// mouse position immediately instead of waiting for the next
    /// `MouseMove` event.
    pub fn reset_after_side_menu(&mut self) {
        self.outcome = PauseMenuOutcome::Pending;
        self.input_state = ModalInputState::new();
        self.noisy_tracker.clear();
        for widget in self.frame.widgets_mut() {
            widget.base_mut().state = UiState::Default;
        }
    }

    /// Re-seed the pause-menu cursor from the live window mouse state.
    /// Used after a blocking side-menu returns so the cursor renders
    /// at the real position instead of wherever it was when the side
    /// menu was launched.
    pub fn seed_mouse_from_window(
        &mut self,
        event_pump: &crate::window::GameWindow,
        screen_w: i32,
        screen_h: i32,
    ) {
        let transform = MenuTransform::centered(screen_w, screen_h);
        self.input_state
            .seed_mouse_from_window(event_pump, transform);
    }

    /// Feed a single event to the menu and return the updated outcome.
    pub fn handle_event(
        &mut self,
        event: &GameEvent,
        screen_w: i32,
        screen_h: i32,
    ) -> PauseMenuOutcome {
        self.handle_event_with_audio(event, screen_w, screen_h, None, None, None)
    }

    /// Feed a single event to the menu and return the updated outcome.
    /// When audio is supplied, hover and activation events trigger the
    /// same menu-button sounds as the original widget implementation.
    pub fn handle_event_with_audio(
        &mut self,
        event: &GameEvent,
        screen_w: i32,
        screen_h: i32,
        sound: Option<&mut SoundManager>,
        audio_backend: Option<&mut dyn AudioBackend>,
        sample_loader: Option<&SampleLoader>,
    ) -> PauseMenuOutcome {
        let transform = MenuTransform::centered(screen_w, screen_h);

        // Keyboard shortcuts (focus-manager behaviour).
        match *event {
            GameEvent::KeyDown {
                keycode: Keycode::Escape,
                ..
            } => {
                self.outcome = PauseMenuOutcome::Continue;
                return self.outcome;
            }
            GameEvent::KeyDown {
                keycode: Keycode::Up,
                ..
            } => self.move_keyboard_selection(-1),
            GameEvent::KeyDown {
                keycode: Keycode::Down,
                ..
            } => self.move_keyboard_selection(1),
            GameEvent::KeyDown {
                keycode: Keycode::Return,
                ..
            }
            | GameEvent::KeyDown {
                keycode: Keycode::KpEnter,
                ..
            } => {
                // Return / KpEnter activate the currently-focused
                // widget. Space is intentionally not a focus-manager
                // activation key — don't add it.
                self.activate(self.keyboard_selection);
                return self.outcome;
            }
            _ => {}
        }

        // Mouse input → widget state machine.
        self.input_state.update_from_event(event, transform);
        let widget_input = self.input_state.as_widget_input();
        let events = self.frame.process_input(&widget_input);
        self.input_state.end_frame();
        if let (Some(sound), Some(sample_loader)) = (sound, sample_loader) {
            widget_bridge::play_frame_widget_noise(
                &events,
                &self.frame,
                widget_bridge::WIDGET_NOISY_BUTTON,
                sound,
                audio_backend,
                sample_loader,
                &mut self.noisy_tracker,
            );
        }

        // Sync keyboard selection with mouse hover.
        for w in self.frame.widgets() {
            if w.base().state != UiState::Default && w.base().enabled {
                self.keyboard_selection = w.id();
            }
        }

        if let Some(id) = widget_bridge::find_activated(&events) {
            self.activate(id);
        }

        self.outcome
    }

    fn move_keyboard_selection(&mut self, direction: i32) {
        let len = self.frame.widget_count() as i32;
        if len == 0 {
            return;
        }
        let mut idx = self.keyboard_selection as i32;
        for _ in 0..len {
            idx = (idx + direction).rem_euclid(len);
            if let Some(w) = self.frame.widget_at(idx as usize)
                && w.base().enabled
            {
                self.keyboard_selection = idx as u32;
                break;
            }
        }
    }

    fn activate(&mut self, id: u32) {
        if let Some(w) = self.frame.widget(id)
            && !w.base().enabled
        {
            return;
        }
        self.outcome = match id {
            PAUSE_BTN_CONTINUE => PauseMenuOutcome::Continue,
            PAUSE_BTN_LOAD => PauseMenuOutcome::OpenLoad,
            PAUSE_BTN_SAVE => PauseMenuOutcome::OpenSave,
            PAUSE_BTN_OPTIONS => PauseMenuOutcome::OpenOptions,
            PAUSE_BTN_RESTART => PauseMenuOutcome::Restart,
            PAUSE_BTN_QUIT => PauseMenuOutcome::Quit,
            _ => PauseMenuOutcome::Pending,
        };
    }

    /// Render the pause menu: dim background, short briefings on the
    /// left, button column on the right.
    pub fn render(
        &self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        briefings: Option<&ShortBriefings>,
        text_lookup: &dyn Fn(u32) -> Option<String>,
    ) {
        let sw = renderer.screen_width() as i32;
        let sh = renderer.screen_height() as i32;
        let transform = MenuTransform::centered(sw, sh);

        dim_screen(renderer);

        if let Some(briefings) = briefings {
            draw_short_briefings(
                renderer,
                resources,
                transform,
                &BRIEFINGS_RECT,
                briefings,
                text_lookup,
            );
        }

        for widget in self.frame.widgets() {
            let kb_highlight =
                widget.id() == self.keyboard_selection && widget.base().state == UiState::Default;
            widget_bridge::draw_widget_button(renderer, resources, transform, widget, kb_highlight);
        }
    }

    /// Button count, exposed for tests.
    #[cfg(test)]
    pub(crate) fn button_count(&self) -> usize {
        self.frame.widget_count()
    }

    /// Access the `i`th button's enabled state, for tests.
    #[cfg(test)]
    pub(crate) fn button_enabled(&self, i: usize) -> bool {
        self.frame.widget_at(i).is_some_and(|w| w.base().enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::MouseButtons;

    fn stub_resources() -> IngameMenuResources {
        IngameMenuResources::stub()
    }

    #[test]
    fn pause_menu_has_six_buttons() {
        let resources = stub_resources();
        let menu = PauseMenu::new(&resources, true);
        assert_eq!(menu.button_count(), 6);
    }

    #[test]
    fn pause_menu_escape_continues() {
        let resources = stub_resources();
        let mut menu = PauseMenu::new(&resources, true);
        let event = GameEvent::KeyDown {
            keycode: Keycode::Escape,
            physical_key: Some(winit::keyboard::KeyCode::Escape),
        };
        assert_eq!(
            menu.handle_event(&event, 1024, 768),
            PauseMenuOutcome::Continue
        );
    }

    #[test]
    fn pause_menu_restart_disabled_in_sherwood() {
        let resources = stub_resources();
        let menu = PauseMenu::new(&resources, false);
        assert!(!menu.button_enabled(PAUSE_BTN_RESTART as usize));
    }

    #[test]
    fn pause_menu_keyboard_nav_skips_disabled() {
        let resources = stub_resources();
        // restart_allowed = false so Restart is disabled — nav must skip it.
        let mut menu = PauseMenu::new(&resources, false);
        // Continue → Load → Save → Options → Quit (Restart skipped) → Continue
        menu.move_keyboard_selection(1);
        assert_eq!(menu.keyboard_selection, PAUSE_BTN_LOAD);
        menu.move_keyboard_selection(1);
        assert_eq!(menu.keyboard_selection, PAUSE_BTN_SAVE);
        menu.move_keyboard_selection(1);
        assert_eq!(menu.keyboard_selection, PAUSE_BTN_OPTIONS);
        menu.move_keyboard_selection(1);
        assert_eq!(menu.keyboard_selection, PAUSE_BTN_QUIT);
        menu.move_keyboard_selection(1);
        assert_eq!(menu.keyboard_selection, PAUSE_BTN_CONTINUE);
    }

    #[test]
    fn pause_menu_options_button_opens_options() {
        let resources = stub_resources();
        let mut menu = PauseMenu::new(&resources, true);
        menu.activate(PAUSE_BTN_OPTIONS);
        assert_eq!(menu.outcome(), PauseMenuOutcome::OpenOptions);
    }

    /// Open Pause → open Save → cancel back → pause still alive →
    /// resume → game continues.  When the save dialog is cancelled,
    /// control returns to the pause menu, after which the player can
    /// still pick Continue to resume.
    #[test]
    fn pause_save_cancel_continue_flow() {
        let resources = stub_resources();
        let mut menu = PauseMenu::new(&resources, true);

        // Initial state: no outcome.
        assert_eq!(menu.outcome(), PauseMenuOutcome::Pending);

        // Click Save → OpenSave outcome is emitted so `game_session`
        // can launch the slot picker.
        menu.activate(PAUSE_BTN_SAVE);
        assert_eq!(menu.outcome(), PauseMenuOutcome::OpenSave);

        // Simulate cancelling the save dialog: `game_session` calls
        // `reset_after_side_menu` which clears the outcome and the
        // stale widget/input state.
        menu.reset_after_side_menu();
        assert_eq!(menu.outcome(), PauseMenuOutcome::Pending);
        // Every widget must be back to Default — otherwise the Save
        // button would still look pressed on the first frame after
        // return.
        for w in menu.frame.widgets() {
            assert_eq!(
                w.base().state,
                UiState::Default,
                "widget {} state not reset after side-menu",
                w.id()
            );
        }

        // Player hits Escape → Continue outcome; caller tears down the
        // menu and the game unpauses.
        let esc = GameEvent::KeyDown {
            keycode: Keycode::Escape,
            physical_key: Some(winit::keyboard::KeyCode::Escape),
        };
        assert_eq!(
            menu.handle_event(&esc, 1024, 768),
            PauseMenuOutcome::Continue
        );
    }

    /// `reset_after_side_menu` must also clear a lingering `LEFT_DOWN`
    /// in the modal input state — otherwise the first MouseMove event
    /// after the side menu returns would act as a drag.
    #[test]
    fn reset_after_side_menu_clears_held_buttons() {
        let resources = stub_resources();
        let mut menu = PauseMenu::new(&resources, true);

        // Simulate a pressed left button inherited from before the
        // side menu launched.
        menu.input_state.buttons |= MouseButtons::LEFT_DOWN;
        assert!(menu.input_state.buttons.contains(MouseButtons::LEFT_DOWN));

        menu.reset_after_side_menu();

        assert!(
            !menu.input_state.buttons.contains(MouseButtons::LEFT_DOWN),
            "LEFT_DOWN should be cleared after side-menu return"
        );
    }

    /// Hitting Escape in the pause menu must produce `Continue`
    /// without first navigating through the buttons — Escape is wired
    /// directly to the Continue button's activation shortcut.
    #[test]
    fn pause_escape_bypasses_keyboard_selection() {
        let resources = stub_resources();
        let mut menu = PauseMenu::new(&resources, true);

        // Move the selection away from Continue.
        menu.move_keyboard_selection(1);
        menu.move_keyboard_selection(1);
        assert_ne!(menu.keyboard_selection, PAUSE_BTN_CONTINUE);

        let esc = GameEvent::KeyDown {
            keycode: Keycode::Escape,
            physical_key: Some(winit::keyboard::KeyCode::Escape),
        };
        assert_eq!(
            menu.handle_event(&esc, 1024, 768),
            PauseMenuOutcome::Continue
        );
    }
}
