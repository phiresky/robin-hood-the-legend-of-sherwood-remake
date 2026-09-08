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
use unicode_segmentation::UnicodeSegmentation;

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
pub(crate) const SHERWOOD_TRADING_OPTION_INDEX: usize = 16;
#[cfg(test)]
pub(crate) const AUTOSAVE_OPTION_INDEX: usize = 17;
#[cfg(test)]
pub(crate) const DETAILED_SAVE_METADATA_OPTION_INDEX: usize = 33;
#[cfg(test)]
pub(crate) const TIMED_MISSIONS_OPTION_INDEX: usize = 34;
#[cfg(test)]
pub(crate) const DYNAMIC_AMBIENCE_OPTION_INDEX: usize = 35;
#[cfg(test)]
pub(crate) const DIPLOMACY_OPTION_INDEX: usize = 36;
#[cfg(test)]
pub(crate) const NPC_FACTION_WARS_OPTION_INDEX: usize = 37;
#[cfg(test)]
pub(crate) const MORE_COMBAT_GESTURES_OPTION_INDEX: usize = 38;
#[cfg(test)]
pub(crate) const GESTURE_QUALITY_DAMAGE_OPTION_INDEX: usize = 39;
#[cfg(test)]
pub(crate) const COMBAT_GESTURE_GUIDE_OPTION_INDEX: usize = 40;
#[cfg(test)]
pub(crate) const COMBAT_GESTURE_COACH_OPTION_INDEX: usize = 41;
#[cfg(test)]
pub(crate) const PLAN_QUICK_ACTIONS_OPTION_INDEX: usize = 42;
#[cfg(test)]
pub(crate) const FOG_OF_WAR_OPTION_INDEX: usize = 43;
pub(crate) const SPELLFORGE_OPTION_INDEX: usize = 44;

/// Stable identity for every persisted Gameplay row.
///
/// Display order is defined by [`GameplaySetting::ALL`]. Simulation authority
/// and mutation dispatch match this type, never a rendered row position.
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GameplaySetting {
    FixHardReactionTimes,
    ControlTacticalUnits,
    EnableUnbinding,
    ShowProductionForecast,
    ReusableCloaks,
    CampaignPresentation,
    CleanHandsNpcKillsInvalidate,
    ShowDetailedXp,
    ShowSpeedrunTracker,
    ShowCleanHandsTracker,
    ShowGhostTracker,
    ShowPileOfBonesTracker,
    ShowAllEnemiesStashedTracker,
    ShowAchievementBadges,
    ShowAchievementDebrief,
    TouchCameraGestures,
    SherwoodTrading,
    AutosaveEnabled,
    AppleCombatInterrupt,
    WaspReliableAcquisition,
    StoneGroundDistraction,
    StoneLongerRange,
    NetSelectiveImmunity,
    AleReliableDistraction,
    NoiseDistractionFeedback,
    PreviewAppleEffect,
    PreviewStoneDirectEffect,
    PreviewStoneDistractionArea,
    PreviewNetCaptureArea,
    PreviewNetCrumplePrediction,
    PreviewAleEffect,
    PreviewPurseEffect,
    PreviewWaspArea,
    DetailedSaveMetadata,
    EnableTimedMissions,
    EnableDynamicAmbience,
    Diplomacy,
    NpcFactionWars,
    MoreCombatGestures,
    GestureQualityDamage,
    ShowCombatGestureGuide,
    CombatGestureCoach,
    PlanQuickActions,
    FogOfWar,
    EnableSpellforgeMissions,
    ReversibleBackgroundPatches,
}

impl GameplaySetting {
    pub(crate) const ALL: [Self; 46] = [
        Self::FixHardReactionTimes,
        Self::ControlTacticalUnits,
        Self::EnableUnbinding,
        Self::ShowProductionForecast,
        Self::ReusableCloaks,
        Self::CampaignPresentation,
        Self::CleanHandsNpcKillsInvalidate,
        Self::ShowDetailedXp,
        Self::ShowSpeedrunTracker,
        Self::ShowCleanHandsTracker,
        Self::ShowGhostTracker,
        Self::ShowPileOfBonesTracker,
        Self::ShowAllEnemiesStashedTracker,
        Self::ShowAchievementBadges,
        Self::ShowAchievementDebrief,
        Self::TouchCameraGestures,
        Self::SherwoodTrading,
        Self::AutosaveEnabled,
        Self::AppleCombatInterrupt,
        Self::WaspReliableAcquisition,
        Self::StoneGroundDistraction,
        Self::StoneLongerRange,
        Self::NetSelectiveImmunity,
        Self::AleReliableDistraction,
        Self::NoiseDistractionFeedback,
        Self::PreviewAppleEffect,
        Self::PreviewStoneDirectEffect,
        Self::PreviewStoneDistractionArea,
        Self::PreviewNetCaptureArea,
        Self::PreviewNetCrumplePrediction,
        Self::PreviewAleEffect,
        Self::PreviewPurseEffect,
        Self::PreviewWaspArea,
        Self::DetailedSaveMetadata,
        Self::EnableTimedMissions,
        Self::EnableDynamicAmbience,
        Self::Diplomacy,
        Self::NpcFactionWars,
        Self::MoreCombatGestures,
        Self::GestureQualityDamage,
        Self::ShowCombatGestureGuide,
        Self::CombatGestureCoach,
        Self::PlanQuickActions,
        Self::FogOfWar,
        Self::EnableSpellforgeMissions,
        Self::ReversibleBackgroundPatches,
    ];

    pub(crate) const fn index(self) -> usize {
        self as usize
    }

    pub(crate) fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

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
    "More Combat Gestures",
    "Gesture Quality Damage",
    "Show Combat Gesture Guide",
    "Combat Gesture Coach",
    "Plan Quick Actions",
    "Fog of War",
    "Allow Spellforge Missions (Next Launch)",
    "Reversible Background Patches (Next Launch)",
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
    "Recognize nine additional sword gestures as composite two-strike techniques.",
    "Scale sword damage to the recognized gesture's deterministic quality tier.",
    "Show reference paths for the additional combat gestures while swordfighting.",
    "Briefly show the recognized or nearest gesture and its quality after drawing.",
    "Allow the rebindable Plan modifier and touch HUD to queue quick actions.",
    "Enable shared allied sight, explored terrain, and temporary hostile intelligence.",
    "Allow executable Spellforge custom missions. This takes effect on the next mission launch.",
    "Repeat animated terrain triggers to reverse their animation, obstacles and doors. Applies to newly launched missions; off preserves original one-shot patches.",
];

pub(crate) fn option_tooltip(index: usize) -> &'static str {
    OPTION_TOOLTIPS
        .get(index)
        .copied()
        .unwrap_or_else(|| panic!("gameplay option tooltip index {index} is out of range"))
}

/// Port-owned Gameplay strings shared by the blocking options screen and the
/// cooperative pause-side state. Keeping one resolved catalogue snapshot
/// prevents the multiplayer-safe path from silently falling back to English.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LocalizedGameplayText {
    spellforge_label: &'static str,
    spellforge_tooltip: &'static str,
    manage_content: &'static str,
}

impl LocalizedGameplayText {
    pub(crate) fn from_application_context(
        application_context: &crate::host::ApplicationContext,
    ) -> Self {
        let required = |key| {
            application_context
                .port_text(key)
                .unwrap_or_else(|error| panic!("Gameplay screen lost localized text: {error}"))
        };
        Self {
            spellforge_label: required(PortTextKey::SpellforgeGameplayAllowLabel),
            spellforge_tooltip: required(PortTextKey::SpellforgeGameplayAllowTooltip),
            manage_content: required(PortTextKey::SpellforgeManageContent),
        }
    }

    #[cfg(test)]
    fn for_locale(locale: &str) -> Self {
        Self {
            spellforge_label: crate::localization::port_text(
                Some(locale),
                PortTextKey::SpellforgeGameplayAllowLabel,
            ),
            spellforge_tooltip: crate::localization::port_text(
                Some(locale),
                PortTextKey::SpellforgeGameplayAllowTooltip,
            ),
            manage_content: crate::localization::port_text(
                Some(locale),
                PortTextKey::SpellforgeManageContent,
            ),
        }
    }

    pub(crate) fn option_label(self, index: usize) -> &'static str {
        if index == SPELLFORGE_OPTION_INDEX {
            self.spellforge_label
        } else {
            OPTION_LABELS
                .get(index)
                .copied()
                .unwrap_or_else(|| panic!("gameplay option label index {index} is out of range"))
        }
    }

    pub(crate) fn option_tooltip(self, index: usize) -> &'static str {
        if index == SPELLFORGE_OPTION_INDEX {
            self.spellforge_tooltip
        } else {
            option_tooltip(index)
        }
    }

    pub(crate) fn manage_content(self) -> &'static str {
        self.manage_content
    }
}

/// Elide a localized label at grapheme boundaries. The full text remains in
/// the row help/tooltip, while button text is guaranteed to stay inside its
/// fixed 640x480 virtual-space hit box.
pub(crate) fn elide_to_width_by(
    text: &str,
    max_width: i32,
    measure: impl Fn(&str) -> i32,
) -> String {
    const ELLIPSIS: &str = "…";
    if max_width <= 0 || measure(ELLIPSIS) > max_width {
        return String::new();
    }
    if measure(text) <= max_width {
        return text.to_owned();
    }

    let mut fit_end = 0usize;
    for boundary in text
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
    {
        let candidate = format!("{}{ELLIPSIS}", text[..boundary].trim_end());
        if measure(&candidate) > max_width {
            break;
        }
        fit_end = boundary;
    }
    format!("{}{ELLIPSIS}", text[..fit_end].trim_end())
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
    OPTION_LABELS
        .len()
        .div_ceil(STANDALONE_OPTIONS_PER_PAGE)
        .max(1)
}

fn standalone_visible_option_range(page: usize) -> std::ops::Range<usize> {
    let page = page.min(standalone_page_count() - 1);
    let start = page * STANDALONE_OPTIONS_PER_PAGE;
    start..(start + STANDALONE_OPTIONS_PER_PAGE).min(OPTION_LABELS.len())
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
        let enabled = option_index != SHERWOOD_TRADING_OPTION_INDEX || sherwood_trading_editable;
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
        crate::window::sleep_ms(16).await;
    }
}

/// Owned, one-frame state for the gameplay settings page.
pub struct GameplayScreenState {
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
        if self.frame.widget(ID_OPT_BASE + 5).is_some()
            && let Some(font) = resources.label_font_any()
        {
            render_text_virt_font(
                renderer,
                font,
                self.transform,
                self.working.campaign_presentation.label(),
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
        Setting::ShowAllEnemiesStashedTracker => {
            config.show_all_enemies_one_building_tracker =
                !config.show_all_enemies_one_building_tracker
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
            Setting::ShowAllEnemiesStashedTracker => config.show_all_enemies_one_building_tracker,
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
    fn standalone_pages_cover_every_gameplay_option_once() {
        assert_eq!(standalone_page_count(), 4);
        assert_eq!(standalone_visible_option_range(0), 0..12);
        assert_eq!(standalone_visible_option_range(1), 12..24);
        assert_eq!(standalone_visible_option_range(2), 24..36);
        assert_eq!(standalone_visible_option_range(3), 36..46);

        let covered: Vec<_> = (0..standalone_page_count())
            .flat_map(standalone_visible_option_range)
            .collect();
        assert_eq!(covered, (0..OPTION_LABELS.len()).collect::<Vec<_>>());
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
                localized.option_label(SPELLFORGE_OPTION_INDEX),
                crate::localization::port_text(
                    Some(locale),
                    PortTextKey::SpellforgeGameplayAllowLabel,
                )
            );
            assert_eq!(
                localized.option_tooltip(SPELLFORGE_OPTION_INDEX),
                crate::localization::port_text(
                    Some(locale),
                    PortTextKey::SpellforgeGameplayAllowTooltip,
                )
            );
            assert_eq!(
                localized.manage_content(),
                crate::localization::port_text(Some(locale), PortTextKey::SpellforgeManageContent)
            );
            assert!(!localized.option_label(SPELLFORGE_OPTION_INDEX).is_empty());
            assert!(!localized.option_tooltip(SPELLFORGE_OPTION_INDEX).is_empty());
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
                    localized.option_label(SPELLFORGE_OPTION_INDEX),
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
        for index in 0..OPTION_LABELS.len() {
            let mut config = baseline;
            let selected_before = is_option_selected(&config, index);
            apply_option_toggle(&mut config, index);
            if index == 5 {
                assert_ne!(
                    config.campaign_presentation, baseline.campaign_presentation,
                    "gameplay row {index} ({}) did not change its setting",
                    OPTION_LABELS[index],
                );
            } else {
                assert_ne!(
                    is_option_selected(&config, index),
                    selected_before,
                    "gameplay row {index} ({}) did not change its setting",
                    OPTION_LABELS[index],
                );
            }
        }
    }

    #[test]
    fn multiplayer_authority_classification_covers_every_simulation_row() {
        let authoritative: Vec<_> = (0..OPTION_LABELS.len())
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
        assert_eq!(OPTION_LABELS.len(), OPTION_TOOLTIPS.len());

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
        assert!(is_option_selected(&config, SHERWOOD_TRADING_OPTION_INDEX));
        assert!(is_option_selected(&config, AUTOSAVE_OPTION_INDEX));
        assert!(is_option_selected(&config, 33));
        assert!(is_option_selected(
            &config,
            DETAILED_SAVE_METADATA_OPTION_INDEX
        ));
        assert!(is_option_selected(&config, TIMED_MISSIONS_OPTION_INDEX));
        assert!(is_option_selected(&config, DYNAMIC_AMBIENCE_OPTION_INDEX));
        assert!(is_option_selected(&config, DIPLOMACY_OPTION_INDEX));
        assert!(is_option_selected(&config, NPC_FACTION_WARS_OPTION_INDEX));
        assert!(is_option_selected(
            &config,
            MORE_COMBAT_GESTURES_OPTION_INDEX
        ));
        assert!(is_option_selected(
            &config,
            GESTURE_QUALITY_DAMAGE_OPTION_INDEX
        ));
        assert!(!is_option_selected(
            &config,
            COMBAT_GESTURE_GUIDE_OPTION_INDEX
        ));
        assert!(!is_option_selected(
            &config,
            COMBAT_GESTURE_COACH_OPTION_INDEX
        ));
        assert!(is_option_selected(&config, PLAN_QUICK_ACTIONS_OPTION_INDEX));
        // Fog of war is opt-in, matching GameplayConfig and its dedicated test.
        assert!(!is_option_selected(&config, FOG_OF_WAR_OPTION_INDEX));
        assert!(is_option_selected(&config, SPELLFORGE_OPTION_INDEX));

        apply_option_toggle(&mut config, SPELLFORGE_OPTION_INDEX);
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

        apply_option_toggle(&mut config, PLAN_QUICK_ACTIONS_OPTION_INDEX);
        assert!(!config.plan_quick_actions);

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

    #[test]
    fn every_combat_gesture_setting_is_independently_toggleable() {
        let mut config: GameplayConfig = serde_json::from_str("{}").expect("empty legacy config");
        for index in [
            MORE_COMBAT_GESTURES_OPTION_INDEX,
            GESTURE_QUALITY_DAMAGE_OPTION_INDEX,
            COMBAT_GESTURE_GUIDE_OPTION_INDEX,
            COMBAT_GESTURE_COACH_OPTION_INDEX,
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
        assert_eq!(OPTION_LABELS[FOG_OF_WAR_OPTION_INDEX], "Fog of War");
        assert!(!is_option_selected(&config, FOG_OF_WAR_OPTION_INDEX));
        apply_option_toggle(&mut config, FOG_OF_WAR_OPTION_INDEX);
        assert!(config.fog_of_war);
        assert_eq!(config.diplomacy, before.diplomacy);
        assert_eq!(config.npc_faction_wars, before.npc_faction_wars);
        assert_eq!(config.more_combat_gestures, before.more_combat_gestures);
        assert_eq!(config.gesture_quality_damage, before.gesture_quality_damage);
        assert_eq!(config.item_gameplay, before.item_gameplay);
    }
}
