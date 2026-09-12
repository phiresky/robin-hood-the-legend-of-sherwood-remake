use std::collections::BTreeSet;
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

/// Computational helpers may use generic geometry locally, but it must not
/// escape into an API or stored field, including through renamed imports and
/// private type aliases.
fn exposes_generic_geometry(source: &str) -> bool {
    struct Paths<'a> {
        forbidden: &'a BTreeSet<String>,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Paths<'_> {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            self.found |= path
                .segments
                .iter()
                .any(|part| self.forbidden.contains(&part.ident.to_string()));
            visit::visit_path(self, path);
        }
    }
    struct Aliases<'a> {
        forbidden: &'a BTreeSet<String>,
        discovered: BTreeSet<String>,
    }
    impl<'ast> Visit<'ast> for Aliases<'_> {
        fn visit_use_rename(&mut self, rename: &'ast syn::UseRename) {
            if self.forbidden.contains(&rename.ident.to_string()) {
                self.discovered.insert(rename.rename.to_string());
            }
        }
        fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
            let mut paths = Paths {
                forbidden: self.forbidden,
                found: false,
            };
            paths.visit_type(&item.ty);
            if paths.found {
                self.discovered.insert(item.ident.to_string());
            }
        }
    }
    struct Surface<'a>(Paths<'a>);
    impl<'ast> Visit<'ast> for Surface<'_> {
        fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
            if !matches!(item.vis, syn::Visibility::Inherited) {
                self.0.visit_signature(&item.sig);
            }
        }
        fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
            if !matches!(item.vis, syn::Visibility::Inherited) {
                self.0.visit_signature(&item.sig);
            }
        }
        fn visit_field(&mut self, field: &'ast syn::Field) {
            // Stored geometry remains domain typed even in private fields.
            self.0.visit_type(&field.ty);
        }
        fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
            if !matches!(item.vis, syn::Visibility::Inherited) {
                self.0.visit_type(&item.ty);
            }
        }
        fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
            if !matches!(item.vis, syn::Visibility::Inherited) {
                // A public generic import is itself an API leak.
                // Inspect use trees separately: they are not syn::Path nodes.
                fn forbidden(tree: &syn::UseTree, names: &BTreeSet<String>) -> bool {
                    match tree {
                        syn::UseTree::Path(path) => {
                            names.contains(&path.ident.to_string()) || forbidden(&path.tree, names)
                        }
                        syn::UseTree::Name(name) => names.contains(&name.ident.to_string()),
                        syn::UseTree::Rename(rename) => names.contains(&rename.ident.to_string()),
                        syn::UseTree::Group(group) => {
                            group.items.iter().any(|tree| forbidden(tree, names))
                        }
                        syn::UseTree::Glob(_) => false,
                    }
                }
                self.0.found |= forbidden(&item.tree, self.0.forbidden);
            }
        }
    }

    let syntax = syn::parse_file(source).expect("geometry guard source must parse");
    let mut forbidden = [
        "geo2d",
        "GeoPoint2D",
        "Vec2D",
        "BBox2D",
        "Segment2D",
        "PolyLine2D",
        "Polygon2D",
        "Line2D",
        "HalfLine2D",
        "Intersection2D",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    loop {
        let mut aliases = Aliases {
            forbidden: &forbidden,
            discovered: BTreeSet::new(),
        };
        aliases.visit_file(&syntax);
        let discovered = aliases.discovered;
        let previous = forbidden.len();
        forbidden.extend(discovered);
        if previous == forbidden.len() {
            break;
        }
    }
    let mut surface = Surface(Paths {
        forbidden: &forbidden,
        found: false,
    });
    surface.visit_file(&syntax);
    surface.0.found
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
        !exposes_generic_geometry(&mouse_way),
        "mouse_way should keep public and stored geometry in ScreenPoint/ScreenVec; generic segment helpers belong inside computations"
    );
}

#[test]
fn geometry_api_guard_allows_local_adapters_but_rejects_alias_leaks() {
    assert!(!exposes_generic_geometry(
        "use engine::geo2d::Segment2D; pub fn crosses(p: ScreenPoint) -> bool { let segment = Segment2D::new(p.to_geo(), p.to_geo()); false }"
    ));
    for source in [
        "pub fn point() -> engine::geo2d::GeoPoint2D { todo!() }",
        "struct State { point: GeoPoint2D }",
        "pub fn direction() -> Vec2D { todo!() }",
        "impl State { pub fn set(&mut self, segment: Segment2D) {} }",
        "use engine::geo2d::GeoPoint2D as Point; type Alias = Point; pub fn set(p: Alias) {}",
        "use engine::geo2d as geometry; pub fn set(p: geometry::Point) {}",
        "pub type Bounds = BBox2D;",
        "pub use engine::geo2d::GeoPoint2D as Point;",
    ] {
        assert!(
            exposes_generic_geometry(source),
            "missed generic API: {source}"
        );
    }
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
