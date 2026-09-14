//! Cooperative pause-side screens.
//!
//! Unlike the compatibility `show_*` menu helpers, these states advance at
//! most one UI frame per mission frame.  The mission driver therefore keeps
//! servicing networking, HTTP control, replay bookkeeping, and simulation
//! while a local side screen is open.

use crate::gfx_types::GameEvent;
use crate::ingame_menu::layout::{
    MenuTransform, dim_screen, draw_screen_background, enter_modal_gpu_phase, render_text_virt_font,
};
use crate::ingame_menu::resources::{
    IngameMenuResources, MT_BTN_CANCEL, MT_BTN_DELETE, MT_BTN_LOAD, MT_BTN_SAVE,
    MT_MSG_REALLY_DELETE_SAVEGAME, MT_MSG_REALLY_OVERWRITE_SAVEGAME,
};
use crate::ingame_menu::save_load::{
    ListRow, PickerAction, PickerController, PickerModel, PickerTarget, begin_picker_delete,
    edit_save_name, feed_save_name, finish_picker_delete, picker_slots, sync_input_for_selection,
};
use crate::ingame_menu::widget_bridge::ModalScreenIo;
use crate::ingame_menu::widget_bridge::{self, ModalCursor, ModalInputState};
use crate::ingame_menu::{SaveLoadMode, YesNoModalState};
use crate::key_config::KeyConfig;
#[cfg(test)]
use crate::options_model::SoundSetting;
use crate::options_model::sound_eq;
use crate::renderer::Renderer;
use crate::savegame::SaveGameManager;
use crate::sound::{AudioBackend, SoundManager};
use crate::widget::FrameWnd;
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
        let window = &mut *io.window;
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        let cursor = io.cursor;
        match self {
            Self::CampaignManager(state) => {
                state.tick_browser(window, renderer, cursor).map(|exit| {
                    if exit {
                        UiTaskOutcome::ExitRequested
                    } else {
                        UiTaskOutcome::ReturnToPause
                    }
                })
            }
            Self::Options(state) => {
                state.tick(application_context, io, sound, audio_backend, sample_loader)
            }
            Self::SaveLoad(state) => state.tick(
                io,
                save_manager,
                profiles,
                sound,
                audio_backend,
                sample_loader,
            ),
            Self::Quit(state) => {
                let (events, transform) =
                    crate::ingame_menu::layout::poll_events_with_transform(window, renderer);
                let exit_requested = events.iter().any(|event| matches!(event, GameEvent::Quit));
                let result = state.handle_events(&events, transform);
                state.render_overlay(renderer, resources, cursor);
                renderer.present();
                if exit_requested {
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
        let window = &mut *io.window;
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        let cursor = io.cursor;
        let (events, transform) =
            crate::ingame_menu::layout::poll_events_with_transform(window, renderer);
        let result = self.dialog.handle_events(&events, transform);
        if events.iter().any(|event| matches!(event, GameEvent::Quit)) {
            self.dialog.render_overlay(renderer, resources, cursor);
            renderer.present();
            return Some(UiTaskOutcome::ExitRequested);
        }
        self.dialog.render_overlay(renderer, resources, cursor);
        renderer.present();
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
        sound_manager: Option<&mut SoundManager>,
        audio_backend: Option<&mut dyn AudioBackend>,
        sample_loader: Option<&SampleLoader>,
    ) -> Option<UiTaskOutcome> {
        self.screen
            .tick(
                application_context,
                io,
                &mut widget_bridge::ScreenAudio {
                    sound: sound_manager,
                    backend: audio_backend.map(|backend| &mut *backend as &mut dyn AudioBackend),
                    sample_loader,
                },
            )
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum SaveConfirmation {
    Overwrite(crate::savegame::SlotName),
    Delete,
}

/// The pause adapter owns scheduling and layout; selection, filtering, input
/// intentions and deletion policy are shared with the standalone picker.
pub(super) struct SaveLoadTaskState {
    mode: SaveLoadMode,
    mission_id: u32,
    model: PickerModel,
    controller: PickerController,
    name: crate::widget::WidgetInputField,
    transform: MenuTransform,
    noise_tracker: widget_bridge::NoisyTracker,
    confirmation: Option<(SaveConfirmation, YesNoModalState)>,
    error_notice: Option<crate::save_recovery::ErrorNotice>,
    text_input_active: bool,
    detailed_metadata: bool,
    local_time_zone: Option<jiff::tz::TimeZone>,
    clock_error_reported: bool,
}

impl SaveLoadTaskState {
    pub(super) fn new(
        window: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        save_manager: &mut SaveGameManager,
        mission_id: u32,
        detailed_metadata: bool,
        mode: SaveLoadMode,
        multiplayer_connected: bool,
    ) -> Self {
        save_manager.sort_by_time();
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let mut input = ModalInputState::new();
        input.seed_mouse_from_window(window, transform);
        let mut state = Self::with_input(
            save_manager,
            mission_id,
            detailed_metadata,
            mode,
            multiplayer_connected,
            transform,
            input,
        );
        state.begin_frame(resources);
        state.text_input_active = mode == SaveLoadMode::Save;
        if state.text_input_active {
            crate::window::start_text_input();
        }
        state
    }

    fn with_input(
        save_manager: &SaveGameManager,
        mission_id: u32,
        detailed_metadata: bool,
        mode: SaveLoadMode,
        multiplayer_connected: bool,
        transform: MenuTransform,
        input: ModalInputState,
    ) -> Self {
        let mut name = crate::widget::WidgetInputField::new(1000);
        name.set_max_length(45);
        if mode == SaveLoadMode::Save {
            name.enter_edit_mode();
        }
        Self {
            mode,
            mission_id,
            model: PickerModel::new(
                mode,
                multiplayer_connected,
                if detailed_metadata { 7 } else { 10 },
                picker_slots(save_manager),
            ),
            controller: PickerController::new(input),
            name,
            transform,
            noise_tracker: widget_bridge::NoisyTracker::new(),
            confirmation: None,
            error_notice: None,
            // Only the window-backed constructor acquires process IME state.
            text_input_active: false,
            detailed_metadata,
            local_time_zone: jiff::tz::TimeZone::try_system()
                .inspect_err(|error| tracing::warn!("Save menu local time is unavailable: {error}"))
                .ok(),
            clock_error_reported: false,
        }
    }

    fn refresh(&mut self, manager: &SaveGameManager) {
        let selected = self.model.selected_slot().cloned();
        self.model.refresh(picker_slots(manager));
        if selected.as_ref() != self.model.selected_slot() {
            self.sync_name(manager);
        }
    }

    fn sync_name(&mut self, manager: &SaveGameManager) {
        sync_input_for_selection(
            &mut self.name,
            self.model.selected_manager_index(),
            self.mode,
            manager,
        );
    }

    fn begin_frame(&mut self, resources: &IngameMenuResources) {
        let row_height = self.row_height();
        self.controller.configure_list(
            &mut self.model,
            crate::ingame_menu::layout::MenuRect {
                x: 30,
                y: 38,
                w: 420,
                h: 372,
            },
            row_height,
            resources,
        );

        let labels = [
            resources.menu_text.get(if self.mode == SaveLoadMode::Save {
                MT_BTN_SAVE
            } else {
                MT_BTN_LOAD
            }),
            resources.menu_text.get(MT_BTN_DELETE),
            resources.menu_text.get(MT_BTN_CANCEL),
        ];
        let (w, h) = resources.button_dimensions();
        let buttons = std::array::from_fn(|index| {
            (
                index as u32,
                labels[index].as_str(),
                640 - w - 10,
                480 - (3 - index as i32) * (h + 2) - 8,
            )
        });
        self.controller.begin_frame(&self.model, &buttons, w, h);
    }

    fn handle_event(&mut self, event: &GameEvent, manager: &SaveGameManager) {
        if self
            .controller
            .handle_event(&mut self.model, event, self.transform)
        {
            self.sync_name(manager);
        }
        if self.mode == SaveLoadMode::Save {
            edit_save_name(&mut self.name, event);
        }
    }

    fn finish_input(&mut self) -> Vec<crate::ui::UiEvent> {
        let events = self.controller.process_widgets(&self.model);
        if self.mode == SaveLoadMode::Save {
            feed_save_name(
                &mut self.name,
                &self.controller.input.as_widget_input(),
                &crate::ui::UiKeyboard::default(),
            );
        }
        self.controller.input.end_frame();
        events
    }

    fn tick(
        &mut self,
        io: &mut ModalScreenIo<'_, '_>,
        save_manager: &mut SaveGameManager,
        profiles: Option<&ProfileManager>,
        sound_manager: Option<&mut SoundManager>,
        audio_backend: Option<&mut dyn AudioBackend>,
        sample_loader: Option<&SampleLoader>,
    ) -> Option<UiTaskOutcome> {
        let window = &mut *io.window;
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        let cursor = io.cursor;
        self.refresh(save_manager);
        if self.error_notice.is_none()
            && let Some(error) = self.model.operation_error()
        {
            self.error_notice = Some(crate::save_recovery::ErrorNotice::new(error.to_owned()));
        }
        if let Some(notice) = &mut self.error_notice {
            if notice.tick(&mut ModalScreenIo {
                window,
                renderer,
                resources,
                cursor,
            }) {
                self.error_notice = None;
                self.model.dismiss_error();
                if window.close_requested {
                    return Some(UiTaskOutcome::ExitRequested);
                }
            }
            return None;
        }
        let (events, transform) =
            crate::ingame_menu::layout::poll_events_with_transform(window, renderer);
        self.transform = transform;
        let exit_requested = events.iter().any(|event| matches!(event, GameEvent::Quit));
        if self.confirmation.is_some() {
            let result = self
                .confirmation
                .as_mut()
                .expect("confirmation exists")
                .1
                .handle_events(&events, transform);
            self.render(renderer, resources, None, save_manager);
            self.confirmation
                .as_mut()
                .expect("confirmation exists")
                .1
                .render_overlay(renderer, resources, cursor);
            renderer.present();
            if exit_requested {
                return Some(UiTaskOutcome::ExitRequested);
            }
            if let Some(yes) = result {
                let (action, _) = self
                    .confirmation
                    .take()
                    .expect("resolved confirmation exists");
                return self.finish_confirmation(action, yes, save_manager, profiles);
            }
            return None;
        }
        if exit_requested {
            self.render(renderer, resources, cursor, save_manager);
            renderer.present();
            return Some(UiTaskOutcome::ExitRequested);
        }
        self.begin_frame(resources);
        for event in &events {
            self.handle_event(event, save_manager);
        }
        let widget_events = self.finish_input();
        play_button_noise(
            &widget_events,
            self.controller.frame(),
            &mut self.noise_tracker,
            sound_manager,
            audio_backend,
            sample_loader,
        );
        let outcome = match self.controller.take_action() {
            Some(PickerAction::Cancel) => Some(UiTaskOutcome::ReturnToPause),
            Some(PickerAction::ConfirmDelete(name)) => {
                if begin_picker_delete(&mut self.model, name) {
                    self.confirmation = Some((
                        SaveConfirmation::Delete,
                        YesNoModalState::new(
                            window,
                            renderer,
                            resources,
                            resources.menu_text.get(MT_MSG_REALLY_DELETE_SAVEGAME),
                        ),
                    ));
                }
                None
            }
            Some(PickerAction::Accept(target)) => {
                if self.mode == SaveLoadMode::Save
                    && let PickerTarget::Existing(name) = target
                {
                    self.confirmation = Some((
                        SaveConfirmation::Overwrite(name),
                        YesNoModalState::new(
                            window,
                            renderer,
                            resources,
                            resources.menu_text.get(MT_MSG_REALLY_OVERWRITE_SAVEGAME),
                        ),
                    ));
                    None
                } else {
                    self.accept_target(target, save_manager, profiles)
                }
            }
            None => None,
        };
        self.render(renderer, resources, cursor, save_manager);
        renderer.present();
        outcome
    }

    fn finish_confirmation(
        &mut self,
        action: SaveConfirmation,
        confirmed: bool,
        manager: &mut SaveGameManager,
        profiles: Option<&ProfileManager>,
    ) -> Option<UiTaskOutcome> {
        match action {
            SaveConfirmation::Delete => {
                let selected = self.model.selected_slot().cloned();
                finish_picker_delete(&mut self.model, manager, confirmed);
                if selected.as_ref() != self.model.selected_slot() {
                    self.sync_name(manager);
                }
                None
            }
            SaveConfirmation::Overwrite(name) if confirmed => {
                self.accept_target(PickerTarget::Existing(name), manager, profiles)
            }
            SaveConfirmation::Overwrite(_) => None,
        }
    }

    fn accept_target(
        &mut self,
        target: PickerTarget,
        manager: &mut SaveGameManager,
        profiles: Option<&ProfileManager>,
    ) -> Option<UiTaskOutcome> {
        let result = (|| -> Result<String, String> {
            match target {
                PickerTarget::New => {
                    if self.mode != SaveLoadMode::Save {
                        return Err("load picker cannot create a save".into());
                    }
                    let text = if self.name.edit_text.trim().is_empty() {
                        mission_name(self.mission_id, profiles)
                            .unwrap_or_else(|| format!("Save {}", manager.count() + 1))
                    } else {
                        self.name.edit_text.trim().to_owned()
                    };
                    manager
                        .create_draft(text, self.mission_id)
                        .map(|handle| handle.name().as_str().to_owned())
                        .map_err(|error| format!("{error:#}"))
                }
                PickerTarget::Existing(name) => {
                    self.refresh(manager);
                    let slot = self
                        .model
                        .visible_slot_index(&name)
                        .ok_or_else(|| "the selected save is no longer available".to_string())?;
                    if self.mode == SaveLoadMode::Save {
                        let text = accepted_name(
                            &self.name.edit_text,
                            manager,
                            slot,
                            self.mission_id,
                            profiles,
                        );
                        let handle = manager
                            .slot_handle(slot)
                            .expect("validated overwrite slot exists");
                        manager
                            .rename_slot(&handle, text)
                            .map_err(|error| format!("{error:#}"))?;
                    }
                    Ok(name.as_str().to_owned())
                }
            }
        })();
        match result {
            Ok(filename) => Some(UiTaskOutcome::SaveLoadSelected {
                mode: self.mode,
                filename,
                mission_id: self.mission_id,
            }),
            Err(error) => {
                self.model.report_error(error);
                None
            }
        }
    }

    fn render(
        &mut self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
        save_manager: &SaveGameManager,
    ) {
        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        if let Some(background) = resources.menu_bg[3] {
            draw_screen_background(renderer, &background);
        }
        if let Some(font) = resources.title_font_any() {
            let title = if self.mode == SaveLoadMode::Save {
                "Save Game"
            } else {
                "Load Game"
            };
            render_text_virt_font(renderer, font, self.transform, title, 30, 8);
        }
        if let Some(font) = resources.label_font_any() {
            let visible = self.model.visible();
            let view = self.controller.view();
            let now_unix = if self.detailed_metadata {
                match crate::save_file::unix_timestamp_now() {
                    Ok(now) => Some(now),
                    Err(error) => {
                        if !self.clock_error_reported {
                            tracing::warn!("Save menu relative time is unavailable: {error:#}");
                            self.clock_error_reported = true;
                        }
                        None
                    }
                }
            } else {
                None
            };
            for row in view.visible_range() {
                let (selected, label, details) = if self.mode == SaveLoadMode::Save && row == 0 {
                    (
                        self.model.selected_row() == Some(ListRow::New),
                        std::borrow::Cow::Borrowed(crate::localization::port_text(
                            resources.menu_text.presentation_locale(),
                            crate::localization::PortTextKey::SaveNewSaveLabel,
                        )),
                        [
                            crate::localization::port_text(
                                resources.menu_text.presentation_locale(),
                                crate::localization::PortTextKey::SaveNewSaveHint,
                            )
                            .to_owned(),
                            String::new(),
                        ],
                    )
                } else {
                    let index = row - usize::from(self.mode == SaveLoadMode::Save);
                    let slot = visible[index];
                    let save = save_manager.get(slot).expect("visible save slot exists");
                    (
                        self.model.selected_row() == Some(ListRow::Existing(index)),
                        crate::ingame_menu::save_load::existing_save_row_label(save),
                        crate::ingame_menu::save_load::cooperative_save_row_detail_lines(
                            save,
                            self.detailed_metadata,
                            now_unix,
                            self.local_time_zone.as_ref(),
                            resources.menu_text.presentation_locale(),
                        ),
                    )
                };
                let prefix = if selected { "> " } else { "  " };
                let row_y = view.row_y(row);
                render_text_virt_font(
                    renderer,
                    font,
                    self.transform,
                    &crate::ingame_menu::layout::truncate_to_pixel_width(
                        font,
                        &format!("{prefix}{label}"),
                        view.content_width() - 20,
                        crate::ingame_menu::layout::TruncationMarker::AsciiEllipsis,
                    ),
                    40,
                    row_y,
                );
                for (line_index, detail) in
                    details.iter().filter(|line| !line.is_empty()).enumerate()
                {
                    render_text_virt_font(
                        renderer,
                        font,
                        self.transform,
                        &crate::ingame_menu::layout::truncate_to_pixel_width(
                            font,
                            detail,
                            view.content_width() - 34,
                            crate::ingame_menu::layout::TruncationMarker::AsciiEllipsis,
                        ),
                        54,
                        row_y + 16 * (line_index as i32 + 1),
                    );
                }
            }
            view.draw_scrollbar(renderer, self.transform, resources);
            if self.mode == SaveLoadMode::Save {
                render_text_virt_font(
                    renderer,
                    font,
                    self.transform,
                    &format!(
                        "Name: {}|{}",
                        self.name
                            .edit_text
                            .chars()
                            .take(self.name.caret_offset)
                            .collect::<String>(),
                        self.name
                            .edit_text
                            .chars()
                            .skip(self.name.caret_offset)
                            .collect::<String>()
                    ),
                    34,
                    438,
                );
            }
        }
        widget_bridge::draw_frame_buttons(
            renderer,
            resources,
            self.transform,
            self.controller.frame(),
        );
        if let Some(cursor) = cursor {
            cursor.draw(renderer, self.transform, &self.controller.input);
        }
    }

    fn cleanup(&mut self) {
        if self.text_input_active {
            crate::window::stop_text_input();
            self.text_input_active = false;
        }
    }

    fn row_height(&self) -> i32 {
        if self.detailed_metadata { 52 } else { 36 }
    }
}

impl Drop for SaveLoadTaskState {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn accepted_name(
    input: &str,
    save_manager: &SaveGameManager,
    slot: usize,
    mission_id: u32,
    profiles: Option<&ProfileManager>,
) -> String {
    let trimmed = input.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    let existing = save_manager.get(slot).expect("accepted save slot exists");
    if !existing.text.trim().is_empty() {
        return existing.text.clone();
    }
    mission_name(mission_id, profiles)
        .unwrap_or_else(|| format!("Save {}", save_manager.count() + 1))
}

fn mission_name(mission_id: u32, profiles: Option<&ProfileManager>) -> Option<String> {
    profiles?
        .missions
        .iter()
        .find(|mission| mission.id == mission_id)
        .map(|mission| mission.mission_name.clone())
        .filter(|name| !name.trim().is_empty())
}

fn play_button_noise(
    events: &[crate::ui::UiEvent],
    frame: &FrameWnd,
    tracker: &mut widget_bridge::NoisyTracker,
    sound_manager: Option<&mut SoundManager>,
    audio_backend: Option<&mut dyn AudioBackend>,
    sample_loader: Option<&SampleLoader>,
) {
    widget_bridge::play_frame_widget_noise(
        events,
        frame,
        widget_bridge::WIDGET_NOISY_BUTTON,
        widget_bridge::ScreenAudio {
            sound: sound_manager,
            // Re-coerce the trait object to the bundle's shorter lifetime.
            backend: audio_backend.map(|backend| &mut *backend as &mut dyn AudioBackend),
            sample_loader,
        },
        tracker,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx_types::Keycode;
    use crate::options_model::OptionsPage;
    use crate::options_model::{
        assign_shortcut as assign_key, promote_shortcut_edits, shortcut_keys as key_vec,
    };
    use crate::scroll_view::ScrollView;
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

    fn pause_picker(manager: &SaveGameManager, mode: SaveLoadMode) -> SaveLoadTaskState {
        SaveLoadTaskState::with_input(
            manager,
            1,
            false,
            mode,
            false,
            MenuTransform::centered(640, 480),
            ModalInputState::new(),
        )
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "requires game data and an offscreen wgpu adapter"]
    fn capture_save_scroll_view() {
        // The workspace root is the install root holding assets/core-datadir.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let data = robin_test_support::original_data::data_directory("");
        let (_, _, context) = crate::main_entry::rust_init_with_roots(Some(&data), Some(root))
            .expect("capture content");
        let gpu = pollster::block_on(async {
            let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
            descriptor.backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::VULKAN);
            let instance = std::sync::Arc::new(wgpu::Instance::new(descriptor));
            let options = wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            };
            let adapter = instance
                .request_adapter(&options)
                .await
                .expect("offscreen adapter");
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("save UI capture"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                        .using_resolution(adapter.limits()),
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    trace: wgpu::Trace::Off,
                })
                .await
                .expect("offscreen device");
            crate::window::GpuContext {
                instance,
                adapter: std::sync::Arc::new(adapter),
                device: std::sync::Arc::new(device),
                queue: std::sync::Arc::new(queue),
                surface_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            }
        });

        let mut renderer = Renderer::offscreen(gpu, 1024, 768);
        let resources = IngameMenuResources::new(
            &mut renderer,
            context.shipping().unwrap(),
            context.preparation_files().unwrap().clone(),
        )
        .expect("menu resources");
        let mut manager = SaveGameManager::new("unused-capture-saves".into());
        for index in 0..30 {
            let mut save = crate::savegame::SaveGame::new(
                format!("Savegame_{index:03}"),
                format!("Save {index:02}"),
                1,
            );
            save.timestamp = "1788940800".into();
            save.mission_name = "The Silver Arrow".into();
            save.player_profile_id = Some(12);
            save.player_name = "Robin".into();
            save.mission_elapsed_seconds = Some(65 * 60 + index);
            manager.insert_test_slot(save, crate::savegame::SlotState::Published);
        }
        let output = std::path::Path::new("target/save-ui");
        std::fs::create_dir_all(output).unwrap();
        for detailed in [false, true] {
            let mut state = SaveLoadTaskState::with_input(
                &manager,
                1,
                detailed,
                SaveLoadMode::Load,
                false,
                MenuTransform::centered(1024, 768),
                ModalInputState::new(),
            );
            state.begin_frame(&resources);
            for _ in 0..30 {
                state.model.navigate(true);
            }
            state.begin_frame(&resources);
            renderer.begin_gpu_frame_clear();
            renderer.begin_ui_only_frame();
            state.render(&mut renderer, &resources, None, &manager);
            let (width, height, pixels) = renderer.try_capture_frame_rgba().unwrap();
            let path = output.join(if detailed {
                "detailed.png"
            } else {
                "compact.png"
            });
            let mut encoder =
                png::Encoder::new(std::fs::File::create(&path).unwrap(), width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
            eprintln!("Captured {}", path.display());
        }
    }

    fn picker_fixture(manager: &mut SaveGameManager, name: &str) {
        let mut save = crate::savegame::SaveGame::new(name.into(), name.into(), 1);
        save.timestamp = "123".into();
        save.mission_name = "The Silver Arrow".into();
        save.player_profile_id = Some(12);
        save.player_name = "Alice".into();
        save.campaign_progress = Some(0);
        save.missions_done = Some(0);
        save.missions_total = Some(1);
        save.gang_size = Some(1);
        save.ransom = Some(0);
        save.blazons = Some(0);
        save.amulets = Some(0);
        save.validate_published_metadata().unwrap();
        manager.insert_test_slot(save, crate::savegame::SlotState::Published);
    }

    fn picker_event_frame(
        state: &mut SaveLoadTaskState,
        manager: &SaveGameManager,
        events: &[GameEvent],
    ) -> Option<PickerAction> {
        state.controller.scroll_view.get_or_insert_with(|| {
            ScrollView::with_geometry([30, 42, 420, 360], 36, 16, 16, false)
        });
        state.refresh(manager);
        state.controller.begin_frame(
            &state.model,
            &[
                (0, "Accept", 460, 300),
                (1, "Delete", 460, 350),
                (2, "Cancel", 460, 400),
            ],
            150,
            40,
        );
        for event in events {
            state.handle_event(event, manager);
        }
        state.finish_input();
        state.controller.take_action()
    }

    fn picker_key(keycode: Keycode) -> GameEvent {
        GameEvent::KeyDown {
            keycode,
            physical_key: None,
        }
    }

    #[test]
    fn pause_picker_uses_shared_draft_autosave_and_multiplayer_policy() {
        let mut manager = SaveGameManager::new("unused-pause-picker-policy".into());
        picker_fixture(&mut manager, "Autosave_100_0000");
        picker_fixture(&mut manager, "Savegame_001");
        manager.create_draft("Draft".into(), 1).unwrap();
        let mut pause = pause_picker(&manager, SaveLoadMode::Load);
        assert_eq!(
            pause.model.visible().len(),
            2,
            "drafts never appear in load picker"
        );
        picker_event_frame(&mut pause, &manager, &[picker_key(Keycode::Down)]);
        assert!(!pause.model.can_delete());
        assert!(pause.model.request_delete().is_none());
        for event in [
            GameEvent::MouseMove {
                x: 480,
                y: 360,
                xrel: 0,
                yrel: 0,
            },
            GameEvent::MouseDown(480, 360, 1, 1),
            GameEvent::MouseUp(480, 360, 1),
        ] {
            assert_eq!(
                picker_event_frame(&mut pause, &manager, &[event]),
                None,
                "pause delete button must not offer protected autosave deletion"
            );
        }
        assert_eq!(
            pause.model.selected_slot().unwrap().as_str(),
            "Autosave_100_0000"
        );
        let manual = manager.find_by_filename("Savegame_001").unwrap();
        manager.get_mut(manual).unwrap().multiplayer_diagnostic = true;
        let connected = SaveLoadTaskState::with_input(
            &manager,
            1,
            false,
            SaveLoadMode::Load,
            true,
            MenuTransform::centered(640, 480),
            ModalInputState::new(),
        );
        assert_eq!(connected.model.visible().len(), 1);
        let save = pause_picker(&manager, SaveLoadMode::Save);
        assert_eq!(save.model.selected_row(), Some(ListRow::New));
        assert!(
            save.model
                .visible()
                .iter()
                .all(|index| !manager.get(*index).unwrap().is_special())
        );
    }

    #[test]
    fn actual_pause_input_matches_standalone_actions_and_edits_non_ascii_at_caret() {
        let manager = SaveGameManager::new("unused-pause-picker-input".into());
        let mut pause = pause_picker(&manager, SaveLoadMode::Save);
        let mut standalone =
            PickerModel::new(SaveLoadMode::Save, false, 10, picker_slots(&manager));
        let mut controller = PickerController::new(ModalInputState::new());
        controller.scroll_view = Some(ScrollView::with_geometry(
            [30, 42, 420, 360],
            36,
            16,
            16,
            false,
        ));
        let traces = [
            vec![GameEvent::TextInput {
                text: "é雪".into()
            }],
            vec![
                picker_key(Keycode::Left),
                GameEvent::TextInput { text: "Ω".into() },
            ],
            vec![picker_key(Keycode::Backspace)],
            vec![picker_key(Keycode::Delete)],
            vec![picker_key(Keycode::Return)],
            vec![picker_key(Keycode::Escape)],
        ];
        for events in traces {
            let action = picker_event_frame(&mut pause, &manager, &events);
            controller.begin_frame(
                &standalone,
                &[
                    (0, "Save", 460, 300),
                    (1, "Delete", 460, 350),
                    (2, "Cancel", 460, 400),
                ],
                150,
                40,
            );
            for event in &events {
                controller.handle_event(&mut standalone, event, MenuTransform::centered(640, 480));
            }
            controller.process_widgets(&standalone);
            controller.input.end_frame();
            assert_eq!(action, controller.take_action());
            assert_eq!(pause.model, standalone);
        }
        assert_eq!(pause.name.edit_text, "é");
        assert_eq!(pause.name.caret_offset, 1);
        assert_eq!(pause.name.base.state, crate::ui::UiState::SelectedEditable);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pause_confirmation_handles_changed_catalog_and_surfaces_delete_failure() {
        let directory = tempfile::tempdir().unwrap();
        let mut manager = SaveGameManager::new(directory.path().to_str().unwrap().into());
        picker_fixture(&mut manager, "Savegame_000");
        picker_fixture(&mut manager, "Savegame_001");
        manager.save_index().unwrap();
        let mut pause = pause_picker(&manager, SaveLoadMode::Load);
        picker_event_frame(&mut pause, &manager, &[picker_key(Keycode::Down)]);
        let name = pause.model.selected_slot().unwrap().clone();
        assert!(begin_picker_delete(&mut pause.model, name.clone()));
        manager.remove_by_filename(name.as_str()).unwrap();
        pause.finish_confirmation(SaveConfirmation::Delete, true, &mut manager, None);
        assert!(
            pause
                .model
                .operation_error()
                .unwrap()
                .contains("no longer available")
        );
        assert_eq!(
            manager.count(),
            1,
            "a changed catalog must not redirect deletion"
        );
        pause.model.dismiss_error();
        picker_event_frame(&mut pause, &manager, &[picker_key(Keycode::Down)]);
        let name = pause.model.selected_slot().unwrap().clone();
        assert!(begin_picker_delete(&mut pause.model, name));
        std::fs::create_dir(directory.path().join("Savegame_001.json")).unwrap();
        pause.finish_confirmation(SaveConfirmation::Delete, true, &mut manager, None);
        assert_eq!(
            manager.count(),
            0,
            "published removal is not rolled back after cleanup failure"
        );
        assert!(pause.model.operation_error().unwrap().contains("cleanup"));
        assert_eq!(pause.model.selected_row(), None);
    }

    #[test]
    fn pause_overwrite_confirmation_revalidates_identity_after_catalog_change() {
        let mut manager = SaveGameManager::new("unused-pause-overwrite".into());
        picker_fixture(&mut manager, "Savegame_000");
        let mut pause = pause_picker(&manager, SaveLoadMode::Save);
        picker_event_frame(&mut pause, &manager, &[picker_key(Keycode::Down)]);
        let name = pause.model.selected_slot().unwrap().clone();
        pause.name.set_text("Edited but not saved");
        assert!(begin_picker_delete(&mut pause.model, name.clone()));
        assert!(
            pause
                .finish_confirmation(SaveConfirmation::Delete, false, &mut manager, None)
                .is_none()
        );
        assert_eq!(
            pause.name.edit_text, "Edited but not saved",
            "cancelling deletion retains pending name edits"
        );
        // Switching to a fresh catalog simulates disappearance while the
        // confirmation's stable identity remains alive.
        let mut replacement = SaveGameManager::new("unused-pause-overwrite".into());
        picker_fixture(&mut replacement, "Savegame_001");
        assert!(
            pause
                .finish_confirmation(
                    SaveConfirmation::Overwrite(name),
                    true,
                    &mut replacement,
                    None
                )
                .is_none()
        );
        assert!(
            pause
                .model
                .operation_error()
                .unwrap()
                .contains("no longer available")
        );
        assert_eq!(replacement.get(0).unwrap().text, "Savegame_001");
    }

    #[test]
    fn pause_existing_slot_actions_match_standalone_controller() {
        let mut manager = SaveGameManager::new("unused-pause-action-trace".into());
        picker_fixture(&mut manager, "Savegame_000");
        picker_fixture(&mut manager, "Savegame_001");
        let mut pause = pause_picker(&manager, SaveLoadMode::Load);
        let mut model = pause.model.clone();
        let mut controller = PickerController::new(ModalInputState::new());
        controller.scroll_view = Some(ScrollView::with_geometry(
            [30, 42, 420, 360],
            36,
            16,
            16,
            false,
        ));
        let mut actions = Vec::new();
        for events in [
            vec![
                picker_key(Keycode::Down),
                picker_key(Keycode::Down),
                picker_key(Keycode::Return),
            ],
            vec![GameEvent::MouseMove {
                x: 480,
                y: 360,
                xrel: 0,
                yrel: 0,
            }],
            vec![GameEvent::MouseDown(480, 360, 1, 1)],
            vec![GameEvent::MouseUp(480, 360, 1)],
            vec![picker_key(Keycode::Escape)],
        ] {
            let action = picker_event_frame(&mut pause, &manager, &events);
            controller.begin_frame(
                &model,
                &[
                    (0, "Load", 460, 300),
                    (1, "Delete", 460, 350),
                    (2, "Cancel", 460, 400),
                ],
                150,
                40,
            );
            for event in &events {
                controller.handle_event(&mut model, event, MenuTransform::centered(640, 480));
            }
            controller.process_widgets(&model);
            controller.input.end_frame();
            assert_eq!(action, controller.take_action());
            assert_eq!(pause.model, model);
            if let Some(action) = action {
                actions.push(action);
            }
        }
        let name = crate::savegame::SlotName::new("Savegame_001").unwrap();
        assert_eq!(
            actions,
            vec![
                PickerAction::Accept(PickerTarget::Existing(name.clone())),
                PickerAction::ConfirmDelete(name),
                PickerAction::Cancel
            ]
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
