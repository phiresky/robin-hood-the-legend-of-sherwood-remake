//! Ordered construction of complete interactive and true-headless missions.

use super::debriefing::run_lost_sherwood_gate;
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
    HeadlessEngineResources, LoadedInteractiveResources, LoadedMissionCore,
    MissionProcessResources, load_level_and_sprite_bank, pre_decode_maps_and_resources,
    setup_mission_audio,
};
use super::{MissionCampaignLease, install_pending_lua_session, setup_multiplayer_session};
use crate::Host;
use crate::campaign::Campaign;
use crate::game::Game;
use crate::game::GameCallbacks;
use crate::game_operation::GameCode;
use crate::loading_screen::{LoadingDatadirKind, LoadingScreenRenderer};
use crate::main_entry::{RustCallbacks, current_mission_id, detect_demo_mode, resolve_loading_pak};
use crate::player_profile::PlayerProfileManager;
use crate::window::GameWindow;
use robin_engine::graphic_config::TextureScaleMode;
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

impl MissionBootstrap {
    pub(super) fn new(
        spec: MissionSpec,
        host: Host,
        game: Game,
        loaded: LoadedMissionCore,
    ) -> Self {
        Self {
            spec,
            host,
            game,
            loaded,
            lifecycle: MissionBootstrapLifecycle::new(),
        }
    }

    /// Run required Spellforge startup after SCB `Initialize` in the engine
    /// constructor and before audio/replay construction.
    pub(super) fn start_required_spellforge(
        &mut self,
    ) -> Result<(), crate::lua_session::SpellforgeSessionError> {
        self.lifecycle
            .require(MissionBootstrapPhase::LevelInitialized);
        if let Some(lua) = self.host.lua_session.as_ref() {
            tracing::info!(
                "Lua: firing Initialize for mission '{}' (seed={})",
                lua.mission_basename(),
                self.loaded.engine_rng_seed,
            );
            self.loaded.engine.with_mission_script_game_host_and_rng(
                &self.loaded.assets,
                |native_parts| {
                    lua.run_required_startup_events(
                        native_parts,
                        self.loaded.engine_rng_seed as i32,
                    )
                },
            )?;
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
    ) {
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
        );
        self.lifecycle.advance(
            MissionBootstrapPhase::SpellforgeStarted,
            MissionBootstrapPhase::AudioPrepared,
        );
    }

    /// Start the campaign segment clock after the lost-Sherwood gate, matching
    /// the original `GameLoop` boundary.
    pub(super) fn start_campaign_clock(&mut self, callbacks: &mut RustCallbacks) {
        self.lifecycle.require(MissionBootstrapPhase::AudioPrepared);
        self.loaded.engine.campaign_reset_mission_length();
        <RustCallbacks as GameCallbacks>::start_play_time(callbacks);
        self.lifecycle.advance(
            MissionBootstrapPhase::AudioPrepared,
            MissionBootstrapPhase::CampaignClockStarted,
        );
    }

    /// Capture the pristine restart state for a tactical mission, or raise the
    /// initial Sherwood campaign map. This must be the last setup stage before
    /// replay/runtime construction.
    pub(super) fn setup_restart_or_sherwood(
        &mut self,
        callbacks: &mut RustCallbacks,
        args: &crate::main_entry::CliArgs,
    ) {
        self.lifecycle
            .require(MissionBootstrapPhase::CampaignClockStarted);
        if !self.game.is_sherwood {
            if args.mission_start_map_output.is_none() {
                let campaign = self
                    .loaded
                    .engine
                    .campaign()
                    .expect("restart snapshot requires the engine campaign");
                let mission_id = current_mission_id(campaign, &self.loaded.assets.profile_manager);
                callbacks.save_manager.write_restart_save_background(
                    &mut self.host,
                    &self.game,
                    &self.loaded.engine,
                    mission_id,
                    Some(&self.loaded.assets.profile_manager),
                    None,
                );
            }
        } else if !args.sherwood {
            self.game.show_campaign_map();
        }
        self.lifecycle.advance(
            MissionBootstrapPhase::CampaignClockStarted,
            MissionBootstrapPhase::EntryPrepared,
        );
    }

    pub(super) fn finish_interactive(
        self,
        frontend: InteractiveFrontend,
        args: &crate::main_entry::CliArgs,
        profiles: &ProfileManager,
    ) -> InteractiveMission {
        assert_eq!(self.spec.frontend, MissionFrontendKind::Interactive);
        self.lifecycle.require(MissionBootstrapPhase::EntryPrepared);
        let wait_for_multiplayer_start = self.host.net.is_some();
        InteractiveMission {
            runtime: self.finish_runtime(
                args,
                profiles,
                FrameContract::Graphical,
                wait_for_multiplayer_start,
            ),
            frontend,
        }
    }

    pub(super) fn finish_headless(
        self,
        args: &crate::main_entry::CliArgs,
        profiles: &ProfileManager,
        policy: HeadlessPolicy,
    ) -> HeadlessMission {
        assert_eq!(self.spec.frontend, MissionFrontendKind::Headless);
        self.lifecycle
            .require(MissionBootstrapPhase::CampaignClockStarted);
        HeadlessMission {
            runtime: self.finish_runtime(
                args,
                profiles,
                FrameContract::Headless,
                policy.wait_for_multiplayer_start,
            ),
            policy,
        }
    }

    fn finish_runtime(
        mut self,
        args: &crate::main_entry::CliArgs,
        profiles: &ProfileManager,
        contract: FrameContract,
        wait_for_multiplayer_start: bool,
    ) -> MissionRuntime {
        assert!(matches!(
            self.lifecycle.phase(),
            MissionBootstrapPhase::CampaignClockStarted | MissionBootstrapPhase::EntryPrepared
        ));
        let campaign = self
            .loaded
            .engine
            .campaign()
            .expect("mission runtime construction requires the engine campaign");
        let mission = campaign.missions.get(self.spec.mission_idx).unwrap_or_else(|| {
            panic!(
                "mission runtime construction requires campaign mission index {} (campaign has {})",
                self.spec.mission_idx,
                campaign.missions.len()
            )
        });
        let mission_id = mission.profile(profiles).mission_filename.clone();
        let assets = Arc::new(self.loaded.assets);
        let replay = init_replay_and_rollback(
            &mut self.loaded.engine,
            Arc::clone(&assets),
            args,
            self.spec.mission_idx,
            &mission_id,
            self.loaded.engine_rng_seed,
            self.host.net.is_some(),
        );
        let timeline = TimelineRuntime::new(
            replay,
            contract,
            wait_for_multiplayer_start,
            self.host.local_seat == robin_engine::player_command::PlayerId::HOST,
        );
        debug_assert_eq!(timeline.frame_contract(), contract);
        let manager = robin_engine::engine_manager::EngineManager::new(
            self.loaded.engine,
            self.host.local_seat,
        );
        let control = MissionControl::new(
            timeline.initially_paused(),
            manager.engine.weather().night_color,
        );
        MissionRuntime::new(
            MissionWorld::new(self.host, self.game, manager, assets, self.loaded.dev),
            timeline,
            control,
        )
    }
}

/// Owns the temporary loading renderer and the presentation configuration it
/// resolved. Consuming [`Self::close_before_renderer`] is the only way to
/// obtain that configuration for the game renderer.
struct MissionLoadingScreen {
    renderer: Option<LoadingScreenRenderer>,
    renderer_config: MissionRendererConfig,
}

impl MissionLoadingScreen {
    fn open(
        window: &mut GameWindow,
        campaign: &Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
    ) -> Self {
        const LOADING_MAX_LEVEL: f32 = 22.0;

        let _ = window.poll_events();
        let proto_level_filename = campaign
            .missions
            .get(mission_idx)
            .map(|mission| mission.profile(profiles).proto_level_filename.clone());
        let loading_pak = resolve_loading_pak(proto_level_filename.as_deref(), None);
        let renderer_config = MissionRendererConfig {
            scale_mode: active_profile_scale_mode(),
            shader_preset: active_profile_shader_preset(),
        };
        let renderer = loading_pak.and_then(|path| {
            let datadir_kind = match detect_demo_mode().map(|(_, _, _, location)| location) {
                Some(MissionLocation::Leicester) => LoadingDatadirKind::DemoI,
                Some(MissionLocation::Lincoln) => LoadingDatadirKind::DemoII,
                _ => LoadingDatadirKind::FullGame,
            };
            LoadingScreenRenderer::new(
                window,
                &path,
                datadir_kind,
                LOADING_MAX_LEVEL,
                renderer_config.scale_mode,
            )
        });
        let mut stage = Self {
            renderer,
            renderer_config,
        };
        stage.status("Initializing audio...", 0.02);
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

    fn close_before_renderer(mut self) -> MissionRendererConfig {
        if let Some(renderer) = self.renderer.take() {
            renderer.close();
        }
        drop(self.renderer);
        self.renderer_config
    }
}

fn active_profile_scale_mode() -> TextureScaleMode {
    let profiles = PlayerProfileManager::global();
    profiles
        .as_ref()
        .and_then(|manager| manager.get_active())
        .map(|profile| profile.graphic_config.scale_mode)
        .unwrap_or_default()
}

fn active_profile_shader_preset() -> String {
    let profiles = PlayerProfileManager::global();
    profiles
        .as_ref()
        .and_then(|manager| manager.get_active())
        .map(|profile| profile.graphic_config.shader_preset.clone())
        .unwrap_or_default()
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

impl InteractiveLoadStage {
    async fn begin(
        window: &mut GameWindow,
        campaign: &Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
    ) -> Result<InteractiveLoadStart, String> {
        let mut loading = MissionLoadingScreen::open(window, campaign, profiles, mission_idx);
        let mut host = Host::new(window.width as f32, window.height as f32);
        install_pending_lua_session(&mut host, args).map_err(|error| error.to_string())?;
        if let Err(error) = setup_multiplayer_session(&mut host, args) {
            tracing::error!("{error}; returning to main menu");
            loading.status("Multiplayer connection failed", 1.0);
            if let Some(renderer) = loading.renderer.as_mut() {
                renderer.refresh();
                crate::window::sleep_ms(1200).await;
            }
            return Ok(InteractiveLoadStart::Finished(GameCode::Quit));
        }

        let mut game = Game::new(location);
        game.global_options = args.global_options.clone();
        loading.status("Loading process resources...", 0.05);
        let process = MissionProcessResources::load(&mut host, &game);
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
        campaign: &mut Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
    ) -> Result<LoadedInteractiveStage, String> {
        self.loading.status("Loading interface resources...", 0.12);
        let (ground_mark, titbit_rows, minimap_widget) =
            self.process.engine_setup_resources(&mut self.host);
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
        )?;
        Ok(LoadedInteractiveStage {
            bootstrap: MissionBootstrap::new(
                MissionSpec::interactive(mission_idx, location, screen_width, screen_height),
                self.host,
                self.game,
                loaded,
            ),
            process: self.process,
            loading: self.loading,
        })
    }
}

/// Owns the post-level-load state until renderer/frontend construction is
/// complete. Its methods are intentionally ordered and guarded by
/// `MissionBootstrapPhase`.
struct LoadedInteractiveStage {
    bootstrap: MissionBootstrap,
    process: MissionProcessResources,
    loading: MissionLoadingScreen,
}

impl LoadedInteractiveStage {
    fn prepare_audio(&mut self, profiles: &ProfileManager) {
        self.loading.status("Loading mission audio...", 0.75);
        self.bootstrap
            .prepare_audio(self.process.audio_backend.as_mut(), profiles);
    }

    fn assemble_frontend(
        mut self,
        window: &mut GameWindow,
        profiles: &ProfileManager,
        args: &crate::main_entry::CliArgs,
    ) -> (MissionBootstrap, InteractiveFrontendAssembly) {
        let LoadedInteractiveResources {
            level_descriptors,
            hud_fonts,
        } = pre_decode_maps_and_resources(
            Some(window),
            &mut self.loading.renderer,
            &mut self.bootstrap.loaded.engine,
            profiles,
            &self.bootstrap.host,
            &self.bootstrap.game,
        );
        let short_briefings = self
            .process
            .resolve_short_briefings(level_descriptors.as_ref());
        let background = self.bootstrap.loaded.pre_decoded_background.take();
        let minimap = self.bootstrap.loaded.pre_decoded_minimap.take();

        let renderer_config = self.loading.close_before_renderer();
        let mut renderer =
            InteractiveRendererAssembly::new_after_loading_screen(window, renderer_config);
        renderer.upload_maps(
            &self.bootstrap.loaded.engine,
            &mut self.bootstrap.host,
            background,
            minimap,
        );
        let frontend = renderer.assemble_process_frontend(
            window,
            &mut self.bootstrap.host,
            &self.bootstrap.game,
            &mut self.bootstrap.loaded.engine,
            &self.bootstrap.loaded.assets,
            self.process,
            LoadedInteractiveResources {
                level_descriptors,
                hud_fonts,
            },
            short_briefings,
            args,
            self.bootstrap.spec.mission_idx,
            self.bootstrap.spec.location,
        );
        (self.bootstrap, frontend)
    }
}

/// A fully constructed interactive mission paired with the session campaign
/// lease it must return exactly once.
pub(super) struct BuiltInteractiveMission<'a> {
    mission: InteractiveMission,
    campaign_return: MissionCampaignLease<'a>,
}

impl BuiltInteractiveMission<'_> {
    pub(super) async fn run(
        &mut self,
        window: &mut GameWindow,
        callbacks: &mut RustCallbacks,
        profiles: &ProfileManager,
        args: &crate::main_entry::CliArgs,
    ) -> Result<GameCode, String> {
        let mut services = MissionServices {
            window,
            callbacks,
            profiles,
            args,
        };
        self.mission.run(&mut services).await
    }

    pub(super) fn finish(mut self, outcome: Result<GameCode, String>) -> Result<GameCode, String> {
        self.campaign_return.finish(
            &mut self.mission.runtime.world.manager.engine,
            outcome,
            "interactive mission finalization",
        )
    }
}

pub(super) enum InteractiveBuildOutcome<'a> {
    Ready(BuiltInteractiveMission<'a>),
    Finished(Result<GameCode, String>),
}

/// Owns only the resource archives proven necessary for engine construction in
/// true-headless mode.
struct HeadlessLoadStage {
    host: Host,
    game: Game,
    resources: HeadlessEngineResources,
}

enum HeadlessLoadStart {
    Ready(HeadlessLoadStage),
    Finished(GameCode),
}

impl HeadlessLoadStage {
    fn begin(
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
    ) -> Result<HeadlessLoadStart, String> {
        let mut host = Host::new(1024.0, 768.0);
        install_pending_lua_session(&mut host, args).map_err(|error| error.to_string())?;
        if let Err(error) = setup_multiplayer_session(&mut host, args) {
            tracing::error!("{error}; aborting headless mission");
            return Ok(HeadlessLoadStart::Finished(GameCode::Quit));
        }
        let mut game = Game::new(location);
        game.global_options = args.global_options.clone();
        let resources = HeadlessEngineResources::load(&host);
        Ok(HeadlessLoadStart::Ready(Self {
            host,
            game,
            resources,
        }))
    }

    fn load_level(
        mut self,
        campaign: &mut Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
    ) -> Result<MissionBootstrap, String> {
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
        )?;
        Ok(MissionBootstrap::new(
            MissionSpec::headless(mission_idx, location),
            self.host,
            self.game,
            loaded,
        ))
    }
}

/// Complete true-headless mission plus the private session return sink for its
/// engine-owned campaign.
pub(super) struct BuiltHeadlessMission<'a> {
    mission: HeadlessMission,
    campaign_return: MissionCampaignLease<'a>,
}

impl BuiltHeadlessMission<'_> {
    pub(super) async fn run(
        &mut self,
        args: &crate::main_entry::CliArgs,
    ) -> HeadlessMissionOutcome {
        self.mission.run(args).await
    }

    pub(super) fn finish(mut self, outcome: HeadlessMissionOutcome) -> Result<GameCode, String> {
        self.campaign_return.finish(
            &mut self.mission.runtime.world.manager.engine,
            Ok(outcome.code),
            outcome.exit.campaign_restore_context(),
        )
    }
}

pub(super) enum HeadlessBuildOutcome<'a> {
    Ready(BuiltHeadlessMission<'a>),
    Finished(Result<GameCode, String>),
}

pub(super) struct HeadlessMissionBuilder;

impl HeadlessMissionBuilder {
    pub(super) fn build<'a>(
        callbacks: &mut RustCallbacks,
        campaign: &'a mut Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
    ) -> Result<HeadlessBuildOutcome<'a>, String> {
        crate::lua_session::validate_launch_mode(
            args,
            crate::http_server::peek_pending_replay_mission_id().is_some(),
        )
        .map_err(|error| error.to_string())?;
        assert!(
            args.headless,
            "headless builder requires headless launch mode"
        );

        let loading = match HeadlessLoadStage::begin(location, args)? {
            HeadlessLoadStart::Ready(stage) => stage,
            HeadlessLoadStart::Finished(code) => {
                return Ok(HeadlessBuildOutcome::Finished(Ok(code)));
            }
        };
        let mut bootstrap = loading.load_level(campaign, profiles, mission_idx, location, args)?;
        let campaign_lease = MissionCampaignLease::new(campaign);
        if let Err(error) = bootstrap.start_required_spellforge() {
            let outcome = campaign_lease.finish(
                &mut bootstrap.loaded.engine,
                Err(error.to_string()),
                "headless Spellforge startup failure",
            );
            return Ok(HeadlessBuildOutcome::Finished(outcome));
        }
        bootstrap.prepare_audio(None, profiles);
        bootstrap.start_campaign_clock(callbacks);
        let mission = bootstrap.finish_headless(args, profiles, HeadlessPolicy::replay_runner());
        Ok(HeadlessBuildOutcome::Ready(BuiltHeadlessMission {
            mission,
            campaign_return: campaign_lease,
        }))
    }
}

/// Ordered interactive bootstrap entry point. The body is deliberately a list
/// of ownership-stage transitions; the work for each stage lives on its
/// smallest owner above.
pub(super) struct InteractiveMissionBuilder;

impl InteractiveMissionBuilder {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn build<'a>(
        window: &mut GameWindow,
        callbacks: &mut RustCallbacks,
        campaign: &'a mut Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
    ) -> Result<InteractiveBuildOutcome<'a>, String> {
        crate::lua_session::validate_launch_mode(
            args,
            crate::http_server::peek_pending_replay_mission_id().is_some(),
        )
        .map_err(|error| error.to_string())?;
        assert!(
            !args.headless,
            "interactive builder cannot construct headless shims"
        );

        let loading = match InteractiveLoadStage::begin(
            window,
            campaign,
            profiles,
            mission_idx,
            location,
            args,
        )
        .await?
        {
            InteractiveLoadStart::Ready(stage) => stage,
            InteractiveLoadStart::Finished(code) => {
                return Ok(InteractiveBuildOutcome::Finished(Ok(code)));
            }
        };
        let mut loaded =
            loading.load_level(window, campaign, profiles, mission_idx, location, args)?;
        let campaign_lease = MissionCampaignLease::new(campaign);

        if let Err(error) = loaded.bootstrap.start_required_spellforge() {
            let outcome = campaign_lease.finish(
                &mut loaded.bootstrap.loaded.engine,
                Err(error.to_string()),
                "interactive Spellforge startup failure",
            );
            return Ok(InteractiveBuildOutcome::Finished(outcome));
        }
        loaded.prepare_audio(profiles);
        let (mut bootstrap, mut frontend) = loaded.assemble_frontend(window, profiles, args);

        if let Some(code) = run_lost_sherwood_gate(
            window,
            &bootstrap.host,
            &bootstrap.loaded.engine,
            &mut frontend,
        )
        .await
        {
            let outcome = campaign_lease.finish(
                &mut bootstrap.loaded.engine,
                Ok(code),
                "lost-campaign Sherwood exit",
            );
            return Ok(InteractiveBuildOutcome::Finished(outcome));
        }

        bootstrap.start_campaign_clock(callbacks);
        bootstrap.setup_restart_or_sherwood(callbacks, args);
        let frontend = frontend.finish(window.width, window.height);
        let mission = bootstrap.finish_interactive(frontend, args, profiles);
        Ok(InteractiveBuildOutcome::Ready(BuiltInteractiveMission {
            mission,
            campaign_return: campaign_lease,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MissionBootstrapLifecycle, MissionBootstrapPhase, MissionFrontendKind, MissionSpec,
    };
    use robin_engine::profiles::MissionLocation;

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
}
