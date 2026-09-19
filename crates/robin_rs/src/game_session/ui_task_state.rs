//! Cooperative pause-side screens.
//!
//! Unlike the compatibility `show_*` menu helpers, these states advance at
//! most one UI frame per mission frame.  The mission driver therefore keeps
//! servicing networking, HTTP control, replay bookkeeping, and simulation
//! while a local side screen is open.

use crate::ingame_menu::resources::IngameMenuResources;
use crate::ingame_menu::widget_bridge::{self, ModalScreenIo};
use crate::ingame_menu::{
    SaveLoadMode, SaveLoadOutcome, SavePickerConfig, SavePickerModalState, YesNoModalState,
};
use crate::key_config::KeyConfig;
#[cfg(test)]
use crate::options_model::SoundSetting;
use crate::options_model::sound_eq;
use crate::renderer::Renderer;
use crate::savegame::SaveGameManager;
use crate::sound::{AudioBackend, SoundManager};
use robin_engine::gameplay_config::GameplayConfig;
use robin_engine::graphic_config::GraphicConfig;
use robin_engine::multiplayer_config::MultiplayerConfig;
use robin_engine::profiles::ProfileManager;
use robin_engine::sound_cache::SampleLoader;
use robin_engine::sound_config::SoundConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OptionsTaskResult {
    pub(super) profile_id: u32,
    pub(super) graphic_config: GraphicConfig,
    /// Authoritative mission-facing values used for command dispatch.
    pub(super) gameplay_config: GameplayConfig,
    /// Profile-facing values. A multiplayer client retains every future-host
    /// rule preference while current host-owned rows are displayed read-only.
    pub(super) profile_gameplay_config: GameplayConfig,
    pub(super) multiplayer_config: MultiplayerConfig,
    /// Mission-facing sound values. `amount_of_speaking` is authoritative
    /// simulation state; the remaining values are local presentation.
    pub(super) sound_config: SoundConfig,
    /// Profile-facing sound values. A multiplayer client retains its own
    /// future-host speech-frequency preference.
    pub(super) profile_sound_config: SoundConfig,
    pub(super) key_config: KeyConfig,
    pub(super) custom_key_config: KeyConfig,
    pub(super) changed: bool,
    pub(super) resolution_changed: bool,
    pub(super) key_config_changed: bool,
    pub(super) original_amount_of_speaking: u16,
    pub(super) original_gameplay_config: GameplayConfig,
}

pub(super) enum UiTaskOutcome {
    ReturnToPause,
    OptionsAccepted(OptionsTaskResult),
    SaveLoadSelected {
        mode: SaveLoadMode,
        filename: String,
        mission_id: u32,
    },
    QuickLoadAccepted {
        load: crate::main_entry::PreparedLoad,
    },
    QuickLoadCancelled,
    QuitMissionRequested,
    ExitRequested,
    MissionEndLeaderboardFinished(
        Option<crate::leaderboard_mission_end::MissionEndLeaderboardController>,
    ),
}

pub(super) enum ActiveUiTask {
    CampaignManager(crate::campaign_map::CampaignMapModalState),
    Options(OptionsTaskState),
    SaveLoad(SaveLoadTaskState),
    Quit(YesNoModalState),
    QuickLoad(QuickLoadTaskState),
    MissionEndLeaderboard(super::leaderboard_runtime::MissionEndLeaderboardTaskState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum UiTaskKind {
    CampaignManager,
    Options,
    SaveLoad,
    QuitConfirmation,
    QuickLoadConfirmation,
    MissionEndLeaderboard,
}

impl ActiveUiTask {
    pub(super) fn tick(
        &mut self,
        application_context: &crate::host::ApplicationContext,
        io: &mut ModalScreenIo<'_, '_>,
        save_manager: &mut SaveGameManager,
        profiles: Option<&ProfileManager>,
        sound: Option<&mut SoundManager>,
        audio_backend: Option<&mut dyn AudioBackend>,
        sample_loader: Option<&SampleLoader>,
    ) -> Option<UiTaskOutcome> {
        let mut audio = widget_bridge::ScreenAudio {
            sound,
            // Re-coerce the trait object to the bundle's shorter lifetime.
            backend: audio_backend.map(|backend| &mut *backend as &mut dyn AudioBackend),
            sample_loader,
        };
        match self {
            Self::CampaignManager(state) => state
                .tick_browser(io.window, io.renderer, io.cursor)
                .map(|exit| {
                    if exit {
                        UiTaskOutcome::ExitRequested
                    } else {
                        UiTaskOutcome::ReturnToPause
                    }
                }),
            Self::Options(state) => state.tick(application_context, io, &mut audio),
            Self::SaveLoad(state) => state.tick(io, save_manager, profiles, audio),
            Self::Quit(state) => {
                let result = state.tick(io);
                if io.window.close_requested {
                    return Some(UiTaskOutcome::ExitRequested);
                }
                result.map(|yes| {
                    if yes {
                        UiTaskOutcome::QuitMissionRequested
                    } else {
                        UiTaskOutcome::ReturnToPause
                    }
                })
            }
            Self::QuickLoad(state) => state.tick(io),
            Self::MissionEndLeaderboard(state) => match state.tick(io) {
                super::leaderboard_runtime::MissionEndLeaderboardTaskProgress::Pending => None,
                super::leaderboard_runtime::MissionEndLeaderboardTaskProgress::Finished => {
                    Some(UiTaskOutcome::MissionEndLeaderboardFinished(None))
                }
                super::leaderboard_runtime::MissionEndLeaderboardTaskProgress::Detach(
                    controller,
                ) => Some(UiTaskOutcome::MissionEndLeaderboardFinished(Some(
                    controller,
                ))),
            },
        }
    }

    pub(super) fn is_mission_end_leaderboard(&self) -> bool {
        matches!(self, Self::MissionEndLeaderboard(_))
    }

    pub(super) fn owns_presentation(&self) -> bool {
        match self {
            Self::MissionEndLeaderboard(state) => state.owns_presentation(),
            Self::CampaignManager(_)
            | Self::Options(_)
            | Self::SaveLoad(_)
            | Self::Quit(_)
            | Self::QuickLoad(_) => true,
        }
    }

    pub(super) fn kind(&self) -> UiTaskKind {
        match self {
            Self::CampaignManager(_) => UiTaskKind::CampaignManager,
            Self::Options(_) => UiTaskKind::Options,
            Self::SaveLoad(_) => UiTaskKind::SaveLoad,
            Self::Quit(_) => UiTaskKind::QuitConfirmation,
            Self::QuickLoad(_) => UiTaskKind::QuickLoadConfirmation,
            Self::MissionEndLeaderboard(_) => UiTaskKind::MissionEndLeaderboard,
        }
    }

    /// Resolve a local task without accepting a destructive or mutating
    /// action. HTTP automation uses this before stepping when auto-dismiss is
    /// enabled; callers still own pause-menu restoration and GPU cleanup.
    pub(super) fn auto_dismiss(&mut self) -> UiTaskOutcome {
        match self {
            Self::QuickLoad(_) => UiTaskOutcome::QuickLoadCancelled,
            Self::CampaignManager(_) | Self::Options(_) | Self::SaveLoad(_) | Self::Quit(_) => {
                UiTaskOutcome::ReturnToPause
            }
            Self::MissionEndLeaderboard(_) => {
                panic!("mission-end leaderboard auto-dismiss must preserve background work")
            }
        }
    }

    pub(super) fn cleanup(&mut self) {
        if let Self::SaveLoad(state) = self {
            state.cleanup();
        }
    }
}

impl UiTaskKind {
    /// Require the per-request opt-in before HTTP stepping cancels a local
    /// pause-side task. These tasks are presentation state rather than
    /// authoritative [`robin_engine::player_command::ModalKind`] values, so a
    /// typed gameplay-modal dismissal cannot stand in for `auto_dismiss`.
    pub(super) fn require_http_auto_dismiss(
        self,
        policy: &crate::http_server::StepModalPolicy,
    ) -> Result<(), String> {
        if self == Self::MissionEndLeaderboard {
            return Err(
                "mission-end leaderboard cannot be discarded by HTTP stepping; dismiss it in the game"
                    .to_owned(),
            );
        }
        if policy.auto_dismiss {
            return Ok(());
        }
        Err(format!(
            "blocked by local UI task {self:?}; retry with auto_dismiss=true or dismiss it in the game"
        ))
    }
}

pub(super) struct QuickLoadTaskState {
    dialog: YesNoModalState,
    load: Option<crate::main_entry::PreparedLoad>,
}

impl QuickLoadTaskState {
    pub(super) fn new(
        window: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        message: String,
        load: crate::main_entry::PreparedLoad,
    ) -> Self {
        Self {
            dialog: YesNoModalState::new(window, renderer, resources, message),
            load: Some(load),
        }
    }

    fn tick(&mut self, io: &mut ModalScreenIo<'_, '_>) -> Option<UiTaskOutcome> {
        let result = self.dialog.tick(io);
        if io.window.close_requested {
            return Some(UiTaskOutcome::ExitRequested);
        }
        result.map(|accepted| {
            if accepted {
                UiTaskOutcome::QuickLoadAccepted {
                    load: self
                        .load
                        .take()
                        .expect("accepted QuickLoad must retain its decoded payload"),
                }
            } else {
                UiTaskOutcome::QuickLoadCancelled
            }
        })
    }
}

/// Mission ownership and persistence adapter around the shared Options presenter.
pub(super) struct OptionsTaskState {
    screen: crate::ingame_menu::options::OptionsModalState,
    seed: OptionsSeed,
}

/// Settings the in-game Options task starts from: the active profile's rows,
/// with deterministic gameplay/sound rows already overridden from the running
/// mission, plus the untouched profile copies used to restore on cancel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OptionsSeed {
    pub(super) profile_id: u32,
    pub(super) graphic: GraphicConfig,
    /// Gameplay rows seeded from the active mission's simulation config.
    pub(super) gameplay: GameplayConfig,
    pub(super) profile_gameplay: GameplayConfig,
    pub(super) multiplayer: MultiplayerConfig,
    /// Sound rows seeded from the active mission's simulation config.
    pub(super) sound: SoundConfig,
    pub(super) profile_sound: SoundConfig,
    pub(super) keys: KeyConfig,
    pub(super) custom_keys: KeyConfig,
    pub(super) host_gameplay_rules_editable: bool,
}

impl OptionsTaskState {
    pub(super) fn new(
        application_context: &crate::host::ApplicationContext,
        window: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        seed: OptionsSeed,
    ) -> Self {
        let controller = options_controller(&seed);
        Self {
            screen: crate::ingame_menu::options::OptionsModalState::new(
                application_context,
                window,
                renderer,
                resources,
                controller,
                crate::ingame_menu::options::OptionsScope {
                    allow_language_switching: false,
                    sherwood_trading_editable: seed.host_gameplay_rules_editable,
                    host_gameplay_rules_editable: seed.host_gameplay_rules_editable,
                    apply_live_preferences: false,
                },
            ),
            seed,
        }
    }

    fn tick(
        &mut self,
        application_context: &crate::host::ApplicationContext,
        io: &mut ModalScreenIo<'_, '_>,
        audio: &mut widget_bridge::ScreenAudio<'_>,
    ) -> Option<UiTaskOutcome> {
        self.screen
            .tick(application_context, io, audio)
            .map(|outcome| {
                if outcome.exit_requested {
                    UiTaskOutcome::ExitRequested
                } else {
                    UiTaskOutcome::OptionsAccepted(options_result(
                        &self.seed,
                        &self.screen.controller,
                    ))
                }
            })
    }
}

fn options_controller(seed: &OptionsSeed) -> crate::options_model::OptionsController {
    crate::options_model::OptionsController::new(
        seed.graphic.clone(),
        seed.sound,
        seed.gameplay,
        seed.multiplayer,
        seed.keys.clone(),
        seed.custom_keys.clone(),
    )
}

fn options_result(
    seed: &OptionsSeed,
    controller: &crate::options_model::OptionsController,
) -> OptionsTaskResult {
    let profile_gameplay = profile_gameplay_after_options(
        controller.gameplay,
        seed.profile_gameplay,
        seed.host_gameplay_rules_editable,
    );
    let profile_sound = profile_sound_after_options(
        controller.sound.working,
        seed.profile_sound,
        seed.host_gameplay_rules_editable,
    );
    OptionsTaskResult {
        profile_id: seed.profile_id,
        graphic_config: controller.graphic.working.clone(),
        gameplay_config: controller.gameplay,
        profile_gameplay_config: profile_gameplay,
        multiplayer_config: controller.multiplayer,
        sound_config: controller.sound.working,
        profile_sound_config: profile_sound,
        key_config: controller.keys.clone(),
        custom_key_config: controller.custom_keys.clone(),
        changed: controller.graphic.changed()
            || controller.gameplay != seed.gameplay
            || profile_gameplay != seed.profile_gameplay
            || controller.multiplayer != seed.multiplayer
            || controller.sound.changed()
            || !sound_eq(&profile_sound, &seed.profile_sound),
        resolution_changed: controller.graphic.resolution_changed(),
        key_config_changed: controller.keys != seed.keys
            || controller.custom_keys != seed.custom_keys,
        original_amount_of_speaking: seed.sound.amount_of_speaking,
        original_gameplay_config: seed.gameplay,
    }
}

fn profile_gameplay_after_options(
    working: GameplayConfig,
    original_profile: GameplayConfig,
    host_gameplay_rules_editable: bool,
) -> GameplayConfig {
    if host_gameplay_rules_editable {
        return working;
    }
    GameplayConfig {
        // These rows display the current host's authoritative values to
        // clients, but must not overwrite the client's preferences for a
        // future session they host themselves.
        fix_hard_reaction_times: original_profile.fix_hard_reaction_times,
        enable_unbinding: original_profile.enable_unbinding,
        clean_hands_npc_kills_invalidate: original_profile.clean_hands_npc_kills_invalidate,
        reusable_cloaks: original_profile.reusable_cloaks,
        item_gameplay: original_profile.item_gameplay,
        noise_distraction_feedback: original_profile.noise_distraction_feedback,
        sherwood_trading: original_profile.sherwood_trading,
        enable_timed_missions: original_profile.enable_timed_missions,
        enable_dynamic_ambience: original_profile.enable_dynamic_ambience,
        diplomacy: original_profile.diplomacy,
        npc_faction_wars: original_profile.npc_faction_wars,
        more_combat_gestures: original_profile.more_combat_gestures,
        gesture_quality_damage: original_profile.gesture_quality_damage,
        fog_of_war: original_profile.fog_of_war,
        ..working
    }
}

fn profile_sound_after_options(
    mut working: SoundConfig,
    original_profile: SoundConfig,
    host_gameplay_rules_editable: bool,
) -> SoundConfig {
    if !host_gameplay_rules_editable {
        // Speech frequency participates in deterministic chorus suppression;
        // clients display the host value read-only without adopting it.
        working.amount_of_speaking = original_profile.amount_of_speaking;
    }
    working
}

/// Mission ownership adapter around the shared save/load picker.
pub(super) struct SaveLoadTaskState {
    mode: SaveLoadMode,
    mission_id: u32,
    picker: SavePickerModalState,
}

impl SaveLoadTaskState {
    pub(super) fn new(
        window: &crate::window::GameWindow,
        renderer: &Renderer,
        save_manager: &mut SaveGameManager,
        mission_id: u32,
        detailed_metadata: bool,
        mode: SaveLoadMode,
        multiplayer_connected: bool,
    ) -> Self {
        Self {
            mode,
            mission_id,
            picker: SavePickerModalState::new(
                window,
                renderer,
                save_manager,
                SavePickerConfig {
                    mode,
                    mission_id: Some(mission_id),
                    detailed_metadata,
                    multiplayer_connected,
                    // Task cancellation paths have no renderer to release a
                    // preview surface with.
                    previews: false,
                },
            ),
        }
    }

    fn tick(
        &mut self,
        io: &mut ModalScreenIo<'_, '_>,
        save_manager: &mut SaveGameManager,
        profiles: Option<&ProfileManager>,
        audio: widget_bridge::ScreenAudio<'_>,
    ) -> Option<UiTaskOutcome> {
        let outcome = self.picker.tick(io, save_manager, profiles, audio);
        if io.window.close_requested {
            return Some(UiTaskOutcome::ExitRequested);
        }
        Some(match outcome? {
            SaveLoadOutcome::Cancel => UiTaskOutcome::ReturnToPause,
            SaveLoadOutcome::Slot(slot) => UiTaskOutcome::SaveLoadSelected {
                mode: self.mode,
                filename: save_manager
                    .slot_name(slot)
                    .expect("accepted save slot has a validated identity")
                    .as_str()
                    .to_owned(),
                mission_id: self.mission_id,
            },
        })
    }

    fn cleanup(&mut self) {
        self.picker.stop_text_input();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options_model::OptionsPage;
    use crate::options_model::{
        assign_shortcut as assign_key, promote_shortcut_edits, shortcut_keys as key_vec,
    };
    use winit::keyboard::KeyCode;

    fn options_fixture() -> (OptionsSeed, crate::options_model::OptionsController) {
        let seed = OptionsSeed {
            profile_id: 1,
            graphic: GraphicConfig::default(),
            sound: SoundConfig::default(),
            profile_sound: SoundConfig::default(),
            gameplay: GameplayConfig::default(),
            profile_gameplay: GameplayConfig::default(),
            multiplayer: MultiplayerConfig::default(),
            keys: KeyConfig::default_preset(),
            custom_keys: KeyConfig::default_preset(),
            host_gameplay_rules_editable: true,
        };
        let controller = options_controller(&seed);
        (seed, controller)
    }

    #[test]
    fn options_final_outcome_persists_custom_only_and_type_only_edits() {
        for change in 0..3 {
            let (seed, mut controller) = options_fixture();
            controller.enter_page(OptionsPage::Shortcuts);
            match change {
                0 => controller
                    .custom_keys
                    .set_binding("ZoomIn", Some(KeyCode::F6), None),
                1 => controller.keys.key_type += 1,
                2 => controller.custom_keys.key_type += 1,
                _ => unreachable!(),
            }
            controller.accept_page(false);
            let result = options_result(&seed, &controller);
            assert!(
                result.key_config_changed,
                "edit {change} must reach persistence"
            );
            assert_eq!(result.key_config, controller.keys);
            assert_eq!(result.custom_key_config, controller.custom_keys);
        }
    }

    #[test]
    fn options_final_outcome_does_not_persist_cancelled_or_reverted_edits() {
        for cancel in [false, true] {
            let (seed, mut controller) = options_fixture();
            controller.enter_page(OptionsPage::Shortcuts);
            controller.custom_keys.key_type += 1;
            if cancel {
                controller.cancel_page();
            } else {
                controller.accept_page(false);
                controller.enter_page(OptionsPage::Shortcuts);
                controller.custom_keys = seed.custom_keys.clone();
                controller.accept_page(false);
            }
            let result = options_result(&seed, &controller);
            assert!(!result.key_config_changed);
        }
    }

    #[test]
    fn multiplayer_client_sees_all_host_rules_read_only_and_retains_own_preferences() {
        let client_profile = GameplayConfig::default();
        let host_values = GameplayConfig {
            fix_hard_reaction_times: false,
            enable_unbinding: false,
            clean_hands_npc_kills_invalidate: true,
            reusable_cloaks: false,
            item_gameplay: robin_engine::gameplay_config::ItemGameplayConfig::classic(),
            noise_distraction_feedback: false,
            sherwood_trading: false,
            enable_timed_missions: false,
            enable_dynamic_ambience: false,
            diplomacy: false,
            npc_faction_wars: false,
            more_combat_gestures: false,
            gesture_quality_damage: false,
            fog_of_war: false,
            ..client_profile
        };
        let persisted = profile_gameplay_after_options(host_values, client_profile, false);
        assert_eq!(persisted, client_profile);

        let hosted = profile_gameplay_after_options(host_values, client_profile, true);
        assert_eq!(hosted, host_values);
    }

    #[test]
    fn multiplayer_client_keeps_local_sound_but_not_host_speech_frequency() {
        assert!(SoundSetting::CommentFrequency.requires_host_authority());
        let profile = SoundConfig {
            fx_volume: 3,
            amount_of_speaking: 2,
            ..SoundConfig::default()
        };
        let host_display = SoundConfig {
            fx_volume: 7,
            amount_of_speaking: 8,
            ..profile
        };
        let persisted = profile_sound_after_options(host_display, profile, false);
        assert_eq!(persisted.fx_volume, 7);
        assert_eq!(persisted.amount_of_speaking, 2);
        assert_eq!(
            profile_sound_after_options(host_display, profile, true).amount_of_speaking,
            8,
        );
    }

    #[test]
    fn shortcut_assignment_clears_previous_owner() {
        let mut keys = KeyConfig::default_preset();
        let key = keys.get_key_by_index(0).expect("default ZoomIn binding");
        assign_key(&mut keys, 18, key);
        assert_eq!(keys.get_key_by_index(0), None);
        assert_eq!(keys.get_key_by_index(18), Some(key));
    }

    #[test]
    fn cooperative_shortcuts_share_shift_and_clear_all_other_conflicts() {
        use crate::key_config::PLAN_QUICK_ACTIONS_INDEX;
        let mut keys = KeyConfig::default_preset();
        keys.set_key_by_index(16, Some(KeyCode::ShiftLeft));
        assign_key(&mut keys, PLAN_QUICK_ACTIONS_INDEX, KeyCode::ShiftLeft);
        assert_eq!(keys.get_key_by_index(16), Some(KeyCode::ShiftLeft));
        assert_eq!(
            keys.get_key_by_index(PLAN_QUICK_ACTIONS_INDEX),
            Some(KeyCode::ShiftLeft)
        );
        for index in [0, 1, 2] {
            keys.set_key_by_index(index, Some(KeyCode::F6));
        }
        assign_key(&mut keys, 18, KeyCode::F6);
        for index in [0, 1, 2] {
            assert_eq!(keys.get_key_by_index(index), None);
        }
        assert_eq!(keys.get_key_by_index(18), Some(KeyCode::F6));
        assert_eq!(keys.key_type, 1);
    }

    #[test]
    fn selecting_a_preset_preserves_stored_custom_bindings() {
        let mut active = KeyConfig::default_preset();
        let custom = KeyConfig::alternate_preset();
        let custom_before = key_vec(&custom);
        let mut custom = custom;
        let mut dirty = false;

        promote_shortcut_edits(&active, &mut custom, &mut dirty);
        active = KeyConfig::alternate_preset();

        assert_eq!(key_vec(&custom), custom_before);
        assert_eq!(key_vec(&active), key_vec(&KeyConfig::alternate_preset()));
    }

    #[test]
    fn direct_shortcut_edits_are_promoted_before_a_preset_switch() {
        let mut active = KeyConfig::default_preset();
        let mut custom = KeyConfig::alternate_preset();
        let replacement = KeyCode::F6;
        assign_key(&mut active, 0, replacement);
        let mut dirty = true;

        promote_shortcut_edits(&active, &mut custom, &mut dirty);

        assert_eq!(custom.get_key_by_index(0), Some(replacement));
        assert_eq!(custom.key_type, 1);
        assert!(!dirty);
    }

    #[test]
    fn strict_http_steps_preserve_every_pause_side_task_kind() {
        let strict = crate::http_server::StepModalPolicy {
            auto_dismiss: false,
            dismissals: Vec::new(),
            synchronized_multiplayer: false,
        };
        for kind in [
            UiTaskKind::Options,
            UiTaskKind::SaveLoad,
            UiTaskKind::QuitConfirmation,
            UiTaskKind::QuickLoadConfirmation,
        ] {
            let error = kind
                .require_http_auto_dismiss(&strict)
                .expect_err("strict HTTP stepping must not cancel local UI state");
            assert!(error.contains("blocked by local UI task"));
            assert!(error.contains(&format!("{kind:?}")));
        }
    }

    #[test]
    fn default_http_policy_auto_dismisses_every_pause_side_task_kind() {
        let default_policy = crate::http_server::StepModalPolicy::default();
        assert!(default_policy.auto_dismiss);
        for kind in [
            UiTaskKind::Options,
            UiTaskKind::SaveLoad,
            UiTaskKind::QuitConfirmation,
            UiTaskKind::QuickLoadConfirmation,
        ] {
            kind.require_http_auto_dismiss(&default_policy)
                .expect("default automation policy must dismiss local UI state");
        }
    }
}
