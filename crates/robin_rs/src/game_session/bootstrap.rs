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
    HeadlessEngineResources, LoadedInteractiveResources, LoadedMissionCore, MissionLoadError,
    MissionProcessResources, load_level_and_sprite_bank, pre_decode_maps_and_resources,
    setup_mission_audio,
};
use super::{MissionOutcome, install_pending_lua_session, setup_multiplayer_session};
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
                let campaign = self.loaded.engine.campaign();
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
        let campaign = self.loaded.engine.campaign();
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

    fn into_campaign(self) -> Campaign {
        self.loaded.engine.into_campaign()
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
        campaign: Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
    ) -> Result<LoadedInteractiveStage, MissionLoadError> {
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
    fn into_campaign(self) -> Campaign {
        self.bootstrap.into_campaign()
    }
    fn prepare_audio(&mut self, profiles: &ProfileManager) {
        self.loading
            .as_mut()
            .expect("interactive loading screen must exist until frontend assembly")
            .status("Loading mission audio...", 0.75);
        self.bootstrap.prepare_audio(
            self.process
                .as_mut()
                .expect("interactive process resources must exist until frontend assembly")
                .audio_backend
                .as_mut(),
            profiles,
        );
    }

    fn assemble_frontend(
        &mut self,
        window: &mut GameWindow,
        profiles: &ProfileManager,
        args: &crate::main_entry::CliArgs,
    ) -> InteractiveFrontendAssembly {
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
        );
        let short_briefings = self
            .process
            .as_mut()
            .expect("interactive process resources must exist until frontend assembly")
            .resolve_short_briefings(level_descriptors.as_ref());
        let background = self.bootstrap.loaded.pre_decoded_background.take();
        let minimap = self.bootstrap.loaded.pre_decoded_minimap.take();

        let renderer_config = self
            .loading
            .take()
            .expect("interactive loading screen must close before renderer construction")
            .close_before_renderer();
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
            self.process
                .take()
                .expect("interactive process resources must move into the frontend once"),
            LoadedInteractiveResources {
                level_descriptors,
                hud_fonts,
            },
            short_briefings,
            args,
            self.bootstrap.spec.mission_idx,
            self.bootstrap.spec.location,
        );
        frontend
    }
}

/// A fully constructed interactive mission. The campaign remains inside its
/// engine until consuming finalization returns it in [`MissionOutcome`].
pub(super) struct BuiltInteractiveMission {
    mission: InteractiveMission,
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
            window,
            callbacks,
            profiles,
            args,
        };
        self.mission.run(&mut services).await
    }

    pub(super) fn finish(self, result: Result<GameCode, String>) -> MissionOutcome {
        MissionOutcome::new(self.mission.runtime.into_campaign(), result)
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
        campaign: Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
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

    pub(super) fn finish(self, outcome: HeadlessMissionOutcome) -> MissionOutcome {
        MissionOutcome::new(self.mission.runtime.into_campaign(), Ok(outcome.code))
    }
}

pub(super) enum HeadlessBuildOutcome {
    Ready(BuiltHeadlessMission),
    Finished(MissionOutcome),
}

pub(super) struct HeadlessMissionBuilder;

impl HeadlessMissionBuilder {
    pub(super) fn build(
        callbacks: &mut RustCallbacks,
        campaign: Campaign,
        profiles: &ProfileManager,
        mission_idx: usize,
        location: MissionLocation,
        args: &crate::main_entry::CliArgs,
    ) -> HeadlessBuildOutcome {
        if let Err(error) = crate::lua_session::validate_launch_mode(
            args,
            crate::http_server::peek_pending_replay_mission_id().is_some(),
        ) {
            return HeadlessBuildOutcome::Finished(MissionOutcome::new(
                campaign,
                Err(error.to_string()),
            ));
        }
        assert!(
            args.headless,
            "headless builder requires headless launch mode"
        );

        let loading = match HeadlessLoadStage::begin(location, args) {
            Err(error) => {
                return HeadlessBuildOutcome::Finished(MissionOutcome::new(campaign, Err(error)));
            }
            Ok(HeadlessLoadStart::Ready(stage)) => stage,
            Ok(HeadlessLoadStart::Finished(code)) => {
                return HeadlessBuildOutcome::Finished(MissionOutcome::new(campaign, Ok(code)));
            }
        };
        let mut bootstrap =
            match loading.load_level(campaign, profiles, mission_idx, location, args) {
                Ok(bootstrap) => bootstrap,
                Err(error) => {
                    return HeadlessBuildOutcome::Finished(MissionOutcome::new(
                        error.campaign,
                        Err(error.message),
                    ));
                }
            };
        if let Err(error) = bootstrap.start_required_spellforge() {
            return HeadlessBuildOutcome::Finished(MissionOutcome::new(
                bootstrap.into_campaign(),
                Err(error.to_string()),
            ));
        }
        bootstrap.prepare_audio(None, profiles);
        bootstrap.start_campaign_clock(callbacks);
        let mission = bootstrap.finish_headless(args, profiles, HeadlessPolicy::replay_runner());
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
    ) -> InteractiveBuildOutcome {
        if let Err(error) = crate::lua_session::validate_launch_mode(
            args,
            crate::http_server::peek_pending_replay_mission_id().is_some(),
        ) {
            return InteractiveBuildOutcome::Finished(MissionOutcome::new(
                campaign,
                Err(error.to_string()),
            ));
        }
        assert!(
            !args.headless,
            "interactive builder cannot construct headless shims"
        );

        let loading = match InteractiveLoadStage::begin(
            window,
            &campaign,
            profiles,
            mission_idx,
            location,
            args,
        )
        .await
        {
            Ok(InteractiveLoadStart::Ready(stage)) => stage,
            Ok(InteractiveLoadStart::Finished(code)) => {
                return InteractiveBuildOutcome::Finished(MissionOutcome::new(campaign, Ok(code)));
            }
            Err(error) => {
                return InteractiveBuildOutcome::Finished(MissionOutcome::new(
                    campaign,
                    Err(error),
                ));
            }
        };
        let mut stage =
            match loading.load_level(window, campaign, profiles, mission_idx, location, args) {
                Ok(stage) => stage,
                Err(error) => {
                    return InteractiveBuildOutcome::Finished(MissionOutcome::new(
                        error.campaign,
                        Err(error.message),
                    ));
                }
            };

        if let Err(error) = stage.bootstrap.start_required_spellforge() {
            return InteractiveBuildOutcome::Finished(MissionOutcome::new(
                stage.into_campaign(),
                Err(error.to_string()),
            ));
        }
        stage.prepare_audio(profiles);
        let mut frontend = stage.assemble_frontend(window, profiles, args);

        if let Some(code) = run_lost_sherwood_gate(
            window,
            &stage.bootstrap.host,
            &stage.bootstrap.loaded.engine,
            &mut frontend,
        )
        .await
        {
            return InteractiveBuildOutcome::Finished(MissionOutcome::new(
                stage.into_campaign(),
                Ok(code),
            ));
        }

        stage.bootstrap.start_campaign_clock(callbacks);
        stage.bootstrap.setup_restart_or_sherwood(callbacks, args);
        let frontend = frontend.finish(window.width, window.height);
        let bootstrap = stage.bootstrap;
        let mission = bootstrap.finish_interactive(frontend, args, profiles);
        InteractiveBuildOutcome::Ready(BuiltInteractiveMission { mission })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MissionBootstrapLifecycle, MissionBootstrapPhase, MissionFrontendKind, MissionSpec,
    };
    use crate::campaign::{Campaign, CampaignValue};
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
