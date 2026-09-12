//! Guard against unlabelled or ambient simulation randomness.
//! Refactoring labelled calls does not require maintaining duplicate counts.

use robin_engine::sim_rng::{AuxiliaryRngSite, RngSite};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use strum::IntoEnumIterator;
use syn::visit::Visit;

const REVIEWED_PUBLIC_ENTRY_POINTS: &[&str] = &[
    "bool",
    "c_rand_unit_inclusive",
    "f32",
    "i16",
    "i32",
    "script_rand",
    // The diagnostic-carrying twin of `script_rand`: the same script
    // `Rand(max)` native dispatch selects it when a VM diagnostic context
    // is attached, and it draws exactly one value from the same
    // `RngSite::ScriptRand`. Authoritative for the same reason.
    "script_rand_with_context",
    "shuffle",
    "u16",
    "u32",
    "u8",
    "usize",
    "with_auxiliary_seed",
    "with_seed",
];

const REVIEWED_AMBIENT_RNG_USES: &[(&str, usize)] = &[
    (
        "crates/robin_engine/src/engine/simulation_rng.rs|fastrand::Rng::with_seed",
        2,
    ),
    (
        "crates/robin_rs/src/game_session/interactive.rs|fastrand::Rng::new",
        1,
    ),
    (
        "crates/robin_rs/src/ingame_menu/dialogue.rs|fastrand::Rng::new",
        2,
    ),
    (
        "crates/robin_rs/src/leaderboard/ranked_session.rs|rand::random",
        2,
    ),
    (
        "crates/robin_rs/src/native_game_identity.rs|rand::random",
        1,
    ),
];

fn is_test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("test")
            || (attr.path().is_ident("cfg")
                && matches!(&attr.meta, syn::Meta::List(list) if list.tokens.to_string().contains("test")))
    })
}

struct RngSourceVisitor<'a> {
    file: &'a Path,
    sites: BTreeMap<String, usize>,
    auxiliary_sites: BTreeMap<String, usize>,
    unlabelled_calls: Vec<String>,
    ambient_rng: Vec<String>,
    macro_rng: Vec<String>,
}

impl RngSourceVisitor<'_> {
    fn path_text(path: &syn::Path) -> String {
        path.segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::")
    }

    fn site_name(expr: &syn::Expr, enum_name: &str) -> Option<String> {
        let syn::Expr::Path(path) = expr else {
            return None;
        };
        let segments = path.path.segments.iter().collect::<Vec<_>>();
        (segments.len() >= 2 && segments[segments.len() - 2].ident == enum_name)
            .then(|| segments.last().expect("checked length").ident.to_string())
    }
}

impl<'ast> Visit<'ast> for RngSourceVisitor<'_> {
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        let tokens = node.tokens.to_string();
        // Naming a site enum is data, not a draw: the `parity_rng_owner`
        // trace lines record which site the *following* statement is
        // about. Only a real entry-point call inside a macro is a hazard,
        // because the macro body may never be evaluated.
        let draws_rng = tokens.split("sim_rng ::").skip(1).any(|rest| {
            let rest = rest.trim_start();
            !rest.starts_with("RngSite") && !rest.starts_with("AuxiliaryRngSite")
        }) || tokens.contains("fastrand ::")
            || tokens.contains("rand ::");
        if draws_rng {
            self.macro_rng.push(Self::path_text(&node.path));
        }
        syn::visit::visit_macro(self, node);
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if !is_test_only(&node.attrs) {
            syn::visit::visit_item_mod(self, node);
        }
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if !is_test_only(&node.attrs) {
            syn::visit::visit_item_fn(self, node);
        }
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if !is_test_only(&node.attrs) {
            syn::visit::visit_impl_item_fn(self, node);
        }
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        let segments = node.path.segments.iter().collect::<Vec<_>>();
        if segments.len() >= 2 && segments[segments.len() - 2].ident == "RngSite" {
            *self
                .sites
                .entry(segments.last().expect("checked length").ident.to_string())
                .or_default() += 1;
        }
        if segments.len() >= 2 && segments[segments.len() - 2].ident == "AuxiliaryRngSite" {
            *self
                .auxiliary_sites
                .entry(segments.last().expect("checked length").ident.to_string())
                .or_default() += 1;
        }
        if segments
            .first()
            .is_some_and(|segment| segment.ident == "fastrand" || segment.ident == "rand")
        {
            self.ambient_rng.push(Self::path_text(&node.path));
        }
        syn::visit::visit_expr_path(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        let syn::Expr::Path(function) = node.func.as_ref() else {
            syn::visit::visit_expr_call(self, node);
            return;
        };
        let path = Self::path_text(&function.path);
        let helper = function
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default();
        let is_draw = path.contains("sim_rng")
            && matches!(
                helper.as_str(),
                "u32"
                    | "i32"
                    | "u16"
                    | "u8"
                    | "i16"
                    | "usize"
                    | "bool"
                    | "f32"
                    | "shuffle"
                    | "c_rand_unit_inclusive"
                    | "script_rand"
            );
        if is_draw {
            let labelled = node
                .args
                .iter()
                .nth(1)
                .and_then(|expr| Self::site_name(expr, "RngSite"))
                .is_some();
            let forwarded_sprite_site = self.file.ends_with("sprite.rs")
                && helper == "u16"
                && node.args.iter().nth(1).is_some_and(
                    |arg| matches!(arg, syn::Expr::Path(path) if path.path.is_ident("site")),
                );
            if !labelled && !forwarded_sprite_site {
                self.unlabelled_calls.push(path.clone());
            }
        }
        if path.contains("sim_rng") && helper == "with_auxiliary_seed" {
            let labelled = node
                .args
                .first()
                .and_then(|expr| Self::site_name(expr, "AuxiliaryRngSite"))
                .is_some();
            if !labelled {
                self.unlabelled_calls.push(path);
            }
        }
        syn::visit::visit_expr_call(self, node);
    }
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

#[test]
fn gaussian_sampler_sites_must_be_explicit_at_each_draw() {
    for (source, expected_unlabelled) in [
        (
            "fn sample() { let site = RngSite::AiRandomValueGauss; sim_rng::i16(sim, site, 0..width); }",
            1,
        ),
        (
            "fn sample() { let draw = || sim_rng::i16(sim, RngSite::AiRandomValueGauss, 0..width); draw(); draw(); draw(); }",
            0,
        ),
    ] {
        let syntax = syn::parse_file(source).expect("parse sampler inventory fixture");
        let mut visitor = RngSourceVisitor {
            file: Path::new("crates/robin_engine/src/ai/controller.rs"),
            sites: BTreeMap::new(),
            auxiliary_sites: BTreeMap::new(),
            unlabelled_calls: Vec::new(),
            ambient_rng: Vec::new(),
            macro_rng: Vec::new(),
        };
        visitor.visit_file(&syntax);
        assert_eq!(visitor.unlabelled_calls.len(), expected_unlabelled);
        assert!(visitor.macro_rng.is_empty());
        assert!(visitor.sites.contains_key("AiRandomValueGauss"));
    }
}

#[test]
fn authoritative_rng_source_inventory_is_reviewed() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository = manifest
        .join("../..")
        .canonicalize()
        .expect("resolve repository root");
    let roots = [
        manifest.join("src"),
        manifest.join("../robin_lua/src"),
        manifest.join("../robin_rs/src"),
    ];
    let mut actual = BTreeMap::<String, usize>::new();
    let mut actual_auxiliary = BTreeMap::<String, usize>::new();
    let mut actual_ambient = BTreeMap::<String, usize>::new();
    let mut violations = Vec::new();

    let sim_rng_source =
        std::fs::read_to_string(manifest.join("src/sim_rng.rs")).expect("read sim_rng.rs");
    let sim_rng_syntax = syn::parse_file(&sim_rng_source).expect("parse sim_rng.rs");
    let public_entry_points = sim_rng_syntax
        .items
        .iter()
        .filter_map(|item| {
            let syn::Item::Fn(function) = item else {
                return None;
            };
            matches!(function.vis, syn::Visibility::Public(_))
                .then(|| function.sig.ident.to_string())
        })
        .collect::<BTreeSet<_>>();
    let expected_entry_points = REVIEWED_PUBLIC_ENTRY_POINTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        public_entry_points, expected_entry_points,
        "update the reviewed public sim_rng entry-point inventory"
    );
    for file in roots.iter().flat_map(|root| rust_sources(root)) {
        if file.ends_with("sim_rng.rs") || file.ends_with("engine/tests.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
        let syntax = syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("parse {}: {error}", file.display()));
        let mut visitor = RngSourceVisitor {
            file: &file,
            sites: BTreeMap::new(),
            auxiliary_sites: BTreeMap::new(),
            unlabelled_calls: Vec::new(),
            ambient_rng: Vec::new(),
            macro_rng: Vec::new(),
        };
        visitor.visit_file(&syntax);
        for (site, count) in visitor.sites {
            *actual.entry(site).or_default() += count;
        }
        for (site, count) in visitor.auxiliary_sites {
            *actual_auxiliary.entry(site).or_default() += count;
        }
        for call in visitor.unlabelled_calls {
            violations.push(format!("{}: unlabelled {call}", file.display()));
        }
        for macro_path in visitor.macro_rng {
            violations.push(format!(
                "{}: RNG call hidden inside {macro_path}! macro",
                file.display()
            ));
        }
        let relative = file
            .canonicalize()
            .expect("resolve scanned source")
            .strip_prefix(&repository)
            .expect("source must be inside repository")
            .to_owned();
        for call in visitor.ambient_rng {
            *actual_ambient
                .entry(format!("{}|{call}", relative.display()))
                .or_default() += 1;
        }
    }

    assert!(violations.is_empty(), "{}", violations.join("\n"));
    let expected_ambient = REVIEWED_AMBIENT_RNG_USES
        .iter()
        .map(|&(site, count)| (site.to_owned(), count))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        actual_ambient, expected_ambient,
        "update the reviewed ambient RNG exception inventory"
    );

    let enum_sites = RngSite::iter()
        .map(|site| format!("{site:?}"))
        .collect::<BTreeSet<_>>();
    let expected_sites = actual.keys().cloned().collect::<BTreeSet<_>>();
    assert_eq!(enum_sites, expected_sites, "RngSite and inventory differ");

    let auxiliary_enum_sites = AuxiliaryRngSite::iter()
        .map(|site| format!("{site:?}"))
        .collect::<BTreeSet<_>>();
    let expected_auxiliary_sites = actual_auxiliary.keys().cloned().collect::<BTreeSet<_>>();
    assert_eq!(
        auxiliary_enum_sites, expected_auxiliary_sites,
        "AuxiliaryRngSite and inventory differ"
    );
}
