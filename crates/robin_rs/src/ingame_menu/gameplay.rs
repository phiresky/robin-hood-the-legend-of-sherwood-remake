//! Gameplay settings sub-screen — toggles for optional gameplay tweaks,
//! plus OK / Cancel.
//!
//! These settings are Rust-port extensions (see
//! [`robin_engine::gameplay_config::GameplayConfig`]); the original game
//! has no equivalent screen, so all labels are literals rather than
//! string-table lookups.
//!
//! Toggle buttons and OK/Cancel are driven by the [`crate::widget`]
//! system via the [`super::widget_bridge`].

use crate::gfx_types::GameEvent;
use crate::gfx_types::Keycode;
use crate::renderer::Renderer;
use crate::widget::FrameWnd;
use robin_engine::gameplay_config::GameplayConfig;

use super::layout::{
    MenuTransform, align_bottom_right, dim_screen, draw_screen_background, enter_modal_gpu_phase,
    render_text_virt,
};
use super::resources::{IngameMenuResources, MT_BTN_CANCEL, MT_BTN_OK};
use super::widget_bridge::{self, ModalCursor, ModalInputState};

const ID_OPT_BASE: u32 = 200;
const ID_OK: u32 = 300;
const ID_CANCEL: u32 = 301;

/// Toggle rows shown on the screen, in display order.
const OPTION_LABELS: &[&str] = &[
    "Fix Hard Reaction Times",
    "Control Tactical Units",
    "Allow Untying NPCs",
    "Sherwood Production Forecast",
    "Reusable Cloaks",
    "Campaign Presentation",
    "NPC Kills Break Clean Hands",
    "Detailed Sword/Bow XP",
    "Speedrun Clock",
    "Clean Hands Tracker",
    "Ghost Tracker",
    "Pile-o-Bones Tracker",
    "All Enemies Stashed Tracker",
    "Campaign Achievement Badges",
    "Achievement Debrief Details",
    "Touch Camera Gestures",
];

/// Display the gameplay sub-screen.  Returns `true` when the player
/// accepted changed settings.
pub async fn show_gameplay(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<ModalCursor<'_>>,
    config: &mut GameplayConfig,
) -> bool {
    let sw = renderer.screen_width() as i32;
    let sh = renderer.screen_height() as i32;
    let transform = MenuTransform::centered(sw, sh);

    let mut working = *config;
    let mut dirty = false;

    // ── OK / Cancel (bottom-right) ─────────────────────────────────
    let (btn_w, btn_h) = resources.button_dimensions();
    let ok_label = resources.menu_text.get(MT_BTN_OK);
    let cancel_label = resources.menu_text.get(MT_BTN_CANCEL);
    let bottom_labels: &[(&str, bool)] = &[(&ok_label, true), (&cancel_label, true)];
    let bottom = align_bottom_right(bottom_labels, btn_w, btn_h);

    // ── Option toggle buttons stacked from (30,100) ───────────────
    let (field_w, field_h) = resources.input_field_dimensions();
    let rows_per_column = OPTION_LABELS.len().div_ceil(2);
    let opt_layout: Vec<super::layout::MenuButton> = OPTION_LABELS
        .iter()
        .enumerate()
        .map(|(i, label)| super::layout::MenuButton {
            label: label.to_string(),
            enabled: true,
            x: if i < rows_per_column { 30 } else { 320 },
            y: 100
                + i32::try_from(i % rows_per_column).expect("gameplay option row fits i32")
                    * (field_h + 2),
            w: field_w,
            h: field_h,
        })
        .collect();

    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;

    for (i, mb) in opt_layout.iter().enumerate() {
        frame.add_widget_absolute(widget_bridge::make_button(
            ID_OPT_BASE + i as u32,
            &mb.label,
            mb.x,
            mb.y,
            mb.w,
            mb.h,
        ));
    }
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

    let title = "Gameplay";

    let mut done = false;
    let mut accepted = false;
    let mut input_state = ModalInputState::new();
    input_state.seed_mouse_from_window(event_pump, transform);

    while !done {
        let (events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit => done = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } => {
                    accepted = true;
                    done = true;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => done = true,
                _ => {}
            }
        }

        let widget_input = input_state.as_widget_input();
        let events = frame.process_input(&widget_input);
        input_state.end_frame();

        if let Some(id) = widget_bridge::find_activated(&events) {
            match id {
                ID_OK => {
                    accepted = true;
                    done = true;
                }
                ID_CANCEL => done = true,
                id if (ID_OPT_BASE..ID_OPT_BASE + OPTION_LABELS.len() as u32).contains(&id) => {
                    apply_option_toggle(&mut working, (id - ID_OPT_BASE) as usize);
                    dirty = true;
                }
                _ => {}
            }
        }

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);

        if let Some(bg) = resources.menu_bg[0] {
            draw_screen_background(renderer, &bg);
        }

        if let Some(font) = resources.title_font() {
            let tw = font.text_width(title);
            render_text_virt(renderer, font, transform, title, (490 - tw) / 2, 20);
        }
        if let Some(font) = resources.label_font() {
            render_text_virt(renderer, font, transform, "Gameplay Tweaks", 30, 80);
        }

        for i in 0..OPTION_LABELS.len() as u32 {
            if let Some(w) = frame.widget(ID_OPT_BASE + i) {
                widget_bridge::draw_widget_radio(
                    renderer,
                    resources,
                    transform,
                    w,
                    is_option_selected(&working, i as usize),
                );
            }
        }
        if let Some(font) = resources.label_font() {
            render_text_virt(
                renderer,
                font,
                transform,
                working.campaign_presentation.label(),
                30,
                335,
            );
        }

        if let Some(w) = frame.widget(ID_OK) {
            widget_bridge::draw_widget_button(renderer, resources, transform, w, false);
        }
        if let Some(w) = frame.widget(ID_CANCEL) {
            widget_bridge::draw_widget_button(renderer, resources, transform, w, false);
        }

        if let Some(c) = &cursor {
            c.draw(renderer, transform, &input_state);
        }

        renderer.present();
        crate::window::sleep_ui_frame().await;
    }

    if accepted && dirty && working != *config {
        *config = working;
        true
    } else {
        false
    }
}

fn apply_option_toggle(config: &mut GameplayConfig, idx: usize) {
    match idx {
        0 => config.fix_hard_reaction_times = !config.fix_hard_reaction_times,
        1 => config.control_tactical_units = !config.control_tactical_units,
        2 => config.enable_unbinding = !config.enable_unbinding,
        3 => config.show_production_forecast = !config.show_production_forecast,
        4 => config.reusable_cloaks = !config.reusable_cloaks,
        5 => config.campaign_presentation = config.campaign_presentation.next(),
        6 => config.clean_hands_npc_kills_invalidate = !config.clean_hands_npc_kills_invalidate,
        7 => config.show_detailed_xp = !config.show_detailed_xp,
        8 => config.show_speedrun_tracker = !config.show_speedrun_tracker,
        9 => config.show_clean_hands_tracker = !config.show_clean_hands_tracker,
        10 => config.show_ghost_tracker = !config.show_ghost_tracker,
        11 => config.show_pile_o_bones_tracker = !config.show_pile_o_bones_tracker,
        12 => {
            config.show_all_enemies_one_building_tracker =
                !config.show_all_enemies_one_building_tracker
        }
        13 => config.show_achievement_badges = !config.show_achievement_badges,
        14 => config.show_achievement_debrief = !config.show_achievement_debrief,
        15 => config.touch_camera_gestures = !config.touch_camera_gestures,
        _ => {}
    }
}

fn is_option_selected(config: &GameplayConfig, idx: usize) -> bool {
    match idx {
        0 => config.fix_hard_reaction_times,
        1 => config.control_tactical_units,
        2 => config.enable_unbinding,
        3 => config.show_production_forecast,
        4 => config.reusable_cloaks,
        5 => {
            config.campaign_presentation
                != robin_engine::gameplay_config::CampaignPresentationMode::ClassicMap
        }
        6 => config.clean_hands_npc_kills_invalidate,
        7 => config.show_detailed_xp,
        8 => config.show_speedrun_tracker,
        9 => config.show_clean_hands_tracker,
        10 => config.show_ghost_tracker,
        11 => config.show_pile_o_bones_tracker,
        12 => config.show_all_enemies_one_building_tracker,
        13 => config.show_achievement_badges,
        14 => config.show_achievement_debrief,
        15 => config.touch_camera_gestures,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gameplay_rows_preserve_independent_setting_mappings() {
        assert_eq!(
            OPTION_LABELS,
            [
                "Fix Hard Reaction Times",
                "Control Tactical Units",
                "Allow Untying NPCs",
                "Sherwood Production Forecast",
                "Reusable Cloaks",
                "Campaign Presentation",
                "NPC Kills Break Clean Hands",
                "Detailed Sword/Bow XP",
                "Speedrun Clock",
                "Clean Hands Tracker",
                "Ghost Tracker",
                "Pile-o-Bones Tracker",
                "All Enemies Stashed Tracker",
                "Campaign Achievement Badges",
                "Achievement Debrief Details",
                "Touch Camera Gestures",
            ]
        );

        let mut config = GameplayConfig::default();
        assert!(!is_option_selected(&config, 1));
        assert!(is_option_selected(&config, 2));
        assert!(is_option_selected(&config, 3));
        assert!(is_option_selected(&config, 4));
        assert!(is_option_selected(&config, 5));
        assert!(!is_option_selected(&config, 6));
        assert!(is_option_selected(&config, 13));
        assert!(is_option_selected(&config, 14));
        assert!(is_option_selected(&config, 15));

        apply_option_toggle(&mut config, 1);
        assert!(config.control_tactical_units);
        assert!(config.enable_unbinding);
        assert!(config.show_production_forecast);
        assert!(config.reusable_cloaks);

        apply_option_toggle(&mut config, 3);
        assert!(config.control_tactical_units);
        assert!(config.enable_unbinding);
        assert!(!config.show_production_forecast);
        assert!(config.reusable_cloaks);

        apply_option_toggle(&mut config, 4);
        assert!(config.control_tactical_units);
        assert!(config.enable_unbinding);
        assert!(!config.show_production_forecast);
        assert!(!config.reusable_cloaks);

        apply_option_toggle(&mut config, 5);
        assert_eq!(
            config.campaign_presentation,
            robin_engine::gameplay_config::CampaignPresentationMode::SherwoodMuseum
        );

        let achievement_settings = (
            config.clean_hands_npc_kills_invalidate,
            config.show_detailed_xp,
            config.show_speedrun_tracker,
            config.show_clean_hands_tracker,
            config.show_ghost_tracker,
            config.show_pile_o_bones_tracker,
            config.show_all_enemies_one_building_tracker,
            config.show_achievement_badges,
            config.show_achievement_debrief,
        );
        apply_option_toggle(&mut config, 15);
        assert!(!config.touch_camera_gestures);
        assert!(config.control_tactical_units);
        assert!(config.enable_unbinding);
        assert!(!config.show_production_forecast);
        assert!(!config.reusable_cloaks);
        assert_eq!(
            achievement_settings,
            (
                config.clean_hands_npc_kills_invalidate,
                config.show_detailed_xp,
                config.show_speedrun_tracker,
                config.show_clean_hands_tracker,
                config.show_ghost_tracker,
                config.show_pile_o_bones_tracker,
                config.show_all_enemies_one_building_tracker,
                config.show_achievement_badges,
                config.show_achievement_debrief,
            )
        );
        assert!(!is_option_selected(&config, 15));
    }
}
