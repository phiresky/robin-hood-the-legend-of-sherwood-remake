//! Leaderboard presentation and upload-consent settings.
//!
//! These preferences affect presentation and upload consent only. Replay
//! capture, the compact replay format, and ranked protocol validation are not
//! user-toggleable.

use crate::gfx_types::{GameEvent, Keycode};
use crate::leaderboard_preferences::LeaderboardPreferences;
use crate::renderer::Renderer;
use crate::widget::FrameWnd;

use super::layout::{
    MenuTransform, TooltipState, align_bottom_right, dim_screen, draw_screen_background,
    enter_modal_gpu_phase, render_text_virt_font,
};
use super::resources::{IngameMenuResources, MT_BTN_CANCEL, MT_BTN_OK};
use super::widget_bridge::{self, ModalCursor, ModalInputState, ModalScreenIo};

const ID_SHOW_MISSION_END_BOARDS: u32 = 0;
const ID_ALWAYS_SUBMIT: u32 = 1;
const ID_OK: u32 = 100;
const ID_CANCEL: u32 = 101;

const OPTIONS: [(&str, &str); 2] = [
    (
        "Mission-end Leaderboards",
        "Automatically show verified boards after wins, losses, and interrupted missions.",
    ),
    (
        "Always Submit Won Runs",
        "Automatically upload each eligible won run. Off keeps per-run consent.",
    ),
];

/// Show the leaderboard settings screen. Changes remain staged until OK.
pub async fn show_leaderboard_settings(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<ModalCursor<'_>>,
    preferences: &mut LeaderboardPreferences,
) -> bool {
    let mut state =
        LeaderboardSettingsModalState::new(event_pump, renderer, resources, preferences);
    let mut io = ModalScreenIo {
        window: event_pump,
        renderer,
        resources,
        cursor: cursor.as_ref(),
    };
    loop {
        let done = state.tick(&mut io);
        crate::window::sleep_ui_frame().await;
        if done {
            return state.commit(preferences);
        }
    }
}

/// Retained widget/input and staged preference state for one modal frame.
pub struct LeaderboardSettingsModalState {
    working: LeaderboardPreferences,
    dirty: bool,
    frame: FrameWnd,
    input: ModalInputState,
    tooltip: TooltipState,
    accepted: bool,
    done: bool,
}

impl LeaderboardSettingsModalState {
    pub fn new(
        event_pump: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        preferences: &LeaderboardPreferences,
    ) -> Self {
        let working = preferences.clone();
        let dirty = false;
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let (field_w, field_h) = resources.input_field_dimensions();
        let mut frame = FrameWnd::interactive();
        for (index, (label, tooltip)) in OPTIONS.iter().enumerate() {
            let id = u32::try_from(index).expect("leaderboard option index fits u32");
            frame.add_widget_absolute(widget_bridge::make_button(
                id,
                label,
                60,
                135 + i32::try_from(index).expect("leaderboard option index fits i32") * 54,
                field_w.max(270),
                field_h,
            ));
            frame
                .widget_mut(id)
                .expect("new leaderboard option widget")
                .base_mut()
                .set_tooltip_text(tooltip);
        }

        let (button_w, button_h) = resources.button_dimensions();
        let ok_label = resources.menu_text.get(MT_BTN_OK);
        let cancel_label = resources.menu_text.get(MT_BTN_CANCEL);
        let bottom = align_bottom_right(
            &[(&ok_label, true), (&cancel_label, true)],
            button_w,
            button_h,
        );
        frame.add_widget_absolute(widget_bridge::make_button(
            ID_OK,
            &bottom[0].label,
            bottom[0].x,
            bottom[0].y,
            bottom[0].w,
            bottom[0].h,
        ));
        frame.add_widget_absolute(widget_bridge::make_button(
            ID_CANCEL,
            &bottom[1].label,
            bottom[1].x,
            bottom[1].y,
            bottom[1].w,
            bottom[1].h,
        ));

        let mut input = ModalInputState::new();
        let tooltip = TooltipState::new();
        input.seed_mouse_from_window(event_pump, transform);

        Self {
            working,
            dirty,
            frame,
            input,
            tooltip,
            accepted: false,
            done: false,
        }
    }

    /// Poll and draw exactly one frame; the caller owns pacing.
    pub fn tick(&mut self, io: &mut ModalScreenIo<'_, '_>) -> bool {
        let event_pump = &mut *io.window;
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        let cursor = io.cursor;
        if self.done {
            return true;
        }
        let (events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            self.input.update_from_event(&event, transform);
            match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => self.done = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Return | Keycode::KpEnter,
                    ..
                } => {
                    self.accepted = true;
                    self.done = true;
                }
                _ => {}
            }
        }
        let events = self.input.process_frame(&mut self.frame);
        if let Some(id) = widget_bridge::find_activated(&events) {
            self.activate(id);
        }

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        if let Some(background) = resources.menu_bg[0] {
            draw_screen_background(renderer, &background);
        }
        if let Some(font) = resources.title_font_any() {
            let title = "Leaderboards";
            render_text_virt_font(
                renderer,
                font,
                transform,
                title,
                (490 - font.text_width(title)) / 2,
                20,
            );
        }
        if let Some(font) = resources.label_font_any() {
            render_text_virt_font(
                renderer,
                font,
                transform,
                "Verified leaderboards and replay submission",
                60,
                90,
            );
        }
        for id in [ID_SHOW_MISSION_END_BOARDS, ID_ALWAYS_SUBMIT] {
            let widget = self
                .frame
                .widget(id)
                .expect("leaderboard option widget exists");
            widget_bridge::draw_widget_radio(
                renderer,
                resources,
                transform,
                widget,
                selected(&self.working, id),
            );
        }
        let mouse =
            robin_engine::coordinates::ScreenPoint::new(self.input.virt_x, self.input.virt_y);
        self.tooltip.update(&self.frame, mouse);
        if let Some(font) = resources.popup_font_any() {
            self.tooltip
                .draw(renderer, font, transform, &self.frame, mouse);
        }
        for id in [ID_OK, ID_CANCEL] {
            if let Some(widget) = self.frame.widget(id) {
                widget_bridge::draw_widget_button(renderer, resources, transform, widget, false);
            }
        }
        if let Some(cursor) = cursor {
            cursor.draw(renderer, transform, &self.input);
        }
        renderer.present();

        self.done
    }

    fn activate(&mut self, id: u32) {
        match id {
            ID_SHOW_MISSION_END_BOARDS | ID_ALWAYS_SUBMIT => {
                toggle(&mut self.working, id);
                self.dirty = true;
            }
            ID_OK => {
                self.accepted = true;
                self.done = true;
            }
            ID_CANCEL => self.done = true,
            _ => {}
        }
    }

    /// Publish staged changes only after acceptance and an actual difference.
    pub fn commit(self, target: &mut LeaderboardPreferences) -> bool {
        if self.accepted && self.dirty && self.working != *target {
            *target = self.working;
            true
        } else {
            false
        }
    }
}

fn toggle(preferences: &mut LeaderboardPreferences, id: u32) {
    match id {
        ID_SHOW_MISSION_END_BOARDS => {
            preferences.show_mission_end_boards = !preferences.show_mission_end_boards;
        }
        ID_ALWAYS_SUBMIT => {
            preferences.always_submit_eligible_runs = !preferences.always_submit_eligible_runs;
        }
        _ => {}
    }
}

fn selected(preferences: &LeaderboardPreferences, id: u32) -> bool {
    match id {
        ID_SHOW_MISSION_END_BOARDS => preferences.show_mission_end_boards,
        ID_ALWAYS_SUBMIT => preferences.always_submit_eligible_runs,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(preferences: LeaderboardPreferences) -> LeaderboardSettingsModalState {
        LeaderboardSettingsModalState {
            working: preferences,
            dirty: false,
            frame: FrameWnd::interactive(),
            input: ModalInputState::new(),
            tooltip: TooltipState::new(),
            accepted: false,
            done: false,
        }
    }

    #[test]
    fn staged_consent_requires_acceptance_and_a_real_change() {
        for (actions, changed) in [
            (vec![ID_ALWAYS_SUBMIT, ID_OK], true),
            (vec![ID_ALWAYS_SUBMIT, ID_CANCEL], false),
            (vec![ID_ALWAYS_SUBMIT, ID_ALWAYS_SUBMIT, ID_OK], false),
            (vec![ID_OK], false),
        ] {
            let original = LeaderboardPreferences::default();
            let mut target = original.clone();
            let mut state = state(original.clone());
            for action in actions {
                state.activate(action);
                assert_eq!(target, original, "editing must not publish staged consent");
            }
            assert!(state.done);
            assert_eq!(state.commit(&mut target), changed);
            assert_eq!(target != original, changed);
            assert_eq!(target.always_submit_eligible_runs, changed);
            assert_eq!(
                target.show_mission_end_boards,
                original.show_mission_end_boards
            );
        }
    }

    #[test]
    fn toggles_are_independent_and_keep_consent_off_by_default() {
        let mut preferences = LeaderboardPreferences::default();
        assert!(selected(&preferences, ID_SHOW_MISSION_END_BOARDS));
        assert!(!selected(&preferences, ID_ALWAYS_SUBMIT));

        toggle(&mut preferences, ID_SHOW_MISSION_END_BOARDS);
        assert!(!preferences.show_mission_end_boards);
        assert!(!preferences.always_submit_eligible_runs);

        toggle(&mut preferences, ID_ALWAYS_SUBMIT);
        assert!(!preferences.show_mission_end_boards);
        assert!(preferences.always_submit_eligible_runs);
    }
}
