use std::sync::Arc;

use super::super::{LevelAssets, MissionScript};

/// Deterministic mission-VM state and globals owned by the script subsystem.
///
/// Native calls still borrow the world, AI, campaign, orders, and feedback
/// state they operate on. Keeping those capabilities outside this owner is
/// important: this type owns the script runtime, not a second engine model.
#[derive(Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct ScriptRuntime {
    pub(crate) globals: Vec<i32>,
    pub(crate) mission: Option<MissionScript>,

    /// Immutable bytecode and native bindings are deliberately omitted from
    /// snapshots. This marker prevents a decoded VM from reaching native
    /// dispatch before the live level has reattached both resources.
    #[serde(skip, default)]
    #[state_hash(skip)]
    native_attachments_ready: bool,
}

impl ScriptRuntime {
    pub(crate) fn new() -> Self {
        Self {
            globals: Vec::new(),
            mission: None,
            native_attachments_ready: false,
        }
    }

    pub(crate) fn from_snapshot(globals: Vec<i32>, mission: Option<MissionScript>) -> Self {
        Self {
            globals,
            mission,
            native_attachments_ready: false,
        }
    }

    pub(crate) fn install_mission(&mut self, mission: MissionScript) {
        self.mission = Some(mission);
        self.native_attachments_ready = false;
    }

    /// Attach the immutable script program and native capabilities belonging
    /// to the currently loaded level. World arrays are copied into the
    /// pre-existing shared binding adapter; they do not become script-owned
    /// authoritative state.
    pub(crate) fn attach_level_assets(
        &mut self,
        assets: &LevelAssets,
        dynamic_sight_obstacles: &[crate::sight_obstacle::SightObstacle],
        static_sight_obstacle_active: &[bool],
    ) {
        let Some(script) = self.mission.as_mut() else {
            self.native_attachments_ready = false;
            return;
        };

        if !script.script_name.is_empty() {
            let program = assets
                .mission_script_programs
                .get(&script.script_name)
                .unwrap_or_else(|| {
                    panic!(
                        "missing mission script program '{}' while attaching level assets",
                        script.script_name
                    )
                });
            script.attach_program(Arc::clone(program));
        }

        Self::attach_native_bindings(
            script,
            assets,
            dynamic_sight_obstacles,
            static_sight_obstacle_active,
        );
        self.native_attachments_ready = true;
    }

    /// Attach native capabilities when the bytecode was installed directly
    /// from the already-loaded level (the normal new-mission path).
    pub(crate) fn attach_native_capabilities(
        &mut self,
        assets: &LevelAssets,
        dynamic_sight_obstacles: &[crate::sight_obstacle::SightObstacle],
        static_sight_obstacle_active: &[bool],
    ) {
        let Some(script) = self.mission.as_mut() else {
            self.native_attachments_ready = false;
            return;
        };
        Self::attach_native_bindings(
            script,
            assets,
            dynamic_sight_obstacles,
            static_sight_obstacle_active,
        );
        self.native_attachments_ready = true;
    }

    fn attach_native_bindings(
        script: &mut MissionScript,
        assets: &LevelAssets,
        dynamic_sight_obstacles: &[crate::sight_obstacle::SightObstacle],
        static_sight_obstacle_active: &[bool],
    ) {
        script.attach_bindings(crate::natives::AttachedScriptBindings {
            profile_manager: assets.profile_manager.clone(),
            hiking_paths: assets.hiking_paths.clone(),
            sight_obstacles: crate::sight_obstacle::SharedSightObstacles {
                static_obstacles: assets.static_sight_obstacles.clone(),
                dynamic_obstacles: Arc::new(dynamic_sight_obstacles.to_vec()),
                static_active: Arc::new(static_sight_obstacle_active.to_vec()),
            },
            script_location_count: assets.script_location_count,
            script_point_count: assets.script_point_count,
            script_building_count: assets.script_building_count,
            script_hiking_path_count: assets.script_hiking_path_count,
            location_positions: assets.script_location_positions.clone(),
            location_layers: assets.script_location_layers.clone(),
            location_sectors: assets.script_location_sectors.clone(),
            script_zone_grid_indices: assets.script_zone_grid_indices.clone(),
            patch_animation_entities: assets.patch_entity_handles.clone(),
            lua_names: assets.script_names.clone(),
        });
    }

    pub(crate) fn refresh_sight_bindings(
        &mut self,
        dynamic_sight_obstacles: &[crate::sight_obstacle::SightObstacle],
        static_sight_obstacle_active: &[bool],
    ) {
        if let Some(script) = self.mission.as_mut() {
            script.bindings.sight_obstacles.dynamic_obstacles =
                Arc::new(dynamic_sight_obstacles.to_vec());
            script.bindings.sight_obstacles.static_active =
                Arc::new(static_sight_obstacle_active.to_vec());
        }
    }

    /// Reattach host-owned resources after replacing the live engine with a
    /// decoded snapshot. The saved mutable VM remains authoritative; only
    /// immutable resources come from the already-loaded engine.
    pub(crate) fn reattach_from(&mut self, previous: &Self) {
        match (self.mission.as_mut(), previous.mission.as_ref()) {
            (Some(restored), Some(live)) => {
                restored
                    .manager
                    .attach_program(live.manager.program.clone());
                restored.bindings = live.bindings.clone();
                self.native_attachments_ready = previous.native_attachments_ready;
            }
            (None, None) => self.native_attachments_ready = false,
            (Some(_), None) => {
                panic!("restored mission script has no live script attachment source")
            }
            (None, Some(_)) => {
                panic!("restored snapshot unexpectedly omits the live mission script")
            }
        }
    }

    /// Fail before native dispatch rather than running a live VM against the
    /// placeholder resources intentionally installed by deserialization.
    pub(crate) fn assert_native_attachments_ready(&self) {
        let Some(script) = self.mission.as_ref() else {
            return;
        };
        assert!(
            self.native_attachments_ready,
            "mission script native dispatch requires live level attachments"
        );
        assert!(
            !script.manager.program.scb.classes.is_empty(),
            "mission script native dispatch requires attached bytecode"
        );
    }
}
