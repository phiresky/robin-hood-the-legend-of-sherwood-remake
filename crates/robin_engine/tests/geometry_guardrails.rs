use std::fs;
use std::path::Path;

// These tests intentionally guard the modules where generic geometry caused
// map/ground/screen coordinate mixups. Low-level geo2d use remains allowed in
// adapter, serialization, and computational geometry internals; see
// docs/COORDINATES.md for the policy.

fn read_src(relative_path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    fs::read_to_string(&path).unwrap_or_else(|err| panic!("failed to read {path:?}: {err}"))
}

#[test]
fn cleaned_map_geometry_modules_do_not_reintroduce_generic_bboxes() {
    for path in [
        "src/engine/level_loading.rs",
        "src/engine/movement.rs",
        "src/engine/jump.rs",
        "src/ai.rs",
        "src/fast_find_grid.rs",
        "src/pathfinder.rs",
        "src/position_interface.rs",
        "src/sound_source.rs",
    ] {
        let src = read_src(path);
        assert!(
            !src.contains("BBox2D"),
            "{path} should keep public and stored geometry in domain bboxes such as MapBBox"
        );
    }
}

#[test]
fn cleaned_vector_math_modules_do_not_reintroduce_raw_geo_points() {
    for path in [
        "src/engine/anti_collision.rs",
        "src/engine/camera.rs",
        "src/engine/display_state.rs",
        "src/engine/tick.rs",
        "src/ai_enemy/battle.rs",
        "src/path.rs",
        "src/material_sectors.rs",
    ] {
        let src = read_src(path);
        assert!(
            !src.contains("geo2d::pt") && !src.contains("use crate::geo2d"),
            "{path} should keep vector math in MapPoint/MapVec/ScreenPoint/ScreenVec"
        );
    }
}

#[test]
fn robin_rs_does_not_reexport_generic_geometry() {
    let lib = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../robin_rs/src/lib.rs")
            .canonicalize()
            .expect("robin_rs lib path should exist"),
    )
    .expect("failed to read robin_rs/src/lib.rs");
    assert!(
        !lib.contains("pub use robin_engine::geo2d"),
        "robin_rs should not re-export generic geo2d; import low-level adapters explicitly"
    );

    let mouse_way = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../robin_rs/src/mouse_way.rs")
            .canonicalize()
            .expect("mouse_way path should exist"),
    )
    .expect("failed to read robin_rs/src/mouse_way.rs");
    assert!(
        !mouse_way.contains("crate::geo2d") && !mouse_way.contains("geo2d::pt"),
        "mouse_way should keep its public geometry in ScreenPoint/ScreenVec"
    );
}
