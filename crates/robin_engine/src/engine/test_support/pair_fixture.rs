//! The recurring two-actor scenario: one square walkable sector, two actors
//! placed and bound to it, completed runtime assets, and a non-zero frame.
// Shared vocabulary: not every helper has a caller in every build.
#![allow(dead_code)]

use crate::coordinates::{MapPoint, MoveBox, WorldPoint3D};
use crate::element::{ActionState, Entity, EntityId};
use crate::engine::{EngineInner, LevelAssets};
use crate::fast_find_grid::SectorIndex;
use crate::position_interface::SectorHandle;

/// Chainable builder for [`PairScene`]. Defaults: 128x128 grid, a
/// 2000x2000 sector numbered 1 on layer 0, actors at (100,100) and
/// (200,100), frame counter 100, no mission script, no think frame.
pub(crate) struct PairFixture {
    first: Entity,
    second: Entity,
    grid: (u16, u16),
    extent: f32,
    positions: [WorldPoint3D; 2],
    frame_counter: u32,
    action_state: Option<ActionState>,
    move_box: Option<MoveBox>,
    direction: Option<i16>,
    active: Option<bool>,
    mission_script: Option<String>,
    think_first: bool,
}

/// Built scenario. `sector` carries its arena index.
pub(crate) struct PairScene {
    pub(crate) engine: EngineInner,
    pub(crate) assets: LevelAssets,
    pub(crate) first: EntityId,
    pub(crate) second: EntityId,
    pub(crate) sector: SectorHandle,
    pub(crate) sector_index: SectorIndex,
}

impl PairFixture {
    pub(crate) fn new(first: Entity, second: Entity) -> Self {
        Self {
            first,
            second,
            grid: (128, 128),
            extent: 2000.0,
            positions: [
                WorldPoint3D::new(100.0, 100.0, 0.0),
                WorldPoint3D::new(200.0, 100.0, 0.0),
            ],
            frame_counter: 100,
            action_state: None,
            move_box: None,
            direction: None,
            active: None,
            mission_script: None,
            think_first: false,
        }
    }

    /// Grid cell dimensions passed to `size_map`.
    pub(crate) fn grid(mut self, width: u16, height: u16) -> Self {
        self.grid = (width, height);
        self
    }

    /// Side length of the square sector, from the origin.
    pub(crate) fn extent(mut self, extent: f32) -> Self {
        self.extent = extent;
        self
    }

    pub(crate) fn positions(mut self, first: WorldPoint3D, second: WorldPoint3D) -> Self {
        self.positions = [first, second];
        self
    }

    /// Place both actors on the row `y`, at `first_x` and `second_x`.
    pub(crate) fn on_row(self, first_x: f32, second_x: f32, y: f32) -> Self {
        self.positions(
            WorldPoint3D::new(first_x, y, 0.0),
            WorldPoint3D::new(second_x, y, 0.0),
        )
    }

    pub(crate) fn frame_counter(mut self, frame: u32) -> Self {
        self.frame_counter = frame;
        self
    }

    /// Action state assigned to both actors.
    pub(crate) fn action_state(mut self, state: ActionState) -> Self {
        self.action_state = Some(state);
        self
    }

    /// Move box assigned to both actors.
    pub(crate) fn move_box(mut self, move_box: MoveBox) -> Self {
        self.move_box = Some(move_box);
        self
    }

    /// Facing assigned instantly to both actors.
    pub(crate) fn direction(mut self, direction: i16) -> Self {
        self.direction = Some(direction);
        self
    }

    /// `active` flag assigned to both actors.
    pub(crate) fn active(mut self, active: bool) -> Self {
        self.active = Some(active);
        self
    }

    /// Install `empty_mission_script(source_file)`.
    pub(crate) fn mission_script(mut self, source_file: &str) -> Self {
        self.mission_script = Some(source_file.to_owned());
        self
    }

    /// Call `enter_ai_think_frame` for the first actor once built.
    pub(crate) fn think_first(mut self) -> Self {
        self.think_first = true;
        self
    }

    pub(crate) fn build(self) -> PairScene {
        let mut engine = EngineInner::new();
        engine
            .world
            .fast_grid_mut()
            .size_map(self.grid.0, self.grid.1);
        engine.world.fast_grid_mut().allocate_layers(1);
        let index = engine.world.fast_grid_mut().add_sector(
            super::square_sector(
                1,
                0,
                MapPoint::new(0.0, 0.0),
                MapPoint::new(self.extent, self.extent),
            ),
            0,
        );
        let sector_index = SectorIndex::new(index).expect("fixture sector index");
        let sector = SectorHandle::new(1)
            .expect("fixture sector handle")
            .with_arena_index(sector_index);

        let first = engine.add_test_entity(self.first);
        let second = engine.add_test_entity(self.second);
        for (id, position) in [(first, self.positions[0]), (second, self.positions[1])] {
            let entity = engine.ent_mut(id);
            entity.element_data_mut().set_position(position);
            entity
                .element_data_mut()
                .set_sector_topology(Some(sector), Some(sector_index));
            if let Some(direction) = self.direction {
                entity.element_data_mut().set_direction_instantly(direction);
            }
            if let Some(active) = self.active {
                entity.element_data_mut().active = active;
            }
            if let Some(state) = self.action_state {
                entity
                    .actor_data_mut()
                    .expect("pair fixture action_state needs actors")
                    .action_state = state;
            }
            if let Some(move_box) = self.move_box {
                entity.position_iface_mut().set_move_box(move_box);
            }
        }

        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        // Runtime completion may rebind positions; the scenario's topology wins.
        for id in [first, second] {
            engine
                .elem_mut(id)
                .set_sector_topology(Some(sector), Some(sector_index));
        }
        if let Some(source_file) = &self.mission_script {
            engine.scripts.mission = Some(super::asm::empty_mission_script(source_file));
        }
        engine.control.frame_counter = self.frame_counter;
        if self.think_first {
            engine.enter_ai_think_frame(first);
        }
        PairScene {
            engine,
            assets,
            first,
            second,
            sector,
            sector_index,
        }
    }
}

impl PairScene {
    /// The `(engine, assets, first, second)` shape most `fn fixture()`s return.
    pub(crate) fn into_tuple(self) -> (EngineInner, LevelAssets, EntityId, EntityId) {
        (self.engine, self.assets, self.first, self.second)
    }
}
