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
use super::widget_bridge::{self, ModalCursor, ModalInputState};

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
    let mut working = preferences.clone();
    let mut dirty = false;
    let transform = MenuTransform::centered(
        renderer.screen_width() as i32,
        renderer.screen_height() as i32,
    );
    let (field_w, field_h) = resources.input_field_dimensions();
    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;
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
    let mut tooltip = TooltipState::new();
    input.seed_mouse_from_window(event_pump, transform);
    let mut accepted = false;
    let mut done = false;
    while !done {
        let (events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            input.update_from_event(&event, transform);
            match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => done = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Return | Keycode::KpEnter,
                    ..
                } => {
                    accepted = true;
                    done = true;
                }
                _ => {}
            }
        }
        let events = frame.process_input(&input.as_widget_input());
        input.end_frame();
        if let Some(id) = widget_bridge::find_activated(&events) {
            match id {
                ID_SHOW_MISSION_END_BOARDS | ID_ALWAYS_SUBMIT => {
                    toggle(&mut working, id);
                    dirty = true;
                }
                ID_OK => {
                    accepted = true;
                    done = true;
                }
                ID_CANCEL => done = true,
                _ => {}
            }
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
            let widget = frame.widget(id).expect("leaderboard option widget exists");
            widget_bridge::draw_widget_radio(
                renderer,
                resources,
                transform,
                widget,
                selected(&working, id),
            );
        }
        let mouse = robin_engine::coordinates::ScreenPoint::new(input.virt_x, input.virt_y);
        tooltip.update(&frame, mouse);
        if let Some(font) = resources.popup_font_any() {
            tooltip.draw(renderer, font, transform, &frame, mouse);
        }
        for id in [ID_OK, ID_CANCEL] {
            if let Some(widget) = frame.widget(id) {
                widget_bridge::draw_widget_button(renderer, resources, transform, widget, false);
            }
        }
        if let Some(cursor) = &cursor {
            cursor.draw(renderer, transform, &input);
        }
        renderer.present();
        crate::window::sleep_ui_frame().await;
    }

    if accepted && dirty && working != *preferences {
        *preferences = working;
        true
    } else {
        false
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
