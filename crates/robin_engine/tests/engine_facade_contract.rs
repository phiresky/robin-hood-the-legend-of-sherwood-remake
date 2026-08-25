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
use syn::{Fields, Item, ItemImpl, ReturnType, Type, Visibility};

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
fn engine_owns_one_private_engine_inner_and_has_no_mutable_projection() {
    let syntax = parse_rust("src/engine/rollback_safe.rs");
    let engine = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "Engine" => Some(item),
            _ => None,
        })
        .expect("rollback-safe facade must define Engine");

    let Fields::Named(fields) = &engine.fields else {
        panic!("Engine must remain a named-field facade");
    };
    assert_eq!(
        fields.named.len(),
        1,
        "Engine must own exactly one field so no second mutable state channel bypasses the facade"
    );
    let inner = fields.named.first().expect("Engine has one field");
    assert_eq!(inner.ident.as_ref().expect("named field"), "inner");
    assert!(
        matches!(inner.vis, Visibility::Inherited),
        "Engine.inner must remain private"
    );
    assert!(
        path_ends_with(&inner.ty, "EngineInner"),
        "Engine.inner must remain the sole EngineInner owner"
    );

    let forbidden_mutable_projection_traits = ["DerefMut", "AsMut", "BorrowMut", "IndexMut"];
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut source_files = Vec::new();
    collect_rust_files(&manifest.join("src"), &mut source_files);
    let mut offenders = Vec::new();
    let mut mutable_return_visitor = MutableEngineInnerReturnVisitor::default();
    for path in source_files {
        let syntax = parse_rust_path(&path);
        for item in &syntax.items {
            let Item::Impl(item_impl) = item else {
                continue;
            };
            if !impl_is_for_engine(item_impl) {
                continue;
            }
            if let Some((_, trait_path, _)) = &item_impl.trait_
                && let Some(segment) = trait_path.segments.last()
                && forbidden_mutable_projection_traits
                    .iter()
                    .any(|forbidden| segment.ident == forbidden)
            {
                offenders.push(format!("{} in {}", segment.ident, path.display()));
            }
            mutable_return_visitor.visit_item_impl(item_impl);
        }
    }
    assert!(
        offenders.is_empty(),
        "Engine must not implement traits that project mutable access to EngineInner: {offenders:?}"
    );

    assert!(
        mutable_return_visitor.methods.is_empty(),
        "public Engine methods must not return mutable EngineInner references: {:?}",
        mutable_return_visitor.methods
    );
}

fn impl_is_for_engine(item: &ItemImpl) -> bool {
    path_ends_with(&item.self_ty, "Engine")
}

#[derive(Default)]
struct MutableEngineInnerReturnVisitor {
    current_public_method: Option<String>,
    inside_mutable_reference: bool,
    methods: Vec<String>,
}

impl<'ast> Visit<'ast> for MutableEngineInnerReturnVisitor {
    fn visit_impl_item_fn(&mut self, method: &'ast syn::ImplItemFn) {
        if !matches!(method.vis, Visibility::Public(_)) {
            return;
        }
        let ReturnType::Type(_, return_type) = &method.sig.output else {
            return;
        };
        self.current_public_method = Some(method.sig.ident.to_string());
        self.visit_type(return_type);
        self.current_public_method = None;
    }

    fn visit_type_reference(&mut self, reference: &'ast syn::TypeReference) {
        let was_inside = self.inside_mutable_reference;
        self.inside_mutable_reference |= reference.mutability.is_some();
        visit::visit_type_reference(self, reference);
        self.inside_mutable_reference = was_inside;
    }

    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
        if self.inside_mutable_reference
            && path
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "EngineInner")
            && let Some(method) = &self.current_public_method
        {
            self.methods.push(method.clone());
        }
        visit::visit_type_path(self, path);
    }
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
}

#[test]
fn host_crate_uses_engine_facade_instead_of_engine_inner() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let host_source = manifest.join("../robin_rs/src");
    let mut files = Vec::new();
    collect_rust_files(&host_source, &mut files);
    assert!(!files.is_empty(), "found no host-crate Rust sources");

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
