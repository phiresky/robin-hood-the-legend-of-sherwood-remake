use std::fs;
use std::path::Path;
use syn::visit::{self, Visit};

fn references_geometry(source: &str, name: &str) -> bool {
    struct References<'a> {
        name: &'a str,
        found: bool,
    }
    impl<'ast> Visit<'ast> for References<'_> {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            self.found |= path
                .segments
                .iter()
                .any(|segment| segment.ident == self.name);
            visit::visit_path(self, path);
        }

        fn visit_use_tree(&mut self, tree: &'ast syn::UseTree) {
            self.found |= match tree {
                syn::UseTree::Path(path) => path.ident == self.name,
                syn::UseTree::Name(name) => name.ident == self.name,
                syn::UseTree::Rename(rename) => rename.ident == self.name,
                _ => false,
            };
            visit::visit_use_tree(self, tree);
        }
    }
    let syntax = syn::parse_file(source).expect("geometry guard source must parse");
    let mut references = References { name, found: false };
    references.visit_file(&syntax);
    references.found
}

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
            !references_geometry(&src, "BBox2D"),
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
        "src/engine/tick/mission.rs",
        "src/engine/tick/paths.rs",
        "src/engine/tick/frame_systems.rs",
        "src/ai_enemy/battle.rs",
        "src/path.rs",
        "src/material_sectors.rs",
    ] {
        let src = read_src(path);
        assert!(
            !references_geometry(&src, "geo2d"),
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
        !references_geometry(&lib, "geo2d"),
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
        !references_geometry(&mouse_way, "geo2d"),
        "mouse_way should keep its public geometry in ScreenPoint/ScreenVec"
    );
}

#[test]
fn geometry_guard_checks_paths_not_comments_or_string_literals() {
    assert!(!references_geometry(
        "// BBox2D\nconst DOC: &str = \"geo2d::pt\";",
        "geo2d"
    ));
    assert!(!references_geometry(
        "/// BBox2D is intentionally forbidden.\nstruct MapBBox;",
        "BBox2D"
    ));
    assert!(references_geometry(
        "use crate::{geometry, geo2d::{pt as point}};",
        "geo2d"
    ));
    assert!(references_geometry(
        "struct State { bounds: crate::geo2d::BBox2D }",
        "BBox2D"
    ));
}
