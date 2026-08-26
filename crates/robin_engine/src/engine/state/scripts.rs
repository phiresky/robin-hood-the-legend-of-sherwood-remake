use std::sync::Arc;

use super::super::{LevelAssets, MissionScript};

/// Deterministic mission-VM state and globals owned by the script subsystem.
///
/// Native calls still borrow the world, AI, campaign, orders, and feedback
/// state they operate on. Keeping those capabilities outside this owner is
/// important: this type owns the script runtime, not a second engine model.
#[derive(
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct ScriptRuntime {
    pub(crate) globals: Vec<i32>,
    pub(crate) mission: Option<MissionScript>,

    /// Immutable bytecode and native bindings are deliberately omitted from
    /// snapshots. This marker prevents a decoded VM from reaching native
    /// dispatch before the live level has reattached both resources.
    #[serde(skip, default)]
    #[state_hash(skip)]
    #[bitcode(skip)]
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

    pub(crate) fn install_mission(&mut self, mission: MissionScript) {
        self.mission = Some(mission);
        self.native_attachments_ready = false;
    }

    /// Validate the decoded script identity and immutable program lookup
    /// without mutating the candidate runtime.
    pub(crate) fn preflight_level_assets(&self, assets: &LevelAssets) -> Result<(), String> {
        match (
            self.mission.as_ref(),
            assets.scripts.mission_name.as_deref(),
        ) {
            (None, None) => return Ok(()),
            (None, Some(expected)) => {
                return Err(format!(
                    "decoded snapshot omits loaded mission script '{expected}'"
                ));
            }
            (Some(script), None) => {
                return Err(format!(
                    "decoded snapshot contains mission script '{}' but the loaded level has none",
                    script.script_name
                ));
            }
            (Some(script), Some(expected)) if script.script_name != expected => {
                return Err(format!(
                    "decoded snapshot mission script '{}' does not match loaded mission script '{expected}'",
                    script.script_name
                ));
            }
            (Some(script), Some(_)) => {
                assets
                    .scripts
                    .mission_programs
                    .get(&script.script_name)
                    .ok_or_else(|| {
                        format!(
                            "missing mission script program '{}' while attaching level assets",
                            script.script_name
                        )
                    })?;
            }
        }
        Ok(())
    }

    /// Attach the preflighted immutable program and native bindings.
    pub(crate) fn attach_preflighted_level_assets(&mut self, assets: &LevelAssets) {
        let Some(script) = self.mission.as_mut() else {
            self.native_attachments_ready = false;
            return;
        };

        if !script.script_name.is_empty() {
            let program = assets
                .scripts
                .mission_programs
                .get(&script.script_name)
                .expect("mission script program was preflighted");
            script.attach_program(Arc::clone(program));
        }

        Self::attach_native_bindings(script, assets);
        self.native_attachments_ready = true;
    }

    /// Attach native capabilities when the bytecode was installed directly
    /// from the already-loaded level (the normal new-mission path).
    pub(crate) fn attach_native_capabilities(&mut self, assets: &LevelAssets) {
        let Some(script) = self.mission.as_mut() else {
            self.native_attachments_ready = false;
            return;
        };
        Self::attach_native_bindings(script, assets);
        self.native_attachments_ready = true;
    }

    fn attach_native_bindings(script: &mut MissionScript, assets: &LevelAssets) {
        script.attach_bindings(crate::natives::AttachedScriptBindings {
            profile_manager: assets.profile_manager.clone(),
            hiking_paths: assets.hiking_paths.clone(),
            script_location_count: assets.scripts.location_count,
            script_point_count: assets.scripts.point_count,
            script_building_count: assets.scripts.building_count,
            script_hiking_path_count: assets.scripts.hiking_path_count,
            location_positions: assets.scripts.location_positions.clone(),
            location_layers: assets.scripts.location_layers.clone(),
            location_sectors: assets.scripts.location_sectors.clone(),
            location_sector_handles: assets.scripts.location_sector_handles.clone(),
            script_zone_grid_indices: assets.scripts.zone_grid_indices.clone(),
            patch_animation_entities: assets.entities.patch_animation_entities.clone(),
            lua_names: assets.scripts.names.clone(),
        });
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
