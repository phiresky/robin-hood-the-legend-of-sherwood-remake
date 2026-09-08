//! `robin_lua` — Lua scripting host for custom missions.
//!
//! This crate retains the native `mlua` host adapter used by developer tools
//! and direct binding tests. Live gameplay uses `robin_spellforge`'s one
//! pure-Rust Lua 5.1 VM on native and wasm so deterministic behavior cannot
//! drift between platform-specific interpreters.
//! The former duplicate Spellforge runtime was removed; `robin_spellforge`
//! owns runtime conformance tests as well as live execution.
//!
//! The mission-load side (extracting zips from `datadirs/mods/`,
//! choosing which `.rhm` + `.lua` to run, hooking the level loader)
//! is owned by a separate workstream and isn't in this crate; this
//! crate exposes the API surface that the loader plugs into.

#![deny(unsafe_op_in_unsafe_fn)]

mod natives;
mod state;

pub use natives::{NATIVE_ALIASES, NativeAbiError, NativeBinding, register_natives};
pub use state::{MissionLuaError, MissionLuaState};
