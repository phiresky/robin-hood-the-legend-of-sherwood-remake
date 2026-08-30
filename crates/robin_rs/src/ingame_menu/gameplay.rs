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

use super::ModalScreenOutcome;
use super::layout::{
    MenuTransform, TooltipState, align_bottom_right, dim_screen, draw_screen_background,
    enter_modal_gpu_phase, render_text_virt_font,
};
use super::resources::{IngameMenuResources, MT_BTN_CANCEL, MT_BTN_OK};
use super::widget_bridge::{self, ModalCursor, ModalInputState};

const ID_OPT_BASE: u32 = 200;
const ID_OK: u32 = 300;
const ID_CANCEL: u32 = 301;
pub(crate) const SHERWOOD_TRADING_OPTION_INDEX: usize = 16;
pub(crate) const AUTOSAVE_OPTION_INDEX: usize = 17;
pub(crate) const DETAILED_SAVE_METADATA_OPTION_INDEX: usize = 33;
pub(crate) const TIMED_MISSIONS_OPTION_INDEX: usize = 34;
pub(crate) const DYNAMIC_AMBIENCE_OPTION_INDEX: usize = 35;
pub(crate) const DIPLOMACY_OPTION_INDEX: usize = 36;
pub(crate) const NPC_FACTION_WARS_OPTION_INDEX: usize = 37;

/// Toggle rows shown on the screen, in display order.
pub(crate) const OPTION_LABELS: &[&str] = &[
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
    "Sherwood Item Trading",
    "Rotating Autosaves",
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
    "Detailed Save Metadata",
    "Authored Mission Timers",
    "Dynamic Ambience Gameplay",
    "Mission Diplomacy",
    "NPC Faction Wars",
];

const OPTION_TOOLTIPS: &[&str] = &[
    "Use the intended Hard reaction-time multiplier.",
    "Allow high-level commands for actors authored with the tactical command interface.",
    "Allow a hero with Tie to release a tied NPC.",
    "Show live item-production forecasts in Sherwood.",
    "Allow heroes with shipped cape art to put their cloaks back on.",
    "Cycle the campaign-map presentation.",
    "Count hostile deaths caused by other NPCs against Clean Hands.",
    "Show detailed sword and bow experience progress.",
    "Show the current mission speedrun clock.",
    "Show live Clean Hands achievement progress.",
    "Show live Ghost achievement progress.",
    "Show live Pile-o-Bones achievement progress.",
    "Show live All Enemies Stashed achievement progress.",
    "Show achievement badges in campaign presentations.",
    "Include achievement details in mission debriefs.",
    "Enable one-finger camera panning, anchored pinch zoom, and touch inertia.",
    "Allow the host to sell Sherwood production inventory for campaign ransom.",
    "Keep rotating campaign and mission autosaves according to the autosave policy.",
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
    "Show mission and player provenance, relative age, and expanded save details.",
    "Enforce time limits authored by Rust JSON missions.",
    "Advance authored day, night, and fog gameplay schedules.",
    "Enable mission-authored and runtime faction relationships.",
    "Allow hostile non-player factions to perceive and fight one another.",
];

pub(crate) fn option_tooltip(index: usize) -> &'static str {
    OPTION_TOOLTIPS
        .get(index)
        .copied()
        .unwrap_or_else(|| panic!("gameplay option tooltip index {index} is out of range"))
}

/// Display the gameplay sub-screen.  Returns `true` when the player
/// accepted changed settings.
pub async fn show_gameplay(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<ModalCursor<'_>>,
    config: &mut GameplayConfig,
    sherwood_trading_editable: bool,
) -> bool {
    let mut state = GameplayScreenState::new(
        event_pump,
        renderer,
        resources,
        config,
        sherwood_trading_editable,
    );
    loop {
        if let Some(outcome) = state.tick(event_pump, renderer, resources, cursor.as_ref()) {
            if let ModalScreenOutcome::Accepted(next) = outcome {
                let changed = next != *config;
                *config = next;
                return changed;
            }
            return false;
        }
        crate::window::sleep_ms(16).await;
    }
}

/// Owned, one-frame state for the gameplay settings page.
pub struct GameplayScreenState {
    working: GameplayConfig,
    original: GameplayConfig,
    frame: FrameWnd,
    input_state: ModalInputState,
    tooltip: TooltipState,
    transform: MenuTransform,
    sherwood_trading_editable: bool,
}

impl GameplayScreenState {
    pub fn new(
        event_pump: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        config: &GameplayConfig,
        sherwood_trading_editable: bool,
    ) -> Self {
        let sw = renderer.screen_width() as i32;
        let sh = renderer.screen_height() as i32;
        let transform = MenuTransform::centered(sw, sh);

        let working = *config;

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
            frame.add_widget_absolute(widget_bridge::make_button_enabled(
                ID_OPT_BASE + i as u32,
                &mb.label,
                i != SHERWOOD_TRADING_OPTION_INDEX || sherwood_trading_editable,
                mb.x,
                mb.y,
                mb.w,
                mb.h,
            ));
            frame
                .widget_mut(ID_OPT_BASE + i as u32)
                .expect("new gameplay option widget")
                .base_mut()
                .set_tooltip_text(option_tooltip(i));
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

        let mut input_state = ModalInputState::new();
        input_state.seed_mouse_from_window(event_pump, transform);

        Self {
            working,
            original: *config,
            frame,
            input_state,
            tooltip: TooltipState::new(),
            transform,
            sherwood_trading_editable,
        }
    }

    pub fn tick(
        &mut self,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) -> Option<ModalScreenOutcome<GameplayConfig>> {
        let mut outcome = None;
        renderer.sync_window_size(event_pump);
        self.transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        for event in event_pump.poll_events() {
            self.input_state.update_from_event(&event, self.transform);
            match event {
                GameEvent::Quit => outcome = Some(ModalScreenOutcome::ExitRequested),
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } => {
                    outcome = Some(ModalScreenOutcome::Accepted(self.working));
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => outcome = Some(ModalScreenOutcome::Cancelled),
                _ => {}
            }
        }

        let widget_input = self.input_state.as_widget_input();
        let events = self.frame.process_input(&widget_input);
        self.input_state.end_frame();

        if let Some(id) = widget_bridge::find_activated(&events) {
            match id {
                ID_OK => {
                    outcome = Some(ModalScreenOutcome::Accepted(self.working));
                }
                ID_CANCEL => outcome = Some(ModalScreenOutcome::Cancelled),
                id if (ID_OPT_BASE..ID_OPT_BASE + OPTION_LABELS.len() as u32).contains(&id) => {
                    let index = (id - ID_OPT_BASE) as usize;
                    if index != SHERWOOD_TRADING_OPTION_INDEX || self.sherwood_trading_editable {
                        apply_option_toggle(&mut self.working, index);
                    }
                }
                _ => {}
            }
        }

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);

        if let Some(bg) = resources.menu_bg[0] {
            draw_screen_background(renderer, &bg);
        }

        if let Some(font) = resources.title_font_any() {
            let tw = font.text_width("Gameplay");
            render_text_virt_font(
                renderer,
                font,
                self.transform,
                "Gameplay",
                (490 - tw) / 2,
                20,
            );
        }
        if let Some(font) = resources.label_font_any() {
            render_text_virt_font(renderer, font, self.transform, "Gameplay Tweaks", 30, 80);
        }

        for i in 0..OPTION_LABELS.len() as u32 {
            if let Some(w) = self.frame.widget(ID_OPT_BASE + i) {
                widget_bridge::draw_widget_radio(
                    renderer,
                    resources,
                    self.transform,
                    w,
                    is_option_selected(&self.working, i as usize),
                );
            }
        }
        if let Some(font) = resources.label_font_any() {
            render_text_virt_font(
                renderer,
                font,
                self.transform,
                self.working.campaign_presentation.label(),
                30,
                335,
            );
        }

        if let Some(w) = self.frame.widget(ID_OK) {
            widget_bridge::draw_widget_button(renderer, resources, self.transform, w, false);
        }
        if let Some(w) = self.frame.widget(ID_CANCEL) {
            widget_bridge::draw_widget_button(renderer, resources, self.transform, w, false);
        }

        let mouse_point = robin_engine::coordinates::ScreenPoint::new(
            self.input_state.virt_x,
            self.input_state.virt_y,
        );
        self.tooltip.update(&self.frame, mouse_point);
        if let Some(font) = resources.popup_font_any() {
            self.tooltip
                .draw(renderer, font, self.transform, &self.frame, mouse_point);
        }

        if let Some(c) = cursor {
            c.draw(renderer, self.transform, &self.input_state);
        }

        renderer.present();
        outcome
    }

    pub fn changed(&self) -> bool {
        self.working != self.original
    }
}

pub(crate) fn apply_option_toggle(config: &mut GameplayConfig, idx: usize) {
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
        SHERWOOD_TRADING_OPTION_INDEX => config.sherwood_trading = !config.sherwood_trading,
        AUTOSAVE_OPTION_INDEX => config.autosave_enabled = !config.autosave_enabled,
        18 => {
            config.item_gameplay.apple_combat_interrupt =
                !config.item_gameplay.apple_combat_interrupt
        }
        19 => {
            config.item_gameplay.wasp_reliable_acquisition =
                !config.item_gameplay.wasp_reliable_acquisition
        }
        20 => {
            config.item_gameplay.stone_ground_distraction =
                !config.item_gameplay.stone_ground_distraction
        }
        21 => config.item_gameplay.stone_longer_range = !config.item_gameplay.stone_longer_range,
        22 => {
            config.item_gameplay.net_selective_immunity =
                !config.item_gameplay.net_selective_immunity
        }
        23 => {
            config.item_gameplay.ale_reliable_distraction =
                !config.item_gameplay.ale_reliable_distraction
        }
        24 => config.noise_distraction_feedback = !config.noise_distraction_feedback,
        25 => config.item_previews.apple_effect = !config.item_previews.apple_effect,
        26 => config.item_previews.stone_direct_effect = !config.item_previews.stone_direct_effect,
        27 => {
            config.item_previews.stone_distraction_area =
                !config.item_previews.stone_distraction_area
        }
        28 => config.item_previews.net_capture_area = !config.item_previews.net_capture_area,
        29 => {
            config.item_previews.net_crumple_prediction =
                !config.item_previews.net_crumple_prediction
        }
        30 => config.item_previews.ale_effect = !config.item_previews.ale_effect,
        31 => config.item_previews.purse_effect = !config.item_previews.purse_effect,
        32 => config.item_previews.wasp_area = !config.item_previews.wasp_area,
        DETAILED_SAVE_METADATA_OPTION_INDEX => {
            config.detailed_save_metadata = !config.detailed_save_metadata
        }
        TIMED_MISSIONS_OPTION_INDEX => config.enable_timed_missions = !config.enable_timed_missions,
        DYNAMIC_AMBIENCE_OPTION_INDEX => {
            config.enable_dynamic_ambience = !config.enable_dynamic_ambience
        }
        DIPLOMACY_OPTION_INDEX => config.diplomacy = !config.diplomacy,
        NPC_FACTION_WARS_OPTION_INDEX => config.npc_faction_wars = !config.npc_faction_wars,
        _ => {}
    }
}

pub(crate) fn is_option_selected(config: &GameplayConfig, idx: usize) -> bool {
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
        SHERWOOD_TRADING_OPTION_INDEX => config.sherwood_trading,
        AUTOSAVE_OPTION_INDEX => config.autosave_enabled,
        18 => config.item_gameplay.apple_combat_interrupt,
        19 => config.item_gameplay.wasp_reliable_acquisition,
        20 => config.item_gameplay.stone_ground_distraction,
        21 => config.item_gameplay.stone_longer_range,
        22 => config.item_gameplay.net_selective_immunity,
        23 => config.item_gameplay.ale_reliable_distraction,
        24 => config.noise_distraction_feedback,
        25 => config.item_previews.apple_effect,
        26 => config.item_previews.stone_direct_effect,
        27 => config.item_previews.stone_distraction_area,
        28 => config.item_previews.net_capture_area,
        29 => config.item_previews.net_crumple_prediction,
        30 => config.item_previews.ale_effect,
        31 => config.item_previews.purse_effect,
        32 => config.item_previews.wasp_area,
        DETAILED_SAVE_METADATA_OPTION_INDEX => config.detailed_save_metadata,
        TIMED_MISSIONS_OPTION_INDEX => config.enable_timed_missions,
        DYNAMIC_AMBIENCE_OPTION_INDEX => config.enable_dynamic_ambience,
        DIPLOMACY_OPTION_INDEX => config.diplomacy,
        NPC_FACTION_WARS_OPTION_INDEX => config.npc_faction_wars,
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
                "Sherwood Item Trading",
                "Rotating Autosaves",
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
                "Detailed Save Metadata",
                "Authored Mission Timers",
                "Dynamic Ambience Gameplay",
                "Mission Diplomacy",
                "NPC Faction Wars",
            ]
        );
        assert_eq!(OPTION_LABELS.len(), OPTION_TOOLTIPS.len());

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
        assert!(is_option_selected(&config, SHERWOOD_TRADING_OPTION_INDEX));
        assert!(is_option_selected(&config, AUTOSAVE_OPTION_INDEX));
        assert!(is_option_selected(&config, 32));
        assert!(is_option_selected(
            &config,
            DETAILED_SAVE_METADATA_OPTION_INDEX
        ));
        assert!(is_option_selected(&config, TIMED_MISSIONS_OPTION_INDEX));
        assert!(is_option_selected(&config, DYNAMIC_AMBIENCE_OPTION_INDEX));
        assert!(is_option_selected(&config, DIPLOMACY_OPTION_INDEX));
        assert!(is_option_selected(&config, NPC_FACTION_WARS_OPTION_INDEX));

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

        let autosave_enabled = config.autosave_enabled;
        apply_option_toggle(&mut config, 21);
        assert!(!config.item_gameplay.stone_longer_range);
        assert!(config.item_gameplay.net_selective_immunity);
        apply_option_toggle(&mut config, 22);
        assert!(!config.item_gameplay.stone_longer_range);
        assert!(!config.item_gameplay.net_selective_immunity);
        assert!(config.item_gameplay.ale_reliable_distraction);
        assert_eq!(config.autosave_enabled, autosave_enabled);
        apply_option_toggle(&mut config, SHERWOOD_TRADING_OPTION_INDEX);
        assert!(!config.sherwood_trading);

        let autosave_enabled = config.autosave_enabled;
        apply_option_toggle(&mut config, DETAILED_SAVE_METADATA_OPTION_INDEX);
        assert!(!config.detailed_save_metadata);
        assert_eq!(config.autosave_enabled, autosave_enabled);
    }

    #[test]
    fn autosave_has_an_independent_gameplay_toggle() {
        let mut config = GameplayConfig::default();
        let before = config;
        assert_eq!(OPTION_LABELS[AUTOSAVE_OPTION_INDEX], "Rotating Autosaves");
        assert!(is_option_selected(&config, AUTOSAVE_OPTION_INDEX));
        apply_option_toggle(&mut config, AUTOSAVE_OPTION_INDEX);
        assert!(!is_option_selected(&config, AUTOSAVE_OPTION_INDEX));
        assert_eq!(
            config.fix_hard_reaction_times,
            before.fix_hard_reaction_times
        );
        assert_eq!(config.control_tactical_units, before.control_tactical_units);
        assert_eq!(config.enable_unbinding, before.enable_unbinding);
        assert_eq!(
            config.show_production_forecast,
            before.show_production_forecast
        );
        assert_eq!(config.reusable_cloaks, before.reusable_cloaks);
        assert_eq!(config.campaign_presentation, before.campaign_presentation);
        assert_eq!(config.touch_camera_gestures, before.touch_camera_gestures);
        assert_eq!(config.item_gameplay, before.item_gameplay);
        assert_eq!(config.item_previews, before.item_previews);
        assert_eq!(
            config.noise_distraction_feedback,
            before.noise_distraction_feedback
        );
        assert_eq!(config.diplomacy, before.diplomacy);
        assert_eq!(config.npc_faction_wars, before.npc_faction_wars);
    }
}
