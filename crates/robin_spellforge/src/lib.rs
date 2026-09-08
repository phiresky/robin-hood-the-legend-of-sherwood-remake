//! Cross-platform deterministic Spellforge mission VM.
//!
//! The live game deliberately uses this one pure-Rust Lua 5.1 interpreter on
//! native and `wasm32-unknown-unknown`.  The engine owns package bytes and the
//! event/native tape; this process-local heap is disposable and reconstructs
//! itself by replaying that tape after load, rollback, or snapshot adoption.

mod package;

pub use package::{
    ARCHIVE_BYTE_LIMIT, ARCHIVE_DIRECTORY_LIMIT, ARCHIVE_ENTRY_LIMIT, PACKAGE_SOURCE_LIMIT,
    SpellforgePackageError, SpellforgePackageErrorKind, build_package_from_archives,
};

mod runtime;
pub use runtime::{
    SpellforgeRuntime51, compute_package_sha256, spellforge_vm_abi, spellforge_vm_abi_digest,
    validate_package,
};
