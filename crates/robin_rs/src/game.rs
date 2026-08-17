//! Game state manager.
//!
//! The top-level shell around the engine. It owns the UI, drives the
//! main game loop, and manages transitions between menus, loading,
//! gameplay, and level results. This module covers the **state
//! management** and **transition logic** for those flows.

use crate::host::Host;
use crate::host::{ApplicationContext, HostSignal};
use robin_engine::engine as engine_api;
#[cfg(test)]
use robin_engine::mission_stat as engine_mission_stat;
use robin_engine::player_command as engine_player_command;
use robin_engine::profiles as engine_profiles;
use serde::{Deserialize, Serialize};

use crate::app_effect::AppEffect;
use crate::stature_hud::StatureFocusLatch;
use robin_engine::campaign::Campaign;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::game_operation::{GameCode, GameOperationState};
use robin_engine::profiles::MissionLocation;

// ─── Constants ──────────────────────────────────────────────────────

// `PANNEL_HEIGHT` lives on the engine (`robin_engine::engine::PANNEL_HEIGHT`)
// as the single source of truth.  Host code imports it from there; the
// duplicate `u16` copy that used to live here drifted out of sync
// (stuck at 165 — the *corner-sprite* height) and is gone.
pub const PANNEL_TOP_RIGHT_WIDTH: u16 = 46;
pub const PANNEL_BOTTOM_LEFT_WIDTH: u16 = 320;
pub const PANNEL_BOTTOM_LEFT_HEIGHT: u16 = 165;
pub const PANNEL_BOTTOM_RIGHT_WIDTH: u16 = 320;
pub const PANNEL_BOTTOM_RIGHT_HEIGHT: u16 = 165;
pub const PANNEL_BOTTOM_CENTER_HEIGHT: u16 = 110;

pub const NUMBER_OF_SAMPLES: usize = 10;

// ─── Game State (serializable subset) ───────────────────────────────

/// Persistent game flags that survive save/load.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GamePersistentState {
    /// Whether the campaign map overlay is active.
    pub campaign_map_active: bool,
    /// Whether the campaign map has been displayed at least once.
    pub campaign_map_displayed: bool,
    /// Whether men-to-blazon conversion mode is active.
    pub men_to_blazon_conversion: bool,
    /// Start-mission widget enabled flag.
    pub start_mission_enabled: bool,
    /// Quit-mission widget enabled flag.
    pub quit_mission_enabled: bool,
    /// Start-mission transient-disable override.
    pub start_mission_disabled_temp: bool,
    /// Quit-mission transient-disable override.
    pub quit_mission_disabled_temp: bool,
    /// Debug "draw hidden" toggle.  The runtime copy lives on
    /// [`InputState::draw_hidden`]; save/load plumbing in
    /// [`GameSaveFile::capture_with_game`] / [`GameSaveFile::apply_to_with_game`]
    /// copies the value in and out so the toggle round-trips.
    pub draw_hidden: bool,
}

fn required_current_mission_id(
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    context: &str,
) -> u32 {
    let mission_id = campaign
        .current_mission_idx
        .and_then(|idx| campaign.missions.get(idx))
        .map(|mission| mission.profile(profiles).id)
        .unwrap_or_else(|| panic!("{context}: campaign must have a current mission"));
    assert_ne!(mission_id, 0, "{context}: mission ID zero is invalid");
    mission_id
}

// ─── Game (full runtime state) ──────────────────────────────────────

/// Full runtime state for the game session.  Non-serializable fields
/// (engine pointers, widget state, etc.) are excluded.
#[derive(Debug, Clone)]
pub struct Game {
    // ── Persistent state ──
    pub persistent: GamePersistentState,

    // ── Operation / state machine ──
    pub operation: GameOperationState,

    // ── Transient flags ──
    pub is_sherwood: bool,
    pub continue_requested: bool,
    /// Pending campaign-map reshow request.  The campaign-map overlay
    /// handler checks and clears this to re-open the modal at the new
    /// resolution after an options-menu resolution change.
    pub campaign_map_redisplay: bool,
    pub restore_sound: bool,
    pub mouse_was_enabled: bool,
    pub quick_load_after_zoom: bool,
    pub quick_save_after_zoom: bool,

    // ── Resolution ──
    pub width: u16,
    pub height: u16,

    // ── QA level ──
    pub level_of_qa: u16,

    // ── Pending message ──
    pub message_text: String,
    pub message_delay: u32,

    // ── Stature arrow widget focus latch ──
    //
    // Marks "the player has pressed a stature-change widget and the
    // sim transition is still in flight" so the persistent-widget
    // simulator latches the initiating arrow into a visually-pressed
    // state for the duration.
    pub stature_focus: StatureFocusLatch,

    // ── Frame timing ──
    pub frame_times: [u32; NUMBER_OF_SAMPLES],
    pub last_tick: u32,

    /// Explicit application context. `Deref<GlobalOptions>` preserves the
    /// existing directory/launcher option reads. Deterministic commands copy
    /// the specific gameplay configuration values they need at creation time.
    pub global_options: ApplicationContext,
}

impl Default for Game {
    fn default() -> Self {
        Self {
            persistent: GamePersistentState::default(),
            operation: GameOperationState::new(),
            is_sherwood: false,
            continue_requested: false,
            campaign_map_redisplay: false,
            restore_sound: true,
            mouse_was_enabled: true,
            quick_load_after_zoom: false,
            quick_save_after_zoom: false,
            width: 0,
            height: 0,
            level_of_qa: 0,
            message_text: String::new(),
            message_delay: 0,
            stature_focus: StatureFocusLatch::default(),
            frame_times: [0; NUMBER_OF_SAMPLES],
            last_tick: 0,
            global_options: ApplicationContext::default(),
        }
    }
}

impl Game {
    /// Create a new game state for the given mission.
    pub fn new(mission_location: MissionLocation) -> Self {
        Self {
            is_sherwood: mission_location == MissionLocation::Sherwood,
            ..Self::default()
        }
    }

    // ── State machine ───────────────────────────────────────────────

    /// Process the current game operation code and determine the next
    /// state.
    ///
    /// Returns `Some(code)` if the game loop should exit with that code,
    /// or `None` if the loop should continue.
    pub fn process_operation(
        &mut self,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
        callbacks: &mut dyn GameCallbacks,
    ) -> Option<GameCode> {
        match self.operation.get_current() {
            GameCode::LevelLoad => self.handle_level_load(campaign, profiles, callbacks),
            GameCode::LevelSave => {
                callbacks.serialize_save(campaign, profiles);
                self.operation.set(GameCode::LevelInProgress);
                None
            }
            GameCode::LevelRestart => {
                callbacks.serialize_for_restart(false);
                self.operation.set(GameCode::LevelInProgress);
                None
            }
            GameCode::Quit => {
                self.handle_quit(campaign, profiles, callbacks);
                Some(GameCode::Quit)
            }
            GameCode::LevelNext => {
                // Original: `original-code/RHgame.cpp`,
                // `RHGame::GameLoop`, `RHGAME_LEVEL_NEXT` applies these
                // in this exact order before returning.
                callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Menu));
                callbacks.emit_app_effect(AppEffect::SetMouseEnabled(true));
                Some(GameCode::LevelNext)
            }
            GameCode::LevelSucceeded => self.handle_level_succeeded(campaign, profiles, callbacks),
            GameCode::LevelFailed | GameCode::LevelInterrupted => {
                self.handle_level_failed(campaign, profiles, callbacks)
            }
            GameCode::LevelInProgress => None,
        }
    }

    fn handle_level_load(
        &mut self,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
        callbacks: &mut dyn GameCallbacks,
    ) -> Option<GameCode> {
        if !callbacks.save_game_file_exists() {
            self.operation.set(GameCode::LevelInProgress);
            return None;
        }

        let save_mission_id = callbacks.save_game_mission_id();
        let current_mission_id =
            required_current_mission_id(campaign, profiles, "Game::handle_level_load");

        if save_mission_id == current_mission_id {
            callbacks.serialize_load(save_mission_id);
            self.operation.set(GameCode::LevelInProgress);

            // In sherwood, re-run production sector initialization.
            if self.is_sherwood {
                callbacks.send_script_message(0, 1001);
            }
            None
        } else {
            // Different mission — must exit and reload at a higher level.
            Some(GameCode::LevelLoad)
        }
    }

    fn handle_quit(
        &mut self,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
        callbacks: &mut dyn GameCallbacks,
    ) {
        callbacks.suspend_play_time();
        callbacks.synchronize_profile_with_campaign(campaign, profiles);

        // Create the "continue" save game.
        let mission_id = required_current_mission_id(campaign, profiles, "Game::handle_quit");
        callbacks.serialize_continue_save(mission_id);
        callbacks.save_profiles();
    }

    fn handle_level_succeeded(
        &mut self,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
        callbacks: &mut dyn GameCallbacks,
    ) -> Option<GameCode> {
        callbacks.suspend_play_time();
        callbacks.synchronize_profile_with_campaign(campaign, profiles);
        // Original: `original-code/RHgame.cpp`, `RHGame::GameLoop`,
        // `RHGAME_LEVEL_SUCCEEDED` plays the jingle before menu mode.
        callbacks.emit_app_effect(AppEffect::PlayJingle(Jingle::MissionWon));

        // Display debriefing if campaign not over.
        if campaign.get_ares() < 10 {
            callbacks.display_debriefing(true);
        }

        callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Menu));

        if callbacks.is_loading_requested() {
            let load_code = callbacks.get_debriefing_game_code();
            if load_code == GameCode::LevelLoad {
                self.operation.set(GameCode::LevelLoad);
            }
            callbacks.start_play_time();
            callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
            None
        } else {
            Some(self.operation.get_current())
        }
    }

    fn handle_level_failed(
        &mut self,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
        callbacks: &mut dyn GameCallbacks,
    ) -> Option<GameCode> {
        let was_interrupted = self.operation.get_current() == GameCode::LevelInterrupted;

        callbacks.suspend_play_time();
        callbacks.synchronize_profile_with_campaign(campaign, profiles);
        // Original: `original-code/RHgame.cpp`, `RHGame::GameLoop`,
        // `RHGAME_LEVEL_FAILED` / `RHGAME_LEVEL_INTERRUPTED` use the
        // same jingle-before-menu ordering as the success branch.
        callbacks.emit_app_effect(AppEffect::PlayJingle(Jingle::MissionLost));

        // Display debriefing if campaign not over.
        if campaign.get_ares() < 10 {
            callbacks.display_debriefing(false);
        }

        callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Menu));

        if callbacks.is_loading_requested() {
            let load_code = callbacks.get_debriefing_game_code();
            if load_code == GameCode::LevelLoad {
                self.operation.set(GameCode::LevelLoad);
            }
            callbacks.start_play_time();
            callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
            None
        } else if was_interrupted {
            Some(GameCode::LevelInterrupted)
        } else {
            Some(GameCode::Quit)
        }
    }

    // ── Campaign map display ────────────────────────────────────────

    /// Request the campaign-map overlay to be raised.
    ///
    /// Intentionally does NOT touch `campaign_map_displayed` — that flag
    /// is flipped on inside the modal-open path (see
    /// [`Self::mark_campaign_map_displayed`]) at the moment the modal
    /// actually opens.  Keeping that split preserves the save/load
    /// invariant: a save taken in the "requested but not yet opened"
    /// window reloads without the overlay flagged as displayed.
    pub fn show_campaign_map(&mut self) {
        self.persistent.campaign_map_active = true;
    }

    /// Request the campaign-map overlay to be torn down and re-opened
    /// at the new resolution.  Used after the options menu commits a
    /// resolution change so the modal rebuilds against the new screen
    /// size.
    ///
    /// The overlay is a blocking modal (`show_campaign_map` in
    /// `campaign_map.rs`) rather than a retained window, so no
    /// explicit close is needed — the handler in `game_session.rs`
    /// checks `campaign_map_redisplay` after the inner loop returns
    /// and re-enters without clearing `campaign_map_active` when the
    /// flag is set.
    pub fn reshow_campaign_map(&mut self) {
        if self.persistent.campaign_map_active {
            self.campaign_map_redisplay = true;
        }
    }

    /// Mark the campaign-map overlay as displayed (first-frame open).
    /// Called from the modal-open path, not from the `show_campaign_map`
    /// setter (see that method's docs for why).
    pub fn mark_campaign_map_displayed(&mut self) {
        self.persistent.campaign_map_displayed = true;
    }

    /// Take and clear the pending redisplay request.  Returns `true`
    /// when a reshow was queued — the overlay handler uses this to
    /// decide whether to re-enter the inner loop at the new resolution
    /// instead of dismissing the modal.
    pub fn take_campaign_map_redisplay(&mut self) -> bool {
        std::mem::take(&mut self.campaign_map_redisplay)
    }

    // ── Menu display ────────────────────────────────────────────────

    /// Request the in-game menu to be displayed next frame.
    pub fn request_ingame_menu(&mut self) {
        self.continue_requested = true;
    }

    // ── Widget enable/disable ───────────────────────────────────────

    pub fn enable_start_mission(&mut self, enable: bool) {
        self.persistent.start_mission_enabled = enable;
    }

    pub fn enable_quit_mission(&mut self, enable: bool) {
        self.persistent.quit_mission_enabled = enable;
    }

    pub fn disable_start_mission_temp(&mut self, disable: bool) {
        self.persistent.start_mission_disabled_temp = disable;
    }

    pub fn disable_quit_mission_temp(&mut self, disable: bool) {
        self.persistent.quit_mission_disabled_temp = disable;
    }

    pub fn start_mission_disabled_temp(&self) -> bool {
        self.persistent.start_mission_disabled_temp
    }

    pub fn quit_mission_disabled_temp(&self) -> bool {
        self.persistent.quit_mission_disabled_temp
    }

    /// Effective enabled state of the start mission widget.
    pub fn is_start_mission_effectively_enabled(&self) -> bool {
        self.persistent.start_mission_enabled && !self.persistent.start_mission_disabled_temp
    }

    // ── Resolution ──────────────────────────────────────────────────

    /// Update the stored screen resolution.
    pub fn set_resolution(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    // ── Post-load sync ──────────────────────────────────────────────

    /// Apply post-load state fix-ups that depend on the slot type:
    ///
    /// - If the player loaded from the Continue auto-save, arm the
    ///   in-game menu so it opens on the next frame.
    /// - Restore `campaign_map_active` from `campaign_map_displayed` so
    ///   the campaign-map overlay reopens if it was visible at save time.
    ///
    /// Information-bar / resolution recomputes are widget layout which
    /// the immediate-mode HUD recomputes per-frame from engine state, so
    /// there's nothing runtime-critical to re-validate beyond what
    /// `Engine::restore` already does.  We still queue an
    /// `UpdateInformationBars` on the engine's command bus (folded into
    /// [`Engine::restore`]) so script-visible side effects (blazon-bar
    /// / requirements recompute logs) fire on the next tick.
    pub fn apply_post_load_sync(&mut self, is_continue: bool) {
        if is_continue {
            self.continue_requested = true;
        }
        self.persistent.campaign_map_active = self.persistent.campaign_map_displayed;

        // Re-dispatch the start/quit mission enable calls.  The
        // immediate-mode HUD polls `is_*_mission_effectively_enabled()`
        // each frame so this re-dispatch is purely for any script hook
        // / sound cue that listens for the enable/disable call itself.
        // Today the setters only flip the persistent flags (no side
        // effects), so calling them is idempotent but still documents
        // intent.
        let start_effective =
            self.persistent.start_mission_enabled && !self.persistent.start_mission_disabled_temp;
        let quit_effective =
            self.persistent.quit_mission_enabled && !self.persistent.quit_mission_disabled_temp;
        self.enable_start_mission(start_effective);
        self.enable_quit_mission(quit_effective);

        // Posture (`MSG_STATURE`) and cached-action (`MSG_SELECT_ACTION`)
        // re-broadcasts are pushed onto the messenger queue by
        // `EngineInner::post_load_fixups`, which runs from
        // `Engine::restore` on the save-load path before this helper
        // fires — so script subscribers see posture + action re-broadcast
        // edges on the next tick drain.
    }

    /// Re-apply the HUD layout after a save-load.  The
    /// immediate-mode HUD re-derives layout from engine state every
    /// frame, so all this needs to do is re-store the resolution so
    /// downstream consumers read the intended value; the
    /// information-bars script-host queue is filled automatically by
    /// [`Engine::restore`].
    pub fn post_load_resolution_resync(&mut self) {
        let (w, h) = (self.width, self.height);
        self.set_resolution(w, h);
    }

    // ── Blazon conversion ───────────────────────────────────────────

    /// Set the persistent men-to-blazon conversion flag.  Callers who
    /// also need the engine-owned mission UI mode updated must dispatch
    /// [`PlayerCommand::SetMenToBlazonConversionMode`] through
    /// `engine.apply_command` so replay / rollback see the toggle at
    /// the same frame.
    pub fn set_men_to_blazon_conversion(&mut self, enabled: bool) {
        self.persistent.men_to_blazon_conversion = enabled;
    }

    pub fn is_men_to_blazon_conversion(&self) -> bool {
        self.persistent.men_to_blazon_conversion
    }

    // ── Message display ─────────────────────────────────────────────

    /// Queue a text message for on-screen display.
    pub fn display_message(&mut self, text: String, delay: u32) {
        self.message_text = text;
        self.message_delay = delay;
    }

    // ── Engine integration ──────────────────────────────────────────

    /// Run one frame of the engine tick and update game operation.
    ///
    /// This bridges the Game state machine and the Engine update loop.
    /// The Game decides whether the engine should tick (via
    /// `should_run_hourglass`), and if the engine reports a state
    /// change, the Game updates its operation code.
    ///
    /// Returns `Some(code)` if the mission ended this frame, `None`
    /// if still in progress.
    #[allow(clippy::too_many_arguments)]
    pub fn run_engine_tick(
        &mut self,
        host: &mut Host,
        display: &mut engine_api::HostDisplayState,
        assets: &LevelAssets,
        engine: &mut Engine,
        dev: &mut engine_api::DevState,
        console_displayed: bool,
        dummy_pause: bool,
    ) -> Option<GameCode> {
        let mission_transitioning = !self.operation.is(GameCode::LevelInProgress);

        if !self.should_run_hourglass(console_displayed, mission_transitioning, dummy_pause) {
            return None;
        }

        // Start/quit-mission widget disable-temp state driven by PC
        // guarded status.
        let pc_guarded = engine.is_pc_guarded();
        self.disable_start_mission_temp(pc_guarded);
        self.disable_quit_mission_temp(pc_guarded);

        // Advance host-side animation phases.  Gated on the same
        // `should_run_hourglass` check above so pause / console freeze
        // them just like the sim tick.
        //
        // The selection ring's ping-pong frame counter only advances
        // when at least one selected PC actually receives a circle
        // this frame — frames with no visible mark must leave the
        // animation paused.  Without this gate the ring would keep
        // ticking while in a building / flying / nothing selected,
        // and re-selecting after an idle period would jump to a
        // different visible frame.
        if engine.any_selection_drawing_selection_mark(host.transport.local_seat) {
            host.selection_mark.tick();
        }
        let viewport = &host.frontend.viewport;
        let view_position = viewport.view_position.to_geo();
        let zoom_factor = viewport.zoom_factor;
        let screen_width = viewport.screen_size.x as i32;
        let screen_height = viewport.screen_size.y as i32;
        host.frontend.trajectory_ground_mark.tick(
            view_position,
            zoom_factor,
            screen_width,
            screen_height,
            engine.frame_counter(),
        );

        let result = crate::sim_timeline::run_engine_tick_core(host, display, assets, engine, dev);

        // The messenger reset path consumes the FPS-cheat flag and
        // promotes it into the debug-info overlay, so toggling the FPS
        // cheat arms the next input reset to leave the info overlay
        // visible.  The cheat flag lives on `DevState::debug.fps_display`
        // while the overlay flag lives on `Host::info_displayed`; the
        // engine-side `reset_input` side-effect is consumed by
        // `apply_side_effects` (which can only see `Host`), so it parks
        // `pending_fps_cheat_promote` for us to apply here where both
        // halves are in scope.
        if host.effects.take_signal(HostSignal::PromoteFpsCheat) {
            host.info_displayed = dev.debug.fps_display;
            dev.debug.fps_display = false;
        }

        // Ambush/tactical silent win swaps the Sherwood start/quit
        // mission widgets.  The engine routes the request through
        // `SideEffects`, the host drains it into
        // `pending_silent_win_widget_swap`, and we apply it here where
        // `&mut self` can mutate the widget-enable flags.
        if host.effects.take_signal(HostSignal::SilentWinWidgetSwap) {
            self.enable_start_mission(true);
            self.enable_quit_mission(false);
        }

        // First-time mission-won banner.  The engine tick flagged the
        // notice; we disable the quit-mission widget here.  The popup
        // itself is blocking — driven from the main game loop where
        // `&mut crate::window::GameWindow` is in scope.  The host-side
        // `pending_mission_state_popup` flag stays set until the main
        // loop shows and dismisses the popup.
        if host.effects.take_signal(HostSignal::MissionStateNotice) {
            self.enable_quit_mission(false);
        }

        if result != GameCode::LevelInProgress {
            self.operation.set(result);
            Some(result)
        } else {
            None
        }
    }

    /// Set up the per-mission game-state (is_sherwood, widget flags,
    /// operation state machine) from the campaign's current mission.
    ///
    /// The engine's own level load (`initialize_from_campaign` +
    /// `initialize`) is folded into the `Engine::new` constructor now,
    /// so this function is pure game-state work — no engine mutation.
    pub fn initialize_for_mission(
        &mut self,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
    ) {
        let idx = campaign
            .current_mission_idx
            .expect("initialize_for_mission: no current mission");
        let location = campaign.missions[idx].profile(profiles).location;

        self.is_sherwood = location == MissionLocation::Sherwood;
        self.operation = GameOperationState::new();

        // Enable/disable widgets based on mission type
        self.enable_start_mission(!self.is_sherwood);
        self.enable_quit_mission(!self.is_sherwood);
    }

    /// Convenience: apply quit updates then process the operation state machine.
    ///
    /// Used in tests; the real game session dispatches
    /// [`PlayerCommand::ApplyQuitMissionUpdates`] and runs
    /// `process_operation` at different points in the frame.
    pub fn finalize_mission(
        &mut self,
        mut engine: Engine,
        assets: &engine_api::LevelAssets,
        callbacks: &mut dyn GameCallbacks,
    ) -> (Option<GameCode>, Campaign) {
        // The engine already owns the one required mission campaign. Apply
        // quit updates to that exact allocation, then return it to the test
        // harness's outer owner. Installing the separate placeholder here
        // would replace the authoritative campaign and violate PA-037's
        // ownership invariant.
        let mut input = engine_api::InputState::default();
        let mut display = engine_api::HostDisplayState::default();
        engine.apply_command(
            &mut display,
            &mut input,
            assets,
            &engine_player_command::PlayerCommand::ApplyQuitMissionUpdates {
                exit_code: self.operation.get_current(),
                difficulty: self.global_options.sim_config().difficulty,
            },
        );
        let mut campaign = engine.into_campaign();
        let result = self.process_operation(&mut campaign, &assets.profile_manager, callbacks);
        (result, campaign)
    }

    // ── Hourglass / engine tick ─────────────────────────────────────

    /// Should the engine hourglass (game tick) run this frame?
    pub fn should_run_hourglass(
        &self,
        console_displayed: bool,
        mission_state_in_transition: bool,
        dummy_pause: bool,
    ) -> bool {
        !console_displayed
            && !mission_state_in_transition
            && !dummy_pause
            && !self.operation.is(GameCode::LevelNext)
            && !self.operation.is(GameCode::LevelLoad)
    }
}

// ─── Callback traits ────────────────────────────────────────────────

pub use crate::app_effect::{Jingle, SoundMode};

/// Callbacks the game state machine fires at transition points.
pub trait GameCallbacks {
    // ── Serialization ──
    fn serialize_save(&mut self, campaign: &Campaign, profiles: &engine_profiles::ProfileManager);
    fn serialize_load(&mut self, mission_id: u32);
    fn serialize_for_restart(&mut self, write: bool);
    fn serialize_continue_save(&mut self, mission_id: u32);
    fn save_profiles(&mut self);
    fn synchronize_profile_with_campaign(
        &mut self,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
    );

    // ── Save game queries ──
    fn save_game_file_exists(&self) -> bool;
    fn save_game_mission_id(&self) -> u32;

    // ── Ordered host effects ──
    fn emit_app_effect(&mut self, effect: AppEffect);

    // ── Script ──
    fn send_script_message(&mut self, target: u32, message: u32);

    // ── UI / menus ──
    fn display_ingame_menu(&mut self);
    fn display_debriefing(&mut self, won: bool);

    // ── Debriefing result ──
    fn is_loading_requested(&self) -> bool;
    fn get_debriefing_game_code(&self) -> GameCode;

    // ── Play time ──
    ///
    /// Lifecycle marker retained for parity with the original game
    /// state machine. Mission length itself advances from deterministic
    /// simulation frames, not host wall-clock time.
    fn start_play_time(&mut self);

    /// Lifecycle marker for pausing/stopping play-time accounting. Paused
    /// frames do not run the simulation tick, so no explicit wall-clock
    /// accumulation is needed.
    fn suspend_play_time(&mut self);

    /// Return the mission's total elapsed simulation time in **seconds**.
    fn get_current_playing_time(&self, campaign: &Campaign) -> u32;
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::campaign::CampaignValue;

    fn fresh_engine() -> (Engine, engine_api::LevelAssets) {
        use robin_engine::campaign::Campaign;
        let mut assets = engine_api::LevelAssets::new();
        let engine =
            Engine::new_for_test(800.0, 600.0, Campaign::default(), &mut assets).expect("engine");
        (engine, assets)
    }

    #[test]
    fn default_game_state() {
        let game = Game::default();
        assert!(game.operation.is(GameCode::LevelInProgress));
        assert!(!game.is_sherwood);
        assert!(!game.persistent.campaign_map_active);
    }

    #[test]
    fn new_sherwood_game() {
        let game = Game::new(MissionLocation::Sherwood);
        assert!(game.is_sherwood);
        assert!(game.operation.is(GameCode::LevelInProgress));
    }

    #[test]
    fn new_non_sherwood_game() {
        let game = Game::new(MissionLocation::Nottingham);
        assert!(!game.is_sherwood);
    }

    #[test]
    fn widget_enable_logic() {
        let mut game = Game::default();

        game.enable_start_mission(true);
        assert!(game.is_start_mission_effectively_enabled());

        game.disable_start_mission_temp(true);
        assert!(!game.is_start_mission_effectively_enabled());

        game.disable_start_mission_temp(false);
        assert!(game.is_start_mission_effectively_enabled());

        game.enable_start_mission(false);
        assert!(!game.is_start_mission_effectively_enabled());
    }

    #[test]
    fn hourglass_conditions() {
        let game = Game::default();

        // Normal state — hourglass should run.
        assert!(game.should_run_hourglass(false, false, false));

        // Console displayed — skip.
        assert!(!game.should_run_hourglass(true, false, false));

        // Mission state transition — skip.
        assert!(!game.should_run_hourglass(false, true, false));

        // Dummy pause — skip.
        assert!(!game.should_run_hourglass(false, false, true));
    }

    #[test]
    fn hourglass_skips_during_level_next() {
        let mut game = Game::default();
        game.operation.set(GameCode::LevelNext);
        assert!(!game.should_run_hourglass(false, false, false));
    }

    #[test]
    fn men_to_blazon_toggle() {
        let mut game = Game::default();
        assert!(!game.is_men_to_blazon_conversion());
        game.set_men_to_blazon_conversion(true);
        assert!(game.is_men_to_blazon_conversion());
    }

    #[test]
    fn persistent_state_serde() {
        let state = GamePersistentState {
            campaign_map_active: true,
            campaign_map_displayed: true,
            men_to_blazon_conversion: false,
            start_mission_enabled: true,
            quit_mission_enabled: false,
            start_mission_disabled_temp: true,
            quit_mission_disabled_temp: false,
            draw_hidden: true,
        };
        let json = serde_json::to_string(&state).unwrap();
        let back: GamePersistentState = serde_json::from_str(&json).unwrap();
        assert!(back.campaign_map_active);
        assert!(!back.men_to_blazon_conversion);
        assert!(back.start_mission_enabled);
        assert!(back.start_mission_disabled_temp);
        assert!(back.draw_hidden);
    }

    #[test]
    fn persistent_state_rejects_missing_current_v52_fields() {
        let json = r#"{
            "campaign_map_active": false,
            "campaign_map_displayed": false,
            "men_to_blazon_conversion": false
        }"#;
        assert!(serde_json::from_str::<GamePersistentState>(json).is_err());
    }

    #[test]
    fn message_display() {
        let mut game = Game::default();
        game.display_message("Test message".into(), 5000);
        assert_eq!(game.message_text, "Test message");
        assert_eq!(game.message_delay, 5000);
    }

    #[test]
    fn menu_request() {
        let mut game = Game::default();
        assert!(!game.continue_requested);
        game.request_ingame_menu();
        assert!(game.continue_requested);
    }

    /// Stub callbacks for testing state transitions.
    struct StubCallbacks {
        save_exists: bool,
        save_mission_id: u32,
        loading_requested: bool,
        debriefing_code: GameCode,
        effects: Vec<AppEffect>,
    }

    impl Default for StubCallbacks {
        fn default() -> Self {
            Self {
                save_exists: false,
                save_mission_id: 0,
                loading_requested: false,
                debriefing_code: GameCode::LevelInProgress,
                effects: Vec::new(),
            }
        }
    }

    impl GameCallbacks for StubCallbacks {
        fn serialize_save(&mut self, _: &Campaign, _: &engine_profiles::ProfileManager) {}
        fn serialize_load(&mut self, _: u32) {}
        fn serialize_for_restart(&mut self, _: bool) {}
        fn serialize_continue_save(&mut self, _: u32) {}
        fn save_profiles(&mut self) {}
        fn synchronize_profile_with_campaign(
            &mut self,
            _: &Campaign,
            _: &engine_profiles::ProfileManager,
        ) {
        }
        fn save_game_file_exists(&self) -> bool {
            self.save_exists
        }
        fn save_game_mission_id(&self) -> u32 {
            self.save_mission_id
        }
        fn emit_app_effect(&mut self, effect: AppEffect) {
            self.effects.push(effect);
        }
        fn send_script_message(&mut self, _: u32, _: u32) {}
        fn display_ingame_menu(&mut self) {}
        fn display_debriefing(&mut self, _: bool) {}
        fn is_loading_requested(&self) -> bool {
            self.loading_requested
        }
        fn get_debriefing_game_code(&self) -> GameCode {
            self.debriefing_code
        }
        fn start_play_time(&mut self) {}
        fn suspend_play_time(&mut self) {}
        fn get_current_playing_time(&self, campaign: &Campaign) -> u32 {
            campaign.get_value(CampaignValue::MissionLength) as u32
        }
    }

    #[test]
    fn process_save_transitions_to_in_progress() {
        let mut game = Game::default();
        game.operation.set(GameCode::LevelSave);
        let campaign = Campaign::default();
        let profiles = engine_profiles::ProfileManager::new();
        let mut cb = StubCallbacks::default();

        let result = game.process_operation(&campaign, &profiles, &mut cb);
        assert!(result.is_none());
        assert!(game.operation.is(GameCode::LevelInProgress));
    }

    #[test]
    fn process_restart_transitions_to_in_progress() {
        let mut game = Game::default();
        game.operation.set(GameCode::LevelRestart);
        let campaign = Campaign::default();
        let profiles = engine_profiles::ProfileManager::new();
        let mut cb = StubCallbacks::default();

        let result = game.process_operation(&campaign, &profiles, &mut cb);
        assert!(result.is_none());
        assert!(game.operation.is(GameCode::LevelInProgress));
    }

    #[test]
    fn process_quit_returns_quit() {
        let mut game = Game::default();
        game.operation.set(GameCode::Quit);
        let mut campaign = Campaign::default();
        campaign.missions.push(robin_engine::mission::Mission {
            profile_idx: Some(0),
            ..Default::default()
        });
        campaign.current_mission_idx = Some(0);
        let mut profiles = engine_profiles::ProfileManager::new();
        profiles.missions.push(engine_profiles::MissionProfile {
            id: 42,
            ..Default::default()
        });
        let mut cb = StubCallbacks::default();

        let result = game.process_operation(&campaign, &profiles, &mut cb);
        assert_eq!(result, Some(GameCode::Quit));
    }

    #[test]
    #[should_panic(expected = "Game::handle_quit: campaign must have a current mission")]
    fn process_quit_rejects_missing_required_mission() {
        let mut game = Game::default();
        game.operation.set(GameCode::Quit);
        game.process_operation(
            &Campaign::default(),
            &engine_profiles::ProfileManager::new(),
            &mut StubCallbacks::default(),
        );
    }

    #[test]
    #[should_panic(expected = "Game::handle_level_load: campaign must have a current mission")]
    fn process_load_rejects_missing_required_mission_when_save_exists() {
        let mut game = Game::default();
        game.operation.set(GameCode::LevelLoad);
        game.process_operation(
            &Campaign::default(),
            &engine_profiles::ProfileManager::new(),
            &mut StubCallbacks {
                save_exists: true,
                save_mission_id: 1,
                ..Default::default()
            },
        );
    }

    #[test]
    fn process_level_next_returns_level_next() {
        let mut game = Game::default();
        game.operation.set(GameCode::LevelNext);
        let campaign = Campaign::default();
        let profiles = engine_profiles::ProfileManager::new();
        let mut cb = StubCallbacks::default();

        let result = game.process_operation(&campaign, &profiles, &mut cb);
        assert_eq!(result, Some(GameCode::LevelNext));
        assert_eq!(
            cb.effects,
            [
                AppEffect::SetSoundMode(SoundMode::Menu),
                AppEffect::SetMouseEnabled(true),
            ]
        );
    }

    #[test]
    fn process_load_no_file_resets() {
        let mut game = Game::default();
        game.operation.set(GameCode::LevelLoad);
        let campaign = Campaign::default();
        let profiles = engine_profiles::ProfileManager::new();
        let mut cb = StubCallbacks {
            save_exists: false,
            ..Default::default()
        };

        let result = game.process_operation(&campaign, &profiles, &mut cb);
        assert!(result.is_none());
        assert!(game.operation.is(GameCode::LevelInProgress));
    }

    #[test]
    fn process_failed_no_load_returns_quit() {
        let mut game = Game::default();
        game.operation.set(GameCode::LevelFailed);
        let campaign = Campaign::default();
        let profiles = engine_profiles::ProfileManager::new();
        let mut cb = StubCallbacks {
            loading_requested: false,
            ..Default::default()
        };

        let result = game.process_operation(&campaign, &profiles, &mut cb);
        assert_eq!(result, Some(GameCode::Quit));
        assert_eq!(
            cb.effects,
            [
                AppEffect::PlayJingle(Jingle::MissionLost),
                AppEffect::SetSoundMode(SoundMode::Menu),
            ]
        );
    }

    #[test]
    fn process_interrupted_no_load_returns_interrupted() {
        let mut game = Game::default();
        game.operation.set(GameCode::LevelInterrupted);
        let campaign = Campaign::default();
        let profiles = engine_profiles::ProfileManager::new();
        let mut cb = StubCallbacks {
            loading_requested: false,
            ..Default::default()
        };

        let result = game.process_operation(&campaign, &profiles, &mut cb);
        assert_eq!(result, Some(GameCode::LevelInterrupted));
    }

    #[test]
    fn process_failed_with_load_continues() {
        let mut game = Game::default();
        game.operation.set(GameCode::LevelFailed);
        let campaign = Campaign::default();
        let profiles = engine_profiles::ProfileManager::new();
        let mut cb = StubCallbacks {
            loading_requested: true,
            debriefing_code: GameCode::LevelLoad,
            ..Default::default()
        };

        let result = game.process_operation(&campaign, &profiles, &mut cb);
        assert!(result.is_none());
        assert!(game.operation.is(GameCode::LevelLoad));
        assert_eq!(
            cb.effects,
            [
                AppEffect::PlayJingle(Jingle::MissionLost),
                AppEffect::SetSoundMode(SoundMode::Menu),
                AppEffect::SetSoundMode(SoundMode::Mission),
            ]
        );
    }

    // ── Engine integration tests ────────────────────────────────

    #[test]
    fn run_engine_tick_in_progress() {
        let mut game = Game::default();
        let mut dev = engine_api::DevState::default();
        let (mut engine, assets) = fresh_engine();
        let mut host = Host::scratch(800.0, 600.0);
        let mut display = engine_api::HostDisplayState::default();

        // Normal state — engine should tick and return None (still in progress)
        let result = game.run_engine_tick(
            &mut host,
            &mut display,
            &assets,
            &mut engine,
            &mut dev,
            false,
            false,
        );
        assert!(result.is_none());
        assert!(game.operation.is(GameCode::LevelInProgress));
        assert_eq!(engine.frame_counter(), 1);
    }

    #[test]
    fn run_engine_tick_skips_when_paused() {
        let mut game = Game::default();
        let mut dev = engine_api::DevState::default();
        let (mut engine, assets) = fresh_engine();
        let mut host = Host::scratch(800.0, 600.0);
        let mut display = engine_api::HostDisplayState::default();

        // Paused — engine should NOT tick
        let result = game.run_engine_tick(
            &mut host,
            &mut display,
            &assets,
            &mut engine,
            &mut dev,
            false,
            true,
        );
        assert!(result.is_none());
        assert_eq!(engine.frame_counter(), 0); // Not incremented
    }

    #[test]
    fn run_engine_tick_skips_when_console() {
        let mut game = Game::default();
        let mut dev = engine_api::DevState::default();
        let (mut engine, assets) = fresh_engine();
        let mut host = Host::scratch(800.0, 600.0);
        let mut display = engine_api::HostDisplayState::default();

        // Console displayed — engine should NOT tick
        let result = game.run_engine_tick(
            &mut host,
            &mut display,
            &assets,
            &mut engine,
            &mut dev,
            true,
            false,
        );
        assert!(result.is_none());
        assert_eq!(engine.frame_counter(), 0);
    }

    #[test]
    fn run_engine_tick_mission_won() {
        let mut game = Game::default();
        let mut dev = engine_api::DevState::default();
        let (mut engine, assets) = fresh_engine();
        let mut host = Host::scratch(800.0, 600.0);
        let mut display = engine_api::HostDisplayState::default();
        engine.test_set_mission_flags(true, false, false);

        let result = game.run_engine_tick(
            &mut host,
            &mut display,
            &assets,
            &mut engine,
            &mut dev,
            false,
            false,
        );
        assert_eq!(result, Some(GameCode::LevelSucceeded));
        assert!(game.operation.is(GameCode::LevelSucceeded));
    }

    #[test]
    fn run_engine_tick_mission_lost() {
        let mut game = Game::default();
        let mut dev = engine_api::DevState::default();
        let (mut engine, assets) = fresh_engine();
        let mut host = Host::scratch(800.0, 600.0);
        let mut display = engine_api::HostDisplayState::default();
        engine.test_set_mission_flags(false, true, false);

        let result = game.run_engine_tick(
            &mut host,
            &mut display,
            &assets,
            &mut engine,
            &mut dev,
            false,
            false,
        );
        assert_eq!(result, Some(GameCode::LevelFailed));
        assert!(game.operation.is(GameCode::LevelFailed));
    }

    #[test]
    fn finalize_mission_does_not_double_credit_money_or_score() {
        // Before this change, finalize_mission re-transferred mission_stat
        // money/score into the campaign.  Now that those values are
        // credited continuously during gameplay through
        // `EngineInner::add_campaign_value`, finalize must NOT re-add
        // them or the player would be paid twice.
        let mut game = Game::default();
        game.operation.set(GameCode::LevelSucceeded);
        let (mut engine, assets) = fresh_engine();
        engine.test_set_mission_stat(engine_mission_stat::MissionStat {
            collected_money: 300,
            added_score: 500,
            ..Default::default()
        });

        let mut cb = StubCallbacks::default();

        let (_result, campaign) = game.finalize_mission(engine, &assets, &mut cb);

        assert_eq!(
            campaign.get_value(CampaignValue::Ransom),
            robin_engine::campaign::INITIAL_RANSOM
        );
        // A successful non-ambush mission earns the normal 1,000-point
        // completion bonus. The synthetic `added_score` above must not be
        // added again (which would make this 1,500).
        assert_eq!(campaign.get_value(CampaignValue::Score), 1000);
    }

    #[test]
    fn show_campaign_map_sets_active_only() {
        // `show_campaign_map` only flips `campaign_map_active` and leaves
        // `campaign_map_displayed` for the modal-open path to flip.
        let mut game = Game::default();
        game.show_campaign_map();
        assert!(game.persistent.campaign_map_active);
        assert!(!game.persistent.campaign_map_displayed);
    }

    #[test]
    fn reshow_campaign_map_only_arms_when_active() {
        // `reshow_campaign_map` is gated on `campaign_map_active` so it
        // is a no-op when no modal is currently open.
        let mut game = Game::default();
        game.reshow_campaign_map();
        assert!(!game.campaign_map_redisplay);
        game.show_campaign_map();
        game.reshow_campaign_map();
        assert!(game.take_campaign_map_redisplay());
        assert!(!game.campaign_map_redisplay);
    }
}
