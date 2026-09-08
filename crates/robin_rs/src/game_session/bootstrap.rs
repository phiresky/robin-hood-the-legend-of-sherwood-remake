//! Ordered construction of complete interactive and true-headless missions.

use super::flow::MissionServices;
use super::headless::{HeadlessMission, HeadlessMissionOutcome, HeadlessPolicy};
use super::interactive::{
    InteractiveFrontend, InteractiveFrontendAssembly, InteractiveMission,
    InteractiveRendererAssembly, MissionRendererConfig,
};
use super::replay_init::init_replay_and_rollback;
use super::runtime::{
    FrameContract, MissionControl, MissionRuntime, MissionWorld, TimelineRuntime,
};
use super::setup::{
    HeadlessEngineResources, LOADING_AUDIO_PROGRESS, LoadedInteractiveResources, LoadedMissionCore,
    MissionLoadError, MissionProcessResources, load_level_and_sprite_bank,
    pre_decode_maps_and_resources, setup_local_seat_and_multiplayer_snapshot, setup_mission_audio,
};
use super::{
    MissionOutcome, install_cold_save_lua_session, install_pending_lua_session,
    pending_cold_save_lua_launch, setup_multiplayer_session,
};
use crate::game::Game;
use crate::host::ApplicationContext;
use crate::host::Host;
use crate::loading_screen::{LoadingDatadirKind, LoadingScreenRenderer};
use crate::main_entry::{
    RustCallbacks, current_mission_id, detect_demo_mode_with_context, resolve_loading_pak,
};
use crate::window::GameWindow;
use robin_engine::campaign::Campaign;
use robin_engine::engine as engine_api;
use robin_engine::game_operation::GameCode;
use robin_engine::profiles::{MissionLocation, ProfileManager};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Which concrete frontend a mission bootstrap must produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum MissionFrontendKind {
    Interactive,
    Headless,
}

/// Pure, serializable inputs identifying one mission construction request.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(super) struct MissionSpec {
    pub(super) mission_idx: usize,
    pub(super) location: MissionLocation,
    pub(super) screen_width: f32,
    pub(super) screen_height: f32,
    pub(super) frontend: MissionFrontendKind,
}

impl MissionSpec {
    pub(super) fn interactive(
        mission_idx: usize,
        location: MissionLocation,
        screen_width: f32,
        screen_height: f32,
    ) -> Self {
        Self {
            mission_idx,
            location,
            screen_width,
            screen_height,
            frontend: MissionFrontendKind::Interactive,
        }
    }

    pub(super) fn headless(mission_idx: usize, location: MissionLocation) -> Self {
        Self {
            mission_idx,
            location,
            screen_width: 1024.0,
            screen_height: 768.0,
            frontend: MissionFrontendKind::Headless,
        }
    }
}

/// Process-owning setup state between CPU level load and frontend completion.
///
/// This deliberately does not implement serde. `Host`, decoded map upload
/// scratch, and level-asset caches exist only while constructing this process'
/// loaded mission.
pub(super) struct MissionBootstrap {
    pub(super) spec: MissionSpec,
    pub(super) host: Host,
    pub(super) game: Game,
    pub(super) loaded: LoadedMissionCore,
    lifecycle: MissionBootstrapLifecycle,
    restart_save_started: bool,
    restart_save_identity: Option<crate::save_file::ReplaySaveIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum MissionBootstrapPhase {
    LevelInitialized,
    SpellforgeStarted,
    AudioPrepared,
    CampaignClockStarted,
    EntryPrepared,
}

impl MissionBootstrapPhase {
    const fn can_advance_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::LevelInitialized, Self::SpellforgeStarted)
                | (Self::SpellforgeStarted, Self::AudioPrepared)
                | (Self::AudioPrepared, Self::CampaignClockStarted)
                | (Self::CampaignClockStarted, Self::EntryPrepared)
        )
    }
}

/// Runtime-enforced bootstrap state machine. The trace is diagnostic process
/// state only; deterministic save data remains entirely in the Engine.
struct MissionBootstrapLifecycle {
    phase: MissionBootstrapPhase,
    trace: Vec<MissionBootstrapPhase>,
}

impl MissionBootstrapLifecycle {
    fn new() -> Self {
        Self {
            phase: MissionBootstrapPhase::LevelInitialized,
            trace: vec![MissionBootstrapPhase::LevelInitialized],
        }
    }

    fn require(&self, expected: MissionBootstrapPhase) {
        assert_eq!(self.phase, expected);
    }

    fn advance(&mut self, expected: MissionBootstrapPhase, next: MissionBootstrapPhase) {
        self.require(expected);
        assert!(
            expected.can_advance_to(next),
            "invalid mission bootstrap transition: {expected:?} -> {next:?}"
        );
        self.phase = next;
        self.trace.push(next);
    }

    fn phase(&self) -> MissionBootstrapPhase {
        self.phase
    }

    #[cfg(test)]
    fn trace(&self) -> &[MissionBootstrapPhase] {
        &self.trace
    }
}

fn built_in_mission_assets_for_loaded_level(
    mission_id: &str,
    proto_level_filename: &str,
    loaded_map_filename: &str,
) -> robin_engine::mission_assets::MissionAssetDescriptor {
    robin_engine::mission_assets::MissionAssetDescriptor::built_in(
        mission_id,
        proto_level_filename,
        loaded_map_filename,
    )
    .expect("loaded built-in mission identity must produce a valid asset descriptor")
}

impl MissionBootstrap {
    pub(super) fn new(
        spec: MissionSpec,
        host: Host,
        game: Game,
        loaded: LoadedMissionCore,
        args: &crate::main_entry::CliArgs,
    ) -> Self {
        let mut bootstrap = Self {
            spec,
            host,
            game,
            loaded,
            lifecycle: MissionBootstrapLifecycle::new(),
            restart_save_started: false,
            restart_save_identity: None,
        };
        bootstrap.install_mission_assets(args);
        bootstrap
    }

    /// Run required Spellforge startup after SCB `Initialize` in the engine
    /// constructor and before audio/replay construction.
    pub(super) fn start_required_spellforge(
        &mut self,
    ) -> Result<(), crate::lua_session::SpellforgeSessionError> {
        self.lifecycle
            .require(MissionBootstrapPhase::LevelInitialized);
        if let Some(lua) = self.host.scripting.lua_session.as_ref() {
            tracing::info!(
                "Lua: deterministic runtime active for mission '{}' (seed={}); Initialize is owned by the engine callback driver",
                lua.mission_basename(),
                self.loaded.engine_rng_seed,
            );
        }
        self.lifecycle.advance(
            MissionBootstrapPhase::LevelInitialized,
            MissionBootstrapPhase::SpellforgeStarted,
        );
        Ok(())
    }

    pub(super) fn prepare_audio(
        &mut self,
        backend: Option<&mut crate::audio_backend::KiraAudioBackend>,
        profiles: &ProfileManager,
    ) -> Result<(), String> {
        self.lifecycle
            .require(MissionBootstrapPhase::SpellforgeStarted);
        setup_mission_audio(
            &mut self.host,
            backend,
            &self.loaded.engine,
            &mut self.loaded.assets,
            profiles,
            self.spec.location,
            &self.game.global_options.sound_directory,
        )?;
        self.lifecycle.advance(
            MissionBootstrapPhase::SpellforgeStarted,
            MissionBootstrapPhase::AudioPrepared,
        );
        Ok(())
    }

    /// Start the campaign segment clock after the lost-Sherwood gate, matching
    /// the original `GameLoop` boundary.
    pub(super) fn start_campaign_clock(&mut self) {
        self.lifecycle.require(MissionBootstrapPhase::AudioPrepared);
        self.loaded.engine.mission_setup().reset_mission_length();
        self.lifecycle.advance(
            MissionBootstrapPhase::AudioPrepared,
            MissionBootstrapPhase::CampaignClockStarted,
        );
    }

    /// Capture the pristine restart state for a tactical mission. Fresh
    /// Sherwood sessions leave the campaign map closed, matching the original
    /// original-game session startup; the player opens it from the HQ widget. This must
    /// be the last setup stage before replay/runtime construction.
    pub(super) fn setup_restart_or_sherwood(
        &mut self,
        callbacks: &mut RustCallbacks,
        args: &crate::main_entry::CliArgs,
    ) {
        self.lifecycle
            .require(MissionBootstrapPhase::CampaignClockStarted);
        if !self.game.is_sherwood && args.mission_start_map_output.is_none() {
            let campaign = self.loaded.engine.campaign();
            let mission_id = current_mission_id(campaign, &self.loaded.assets.profile_manager);
            self.restart_save_started = match callbacks.save_manager.write_restart_save_background(
                &mut self.host,
                &self.game,
                &self.loaded.engine,
                mission_id,
                Some(&self.loaded.assets.profile_manager),
                None,
            ) {
                Ok(()) => {
                    self.restart_save_identity = callbacks.save_manager.restart_session_identity();
                    true
                }
                Err(error) => {
                    tracing::error!("Restart save could not start: {error:#}");
                    false
                }
            };
        }
        self.lifecycle.advance(
            MissionBootstrapPhase::CampaignClockStarted,
            MissionBootstrapPhase::EntryPrepared,
        );
    }

    /// Admit an already-lost Sherwood mission into the outer frame driver
    /// without crossing either post-gate initialization boundary.
    ///
    /// The frame-owned lost-campaign debriefing needs a complete runtime so
    /// network/HTTP/replay services keep draining. Advancing only the type-
    /// state markers lets that runtime be constructed while deliberately not
    /// starting play time and not creating restart/Sherwood entry state.
    pub(super) fn defer_lost_sherwood_entry(&mut self) {
        self.lifecycle.require(MissionBootstrapPhase::AudioPrepared);
        assert!(self.game.is_sherwood, "only Sherwood entry can be deferred");
        assert_eq!(
            self.loaded.engine.campaign().get_ares(),
            0,
            "only an already-lost campaign can defer Sherwood entry"
        );
        self.lifecycle.advance(
            MissionBootstrapPhase::AudioPrepared,
            MissionBootstrapPhase::CampaignClockStarted,
        );
        self.lifecycle.advance(
            MissionBootstrapPhase::CampaignClockStarted,
            MissionBootstrapPhase::EntryPrepared,
        );
    }

    pub(super) fn finish_interactive(
        self,
        frontend: InteractiveFrontend,
        args: &crate::main_entry::CliArgs,
    ) -> InteractiveMission {
        assert_eq!(self.spec.frontend, MissionFrontendKind::Interactive);
        self.lifecycle.require(MissionBootstrapPhase::EntryPrepared);
        let wait_for_multiplayer_start = self.host.transport.net.is_some();
        InteractiveMission {
            runtime: self.finish_runtime(
                args,
                FrameContract::Graphical,
                wait_for_multiplayer_start,
            ),
            frontend,
            campaign_transition: None,
        }
    }

    async fn sign_ranked_session_before_frame_zero(&mut self, args: &crate::main_entry::CliArgs) {
        let custom_package_present = self.host.scripting.lua_session.is_some()
            || args.custom_mission.is_some()
            || args.pending_lua_mission.is_some();
        if let Some(net) = self.host.transport.net.as_ref() {
            self.loaded
                .ranked_admission
                .install_multiplayer_before_frame_zero(net, custom_package_present)
                .await;
        } else {
            self.loaded
                .ranked_admission
                .sign_before_frame_zero(custom_package_present)
                .await;
        }
    }

    pub(super) fn finish_headless(
        self,
        args: &crate::main_entry::CliArgs,
        policy: HeadlessPolicy,
    ) -> HeadlessMission {
        assert_eq!(self.spec.frontend, MissionFrontendKind::Headless);
        self.lifecycle
            .require(MissionBootstrapPhase::CampaignClockStarted);
        let wait_for_multiplayer_start = self.host.transport.net.is_some();
        HeadlessMission {
            modals: super::session_policy::SessionModalScheduler::default(),
            runtime: self.finish_runtime(args, FrameContract::Headless, wait_for_multiplayer_start),
            policy,
        }
    }

    /// Install the exact asset identity before any restart save can capture the game.
    fn install_mission_assets(&mut self, args: &crate::main_entry::CliArgs) {
        let campaign = self.loaded.engine.campaign();
        let mission = campaign
            .missions
            .get(self.spec.mission_idx)
            .unwrap_or_else(|| {
                panic!(
                    "mission bootstrap requires campaign mission index {} (campaign has {})",
                    self.spec.mission_idx,
                    campaign.missions.len()
                )
            });
        let mission_profile = mission.profile(&self.loaded.assets.profile_manager);
        let mission_id = mission_profile.mission_filename.clone();
        let loaded_map_filename = self.loaded.engine.mission_map_name().to_owned();
        let mission_assets = if let Some(resolved) = args.resolved_mission_assets.as_ref() {
            let descriptor = resolved.descriptor().clone();
            if let Some(replay) = args.replay_data.as_ref() {
                assert_eq!(
                    descriptor,
                    replay.header().mission_assets,
                    "replay mission asset lifetime differs from its persisted descriptor"
                );
            }
            descriptor
        } else if let Some(replay) = args.replay_data.as_ref() {
            replay.header().mission_assets.clone()
        } else {
            assert!(
                args.pending_lua_mission.is_none() && args.custom_mission.is_none(),
                "custom mission bootstrap requires an exact archive descriptor before engine construction"
            );
            built_in_mission_assets_for_loaded_level(
                &mission_id,
                &mission_profile.proto_level_filename,
                &loaded_map_filename,
            )
        };
        assert!(
            mission_id.eq_ignore_ascii_case(&mission_assets.mission_basename),
            "loaded mission `{mission_id}` does not match asset descriptor mission `{}`",
            mission_assets.mission_basename
        );
        assert!(
            mission_profile
                .proto_level_filename
                .eq_ignore_ascii_case(&mission_assets.proto_level_filename),
            "loaded mission proto `{}` does not match asset descriptor proto `{}`",
            mission_profile.proto_level_filename,
            mission_assets.proto_level_filename
        );
        assert!(
            loaded_map_filename.eq_ignore_ascii_case(&mission_assets.map_filename),
            "loaded RHM map `{loaded_map_filename}` does not match asset descriptor map `{}`",
            mission_assets.map_filename
        );
        self.game
            .set_mission_assets(mission_assets)
            .expect("mission bootstrap produced an invalid asset descriptor");
    }

    fn finish_runtime(
        mut self,
        args: &crate::main_entry::CliArgs,
        contract: FrameContract,
        wait_for_multiplayer_start: bool,
    ) -> MissionRuntime {
        assert!(matches!(
            self.lifecycle.phase(),
            MissionBootstrapPhase::CampaignClockStarted | MissionBootstrapPhase::EntryPrepared
        ));
        let mission_assets = self
            .game
            .mission_assets()
            .expect("mission assets must be installed before runtime construction")
            .clone();
        let mission_id = mission_assets.mission_basename.clone();
        let ranked_admission = self.loaded.ranked_admission.take_mission_admission();
        let ranked_multiplayer_port =
            self.host
                .transport
                .net
                .as_ref()
                .and_then(|net| match net.ranked_port() {
                    Ok(port) => Some(port),
                    Err(error) => {
                        tracing::warn!("ranked multiplayer mission-end port unavailable: {error}");
                        None
                    }
                });
        let leaderboard = (contract == FrameContract::Graphical).then(|| {
            super::leaderboard_runtime::MissionLeaderboardRuntime::new(
                &self.loaded.replay_campaign,
                mission_id.clone(),
                self.host.transport.net.is_some(),
                ranked_admission,
                ranked_multiplayer_port,
            )
        });
        let assets = Arc::new(self.loaded.assets);
        let replay = init_replay_and_rollback(
            &self.loaded.replay_campaign,
            Arc::clone(&assets),
            args,
            self.spec.mission_idx,
            &mission_id,
            mission_assets,
            self.loaded.engine_rng_seed,
            self.loaded.engine_sim_config,
            self.host.transport.net.is_some(),
        );
        let mut timeline = TimelineRuntime::new(
            replay,
            contract,
            wait_for_multiplayer_start,
            self.host.transport.local_seat == robin_engine::player_command::PlayerId::HOST,
        );
        debug_assert_eq!(timeline.frame_contract(), contract);
        // Entry eligibility is insufficient: capture/indexing can fail, and
        // headless startup can skip restart creation entirely. Only an admitted
        // background save represents a frame-0 payload that can later be loaded.
        timeline.register_bootstrap_save(
            &self.loaded.engine,
            &self.host,
            &self.game,
            self.restart_save_started,
            self.restart_save_identity,
        );
        let manager = robin_engine::engine_manager::EngineManager::new(self.loaded.engine);
        let dynamic_visuals = self
            .host
            .application_context()
            .active_profile_snapshot()
            .map(|profile| profile.graphic_config.dynamic_ambience_visuals)
            .unwrap_or(true);
        let visual_ambiance = if dynamic_visuals {
            manager.engine.weather().ambiance
        } else {
            manager.engine.initial_mission_ambiance()
        };
        let visual_shadow = if dynamic_visuals {
            manager.engine.weather().night_color
        } else {
            manager.engine.initial_mission_night_color()
        };
        let control =
            MissionControl::new(timeline.initially_paused(), visual_shadow, visual_ambiance);
        MissionRuntime::new(
            MissionWorld::new(self.host, self.game, manager, assets, self.loaded.dev),
            timeline,
            control,
            leaderboard,
        )
    }

    fn into_campaign_and_simulation(self) -> (Campaign, u64, engine_api::SimConfig) {
        self.loaded.engine.into_campaign_and_simulation()
    }
}

/// Diagnostic control for same-package startup measurements.
fn prepare_renderer_early() -> bool {
    #[cfg(target_arch = "wasm32")]
    let value = {
        let window = web_sys::window().expect("interactive renderer requires a browser window");
        let search = window
            .location()
            .search()
            .expect("read renderer preparation query");
        web_sys::UrlSearchParams::new_with_str(&search)
            .expect("parse renderer preparation query")
            .get("renderer-preparation")
    };
    #[cfg(not(target_arch = "wasm32"))]
    let value = std::env::var("ROBIN_RENDERER_PREPARATION").ok();
    match value.as_deref() {
        None | Some("early") => true,
        Some("late") => false,
        Some(value) => panic!("unknown renderer-preparation policy {value:?}"),
    }
}

/// Owns the temporary loading renderer and the presentation configuration it
/// resolved. Consuming [`Self::close_before_renderer`] is the only way to
/// obtain that configuration for the game renderer.
struct MissionLoadingScreen {
    renderer: Option<LoadingScreenRenderer>,
    /// When no loading artwork exists, prepare the GPU pipelines after mission
    /// downloads have started so construction overlaps their worker/network work.
    prepared_renderer: Option<crate::renderer::Renderer>,
    prepare_renderer_early: bool,
    renderer_config: MissionRendererConfig,
}

impl MissionLoadingScreen {
    const MAX_LEVEL: f32 = 22.0;

    fn open(
        window: &mut GameWindow,
        campaign: &Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
        application_context: &ApplicationContext,
    ) -> Self {
        let _ = window.poll_events();
        let proto_level_filename = campaign
            .missions
            .get(mission_idx)
            .map(|mission| mission.profile(profiles).proto_level_filename.clone());
        let loading_pak =
            resolve_loading_pak(application_context, proto_level_filename.as_deref(), None);
        let profile = application_context
            .active_profile_snapshot()
            .unwrap_or_else(|error| {
                panic!("mission renderer setup requires an active profile: {error}")
            });
        let renderer_config = MissionRendererConfig {
            scale_mode: profile.graphic_config.scale_mode,
            shader_preset: profile.graphic_config.shader_preset,
            native_refresh_presentation: profile.graphic_config.native_refresh_presentation,
            texture_effect: profile.graphic_config.texture_effect,
            upscale_parameters: profile.graphic_config.upscale_parameters,
            texture_effect_parameters: profile.graphic_config.texture_effect_parameters,
        };
        let renderer = loading_pak.and_then(|path| {
            let datadir_kind = match detect_demo_mode_with_context(application_context)
                .map(|(_, _, _, location)| location)
            {
                Some(MissionLocation::Leicester) => LoadingDatadirKind::DemoI,
                Some(MissionLocation::Lincoln) => LoadingDatadirKind::DemoII,
                _ => LoadingDatadirKind::FullGame,
            };
            LoadingScreenRenderer::new(
                window,
                application_context.shipping().unwrap_or_else(|error| {
                    panic!("loading screen lost its ApplicationContext: {error}")
                }),
                application_context
                    .preparation_files()
                    .unwrap_or_else(|error| {
                        panic!("loading screen requires resource preparation authority: {error}")
                    }),
                &path,
                datadir_kind,
                Self::MAX_LEVEL,
                renderer_config.scale_mode,
            )
        });
        let mut stage = Self {
            renderer,
            prepared_renderer: None,
            prepare_renderer_early: prepare_renderer_early(),
            renderer_config,
        };
        stage.status("Preparing mission data...", 0.02);
        if let Some(renderer) = stage.renderer.as_mut() {
            renderer.refresh();
            renderer.drain_events(window);
        }
        stage
    }

    fn status(&mut self, text: &str, progress: f32) {
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_status(text, progress);
        }
    }

    fn shipping_component(
        &mut self,
        window: &mut GameWindow,
        progress: crate::shipping_mission::MissionLoadProgress<'_>,
    ) {
        if self.prepare_renderer_early
            && self.renderer.is_none()
            && self.prepared_renderer.is_none()
            && progress.completed > 0
        {
            let mut timer = super::setup::PhaseTimer::new("streaming frontend preparation");
            self.prepared_renderer = Some(crate::renderer::Renderer::new(
                window,
                window.width as u16,
                window.height as u16,
                self.renderer_config.scale_mode,
            ));
            timer.step("prepare game renderer");
        }
        let fraction = if progress.total == 0 {
            1.0
        } else {
            progress.completed as f32 / progress.total as f32
        };
        let (start, span, label) = match progress.phase {
            crate::shipping_mission::MissionLoadPhase::Data => {
                let span = if cfg!(all(target_arch = "wasm32", feature = "audio")) {
                    0.06
                } else {
                    0.08
                };
                (0.02, span, "mission data")
            }
            crate::shipping_mission::MissionLoadPhase::Audio => (0.08, 0.02, "mission audio"),
        };
        let target = start + span * fraction;
        let text = match progress.file {
            Some(file) => format!(
                "Loading {label} ({}/{}): {file}",
                progress.completed, progress.total
            ),
            None if progress.completed == progress.total => format!("{label} ready"),
            None => format!("Loading {label} (0/{})", progress.total),
        };
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_counted_status(text, target);
            renderer.drain_events(window);
        }
    }

    fn close_before_renderer(self) -> (MissionRendererConfig, Option<crate::renderer::Renderer>) {
        if !self.prepare_renderer_early {
            return (self.renderer_config, None);
        }
        let renderer = self
            .renderer
            .map(LoadingScreenRenderer::into_mission_renderer)
            .or(self.prepared_renderer);
        (self.renderer_config, renderer)
    }
}

/// Owns every process resource acquired before level construction.
struct InteractiveLoadStage {
    loading: MissionLoadingScreen,
    host: Host,
    game: Game,
    process: MissionProcessResources,
}

enum InteractiveLoadStart {
    Ready(InteractiveLoadStage),
    Finished(GameCode),
}

/// Whether an interactive owner treats multiplayer construction failure as a
/// fatal launch error or as an explicit return to its already-running menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MultiplayerSetupFailurePolicy {
    Fatal,
    ReturnToMenu,
}

impl MultiplayerSetupFailurePolicy {
    fn resolve(self, setup: Result<(), String>) -> Result<Option<GameCode>, String> {
        match (self, setup) {
            (_, Ok(())) => Ok(None),
            (Self::Fatal, Err(error)) => Err(error),
            (Self::ReturnToMenu, Err(error)) => {
                tracing::error!("{error}; returning to main menu");
                Ok(Some(GameCode::Quit))
            }
        }
    }
}

impl InteractiveLoadStage {
    async fn begin(
        window: &mut GameWindow,
        #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
        multiplayer_campaign: &crate::multiplayer::MultiplayerCampaignSession,
        campaign: &Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
        rng_seed: u64,
        sim_config: engine_api::SimConfig,
        cold_save_lua: Option<(String, robin_engine::spellforge::SpellforgePackage)>,
        multiplayer_setup_failure_policy: MultiplayerSetupFailurePolicy,
        mut loading: MissionLoadingScreen,
    ) -> Result<InteractiveLoadStart, String> {
        let mission_id = campaign.missions[mission_idx]
            .profile(profiles)
            .mission_filename
            .clone();
        let mut host = Host::new(
            crate::host::ReadyApplicationContext::try_from(args.global_options.clone())?,
            window.width as f32,
            window.height as f32,
        )?;
        host.bind_session_achievement_eligibility(
            crate::session_achievement::SessionAchievementEligibility::from_launch(
                args,
                crate::session_achievement::SessionExecutionMode::Interactive,
            ),
        )?;
        install_pending_lua_session(&mut host, args).map_err(|error| error.to_string())?;
        install_cold_save_lua_session(&mut host, args, cold_save_lua)
            .map_err(|error| error.to_string())?;
        if let Some(code) = multiplayer_setup_failure_policy.resolve(
            setup_multiplayer_session(
                &mut host,
                args,
                &mission_id,
                rng_seed,
                sim_config,
                #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
                multiplayer_campaign,
            )
            .await,
        )? {
            loading.status("Multiplayer connection failed", 1.0);
            if let Some(renderer) = loading.renderer.as_mut() {
                renderer.refresh();
                crate::window::sleep_ms(1200).await;
            }
            return Ok(InteractiveLoadStart::Finished(code));
        }

        let mut game = Game::new(location);
        game.global_options = args.global_options.clone();
        loading.status("Loading process resources...", 0.11);
        let process = MissionProcessResources::load(
            &mut host,
            &game,
            args.replay.is_none() && args.replay_data.is_none(),
        )?;
        Ok(InteractiveLoadStart::Ready(Self {
            loading,
            host,
            game,
            process,
        }))
    }

    fn load_level(
        mut self,
        window: &mut GameWindow,
        campaign: Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
        rng_seed: u64,
        sim_config: engine_api::SimConfig,
        ranked_plan: super::leaderboard_runtime::RankedPreFramePlan,
    ) -> Result<LoadedInteractiveStage, MissionLoadError> {
        self.loading.status("Loading interface resources...", 0.12);
        let (ground_mark, titbit_rows, minimap_widget) =
            self.process.engine_setup_resources(&mut self.host);
        // Pre-engine metadata extraction is done with the interface archive;
        // send it off to get its JXL pictures decoded while the level loads.
        self.process.start_interface_decode();
        let screen_width = window.width as f32;
        let screen_height = window.height as f32;
        let loaded = load_level_and_sprite_bank(
            Some(window),
            &mut self.loading.renderer,
            &mut self.host,
            &mut self.game,
            campaign,
            profiles,
            &mut self.process.text,
            args,
            screen_width,
            screen_height,
            ground_mark,
            titbit_rows,
            minimap_widget,
            rng_seed,
            sim_config,
            ranked_plan,
            true,
        )?;
        Ok(LoadedInteractiveStage {
            bootstrap: MissionBootstrap::new(
                MissionSpec::interactive(mission_idx, location, screen_width, screen_height),
                self.host,
                self.game,
                loaded,
                args,
            ),
            process: Some(self.process),
            loading: Some(self.loading),
        })
    }
}

/// Owns the post-level-load state until renderer/frontend construction is
/// complete. Its methods are intentionally ordered and guarded by
/// `MissionBootstrapPhase`.
struct LoadedInteractiveStage {
    bootstrap: MissionBootstrap,
    process: Option<MissionProcessResources>,
    loading: Option<MissionLoadingScreen>,
}

impl LoadedInteractiveStage {
    fn into_campaign_and_simulation(self) -> (Campaign, u64, engine_api::SimConfig) {
        self.bootstrap.into_campaign_and_simulation()
    }
    fn prepare_audio(&mut self, profiles: &ProfileManager) -> Result<(), String> {
        self.loading
            .as_mut()
            .expect("interactive loading screen must exist until frontend assembly")
            .status("Loading mission audio...", LOADING_AUDIO_PROGRESS);
        self.bootstrap.prepare_audio(
            self.process
                .as_mut()
                .expect("interactive process resources must exist until frontend assembly")
                .audio_backend
                .as_mut(),
            profiles,
        )
    }

    async fn assemble_frontend(
        &mut self,
        window: &mut GameWindow,
        profiles: &ProfileManager,
        args: &crate::main_entry::CliArgs,
    ) -> Result<InteractiveFrontendAssembly, String> {
        let loading = self
            .loading
            .as_mut()
            .expect("interactive loading screen must exist until frontend assembly");
        let LoadedInteractiveResources {
            level_descriptors,
            hud_fonts,
        } = pre_decode_maps_and_resources(
            Some(window),
            &mut loading.renderer,
            &mut self.bootstrap.loaded.engine,
            profiles,
            &self.bootstrap.host,
            &self.bootstrap.game,
        )?;
        let short_briefings = self
            .process
            .as_mut()
            .expect("interactive process resources must exist until frontend assembly")
            .resolve_short_briefings(level_descriptors.as_ref());

        let mut timer = super::setup::PhaseTimer::new("frontend assembly");
        let (renderer_config, prepared_renderer) = self
            .loading
            .take()
            .expect("interactive loading screen must close before renderer construction")
            .close_before_renderer();
        let mut renderer = InteractiveRendererAssembly::new_after_loading_screen(
            window,
            renderer_config,
            prepared_renderer,
        );
        timer.step("game renderer construction");

        // Deferred-terrain join: this is the first point that needs the
        // decoded pixels, so the decode overlapped everything since the
        // mission header was read. A decode failure aborts the mission
        // launch here (the engine's campaign is recovered by the caller).
        let (background, minimap) = match self.bootstrap.loaded.pending_terrain.take() {
            Some(pending) => {
                let decoded = pending.join().await;
                let background = decoded.background?;
                if let Some(bg) = background.as_ref() {
                    assert_eq!(
                        (bg.width as f32, bg.height as f32),
                        self.bootstrap.loaded.bg_pixel_dims,
                        "background map header dimensions diverge from decoded bitmap"
                    );
                }
                timer.step("terrain decode join");
                (background, decoded.minimap)
            }
            None => (
                self.bootstrap.loaded.pre_decoded_background.take(),
                self.bootstrap.loaded.pre_decoded_minimap.take(),
            ),
        };
        let ambience_backgrounds =
            std::mem::take(&mut self.bootstrap.loaded.pre_decoded_ambience_backgrounds);
        let ambience_minimaps =
            std::mem::take(&mut self.bootstrap.loaded.pre_decoded_ambience_minimaps);
        renderer.upload_maps(
            &self.bootstrap.loaded.engine,
            &mut self.bootstrap.host,
            background,
            minimap,
            ambience_backgrounds,
            ambience_minimaps,
        );
        timer.step("map upload");

        // Interface pre-decode join: `load_mission_sprites` and the in-game
        // menus consume these managers next.
        let (cursor, menu_res) = self
            .process
            .as_mut()
            .expect("interactive process resources must exist until frontend assembly")
            .take_interface()
            .await;
        timer.step("interface decode join");

        let process = self
            .process
            .take()
            .expect("interactive process resources must move into the frontend once");
        renderer.assemble_process_frontend(
            window,
            &mut self.bootstrap.host,
            &self.bootstrap.game,
            &mut self.bootstrap.loaded.engine,
            &self.bootstrap.loaded.assets,
            process.text,
            cursor,
            menu_res,
            process.audio_backend,
            LoadedInteractiveResources {
                level_descriptors,
                hud_fonts,
            },
            short_briefings,
            args,
            self.bootstrap.spec.mission_idx,
            self.bootstrap.spec.location,
        )
    }
}

/// A fully constructed interactive mission. The campaign remains inside its
/// engine until consuming finalization returns it in [`MissionOutcome`].
pub(super) struct BuiltInteractiveMission {
    mission: InteractiveMission,
    #[cfg(all(target_arch = "wasm32", feature = "audio"))]
    startup_audio_pause: Option<crate::web_audio_backend::StartupWarmupPause>,
}

impl BuiltInteractiveMission {
    pub(super) async fn run(
        &mut self,
        window: &mut GameWindow,
        callbacks: &mut RustCallbacks,
        profiles: &ProfileManager,
        args: &crate::main_entry::CliArgs,
    ) -> Result<GameCode, String> {
        let mut services = MissionServices {
            #[cfg(all(target_arch = "wasm32", feature = "audio"))]
            startup_audio_pause: &mut self.startup_audio_pause,
            window,
            callbacks,
            profiles,
            args,
        };
        self.mission.run(&mut services).await
    }

    pub(super) fn finish(mut self, result: Result<GameCode, String>) -> MissionOutcome {
        self.mission
            .runtime
            .world
            .host_phase()
            .host
            .frontend
            .mission_surfaces
            .retire(&mut self.mission.frontend.presentation.renderer);
        if result.is_ok() {
            self.mission
                .runtime
                .preserve_multiplayer_session_for_next_mission();
        }
        let transition = self.mission.campaign_transition.take();
        let (campaign, rng_seed, sim_config) = self.mission.runtime.into_campaign_and_simulation();
        MissionOutcome::from_engine(campaign, rng_seed, sim_config, result)
            .with_transition(transition)
    }
}

pub(super) enum InteractiveBuildOutcome {
    Ready(BuiltInteractiveMission),
    Finished(MissionOutcome),
}

/// Owns only the resource archives proven necessary for engine construction in
/// true-headless mode.
struct HeadlessLoadStage {
    host: Host,
    game: Game,
    resources: HeadlessEngineResources,
}

impl HeadlessLoadStage {
    async fn begin(
        #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
        multiplayer_campaign: &crate::multiplayer::MultiplayerCampaignSession,
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
        mission_id: &str,
        rng_seed: u64,
        sim_config: engine_api::SimConfig,
        cold_save_lua: Option<(String, robin_engine::spellforge::SpellforgePackage)>,
    ) -> Result<HeadlessLoadStage, String> {
        let mut host = Host::new(
            crate::host::ReadyApplicationContext::try_from(args.global_options.clone())?,
            1024.0,
            768.0,
        )?;
        host.bind_session_achievement_eligibility(
            crate::session_achievement::SessionAchievementEligibility::from_launch(
                args,
                crate::session_achievement::SessionExecutionMode::Headless,
            ),
        )?;
        install_pending_lua_session(&mut host, args).map_err(|error| error.to_string())?;
        install_cold_save_lua_session(&mut host, args, cold_save_lua)
            .map_err(|error| error.to_string())?;
        let setup_exit = MultiplayerSetupFailurePolicy::Fatal.resolve(
            setup_multiplayer_session(
                &mut host,
                args,
                mission_id,
                rng_seed,
                sim_config,
                #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
                multiplayer_campaign,
            )
            .await,
        )?;
        debug_assert!(
            setup_exit.is_none(),
            "fatal headless multiplayer setup cannot return a menu outcome"
        );
        let mut game = Game::new(location);
        game.global_options = args.global_options.clone();
        let resources = HeadlessEngineResources::load(&host)?;
        Ok(Self {
            host,
            game,
            resources,
        })
    }

    fn load_level(
        mut self,
        campaign: Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
        rng_seed: u64,
        sim_config: engine_api::SimConfig,
    ) -> Result<MissionBootstrap, MissionLoadError> {
        let (ground_mark, titbit_rows, minimap_widget) =
            self.resources.engine_setup_resources(&mut self.host);
        let loaded = load_level_and_sprite_bank(
            None,
            &mut None,
            &mut self.host,
            &mut self.game,
            campaign,
            profiles,
            &mut self.resources.text,
            args,
            1024.0,
            768.0,
            ground_mark,
            titbit_rows,
            minimap_widget,
            rng_seed,
            sim_config,
            super::leaderboard_runtime::RankedPreFramePlan::browse_only(
                "headless and replay-runner missions are not eligible for leaderboard submission",
            ),
            // True-headless has no frontend-assembly join point; collect the
            // decoded terrain synchronously right after engine construction.
            false,
        )?;
        Ok(MissionBootstrap::new(
            MissionSpec::headless(mission_idx, location),
            self.host,
            self.game,
            loaded,
            args,
        ))
    }
}

/// Complete true-headless mission plus the private session return sink for its
/// engine-owned campaign.
pub(super) struct BuiltHeadlessMission {
    mission: HeadlessMission,
}

impl BuiltHeadlessMission {
    pub(super) async fn run(
        &mut self,
        args: &crate::main_entry::CliArgs,
    ) -> HeadlessMissionOutcome {
        self.mission.run(args).await
    }

    pub(super) fn finish(mut self, outcome: HeadlessMissionOutcome) -> MissionOutcome {
        if outcome.code == GameCode::LevelRestart {
            self.mission
                .runtime
                .preserve_multiplayer_session_for_next_mission();
        }
        let (campaign, rng_seed, sim_config) = self.mission.runtime.into_campaign_and_simulation();
        MissionOutcome::from_engine(campaign, rng_seed, sim_config, Ok(outcome.code))
    }
}

pub(super) enum HeadlessBuildOutcome {
    Ready(BuiltHeadlessMission),
    Finished(MissionOutcome),
}

pub(super) struct HeadlessMissionBuilder;

impl HeadlessMissionBuilder {
    pub(super) async fn build(
        callbacks: &mut RustCallbacks,
        campaign: Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
        rng_seed: u64,
        sim_config: engine_api::SimConfig,
    ) -> HeadlessBuildOutcome {
        if let Err(error) = crate::lua_session::validate_launch_mode(
            args,
            crate::http_server::peek_pending_replay_mission_id().is_some(),
        ) {
            return HeadlessBuildOutcome::Finished(MissionOutcome::new(
                campaign,
                rng_seed,
                sim_config,
                Err(error.to_string()),
            ));
        }
        assert!(
            args.headless,
            "headless builder requires headless launch mode"
        );

        let mission_id = campaign.missions[mission_idx]
            .profile(profiles)
            .mission_filename
            .clone();
        let cold_save_lua = match pending_cold_save_lua_launch(callbacks, args) {
            Ok(launch) => launch,
            Err(error) => {
                return HeadlessBuildOutcome::Finished(MissionOutcome::new(
                    campaign,
                    rng_seed,
                    sim_config,
                    Err(error),
                ));
            }
        };
        let loading = match HeadlessLoadStage::begin(
            #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
            &callbacks.multiplayer_campaign,
            location,
            args,
            &mission_id,
            rng_seed,
            sim_config,
            cold_save_lua,
        )
        .await
        {
            Err(error) => {
                return HeadlessBuildOutcome::Finished(MissionOutcome::new(
                    campaign,
                    rng_seed,
                    sim_config,
                    Err(error),
                ));
            }
            Ok(stage) => stage,
        };
        let mut bootstrap = match loading.load_level(
            campaign,
            profiles,
            mission_idx,
            location,
            args,
            rng_seed,
            sim_config,
        ) {
            Ok(bootstrap) => bootstrap,
            Err(error) => {
                return HeadlessBuildOutcome::Finished(MissionOutcome::new(
                    error.campaign,
                    rng_seed,
                    sim_config,
                    Err(error.message),
                ));
            }
        };
        #[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
        if args.simulation_content_export.is_some() {
            let (campaign, rng_seed, sim_config) = bootstrap.into_campaign_and_simulation();
            return HeadlessBuildOutcome::Finished(MissionOutcome::from_engine(
                campaign,
                rng_seed,
                sim_config,
                Ok(robin_engine::game_operation::GameCode::Quit),
            ));
        }
        if let Err(error) = bootstrap.start_required_spellforge() {
            let (campaign, rng_seed, sim_config) = bootstrap.into_campaign_and_simulation();
            return HeadlessBuildOutcome::Finished(MissionOutcome::from_engine(
                campaign,
                rng_seed,
                sim_config,
                Err(error.to_string()),
            ));
        }
        if let Err(error) = bootstrap.prepare_audio(None, profiles) {
            let (campaign, rng_seed, sim_config) = bootstrap.into_campaign_and_simulation();
            return HeadlessBuildOutcome::Finished(MissionOutcome::from_engine(
                campaign,
                rng_seed,
                sim_config,
                Err(error),
            ));
        }
        // Graphical frontend assembly registers these campaign-owned names
        // after audio preparation and before seat/bootstrap snapshots. The
        // CPU-loaded pool is sufficient; headless must not construct a UI or
        // skip this hashed state merely because it does not render portraits.
        let mut localized_names = std::array::from_fn(|_| None);
        super::setup::register_mission_peasant_names(
            &mut localized_names,
            &mut bootstrap.loaded.engine,
            &bootstrap.loaded.assets,
        );
        setup_local_seat_and_multiplayer_snapshot(
            &mut bootstrap.loaded.engine,
            &mut bootstrap.host,
            &bootstrap.loaded.assets,
            args,
        );
        bootstrap.start_campaign_clock();
        let mission = bootstrap.finish_headless(args, HeadlessPolicy::replay_runner());
        HeadlessBuildOutcome::Ready(BuiltHeadlessMission { mission })
    }
}

/// Ordered interactive bootstrap entry point. The body is deliberately a list
/// of ownership-stage transitions; the work for each stage lives on its
/// smallest owner above.
pub(super) struct InteractiveMissionBuilder;

impl InteractiveMissionBuilder {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn build(
        window: &mut GameWindow,
        callbacks: &mut RustCallbacks,
        campaign: Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
        rng_seed: u64,
        sim_config: engine_api::SimConfig,
        multiplayer_setup_failure_policy: MultiplayerSetupFailurePolicy,
    ) -> InteractiveBuildOutcome {
        // A checkpoint belongs to one running mission, including when entry
        // into the next mission fails or takes the lost-Sherwood shortcut.
        callbacks.save_manager.clear_session_restart();

        if let Err(error) = crate::lua_session::validate_launch_mode(
            args,
            crate::http_server::peek_pending_replay_mission_id().is_some(),
        ) {
            return InteractiveBuildOutcome::Finished(MissionOutcome::new(
                campaign,
                rng_seed,
                sim_config,
                Err(error.to_string()),
            ));
        }
        assert!(
            !args.headless,
            "interactive builder cannot construct headless shims"
        );

        #[cfg(all(target_arch = "wasm32", feature = "audio"))]
        let startup_audio_pause = if args.global_options.sound_enabled {
            match args
                .global_options
                .browser_audio()
                .and_then(|audio| audio.pause_warmup_until_first_frame())
            {
                Ok(pause) => pause,
                Err(error) => {
                    return InteractiveBuildOutcome::Finished(MissionOutcome::new(
                        campaign,
                        rng_seed,
                        sim_config,
                        Err(error),
                    ));
                }
            }
        } else {
            None
        };

        let graphic_config = args
            .global_options
            .active_profile_snapshot()
            .unwrap_or_else(|error| {
                panic!("mission display setup requires an active profile: {error}")
            })
            .graphic_config;
        window.set_logical_resolution_policy(&graphic_config);
        let mut loading = MissionLoadingScreen::open(
            window,
            &campaign,
            profiles,
            mission_idx,
            &args.global_options,
        );
        let mission_id = campaign.missions[mission_idx]
            .profile(profiles)
            .mission_filename
            .clone();
        let cold_save_lua = match pending_cold_save_lua_launch(callbacks, args) {
            Ok(launch) => launch,
            Err(error) => {
                return InteractiveBuildOutcome::Finished(MissionOutcome::new(
                    campaign,
                    rng_seed,
                    sim_config,
                    Err(error),
                ));
            }
        };
        let has_decoded_saved_world = super::pending_decoded_saved_world(callbacks);
        let archive_restored = args
            .resolved_mission_assets
            .as_ref()
            .is_some_and(|resolved| resolved.is_archive());
        if !archive_restored
            && let Err(error) = super::ensure_shipping_mission(
                args,
                &mission_id,
                &campaign,
                profiles,
                has_decoded_saved_world,
                |progress| loading.shipping_component(window, progress),
            )
            .await
        {
            return InteractiveBuildOutcome::Finished(MissionOutcome::new(
                campaign,
                rng_seed,
                sim_config,
                Err(error),
            ));
        }
        let campaign = super::establish_mission_restart_boundary(campaign, rng_seed, sim_config);
        let mut timer = super::setup::PhaseTimer::new("mission bootstrap");
        let mut loading = match InteractiveLoadStage::begin(
            window,
            #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
            &callbacks.multiplayer_campaign,
            &campaign,
            profiles,
            mission_idx,
            location,
            args,
            rng_seed,
            sim_config,
            cold_save_lua,
            multiplayer_setup_failure_policy,
            loading,
        )
        .await
        {
            Ok(InteractiveLoadStart::Ready(stage)) => stage,
            Ok(InteractiveLoadStart::Finished(code)) => {
                return InteractiveBuildOutcome::Finished(MissionOutcome::new(
                    campaign,
                    rng_seed,
                    sim_config,
                    Ok(code),
                ));
            }
            Err(error) => {
                return InteractiveBuildOutcome::Finished(MissionOutcome::new(
                    campaign,
                    rng_seed,
                    sim_config,
                    Err(error),
                ));
            }
        };
        timer.step("load stage begin");
        loading
            .loading
            .status("Checking ranked mission authority...", 0.115);
        let ranked_plan = if args.replay.is_some() || args.replay_data.is_some() {
            super::leaderboard_runtime::RankedPreFramePlan::browse_only(
                "replay playback cannot submit a new ranked run",
            )
        } else if loading.host.scripting.lua_session.is_some()
            || args.custom_mission.is_some()
            || args.pending_lua_mission.is_some()
        {
            super::leaderboard_runtime::RankedPreFramePlan::browse_only(
                "custom or Spellforge mission content is outside the official demo/full ranked policy",
            )
        } else {
            match super::leaderboard_runtime::fetch_single_player_authority(
                &mission_id,
                sim_config,
                &campaign,
            )
            .await
            {
                Ok(authority) => {
                    super::leaderboard_runtime::RankedPreFramePlan::Authority(authority)
                }
                Err(reason) => {
                    tracing::warn!("mission is browse-only: {reason}");
                    super::leaderboard_runtime::RankedPreFramePlan::browse_only(reason)
                }
            }
        };
        let mut stage = match loading.load_level(
            window,
            campaign,
            profiles,
            mission_idx,
            location,
            args,
            rng_seed,
            sim_config,
            ranked_plan,
        ) {
            Ok(stage) => stage,
            Err(error) => {
                return InteractiveBuildOutcome::Finished(MissionOutcome::new(
                    error.campaign,
                    rng_seed,
                    sim_config,
                    Err(error.message),
                ));
            }
        };
        timer.step("level load");

        if let Err(error) = stage.bootstrap.start_required_spellforge() {
            let (campaign, rng_seed, sim_config) = stage.into_campaign_and_simulation();
            return InteractiveBuildOutcome::Finished(MissionOutcome::from_engine(
                campaign,
                rng_seed,
                sim_config,
                Err(error.to_string()),
            ));
        }
        timer.step("spellforge startup");
        stage
            .bootstrap
            .sign_ranked_session_before_frame_zero(args)
            .await;
        timer.step("ranked genesis signing");
        if let Err(error) = stage.prepare_audio(profiles) {
            let (campaign, rng_seed, sim_config) = stage.into_campaign_and_simulation();
            return InteractiveBuildOutcome::Finished(MissionOutcome::from_engine(
                campaign,
                rng_seed,
                sim_config,
                Err(error),
            ));
        }
        timer.step("audio prepare");
        let frontend = match stage.assemble_frontend(window, profiles, args).await {
            Ok(frontend) => frontend,
            Err(error) => {
                let (campaign, rng_seed, sim_config) = stage.into_campaign_and_simulation();
                return InteractiveBuildOutcome::Finished(MissionOutcome::from_engine(
                    campaign,
                    rng_seed,
                    sim_config,
                    Err(error),
                ));
            }
        };
        timer.step("frontend assembly");

        let lost_sherwood = stage.bootstrap.game.is_sherwood
            && stage.bootstrap.loaded.engine.campaign().get_ares() == 0;
        if lost_sherwood {
            stage.bootstrap.defer_lost_sherwood_entry();
        } else {
            stage.bootstrap.start_campaign_clock();
            stage.bootstrap.setup_restart_or_sherwood(callbacks, args);
        }
        let frontend = frontend.finish(window.width, window.height);
        timer.step("HUD sprite finish");
        let bootstrap = stage.bootstrap;
        let mission = bootstrap.finish_interactive(frontend, args);
        timer.step("runtime + replay init");
        timer.total();
        InteractiveBuildOutcome::Ready(BuiltInteractiveMission {
            mission,
            #[cfg(all(target_arch = "wasm32", feature = "audio"))]
            startup_audio_pause,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MissionBootstrapLifecycle, MissionBootstrapPhase, MissionFrontendKind, MissionSpec,
        MultiplayerSetupFailurePolicy, built_in_mission_assets_for_loaded_level,
    };
    use robin_engine::campaign::{Campaign, CampaignValue};
    use robin_engine::game_operation::GameCode;
    use robin_engine::profiles::MissionLocation;
    use std::cell::Cell;
    use std::future::Future;
    use std::pin::Pin;
    use std::rc::Rc;
    use std::task::{Context, Poll, Waker};

    struct PendingCampaignFuture {
        campaign: Campaign,
        observed_allocation: Rc<Cell<usize>>,
    }

    impl Future for PendingCampaignFuture {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    impl Drop for PendingCampaignFuture {
        fn drop(&mut self) {
            self.observed_allocation
                .set(self.campaign.production_sectors.as_ptr() as usize);
        }
    }

    fn marked_campaign(marker: i32) -> Campaign {
        let mut campaign = Campaign::default();
        campaign.values[CampaignValue::Custom20] = marker;
        campaign
    }

    #[test]
    fn mission_spec_round_trips_without_process_resources() {
        let expected = MissionSpec::interactive(3, MissionLocation::Derby, 1024.0, 768.0);

        let json = serde_json::to_string(&expected).expect("mission spec should serialize");
        let actual: MissionSpec =
            serde_json::from_str(&json).expect("mission spec should deserialize");

        assert_eq!(actual, expected);
        assert_eq!(actual.frontend, MissionFrontendKind::Interactive);
    }

    #[test]
    fn headless_spec_uses_the_existing_logical_viewport() {
        let spec = MissionSpec::headless(1, MissionLocation::Leicester);

        assert_eq!((spec.screen_width, spec.screen_height), (1024.0, 768.0));
        assert_eq!(spec.frontend, MissionFrontendKind::Headless);
    }

    #[test]
    fn bootstrap_installs_save_assets_and_tracks_failed_restart_creation() {
        use super::{LoadedMissionCore, MissionBootstrap};
        use robin_engine::engine::{Engine, EngineArgs, LevelAssets, LevelLoadArgs};

        let mut assets = LevelAssets::new();
        // Seed a valid campaign/profile fixture, then construct a level with
        // distinct proto and map names so saves must retain the loaded map.
        let fixture = Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut assets)
            .expect("fixture campaign");
        let campaign = fixture.campaign().clone();
        let profile = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).missions[0];
        profile.id = 1;
        profile.mission_filename = "Mission".into();
        profile.proto_level_filename = "ProtoLevel".into();
        let mut level = robin_engine::level_data::LoadedLevel::empty_for_test();
        level.mission.header.map_filename = "TerrainMap".into();
        let sim_config = robin_engine::engine::SimConfig {
            script_enabled: false,
            ..Default::default()
        };
        let engine = Engine::new(EngineArgs {
            campaign: campaign.clone(),
            level: LevelLoadArgs {
                assets: &mut assets,
                level_directory: "",
                progress: &mut |_| {},
                loaded: level,
                bg_pixel_dims: (0.0, 0.0),
            },
            ground_mark_sprite: None,
            titbit_row_frame_counts: Vec::new(),
            rng_seed: 0,
            original_rng_replay: None,
            sim_config,
        })
        .expect("fixture level");
        let mut bootstrap = MissionBootstrap::new(
            MissionSpec::interactive(0, MissionLocation::Lincoln, 1024.0, 768.0),
            crate::host::Host::scratch(1024.0, 768.0),
            crate::game::Game::new(MissionLocation::Lincoln),
            LoadedMissionCore {
                engine,
                replay_campaign: campaign,
                assets,
                dev: Default::default(),
                pre_decoded_background: None,
                pre_decoded_minimap: None,
                pending_terrain: None,
                bg_pixel_dims: (0.0, 0.0),
                pre_decoded_ambience_backgrounds: Vec::new(),
                pre_decoded_ambience_minimaps: Vec::new(),
                engine_rng_seed: 0,
                engine_sim_config: sim_config,
                ranked_admission:
                    super::super::leaderboard_runtime::PreparedRankedAdmission::BrowseOnly {
                        reason: "test fixture".into(),
                    },
            },
            &crate::main_entry::CliArgs::default(),
        );
        assert_eq!(
            bootstrap.lifecycle.phase(),
            MissionBootstrapPhase::LevelInitialized
        );
        assert_eq!(
            bootstrap
                .game
                .mission_assets()
                .expect("assets available before restart save"),
            &built_in_mission_assets_for_loaded_level("Mission", "ProtoLevel", "TerrainMap"),
        );
        // Scratch hosts have no active player profile, so the real background
        // save path must reject capture. Mission startup still advances, but
        // must not claim a frame-0 restart snapshot exists.
        let directory = tempfile::tempdir().unwrap();
        let save_root = directory.path().to_string_lossy().into_owned();
        let mut players =
            robin_engine::player_profile::PlayerProfileManager::new(save_root.clone());
        let player = players.create_profile(
            "Bootstrap Test".into(),
            robin_engine::player_profile::DifficultyLevel::Medium,
        );
        players.set_active(player);
        let application_context = crate::host::ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&save_root),
            robin_engine::engine::GlobalOptions::default(),
            players,
            crate::key_config_store::KeyConfigStore::new(save_root),
            None,
        )
        .unwrap();
        let mut callbacks = crate::main_entry::RustCallbacks::new(application_context);
        bootstrap.start_required_spellforge().unwrap();
        bootstrap.lifecycle.advance(
            MissionBootstrapPhase::SpellforgeStarted,
            MissionBootstrapPhase::AudioPrepared,
        );
        bootstrap.start_campaign_clock();
        bootstrap.setup_restart_or_sherwood(&mut callbacks, &crate::main_entry::CliArgs::default());
        assert!(!bootstrap.restart_save_started);
        assert_eq!(
            bootstrap.lifecycle.phase(),
            MissionBootstrapPhase::EntryPrepared
        );
    }

    #[test]
    fn built_in_descriptor_retains_actual_loaded_map_identity() {
        let descriptor =
            built_in_mission_assets_for_loaded_level("Mission", "ProtoLevel", "TerrainMap");

        assert_eq!(descriptor.proto_level_filename, "ProtoLevel");
        assert_eq!(descriptor.map_filename, "TerrainMap");
    }

    #[test]
    fn fatal_setup_policy_preserves_direct_and_headless_errors() {
        for error in [
            "multiplayer Welcome mission mismatch",
            "multiplayer cannot be combined with replay playback",
        ] {
            let actual = MultiplayerSetupFailurePolicy::Fatal
                .resolve(Err(error.to_string()))
                .unwrap_err();
            assert_eq!(actual, error);
        }
    }

    #[test]
    fn menu_owned_setup_policy_returns_to_existing_menu() {
        let actual = MultiplayerSetupFailurePolicy::ReturnToMenu
            .resolve(Err("multiplayer Welcome mission mismatch".to_string()))
            .unwrap();

        assert_eq!(actual, Some(GameCode::Quit));
    }

    #[test]
    fn interactive_bootstrap_lifecycle_preserves_original_order() {
        use MissionBootstrapPhase as Phase;
        let expected = [
            Phase::LevelInitialized,
            Phase::SpellforgeStarted,
            Phase::AudioPrepared,
            Phase::CampaignClockStarted,
            Phase::EntryPrepared,
        ];
        let mut lifecycle = MissionBootstrapLifecycle::new();

        for pair in expected.windows(2) {
            lifecycle.advance(pair[0], pair[1]);
        }

        assert_eq!(lifecycle.trace(), expected);
        assert_eq!(lifecycle.phase(), Phase::EntryPrepared);
    }

    #[test]
    #[should_panic(expected = "invalid mission bootstrap transition")]
    fn bootstrap_lifecycle_transition_method_rejects_ordering_shortcuts() {
        use MissionBootstrapPhase as Phase;
        let mut lifecycle = MissionBootstrapLifecycle::new();

        lifecycle.advance(Phase::LevelInitialized, Phase::AudioPrepared);
    }

    #[test]
    fn cancelling_pending_campaign_future_drops_the_exact_owned_allocation() {
        let engine_campaign = marked_campaign(0x62_62_62);
        let production_sectors = engine_campaign.production_sectors.as_ptr();
        let observed_allocation = Rc::new(Cell::new(0));

        let mut future = Box::pin(PendingCampaignFuture {
            campaign: engine_campaign,
            observed_allocation: Rc::clone(&observed_allocation),
        });
        let mut context = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut context).is_pending());
        drop(future);

        assert_eq!(observed_allocation.get(), production_sectors as usize);
    }
}
