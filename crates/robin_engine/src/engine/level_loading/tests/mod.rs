//! Level-loading test suites split out of `level_loading.rs`.
//!
//! The submodules were written as direct children of `level_loading` and
//! import its items through `use super::…`; the glob below re-exposes the
//! parent's items at this level so those imports keep resolving unchanged.
use super::*;

mod accessory_publication_tests;
mod all_sprite_ambiance_variant_tests;
mod animation_placement_tests;
mod legacy_grid_topology_tests;
mod lift_endpoint_tests;
mod mission_level_builder_tests;
mod mission_start_sprite_tests;
mod rng_order_tests;

#[test]
fn motion_map_bounds_preserve_binary_entry_points_and_authored_partial_cells() {
    for (legacy_navigation, expected_height, expected_border, expected_outside) in
        [(true, 40, 2303.0, 2383.0), (false, 41, 2317.0, 2397.0)]
    {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let mut staging = LevelLoadStaging::default();
        engine.set_level_size(1024.0, 2318.0);
        let motion = crate::level_data::RawMotionData {
            legacy_navigation,
            layers: vec![vec![], vec![]],
            graph_bytes: vec![],
        };
        engine.initialize_motion_from_level_data(&mut assets, &mut staging, &motion, &[]);
        let grid = &engine.world.fast_grid.level;
        assert_eq!(grid.grid_width, 16);
        assert_eq!(grid.grid_height, expected_height);
        let (border, outside) =
            crate::natives::compute_border_point_bbox(grid.map_bbox, (400.0, 2100.0), 0);
        assert_eq!(border, (400.0, expected_border));
        assert_eq!(outside, (400.0, expected_outside));
    }
}
