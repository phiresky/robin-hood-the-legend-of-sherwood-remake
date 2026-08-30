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
    MenuTransform, TooltipState, align_bottom_right, align_on_first_widget, dim_screen,
    draw_screen_background, enter_modal_gpu_phase, render_text_virt,
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
    "Apple Combat Interrupt",
    "Reliable Wasp Acquisition",
    "Stone Ground Distraction",
    "Longer Stone Range",
    "Selective Net Immunity",
    "Reliable Ale Distraction",
    "Stone Distraction Feedback",
    "Preview Apple Effect",
    "Preview Stone Direct Hit",
    "Preview Stone Noise Area",
    "Preview Net Capture Area",
    "Predict Net Crumpling",
    "Preview Ale Effect",
    "Preview Purse Effect",
    "Preview Wasp Area",
];

const OPTION_TOOLTIPS: &[&str] = &[
    "Use the intended Hard reaction-time multiplier.",
    "Allow high-level commands for actors authored with the tactical command interface.",
    "Allow a hero with Tie to release a tied NPC.",
    "Show live item-production forecasts in Sherwood.",
    "Allow heroes with shipped cape art to put their cloaks back on.",
    "Cycle the campaign-map presentation.",
    "Let direct apple hits interrupt active swordfights.",
    "Increase initial wasp acquisition from 50 to 75 world units.",
    "Allow ground-thrown stones to attract eligible hostiles within 240 world units.",
    "Use base range 300 for stones instead of the shipped 200.",
    "Skip VIPs, riders, and Stuteley while catching other people in the net circle.",
    "Let outdoor non-VIP soldiers with no beer interest accept ale at potency 20.",
    "Play the optional impact cue for a ground-thrown stone distraction.",
    "Explain apple daze, scent, and combat-interrupt eligibility while aiming.",
    "Explain stone direct-hit damage and concussion while aiming.",
    "Show the 240-unit ground-stone distraction area.",
    "Show the original 40-unit net capture area and friendly-capture behavior.",
    "Predict victim and terrain conditions that crumple a net.",
    "Explain visibility, outdoor, drunkenness, and beer-interest conditions.",
    "Explain purse value and money-interest conditions.",
    "Show wasp acquisition range and target eligibility.",
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
    let mut opt_layout: Vec<super::layout::MenuButton> = OPTION_LABELS
        .iter()
        .enumerate()
        .map(|(i, label)| super::layout::MenuButton {
            label: label.to_string(),
            enabled: true,
            x: 30,
            y: if i == 0 { 100 } else { 0 },
            w: field_w,
            h: field_h,
        })
        .collect();
    align_on_first_widget(&mut opt_layout, 2);

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
        frame
            .widget_mut(ID_OPT_BASE + i as u32)
            .expect("new gameplay option widget")
            .base_mut()
            .set_tooltip_text(OPTION_TOOLTIPS[i]);
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
    let mut tooltip = TooltipState::new();
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
                315,
                opt_layout[5].y + 7,
            );
        }

        let mouse_point =
            robin_engine::coordinates::ScreenPoint::new(input_state.virt_x, input_state.virt_y);
        tooltip.update(&frame, mouse_point);
        if let Some(font) = resources.popup_font() {
            tooltip.draw(renderer, font, transform, &frame, mouse_point);
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
        crate::window::sleep_ms(16).await;
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
        6 => {
            config.item_gameplay.apple_combat_interrupt =
                !config.item_gameplay.apple_combat_interrupt
        }
        7 => {
            config.item_gameplay.wasp_reliable_acquisition =
                !config.item_gameplay.wasp_reliable_acquisition
        }
        8 => {
            config.item_gameplay.stone_ground_distraction =
                !config.item_gameplay.stone_ground_distraction
        }
        9 => config.item_gameplay.stone_longer_range = !config.item_gameplay.stone_longer_range,
        10 => {
            config.item_gameplay.net_selective_immunity =
                !config.item_gameplay.net_selective_immunity
        }
        11 => {
            config.item_gameplay.ale_reliable_distraction =
                !config.item_gameplay.ale_reliable_distraction
        }
        12 => config.noise_distraction_feedback = !config.noise_distraction_feedback,
        13 => config.item_previews.apple_effect = !config.item_previews.apple_effect,
        14 => config.item_previews.stone_direct_effect = !config.item_previews.stone_direct_effect,
        15 => {
            config.item_previews.stone_distraction_area =
                !config.item_previews.stone_distraction_area
        }
        16 => config.item_previews.net_capture_area = !config.item_previews.net_capture_area,
        17 => {
            config.item_previews.net_crumple_prediction =
                !config.item_previews.net_crumple_prediction
        }
        18 => config.item_previews.ale_effect = !config.item_previews.ale_effect,
        19 => config.item_previews.purse_effect = !config.item_previews.purse_effect,
        20 => config.item_previews.wasp_area = !config.item_previews.wasp_area,
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
        6 => config.item_gameplay.apple_combat_interrupt,
        7 => config.item_gameplay.wasp_reliable_acquisition,
        8 => config.item_gameplay.stone_ground_distraction,
        9 => config.item_gameplay.stone_longer_range,
        10 => config.item_gameplay.net_selective_immunity,
        11 => config.item_gameplay.ale_reliable_distraction,
        12 => config.noise_distraction_feedback,
        13 => config.item_previews.apple_effect,
        14 => config.item_previews.stone_direct_effect,
        15 => config.item_previews.stone_distraction_area,
        16 => config.item_previews.net_capture_area,
        17 => config.item_previews.net_crumple_prediction,
        18 => config.item_previews.ale_effect,
        19 => config.item_previews.purse_effect,
        20 => config.item_previews.wasp_area,
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
                "Apple Combat Interrupt",
                "Reliable Wasp Acquisition",
                "Stone Ground Distraction",
                "Longer Stone Range",
                "Selective Net Immunity",
                "Reliable Ale Distraction",
                "Stone Distraction Feedback",
                "Preview Apple Effect",
                "Preview Stone Direct Hit",
                "Preview Stone Noise Area",
                "Preview Net Capture Area",
                "Predict Net Crumpling",
                "Preview Ale Effect",
                "Preview Purse Effect",
                "Preview Wasp Area",
            ]
        );
        assert_eq!(OPTION_LABELS.len(), OPTION_TOOLTIPS.len());

        let mut config = GameplayConfig::default();
        assert!(!is_option_selected(&config, 1));
        assert!(is_option_selected(&config, 2));
        assert!(is_option_selected(&config, 3));
        assert!(is_option_selected(&config, 4));
        assert!(is_option_selected(&config, 5));
        assert!(is_option_selected(&config, 6));
        assert!(is_option_selected(&config, 20));

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

        apply_option_toggle(&mut config, 9);
        assert!(!config.item_gameplay.stone_longer_range);
        assert!(config.item_gameplay.net_selective_immunity);
        apply_option_toggle(&mut config, 10);
        assert!(!config.item_gameplay.stone_longer_range);
        assert!(!config.item_gameplay.net_selective_immunity);
        assert!(config.item_gameplay.ale_reliable_distraction);
    }
}
