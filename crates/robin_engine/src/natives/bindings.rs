use std::collections::BTreeMap;
use std::sync::Arc;

use crate::coordinates::MapBBox;
use crate::fast_find_grid::LevelGrid;
use crate::level_data::RawHikingPath;
use crate::profiles::ProfileManager;
use crate::sight_obstacle::SharedSightObstacles;

/// Immutable level data attached to a mission script after construction or
/// snapshot decode. This is deliberately absent from script snapshots: the
/// canonical allocations live in [`crate::engine::LevelAssets`] and are
/// reattached by the engine that owns those assets.
#[derive(Clone, Default)]
pub struct AttachedScriptBindings {
    pub profile_manager: Arc<ProfileManager>,
    pub hiking_paths: Arc<Vec<RawHikingPath>>,
    pub level_grid: Arc<LevelGrid>,
    pub sight_obstacles: SharedSightObstacles,
    pub script_location_count: usize,
    pub script_point_count: usize,
    pub script_building_count: usize,
    pub script_hiking_path_count: usize,
    pub location_positions: Arc<Vec<(f32, f32)>>,
    pub location_layers: Arc<Vec<u16>>,
    pub location_sectors: Arc<Vec<u16>>,
    pub script_zone_grid_indices: Arc<Vec<u32>>,
    pub patch_animation_entities: Arc<Vec<Option<i32>>>,
    pub lua_names: Arc<ScriptNameBindings>,
}

impl AttachedScriptBindings {
    pub fn empty_ref() -> &'static Self {
        static EMPTY: std::sync::OnceLock<AttachedScriptBindings> = std::sync::OnceLock::new();
        EMPTY.get_or_init(AttachedScriptBindings::default)
    }

    pub fn view(&self) -> ScriptBindings<'_> {
        ScriptBindings { attached: self }
    }
}

/// Spellforge Lua name tables. Vanilla missions leave these empty.
#[derive(Clone, Debug, Default)]
pub struct ScriptNameBindings {
    pub actors: BTreeMap<String, i32>,
    pub items: BTreeMap<String, i32>,
    pub locations: BTreeMap<String, i32>,
    pub patrols: BTreeMap<String, i32>,
    pub scrolls: BTreeMap<String, i32>,
}

/// Short-lived borrowed view installed on one native dispatcher.
#[derive(Clone, Copy)]
pub struct ScriptBindings<'a> {
    attached: &'a AttachedScriptBindings,
}

impl<'a> ScriptBindings<'a> {
    pub fn empty() -> Self {
        Self {
            attached: AttachedScriptBindings::empty_ref(),
        }
    }

    pub fn map_bbox(self) -> MapBBox {
        self.attached.level_grid.map_bbox
    }
}

impl std::ops::Deref for ScriptBindings<'_> {
    type Target = AttachedScriptBindings;

    fn deref(&self) -> &Self::Target {
        self.attached
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::{HostFunctions, NativeStack};
    use crate::natives::{GameHost, NativeContext, NativeFn, ScriptState};

    fn get_location(bindings: &AttachedScriptBindings, index: i32) -> i32 {
        let mut host = GameHost::new();
        let mut state = ScriptState::default();
        let mut script_domains = crate::engine::ScriptDomains::default();
        let mut stack = NativeStack::default();
        stack.push_i32(index);
        let mut context = NativeContext::with_bindings(
            &mut host,
            &mut state,
            &mut script_domains,
            bindings,
            crate::natives::NativeQueryViews::default(),
        );
        HostFunctions::call(&mut context, NativeFn::GetLocationScript as u32, &mut stack)
            .expect_return("GetLocationScript is synchronous")
    }

    #[test]
    fn dispatch_bindings_are_isolated_between_engine_instances() {
        let first = AttachedScriptBindings {
            script_location_count: 1,
            sight_obstacles: SharedSightObstacles {
                static_active: Arc::new(vec![true]),
                ..Default::default()
            },
            ..Default::default()
        };
        let second = AttachedScriptBindings {
            script_location_count: 2,
            sight_obstacles: SharedSightObstacles {
                static_active: Arc::new(vec![true, false]),
                ..Default::default()
            },
            ..Default::default()
        };

        assert_ne!(get_location(&second, 1), 0);
        assert_eq!(get_location(&first, 1), 0);
        assert_eq!(first.view().sight_obstacles.static_active.len(), 1);
        assert_eq!(second.view().sight_obstacles.static_active.len(), 2);
    }
}
