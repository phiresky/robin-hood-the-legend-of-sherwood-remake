//! Structural policy where visibility alone is insufficient: reconstruction
//! must not create host scratch, and production restore entry points must use
//! the complete recorded-frame API. This is not a compiler capability test.

use std::collections::BTreeSet;
use syn::visit::{self, Visit};

const HOST_NAMES: &[&str] = &[
    "Host",
    "HostDisplayState",
    "InputState",
    "DevState",
    "run_engine_frame_core",
];
const REPLAY_NAMES: &[&str] = &[
    "replay_frames_to_frame",
    "replay_authoritative_frame",
    "replay_authoritative_frame_profiled",
];

#[derive(Default)]
struct BodyPolicy {
    host_paths: BTreeSet<String>,
    calls: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for BodyPolicy {
    fn visit_macro(&mut self, expression: &'ast syn::Macro) {
        // Common expression-list macros (dbg!, vec!, assertions) can contain
        // real host constructors too. Inspect parsed expressions without
        // mistaking a format-string literal for Rust code.
        use syn::parse::Parser;
        let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
        if let Ok(arguments) = parser.parse2(expression.tokens.clone()) {
            for argument in &arguments {
                self.visit_expr(argument);
            }
        }
        visit::visit_macro(self, expression);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        for segment in &path.segments {
            if HOST_NAMES.iter().any(|name| segment.ident == name) {
                self.host_paths.insert(segment.ident.to_string());
            }
        }
        visit::visit_path(self, path);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = call.func.as_ref()
            && let Some(name) = path.path.segments.last()
        {
            self.calls.insert(name.ident.to_string());
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.calls.insert(call.method.to_string());
        visit::visit_expr_method_call(self, call);
    }
}

struct FunctionPolicy<'a> {
    name: &'a str,
    bodies: Vec<BodyPolicy>,
}

impl FunctionPolicy<'_> {
    fn inspect(&mut self, signature: &syn::Signature, body: &syn::Block) {
        if signature.ident == self.name {
            let mut policy = BodyPolicy::default();
            policy.visit_signature(signature);
            policy.visit_block(body);
            self.bodies.push(policy);
        }
    }
}

impl<'ast> Visit<'ast> for FunctionPolicy<'_> {
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        self.inspect(&function.sig, &function.block);
        visit::visit_item_fn(self, function);
    }

    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        self.inspect(&function.sig, &function.block);
        visit::visit_impl_item_fn(self, function);
    }
}

fn inspect_function(source: &str, name: &str) -> BodyPolicy {
    let syntax = syn::parse_file(source).expect("reconstruction Rust must parse");
    let mut visitor = FunctionPolicy {
        name,
        bodies: Vec::new(),
    };
    visitor.visit_file(&syntax);
    assert_eq!(visitor.bodies.len(), 1, "expected exactly one {name}");
    visitor.bodies.pop().expect("checked one function")
}

#[test]
fn production_reconstruction_has_no_host_scratch_and_uses_complete_frames() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // Named policy boundaries, not spelling/layout of their implementation.
    // A deliberate entry-point move should update this small routing table.
    let roots = [
        ("src/rewind.rs", "rewind_to", false),
        (
            "../robin_engine/src/sim_timeline.rs",
            "replay_frames_to_frame",
            false,
        ),
        (
            "../robin_engine/src/sim_timeline.rs",
            "replay_authoritative_frame",
            false,
        ),
        (
            "../robin_engine/src/sim_timeline.rs",
            "replay_authoritative_frame_profiled",
            true,
        ),
        (
            "src/game_session/multiplayer.rs",
            "rewind_from_recent_timeline_history",
            false,
        ),
        ("src/rollback_checker.rs", "run", false),
    ];
    for (path, function, is_admission) in roots {
        let source = std::fs::read_to_string(manifest.join(path))
            .unwrap_or_else(|error| panic!("read {path}: {error}"));
        let policy = inspect_function(&source, function);
        assert!(
            policy.host_paths.is_empty(),
            "{path}::{function} consumes host scratch: {:?}",
            policy.host_paths
        );
        let complete_frame_call = if is_admission {
            policy.calls.contains("advance_frame")
        } else {
            REPLAY_NAMES.iter().any(|name| policy.calls.contains(*name))
        };
        assert!(
            complete_frame_call,
            "{path}::{function} must use complete recorded-frame admission"
        );
    }
}

#[test]
fn structural_policy_ignores_comments_strings_and_braces() {
    let policy = inspect_function(
        r#"
        fn restore() {
            // Host::scratch() and run_engine_frame_core() are forbidden here.
            let explanation = "} HostDisplayState { InputState DevState";
            crate::timeline::replay_authoritative_frame
                (snapshot, assets, frame);
        }
    "#,
        "restore",
    );
    assert!(policy.host_paths.is_empty());
    assert!(policy.calls.contains("replay_authoritative_frame"));
}

#[test]
fn structural_policy_rejects_real_host_paths_and_missing_admission() {
    let policy = inspect_function(
        r#"
        impl Restore {
            fn restore(&mut self) {
                let host = crate::host::Host :: scratch(800.0, 600.0);
                let input: InputState = Default::default();
                crate::sim_timeline::run_engine_frame_core(host, input);
            }
        }
    "#,
        "restore",
    );
    assert_eq!(
        policy.host_paths,
        BTreeSet::from([
            "Host".to_owned(),
            "InputState".to_owned(),
            "run_engine_frame_core".to_owned(),
        ])
    );
    assert!(!REPLAY_NAMES.iter().any(|name| policy.calls.contains(*name)));
}

#[test]
fn structural_policy_inspects_expression_macros_without_reading_string_contents() {
    let policy = inspect_function(
        r#"fn restore() {
            let host = dbg!(crate::host::Host::default());
            println!("HostDisplayState {{}}", 1);
        }"#,
        "restore",
    );
    assert_eq!(policy.host_paths, BTreeSet::from(["Host".to_owned()]));
}
