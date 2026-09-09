//! Ordered construction of complete interactive and true-headless missions.

use super::flow::MissionServices;
use super::headless::{HeadlessMission, HeadlessMissionOutcome, HeadlessPolicy};
use super::interactive::{
    InteractiveFrontendAssembly, InteractiveMission, InteractiveRendererAssembly,
    MissionRendererConfig,
};
use super::replay_init::init_replay_and_rollback;
use super::runtime::{
    FrameContract, MissionControl, MissionRuntime, MissionWorld, TimelineRuntime,
};
use super::setup::{
    DecodingInterfaceResources, LOADING_AUDIO_PROGRESS, LoadedInteractiveResources,
    LoadedMissionCore, MissionEngineResources, MissionInterfaceSetup, MissionLaunchSetup,
    MissionLoadError, MissionProcessResources, TerrainJoinPoint, pre_decode_maps_and_resources,
    prepare_mission, setup_local_seat_and_multiplayer_snapshot, setup_mission_audio,
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
    restart_save: RestartSaveState,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
enum RestartSaveState {
    #[default]
    Absent,
    Pending(super::runtime::BootstrapSaveBoundary),
    Completed(super::runtime::BootstrapSaveBoundary),
}

impl RestartSaveState {
    fn observe_completion(
        &mut self,
        result: anyhow::Result<crate::savegame::SaveWriteStatus>,
    ) -> bool {
        let Self::Pending(boundary) = *self else {
            panic!("Restart completion requires an admitted pending save");
        };
        match result {
            Ok(crate::savegame::SaveWriteStatus::Queued) => false,
            Ok(crate::savegame::SaveWriteStatus::Completed) => {
                *self = Self::Completed(boundary);
                true
            }
            Err(error) => {
                tracing::error!(
                    "Restart save publication failed; no replay save marker: {error:#}"
                );
                *self = Self::Absent;
                true
            }
        }
    }
}

/// Audio preparation grants the only owner that can complete mission entry.
/// The simulation stays boxed through each consuming frontend handoff.
struct AudioPreparedBootstrap(Box<MissionBootstrap>);

impl Serialize for AudioPreparedBootstrap {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.spec.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AudioPreparedBootstrap {
    fn deserialize<D: serde::Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "mission bootstrap requires live resource preparation",
        ))
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
            restart_save: RestartSaveState::Absent,
        };
        bootstrap.install_mission_assets(args);
        bootstrap
    }

    /// Initialize has already run in engine construction. Report that fact;
    /// this is not another fallible startup stage.
    fn report_spellforge_startup(&self) {
        if let Some(lua) = self.host.scripting.lua_session.as_ref() {
            tracing::info!(
                "Lua: deterministic runtime active for mission '{}' (seed={}); Initialize is owned by the engine callback driver",
                lua.mission_basename(),
                self.loaded.engine_rng_seed,
            );
        }
    }

    fn prepare_audio(
        mut self: Box<Self>,
        backend: Option<&mut crate::audio_backend::KiraAudioBackend>,
        profiles: &ProfileManager,
    ) -> Result<AudioPreparedBootstrap, (Box<Self>, String)> {
        if let Err(error) = setup_mission_audio(
            &mut self.host,
            backend,
            &self.loaded.engine,
            &mut self.loaded.assets,
            profiles,
            self.spec.location,
            &self.game.global_options.sound_directory,
        ) {
            return Err((self, error));
        }
        Ok(AudioPreparedBootstrap(self))
    }

    /// Start the campaign segment clock after the lost-Sherwood gate, matching
    /// the original `GameLoop` boundary.
    fn start_campaign_clock(&mut self) {
        self.loaded
            .engine
            .finish_mission_bootstrap(engine_api::MissionBootstrapCompletion::StartClock);
    }

    fn prepare_interactive_entry(
        &mut self,
        callbacks: &mut RustCallbacks,
        args: &crate::main_entry::CliArgs,
    ) {
        // A lost Sherwood campaign still needs a runtime for debriefing and
        // network/HTTP draining, but must not start play time or restart state.
        if !(self.game.is_sherwood && self.loaded.engine.campaign().get_ares() == 0) {
            self.start_campaign_clock();
            self.setup_restart_or_sherwood(callbacks, args);
        } else {
            self.loaded
                .engine
                .finish_mission_bootstrap(engine_api::MissionBootstrapCompletion::DebriefOnly);
        }
    }

    /// Capture the pristine restart state for a tactical mission. Fresh
    /// Sherwood sessions leave the campaign map closed, matching the original
    /// original-game session startup; the player opens it from the HQ widget. This must
    /// be the last setup stage before replay/runtime construction.
    fn setup_restart_or_sherwood(
        &mut self,
        callbacks: &mut RustCallbacks,
        args: &crate::main_entry::CliArgs,
    ) {
        let playing_back = args.replay_data.is_some() || args.replay.is_some();
        // Playback pins its frame-0 save markers in TimelineRuntime and replays
        // load-back records from those immutable snapshots. It must not create
        // a live disk Restart save or replace the user's previous recovery point.
        if !playing_back && !self.game.is_sherwood && args.mission_start_map_output.is_none() {
            let campaign = self.loaded.engine.campaign();
            let mission_id = current_mission_id(campaign, &self.loaded.assets.profile_manager);
            self.restart_save = match callbacks.save_manager.write_restart_save_background(
                &mut self.host,
                &self.game,
                &self.loaded.engine,
                mission_id,
                Some(&self.loaded.assets.profile_manager),
                None,
            ) {
                Ok(status) => {
                    let boundary = super::runtime::BootstrapSaveBoundary::capture(
                        &self.loaded.engine,
                        &self.host,
                        &self.game,
                        callbacks.save_manager.restart_session_identity(),
                    );
                    match status {
                        crate::savegame::SaveWriteStatus::Queued => {
                            RestartSaveState::Pending(boundary)
                        }
                        crate::savegame::SaveWriteStatus::Completed => {
                            RestartSaveState::Completed(boundary)
                        }
                    }
                }
                Err(error) => {
                    tracing::error!("Restart save could not start: {error:#}");
                    RestartSaveState::Absent
                }
            };
        }
    }

    /// Resolve persistence before opening recorder frame zero. Polling uses
    /// the normal runtime pacing hook: native serialization stays on its worker
    /// while the dedicated game thread waits, and the browser yields its loop.
    async fn complete_restart_save(&mut self, callbacks: &mut RustCallbacks) {
        if !matches!(self.restart_save, RestartSaveState::Pending(_)) {
            return;
        }
        while !self
            .restart_save
            .observe_completion(callbacks.save_manager.try_finish_background())
        {
            crate::window::sleep_ms(1).await;
        }
    }

    async fn sign_ranked_session_before_frame_zero(&mut self, args: &crate::main_entry::CliArgs) {
        let custom_package_present = self.host.scripting.lua_session.is_some()
            || args.custom_mission.is_some()
            || args.pending_lua_mission.is_some();
        if let Some(net) = self.host.transport.net() {
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
                .net()
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
                self.host.transport.net().is_some(),
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
            self.host.transport.net().is_some(),
        );
        let mut timeline = TimelineRuntime::new(
            replay,
            contract,
            wait_for_multiplayer_start,
            self.host.transport.local_seat() == robin_engine::player_command::PlayerId::HOST,
        );
        debug_assert_eq!(timeline.frame_contract(), contract);
        timeline.register_bootstrap_save(match self.restart_save {
            RestartSaveState::Absent => None,
            RestartSaveState::Completed(boundary) => Some(boundary),
            RestartSaveState::Pending(_) => panic!("runtime opened before Restart save completion"),
        });
        let manager = robin_engine::engine_manager::EngineManager::new(self.loaded.engine);
        let dynamic_visuals = self
            .host
            .application_context()
            .active_profile_snapshot()
            .map(|profile| profile.graphic_config.dynamic_ambience_visuals)
            .unwrap_or_else(|error| {
                panic!("mission visual setup requires an active profile: {error}")
            });
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

impl AudioPreparedBootstrap {
    fn into_campaign_and_simulation(self) -> (Campaign, u64, engine_api::SimConfig) {
        self.0.into_campaign_and_simulation()
    }

    fn finish_interactive(
        self,
        frontend: InteractiveFrontendAssembly,
        width: u32,
        height: u32,
        args: &crate::main_entry::CliArgs,
    ) -> InteractiveMission {
        let bootstrap = self.0;
        assert_eq!(bootstrap.spec.frontend, MissionFrontendKind::Interactive);
        let frontend = frontend.finish(width, height);
        let wait_for_multiplayer_start = bootstrap.host.transport.net().is_some();
        InteractiveMission {
            runtime: bootstrap.finish_runtime(
                args,
                FrameContract::Graphical,
                wait_for_multiplayer_start,
            ),
            frontend,
            campaign_transition: None,
        }
    }

    fn finish_headless(
        self,
        args: &crate::main_entry::CliArgs,
        policy: HeadlessPolicy,
    ) -> HeadlessMission {
        let mut bootstrap = self.0;
        assert_eq!(bootstrap.spec.frontend, MissionFrontendKind::Headless);
        // Match graphical assembly's hashed name registration and seat
        // snapshot boundary without constructing presentation resources.
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
        let wait_for_multiplayer_start = bootstrap.host.transport.net().is_some();
        HeadlessMission {
            modals: super::session_policy::SessionModalScheduler::default(),
            runtime: bootstrap.finish_runtime(
                args,
                FrameContract::Headless,
                wait_for_multiplayer_start,
            ),
            policy,
        }
    }

    /// Frontend resources may be consumed on failure, but mission ownership
    /// must remain available for the caller's campaign recovery path.
    async fn retain_during_frontend_assembly(
        mut self,
        assemble: impl AsyncFnOnce(&mut MissionBootstrap) -> Result<InteractiveFrontendAssembly, String>,
    ) -> (Self, Result<InteractiveFrontendAssembly, String>) {
        let result = assemble(&mut self.0).await;
        (self, result)
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
        )
        .map_err(|error| error.to_string())?;
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
        let mut process = self.process.start_interface_decode();
        let screen_width = window.width as f32;
        let screen_height = window.height as f32;
        let mut feedback = (Some(window), &mut self.loading.renderer);
        let prepared = prepare_mission(
            &mut feedback,
            &mut self.host,
            &mut self.game,
            campaign,
            profiles,
            &mut process.text,
            args,
            MissionInterfaceSetup {
                ground_mark,
                titbit_rows,
                minimap_widget,
                screen_dimensions: (screen_width, screen_height),
            },
            MissionLaunchSetup {
                rng_seed,
                sim_config,
                ranked_plan,
            },
        )?;
        let constructed = prepared.construct_engine(args, &mut feedback)?;
        let loaded = constructed.attach_presentation(
            &mut self.host,
            args,
            &mut feedback,
            TerrainJoinPoint::BeforePresentationUpload,
        )?;
        Ok(LoadedInteractiveStage {
            bootstrap: Box::new(MissionBootstrap::new(
                MissionSpec::interactive(mission_idx, location, screen_width, screen_height),
                self.host,
                self.game,
                loaded,
                args,
            )),
            process,
            loading: self.loading,
        })
    }
}

/// Owns the post-level-load state until renderer/frontend construction is
/// complete. Only successful audio preparation grants frontend assembly.
struct LoadedInteractiveStage<Bootstrap = Box<MissionBootstrap>> {
    // The bootstrap includes the complete simulation. Keep ownership on the
    // heap across consuming async frontend stages rather than embedding it in
    // every nested future and its returned result.
    bootstrap: Bootstrap,
    process: MissionProcessResources<DecodingInterfaceResources>,
    loading: MissionLoadingScreen,
}

impl LoadedInteractiveStage {
    fn prepare_audio(
        mut self,
        profiles: &ProfileManager,
    ) -> Result<LoadedInteractiveStage<AudioPreparedBootstrap>, (Box<MissionBootstrap>, String)>
    {
        self.loading
            .status("Loading mission audio...", LOADING_AUDIO_PROGRESS);
        let bootstrap = self
            .bootstrap
            .prepare_audio(self.process.audio_backend.as_mut(), profiles)?;
        Ok(LoadedInteractiveStage {
            bootstrap,
            process: self.process,
            loading: self.loading,
        })
    }
}

impl LoadedInteractiveStage<AudioPreparedBootstrap> {
    // Loading renderers/resource managers also make this handoff substantial.
    // Do not embed it in the complete mission builder's future merely because
    // the simulation and inner GPU upload are already boxed.
    fn assemble_frontend<'a>(
        self,
        window: &'a mut GameWindow,
        profiles: &'a ProfileManager,
        args: &'a crate::main_entry::CliArgs,
    ) -> futures::future::LocalBoxFuture<
        'a,
        (
            AudioPreparedBootstrap,
            Result<InteractiveFrontendAssembly, String>,
        ),
    > {
        Box::pin(self.assemble_frontend_inner(window, profiles, args))
    }

    async fn assemble_frontend_inner(
        self,
        window: &mut GameWindow,
        profiles: &ProfileManager,
        args: &crate::main_entry::CliArgs,
    ) -> (
        AudioPreparedBootstrap,
        Result<InteractiveFrontendAssembly, String>,
    ) {
        let Self {
            bootstrap,
            process,
            loading,
        } = self;
        bootstrap
            .retain_during_frontend_assembly(async |bootstrap| {
                Self::assemble_process_frontend(bootstrap, process, loading, window, profiles, args)
                    .await
            })
            .await
    }

    // Keep the renderer/upload future behind a pointer as well: inlining it
    // into the consuming stage and its owner-retention closure multiplies the
    // stack required to construct and poll graphical startup.
    fn assemble_process_frontend<'a>(
        bootstrap: &'a mut MissionBootstrap,
        process: MissionProcessResources<DecodingInterfaceResources>,
        loading: MissionLoadingScreen,
        window: &'a mut GameWindow,
        profiles: &'a ProfileManager,
        args: &'a crate::main_entry::CliArgs,
    ) -> futures::future::LocalBoxFuture<'a, Result<InteractiveFrontendAssembly, String>> {
        Box::pin(Self::assemble_process_frontend_inner(
            bootstrap, process, loading, window, profiles, args,
        ))
    }

    async fn assemble_process_frontend_inner(
        bootstrap: &mut MissionBootstrap,
        mut process: MissionProcessResources<DecodingInterfaceResources>,
        mut loading: MissionLoadingScreen,
        window: &mut GameWindow,
        profiles: &ProfileManager,
        args: &crate::main_entry::CliArgs,
    ) -> Result<InteractiveFrontendAssembly, String> {
        let LoadedInteractiveResources {
            level_descriptors,
            hud_fonts,
        } = pre_decode_maps_and_resources(
            Some(window),
            &mut loading.renderer,
            &mut bootstrap.loaded.engine,
            profiles,
            &bootstrap.host,
            &bootstrap.game,
        )?;
        let short_briefings = process
            .resolve_short_briefings(level_descriptors.as_ref())
            .map_err(|error| error.to_string())?;

        let mut timer = super::setup::PhaseTimer::new("frontend assembly");
        let (renderer_config, prepared_renderer) = loading.close_before_renderer();
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
        let (background, minimap) = match bootstrap.loaded.pending_terrain.take() {
            Some(pending) => {
                let decoded = pending.join().await;
                let background = decoded.background?;
                if let Some(bg) = background.as_ref() {
                    assert_eq!(
                        (bg.width as f32, bg.height as f32),
                        bootstrap.loaded.bg_pixel_dims,
                        "background map header dimensions diverge from decoded bitmap"
                    );
                }
                timer.step("terrain decode join");
                (background, decoded.minimap)
            }
            None => (
                bootstrap.loaded.pre_decoded_background.take(),
                bootstrap.loaded.pre_decoded_minimap.take(),
            ),
        };
        let ambience_backgrounds =
            std::mem::take(&mut bootstrap.loaded.pre_decoded_ambience_backgrounds);
        let ambience_minimaps = std::mem::take(&mut bootstrap.loaded.pre_decoded_ambience_minimaps);
        renderer.upload_maps(
            &bootstrap.loaded.engine,
            &mut bootstrap.host,
            background,
            minimap,
            ambience_backgrounds,
            ambience_minimaps,
        );
        timer.step("map upload");

        // Interface pre-decode join: `load_mission_sprites` and the in-game
        // menus consume these managers next.
        let (text, cursor, menu_res, audio_backend) = process.collect().await;
        timer.step("interface decode join");

        renderer.assemble_process_frontend(
            window,
            &mut bootstrap.host,
            &bootstrap.game,
            &mut bootstrap.loaded.engine,
            &bootstrap.loaded.assets,
            text,
            cursor,
            menu_res,
            audio_backend,
            LoadedInteractiveResources {
                level_descriptors,
                hud_fonts,
            },
            short_briefings,
            args,
            bootstrap.spec.mission_idx,
            bootstrap.spec.location,
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
    resources: MissionEngineResources,
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
        let resources = MissionEngineResources::load(&host).map_err(|error| error.to_string())?;
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
        let mut loading_screen = None;
        let mut feedback = (None, &mut loading_screen);
        let prepared = prepare_mission(
            &mut feedback,
            &mut self.host,
            &mut self.game,
            campaign,
            profiles,
            &mut self.resources.text,
            args,
            MissionInterfaceSetup {
                ground_mark,
                titbit_rows,
                minimap_widget,
                screen_dimensions: (1024.0, 768.0),
            },
            MissionLaunchSetup {
                rng_seed,
                sim_config,
                ranked_plan: super::leaderboard_runtime::RankedPreFramePlan::browse_only(
                    "headless and replay-runner missions are not eligible for leaderboard submission",
                ),
            },
        )?;
        let constructed = prepared.construct_engine(args, &mut feedback)?;
        // True-headless has no frontend-assembly join point.
        let loaded = constructed.attach_presentation(
            &mut self.host,
            args,
            &mut feedback,
            TerrainJoinPoint::BeforeHeadlessRuntime,
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

/// Export prepared mission inputs without constructing callbacks that open
/// player saves. Official projection contexts deliberately disable persistence.
#[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
pub(crate) async fn export_official_mission_headless(
    campaign: Campaign,
    profiles: &ProfileManager,
    mission_idx: usize,
    location: MissionLocation,
    args: &crate::main_entry::CliArgs,
    rng_seed: u64,
    sim_config: engine_api::SimConfig,
) -> Result<(), String> {
    if !args.headless || args.simulation_content_export.is_none() {
        return Err("official projection requires a headless export request".to_owned());
    }
    crate::lua_session::validate_launch_mode(args, false).map_err(|error| error.to_string())?;
    let mission_id = campaign.missions[mission_idx]
        .profile(profiles)
        .mission_filename
        .clone();
    super::ensure_shipping_mission(args, &mission_id, &campaign, profiles, false, |_| {}).await?;
    let campaign = super::establish_mission_restart_boundary(campaign, rng_seed, sim_config);
    let loading = HeadlessLoadStage::begin(
        #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
        &crate::multiplayer::MultiplayerCampaignSession::default(),
        location,
        args,
        &mission_id,
        rng_seed,
        sim_config,
        None,
    )
    .await?;
    let bootstrap = loading
        .load_level(
            campaign,
            profiles,
            mission_idx,
            location,
            args,
            rng_seed,
            sim_config,
        )
        .map_err(|error| error.message)?;
    drop(bootstrap);
    Ok(())
}

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
            args.global_options
                .replay_launches()
                .pending_mission()
                .is_some(),
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
        let bootstrap = match loading.load_level(
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
        bootstrap.report_spellforge_startup();
        let bootstrap = match Box::new(bootstrap).prepare_audio(None, profiles) {
            Ok(bootstrap) => bootstrap,
            Err((bootstrap, error)) => {
                let (campaign, rng_seed, sim_config) = bootstrap.into_campaign_and_simulation();
                return HeadlessBuildOutcome::Finished(MissionOutcome::from_engine(
                    campaign,
                    rng_seed,
                    sim_config,
                    Err(error),
                ));
            }
        };
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
        callbacks.begin_profile_clock_session();

        if let Err(error) = crate::lua_session::validate_launch_mode(
            args,
            args.global_options
                .replay_launches()
                .pending_mission()
                .is_some(),
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

        stage.bootstrap.report_spellforge_startup();
        timer.step("spellforge startup");
        stage
            .bootstrap
            .sign_ranked_session_before_frame_zero(args)
            .await;
        timer.step("ranked genesis signing");
        let stage = match stage.prepare_audio(profiles) {
            Ok(stage) => stage,
            Err((bootstrap, error)) => {
                let (campaign, rng_seed, sim_config) = bootstrap.into_campaign_and_simulation();
                return InteractiveBuildOutcome::Finished(MissionOutcome::from_engine(
                    campaign,
                    rng_seed,
                    sim_config,
                    Err(error),
                ));
            }
        };
        timer.step("audio prepare");
        let (mut bootstrap, frontend) = stage.assemble_frontend(window, profiles, args).await;
        let frontend = match frontend {
            Ok(frontend) => frontend,
            Err(error) => {
                let (campaign, rng_seed, sim_config) = bootstrap.into_campaign_and_simulation();
                return InteractiveBuildOutcome::Finished(MissionOutcome::from_engine(
                    campaign,
                    rng_seed,
                    sim_config,
                    Err(error),
                ));
            }
        };
        timer.step("frontend assembly");

        bootstrap.0.prepare_interactive_entry(callbacks, args);
        bootstrap.0.complete_restart_save(callbacks).await;
        let mission = bootstrap.finish_interactive(frontend, window.width, window.height, args);
        timer.step("mission entry + HUD finish + runtime/replay init");
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
        AudioPreparedBootstrap, MissionFrontendKind, MissionSpec, MultiplayerSetupFailurePolicy,
        RestartSaveState, built_in_mission_assets_for_loaded_level,
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

    fn scratch_bootstrap_fixture() -> super::MissionBootstrap {
        use super::{LoadedMissionCore, MissionBootstrap};
        use robin_engine::engine::{Engine, EngineArgs, LevelAssets, LevelLoadArgs};

        let mut assets = LevelAssets::new();
        // Seed a valid campaign/profile fixture, then construct a level with
        // distinct proto and map names so saves must retain the loaded map.
        let fixture = Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut assets)
            .expect("fixture campaign");
        let mut campaign = fixture.campaign().clone();
        campaign.values[CampaignValue::MissionLength] = 23;
        campaign.set_ares(0);
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
        MissionBootstrap::new(
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
        )
    }

    #[test]
    fn frontend_retention_future_does_not_embed_the_simulation_owner() {
        let bootstrap = AudioPreparedBootstrap(Box::new(scratch_bootstrap_fixture()));
        let future = bootstrap
            .retain_during_frontend_assembly(async |_| Err("assembly rejected".to_owned()));
        assert!(
            std::mem::size_of_val(&future) < 4096,
            "frontend ownership handoff must retain a pointer, not inline simulation state: {} bytes",
            std::mem::size_of_val(&future),
        );
    }

    #[test]
    fn production_frontend_futures_keep_simulation_and_upload_owners_boxed() {
        use super::{DecodingInterfaceResources, MissionProcessResources};
        use super::{LoadedInteractiveStage, MissionBootstrap, MissionLoadingScreen};
        use crate::main_entry::CliArgs;
        use crate::window::GameWindow;
        use robin_engine::profiles::ProfileManager;

        // Infer the concrete future from the actual production function, not
        // a smaller stand-in closure. No GPU is needed to inspect its layout.
        fn upload_size<F: Future>(
            _: impl FnOnce(
                &'static mut MissionBootstrap,
                MissionProcessResources<DecodingInterfaceResources>,
                MissionLoadingScreen,
                &'static mut GameWindow,
                &'static ProfileManager,
                &'static CliArgs,
            ) -> F,
        ) -> usize {
            std::mem::size_of::<F>()
        }
        fn assembly_size<F: Future>(
            _: impl FnOnce(
                LoadedInteractiveStage<AudioPreparedBootstrap>,
                &'static mut GameWindow,
                &'static ProfileManager,
                &'static CliArgs,
            ) -> F,
        ) -> usize {
            std::mem::size_of::<F>()
        }

        type Stage = LoadedInteractiveStage<AudioPreparedBootstrap>;
        let upload = upload_size(Stage::assemble_process_frontend);
        assert_eq!(
            upload,
            std::mem::size_of::<
                futures::future::LocalBoxFuture<
                    'static,
                    Result<super::InteractiveFrontendAssembly, String>,
                >,
            >()
        );
        let assembly = assembly_size(Stage::assemble_frontend);
        assert_eq!(assembly, upload, "both production handoffs must be boxed");
        // The real loading-screen/process handoff is about 29 KiB on native
        // desktop builds. Budget one such construction, never an inline copy
        // in every ancestor future (the source of the earlier stack overflow).
        let assembly_construction = assembly_size(Stage::assemble_frontend_inner);
        assert!(
            assembly_construction < 32 * 1024,
            "production assembly construction is {assembly_construction} bytes"
        );
        // Boxing still constructs this future once on the stack. Keep a
        // separate budget for that real construction (before boxing).
        let construction = upload_size(Stage::assemble_process_frontend_inner);
        assert!(
            construction < 128 * 1024,
            "production upload future construction is {construction} bytes"
        );
    }

    #[test]
    fn audio_preparation_failure_returns_the_exact_unadvanced_mission() {
        let bootstrap = Box::new(scratch_bootstrap_fixture());
        let original_allocation = bootstrap.loaded.engine.campaign().missions.as_ptr();
        let expected = serde_json::to_value(bootstrap.loaded.engine.campaign()).unwrap();
        let profiles = std::sync::Arc::clone(&bootstrap.loaded.assets.profile_manager);
        let (bootstrap, error) = match bootstrap.prepare_audio(None, &profiles) {
            Ok(_) => panic!("scratch host unexpectedly acquired resource authority"),
            Err(failure) => failure,
        };
        assert!(!error.is_empty());
        assert!(matches!(bootstrap.restart_save, RestartSaveState::Absent));
        let (campaign, _, _) = bootstrap.into_campaign_and_simulation();
        assert_eq!(campaign.missions.as_ptr(), original_allocation);
        assert_eq!(serde_json::to_value(campaign).unwrap(), expected);
    }

    #[test]
    fn serialized_audio_stage_cannot_forge_preparation_authority() {
        let prepared = AudioPreparedBootstrap(Box::new(scratch_bootstrap_fixture()));
        let diagnostic = serde_json::to_string(&prepared).unwrap();
        assert!(serde_json::from_str::<AudioPreparedBootstrap>(&diagnostic).is_err());
    }

    #[test]
    fn consuming_frontend_failure_retains_the_original_campaign_and_simulation() {
        let bootstrap = AudioPreparedBootstrap(Box::new(scratch_bootstrap_fixture()));
        let expected_campaign = serde_json::to_value(bootstrap.0.loaded.engine.campaign()).unwrap();
        let original_allocation = bootstrap.0.loaded.engine.campaign().missions.as_ptr();
        let expected_config = bootstrap.0.loaded.engine_sim_config;
        let expected_seed = bootstrap.0.loaded.engine_rng_seed;
        let (bootstrap, result) = futures::executor::block_on(
            bootstrap.retain_during_frontend_assembly(async |bootstrap| {
                // Exercise the real non-GPU preparation failure: a scratch
                // host cannot borrow application-owned resource readers.
                let error = match bootstrap.host.preparation_files() {
                    Err(error) => error,
                    Ok(_) => panic!("scratch fixture unexpectedly has resource authority"),
                };
                Err(error)
            }),
        );
        assert!(result.is_err());
        assert_eq!(
            bootstrap.0.loaded.engine.campaign().missions.as_ptr(),
            original_allocation
        );
        let (campaign, seed, config) = bootstrap.into_campaign_and_simulation();
        assert_eq!(campaign.missions.as_ptr(), original_allocation);
        assert_eq!(serde_json::to_value(campaign).unwrap(), expected_campaign);
        assert_eq!(seed, expected_seed);
        assert_eq!(config, expected_config);
    }

    #[test]
    fn restart_completion_preserves_captured_boundary_and_failure_never_registers_it() {
        let bootstrap = scratch_bootstrap_fixture();
        let boundary = super::super::runtime::BootstrapSaveBoundary::capture(
            &bootstrap.loaded.engine,
            &bootstrap.host,
            &bootstrap.game,
            None,
        );
        let expected = serde_json::to_value(boundary).unwrap();
        let mut successful = RestartSaveState::Pending(boundary);
        assert!(!successful.observe_completion(Ok(crate::savegame::SaveWriteStatus::Queued)));
        assert!(matches!(successful, RestartSaveState::Pending(_)));
        assert!(successful.observe_completion(Ok(crate::savegame::SaveWriteStatus::Completed)));
        let RestartSaveState::Completed(captured) = successful else {
            panic!("successful completion lost its captured boundary");
        };
        assert_eq!(serde_json::to_value(captured).unwrap(), expected);

        let mut failed = RestartSaveState::Pending(boundary);
        assert!(failed.observe_completion(Err(anyhow::anyhow!("disk publication failed"))));
        assert!(matches!(failed, RestartSaveState::Absent));
    }

    #[test]
    fn bootstrap_installs_save_assets_and_tracks_failed_restart_creation() {
        let mut bootstrap = scratch_bootstrap_fixture();
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
        let mut callbacks = crate::main_entry::RustCallbacks::new(application_context).unwrap();
        bootstrap.prepare_interactive_entry(&mut callbacks, &crate::main_entry::CliArgs::default());
        assert!(matches!(bootstrap.restart_save, RestartSaveState::Absent));
        assert_eq!(
            bootstrap.loaded.engine.campaign().values[CampaignValue::MissionLength],
            0
        );

        let mut lost = scratch_bootstrap_fixture();
        lost.game.is_sherwood = true;
        lost.prepare_interactive_entry(&mut callbacks, &crate::main_entry::CliArgs::default());
        assert_eq!(
            lost.loaded.engine.campaign().values[CampaignValue::MissionLength],
            23
        );
        assert!(matches!(lost.restart_save, RestartSaveState::Absent));
    }

    #[test]
    fn replay_bootstrap_creates_no_restart_recording_or_autosave() {
        let mut bootstrap = scratch_bootstrap_fixture();
        let directory = tempfile::tempdir().unwrap();
        let save_root = directory.path().to_string_lossy().into_owned();
        let mut players =
            robin_engine::player_profile::PlayerProfileManager::new(save_root.clone());
        let player = players.create_profile(
            "Replay Test".into(),
            robin_engine::player_profile::DifficultyLevel::Medium,
        );
        players.set_active(player);
        let files = std::sync::Arc::new(
            robin_engine::sbfile::SbFileSystem::new(std::sync::Arc::new(
                robin_util::asset_fs::AssetVfs::default(),
            ))
            .snapshot(),
        );
        let context = crate::host::ApplicationContext::complete_with_localization_and_files(
            crate::player_profile_store::PlayerProfileStore::for_directory(&save_root),
            robin_engine::engine::GlobalOptions::default(),
            players,
            crate::key_config_store::KeyConfigStore::new(save_root),
            None,
            crate::localization::LocalizationService::disabled(),
            Some(files),
        )
        .unwrap();
        bootstrap.host =
            crate::host::Host::new(context.clone().try_into().unwrap(), 1024.0, 768.0).unwrap();
        let mut callbacks = crate::main_entry::RustCallbacks::new(context).unwrap();
        let descriptor = bootstrap.game.mission_assets().unwrap().clone();
        let replay: robin_engine::replay::ReplayData = robin_engine::replay::ReplayFile {
            header: robin_engine::replay::ReplayHeader {
                mission_id: descriptor.mission_basename.clone(),
                mission_assets: descriptor.clone(),
                rng_seed: 0,
                sim_config: bootstrap.loaded.engine_sim_config,
                spellforge_package: None,
                version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
                total_frames: 0,
                rankability: robin_engine::replay_rankability::ReplayRankability::rankable(),
                campaign: bitcode::encode(&bootstrap.loaded.replay_campaign),
            },
            frames: Default::default(),
            hashes: Default::default(),
            save_markers: Default::default(),
            load_backs: Default::default(),
        }
        .try_into()
        .unwrap();
        let args = crate::main_entry::CliArgs {
            replay_data: Some(replay),
            global_options: callbacks.application_context().clone(),
            ..Default::default()
        };
        let profiles = std::sync::Arc::clone(&bootstrap.loaded.assets.profile_manager);
        let prepared = match Box::new(bootstrap).prepare_audio(None, &profiles) {
            Ok(prepared) => prepared,
            Err((_, error)) => panic!("initialized replay fixture must prepare audio: {error}"),
        };
        assert_eq!(
            prepared.0.loaded.engine.campaign().values[CampaignValue::MissionLength],
            23
        );
        assert!(matches!(prepared.0.restart_save, RestartSaveState::Absent));
        let mut bootstrap = *prepared.0;
        let files_before = std::fs::read_dir(directory.path()).unwrap().count();
        bootstrap.prepare_interactive_entry(&mut callbacks, &args);
        assert!(matches!(bootstrap.restart_save, RestartSaveState::Absent));
        assert!(!callbacks.save_manager.has_restart_save());
        let replay = super::super::replay_init::init_replay_and_rollback(
            &bootstrap.loaded.replay_campaign,
            std::sync::Arc::new(bootstrap.loaded.assets),
            &args,
            0,
            &descriptor.mission_basename,
            descriptor.clone(),
            0,
            bootstrap.loaded.engine_sim_config,
            false,
        );
        assert!(replay.player.is_some());
        assert!(replay.recorder.is_none());
        assert!(replay.rollback_checker.is_none());
        let allowed =
            crate::autosave::session_allows_autosave(true, false, replay.player.is_some(), false);
        assert!(
            callbacks
                .plan_autosave(allowed, true, 1, 0, false, false)
                .is_none()
        );
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            files_before
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
