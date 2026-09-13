//! Stable native IDs, signatures, and mission name bindings.

mod defs;
pub mod signatures;
pub use defs::{NativeFn, ORIGINAL_NATIVE_COUNT, RUST_EXTENSION_NATIVE_START, native_name};
pub use signatures::*;

/// Default resource policy for imported globals and new native allocations.
/// This is not an array-size or file-format constraint. Imports may
/// explicitly choose a larger limit; native writes to existing slots remain
/// valid, but `InitGlobal` cannot request an unbounded new allocation.
pub const DEFAULT_SCRIPT_GLOBAL_SLOT_LIMIT: usize = 65_535;

/// Dispatch category attached to each entry of the native registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum NativeDomain {
    ScriptCore,
    Actors,
    Ai,
    Sequences,
    World,
    Campaign,
}

/// Spellforge Lua name tables. Vanilla missions leave these empty.
#[derive(
    Clone, Debug, Default, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct ScriptNameBindings {
    pub actors: std::collections::BTreeMap<String, i32>,
    pub items: std::collections::BTreeMap<String, i32>,
    pub locations: std::collections::BTreeMap<String, i32>,
    pub patrols: std::collections::BTreeMap<String, i32>,
    pub scrolls: std::collections::BTreeMap<String, i32>,
}
