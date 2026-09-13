//! Container-level reader for original-game RHSG save files.
//!
//! The original game writes this header before
//! serializing any campaign or engine state:
//!
//! ```text
//! 0x00  four-byte signature: "RHSG" (Linux i386 port) or "GSHR" (retail Win32)
//! 0x04  u32      save header version
//! 0x08  u32      mission profile id
//! 0x0c  u32      legacy file stream version
//! ```
//!
//! The header reader leaves callers at byte 16. [`campaign`] then decodes the
//! two consecutive, lengthless campaign streams which precede
//! the engine state.

// External surface (robin_rs / robin_parity): decode a save, then adopt it.
pub mod adopt_engine;
pub use robin_legacy_save::body;
pub use robin_legacy_save::elements;
pub mod initialized;

// Crate-internal consumers outside `legacy_save` (engine topology/tests).
pub(crate) mod adopt_elements;
pub(crate) mod gate_topology;
pub(crate) mod topology_adapter;

// Everything else is private to the save pipeline, so `dead_code` stays
// meaningful for the decoded payload types.
mod adopt;
mod adopt_actor_ownership;
mod adopt_camera;
mod adopt_campaign;
mod adopt_common;
mod adopt_dynamic_elements;
mod adopt_grid;
mod adopt_hiking_tail;
mod adopt_mobile;
mod adopt_object_leaves;
mod adopt_paths;
mod adopt_pc_human;
mod adopt_post_load;
mod adopt_preamble;
mod adopt_preamble_services;
mod adopt_sequences;
mod adopt_simple;
mod adopt_tail_basic;
mod adopt_tail_runtime;
mod adopt_vm_arena;
mod campaign;
use robin_legacy_save::engine;
use robin_legacy_save::payload_actors;
use robin_legacy_save::payload_ai;
use robin_legacy_save::payload_base;
use robin_legacy_save::payload_context;
use robin_legacy_save::payload_dispatch;
use robin_legacy_save::payload_nonactors;
use robin_legacy_save::payload_objects;
use robin_legacy_save::payload_sequences;
use robin_legacy_save::payload_vm;
use robin_legacy_save::post_grid;
use robin_legacy_save::post_hiking;
use robin_legacy_save::post_sequence_manager;
use robin_legacy_save::post_simple;
use robin_legacy_save::post_tail;
#[cfg(test)]
mod test_support;
mod vm_schema;

pub use robin_legacy_save::{
    LegacySaveAbiProfile, LegacySaveHeader, PORT_LINUX_I386_MAGIC, RETAIL_WINDOWS_X86_MAGIC,
    RHSG_HEADER_LEN, RHSG_MAGIC, RHSG_VERSION, read_header,
};
