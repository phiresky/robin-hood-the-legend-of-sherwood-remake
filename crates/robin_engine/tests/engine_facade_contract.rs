//! Architectural guardrails for the cross-crate simulation mutation boundary.
//!
//! `Engine` deliberately gives downstream crates read-only access to
//! `EngineInner`; all mutation has to pass through a named facade operation.
//! These source-structural checks make that property fail in CI if a future
//! refactor accidentally adds mutable dereferencing, exposes the wrapped
//! value, or starts using `EngineInner` directly from the host crate.

use std::fs;
use std::path::{Path, PathBuf};

use syn::visit::{self, Visit};
use syn::{Fields, Item, ItemImpl, Type, UseTree, Visibility};

fn parse_rust(relative_path: &str) -> syn::File {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    parse_rust_path(&path)
}

fn parse_rust_path(path: &Path) -> syn::File {
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    syn::parse_file(&source)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn collect_rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()));
    for entry in entries {
        let path = entry.expect("directory entry should be readable").path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    files.sort();
}

fn path_ends_with(ty: &Type, expected: &str) -> bool {
    matches!(
        ty,
        Type::Path(path)
            if path
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == expected)
    )
}

#[test]
fn engine_has_one_private_inner_owner_and_no_ownership_escape() {
    let syntax = parse_rust("src/engine/rollback_safe.rs");
    let engine = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "Engine" => Some(item),
            _ => None,
        })
        .expect("rollback-safe facade must define Engine");

    let fields: Vec<_> = match &engine.fields {
        Fields::Named(fields) => fields.named.iter().collect(),
        Fields::Unnamed(fields) => fields.unnamed.iter().collect(),
        Fields::Unit => Vec::new(),
    };
    assert!(
        fields
            .iter()
            .all(|field| matches!(field.vis, Visibility::Inherited)),
        "every Engine field must remain private"
    );
    let inner_fields: Vec<_> = fields
        .iter()
        .filter(|field| type_mentions_ident(&field.ty, "EngineInner"))
        .collect();
    assert_eq!(
        inner_fields.len(),
        1,
        "Engine must have exactly one private field that owns EngineInner"
    );

    let forbidden_mutable_projection_traits = ["DerefMut", "AsMut", "BorrowMut", "IndexMut"];
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut source_files = Vec::new();
    collect_rust_files(&manifest.join("src"), &mut source_files);
    let mut offenders = Vec::new();
    let mut api_exposures = Vec::new();
    for path in source_files {
        let syntax = parse_rust_path(&path);
        let mut visitor = EngineImplVisitor {
            path: &path,
            forbidden_traits: &forbidden_mutable_projection_traits,
            trait_offenders: &mut offenders,
            api_exposures: &mut api_exposures,
        };
        visitor.visit_file(&syntax);
    }
    assert!(
        offenders.is_empty(),
        "Engine must not implement traits that project mutable access to EngineInner: {offenders:?}"
    );

    assert!(
        api_exposures.is_empty(),
        "public Engine method signatures must not expose or accept EngineInner directly, by reference, or through a wrapper: {api_exposures:?}"
    );
}

#[test]
fn engine_public_mutation_surface_is_an_exact_capability_allowlist() {
    let syntax = parse_rust("src/engine/rollback_safe.rs");
    let mut mutable_methods = Vec::new();
    for item in &syntax.items {
        let Item::Impl(item_impl) = item else {
            continue;
        };
        if !impl_is_for_engine(item_impl) {
            continue;
        }
        for item in &item_impl.items {
            let syn::ImplItem::Fn(method) = item else {
                continue;
            };
            let Some(receiver) = method.sig.receiver() else {
                continue;
            };
            if matches!(method.vis, Visibility::Public(_))
                && receiver.reference.is_some()
                && receiver.mutability.is_some()
            {
                mutable_methods.push(method.sig.ident.to_string());
            }
        }
    }
    mutable_methods.sort();

    let mut allowed = vec![
        "advance_frame",
        "host_console",
        "mission_setup",
        "parity_replay_setup",
        "test_add_entity",
        "test_set_camera_transition_inputs",
        "test_set_engine_scalars",
        "test_set_frame_counter",
        "test_set_mission_flags",
        "test_set_mission_stat",
    ];
    allowed.sort();

    assert_eq!(
        mutable_methods, allowed,
        "new public &mut Engine methods must be crate-private, represented as frame input, or deliberately added as a capability opener/test helper"
    );
}

fn impl_is_for_engine(item: &ItemImpl) -> bool {
    path_ends_with(&item.self_ty, "Engine")
}

fn type_mentions_ident(ty: &Type, expected: &str) -> bool {
    let mut visitor = IdentUseVisitor {
        expected,
        found: false,
    };
    visitor.visit_type(ty);
    visitor.found
}

struct IdentUseVisitor<'a> {
    expected: &'a str,
    found: bool,
}

impl<'ast> Visit<'ast> for IdentUseVisitor<'_> {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.found |= path
            .segments
            .iter()
            .any(|segment| segment.ident == self.expected);
        visit::visit_path(self, path);
    }
}

struct EngineImplVisitor<'a> {
    path: &'a Path,
    forbidden_traits: &'a [&'a str],
    trait_offenders: &'a mut Vec<String>,
    api_exposures: &'a mut Vec<String>,
}

impl<'ast> Visit<'ast> for EngineImplVisitor<'_> {
    fn visit_item_impl(&mut self, item_impl: &'ast ItemImpl) {
        if impl_is_for_engine(item_impl) {
            if let Some((_, trait_path, _)) = &item_impl.trait_
                && let Some(segment) = trait_path.segments.last()
                && self
                    .forbidden_traits
                    .iter()
                    .any(|forbidden| segment.ident == forbidden)
            {
                self.trait_offenders
                    .push(format!("{} in {}", segment.ident, self.path.display()));
            }
            for item in &item_impl.items {
                let syn::ImplItem::Fn(method) = item else {
                    continue;
                };
                if !matches!(method.vis, Visibility::Public(_)) {
                    continue;
                }
                let mut signature_visitor = IdentUseVisitor {
                    expected: "EngineInner",
                    found: false,
                };
                signature_visitor.visit_signature(&method.sig);
                if signature_visitor.found {
                    self.api_exposures.push(format!(
                        "Engine::{} in {}",
                        method.sig.ident,
                        self.path.display()
                    ));
                }
            }
        }
        visit::visit_item_impl(self, item_impl);
    }
}

#[test]
fn facade_guard_finds_nested_trait_and_owned_wrapper_escapes() {
    let syntax = syn::parse_file(
        r#"
        mod nested {
            struct Engine;
            struct EngineInner;

            impl Engine {
                pub fn into_inner(self) -> Option<EngineInner> { unreachable!() }
                pub fn with_inner(&mut self, visit: impl FnOnce(&mut EngineInner)) {
                    let _ = visit;
                }
            }

            impl std::ops::DerefMut for Engine {
                fn deref_mut(&mut self) -> &mut Self::Target { unreachable!() }
            }
        }
        "#,
    )
    .expect("synthetic Rust source parses");
    let forbidden_traits = ["DerefMut"];
    let mut trait_offenders = Vec::new();
    let mut api_exposures = Vec::new();
    let mut visitor = EngineImplVisitor {
        path: Path::new("synthetic.rs"),
        forbidden_traits: &forbidden_traits,
        trait_offenders: &mut trait_offenders,
        api_exposures: &mut api_exposures,
    };
    visitor.visit_file(&syntax);

    assert_eq!(trait_offenders.len(), 1);
    assert_eq!(api_exposures.len(), 2);
    assert!(
        api_exposures
            .iter()
            .any(|exposure| exposure.contains("Engine::into_inner"))
    );
    assert!(
        api_exposures
            .iter()
            .any(|exposure| exposure.contains("Engine::with_inner"))
    );
}

#[derive(Default)]
struct EngineInnerUseVisitor {
    found: bool,
}

impl<'ast> Visit<'ast> for EngineInnerUseVisitor {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if path
            .segments
            .iter()
            .any(|segment| segment.ident == "EngineInner")
        {
            self.found = true;
        }
        visit::visit_path(self, path);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.found |= use_tree_mentions_ident(&item.tree, "EngineInner");
        visit::visit_item_use(self, item);
    }
}

fn use_tree_mentions_ident(tree: &UseTree, expected: &str) -> bool {
    match tree {
        UseTree::Path(path) => {
            path.ident == expected || use_tree_mentions_ident(&path.tree, expected)
        }
        UseTree::Name(name) => name.ident == expected,
        UseTree::Rename(rename) => rename.ident == expected,
        UseTree::Group(group) => group
            .items
            .iter()
            .any(|tree| use_tree_mentions_ident(tree, expected)),
        UseTree::Glob(_) => false,
    }
}

fn source_uses_engine_inner(source: &str) -> bool {
    let syntax = syn::parse_file(source).expect("synthetic Rust source parses");
    let mut visitor = EngineInnerUseVisitor::default();
    visitor.visit_file(&syntax);
    visitor.found
}

#[test]
fn host_guard_detects_direct_and_renamed_engine_inner_imports() {
    assert!(source_uses_engine_inner(
        "use robin_engine::engine::EngineInner;"
    ));
    assert!(source_uses_engine_inner(
        "use robin_engine::engine::EngineInner as MutableEngine;"
    ));
    assert!(source_uses_engine_inner(
        "use robin_engine::engine::{Engine, EngineInner as MutableEngine};"
    ));
    assert!(!source_uses_engine_inner(
        "use robin_engine::engine::{Engine, HostDisplayState};"
    ));
}

#[test]
fn host_crate_targets_use_engine_facade_instead_of_engine_inner() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for relative in [
        "../robin_rs/src",
        "../robin_rs/examples",
        "../robin_rs/tests",
    ] {
        let directory = manifest.join(relative);
        if directory.is_dir() {
            collect_rust_files(&directory, &mut files);
        }
    }
    assert!(!files.is_empty(), "found no host-crate Rust targets");

    let mut offenders = Vec::new();
    for path in files {
        let syntax = parse_rust_path(&path);
        let mut visitor = EngineInnerUseVisitor::default();
        visitor.visit_file(&syntax);
        if visitor.found {
            offenders.push(
                path.strip_prefix(manifest.join("../.."))
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
            );
        }
    }

    assert!(
        offenders.is_empty(),
        "host code must mutate simulation only through Engine; direct EngineInner uses found in {offenders:?}"
    );
}
