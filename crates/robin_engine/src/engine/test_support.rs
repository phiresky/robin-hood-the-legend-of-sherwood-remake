//! Shared fixtures and unwind-safe, observational test instrumentation.
pub(crate) mod actors;
pub(crate) mod asm;

use super::commands::SelectionCommandBatchMode;
use super::{EngineInner, HostDisplayState, InputState, LevelAssets};
use crate::player_command::{PlayerCommand, PlayerInput};

/// Sprite action-conversion table with every action unmapped; tests then map
/// the few rows their synthetic scripts provide.
pub(crate) fn unmapped_conversion() -> Vec<u16> {
    vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END]
}

impl EngineInner {
    /// Default [`LevelAssets`] completed by
    /// [`super::complete_test_runtime_fixture`] for the entities added so far.
    ///
    /// The fixture snapshots the current roster, so call this after the
    /// scenario's actors are registered, exactly where the two-step
    /// `LevelAssets::new()` + `complete_test_runtime_fixture` pair used to be.
    pub(crate) fn test_runtime_assets(&mut self) -> LevelAssets {
        let mut assets = LevelAssets::new();
        super::complete_test_runtime_fixture(self, &mut assets);
        assets
    }
}

/// Test adapters over the authoritative command dispatcher
/// (`commands::apply_commands_authoritative`). Production callers go
/// through `Engine::advance_frame`; unit tests across the engine drive
/// batches through these.
impl EngineInner {
    /// Apply a batch of player commands for the current frame.
    /// Per-frame scroll dedupe (`frame_scrolled`) is reset at the end
    /// of `perform_hourglass` (after `tick_display_state`), not here —
    /// the live game pushes scroll commands via `apply_command`
    /// (singular) one-at-a-time during input handling, while the
    /// rollback path calls `apply_commands` in a batch; both paths
    /// must dedupe identically, and the display-state tick still needs
    /// to see which directions were pressed this frame.
    pub(crate) fn apply_commands(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        commands: &[PlayerInput],
    ) {
        self.apply_commands_with_mode(
            sim,
            display,
            input,
            assets,
            commands,
            SelectionCommandBatchMode::InferNestedSelection,
        );
    }

    pub(crate) fn apply_commands_with_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        commands: &[PlayerInput],
        mode: SelectionCommandBatchMode,
    ) {
        let event_start = self.feedback.pending_side_effects.host_events.len();
        let mut camera = self.feedback.cutscene_camera.display.clone();
        self.apply_commands_authoritative(sim, &mut camera, assets, commands, mode);
        self.feedback.cutscene_camera.display = camera;
        for event in self.feedback.pending_side_effects.host_events[event_start..]
            .iter()
            .cloned()
        {
            display.apply_host_event(input, event);
        }
    }

    /// Apply a batch of commands tagged as issued by the local seat.
    /// Convenience wrapper around [`Self::apply_commands`] for the
    /// single-player input pipeline: each raw [`PlayerCommand`] is
    /// stamped with [`crate::player_command::PlayerId::HOST`] before
    /// dispatch.  Live multiplayer pipelines should build
    /// [`PlayerInput`]s with their `Host::local_seat` and call
    /// [`Self::apply_commands`] directly so the seat tag is
    /// data-driven.
    pub(crate) fn apply_local_commands(
        &mut self,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        commands: &[PlayerCommand],
    ) {
        let sim = self.control.simulation_context();
        let commands = commands
            .iter()
            .cloned()
            .map(PlayerInput::host)
            .collect::<Vec<_>>();
        self.apply_commands(&sim, display, input, assets, &commands);
    }

    /// Apply a single [`PlayerCommand`] as if it came from
    /// [`crate::player_command::PlayerId::HOST`].
    ///
    /// Test adapter over the same authoritative batch dispatcher used by
    /// [`crate::engine::Engine::advance_frame`].
    pub(crate) fn apply_command(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        cmd: &PlayerCommand,
    ) {
        self.apply_commands(
            sim,
            display,
            input,
            assets,
            &[PlayerInput::host(cmd.clone())],
        );
    }
}

/// Axis-aligned walkable test geometry, with stable counter-clockwise vertices.
pub(crate) fn square_sector(
    number: i16,
    layer: u16,
    min: crate::coordinates::MapPoint,
    max: crate::coordinates::MapPoint,
) -> crate::fast_find_grid::GridSector {
    use crate::coordinates::{MapBBox, MapPoint};
    crate::fast_find_grid::GridSector {
        points: vec![
            min,
            MapPoint::new(max.x, min.y),
            max,
            MapPoint::new(min.x, max.y),
        ],
        bounding_box: MapBBox::from_coords(min.x, min.y, max.x, max.y),
        sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        layer,
        sector_number: crate::sector::SectorNumber::new(number),
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    }
}

/// Supply explicit ordinary topology for fixtures that query only sector
/// metadata. No polygon is invented: geometric routing tests must install
/// their real geometry separately. Existing canonical sectors are preserved.
pub(crate) fn ensure_ordinary_sector(
    engine: &mut super::EngineInner,
    raw_sector: u16,
    layer: u16,
) -> crate::position_interface::SectorHandle {
    let number = crate::sector::SectorNumber::new(raw_sector as i16);
    let grid = engine.world.fast_grid_mut();
    if let Some(&index) = grid.level.sector_number_map.get(&number) {
        let sector = grid
            .level
            .sectors
            .get(index)
            .expect("fixture sector map must resolve");
        assert_eq!(sector.layer, layer, "fixture sector has a different layer");
    } else {
        grid.add_sector(
            crate::fast_find_grid::GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
                layer,
                sector_number: number,
                door_index: None,
                lift_type: None,
                lift_direction: 0,
                force_crouched: false,
                building_index: None,
                low_exit_point: None,
                high_exit_point: None,
                lowest_door_index: None,
                jump_line_indices: Vec::new(),
                gate_indices: Vec::new(),
                underlying_sector: None,
            },
            layer,
        );
    }
    crate::position_interface::SectorHandle::from_number(number)
}

pub(crate) use robin_test_support::probe::Probe;
