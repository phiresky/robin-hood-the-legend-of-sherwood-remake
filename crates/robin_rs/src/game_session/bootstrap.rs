//! Ordered construction of complete interactive and true-headless missions.

use super::headless::{HeadlessMission, HeadlessPolicy};
use super::interactive::{InteractiveFrontend, InteractiveMission};
use super::replay_init::init_replay_and_rollback;
use super::runtime::{
    FrameContract, MissionControl, MissionRuntime, MissionWorld, TimelineRuntime,
};
use super::setup::{LoadedMissionCore, setup_mission_audio};
use crate::Host;
use crate::game::Game;
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
    spellforge_started: bool,
    audio_prepared: bool,
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
            spellforge_started: false,
            audio_prepared: false,
        }
    }

    /// Run required Spellforge startup after SCB `Initialize` in the engine
    /// constructor and before audio/replay construction.
    pub(super) fn start_required_spellforge(
        &mut self,
    ) -> Result<(), crate::lua_session::SpellforgeSessionError> {
        if let Some(lua) = self.host.lua_session.as_ref() {
            tracing::info!(
                "Lua: firing Initialize for mission '{}' (seed={})",
                lua.mission_basename(),
                self.loaded.engine_rng_seed,
            );
            self.loaded
                .engine
                .with_mission_script_game_host_and_rng(|native_parts| {
                    lua.run_required_startup_events(
                        native_parts,
                        self.loaded.engine_rng_seed as i32,
                    )
                })?;
        }
        self.spellforge_started = true;
        Ok(())
    }

    pub(super) fn prepare_audio(
        &mut self,
        backend: Option<&mut crate::audio_backend::KiraAudioBackend>,
        profiles: &ProfileManager,
    ) {
        assert!(
            self.spellforge_started,
            "mission audio must be prepared after required Spellforge startup"
        );
        setup_mission_audio(
            &mut self.host,
            backend,
            &self.loaded.engine,
            &mut self.loaded.assets,
            profiles,
            self.spec.location,
            &self.game.global_options.sound_directory,
        );
        self.audio_prepared = true;
    }

    pub(super) fn finish_interactive(
        self,
        frontend: InteractiveFrontend,
        args: &crate::main_entry::CliArgs,
        profiles: &ProfileManager,
    ) -> InteractiveMission {
        assert_eq!(self.spec.frontend, MissionFrontendKind::Interactive);
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
        assert!(
            self.audio_prepared,
            "replay/runtime construction must follow mission audio preparation"
        );
        let mission_id = self
            .loaded
            .engine
            .campaign()
            .and_then(|campaign| {
                campaign
                    .missions
                    .get(self.spec.mission_idx)
                    .map(|mission| mission.profile(profiles).mission_filename.clone())
            })
            .unwrap_or_else(|| format!("mission_{}", self.spec.mission_idx));
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

#[cfg(test)]
mod tests {
    use super::{MissionFrontendKind, MissionSpec};
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
}
