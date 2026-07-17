//! `robin_lua` — Lua scripting host for custom missions.
//!
//! Background, determinism limits, and architecture audit: see
//! `docs/PARITY_AUDIT.md`.
//!
//! In a sentence: this crate owns the `mlua::Lua` state for one
//! mission, registers the Spellforge-compatible API onto it, and
//! routes engine-side script events (`Initialize`, `Timer`,
//! `ProcessMessage`, …) into the loaded `.lua` file.
//!
//! The mission-load side (extracting zips from `datadirs/mods/`,
//! choosing which `.rhm` + `.lua` to run, hooking the level loader)
//! is owned by a separate workstream and isn't in this crate; this
//! crate exposes the API surface that the loader plugs into.

#![deny(unsafe_op_in_unsafe_fn)]

mod natives;
mod state;

pub use natives::{NATIVE_ALIASES, NativeBinding, register_natives};
pub use state::{MissionLuaError, MissionLuaState};
