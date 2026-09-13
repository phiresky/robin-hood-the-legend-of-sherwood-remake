//! Parse-once cache over the shared source inventory.
//!
//! Included with `#[path = "support/syntax.rs"] mod syntax;` only by the
//! guardrail binaries that inspect syntax trees, so text-only scanners do not
//! compile an unused parser.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::support::{SourceFile, rust_sources};

/// The parsed syntax tree for `file`, parsed at most once per thread.
///
/// `syn` nodes hold `proc_macro2` spans, which are neither `Send` nor `Sync`,
/// so parsed trees cannot live in the binary-wide `OnceLock` next to the
/// source text; tests that share one tree run their checks on one thread.
pub fn parse(file: &SourceFile) -> Rc<syn::File> {
    thread_local! {
        static TREES: RefCell<HashMap<PathBuf, Rc<syn::File>>> = RefCell::new(HashMap::new());
    }
    TREES.with_borrow_mut(|trees| {
        Rc::clone(trees.entry(file.path.clone()).or_insert_with(|| {
            Rc::new(
                syn::parse_file(&file.text).unwrap_or_else(|error| {
                    panic!("failed to parse {}: {error}", file.path.display())
                }),
            )
        }))
    })
}

/// The parsed syntax tree of the single Rust file at `path`.
pub fn parse_path(path: &Path) -> Rc<syn::File> {
    let files = rust_sources(path);
    let [file] = &files[..] else {
        panic!("{} must name exactly one Rust file", path.display());
    };
    parse(file)
}
