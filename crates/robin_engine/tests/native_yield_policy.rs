//! Static, exhaustive cross-check of the native registry's yield column.
//!
//! `NativeFn::may_yield()` feeds the Lua adapter's preflight
//! (`NativeContext::requires_engine_driver`). A native whose dispatch can set
//! `pending_yield` but is registered `Never` would mutate the recorder, entities
//! or sequence manager before the adapter notices the yield; one registered
//! `Always`/`Conditional` that cannot yield is mis-documented and forces a
//! needless engine-driver round trip. The runtime assertion in
//! `HostFunctions::call` only catches mis-marked natives that some test happens
//! to execute, so this test derives "can yield" from the dispatch source.

use robin_engine::natives::NATIVE_REGISTRY;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use syn::visit::Visit;

fn is_test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("test")
            || (attr.path().is_ident("cfg")
                && matches!(&attr.meta, syn::Meta::List(list) if list.tokens.to_string().contains("test")))
    })
}

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_owned()];
    let mut result = Vec::new();
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        {
            let path = entry.expect("read source entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                result.push(path);
            }
        }
    }
    result.sort();
    result
}

fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "tests.rs" || name.ends_with("_tests.rs"))
        || path
            .components()
            .any(|component| component.as_os_str() == "tests")
}

fn parse(path: &Path) -> syn::File {
    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    syn::parse_file(&source).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn is_self(expr: &syn::Expr) -> bool {
    matches!(expr, syn::Expr::Path(path) if path.path.is_ident("self"))
}

fn is_some_call(expr: &syn::Expr) -> bool {
    matches!(
        expr,
        syn::Expr::Call(call)
            if matches!(call.func.as_ref(), syn::Expr::Path(path) if path.path.is_ident("Some"))
    )
}

fn last_ident(path: &syn::Path) -> Option<String> {
    path.segments
        .last()
        .map(|segment| segment.ident.to_string())
}

/// Yield evidence in one body: a direct `self.pending_yield = Some(..)` write,
/// plus every `self.method(..)` it calls (resolved transitively afterwards).
#[derive(Default)]
struct YieldEvidence {
    writes_pending_yield: bool,
    self_calls: BTreeSet<String>,
}

impl YieldEvidence {
    fn of(visit: impl FnOnce(&mut Self)) -> Self {
        let mut evidence = Self::default();
        visit(&mut evidence);
        evidence
    }

    fn merge(&mut self, other: Self) {
        self.writes_pending_yield |= other.writes_pending_yield;
        self.self_calls.extend(other.self_calls);
    }

    fn reaches_yield(&self, yielding_methods: &BTreeSet<String>) -> bool {
        self.writes_pending_yield
            || self
                .self_calls
                .iter()
                .any(|method| yielding_methods.contains(method))
    }
}

impl<'ast> Visit<'ast> for YieldEvidence {
    fn visit_expr_assign(&mut self, node: &'ast syn::ExprAssign) {
        if let syn::Expr::Field(field) = node.left.as_ref()
            && matches!(&field.member, syn::Member::Named(name) if name == "pending_yield")
            && is_self(&field.base)
            && is_some_call(&node.right)
        {
            self.writes_pending_yield = true;
        }
        syn::visit::visit_expr_assign(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if is_self(&node.receiver) {
            self.self_calls.insert(node.method.to_string());
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        // `Self::helper(self, ..)` spelling of a method call.
        if let syn::Expr::Path(path) = node.func.as_ref()
            && path.path.segments.len() == 2
            && path.path.segments[0].ident == "Self"
            && let Some(method) = last_ident(&path.path)
        {
            self.self_calls.insert(method);
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        // Macro bodies are opaque token streams to syn; scan their text so a
        // yield inside a macro argument cannot hide from the policy check.
        let tokens = node.tokens.to_string();
        if tokens.contains("self . pending_yield = Some") {
            self.writes_pending_yield = true;
        }
        for rest in tokens.split("self . ").skip(1) {
            let method: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !method.is_empty() && rest[method.len()..].trim_start().starts_with('(') {
                self.self_calls.insert(method);
            }
        }
        syn::visit::visit_macro(self, node);
    }
}

/// Finds the outermost `match native { .. }` of a domain dispatch function.
#[derive(Default)]
struct NativeMatchFinder<'ast> {
    found: Option<&'ast syn::ExprMatch>,
}

impl<'ast> Visit<'ast> for NativeMatchFinder<'ast> {
    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        if self.found.is_none()
            && matches!(node.expr.as_ref(), syn::Expr::Path(path) if path.path.is_ident("native"))
        {
            self.found = Some(node);
            return;
        }
        syn::visit::visit_expr_match(self, node);
    }
}

fn arm_variants(pattern: &syn::Pat, dispatch_fn: &str, out: &mut Vec<String>) {
    match pattern {
        syn::Pat::Ident(ident) if ident.subpat.is_none() => out.push(ident.ident.to_string()),
        syn::Pat::Path(path) => out.push(
            last_ident(&path.path).expect("native dispatch path pattern has a final segment"),
        ),
        syn::Pat::Or(or) => {
            for case in &or.cases {
                arm_variants(case, dispatch_fn, out);
            }
        }
        syn::Pat::Guard(guarded) => arm_variants(&guarded.pat, dispatch_fn, out),
        // The catch-all arm forwards to the next domain's dispatch function,
        // whose own arms are scanned separately.
        syn::Pat::Wild(_) => {}
        _ => panic!(
            "{dispatch_fn}: unsupported native dispatch pattern; extend the yield-policy scan"
        ),
    }
}

/// Finds `if index == NativeFn::X as u32 { .. NativeCallOutcome::Yield .. }`
/// natives resolved as explicit VM control flow before immediate dispatch.
#[derive(Default)]
struct PreDispatchYieldFinder {
    natives: BTreeSet<String>,
}

#[derive(Default)]
struct YieldOutcomeFinder {
    found: bool,
}

impl<'ast> Visit<'ast> for YieldOutcomeFinder {
    fn visit_path(&mut self, node: &'ast syn::Path) {
        let segments: Vec<_> = node.segments.iter().map(|s| s.ident.to_string()).collect();
        if segments.ends_with(&["NativeCallOutcome".to_owned(), "Yield".to_owned()]) {
            self.found = true;
        }
        syn::visit::visit_path(self, node);
    }
}

fn cast_native_name(expr: &syn::Expr) -> Option<String> {
    let syn::Expr::Cast(cast) = expr else {
        return None;
    };
    let syn::Expr::Path(path) = cast.expr.as_ref() else {
        return None;
    };
    let segments: Vec<_> = path.path.segments.iter().collect();
    (segments.len() == 2 && segments[0].ident == "NativeFn").then(|| segments[1].ident.to_string())
}

impl<'ast> Visit<'ast> for PreDispatchYieldFinder {
    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        if let syn::Expr::Binary(binary) = node.cond.as_ref()
            && matches!(binary.op, syn::BinOp::Eq(_))
            && let Some(native) =
                cast_native_name(&binary.right).or_else(|| cast_native_name(&binary.left))
        {
            let mut outcome = YieldOutcomeFinder::default();
            outcome.visit_block(&node.then_branch);
            if outcome.found {
                self.natives.insert(native);
            }
        }
        syn::visit::visit_expr_if(self, node);
    }
}

#[derive(Default)]
struct NativesSource {
    /// `NativeContext` method name -> merged yield evidence. Same-named methods
    /// in different impl blocks are merged conservatively.
    methods: BTreeMap<String, YieldEvidence>,
    /// Native variant name -> yield evidence of its immediate dispatch arm.
    dispatch_arms: BTreeMap<String, YieldEvidence>,
    /// Natives that yield before immediate dispatch.
    pre_dispatch_yields: BTreeSet<String>,
    problems: Vec<String>,
}

impl NativesSource {
    fn record_dispatch_arms(&mut self, dispatch_fn: &str, block: &syn::Block) {
        let mut finder = NativeMatchFinder::default();
        finder.visit_block(block);
        let Some(native_match) = finder.found else {
            self.problems
                .push(format!("{dispatch_fn}: no `match native` found"));
            return;
        };
        for arm in &native_match.arms {
            let mut variants = Vec::new();
            arm_variants(&arm.pat, dispatch_fn, &mut variants);
            for variant in variants {
                let evidence = YieldEvidence::of(|evidence| {
                    // syn 3 folds a match guard into the pattern
                    // (`Pat::Guard`); visiting the pattern covers it.
                    evidence.visit_pat(&arm.pat);
                    evidence.visit_expr(&arm.body);
                });
                if self
                    .dispatch_arms
                    .insert(variant.clone(), evidence)
                    .is_some()
                {
                    self.problems
                        .push(format!("{variant}: more than one dispatch arm"));
                }
            }
        }
    }
}

impl<'ast> Visit<'ast> for NativesSource {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if !is_test_only(&node.attrs) {
            syn::visit::visit_item_mod(self, node);
        }
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let syn::Type::Path(self_type) = node.self_ty.as_ref() else {
            return;
        };
        if is_test_only(&node.attrs)
            || last_ident(&self_type.path).is_none_or(|name| name != "NativeContext")
        {
            return;
        }
        let trait_name = node.trait_.as_ref().and_then(|(path, _)| last_ident(path));
        for item in &node.items {
            let syn::ImplItem::Fn(function) = item else {
                continue;
            };
            if is_test_only(&function.attrs) {
                continue;
            }
            let name = function.sig.ident.to_string();
            let evidence = YieldEvidence::of(|evidence| evidence.visit_block(&function.block));
            self.methods
                .entry(name.clone())
                .or_default()
                .merge(evidence);
            if name.starts_with("dispatch_") {
                self.record_dispatch_arms(&name, &function.block);
            }
            if trait_name.as_deref() == Some("HostFunctions") && name == "call" {
                let mut finder = PreDispatchYieldFinder::default();
                finder.visit_block(&function.block);
                self.pre_dispatch_yields.extend(finder.natives);
            }
        }
    }
}

/// Method names (on any receiver other than `self`) and non-`self`
/// `pending_yield` writes: yields routed around the `self.` call graph.
#[derive(Default)]
struct ForeignYieldUses {
    method_calls: BTreeSet<String>,
    pending_yield_writes: usize,
}

impl<'ast> Visit<'ast> for ForeignYieldUses {
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if !is_self(&node.receiver) {
            self.method_calls.insert(node.method.to_string());
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_assign(&mut self, node: &'ast syn::ExprAssign) {
        if let syn::Expr::Field(field) = node.left.as_ref()
            && matches!(&field.member, syn::Member::Named(name) if name == "pending_yield")
            && !is_self(&field.base)
        {
            self.pending_yield_writes += 1;
        }
        syn::visit::visit_expr_assign(self, node);
    }
}

fn yielding_methods(methods: &BTreeMap<String, YieldEvidence>) -> BTreeSet<String> {
    let mut yielding: BTreeSet<String> = methods
        .iter()
        .filter(|(_, evidence)| evidence.writes_pending_yield)
        .map(|(name, _)| name.clone())
        .collect();
    loop {
        let newly_yielding: Vec<String> = methods
            .iter()
            .filter(|(name, evidence)| {
                !yielding.contains(*name) && evidence.reaches_yield(&yielding)
            })
            .map(|(name, _)| name.clone())
            .collect();
        if newly_yielding.is_empty() {
            return yielding;
        }
        yielding.extend(newly_yielding);
    }
}

#[test]
fn registry_yield_policy_matches_dispatch_source_for_every_native() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let natives_root = manifest.join("src/natives");

    // The scan below only reads `natives/`; nothing else may write the field.
    for file in rust_sources(&manifest.join("src")) {
        if file.starts_with(&natives_root) || is_test_file(&file) {
            continue;
        }
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
        assert!(
            !text.contains("pending_yield"),
            "{} touches NativeContext::pending_yield outside natives/; extend the yield-policy scan",
            file.display()
        );
    }

    let natives_files: Vec<_> = rust_sources(&natives_root)
        .into_iter()
        .filter(|file| !is_test_file(file))
        .map(|file| (parse(&file), file))
        .collect();
    let mut source = NativesSource::default();
    let mut foreign = ForeignYieldUses::default();
    for (syntax, _) in &natives_files {
        source.visit_file(syntax);
        foreign.visit_file(syntax);
    }
    assert!(
        source.problems.is_empty(),
        "native dispatch source is not in the shape the yield-policy scan understands:\n{}",
        source.problems.join("\n")
    );

    let yielding = yielding_methods(&source.methods);
    for helper in ["yield_engine_action", "launch_script_sequence"] {
        assert!(
            yielding.contains(helper),
            "yield-policy scan no longer recognises the `{helper}` yield helper"
        );
    }
    assert_eq!(
        foreign.pending_yield_writes, 0,
        "pending_yield written through a non-self receiver; extend the yield-policy scan"
    );
    // Domain dispatchers are legitimately entered through a context parameter
    // by `dispatch::call_immediate`; their arms are checked individually.
    let foreign_yield_calls: Vec<_> = foreign
        .method_calls
        .intersection(&yielding)
        .filter(|method| !method.starts_with("dispatch_"))
        .collect();
    assert!(
        foreign_yield_calls.is_empty(),
        "yielding NativeContext helpers called through a non-self receiver: {foreign_yield_calls:?}; extend the yield-policy scan"
    );

    let registry_names: BTreeSet<&str> = NATIVE_REGISTRY
        .iter()
        .map(|definition| definition.signature.name)
        .collect();
    let mut mismatches = Vec::new();
    let mut source_yielding = BTreeSet::new();
    for definition in NATIVE_REGISTRY {
        let name = definition.signature.name;
        let arm = source.dispatch_arms.get(name);
        let pre_dispatch = source.pre_dispatch_yields.contains(name);
        if arm.is_none() && !pre_dispatch {
            mismatches.push(format!("{name}: no dispatch arm found"));
            continue;
        }
        let can_yield = pre_dispatch || arm.is_some_and(|arm| arm.reaches_yield(&yielding));
        if can_yield {
            source_yielding.insert(name);
        }
        let registered = definition.native.may_yield();
        if can_yield != registered {
            mismatches.push(format!(
                "{name}: registry may_yield() = {registered}, but its dispatch {}",
                if can_yield {
                    "can set pending_yield"
                } else {
                    "never sets pending_yield"
                }
            ));
        }
    }
    for name in source.dispatch_arms.keys() {
        if !registry_names.contains(name.as_str()) {
            mismatches.push(format!("{name}: dispatch arm names no registered native"));
        }
    }
    for name in &source.pre_dispatch_yields {
        if !registry_names.contains(name.as_str()) {
            mismatches.push(format!(
                "{name}: pre-dispatch yield names no registered native"
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "native registry yield column disagrees with dispatch source:\n{}",
        mismatches.join("\n")
    );
    assert!(
        !source_yielding.is_empty(),
        "yield-policy scan found no yielding natives"
    );
}
