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
use syn::{Fields, Item, ItemImpl, ReturnType, Type, UseTree, Visibility};

fn parse_rust(relative_path: &str) -> syn::File {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    parse_rust_path(&path)
}

fn parse_rust_path(path: &Path) -> syn::File {
    let source = fs::read_to_string(path)
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
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut source_files = Vec::new();
    collect_rust_files(&manifest.join("src"), &mut source_files);

    let mut mutable_engine_methods = Vec::new();
    let mut mutable_inner_methods = Vec::new();
    for path in source_files {
        let syntax = parse_rust_path(&path);
        let mut visitor = PublicMutableMethodVisitor {
            path: &path,
            mutable_engine_methods: &mut mutable_engine_methods,
            mutable_inner_methods: &mut mutable_inner_methods,
        };
        visitor.visit_file(&syntax);
    }
    mutable_engine_methods.sort();
    mutable_inner_methods.sort();

    let mut allowed = vec![
        "advance_frame",
        // Mutates only the non-authoritative presentation clone used for
        // render interpolation; it cannot open the live simulation owner.
        "apply_spatial_presentation",
        "host_console",
        "mission_setup",
        "parity_replay_setup",
        // Host-side, exactly-once policy attestation for the terminal attempt.
        // Calculation itself remains deterministic engine state; this opener
        // attaches eligibility before profile-history promotion.
        "promote_mission_achievement_results",
        "test_add_entity",
        "test_set_camera_transition_inputs",
        "test_set_engine_scalars",
        "test_set_frame_counter",
        "test_set_mission_flags",
        "test_set_mission_stat",
    ];
    allowed.sort();

    assert_eq!(
        mutable_engine_methods, allowed,
        "new public &mut Engine methods must be crate-private, represented as frame input, or deliberately added as a capability opener/test helper"
    );
    assert!(
        mutable_inner_methods.is_empty(),
        "EngineInner is a public read-only projection; public &mut EngineInner methods bypass the authoritative Engine facade: {mutable_inner_methods:?}"
    );
}

#[test]
fn engine_inner_is_a_borrow_only_projection() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut source_files = Vec::new();
    collect_rust_files(&manifest.join("src"), &mut source_files);

    let mut offenders = Vec::new();
    for path in source_files {
        let syntax = parse_rust_path(&path);
        let mut visitor = EngineInnerOwnershipVisitor {
            path: &path,
            offenders: &mut offenders,
        };
        visitor.visit_file(&syntax);
        let mut capability_visitor = PublicEngineInnerCapabilityVisitor {
            path: &path,
            offenders: &mut offenders,
        };
        capability_visitor.visit_file(&syntax);
    }

    assert!(
        offenders.is_empty(),
        "EngineInner may only be borrowed through Engine; production clone/default/codec implementations and public owned-value constructors are forbidden: {offenders:?}"
    );
}

struct PublicEngineInnerCapabilityVisitor<'a> {
    path: &'a Path,
    offenders: &'a mut Vec<String>,
}

impl Visit<'_> for PublicEngineInnerCapabilityVisitor<'_> {
    fn visit_item_fn(&mut self, function: &syn::ItemFn) {
        if matches!(function.vis, Visibility::Public(_)) {
            self.check_signature(&function.sig, format!("function {}", function.sig.ident));
        }
        visit::visit_item_fn(self, function);
    }

    fn visit_item_impl(&mut self, item_impl: &ItemImpl) {
        let owner = type_name(&item_impl.self_ty);
        for item in &item_impl.items {
            let syn::ImplItem::Fn(method) = item else {
                continue;
            };
            if matches!(method.vis, Visibility::Public(_)) {
                self.check_signature(&method.sig, format!("{owner}::{}", method.sig.ident));
            }
        }
        visit::visit_item_impl(self, item_impl);
    }
}

impl PublicEngineInnerCapabilityVisitor<'_> {
    fn check_signature(&mut self, signature: &syn::Signature, label: String) {
        if signature
            .inputs
            .iter()
            .any(argument_exposes_mut_engine_inner)
        {
            self.offenders.push(format!(
                "public {label} accepts mutable EngineInner in {}",
                self.path.display()
            ));
        }
        if return_type_contains_owned_engine_inner(&signature.output) {
            self.offenders.push(format!(
                "public {label} returns owned EngineInner in {}",
                self.path.display()
            ));
        }
    }
}

fn type_name(ty: &Type) -> String {
    let Type::Path(path) = ty else {
        return "unknown type".to_owned();
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
        .unwrap_or_else(|| "unknown type".to_owned())
}

fn argument_exposes_mut_engine_inner(argument: &syn::FnArg) -> bool {
    let syn::FnArg::Typed(argument) = argument else {
        return false;
    };
    let mut visitor = MutableEngineInnerReferenceVisitor { found: false };
    visitor.visit_type(&argument.ty);
    visitor.found
}

struct MutableEngineInnerReferenceVisitor {
    found: bool,
}

impl<'ast> Visit<'ast> for MutableEngineInnerReferenceVisitor {
    fn visit_type_reference(&mut self, reference: &'ast syn::TypeReference) {
        if reference.mutability.is_some() && type_mentions_ident(&reference.elem, "EngineInner") {
            self.found = true;
        }
        visit::visit_type_reference(self, reference);
    }
}

fn return_type_contains_owned_engine_inner(output: &ReturnType) -> bool {
    let ReturnType::Type(_, ty) = output else {
        return false;
    };
    let mut visitor = OwnedEngineInnerVisitor { found: false };
    visitor.visit_type(ty);
    visitor.found
}

struct OwnedEngineInnerVisitor {
    found: bool,
}

impl<'ast> Visit<'ast> for OwnedEngineInnerVisitor {
    fn visit_type_reference(&mut self, _reference: &'ast syn::TypeReference) {
        // Borrowed read-only projections are the supported public query API.
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.found |= path
            .segments
            .iter()
            .any(|segment| segment.ident == "EngineInner");
        visit::visit_path(self, path);
    }
}

struct PublicMutableMethodVisitor<'a> {
    path: &'a Path,
    mutable_engine_methods: &'a mut Vec<String>,
    mutable_inner_methods: &'a mut Vec<String>,
}

impl Visit<'_> for PublicMutableMethodVisitor<'_> {
    fn visit_item_impl(&mut self, item_impl: &ItemImpl) {
        let target = if impl_is_for_engine(item_impl) {
            Some(&mut self.mutable_engine_methods)
        } else if path_ends_with(&item_impl.self_ty, "EngineInner") {
            Some(&mut self.mutable_inner_methods)
        } else {
            None
        };
        if let Some(methods) = target {
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
                    let name = method.sig.ident.to_string();
                    if impl_is_for_engine(item_impl) {
                        methods.push(name);
                    } else {
                        methods.push(format!("{name} in {}", self.path.display()));
                    }
                }
            }
        }
        visit::visit_item_impl(self, item_impl);
    }
}

struct EngineInnerOwnershipVisitor<'a> {
    path: &'a Path,
    offenders: &'a mut Vec<String>,
}

impl Visit<'_> for EngineInnerOwnershipVisitor<'_> {
    fn visit_item_struct(&mut self, item_struct: &syn::ItemStruct) {
        if item_struct.ident == "EngineInner" {
            for forbidden in ["Clone", "Default", "Deserialize", "Encode", "Decode"] {
                if derives_trait(&item_struct.attrs, forbidden) {
                    self.offenders.push(format!(
                        "EngineInner derives {forbidden} in {}",
                        self.path.display()
                    ));
                }
            }
        }
        visit::visit_item_struct(self, item_struct);
    }

    fn visit_item_impl(&mut self, item_impl: &ItemImpl) {
        if !path_ends_with(&item_impl.self_ty, "EngineInner") {
            visit::visit_item_impl(self, item_impl);
            return;
        }

        if let Some((_, trait_path, _)) = &item_impl.trait_
            && let Some(trait_name) = trait_path.segments.last()
            && ["Clone", "Default", "Deserialize", "Encode", "Decode"]
                .iter()
                .any(|forbidden| trait_name.ident == forbidden)
            && !is_test_only(&item_impl.attrs)
        {
            self.offenders.push(format!(
                "EngineInner implements {} in {}",
                trait_name.ident,
                self.path.display()
            ));
        } else if item_impl.trait_.is_none() {
            for item in &item_impl.items {
                let syn::ImplItem::Fn(method) = item else {
                    continue;
                };
                if matches!(method.vis, Visibility::Public(_))
                    && return_type_mentions_owned_engine_inner(&method.sig.output)
                {
                    self.offenders.push(format!(
                        "EngineInner::{} returns an owned EngineInner in {}",
                        method.sig.ident,
                        self.path.display()
                    ));
                }
            }
        }

        visit::visit_item_impl(self, item_impl);
    }
}

fn derives_trait(attrs: &[syn::Attribute], expected: &str) -> bool {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("derive"))
        .any(|attr| {
            attr.parse_args_with(
                syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
            )
            .is_ok_and(|traits| {
                traits.iter().any(|path| {
                    path.segments
                        .last()
                        .is_some_and(|item| item.ident == expected)
                })
            })
        })
}

fn is_test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let syn::Meta::List(meta) = &attr.meta else {
            return false;
        };
        meta.path.is_ident("cfg") && meta.tokens.to_string() == "test"
    })
}

fn return_type_mentions_owned_engine_inner(output: &ReturnType) -> bool {
    let ReturnType::Type(_, ty) = output else {
        return false;
    };
    type_mentions_ident(ty, "EngineInner") || type_mentions_ident(ty, "Self")
}

#[test]
fn engine_inner_guards_detect_mutation_and_ownership_escapes() {
    let syntax = syn::parse_file(
        r#"
        #[derive(Clone, bitcode::Encode, bitcode::Decode)]
        struct EngineInner;

        impl EngineInner {
            pub fn mutate(&mut self) {}
            pub fn copy_out(&self) -> Self { Self }
            pub fn query(&self) -> bool { true }
        }

        pub fn mutate_from_free_function(engine: &mut EngineInner) { let _ = engine; }
        pub fn construct_from_free_function() -> Option<EngineInner> { None }
        pub fn query_from_free_function(engine: &EngineInner) { let _ = engine; }
        "#,
    )
    .expect("synthetic Rust source parses");

    let mut mutable_engine_methods = Vec::new();
    let mut mutable_inner_methods = Vec::new();
    let mut mutable_visitor = PublicMutableMethodVisitor {
        path: Path::new("synthetic.rs"),
        mutable_engine_methods: &mut mutable_engine_methods,
        mutable_inner_methods: &mut mutable_inner_methods,
    };
    mutable_visitor.visit_file(&syntax);
    assert!(mutable_engine_methods.is_empty());
    assert_eq!(mutable_inner_methods.len(), 1);
    assert!(mutable_inner_methods[0].contains("mutate"));

    let mut ownership_offenders = Vec::new();
    let mut ownership_visitor = EngineInnerOwnershipVisitor {
        path: Path::new("synthetic.rs"),
        offenders: &mut ownership_offenders,
    };
    ownership_visitor.visit_file(&syntax);
    assert_eq!(ownership_offenders.len(), 4);
    assert!(
        ownership_offenders
            .iter()
            .any(|offender| offender.contains("derives Clone"))
    );
    assert!(
        ownership_offenders
            .iter()
            .any(|offender| offender.contains("derives Encode"))
    );
    assert!(
        ownership_offenders
            .iter()
            .any(|offender| offender.contains("derives Decode"))
    );
    assert!(
        ownership_offenders
            .iter()
            .any(|offender| offender.contains("copy_out"))
    );

    let mut capability_offenders = Vec::new();
    let mut capability_visitor = PublicEngineInnerCapabilityVisitor {
        path: Path::new("synthetic.rs"),
        offenders: &mut capability_offenders,
    };
    capability_visitor.visit_file(&syntax);
    assert_eq!(capability_offenders.len(), 2);
    assert!(
        capability_offenders
            .iter()
            .any(|offender| offender.contains("mutate_from_free_function"))
    );
    assert!(
        capability_offenders
            .iter()
            .any(|offender| offender.contains("construct_from_free_function"))
    );
}

#[test]
fn legacy_hourglass_adapters_are_test_only_and_use_explicit_input() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut source_files = Vec::new();
    collect_rust_files(&manifest.join("src"), &mut source_files);

    let mut adapters = Vec::new();
    let mut offenders = Vec::new();
    for path in source_files {
        let syntax = parse_rust_path(&path);
        let mut visitor = HourglassAdapterVisitor {
            path: &path,
            adapters: &mut adapters,
            offenders: &mut offenders,
        };
        visitor.visit_file(&syntax);
    }
    adapters.sort();

    assert_eq!(
        adapters,
        [
            "Engine::perform_hourglass",
            "EngineInner::perform_hourglass"
        ],
        "the legacy command/hourglass test seam must not grow new entry points"
    );
    assert!(
        offenders.is_empty(),
        "legacy hourglass adapters must be cfg(test) pub(crate), accept explicit mutable InputState, and never fabricate a default input: {offenders:?}"
    );
}

struct HourglassAdapterVisitor<'a> {
    path: &'a Path,
    adapters: &'a mut Vec<String>,
    offenders: &'a mut Vec<String>,
}

impl Visit<'_> for HourglassAdapterVisitor<'_> {
    fn visit_item_impl(&mut self, item_impl: &ItemImpl) {
        let owner = if impl_is_for_engine(item_impl) {
            Some("Engine")
        } else if path_ends_with(&item_impl.self_ty, "EngineInner") {
            Some("EngineInner")
        } else {
            None
        };
        let Some(owner) = owner else {
            visit::visit_item_impl(self, item_impl);
            return;
        };

        for item in &item_impl.items {
            let syn::ImplItem::Fn(method) = item else {
                continue;
            };
            if method.sig.ident != "perform_hourglass" {
                continue;
            }

            let adapter = format!("{owner}::perform_hourglass");
            self.adapters.push(adapter.clone());
            if !is_test_only(&method.attrs) {
                self.offenders.push(format!(
                    "{adapter} is production-reachable in {}",
                    self.path.display()
                ));
            }
            if !is_pub_crate(&method.vis) {
                self.offenders.push(format!(
                    "{adapter} is not pub(crate) in {}",
                    self.path.display()
                ));
            }
            if !has_explicit_mut_input_state(&method.sig) {
                self.offenders.push(format!(
                    "{adapter} has no &mut InputState parameter in {}",
                    self.path.display()
                ));
            }

            let mut defaults = InputStateDefaultVisitor { found: false };
            defaults.visit_block(&method.block);
            if defaults.found {
                self.offenders.push(format!(
                    "{adapter} fabricates InputState::default() in {}",
                    self.path.display()
                ));
            }
        }

        visit::visit_item_impl(self, item_impl);
    }
}

fn is_pub_crate(visibility: &Visibility) -> bool {
    matches!(
        visibility,
        Visibility::Restricted(restricted) if restricted.path.is_ident("crate")
    )
}

fn has_explicit_mut_input_state(signature: &syn::Signature) -> bool {
    signature.inputs.iter().any(|argument| {
        let syn::FnArg::Typed(argument) = argument else {
            return false;
        };
        matches!(
            argument.ty.as_ref(),
            Type::Reference(reference)
                if reference.mutability.is_some()
                    && type_mentions_ident(&reference.elem, "InputState")
        )
    })
}

struct InputStateDefaultVisitor {
    found: bool,
}

impl<'ast> Visit<'ast> for InputStateDefaultVisitor {
    fn visit_expr_call(&mut self, expression: &'ast syn::ExprCall) {
        if let syn::Expr::Path(function) = expression.func.as_ref()
            && function.path.segments.len() >= 2
            && function
                .path
                .segments
                .iter()
                .any(|segment| segment.ident == "InputState")
            && function
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "default")
        {
            self.found = true;
        }
        visit::visit_expr_call(self, expression);
    }
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
