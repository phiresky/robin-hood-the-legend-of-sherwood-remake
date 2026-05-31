use std::fs;
use std::path::Path;

fn read_src(relative_path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    fs::read_to_string(&path).unwrap_or_else(|err| panic!("failed to read {path:?}: {err}"))
}

#[test]
fn cleaned_map_geometry_modules_do_not_reintroduce_generic_bboxes() {
    for path in [
        "src/engine/level_loading.rs",
        "src/engine/movement.rs",
        "src/fast_find_grid.rs",
        "src/pathfinder.rs",
        "src/position_interface.rs",
        "src/sound_source.rs",
    ] {
        let src = read_src(path);
        assert!(
            !src.contains("BBox2D"),
            "{path} should use domain coordinates such as MapBBox, not generic geo2d bboxes"
        );
    }
}

#[test]
fn cleaned_vector_math_modules_do_not_reintroduce_raw_geo_points() {
    for path in [
        "src/engine/anti_collision.rs",
        "src/engine/camera.rs",
        "src/engine/display_state.rs",
        "src/material_sectors.rs",
    ] {
        let src = read_src(path);
        assert!(
            !src.contains("geo2d::pt") && !src.contains("use crate::geo2d"),
            "{path} should use MapPoint/MapVec/ScreenPoint/ScreenVec for domain vector math"
        );
    }
}
