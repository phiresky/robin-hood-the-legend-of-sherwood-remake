//! Game-flow callbacks, save/load plumbing, and mission-launch helpers.

use crate::app_effect::{AppEffect, AppEffectExecutionError, AppEffectQueue};
use crate::autosave::{AutosaveCoordinator, AutosavePollResult, AutosaveReason};
use crate::host::ApplicationContext;
use crate::leaderboard_mission_end::{
    MissionEndLeaderboardBackground, MissionEndLeaderboardController, MissionSubmissionState,
};
use crate::renderer::Renderer;
use crate::save_file::special_slots;
use crate::savegame::{SaveGameManager, SlotHandle, SpecialSlot};
use crate::sound::{Jingle as SoundJingle, SoundMode as AudioSoundMode};
use robin_assets::picture::Picture;
use robin_engine::campaign as engine_campaign;
use robin_engine::campaign::Campaign;
use robin_engine::engine as engine_api;
use robin_engine::game_operation::GameCode;
use robin_engine::profiles as engine_profiles;
use robin_engine::profiles::{MissionLocation, ProfileManager};

use super::cli::CliArgs;

/// Real implementation of [`GameCallbacks`](crate::game::GameCallbacks)
/// for the pure-Rust path.  Owns the [`SaveGameManager`] and serves as
/// the integration point between the Game state machine and persistent
/// storage.
///
/// Non-save callbacks enqueue host effects or drive the game-flow state machine.
///
/// ### Save/load semantics
///
/// The callback trait passes the `Campaign` but not the `Engine`, so
/// `request_save` / `request_load` only queue an intent here.
/// The actual file I/O — which needs live engine access — is performed
/// by [`crate::game_session::perform_pending_save_load`] before the next
/// engine tick, using [`crate::save_file::GameSaveFile`].
pub(crate) struct RustCallbacks {
    #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
    pub(crate) multiplayer_campaign: crate::multiplayer::MultiplayerCampaignSession,
    application_context: ApplicationContext,
    /// Save-slot metadata manager, persists slot list as `saves.json`.
    pub save_manager: SaveGameManager,
    autosave: AutosaveCoordinator,
    /// Pending save/load request queued by the state machine, handled
    /// before the next engine tick in `game_session`.
    pending: Option<OperationRequest>,
    /// Ordered game-flow effects awaiting the host executor. A FIFO is
    /// required because one transition can request Menu and then Mission
    /// sound in the same frame; neither request may overwrite the other.
    pub app_effects: AppEffectQueue,
    /// Application-lifetime presentation mailbox, distinct from operation I/O.
    /// Kept across mission teardown because autosaves finish asynchronously.
    pub(crate) autosave_notices: AutosaveNotices,

    /// Consented leaderboard authorization/uploads survive mission teardown and
    /// are cooperatively polled by the next session frame.
    leaderboard_background: MissionEndLeaderboardBackground,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct PendingMultiplayerCampaignExit {
    pub not_before_frame: u32,
    pub mission_id: u32,
}

/// Save-slot bookkeeping passed from an in-mission "load" click through
/// the `GameCode::LevelLoad` exit back into the outer session loop.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PendingLevelLoad {
    /// Runtime-owned slot, absent only for a committed remote snapshot.
    pub slot: Option<SlotHandle>,
    /// Mission profile ID the save's header reports.
    pub target_mission_id: u32,
    #[serde(skip)]
    pub(crate) origin: OperationOrigin,
    /// Already-decoded payload. It is moved into the destination load request
    /// so the bytes preflighted before Engine construction are the bytes applied.
    pub save: crate::save_file::GameSaveFile,
}

impl PendingLevelLoad {
    pub(crate) fn validate_slot(&self, manager: &SaveGameManager) -> anyhow::Result<()> {
        match &self.slot {
            Some(handle) => {
                manager.resolve_handle(handle)?;
            }
            None => anyhow::ensure!(
                self.origin == OperationOrigin::CommittedMultiplayer,
                "local level load is missing its save slot handle"
            ),
        }
        Ok(())
    }
}

impl std::fmt::Debug for PendingLevelLoad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingLevelLoad")
            .field("slot", &self.slot)
            .field("target_mission_id", &self.target_mission_id)
            .finish_non_exhaustive()
    }
}

/// Slot-type-dependent post-load state the frame loop must apply once
/// the engine has been loaded. Thread the slot type through so we can
/// replay the continue / campaign-map fix-ups without the save-I/O layer
/// needing a `&mut Game` handle.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct PostLoadSync {
    /// True when the loaded slot is the Continue auto-save.
    pub is_continue: bool,
}

/// Which banner to show after a save/load succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SaveBannerKind {
    Saved,
    Loaded,
    Autosaved,
    AutosaveFailed,
    SaveFailed,
}

/// Pending save/load intent set by the state machine and consumed
/// outside the callback boundary by `game_session`.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum SaveLoadRequest {
    /// Persist the current engine state to the caller-provided slot.
    /// `None` slot = write the Continue auto-save.
    Save {
        slot: Option<SlotHandle>,
        mission_id: u32,
    },
    /// Load a save and apply it to the engine.
    /// `None` slot = load the Continue auto-save.
    ///
    /// `mission_id` records the mission expected by the request producer.
    /// Apply-time validation derives the active mission from the live Engine,
    /// validates the decoded header against its campaign, and routes a valid
    /// cross-mission payload through the session reload boundary.
    Load {
        slot: Option<SlotHandle>,
        mission_id: u32,
        /// Preflighted payload for initial/cross-mission loads. `None` only
        /// before the request reaches its preconstruction boundary.
        save: Option<crate::save_file::GameSaveFile>,
    },
    /// Write the Restart auto-save (pre-restart snapshot), called once
    /// per mission right after level init finishes. The live campaign
    /// supplies the required mission ID when the request is processed.
    Restart,
    /// Apply the Restart auto-save to the engine, restoring the
    /// pristine post-level-init state without reloading the level.
    /// Called when the op code transitions to a level-restart
    /// (typically from a script command).
    LoadRestart,
    /// Write the Continue auto-save (quit / end-of-mission flow).
    Continue { mission_id: u32 },
    /// Write the QuickSave auto-save (F5 hotkey).
    /// Rotates the previous QuickSave to ExQuickSave and then writes
    /// the fresh snapshot.
    QuickSave { mission_id: u32 },
    /// Load the QuickSave auto-save (F12 hotkey).
    ///
    /// `use_backup` selects the previous (ExQuickSave) slot when `Shift`
    /// is held at keypress time. Without Shift, loads the newest
    /// QuickSave.
    QuickLoad { use_backup: bool },
    /// Write the Sherwood checkpoint save (transition out of Sherwood map).
    Sherwood { mission_id: u32 },
}

/// Provenance travels with a request; an unrelated later load cannot inherit
/// a previously committed multiplayer barrier's authority.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum OperationOrigin {
    #[default]
    Local,
    CommittedMultiplayer,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct OperationRequest {
    action: SaveLoadRequest,
    #[serde(skip)]
    origin: OperationOrigin,
}

/// Which replay-timeline-relevant effect a flushed save/load request had.
///
/// Consumed by `TimelineRuntime::note_save_load_event`, which turns saves
/// into replay save markers and same-session loads into linear load-back
/// records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum SaveLoadEvent {
    /// A save payload capturing the current engine state was written.
    SaveWritten {
        identity: crate::save_file::ReplaySaveIdentity,
    },
    /// A save payload was applied to the live engine, replacing its state.
    LoadApplied {
        /// Identity computed from the decoded payload before post-load fixups
        /// mutate the live engine.
        identity: crate::save_file::ReplaySaveIdentity,
        /// Whether the decoded payload came from the Continue auto-save.
        is_continue: bool,
    },
}

/// Explicit presentation reducer for asynchronous completion results. The
/// existing policy is latest autosave completion wins; a manual-operation
/// banner takes this frame without consuming the pending autosave notice.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct AutosaveNotices {
    pending: Option<SaveBannerKind>,
}

impl AutosaveNotices {
    pub(crate) fn observe(&mut self, results: impl IntoIterator<Item = AutosavePollResult>) {
        for result in results {
            self.pending = Some(match result {
                AutosavePollResult::Saved { .. } => SaveBannerKind::Autosaved,
                AutosavePollResult::Failed { .. } => SaveBannerKind::AutosaveFailed,
            });
        }
    }

    pub(crate) fn enqueue_failed(&mut self) {
        self.pending = Some(SaveBannerKind::AutosaveFailed);
    }

    pub(crate) fn enqueue_save_failed(&mut self, details: String) {
        tracing::error!("Save operation failed: {details}");
        self.pending = Some(SaveBannerKind::SaveFailed);
    }

    pub(crate) fn select_banner(
        &mut self,
        manual: Option<SaveBannerKind>,
    ) -> Option<SaveBannerKind> {
        manual.or_else(|| self.pending.take())
    }
}

/// One executor result consumed by the frame that requested the operation.
/// Restore synchronization and frontend cleanup cannot leak through mutable
/// callback flags into a later frame. Autosave notices remain independently
/// queued because autosaves do not run through this executor.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct OperationOutcome {
    /// A request was present and handled (successfully or not).
    pub processed: bool,
    /// The state-affecting event that actually completed, if any.
    pub event: Option<SaveLoadEvent>,
    pub restore: Option<PostLoadSync>,
    pub reset_input: bool,
    pub banner: Option<SaveBannerKind>,
    pub restart_requested: bool,
    pub transition: Option<PendingLevelLoad>,
}

impl OperationOutcome {
    const NOT_PENDING: Self = Self {
        processed: false,
        transition: None,
        event: None,
        restore: None,
        reset_input: false,
        banner: None,
        restart_requested: false,
    };
    const NO_EVENT: Self = Self {
        processed: true,
        transition: None,
        event: None,
        restore: None,
        reset_input: false,
        banner: None,
        restart_requested: false,
    };
}

impl RustCallbacks {
    /// A host iteration has one operation slot. The last request before the
    /// flush replaces an earlier request, preserving the existing UI/script
    /// precedence; these are choices, not a FIFO of disk writes.
    pub(crate) fn queue_operation(&mut self, request: SaveLoadRequest) {
        self.pending = Some(OperationRequest {
            action: request,
            origin: OperationOrigin::Local,
        });
    }

    pub(crate) fn queue_committed_load(&mut self, request: SaveLoadRequest) {
        assert!(
            matches!(request, SaveLoadRequest::Load { save: Some(_), .. }),
            "only a decoded load may carry committed multiplayer authority"
        );
        self.pending = Some(OperationRequest {
            action: request,
            origin: OperationOrigin::CommittedMultiplayer,
        });
    }

    pub(crate) fn pending_request(&self) -> Option<&SaveLoadRequest> {
        self.pending.as_ref().map(|request| &request.action)
    }

    pub(crate) fn clear_operation(&mut self) {
        self.pending = None;
    }

    pub(crate) fn take_initial_request(&mut self) -> Option<SaveLoadRequest> {
        self.pending.take().map(|request| {
            assert_eq!(
                request.origin,
                OperationOrigin::Local,
                "initial menu load cannot consume a committed in-session transition"
            );
            request.action
        })
    }

    pub fn new(
        application_context: ApplicationContext,
    ) -> Result<Self, crate::save_recovery::SaveStoreOpenError> {
        let save_manager = crate::save_recovery::try_open(&application_context)?;
        Ok(Self::with_save_manager(application_context, save_manager))
    }

    pub(crate) async fn new_for_window(
        application_context: ApplicationContext,
        window: &mut crate::window::GameWindow,
    ) -> Result<Option<Self>, String> {
        match crate::save_recovery::open_for_launch(&application_context, window).await? {
            crate::save_recovery::OpenedSaveStore::Ready(manager) => {
                Ok(Some(Self::with_save_manager(application_context, manager)))
            }
            crate::save_recovery::OpenedSaveStore::Cancelled
            | crate::save_recovery::OpenedSaveStore::ExitRequested => Ok(None),
        }
    }

    fn with_save_manager(
        application_context: ApplicationContext,
        save_manager: SaveGameManager,
    ) -> Self {
        Self {
            #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
            multiplayer_campaign: crate::multiplayer::MultiplayerCampaignSession::default(),
            application_context,
            save_manager,
            autosave: AutosaveCoordinator::default(),
            pending: None,
            app_effects: AppEffectQueue::default(),
            leaderboard_background: MissionEndLeaderboardBackground::default(),
            autosave_notices: AutosaveNotices::default(),
        }
    }

    pub(crate) fn detach_leaderboard_submission(
        &mut self,
        controller: MissionEndLeaderboardController,
    ) {
        self.leaderboard_background.adopt(controller);
    }

    pub(crate) fn poll_leaderboard_submissions(&mut self) {
        self.leaderboard_background.poll(&self.application_context);
        self.application_context.poll_leaderboard_receipts();
    }

    pub(crate) fn application_context(&self) -> ApplicationContext {
        self.application_context.clone()
    }

    #[allow(dead_code)]
    pub(crate) fn take_leaderboard_submission_notice(&mut self) -> Option<MissionSubmissionState> {
        self.leaderboard_background.take_completed()
    }

    pub(crate) fn autosave_enabled(&self) -> bool {
        self.application_context
            .active_profile_snapshot()
            .unwrap_or_else(|error| panic!("autosave requires an active profile: {error}"))
            .gameplay_config
            .autosave_enabled
    }

    pub(crate) fn plan_autosave(
        &mut self,
        enabled_and_allowed: bool,
        snapshot_available: bool,
        mission_id: u32,
        frame: u64,
        backgrounded: bool,
        terminal_transition: bool,
    ) -> Option<AutosaveReason> {
        self.autosave.plan(
            enabled_and_allowed,
            snapshot_available,
            mission_id,
            frame,
            backgrounded,
            terminal_transition,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enqueue_autosave(
        &mut self,
        host: &crate::host::Host,
        game: &crate::game::Game,
        engine: &engine_api::Engine,
        mission_id: u32,
        profiles: &engine_profiles::ProfileManager,
        thumbnail: Option<crate::save_file::Thumbnail>,
        reason: AutosaveReason,
    ) -> Result<(), String> {
        self.autosave
            .enqueue(
                &self.save_manager,
                host,
                game,
                engine,
                mission_id,
                profiles,
                thumbnail,
                reason,
            )
            .map_err(|error| format!("{error:#}"))
    }

    #[cfg(target_arch = "wasm32")]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enqueue_initial_autosave_with_thumbnail(
        &mut self,
        host: &crate::host::Host,
        game: &crate::game::Game,
        engine: &engine_api::Engine,
        mission_id: u32,
        profiles: &engine_profiles::ProfileManager,
        thumbnail: std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<crate::save_file::Thumbnail>>>,
        >,
    ) -> Result<(), String> {
        self.autosave
            .enqueue_initial_with_thumbnail(
                &self.save_manager,
                host,
                game,
                engine,
                mission_id,
                profiles,
                thumbnail,
            )
            .map_err(|error| format!("{error:#}"))
    }

    pub(crate) fn poll_autosaves(&mut self) -> Vec<AutosavePollResult> {
        if let Err(error) = self.save_manager.poll_background() {
            self.autosave_notices
                .enqueue_save_failed(format!("{error:#}"));
        }
        self.autosave.poll(&mut self.save_manager)
    }

    /// Retire every accepted filesystem operation before returning to a menu,
    /// changing profiles, or reporting session completion. Always drain both
    /// owners even when one reports failure.
    pub(crate) fn finish_save_operations(&mut self) -> Result<(), String> {
        let mut errors = Vec::new();
        if let Err(error) = self.save_manager.finish_background() {
            let error = format!("{error:#}");
            self.autosave_notices.enqueue_save_failed(error.clone());
            errors.push(error);
        }
        for result in self.autosave.shutdown_and_poll(&mut self.save_manager) {
            if let AutosavePollResult::Failed {
                filename, error, ..
            } = result
            {
                self.autosave_notices.enqueue_failed();
                errors.push(format!(
                    "autosave {filename} failed during retirement: {error}"
                ));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

impl Drop for RustCallbacks {
    fn drop(&mut self) {
        let results = self.autosave.shutdown_and_poll(&mut self.save_manager);
        for result in results {
            match result {
                AutosavePollResult::Saved { filename, reason } => {
                    tracing::info!(filename, ?reason, "Autosave completed during shutdown");
                }
                AutosavePollResult::Failed {
                    filename,
                    reason,
                    error,
                } => {
                    tracing::error!(
                        filename,
                        ?reason,
                        "Autosave failed during shutdown: {error}"
                    );
                }
            }
        }
    }
}

impl SaveLoadRequest {
    /// Whether this request writes a save payload and should get a fresh
    /// thumbnail from a rendered frame.
    pub(crate) fn writes_save_payload(&self) -> bool {
        matches!(
            self,
            SaveLoadRequest::Save { .. }
                | SaveLoadRequest::Restart
                | SaveLoadRequest::Continue { .. }
                | SaveLoadRequest::QuickSave { .. }
                | SaveLoadRequest::Sherwood { .. }
        )
    }
}

impl crate::game::GameCallbacks for RustCallbacks {
    fn request_save(&mut self, campaign: &Campaign, profiles: &engine_profiles::ProfileManager) {
        let mission_id = current_mission_id(campaign, profiles);
        self.queue_operation(SaveLoadRequest::Save {
            slot: None,
            mission_id,
        });
    }
    fn request_load(&mut self, mission_id: u32) {
        self.queue_operation(SaveLoadRequest::Load {
            slot: None,
            mission_id,
            save: None,
        });
    }
    fn request_restart_snapshot(&mut self, write: bool) {
        self.queue_operation(if write {
            SaveLoadRequest::Restart
        } else {
            SaveLoadRequest::LoadRestart
        });
    }
    fn request_continue_save(&mut self, mission_id: u32) {
        self.queue_operation(SaveLoadRequest::Continue { mission_id });
    }
    fn save_profiles(&mut self) {
        match self
            .application_context
            .with_player_profiles_mut(|mgr| self.application_context.persist_player_profiles(mgr))
        {
            Ok(Ok(())) => {}
            Ok(Err(err)) => tracing::error!("save_profiles failed: {err}"),
            Err(error) => panic!("save_profiles lost its ApplicationContext: {error}"),
        }
    }
    fn synchronize_profile_with_campaign(
        &mut self,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
    ) {
        let mission_secs = if campaign.latest_mission_attempt().is_some_and(|attempt| {
            attempt.kind() == robin_engine::campaign_history::MissionAttemptKind::HistoryReplay
        }) {
            0
        } else {
            self.get_current_playing_time(campaign)
        };
        let persistence = self
            .application_context
            .with_player_profiles_mut(|manager| {
                let added = {
                    let profile = manager
                        .get_active_mut()
                        .expect("ApplicationContext lost its required active player profile");
                    profile.score =
                        campaign.get_value(engine_campaign::CampaignValue::Score) as u32;
                    profile.ransom =
                        campaign.get_value(engine_campaign::CampaignValue::Ransom) as u32;
                    profile.progression = campaign.get_progression(profiles);
                    profile.play_time += mission_secs;
                    let achievements_before = profile.earned_achievements();
                    let added = profile
                        .promote_campaign_history(campaign, profiles)
                        .unwrap_or_else(|error| {
                            panic!("cannot promote campaign attempt into profile history: {error}")
                        });
                    let _newly_earned = profile
                        .earned_achievements()
                        .difference(achievements_before);

                    let dead =
                        campaign.get_value(engine_campaign::CampaignValue::DeadSoldiers) as u32;
                    let alive =
                        campaign.get_value(engine_campaign::CampaignValue::LivingSoldiers) as u32;
                    profile.preserved_lives = if dead != 0 || alive != 0 {
                        (100.0 * alive as f32 / (dead + alive) as f32) as u32
                    } else {
                        0
                    };
                    added
                };
                if added != 0 {
                    self.application_context.persist_player_profiles(manager)?;
                }
                Ok::<(), std::io::Error>(())
            });
        match persistence {
            Ok(Ok(())) => {}
            Ok(Err(error)) => panic!("cannot persist promoted campaign history: {error}"),
            Err(error) => {
                panic!("profile synchronization lost its ApplicationContext: {error}")
            }
        }
    }
    fn save_game_file_exists(&self) -> bool {
        self.save_manager
            .find_by_filename(special_slots::CONTINUE)
            .map(|idx| self.save_manager.slot_file_exists(idx))
            .unwrap_or(false)
    }
    fn save_game_mission_id(&self) -> u32 {
        let idx = self
            .save_manager
            .find_by_filename(special_slots::CONTINUE)
            .expect("save_game_mission_id: Continue slot must exist after file-exists check");
        required_mission_id(
            self.save_manager.slot_mission_id(idx),
            "save_game_mission_id: Continue slot must have a cached mission ID",
        )
    }
    fn emit_app_effect(&mut self, effect: AppEffect) {
        self.app_effects.push(effect);
    }
    fn send_script_message(&mut self, target: u32, message: u32) {
        tracing::debug!("Script message: target={} msg={}", target, message);
    }
    fn display_ingame_menu(&mut self) {
        tracing::warn!("display_ingame_menu: stub");
    }
    fn display_debriefing(&mut self, won: bool) {
        tracing::info!("Debriefing (won={}): stub", won);
    }
    fn debriefing_load_request(&self) -> Option<GameCode> {
        // The cooperative terminal flow queues its concrete request directly.
        // This synchronous compatibility callback therefore has no result.
        None
    }
    fn get_current_playing_time(&self, campaign: &Campaign) -> u32 {
        // Returns deterministic simulation seconds. Downstream consumers
        // (debriefing mission-length, profile `play_time` sync) all see
        // the campaign-owned counter.
        campaign
            .get_value(engine_campaign::CampaignValue::MissionLength)
            .max(0) as u32
    }
}

/// Require a real mission ID from state that has already been established.
///
/// The original game directly
/// dereference the campaign's current mission/profile when comparing or
/// writing saves; zero is not substituted for missing required state.
pub(crate) fn required_mission_id(mission_id: Option<u32>, context: &str) -> u32 {
    let mission_id = mission_id.unwrap_or_else(|| panic!("{context}"));
    assert_ne!(mission_id, 0, "{context}: mission ID zero is invalid");
    mission_id
}

/// Resolve the required mission profile ID of the campaign's current mission.
pub(crate) fn current_mission_id(
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
) -> u32 {
    required_mission_id(
        campaign
            .current_mission_idx
            .and_then(|idx| campaign.missions.get(idx))
            .map(|m| m.profile(profiles).id),
        "current_mission_id: campaign must have a valid current mission",
    )
}

/// Validate the mission identity embedded redundantly in a save header and
/// its authoritative campaign snapshot before any destination Engine exists.
pub(crate) fn validate_save_mission(
    save: &crate::save_file::GameSaveFile,
    profiles: &engine_profiles::ProfileManager,
) -> Result<usize, String> {
    let mission_id = save.header.mission_id;
    if mission_id == 0 {
        return Err("save header mission ID zero is invalid".to_string());
    }
    let campaign = save.engine.campaign();
    let mut mission_idx = None;
    for (index, mission) in campaign.missions.iter().enumerate() {
        let profile_idx = mission
            .profile_idx
            .ok_or_else(|| format!("save campaign mission at index {index} has no profile_idx"))?
            as usize;
        let profile = profiles.missions.get(profile_idx).ok_or_else(|| {
            format!(
                "save campaign mission at index {index} references out-of-range profile_idx {profile_idx}"
            )
        })?;
        if profile.id == mission_id && mission_idx.is_none() {
            mission_idx = Some(index);
        }
    }
    let mission_idx = mission_idx
        .ok_or_else(|| format!("save mission id {mission_id} is absent from its campaign"))?;
    if campaign.current_mission_idx != Some(mission_idx) {
        return Err(format!(
            "save campaign current mission {:?} does not match header mission id {mission_id} at index {mission_idx}",
            campaign.current_mission_idx,
        ));
    }
    if !campaign.has_restart_simulation_checkpoint() {
        return Err("save campaign is missing its mission restart checkpoint".to_string());
    }
    Ok(mission_idx)
}

/// Validate the decoded current schema and decide whether its immutable level
/// assets match the active mission. `Some(id)` means the
/// payload is valid but must be routed through a mission reload before apply;
/// `None` means it may be applied to the current assets. A different asset
/// descriptor is deliberately returned before profile lookup: the outer
/// session must first unmount the old lifetime and resolve the exact saved
/// descriptor, then validate the saved campaign against the reconstructed
/// profile graph.
pub(crate) fn validated_save_reload_target(
    save: &crate::save_file::GameSaveFile,
    profiles: &engine_profiles::ProfileManager,
    active_mission_id: u32,
    active_mission_assets: &robin_engine::mission_assets::MissionAssetDescriptor,
    active_spellforge_package: Option<&robin_engine::spellforge::SpellforgePackage>,
) -> Result<Option<u32>, String> {
    assert_ne!(
        active_mission_id, 0,
        "validated_save_reload_target: active mission ID zero is invalid"
    );
    save.validate_current_schema()
        .map_err(|error| format!("invalid current save schema: {error:#}"))?;
    if save.header.mission_id != active_mission_id
        || save.header.mission_assets != *active_mission_assets
        || save.engine.spellforge_package().as_deref() != active_spellforge_package
    {
        return Ok(Some(save.header.mission_id));
    }
    validate_save_mission(save, profiles)?;
    Ok(None)
}

/// Consume an already-decoded save with its exact preflighted slot, or do the
/// one allowed disk read for a not-yet-preflighted request. Once `save` is
/// present this function validates runtime ownership without rereading its path.
pub(crate) fn preflight_or_use_decoded_load(
    save_manager: &crate::savegame::SaveGameManager,
    slot: Option<SlotHandle>,
    save: Option<crate::save_file::GameSaveFile>,
) -> anyhow::Result<Option<(Option<SlotHandle>, crate::save_file::PreparedGameSave)>> {
    preflight_load_with_origin(save_manager, slot, save, OperationOrigin::Local)
}

fn preflight_load_with_origin(
    save_manager: &SaveGameManager,
    slot: Option<SlotHandle>,
    save: Option<crate::save_file::GameSaveFile>,
    origin: OperationOrigin,
) -> anyhow::Result<Option<(Option<SlotHandle>, crate::save_file::PreparedGameSave)>> {
    let index = slot
        .as_ref()
        .map(|handle| save_manager.resolve_handle(handle))
        .transpose()?;
    match save {
        Some(save) => {
            anyhow::ensure!(
                slot.is_some() || origin == OperationOrigin::CommittedMultiplayer,
                "preflighted local load is missing its exact decoded slot"
            );
            Ok(Some((slot, save.into())))
        }
        None => {
            anyhow::ensure!(
                origin == OperationOrigin::Local,
                "committed multiplayer load is missing its decoded payload"
            );
            save_manager
                .preflight_load(index)?
                .map(|(index, save)| Ok((Some(save_manager.slot_handle(index)?), save)))
                .transpose()
        }
    }
}

/// Execute queued game-flow effects against the narrow host facilities they
/// are allowed to mutate: sound playback and mouse-event acceptance.
///
/// Call after `game.process_operation` and before any early return from code
/// that emitted an effect. The callback boundary cannot touch these host
/// resources directly because the frame loop owns them.
pub(crate) fn execute_app_effects(
    effects: &mut AppEffectQueue,
    sound: &mut crate::sound::SoundManager,
    threaded_input: &mut crate::input::ThreadedInput,
    mut audio_backend: Option<&mut dyn crate::sound::AudioBackend>,
) {
    let result: Result<(), AppEffectExecutionError<std::convert::Infallible>> = effects
        .try_execute(|effect| {
            match effect {
                AppEffect::SetSoundMode(mode) => {
                    // `None` is an explicit audio-disabled state established
                    // by setup (`-NOSOUND`, headless, or a logged init
                    // failure), so acknowledging sound-only effects here is
                    // intentional rather than a fabricated fallback value.
                    if let Some(backend) = audio_backend.as_deref_mut() {
                        let sound_mode = match mode {
                            crate::app_effect::SoundMode::Menu => AudioSoundMode::Menu,
                            crate::app_effect::SoundMode::Mission => AudioSoundMode::Mission,
                        };
                        sound.set_mode(sound_mode, backend);
                    }
                }
                AppEffect::PlayJingle(jingle) => {
                    if let Some(backend) = audio_backend.as_deref_mut() {
                        let sound_jingle = match jingle {
                            crate::app_effect::Jingle::MissionWon => SoundJingle::MissionWon,
                            crate::app_effect::Jingle::MissionLost => SoundJingle::MissionLost,
                        };
                        sound.play_jingle(sound_jingle, backend);
                    }
                }
                AppEffect::SetMouseEnabled(enabled) => {
                    // Disable the input pump's mouse-event branch during
                    // cinematics / mission briefings / movie playback so
                    // motion and clicks do not leak to the game.
                    threaded_input.set_enabled(enabled);
                }
            }
            Ok(())
        });

    if let Err(AppEffectExecutionError::Effect { source, .. }) = result {
        match source {}
    }
}

/// Flush any pending save/load request queued by the Game state machine.
///
/// Called from `game_session` between `game.process_operation` and the
/// next engine tick.  This is where the actual disk I/O happens, with
/// live access to both engine and campaign.
///
/// `thumbnail` is the captured screen preview written alongside save
/// slots, right after the main save payload.  Callers should render a
/// dedicated throwaway frame and read it back, mirroring the HTTP
/// screenshot path, so thumbnail contents never depend on stale render
/// target state.  If the capture failed or the caller has no renderer
/// handy, pass `None` and the save is written without a thumbnail.
fn replay_save_written_event(
    engine: &engine_api::Engine,
    host: &crate::host::Host,
    game: &crate::game::Game,
) -> Option<SaveLoadEvent> {
    match crate::save_file::GameRuntimeSnapshot::identity_of_live(engine, host, game) {
        Ok(identity) => Some(SaveLoadEvent::SaveWritten { identity }),
        Err(error) => {
            tracing::error!(
                "save succeeded, but its replay identity could not be computed: {error:#}"
            );
            None
        }
    }
}

fn replay_loaded_identity(
    save: &crate::save_file::PreparedGameSave,
) -> Option<crate::save_file::ReplaySaveIdentity> {
    match save.replay_identity() {
        Ok(identity) => Some(identity),
        Err(error) => {
            tracing::error!("loaded save has no usable replay identity: {error:#}");
            None
        }
    }
}

fn begin_multiplayer_snapshot_transition(
    host: &mut crate::host::Host,
    slot: SlotHandle,
    save: crate::save_file::GameSaveFile,
) -> Result<bool, String> {
    let Some(net) = host.transport.net.as_ref() else {
        return Ok(false);
    };
    if host.transport.local_seat != robin_engine::player_command::PlayerId::HOST {
        return Err(
            "only the multiplayer host can load, restart, or quick-load the session".to_string(),
        );
    }
    if host.transport.reconnecting || host.transport.snapshot_transition.is_some() {
        return Err("a multiplayer snapshot transition is already in progress".to_string());
    }
    if save.header.multiplayer_diagnostic {
        return Err("multiplayer diagnostic saves cannot be loaded while connected".to_string());
    }
    save.validate_current_schema()
        .map_err(|error| format!("multiplayer snapshot is not a current valid save: {error:#}"))?;
    let mission_id = save.header.mission_id;
    let save_bytes = serde_json::to_vec(&save)
        .map_err(|error| format!("encode multiplayer snapshot transition: {error}"))?;
    let id = net.begin_snapshot_transition(mission_id, save_bytes)?;
    host.transport.snapshot_transition = Some(crate::host::PendingSnapshotTransition {
        id,
        payload: crate::host::PendingSnapshotTransitionPayload::Save {
            slot: Some(slot),
            save: Box::new(save),
        },
        committed: false,
    });
    host.transport.reconnecting = true;
    tracing::info!(
        ?id,
        mission_id,
        "multiplayer: waiting for every peer to validate the authoritative transition"
    );
    Ok(true)
}

pub(crate) fn perform_pending_save_load(
    host: &mut crate::host::Host,
    game: &mut crate::game::Game,
    callbacks: &mut RustCallbacks,
    engine: &mut engine_api::Engine,
    assets: &engine_api::LevelAssets,
    profiles: &engine_profiles::ProfileManager,
    thumbnail: Option<crate::save_file::Thumbnail>,
) -> OperationOutcome {
    let Some(OperationRequest {
        action: request,
        origin,
    }) = callbacks.pending.take()
    else {
        return OperationOutcome::NOT_PENDING;
    };
    // A load/restart must observe the completed payload and its matching index,
    // never a physical replacement that the owner has not committed yet.
    if let Err(error) = callbacks.save_manager.finish_background() {
        callbacks
            .autosave_notices
            .enqueue_save_failed(format!("{error:#}"));
        return OperationOutcome {
            banner: Some(SaveBannerKind::SaveFailed),
            ..OperationOutcome::NO_EVENT
        };
    }
    let mut outcome = OperationOutcome::NO_EVENT;
    let thumb_ref = thumbnail.as_ref();
    let mut event = None;
    match request {
        SaveLoadRequest::Save { slot, mission_id } => {
            let slot = match slot
                .as_ref()
                .map(|handle| callbacks.save_manager.resolve_handle(handle))
                .transpose()
            {
                Ok(slot) => slot,
                Err(error) => {
                    tracing::error!("Save rejected stale slot handle: {error:#}");
                    return outcome;
                }
            };
            if host.transport.net.is_some() {
                let idx = slot.unwrap_or_else(|| {
                    callbacks
                        .save_manager
                        .create("Multiplayer diagnostic".to_string(), mission_id)
                });
                let result = callbacks
                    .save_manager
                    .write_multiplayer_diagnostic_from_engine(
                        host,
                        game,
                        idx,
                        engine,
                        mission_id,
                        Some(profiles),
                        thumb_ref,
                    )
                    .and_then(|()| {
                        callbacks
                            .save_manager
                            .save_index()
                            .map_err(anyhow::Error::msg)
                    });
                match result {
                    Ok(()) => {
                        outcome.banner = Some(SaveBannerKind::Saved);
                        tracing::info!(
                            slot = idx,
                            mission_id,
                            "multiplayer diagnostic save written locally"
                        );
                    }
                    Err(error) => {
                        tracing::error!("Multiplayer diagnostic save failed: {error:#}")
                    }
                }
                return outcome;
            }
            // `slot = None` ⇒ auto Continue-save.
            // `slot = Some(idx)` ⇒ player-chosen slot.
            let (result, explicit_slot) = match slot {
                Some(idx) => (
                    callbacks
                        .save_manager
                        .write_save_from_engine(
                            host,
                            game,
                            idx,
                            engine,
                            mission_id,
                            Some(profiles),
                            thumb_ref,
                        )
                        .and_then(|()| {
                            callbacks
                                .save_manager
                                .save_index()
                                .map_err(|e| anyhow::anyhow!(e))
                        }),
                    true,
                ),
                None => (
                    callbacks.save_manager.write_continue_save(
                        host,
                        game,
                        engine,
                        mission_id,
                        Some(profiles),
                        thumb_ref,
                    ),
                    false,
                ),
            };
            if let Err(err) = result {
                tracing::error!("Save failed: {err:#}");
            } else {
                tracing::info!("Save completed (mission={mission_id})");
                event = replay_save_written_event(engine, host, game);
                // Mirror the manual save into the Continue slot. The
                // guard keeps Continue→Continue copies from clobbering
                // themselves; Restart / Sherwood slots also skip the
                // mirror and the banner branch.
                if explicit_slot {
                    let is_special = slot
                        .and_then(|idx| callbacks.save_manager.get(idx))
                        .and_then(|s| s.special);
                    let is_continue_or_restart = matches!(
                        is_special,
                        Some(SpecialSlot::Continue) | Some(SpecialSlot::Restart)
                    );
                    if !is_continue_or_restart
                        && let Err(err) = callbacks.save_manager.write_continue_save(
                            host,
                            game,
                            engine,
                            mission_id,
                            Some(profiles),
                            thumb_ref,
                        )
                    {
                        tracing::warn!("Continue-mirror after save failed: {err:#}");
                    }
                    // Show "Game saved." banner unless the slot is one
                    // of the filtered types (Restart / Sherwood).
                    let is_sherwood = matches!(is_special, Some(SpecialSlot::Sherwood));
                    if !is_continue_or_restart && !is_sherwood {
                        outcome.banner = Some(SaveBannerKind::Saved);
                    }
                }
            }
        }
        SaveLoadRequest::Load {
            slot,
            mission_id: _,
            save,
        } => {
            let applying_multiplayer_transition = origin == OperationOrigin::CommittedMultiplayer;
            // If the save targets a different mission than the one currently
            // running, stash a `PendingLevelLoad` and let the session loop
            // switch missions before re-applying. This replaces the previous
            // warn-and-apply behaviour, which corrupted engine state when
            // the payload's mission didn't match the active level.
            let resolved =
                match preflight_load_with_origin(&callbacks.save_manager, slot, save, origin) {
                    Ok(resolved) => resolved,
                    Err(error) => {
                        tracing::error!("Load preflight failed: {error:#}");
                        return outcome;
                    }
                };
            match resolved {
                Some((slot, save)) => {
                    // Remote committed snapshots have no local slot metadata;
                    // all locally selected handles were validated by preflight.
                    let idx = match slot
                        .as_ref()
                        .map(|handle| callbacks.save_manager.resolve_handle(handle))
                        .transpose()
                    {
                        Ok(index) => index,
                        Err(error) => {
                            tracing::error!("Load rejected stale slot handle: {error:#}");
                            return outcome;
                        }
                    };
                    if host.transport.net.is_some() && !applying_multiplayer_transition {
                        let Some(slot) = slot else {
                            tracing::error!("Local multiplayer load is missing its selected slot");
                            return outcome;
                        };
                        match begin_multiplayer_snapshot_transition(host, slot, save.into_payload())
                        {
                            Ok(true) => return outcome,
                            Ok(false) => unreachable!("multiplayer transition guard checked net"),
                            Err(error) => {
                                tracing::error!("Load rejected: {error}");
                                return outcome;
                            }
                        }
                    }
                    let active_mission_id = current_mission_id(engine.campaign(), profiles);
                    let active_mission_assets = match game.mission_assets() {
                        Ok(descriptor) => descriptor,
                        Err(error) => {
                            tracing::error!("Load rejected: {error}");
                            return outcome;
                        }
                    };
                    let active_spellforge_package = engine.spellforge_package();
                    let reload_target = match validated_save_reload_target(
                        &save,
                        profiles,
                        active_mission_id,
                        active_mission_assets,
                        active_spellforge_package.as_deref(),
                    ) {
                        Ok(target) => target,
                        Err(error) => {
                            tracing::error!("Load preflight rejected slot {idx:?}: {error}");
                            return outcome;
                        }
                    };
                    if let Some(target_mission_id) = reload_target {
                        tracing::info!(
                            "Load slot {idx:?}: cross-mission load (header={}, current={}) — \
                             routing through session LevelLoad",
                            target_mission_id,
                            active_mission_id,
                        );
                        outcome.transition = Some(PendingLevelLoad {
                            slot,
                            target_mission_id,
                            origin,
                            save: save.into_payload(),
                        });
                        return outcome;
                    }
                    let validated_mission_id = save.header.mission_id;
                    let replay_identity = replay_loaded_identity(&save);
                    match save.apply_to_with_game(engine, host, game, assets) {
                        Err(err) => {
                            tracing::error!("Load failed: {err:#}");
                        }
                        _ => {
                            // Thread the slot type through so the frame loop
                            // can replay the continue / campaign-map fix-ups.
                            let (is_continue, is_restart, is_sherwood) = match idx {
                                Some(idx) => {
                                    let Some(metadata) = callbacks.save_manager.get(idx) else {
                                        tracing::error!(
                                            "Loaded slot metadata disappeared after preflight"
                                        );
                                        return outcome;
                                    };
                                    (
                                        metadata.is_continue(),
                                        metadata.is_restart(),
                                        metadata.is_sherwood(),
                                    )
                                }
                                // A remote snapshot is deliberately not any of
                                // this peer's local special slots.
                                None => (false, false, false),
                            };
                            outcome.restore = Some(PostLoadSync { is_continue });
                            // The frame loop clears the translator's
                            // key-edge state so half-pressed keys at save
                            // time don't stick across the load.
                            outcome.reset_input = true;
                            // Mirror the load into the Continue slot,
                            // guarded by IsContinue/IsRestart so we
                            // don't clobber the slot we just loaded.
                            if !is_continue
                                && !is_restart
                                && let Err(error) =
                                    callbacks.save_manager.write_continue_save_background(
                                        host,
                                        game,
                                        engine,
                                        validated_mission_id,
                                        Some(profiles),
                                        thumb_ref,
                                    )
                            {
                                tracing::warn!(
                                    "Continue-mirror after load could not start: {error:#}"
                                );
                            }
                            // Show "Game loaded." banner unless the slot
                            // is Restart / Sherwood.
                            if !is_restart && !is_sherwood {
                                outcome.banner = Some(SaveBannerKind::Loaded);
                            }
                            tracing::info!("Load completed from slot {idx:?}");
                            event = replay_identity.map(|identity| SaveLoadEvent::LoadApplied {
                                identity,
                                is_continue,
                            });
                        }
                    }
                }
                None => {
                    tracing::warn!("Load requested but no matching save slot found");
                }
            }
        }
        SaveLoadRequest::Restart => {
            let campaign = engine.campaign();
            let mid = current_mission_id(campaign, profiles);
            if let Err(err) = callbacks.save_manager.write_restart_save(
                host,
                game,
                engine,
                mid,
                Some(profiles),
                thumb_ref,
            ) {
                tracing::error!("Restart save failed: {err:#}");
            } else {
                event = match callbacks.save_manager.restart_session_identity() {
                    Some(identity) => Some(SaveLoadEvent::SaveWritten { identity }),
                    None => replay_save_written_event(engine, host, game),
                };
            }
        }
        SaveLoadRequest::LoadRestart => {
            if host.transport.net.is_some() {
                match callbacks.save_manager.preflight_restart_save() {
                    Ok(Some((idx, save))) => {
                        let slot = match callbacks.save_manager.slot_handle(idx) {
                            Ok(slot) => slot,
                            Err(error) => {
                                tracing::error!(
                                    "Multiplayer restart rejected stale slot: {error:#}"
                                );
                                return outcome;
                            }
                        };
                        if let Err(error) =
                            begin_multiplayer_snapshot_transition(host, slot, save.into_payload())
                        {
                            tracing::error!("Multiplayer restart rejected: {error}");
                        }
                    }
                    Ok(None) => {
                        tracing::error!("Multiplayer restart rejected: no restart snapshot exists")
                    }
                    Err(error) => {
                        tracing::error!("Multiplayer restart snapshot preflight failed: {error:#}")
                    }
                }
                return outcome;
            }
            let restore_result = (|| -> anyhow::Result<_> {
                let (_idx, save) = callbacks
                    .save_manager
                    .preflight_restart_save()?
                    .ok_or_else(|| anyhow::anyhow!("no restart snapshot exists"))?;
                let active_mission_id = current_mission_id(engine.campaign(), profiles);
                let active_spellforge_package = engine.spellforge_package();
                if let Some(target_mission_id) = validated_save_reload_target(
                    &save,
                    profiles,
                    active_mission_id,
                    game.mission_assets().map_err(anyhow::Error::msg)?,
                    active_spellforge_package.as_deref(),
                )
                .map_err(anyhow::Error::msg)?
                {
                    anyhow::bail!(
                        "save mission {target_mission_id} does not match active mission {active_mission_id}"
                    );
                }
                let replay_identity = replay_loaded_identity(&save);
                save.apply_to_with_game(engine, host, game, assets)?;
                Ok(replay_identity)
            })();
            match restore_result {
                Ok(replay_identity) => {
                    // Restart = never Continue slot; still sync campaign-map state.
                    outcome.restore = Some(PostLoadSync { is_continue: false });
                    tracing::info!("Restart snapshot restored");
                    event = replay_identity.map(|identity| SaveLoadEvent::LoadApplied {
                        identity,
                        is_continue: false,
                    });
                }
                Err(error) => {
                    tracing::error!(
                        "Restart snapshot could not be restored; routing through authoritative LevelRestart: {error:#}"
                    );
                    outcome.restart_requested = true;
                    game.operation.set(GameCode::LevelRestart);
                }
            }
        }
        SaveLoadRequest::Continue { mission_id } => {
            if let Err(err) = callbacks.save_manager.write_continue_save(
                host,
                game,
                engine,
                mission_id,
                Some(profiles),
                thumb_ref,
            ) {
                tracing::error!("Continue save failed: {err:#}");
            } else {
                event = replay_save_written_event(engine, host, game);
            }
        }
        SaveLoadRequest::QuickSave { mission_id } => {
            if host.transport.net.is_some() {
                let idx = callbacks
                    .save_manager
                    .create("Multiplayer quick diagnostic".to_string(), mission_id);
                let result = callbacks
                    .save_manager
                    .write_multiplayer_diagnostic_from_engine(
                        host,
                        game,
                        idx,
                        engine,
                        mission_id,
                        Some(profiles),
                        thumb_ref,
                    )
                    .and_then(|()| {
                        callbacks
                            .save_manager
                            .save_index()
                            .map_err(anyhow::Error::msg)
                    });
                match result {
                    Ok(()) => {
                        outcome.banner = Some(SaveBannerKind::Saved);
                        tracing::info!(
                            slot = idx,
                            mission_id,
                            "multiplayer quick-save captured as a local diagnostic"
                        );
                    }
                    Err(error) => {
                        tracing::error!("Multiplayer quick diagnostic failed: {error:#}")
                    }
                }
                return outcome;
            }
            match callbacks.save_manager.write_quick_save(
                host,
                game,
                engine,
                mission_id,
                Some(profiles),
                thumb_ref,
            ) {
                Err(err) => {
                    tracing::error!("Quick save failed: {err:#}");
                }
                _ => {
                    tracing::info!("Quick save written (mission={mission_id})");
                    event = replay_save_written_event(engine, host, game);
                    // QuickSave is neither Continue nor Restart, so the
                    // Continue-slot mirror runs.
                    if let Err(err) = callbacks.save_manager.write_continue_save(
                        host,
                        game,
                        engine,
                        mission_id,
                        Some(profiles),
                        thumb_ref,
                    ) {
                        tracing::warn!("Continue-mirror after quick-save failed: {err:#}");
                    }
                    outcome.banner = Some(SaveBannerKind::Saved);
                }
            }
        }
        SaveLoadRequest::QuickLoad { use_backup } => {
            // Shift+F12 loads `ExQuickSave` (the backup).
            // Plain F12 loads `QuickSave`.
            let slot_name = if use_backup {
                special_slots::EX_QUICK
            } else {
                special_slots::QUICK
            };
            let idx = callbacks.save_manager.find_by_filename(slot_name);
            match idx {
                Some(i) if callbacks.save_manager.slot_file_exists(i) => {
                    match callbacks.save_manager.preflight_load(Some(i)) {
                        Err(error) => {
                            tracing::error!("Quick load ({slot_name}) preflight failed: {error:#}");
                        }
                        Ok(None) => {
                            tracing::error!(
                                "Quick load ({slot_name}) lost its selected slot during preflight"
                            );
                        }
                        Ok(Some((decoded_idx, save))) => {
                            let slot = match callbacks.save_manager.slot_handle(decoded_idx) {
                                Ok(slot) => slot,
                                Err(error) => {
                                    tracing::error!(
                                        "Quick load ({slot_name}) rejected stale slot: {error:#}"
                                    );
                                    return outcome;
                                }
                            };
                            if host.transport.net.is_some() {
                                match begin_multiplayer_snapshot_transition(
                                    host,
                                    slot,
                                    save.into_payload(),
                                ) {
                                    Ok(true) => return outcome,
                                    Ok(false) => {
                                        unreachable!("multiplayer transition guard checked net")
                                    }
                                    Err(error) => {
                                        tracing::error!(
                                            "Quick load ({slot_name}) rejected: {error}"
                                        );
                                        return outcome;
                                    }
                                }
                            }
                            let active_mission_id = current_mission_id(engine.campaign(), profiles);
                            let active_mission_assets = match game.mission_assets() {
                                Ok(descriptor) => descriptor,
                                Err(error) => {
                                    tracing::error!("Quick load ({slot_name}) rejected: {error}");
                                    return outcome;
                                }
                            };
                            let active_spellforge_package = engine.spellforge_package();
                            let reload_target = match validated_save_reload_target(
                                &save,
                                profiles,
                                active_mission_id,
                                active_mission_assets,
                                active_spellforge_package.as_deref(),
                            ) {
                                Ok(target) => target,
                                Err(error) => {
                                    tracing::error!("Quick load ({slot_name}) rejected: {error}");
                                    return outcome;
                                }
                            };
                            if let Some(target_mission_id) = reload_target {
                                tracing::info!(
                                    "Quick load ({slot_name}): routing mission {target_mission_id} through session LevelLoad"
                                );
                                outcome.transition = Some(PendingLevelLoad {
                                    slot: Some(slot),
                                    target_mission_id,
                                    origin,
                                    save: save.into_payload(),
                                });
                                return outcome;
                            }
                            let validated_mission_id = save.header.mission_id;
                            let replay_identity = replay_loaded_identity(&save);
                            if let Err(error) = save.apply_to_with_game(engine, host, game, assets)
                            {
                                tracing::error!("Quick load ({slot_name}) failed: {error:#}");
                                return outcome;
                            }
                            // QuickSave is not the Continue slot; just re-sync
                            // campaign-map state.
                            outcome.restore = Some(PostLoadSync { is_continue: false });
                            outcome.reset_input = true;
                            // Mirror into the Continue slot — QuickSave is
                            // neither Continue nor Restart so it always
                            // mirrors.
                            if let Err(error) =
                                callbacks.save_manager.write_continue_save_background(
                                    host,
                                    game,
                                    engine,
                                    validated_mission_id,
                                    Some(profiles),
                                    thumb_ref,
                                )
                            {
                                tracing::warn!(
                                    "Continue-mirror after quick-load could not start: {error:#}"
                                );
                            }
                            outcome.banner = Some(SaveBannerKind::Loaded);
                            tracing::info!("Quick save loaded from {slot_name}");
                            event = replay_identity.map(|identity| SaveLoadEvent::LoadApplied {
                                identity,
                                is_continue: false,
                            });
                        }
                    }
                }
                _ => tracing::warn!("Quick load requested but no {slot_name} save on disk"),
            }
        }
        SaveLoadRequest::Sherwood { mission_id } => {
            match callbacks.save_manager.write_sherwood_save(
                host,
                game,
                engine,
                mission_id,
                Some(profiles),
                thumb_ref,
            ) {
                Err(err) => {
                    tracing::error!("Sherwood checkpoint save failed: {err:#}");
                }
                _ => {
                    tracing::info!("Sherwood checkpoint saved (mission={mission_id})");
                    event = replay_save_written_event(engine, host, game);
                }
            }
        }
    }
    OperationOutcome {
        processed: true,
        event,
        ..outcome
    }
}

// ─── Resource helpers ───────────────────────────────────────────────

#[cfg(test)]
mod operation_outcome_tests {
    use super::*;

    #[test]
    fn autosave_notices_preserve_latest_result_and_defer_behind_manual_banners() {
        let mut notices = AutosaveNotices::default();
        notices.enqueue_failed();
        assert_eq!(
            notices.select_banner(Some(SaveBannerKind::Saved)),
            Some(SaveBannerKind::Saved)
        );
        assert_eq!(
            notices.select_banner(None),
            Some(SaveBannerKind::AutosaveFailed)
        );
        assert_eq!(notices.select_banner(None), None);
        notices.enqueue_failed();
        notices.observe([AutosavePollResult::Saved {
            filename: "test".to_owned(),
            reason: AutosaveReason::Periodic,
        }]);
        assert_eq!(notices.select_banner(None), Some(SaveBannerKind::Autosaved));
    }

    #[test]
    fn decoded_load_requires_live_handle_even_when_payload_is_already_owned() {
        let directory = tempfile::tempdir().unwrap();
        let mut manager = SaveGameManager::new(directory.path().to_string_lossy().into_owned());
        let other = manager.create("other row".into(), 7);
        let index = manager.create("selected".into(), 7);
        let handle = manager.slot_handle(index).unwrap();
        let name = handle.name().as_str().to_owned();
        let mut assets = engine_api::LevelAssets::default();
        let engine =
            engine_api::Engine::new_for_test(640.0, 480.0, Campaign::default(), &mut assets)
                .unwrap();
        let host = crate::host::Host::scratch(640.0, 480.0);
        let save = crate::save_file::GameSaveFile::capture(&engine, &host, 7, "owned".into());

        let (resolved, prepared) =
            preflight_or_use_decoded_load(&manager, Some(handle.clone()), Some(save.clone()))
                .unwrap()
                .unwrap();
        assert_eq!(manager.resolve_handle(&resolved.unwrap()).unwrap(), index);
        assert_eq!(prepared.header.display_text, "owned");

        let foreign = SaveGameManager::new(directory.path().to_string_lossy().into_owned());
        assert!(
            preflight_or_use_decoded_load(&foreign, Some(handle.clone()), Some(save.clone()))
                .is_err()
        );
        let request = SaveLoadRequest::Load {
            slot: Some(handle.clone()),
            mission_id: 7,
            save: Some(save.clone()),
        };
        let decoded: SaveLoadRequest =
            serde_json::from_slice(&serde_json::to_vec(&request).unwrap()).unwrap();
        let SaveLoadRequest::Load {
            slot,
            save: decoded_save,
            ..
        } = decoded
        else {
            unreachable!()
        };
        assert!(preflight_or_use_decoded_load(&manager, slot, decoded_save).is_err());

        manager.remove(other).unwrap();
        let (resolved, _) =
            preflight_or_use_decoded_load(&manager, Some(handle.clone()), Some(save.clone()))
                .unwrap()
                .unwrap();
        let shifted_index = manager.resolve_handle(&resolved.unwrap()).unwrap();
        assert_ne!(shifted_index, index);
        manager.remove(shifted_index).unwrap();
        assert!(
            preflight_or_use_decoded_load(&manager, Some(handle.clone()), Some(save.clone()))
                .is_err()
        );
        let recreated = manager.create_with_filename(name, "replacement".into(), 7);
        assert!(preflight_or_use_decoded_load(&manager, Some(handle), Some(save.clone())).is_err());
        assert!(
            preflight_or_use_decoded_load(
                &manager,
                Some(manager.slot_handle(recreated).unwrap()),
                Some(save)
            )
            .is_ok()
        );
    }

    #[test]
    fn remote_decoded_load_requires_process_local_committed_origin() {
        let directory = tempfile::tempdir().unwrap();
        let manager = SaveGameManager::new(directory.path().to_string_lossy().into_owned());
        let mut assets = engine_api::LevelAssets::default();
        let engine =
            engine_api::Engine::new_for_test(640.0, 480.0, Campaign::default(), &mut assets)
                .unwrap();
        let host = crate::host::Host::scratch(640.0, 480.0);
        let save = crate::save_file::GameSaveFile::capture(&engine, &host, 7, "remote".into());
        assert!(preflight_or_use_decoded_load(&manager, None, Some(save.clone())).is_err());
        let (slot, decoded) = preflight_load_with_origin(
            &manager,
            None,
            Some(save.clone()),
            OperationOrigin::CommittedMultiplayer,
        )
        .unwrap()
        .unwrap();
        assert!(slot.is_none());
        assert_eq!(decoded.header.display_text, "remote");
        assert!(
            preflight_load_with_origin(&manager, None, None, OperationOrigin::CommittedMultiplayer)
                .is_err()
        );

        let transition = PendingLevelLoad {
            slot: None,
            target_mission_id: 7,
            origin: OperationOrigin::CommittedMultiplayer,
            save,
        };
        transition.validate_slot(&manager).unwrap();
        let restored: PendingLevelLoad =
            serde_json::from_slice(&serde_json::to_vec(&transition).unwrap()).unwrap();
        assert_eq!(restored.origin, OperationOrigin::Local);
        assert!(restored.validate_slot(&manager).is_err());
    }

    #[test]
    fn failed_restart_is_an_explicit_one_frame_outcome_and_latest_request_wins() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_str().unwrap().to_owned();
        let mut profiles = robin_engine::player_profile::PlayerProfileManager::new(path.clone());
        let index = profiles.create_profile(
            "Test".to_owned(),
            robin_engine::player_profile::DifficultyLevel::Medium,
        );
        profiles.set_active(index);
        let context = ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&path),
            engine_api::GlobalOptions::default(),
            profiles,
            crate::key_config_store::KeyConfigStore::new(path),
            None,
        )
        .unwrap();
        let mut host =
            crate::host::Host::new(context.clone().try_into().unwrap(), 640.0, 480.0).unwrap();
        let mut callbacks = RustCallbacks::new(context).unwrap();
        let mut assets = engine_api::LevelAssets::default();
        let mut engine =
            engine_api::Engine::new_for_test(640.0, 480.0, Campaign::default(), &mut assets)
                .unwrap();
        let mut game = crate::game::Game::default();
        let profiles = ProfileManager::default();
        callbacks.queue_operation(SaveLoadRequest::QuickLoad { use_backup: false });
        callbacks.queue_operation(SaveLoadRequest::LoadRestart);
        assert!(matches!(
            callbacks.pending_request(),
            Some(SaveLoadRequest::LoadRestart)
        ));
        let outcome = perform_pending_save_load(
            &mut host,
            &mut game,
            &mut callbacks,
            &mut engine,
            &assets,
            &profiles,
            None,
        );
        assert!(outcome.processed);
        assert!(outcome.restart_requested);
        assert!(outcome.restore.is_none());
        assert!(outcome.event.is_none());
        assert!(!outcome.reset_input);
        assert!(callbacks.pending.is_none());
        let next = perform_pending_save_load(
            &mut host,
            &mut game,
            &mut callbacks,
            &mut engine,
            &assets,
            &profiles,
            None,
        );
        assert!(!next.processed);
        assert!(!next.restart_requested);
        assert!(next.restore.is_none());

        let save = crate::save_file::GameSaveFile::capture(&engine, &host, 7, "handoff".to_owned());
        callbacks.queue_committed_load(SaveLoadRequest::Load {
            slot: None,
            mission_id: 7,
            save: Some(save.clone()),
        });
        assert_eq!(
            callbacks.pending.as_ref().unwrap().origin,
            OperationOrigin::CommittedMultiplayer
        );
        callbacks.queue_operation(SaveLoadRequest::QuickLoad { use_backup: false });
        assert_eq!(
            callbacks.pending.as_ref().unwrap().origin,
            OperationOrigin::Local
        );
        callbacks.clear_operation();
        assert!(callbacks.pending_request().is_none());

        let transition = PendingLevelLoad {
            slot: None,
            target_mission_id: 7,
            origin: OperationOrigin::CommittedMultiplayer,
            save,
        };
        let loaded = crate::game_session::MissionOutcome::from_engine(
            Campaign::default(),
            4,
            engine_api::SimConfig::default(),
            Ok(GameCode::LevelLoad),
        )
        .with_transition(Some(transition.clone()));
        assert_eq!(
            loaded.transition.unwrap().save.header.display_text,
            "handoff"
        );
        let failed = crate::game_session::MissionOutcome::from_engine(
            Campaign::default(),
            4,
            engine_api::SimConfig::default(),
            Err("exit failed".to_owned()),
        )
        .with_transition(Some(transition));
        assert!(failed.transition.is_none());
        assert_eq!(failed.result, Err("exit failed".to_owned()));
    }
}

/// Upload a 16-bit RGB565 Picture into a new renderer surface.
pub(crate) fn picture_to_surface(renderer: &mut Renderer, pic: &Picture) -> u32 {
    let pixels: Vec<u16> = pic
        .data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    renderer
        .create_surface_from_rgb565(pic.width, pic.height, &pixels)
        .expect("picture_to_surface: decoded picture dimensions must match RGB565 payload")
}

// ─── Top-level entry ────────────────────────────────────────────────

/// Detect demo mode at runtime by checking for demo mission files.
/// Returns `(mission_name, proto_name, pc_string, location)` if a demo is detected.
pub(crate) fn detect_demo_mode_with_context(
    application_context: &ApplicationContext,
) -> Option<(&'static str, &'static str, &'static str, MissionLocation)> {
    let files = application_context
        .preparation_files()
        .expect("demo detection requires explicit resource authority");
    let resolve = |path: &str| {
        files
            .try_exists(path)
            .unwrap_or_else(|status| panic!("demo asset lookup failed for {path}: {status}"))
    };
    let shipping_has_level = |mission: &str| {
        application_context
            .shipping()
            .expect("demo detection requires an initialized ApplicationContext")
            .is_some_and(|dd| dd.has_mission(mission))
    };
    if resolve("Data/Levels/Dem_Lei_MP.rhm") || shipping_has_level("Dem_Lei_MP") {
        // Leicester demo — R=Robin, J=Jean, M=Marianne, T=Tuck, F=Ferris.
        Some((
            "Dem_Lei_MP",
            "Leicester",
            "RJMTF",
            MissionLocation::Leicester,
        ))
    } else if resolve("Data/Levels/Demo_Lin.rhm") || shipping_has_level("Demo_Lin") {
        // Lincoln demo — R=Robin, S=Stutely, A/B/C=Peasants
        Some(("Demo_Lin", "Lincoln", "RSABC", MissionLocation::Lincoln))
    } else {
        None
    }
}

/// Resolve the loading screen `.pak` file path.
///
/// First probes `Data/Levels/<ambience:%02u>/<proto_level_filename>.pak`,
/// falling back to `Data/Interface/Loading.pak` when the per-ambience file
/// is missing. Returns `None` when neither exists.
///
/// `proto_level_filename` comes from the mission's profile. The caller
/// threads it from `campaign_ref.missions[mission_idx].profile(..)`.
///
/// `ambience` is the raw ambience bitmask (1=Day, 2=Fog, 4=Night) read
/// from the `.rhm` header. The loading screen is shown *before* opening
/// the mission file, so the precise ambience isn't known yet — pass
/// `None` and we probe each candidate (`01`, `02`, `04`) in turn. Only
/// one ambience pak ever ships per mission, so the probe degenerates to
/// the same answer as an exact lookup.
pub(crate) fn resolve_loading_pak(
    application_context: &ApplicationContext,
    proto_level_filename: Option<&str>,
    ambience: Option<u32>,
) -> Option<String> {
    let shipping = application_context
        .shipping()
        .expect("loading pak resolution requires an initialized ApplicationContext");
    let data_asset_exists = |path: &str| {
        if application_context
            .preparation_files()
            .expect("loading pak resolution requires explicit resource authority")
            .try_exists(path)
            .unwrap_or_else(|status| panic!("loading asset lookup failed for {path}: {status}"))
        {
            return true;
        }
        let normalized = path.replace('\\', "/").to_ascii_lowercase();
        let key = normalized.strip_prefix("data/").unwrap_or(&normalized);
        shipping.is_some_and(|dd| dd.localized_pak(key).is_some())
    };

    if let Some(proto) = proto_level_filename {
        // Day=1, Fog=2, Night=4. Probe all three when the caller doesn't
        // have the exact ambience yet; only one mission-specific pak
        // exists per mission, so the result matches an exact lookup
        // either way.
        let single = ambience.map(|a| [a]);
        let candidates: &[u32] = match single.as_ref() {
            Some(arr) => arr,
            None => &[1, 2, 4],
        };
        for &amb in candidates {
            let candidate = format!("Data/Levels/{:02}/{}.pak", amb, proto);
            if data_asset_exists(&candidate) {
                tracing::info!("Loading screen .pak: using mission-specific {candidate}");
                return Some(candidate);
            }
        }
    }
    let default_path = "Data/Interface/Loading.pak";
    if data_asset_exists(default_path) {
        Some(default_path.to_string())
    } else {
        tracing::info!("Loading screen .pak not found at {}", default_path);
        None
    }
}

pub(super) fn force_mission_launch(
    campaign: &mut Campaign,
    profiles: &mut std::sync::Arc<ProfileManager>,
    application_context: &ApplicationContext,
    args: &CliArgs,
) -> Result<Option<(usize, MissionLocation)>, String> {
    let Some(mission_name) = args.mission.as_deref() else {
        return Ok(None);
    };
    let proto_name = args
        .proto
        .clone()
        .or_else(|| {
            profiles
                .missions
                .iter()
                .find(|profile| profile.mission_filename.eq_ignore_ascii_case(mission_name))
                .map(|profile| profile.proto_level_filename.clone())
        })
        .unwrap_or_else(|| mission_name.to_owned());

    tracing::info!("--mission: launching `{mission_name}` with proto-level `{proto_name}`");

    let profiles_mut = std::sync::Arc::make_mut(profiles);
    if args.preserve_forced_mission_campaign {
        let idx = campaign
            .current_mission_idx
            .ok_or_else(|| "preserved capture campaign has no current mission".to_owned())?;
        let profile = campaign.missions[idx].profile(profiles_mut);
        if !profile.mission_filename.eq_ignore_ascii_case(mission_name)
            || !profile
                .proto_level_filename
                .eq_ignore_ascii_case(&proto_name)
        {
            return Err(format!(
                "preserved capture campaign mission {}/{} disagrees with requested {mission_name}/{proto_name}",
                profile.mission_filename, profile.proto_level_filename
            ));
        }
        return Ok(Some((idx, profile.location)));
    }
    campaign.reset(profiles_mut, application_context.sim_config().difficulty);
    if robin_engine::level_data::hackable_level_exists(mission_name) {
        // Hackable JSON levels are not part of the legacy campaign and
        // therefore have no preceding mission from which to inherit a gang.
        campaign.create_gang_from_pcs(
            "R",
            profiles_mut,
            application_context.sim_config().difficulty,
        );
    }
    if args.mission_start_map_output.is_some() {
        // Use the walkthrough's practical campaign teams where it gives one.
        // For optional missions, derive the recruited heroes from prerequisite
        // history and fill the remaining slots with useful Merry Men. This
        // avoids injecting heroes who cannot exist yet while still producing
        // representative maps beyond the Robin-only opening mission.
        // TODO(export-team): accept a campaign save/team preset when callers
        // need an exact player-selected lineup.
        let export_pcs = detect_demo_mode_with_context(application_context)
            .filter(|(demo_mission, _, _, _)| demo_mission.eq_ignore_ascii_case(mission_name))
            .map(|(_, _, pcs, _)| Ok(pcs.to_owned()))
            .unwrap_or_else(|| recommended_export_team(profiles_mut, mission_name))?;
        campaign.create_gang_from_pcs(
            &export_pcs,
            profiles_mut,
            application_context.sim_config().difficulty,
        );
    }
    let idx = campaign
        .force_next_mission_by_name(profiles_mut, mission_name, &proto_name, true)
        .ok_or_else(|| {
            format!("--mission: failed to force mission `{mission_name}` with proto `{proto_name}`")
        })?;
    campaign.current_mission_idx = Some(idx);
    let location = campaign.missions[idx].profile(profiles_mut).location;

    Ok(Some((idx, location)))
}

/// Pick a plausible team for a context-free mission-map export.
///
/// Codes follow `Campaign::create_gang_from_pcs`. The fixed entries are the
/// recommendations in Steven W. Carter's retail walkthrough. Optional
/// ambush/tactical missions have no per-map recommendations, so their roster
/// is inferred from completed prerequisite missions instead.
pub(super) fn recommended_export_team(
    profiles: &robin_engine::profiles::ProfileManager,
    mission_filename: &str,
) -> Result<String, String> {
    let fixed = match mission_filename.to_ascii_lowercase().as_str() {
        // The original final-outro launcher selects every VIP in the gang.
        // The mission prerequisite graph only describes reachability, not all
        // rescued heroes, so deriving this roster would omit required PCs.
        "sherwoodoutro" => Some("RJTSWM"),
        "h01_lin_vl" | "s01_not_vl" => Some("R"),
        "s02_lei_mp" | "h02_not_ec" | "h03_der_mk" | "s03_fob_mp" | "h04_lei_vl" => Some("RSBC"),
        "h05_lin_ec" => Some("RSWBC"),
        "s04_der_ec" => Some("RJSB"),
        "h07_not_mk" => Some("R"),
        "str02_der_mp" => Some("RJWTB"),
        "s05_yrk_ec" => Some("RJTB"),
        "h09_not_vl" => Some("MJTB"),
        "h10_yor_vl" | "str03_yor_mk" => Some("RJTMB"),
        "h12_not_mp" => Some("RJTMW"),
        _ => None,
    };
    if let Some(team) = fixed {
        return Ok(team.to_owned());
    }

    let target = profiles
        .missions
        .iter()
        .find(|profile| {
            profile
                .mission_filename
                .eq_ignore_ascii_case(mission_filename)
        })
        .ok_or_else(|| {
            format!("mission-map team: no mission profile found for {mission_filename:?}")
        })?;

    let mut completed = std::collections::HashSet::new();
    let mut pending = target.missions_required_to_be_done.clone();
    while let Some(id) = pending.pop() {
        if !completed.insert(id) {
            continue;
        }
        let profile = profiles
            .missions
            .iter()
            .find(|profile| profile.id == id)
            .ok_or_else(|| {
                format!(
                    "mission-map team: prerequisite mission profile id {id} referenced by {:?} was not found",
                    target.mission_filename
                )
            })?;
        pending.extend(profile.missions_required_to_be_done.iter().copied());
    }

    let recruited = |rescue_filename: &str| {
        profiles.missions.iter().any(|profile| {
            profile
                .mission_filename
                .eq_ignore_ascii_case(rescue_filename)
                && completed.contains(&profile.id)
        })
    };

    // Prefer the guide's generally strongest/useful lineup. MerryManB is the
    // healer and MerryManC the strong body-carrier used in early missions.
    let mut team = String::from("R");
    for (rescue, code) in [
        ("S03_FoB_MP", 'J'),
        ("S04_Der_EC", 'T'),
        ("S05_Yrk_EC", 'M'),
        ("S02_Lei_MP", 'W'),
        ("S01_Not_VL", 'S'),
    ] {
        if recruited(rescue) && team.len() < 4 {
            team.push(code);
        }
    }
    team.push('B');
    if team.len() < 5 {
        team.push('C');
    }
    Ok(team)
}
