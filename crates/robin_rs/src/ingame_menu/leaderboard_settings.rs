//! Leaderboard presentation and upload-consent settings.
//!
//! These preferences affect presentation and upload consent only. Replay
//! capture, the compact replay format, and ranked protocol validation are not
//! user-toggleable.

use crate::leaderboard_preferences::LeaderboardPreferences;
use crate::widget::FrameWnd;

use super::layout::{TooltipState, render_text_virt_font};
use super::widget_bridge::{self, ModalInputState, ModalScreenIo, OkCancelScreen, ScreenFrame};

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
    io: &mut ModalScreenIo<'_, '_>,
    preferences: &mut LeaderboardPreferences,
) -> bool {
    let mut state = LeaderboardSettingsModalState::new(io, preferences);
    widget_bridge::run_modal(io, |io| state.tick(io)).await;
    state.commit(preferences)
}

/// Retained widget/input and staged preference state for one modal frame.
pub struct LeaderboardSettingsModalState {
    working: LeaderboardPreferences,
    dirty: bool,
    frame: FrameWnd,
    input: ModalInputState,
    tooltip: TooltipState,
    pub(crate) close: OkCancelScreen,
}

impl LeaderboardSettingsModalState {
    pub fn new(io: &ModalScreenIo<'_, '_>, preferences: &LeaderboardPreferences) -> Self {
        let resources = io.resources;
        let working = preferences.clone();
        let dirty = false;
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

        let close = OkCancelScreen::new(ID_OK, ID_CANCEL);
        close.add_buttons(&mut frame, resources);

        let input = ModalInputState::for_screen(io.window, io.renderer);
        let tooltip = TooltipState::new();

        Self {
            working,
            dirty,
            frame,
            input,
            tooltip,
            close,
        }
    }

    /// Poll and draw exactly one frame; the caller owns pacing. The frame that
    /// closes the screen is still drawn and paced: `Some(())` is only reported
    /// on the following tick, before polling.
    pub fn tick(&mut self, io: &mut ModalScreenIo<'_, '_>) -> Option<()> {
        if self.close.done {
            return Some(());
        }
        let screen = ScreenFrame::begin(io, &mut self.input);
        self.close.poll_keys(&screen);
        let (_, activated) = ScreenFrame::dispatch(&mut self.input, &mut self.frame);
        if let Some(id) = activated {
            self.activate(id);
        }

        let transform = screen.transform;
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        self.close
            .draw_chrome(&screen, renderer, resources, "Leaderboards", 490);
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
        self.close
            .draw_buttons(renderer, resources, transform, &self.frame);
        screen.finish(io, &self.input);

        None
    }

    fn activate(&mut self, id: u32) {
        if !self.close.activate(id) && matches!(id, ID_SHOW_MISSION_END_BOARDS | ID_ALWAYS_SUBMIT) {
            toggle(&mut self.working, id);
            self.dirty = true;
        }
    }

    /// Publish staged changes only after acceptance and an actual difference.
    pub fn commit(self, target: &mut LeaderboardPreferences) -> bool {
        if self.close.accepted && self.dirty && self.working != *target {
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
            close: OkCancelScreen::new(ID_OK, ID_CANCEL),
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
            assert!(state.close.done);
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
