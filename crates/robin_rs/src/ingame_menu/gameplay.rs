//! Gameplay settings sub-screen — toggles for optional gameplay tweaks,
//! plus OK / Cancel.
//!
//! These settings are Rust-port extensions (see
//! [`robin_engine::gameplay_config::GameplayConfig`]); the original game
//! has no equivalent screen, so labels come from the port-localized catalogue
//! rather than the original string tables.
//!
//! Toggle buttons and OK/Cancel are driven by the [`crate::widget`]
//! system via the [`super::widget_bridge`].

use crate::gfx_types::GameEvent;
use crate::gfx_types::Keycode;
use crate::localization::PortTextKey;
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
const ID_PREVIOUS_PAGE: u32 = 302;
const ID_NEXT_PAGE: u32 = 303;
const ID_CONTENT: u32 = 304;
const STANDALONE_OPTIONS_PER_PAGE: usize = 12;
const OPTION_COLUMN_LEFT_X: i32 = 30;
const OPTION_COLUMN_RIGHT_X: i32 = 330;
const OPTION_ROW_START_Y: i32 = 100;
const OPTION_ROW_GAP: i32 = 6;
const OPTION_COLUMN_WIDTH_LIMIT: i32 = 280;
pub(crate) use crate::gameplay_settings::GameplaySetting;

impl GameplaySetting {
    pub(crate) const fn requires_host_authority(self) -> bool {
        matches!(
            self,
            Self::FixHardReactionTimes
                | Self::EnableUnbinding
                | Self::ReusableCloaks
                | Self::CleanHandsNpcKillsInvalidate
                | Self::SherwoodTrading
                | Self::AppleCombatInterrupt
                | Self::WaspReliableAcquisition
                | Self::StoneGroundDistraction
                | Self::StoneLongerRange
                | Self::NetSelectiveImmunity
                | Self::AleReliableDistraction
                | Self::NoiseDistractionFeedback
                | Self::EnableTimedMissions
                | Self::EnableDynamicAmbience
                | Self::Diplomacy
                | Self::NpcFactionWars
                | Self::MoreCombatGestures
                | Self::GestureQualityDamage
                | Self::FogOfWar
        )
    }
}

/// Locale snapshot shared by the standalone and cooperative options screens.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct LocalizedGameplayText {
    locale: Option<String>,
}
impl LocalizedGameplayText {
    pub(crate) fn from_application_context(context: &crate::host::ApplicationContext) -> Self {
        Self {
            locale: context
                .active_locale()
                .expect("Gameplay screen requires localized text"),
        }
    }
    #[cfg(test)]
    pub(crate) fn for_locale(locale: &str) -> Self {
        Self {
            locale: Some(locale.to_owned()),
        }
    }
    pub(crate) fn option_label(&self, index: usize) -> &'static str {
        let setting = GameplaySetting::from_index(index).expect("valid gameplay row");
        crate::localization::port_text(self.locale.as_deref(), setting.label_key())
    }
    pub(crate) fn option_tooltip(&self, index: usize) -> &'static str {
        let setting = GameplaySetting::from_index(index).expect("valid gameplay row");
        crate::localization::port_text(self.locale.as_deref(), setting.tooltip_key())
    }
    pub(crate) fn campaign_presentation(
        &self,
        mode: robin_engine::gameplay_config::CampaignPresentationMode,
    ) -> &'static str {
        use robin_engine::gameplay_config::CampaignPresentationMode::*;
        let key = match mode {
            ClassicMap => PortTextKey::CampaignClassicMap,
            ProgressTree => PortTextKey::CampaignProgressTree,
            SherwoodMuseum => PortTextKey::CampaignSherwoodMuseum,
        };
        crate::localization::port_text(self.locale.as_deref(), key)
    }
    pub(crate) fn manage_content(&self) -> &'static str {
        crate::localization::port_text(self.locale.as_deref(), PortTextKey::SpellforgeManageContent)
    }
}
#[cfg(test)]
fn english_labels() -> Vec<&'static str> {
    let text = LocalizedGameplayText::for_locale("en-US");
    GameplaySetting::ALL
        .iter()
        .map(|setting| text.option_label(setting.index()))
        .collect()
}

/// Elide a localized label at grapheme boundaries. The full text remains in
/// the row help/tooltip, while button text is guaranteed to stay inside its
/// fixed 640x480 virtual-space hit box.
pub(crate) fn elide_to_width_by(
    text: &str,
    max_width: i32,
    measure: impl Fn(&str) -> i32,
) -> String {
    super::layout::elide_text_to_width_by(text, max_width, false, measure)
}

pub(crate) fn fit_button_label(
    resources: &IngameMenuResources,
    label: &str,
    enabled: bool,
    width: i32,
) -> String {
    resources.menu_button_font_any(enabled).map_or_else(
        || label.to_owned(),
        |font| {
            elide_to_width_by(label, (width - 24).max(1), |candidate| {
                font.text_width(candidate)
            })
        },
    )
}

fn standalone_page_count() -> usize {
    GameplaySetting::ALL
        .len()
        .div_ceil(STANDALONE_OPTIONS_PER_PAGE)
        .max(1)
}

fn standalone_visible_option_range(page: usize) -> std::ops::Range<usize> {
    let page = page.min(standalone_page_count() - 1);
    let start = page * STANDALONE_OPTIONS_PER_PAGE;
    start..(start + STANDALONE_OPTIONS_PER_PAGE).min(GameplaySetting::ALL.len())
}

fn standalone_option_rect(
    visible_index: usize,
    visible_count: usize,
    field_w: i32,
    field_h: i32,
) -> (i32, i32, i32, i32) {
    let rows_per_column = visible_count.div_ceil(2).max(1);
    let x = if visible_index < rows_per_column {
        OPTION_COLUMN_LEFT_X
    } else {
        OPTION_COLUMN_RIGHT_X
    };
    let y = OPTION_ROW_START_Y
        + i32::try_from(visible_index % rows_per_column).expect("gameplay option row fits i32")
            * (field_h + OPTION_ROW_GAP);
    (x, y, field_w, field_h)
}

fn build_standalone_frame(
    application_context: &crate::host::ApplicationContext,
    resources: &IngameMenuResources,
    page: usize,
    sherwood_trading_editable: bool,
) -> FrameWnd {
    let localized = LocalizedGameplayText::from_application_context(application_context);
    let (btn_w, btn_h) = resources.button_dimensions();
    let ok_label = resources.menu_text.get(MT_BTN_OK);
    let cancel_label = resources.menu_text.get(MT_BTN_CANCEL);
    let bottom_labels: &[(&str, bool)] = &[(&ok_label, true), (&cancel_label, true)];
    let bottom = align_bottom_right(bottom_labels, btn_w, btn_h);

    let visible = standalone_visible_option_range(page);
    let visible_count = visible.len();
    let field_w = OPTION_COLUMN_WIDTH_LIMIT;
    let field_h = btn_h;
    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;

    for (visible_index, option_index) in visible.enumerate() {
        let (x, y, field_w, field_h) =
            standalone_option_rect(visible_index, visible_count, field_w, field_h);
        let enabled =
            option_index != GameplaySetting::SherwoodTrading.index() || sherwood_trading_editable;
        let label = fit_button_label(
            resources,
            localized.option_label(option_index),
            enabled,
            field_w,
        );
        frame.add_widget_absolute(widget_bridge::make_button_enabled(
            ID_OPT_BASE + option_index as u32,
            &label,
            enabled,
            x,
            y,
            field_w,
            field_h,
        ));
        frame
            .widget_mut(ID_OPT_BASE + option_index as u32)
            .expect("new gameplay option widget")
            .base_mut()
            .set_tooltip_text(localized.option_tooltip(option_index));
    }

    frame.add_widget_absolute(widget_bridge::make_button_enabled(
        ID_PREVIOUS_PAGE,
        "Previous Page",
        page > 0,
        30,
        388,
        btn_w,
        btn_h,
    ));
    frame.add_widget_absolute(widget_bridge::make_button_enabled(
        ID_NEXT_PAGE,
        "Next Page",
        page + 1 < standalone_page_count(),
        30,
        388 + btn_h + OPTION_ROW_GAP,
        btn_w,
        btn_h,
    ));
    frame.add_widget_absolute(widget_bridge::make_button(
        ID_CONTENT,
        &fit_button_label(resources, localized.manage_content(), true, field_w),
        OPTION_COLUMN_RIGHT_X,
        52,
        field_w,
        field_h,
    ));
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
    frame
}

/// Display the gameplay sub-screen.  Returns `true` when the player
/// accepted changed settings.
pub async fn show_gameplay(
    application_context: &crate::host::ApplicationContext,
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    mut cursor: Option<ModalCursor<'_>>,
    config: &mut GameplayConfig,
    sherwood_trading_editable: bool,
) -> bool {
    let mut state = GameplayScreenState::new(
        application_context,
        event_pump,
        renderer,
        resources,
        config,
        sherwood_trading_editable,
    );
    loop {
        let outcome = state.tick(
            application_context,
            event_pump,
            renderer,
            resources,
            cursor.as_ref(),
        );
        if state.take_content_request() {
            super::spellforge_content::show_spellforge_content_settings(
                application_context,
                event_pump,
                renderer,
                resources,
                cursor.as_mut().map(|cursor| cursor.reborrow()),
            )
            .await;
            state.resume_after_content(event_pump, renderer);
            continue;
        }
        if let Some(outcome) = outcome {
            if let ModalScreenOutcome::Accepted(next) = outcome {
                let changed = next != *config;
                *config = next;
                return changed;
            }
            return false;
        }
        crate::window::sleep_ui_frame().await;
    }
}

/// Owned, one-frame state for the gameplay settings page.
pub struct GameplayScreenState {
    localized: LocalizedGameplayText,
    working: GameplayConfig,
    original: GameplayConfig,
    page: usize,
    frame: FrameWnd,
    input_state: ModalInputState,
    tooltip: TooltipState,
    transform: MenuTransform,
    sherwood_trading_editable: bool,
    content_requested: bool,
}

impl GameplayScreenState {
    pub fn new(
        application_context: &crate::host::ApplicationContext,
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

        let page = 0;
        let frame = build_standalone_frame(
            application_context,
            resources,
            page,
            sherwood_trading_editable,
        );

        let mut input_state = ModalInputState::new();
        input_state.seed_mouse_from_window(event_pump, transform);

        Self {
            localized: LocalizedGameplayText::from_application_context(application_context),
            working,
            original: *config,
            page,
            frame,
            input_state,
            tooltip: TooltipState::new(),
            transform,
            sherwood_trading_editable,
            content_requested: false,
        }
    }

    fn rebuild_page(
        &mut self,
        application_context: &crate::host::ApplicationContext,
        resources: &IngameMenuResources,
    ) {
        self.frame = build_standalone_frame(
            application_context,
            resources,
            self.page,
            self.sherwood_trading_editable,
        );
    }

    fn take_content_request(&mut self) -> bool {
        std::mem::take(&mut self.content_requested)
    }

    fn resume_after_content(
        &mut self,
        event_pump: &crate::window::GameWindow,
        renderer: &Renderer,
    ) {
        self.transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        self.input_state
            .seed_mouse_from_window(event_pump, self.transform);
    }

    pub fn tick(
        &mut self,
        application_context: &crate::host::ApplicationContext,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) -> Option<ModalScreenOutcome<GameplayConfig>> {
        let mut outcome = None;
        let (events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        self.transform = transform;
        for event in events {
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
                ID_PREVIOUS_PAGE if self.page > 0 => {
                    self.page -= 1;
                    self.rebuild_page(application_context, resources);
                }
                ID_NEXT_PAGE if self.page + 1 < standalone_page_count() => {
                    self.page += 1;
                    self.rebuild_page(application_context, resources);
                }
                ID_CONTENT => {
                    self.content_requested = true;
                }
                id if (ID_OPT_BASE..ID_OPT_BASE + GameplaySetting::ALL.len() as u32)
                    .contains(&id) =>
                {
                    let index = (id - ID_OPT_BASE) as usize;
                    if index != GameplaySetting::SherwoodTrading.index()
                        || self.sherwood_trading_editable
                    {
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

        for i in 0..GameplaySetting::ALL.len() as u32 {
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
        if self
            .frame
            .widget(ID_OPT_BASE + GameplaySetting::CampaignPresentation.index() as u32)
            .is_some()
            && let Some(font) = resources.label_font_any()
        {
            render_text_virt_font(
                renderer,
                font,
                self.transform,
                self.localized
                    .campaign_presentation(self.working.campaign_presentation),
                30,
                335,
            );
        }

        if let Some(widget) = self.frame.widget(ID_CONTENT) {
            widget_bridge::draw_widget_button(renderer, resources, self.transform, widget, false);
        }

        for id in [ID_PREVIOUS_PAGE, ID_NEXT_PAGE] {
            if let Some(w) = self.frame.widget(id) {
                widget_bridge::draw_widget_button(renderer, resources, self.transform, w, false);
            }
        }
        if let Some(font) = resources.label_font_any() {
            let page_label = format!("Page {} / {}", self.page + 1, standalone_page_count());
            render_text_virt_font(renderer, font, self.transform, &page_label, 30, 362);
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
    let Some(setting) = GameplaySetting::from_index(idx) else {
        return;
    };
    apply_setting(config, setting);
}

pub(crate) fn apply_setting(config: &mut GameplayConfig, setting: GameplaySetting) {
    use GameplaySetting as Setting;
    match setting {
        Setting::FixHardReactionTimes => {
            config.fix_hard_reaction_times = !config.fix_hard_reaction_times
        }
        Setting::ControlTacticalUnits => {
            config.control_tactical_units = !config.control_tactical_units
        }
        Setting::EnableUnbinding => config.enable_unbinding = !config.enable_unbinding,
        Setting::ShowProductionForecast => {
            config.show_production_forecast = !config.show_production_forecast
        }
        Setting::ReversibleBackgroundPatches => {
            config.reversible_background_patches = !config.reversible_background_patches
        }
        Setting::ReusableCloaks => config.reusable_cloaks = !config.reusable_cloaks,
        Setting::CampaignPresentation => {
            config.campaign_presentation = config.campaign_presentation.next()
        }
        Setting::CleanHandsNpcKillsInvalidate => {
            config.clean_hands_npc_kills_invalidate = !config.clean_hands_npc_kills_invalidate
        }
        Setting::ShowDetailedXp => config.show_detailed_xp = !config.show_detailed_xp,
        Setting::ShowSpeedrunTracker => {
            config.show_speedrun_tracker = !config.show_speedrun_tracker
        }
        Setting::ShowCleanHandsTracker => {
            config.show_clean_hands_tracker = !config.show_clean_hands_tracker
        }
        Setting::ShowGhostTracker => config.show_ghost_tracker = !config.show_ghost_tracker,
        Setting::ShowPileOfBonesTracker => {
            config.show_pile_o_bones_tracker = !config.show_pile_o_bones_tracker
        }
        Setting::ShowNewAchievementTrackers => {
            config.show_new_achievement_trackers = !config.show_new_achievement_trackers
        }
        Setting::ShowAchievementBadges => {
            config.show_achievement_badges = !config.show_achievement_badges
        }
        Setting::ShowAchievementDebrief => {
            config.show_achievement_debrief = !config.show_achievement_debrief
        }
        Setting::TouchCameraGestures => {
            config.touch_camera_gestures = !config.touch_camera_gestures
        }
        Setting::SherwoodTrading => config.sherwood_trading = !config.sherwood_trading,
        Setting::AutosaveEnabled => config.autosave_enabled = !config.autosave_enabled,
        Setting::AppleCombatInterrupt => {
            config.item_gameplay.apple_combat_interrupt =
                !config.item_gameplay.apple_combat_interrupt
        }
        Setting::WaspReliableAcquisition => {
            config.item_gameplay.wasp_reliable_acquisition =
                !config.item_gameplay.wasp_reliable_acquisition
        }
        Setting::StoneGroundDistraction => {
            config.item_gameplay.stone_ground_distraction =
                !config.item_gameplay.stone_ground_distraction
        }
        Setting::StoneLongerRange => {
            config.item_gameplay.stone_longer_range = !config.item_gameplay.stone_longer_range
        }
        Setting::NetSelectiveImmunity => {
            config.item_gameplay.net_selective_immunity =
                !config.item_gameplay.net_selective_immunity
        }
        Setting::AleReliableDistraction => {
            config.item_gameplay.ale_reliable_distraction =
                !config.item_gameplay.ale_reliable_distraction
        }
        Setting::NoiseDistractionFeedback => {
            config.noise_distraction_feedback = !config.noise_distraction_feedback
        }
        Setting::PreviewAppleEffect => {
            config.item_previews.apple_effect = !config.item_previews.apple_effect
        }
        Setting::PreviewStoneDirectEffect => {
            config.item_previews.stone_direct_effect = !config.item_previews.stone_direct_effect
        }
        Setting::PreviewStoneDistractionArea => {
            config.item_previews.stone_distraction_area =
                !config.item_previews.stone_distraction_area
        }
        Setting::PreviewNetCaptureArea => {
            config.item_previews.net_capture_area = !config.item_previews.net_capture_area
        }
        Setting::PreviewNetCrumplePrediction => {
            config.item_previews.net_crumple_prediction =
                !config.item_previews.net_crumple_prediction
        }
        Setting::PreviewAleEffect => {
            config.item_previews.ale_effect = !config.item_previews.ale_effect
        }
        Setting::PreviewPurseEffect => {
            config.item_previews.purse_effect = !config.item_previews.purse_effect
        }
        Setting::PreviewWaspArea => {
            config.item_previews.wasp_area = !config.item_previews.wasp_area
        }
        Setting::DetailedSaveMetadata => {
            config.detailed_save_metadata = !config.detailed_save_metadata
        }
        Setting::EnableTimedMissions => {
            config.enable_timed_missions = !config.enable_timed_missions
        }
        Setting::EnableDynamicAmbience => {
            config.enable_dynamic_ambience = !config.enable_dynamic_ambience
        }
        Setting::Diplomacy => config.diplomacy = !config.diplomacy,
        Setting::NpcFactionWars => config.npc_faction_wars = !config.npc_faction_wars,
        Setting::MoreCombatGestures => config.more_combat_gestures = !config.more_combat_gestures,
        Setting::GestureQualityDamage => {
            config.gesture_quality_damage = !config.gesture_quality_damage
        }
        Setting::ShowCombatGestureGuide => {
            config.show_combat_gesture_guide = !config.show_combat_gesture_guide
        }
        Setting::CombatGestureCoach => config.combat_gesture_coach = !config.combat_gesture_coach,
        Setting::PlanQuickActions => config.plan_quick_actions = !config.plan_quick_actions,
        Setting::FogOfWar => config.fog_of_war = !config.fog_of_war,
        Setting::EnableSpellforgeMissions => {
            config.enable_spellforge_missions = !config.enable_spellforge_missions
        }
    }
}

pub(crate) fn is_option_selected(config: &GameplayConfig, idx: usize) -> bool {
    GameplaySetting::from_index(idx).is_some_and(|setting| setting.is_selected(config))
}

impl GameplaySetting {
    pub(crate) fn is_selected(self, config: &GameplayConfig) -> bool {
        use GameplaySetting as Setting;
        match self {
            Setting::FixHardReactionTimes => config.fix_hard_reaction_times,
            Setting::ControlTacticalUnits => config.control_tactical_units,
            Setting::EnableUnbinding => config.enable_unbinding,
            Setting::ShowProductionForecast => config.show_production_forecast,
            Setting::ReversibleBackgroundPatches => config.reversible_background_patches,
            Setting::ReusableCloaks => config.reusable_cloaks,
            Setting::CampaignPresentation => {
                config.campaign_presentation
                    != robin_engine::gameplay_config::CampaignPresentationMode::ClassicMap
            }
            Setting::CleanHandsNpcKillsInvalidate => config.clean_hands_npc_kills_invalidate,
            Setting::ShowDetailedXp => config.show_detailed_xp,
            Setting::ShowSpeedrunTracker => config.show_speedrun_tracker,
            Setting::ShowCleanHandsTracker => config.show_clean_hands_tracker,
            Setting::ShowGhostTracker => config.show_ghost_tracker,
            Setting::ShowPileOfBonesTracker => config.show_pile_o_bones_tracker,
            Setting::ShowNewAchievementTrackers => config.show_new_achievement_trackers,
            Setting::ShowAchievementBadges => config.show_achievement_badges,
            Setting::ShowAchievementDebrief => config.show_achievement_debrief,
            Setting::TouchCameraGestures => config.touch_camera_gestures,
            Setting::SherwoodTrading => config.sherwood_trading,
            Setting::AutosaveEnabled => config.autosave_enabled,
            Setting::AppleCombatInterrupt => config.item_gameplay.apple_combat_interrupt,
            Setting::WaspReliableAcquisition => config.item_gameplay.wasp_reliable_acquisition,
            Setting::StoneGroundDistraction => config.item_gameplay.stone_ground_distraction,
            Setting::StoneLongerRange => config.item_gameplay.stone_longer_range,
            Setting::NetSelectiveImmunity => config.item_gameplay.net_selective_immunity,
            Setting::AleReliableDistraction => config.item_gameplay.ale_reliable_distraction,
            Setting::NoiseDistractionFeedback => config.noise_distraction_feedback,
            Setting::PreviewAppleEffect => config.item_previews.apple_effect,
            Setting::PreviewStoneDirectEffect => config.item_previews.stone_direct_effect,
            Setting::PreviewStoneDistractionArea => config.item_previews.stone_distraction_area,
            Setting::PreviewNetCaptureArea => config.item_previews.net_capture_area,
            Setting::PreviewNetCrumplePrediction => config.item_previews.net_crumple_prediction,
            Setting::PreviewAleEffect => config.item_previews.ale_effect,
            Setting::PreviewPurseEffect => config.item_previews.purse_effect,
            Setting::PreviewWaspArea => config.item_previews.wasp_area,
            Setting::DetailedSaveMetadata => config.detailed_save_metadata,
            Setting::EnableTimedMissions => config.enable_timed_missions,
            Setting::EnableDynamicAmbience => config.enable_dynamic_ambience,
            Setting::Diplomacy => config.diplomacy,
            Setting::NpcFactionWars => config.npc_faction_wars,
            Setting::MoreCombatGestures => config.more_combat_gestures,
            Setting::GestureQualityDamage => config.gesture_quality_damage,
            Setting::ShowCombatGestureGuide => config.show_combat_gesture_guide,
            Setting::CombatGestureCoach => config.combat_gesture_coach,
            Setting::PlanQuickActions => config.plan_quick_actions,
            Setting::FogOfWar => config.fog_of_war,
            Setting::EnableSpellforgeMissions => config.enable_spellforge_missions,
        }
    }
}

/// Whether this row mutates deterministic mission state shared by every peer.
///
/// Multiplayer clients may still inspect these rows, but only the host may
/// change them. Keep this classification beside the authoritative row mapping
/// so adding a new setting cannot accidentally make a simulation rule local.
#[cfg(test)]
pub(crate) fn option_requires_host_authority(idx: usize) -> bool {
    match GameplaySetting::ALL.get(idx) {
        Some(setting) => setting.requires_host_authority(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIPPING_LOCALES: &[&str] = &[
        "en-US", "de-DE", "und", "fr-FR", "it-IT", "pt-PT", "es-ES", "ru-RU", "ja-JP", "cs-CZ",
        "pl-PL", "pt-BR", "zh-TW", "ko-KR", "zh-CN", "th-TH",
    ];

    #[test]
    fn every_setting_has_keyed_text_and_german_uses_the_catalogue() {
        let en = LocalizedGameplayText::for_locale("en-US");
        let de = LocalizedGameplayText::for_locale("de-DE");
        let unknown = LocalizedGameplayText::for_locale("zz-ZZ");
        for (index, setting) in GameplaySetting::ALL.into_iter().enumerate() {
            assert_eq!(setting.index(), index);
            assert_eq!(GameplaySetting::from_index(index), Some(setting));
            assert!(!en.option_label(index).is_empty());
            assert!(!en.option_tooltip(index).is_empty());
            assert_ne!(en.option_label(index), de.option_label(index));
            assert_ne!(en.option_tooltip(index), de.option_tooltip(index));
            assert_eq!(unknown.option_label(index), en.option_label(index));
            assert_eq!(unknown.option_tooltip(index), en.option_tooltip(index));
            assert_eq!(
                serde_json::from_str::<GameplaySetting>(&serde_json::to_string(&setting).unwrap())
                    .unwrap(),
                setting
            );
        }
        assert!(GameplaySetting::from_index(GameplaySetting::ALL.len()).is_none());
        assert_eq!(
            de.campaign_presentation(
                robin_engine::gameplay_config::CampaignPresentationMode::ProgressTree
            ),
            "Fortschrittsbaum"
        );
    }

    #[test]
    fn standalone_pages_cover_every_gameplay_option_once() {
        assert_eq!(standalone_page_count(), 4);
        assert_eq!(standalone_visible_option_range(0), 0..12);
        assert_eq!(standalone_visible_option_range(1), 12..24);
        assert_eq!(standalone_visible_option_range(2), 24..36);
        assert_eq!(standalone_visible_option_range(3), 36..46);

        let covered: Vec<_> = (0..standalone_page_count())
            .flat_map(standalone_visible_option_range)
            .collect();
        assert_eq!(covered, (0..GameplaySetting::ALL.len()).collect::<Vec<_>>());
    }

    #[test]
    fn standalone_page_layout_stays_inside_virtual_screen_and_above_navigation() {
        for page in 0..standalone_page_count() {
            let count = standalone_visible_option_range(page).len();
            for visible_index in 0..count {
                let (x, y, width, height) =
                    standalone_option_rect(visible_index, count, OPTION_COLUMN_WIDTH_LIMIT, 34);
                assert!(x >= 0 && x + width <= 640);
                assert!(y >= OPTION_ROW_START_Y);
                assert!(y + height < 388, "row overlaps page navigation");
            }
        }
    }

    #[test]
    fn standalone_and_cooperative_coordinates_round_trip_at_widescreen_sizes() {
        for (screen_width, screen_height) in [(640, 480), (1024, 768), (1280, 720), (1920, 1080)] {
            let transform = MenuTransform::centered(screen_width, screen_height);
            for point in [
                (OPTION_COLUMN_LEFT_X, OPTION_ROW_START_Y),
                (OPTION_COLUMN_RIGHT_X, OPTION_ROW_START_Y),
                (320, 52),
                (640 - 1, 480 - 1),
            ] {
                let screen = transform.to_screen(point.0, point.1);
                assert_eq!(transform.from_screen(screen.0, screen.1), point);
            }
        }
    }

    #[test]
    fn localized_spellforge_rows_are_shared_by_both_gameplay_paths() {
        for locale in SHIPPING_LOCALES {
            let localized = LocalizedGameplayText::for_locale(locale);
            assert_eq!(
                localized.option_label(GameplaySetting::EnableSpellforgeMissions.index()),
                crate::localization::port_text(
                    Some(locale),
                    PortTextKey::SpellforgeGameplayAllowLabel,
                )
            );
            assert_eq!(
                localized.option_tooltip(GameplaySetting::EnableSpellforgeMissions.index()),
                crate::localization::port_text(
                    Some(locale),
                    PortTextKey::SpellforgeGameplayAllowTooltip,
                )
            );
            assert_eq!(
                localized.manage_content(),
                crate::localization::port_text(Some(locale), PortTextKey::SpellforgeManageContent)
            );
            assert!(
                !localized
                    .option_label(GameplaySetting::EnableSpellforgeMissions.index())
                    .is_empty()
            );
            assert!(
                !localized
                    .option_tooltip(GameplaySetting::EnableSpellforgeMissions.index())
                    .is_empty()
            );
            assert!(!localized.manage_content().is_empty());
        }
    }

    #[test]
    fn longest_localized_gameplay_labels_elide_on_grapheme_boundaries() {
        let longest = SHIPPING_LOCALES
            .iter()
            .copied()
            .flat_map(|locale| {
                let localized = LocalizedGameplayText::for_locale(locale);
                [
                    localized.option_label(GameplaySetting::EnableSpellforgeMissions.index()),
                    localized.manage_content(),
                ]
            })
            .max_by_key(|text| text.chars().count())
            .expect("shipping locale catalogue is non-empty");
        let fitted = elide_to_width_by(longest, 18, |candidate| candidate.chars().count() as i32);
        assert!(fitted.chars().count() <= 18);
        assert!(fitted.ends_with('…'));
        assert!(!fitted.contains('\u{fffd}'));
    }

    #[test]
    fn every_gameplay_row_has_a_live_setting_mapping() {
        let baseline = GameplayConfig::default();
        for (index, label) in english_labels().iter().enumerate() {
            let mut config = baseline;
            let selected_before = is_option_selected(&config, index);
            apply_option_toggle(&mut config, index);
            if index == 5 {
                assert_ne!(
                    config.campaign_presentation, baseline.campaign_presentation,
                    "gameplay row {index} ({}) did not change its setting",
                    label,
                );
            } else {
                assert_ne!(
                    is_option_selected(&config, index),
                    selected_before,
                    "gameplay row {index} ({}) did not change its setting",
                    label,
                );
            }
        }
    }

    #[test]
    fn multiplayer_authority_classification_covers_every_simulation_row() {
        let authoritative: Vec<_> = (0..GameplaySetting::ALL.len())
            .filter(|index| option_requires_host_authority(*index))
            .collect();
        assert_eq!(
            authoritative,
            [
                0, 2, 4, 6, 16, 18, 19, 20, 21, 22, 23, 24, 34, 35, 36, 37, 38, 39, 43,
            ]
        );

        for local in [
            1, 3, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 17, 25, 26, 27, 28, 29, 30, 31, 32, 33, 40,
            41, 42, 44, 45,
        ] {
            assert!(
                !option_requires_host_authority(local),
                "local/presentation row {local} was classified as host-owned",
            );
        }
    }

    #[test]
    fn gameplay_rows_preserve_independent_setting_mappings() {
        assert_eq!(
            english_labels(),
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
                "Additional Achievement Trackers",
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
                "More Combat Gestures",
                "Gesture Quality Damage",
                "Show Combat Gesture Guide",
                "Combat Gesture Coach",
                "Plan Quick Actions",
                "Fog of War",
                "Allow Spellforge Missions (Next Launch)",
                "Reversible Background Patches (Next Launch)",
            ]
        );
        let text = LocalizedGameplayText::for_locale("en-US");
        for setting in GameplaySetting::ALL {
            assert!(!text.option_tooltip(setting.index()).is_empty());
        }

        let mut config = GameplayConfig::default();
        assert!(is_option_selected(&config, 0));
        assert!(!is_option_selected(&config, 1));
        assert!(is_option_selected(&config, 2));
        assert!(is_option_selected(&config, 3));
        assert!(is_option_selected(&config, 4));
        assert!(is_option_selected(&config, 5));
        assert!(!is_option_selected(&config, 6));
        assert!(is_option_selected(&config, 13));
        assert!(is_option_selected(&config, 14));
        assert!(is_option_selected(&config, 15));
        assert!(is_option_selected(
            &config,
            GameplaySetting::SherwoodTrading.index()
        ));
        assert!(is_option_selected(
            &config,
            GameplaySetting::AutosaveEnabled.index()
        ));
        assert!(is_option_selected(&config, 33));
        assert!(is_option_selected(
            &config,
            GameplaySetting::DetailedSaveMetadata.index()
        ));
        assert!(is_option_selected(
            &config,
            GameplaySetting::EnableTimedMissions.index()
        ));
        assert!(is_option_selected(
            &config,
            GameplaySetting::EnableDynamicAmbience.index()
        ));
        assert!(is_option_selected(
            &config,
            GameplaySetting::Diplomacy.index()
        ));
        assert!(is_option_selected(
            &config,
            GameplaySetting::NpcFactionWars.index()
        ));
        assert!(is_option_selected(
            &config,
            GameplaySetting::MoreCombatGestures.index()
        ));
        assert!(is_option_selected(
            &config,
            GameplaySetting::GestureQualityDamage.index()
        ));
        assert!(!is_option_selected(
            &config,
            GameplaySetting::ShowCombatGestureGuide.index()
        ));
        assert!(!is_option_selected(
            &config,
            GameplaySetting::CombatGestureCoach.index()
        ));
        assert!(is_option_selected(
            &config,
            GameplaySetting::PlanQuickActions.index()
        ));
        // Fog of war is opt-in, matching GameplayConfig and its dedicated test.
        assert!(!is_option_selected(
            &config,
            GameplaySetting::FogOfWar.index()
        ));
        assert!(is_option_selected(
            &config,
            GameplaySetting::EnableSpellforgeMissions.index()
        ));

        apply_option_toggle(
            &mut config,
            GameplaySetting::EnableSpellforgeMissions.index(),
        );
        assert!(!config.enable_spellforge_missions);

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
            config.show_new_achievement_trackers,
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
                config.show_new_achievement_trackers,
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
        apply_option_toggle(&mut config, GameplaySetting::SherwoodTrading.index());
        assert!(!config.sherwood_trading);

        apply_option_toggle(&mut config, GameplaySetting::PlanQuickActions.index());
        assert!(!config.plan_quick_actions);

        let autosave_enabled = config.autosave_enabled;
        apply_option_toggle(&mut config, GameplaySetting::DetailedSaveMetadata.index());
        assert!(!config.detailed_save_metadata);
        assert_eq!(config.autosave_enabled, autosave_enabled);
    }

    #[test]
    fn autosave_has_an_independent_gameplay_toggle() {
        let mut config = GameplayConfig::default();
        let before = config;
        assert_eq!(
            english_labels()[GameplaySetting::AutosaveEnabled.index()],
            "Rotating Autosaves"
        );
        assert!(is_option_selected(
            &config,
            GameplaySetting::AutosaveEnabled.index()
        ));
        apply_option_toggle(&mut config, GameplaySetting::AutosaveEnabled.index());
        assert!(!is_option_selected(
            &config,
            GameplaySetting::AutosaveEnabled.index()
        ));
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

    #[test]
    fn every_combat_gesture_setting_is_independently_toggleable() {
        let mut config: GameplayConfig = serde_json::from_str("{}").expect("empty legacy config");
        for index in [
            GameplaySetting::MoreCombatGestures.index(),
            GameplaySetting::GestureQualityDamage.index(),
            GameplaySetting::ShowCombatGestureGuide.index(),
            GameplaySetting::CombatGestureCoach.index(),
        ] {
            assert!(!is_option_selected(&config, index));
            apply_option_toggle(&mut config, index);
            assert!(is_option_selected(&config, index));
        }
        assert!(config.more_combat_gestures);
        assert!(config.gesture_quality_damage);
        assert!(config.show_combat_gesture_guide);
        assert!(config.combat_gesture_coach);
    }

    #[test]
    fn fog_of_war_has_an_independent_default_off_toggle() {
        let mut config = GameplayConfig::default();
        let before = config;
        assert_eq!(
            english_labels()[GameplaySetting::FogOfWar.index()],
            "Fog of War"
        );
        assert!(!is_option_selected(
            &config,
            GameplaySetting::FogOfWar.index()
        ));
        apply_option_toggle(&mut config, GameplaySetting::FogOfWar.index());
        assert!(config.fog_of_war);
        assert_eq!(config.diplomacy, before.diplomacy);
        assert_eq!(config.npc_faction_wars, before.npc_faction_wars);
        assert_eq!(config.more_combat_gestures, before.more_combat_gestures);
        assert_eq!(config.gesture_quality_damage, before.gesture_quality_damage);
        assert_eq!(config.item_gameplay, before.item_gameplay);
    }
}
