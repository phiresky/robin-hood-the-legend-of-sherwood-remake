//! Versioned deterministic contract between the engine and a Spellforge Lua VM.
//!
//! The engine owns the serializable tape.  A process-local Lua implementation
//! is attached through [`SpellforgeRuntime`] in [`crate::engine::LevelAssets`].
//! After save/load or rollback adoption the runtime reconstructs its arbitrary
//! Lua heap by replaying the tape while returning the recorded native results
//! without applying their engine side effects a second time.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::hash::Hasher;
use std::ops::Index;
use std::path::{Component, Path};
use std::sync::Arc;

use robin_util::state_hash::StateHash;

pub const SPELLFORGE_CONTRACT_VERSION: u32 = 1;
/// Prefix for the executable ABI's SHA-256 identity. The runtime computes the
/// digest from its interpreter source pin, bootstrap, registry, sandbox,
/// limits, and contract semantics; engine-only snapshot code validates the
/// stable wire shape before the concrete runtime performs exact comparison.
pub const SPELLFORGE_VM_ABI_SCHEME: &str = "spellforge-v1-sha256:";

/// Authoritative Spellforge history ceilings. These are part of the VM ABI:
/// peers, saves, and replays must reject a tape before it can exceed the
/// browser-safe snapshot budget rather than eventually failing an allocation.
pub const SPELLFORGE_TAPE_EVENT_LIMIT: u32 = 131_072;
pub const SPELLFORGE_TAPE_NATIVE_CALL_LIMIT: u32 = 1_048_576;
pub const SPELLFORGE_TAPE_TRANSCRIPT_ENTRY_LIMIT: u32 = 1_048_576;
pub const SPELLFORGE_TAPE_BYTE_LIMIT: u64 = 16 * 1024 * 1024;
pub const SPELLFORGE_SNAPSHOT_BYTE_LIMIT: u64 = 32 * 1024 * 1024;
pub const SPELLFORGE_EVENT_NATIVE_CALL_LIMIT: usize = 65_536;
pub const SPELLFORGE_EVENT_TRANSCRIPT_ENTRY_LIMIT: usize = 131_072;
pub const SPELLFORGE_ARGUMENT_WORD_LIMIT: usize = 256;
pub const SPELLFORGE_TAPE_STRING_BYTE_LIMIT: usize = 4 * 1024;
/// Maximum combined uncompressed Lua source retained by one canonical
/// package. These wire limits live in the engine so replay/save/network
/// admission can reject hostile serialized packages before constructing a VM.
pub const SPELLFORGE_PACKAGE_SOURCE_LIMIT: usize = 16 * 1024 * 1024;
pub const SPELLFORGE_PACKAGE_FILE_LIMIT: usize = 2_048;
pub const SPELLFORGE_PACKAGE_PATH_LIMIT: usize = 1_024;
pub const SPELLFORGE_PACKAGE_METADATA_LIMIT: usize = 2 * 1024 * 1024;

pub fn is_spellforge_vm_abi_identifier(identifier: &str) -> bool {
    identifier
        .strip_prefix(SPELLFORGE_VM_ABI_SCHEME)
        .is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum SpellforgeScriptMode {
    /// The upstream Spellforge convention: the Lua companion replaces SCB.
    Replace,
    /// Run Lua first, then the matching SCB callback.
    AugmentBefore,
    /// Run SCB first, then the matching Lua callback.  The Lua result wins.
    AugmentAfter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub struct SpellforgePackage {
    pub contract_version: u32,
    pub vm_abi: String,
    pub script_mode: SpellforgeScriptMode,
    /// Canonical path within `files` for the mission chunk.
    pub entrypoint: String,
    /// Canonical lowercase forward-slash paths to the exact mission and lib
    /// bytes.  Saves and initial multiplayer snapshots therefore carry the
    /// package needed to verify/rebuild the VM, not merely a mutable disk path.
    pub files: BTreeMap<String, Vec<u8>>,
    pub sha256: [u8; 32],
}

impl SpellforgePackage {
    /// Recompute the exact package identity. Keeping this on the engine-owned
    /// wire type lets snapshot admission validate bytes before state hashing;
    /// callers never have to trust the digest supplied by a remote peer.
    pub fn computed_sha256(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(self.contract_version.to_le_bytes());
        digest.update(self.vm_abi.as_bytes());
        digest.update([match self.script_mode {
            SpellforgeScriptMode::Replace => 0,
            SpellforgeScriptMode::AugmentBefore => 1,
            SpellforgeScriptMode::AugmentAfter => 2,
        }]);
        digest.update((self.entrypoint.len() as u64).to_le_bytes());
        digest.update(self.entrypoint.as_bytes());
        for (path, bytes) in &self.files {
            digest.update((path.len() as u64).to_le_bytes());
            digest.update(path.as_bytes());
            digest.update((bytes.len() as u64).to_le_bytes());
            digest.update(bytes);
        }
        digest.finalize().into()
    }

    /// Validate the complete serialized package shape without executing guest
    /// code. Concrete runtimes additionally compare `vm_abi` with their exact
    /// executable digest before compiling the entrypoint.
    pub fn validate_wire(&self) -> Result<(), String> {
        if self.contract_version != SPELLFORGE_CONTRACT_VERSION {
            return Err(format!(
                "unsupported Spellforge package contract {}; expected {SPELLFORGE_CONTRACT_VERSION}",
                self.contract_version
            ));
        }
        if !is_spellforge_vm_abi_identifier(&self.vm_abi) {
            return Err(format!(
                "invalid Spellforge VM ABI identifier `{}`; expected {SPELLFORGE_VM_ABI_SCHEME}<64 lowercase hex digits>",
                self.vm_abi,
            ));
        }
        if self.entrypoint.is_empty() || !self.files.contains_key(&self.entrypoint) {
            return Err(format!(
                "Spellforge package entrypoint `{}` is absent",
                self.entrypoint
            ));
        }
        if self.entrypoint.contains('/') {
            return Err("Spellforge entrypoint must be at the package root".into());
        }
        if self.files.len() > SPELLFORGE_PACKAGE_FILE_LIMIT {
            return Err(format!(
                "Spellforge package contains {} files; limit is {SPELLFORGE_PACKAGE_FILE_LIMIT}",
                self.files.len()
            ));
        }
        let mut metadata_bytes = 0usize;
        let mut source_bytes = 0usize;
        for (path, bytes) in &self.files {
            if path.len() > SPELLFORGE_PACKAGE_PATH_LIMIT {
                return Err(format!(
                    "Spellforge package path is {} bytes; limit is {SPELLFORGE_PACKAGE_PATH_LIMIT}",
                    path.len()
                ));
            }
            metadata_bytes = metadata_bytes
                .checked_add(path.len())
                .ok_or_else(|| "Spellforge package path metadata size overflow".to_owned())?;
            if metadata_bytes > SPELLFORGE_PACKAGE_METADATA_LIMIT {
                return Err(format!(
                    "Spellforge package path metadata exceeds {} MiB",
                    SPELLFORGE_PACKAGE_METADATA_LIMIT / (1024 * 1024)
                ));
            }
            if path.is_empty()
                || path.contains('\\')
                || path.contains('\0')
                || path.split('/').any(str::is_empty)
                || path != &path.to_ascii_lowercase()
            {
                return Err(format!(
                    "Spellforge package path `{path}` is not canonical lowercase forward-slash form"
                ));
            }
            let path_value = Path::new(path);
            if path_value.is_absolute()
                || path_value.components().any(|component| {
                    matches!(
                        component,
                        Component::CurDir
                            | Component::ParentDir
                            | Component::RootDir
                            | Component::Prefix(_)
                    )
                })
            {
                return Err(format!(
                    "unsafe Spellforge package path `{}`",
                    path_value.display()
                ));
            }
            source_bytes = source_bytes
                .checked_add(bytes.len())
                .ok_or_else(|| "Spellforge package source size overflow".to_owned())?;
            if source_bytes > SPELLFORGE_PACKAGE_SOURCE_LIMIT {
                return Err(format!(
                    "Spellforge package source exceeds the {} MiB runtime limit",
                    SPELLFORGE_PACKAGE_SOURCE_LIMIT / (1024 * 1024)
                ));
            }
        }
        let computed = self.computed_sha256();
        if computed != self.sha256 {
            return Err(format!(
                "Spellforge package hash mismatch: declared {}, computed {}",
                hex_hash(&self.sha256),
                hex_hash(&computed),
            ));
        }
        Ok(())
    }

    fn accounted_bytes(&self) -> u64 {
        let fixed = 4_u64 + 1 + 32;
        self.files.iter().fold(
            fixed
                .saturating_add(accounted_string_bytes(&self.vm_abi))
                .saturating_add(accounted_string_bytes(&self.entrypoint)),
            |total, (path, bytes)| {
                total
                    .saturating_add(accounted_string_bytes(path))
                    .saturating_add(8)
                    .saturating_add(bytes.len() as u64)
            },
        )
    }
}

/// Package bytes are immutable and validated at every construction/snapshot
/// boundary, so deterministic frame hashes use their cryptographic identity
/// instead of walking up to 16 MiB of source every second.
impl StateHash for SpellforgePackage {
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        self.sha256.state_hash(state);
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum SpellforgeTarget {
    Global,
    Actor {
        class: String,
        handle: i32,
    },
    Target {
        class: String,
        handle: i32,
    },
    Scroll {
        class: String,
        handle: i32,
    },
    Zone {
        class: String,
        index: u32,
    },
    Waypoint {
        class: String,
        path: u16,
        waypoint: u8,
    },
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SpellforgeInvocation {
    pub target: SpellforgeTarget,
    /// Spellforge-facing event name (`Timer`, `Enter`, `Taken`, ...), not the
    /// Original-game SCB spelling used by the engine.
    pub event: String,
    pub args: Vec<i32>,
    pub script_this: i32,
    pub current_scroll: i32,
}

/// Stable failure categories crossing the guest/engine/host boundary.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum SpellforgeGuestErrorKind {
    Compatibility,
    Syntax,
    Runtime,
    Memory,
    Budget,
    Sandbox,
    Divergence,
    Protocol,
    ResourceLimit,
    HostNative,
    Internal,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SpellforgeTraceFrame {
    pub source: String,
    pub line: u32,
    pub function: Option<String>,
}

/// Serializable guest failure retained by saves and multiplayer diagnostics.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
    thiserror::Error,
)]
#[error("{kind:?}: {message}")]
pub struct SpellforgeGuestError {
    pub kind: SpellforgeGuestErrorKind,
    pub message: String,
    pub invocation: Option<SpellforgeInvocation>,
    pub traceback: Vec<SpellforgeTraceFrame>,
}

impl SpellforgeGuestError {
    pub fn new(kind: SpellforgeGuestErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            invocation: None,
            traceback: Vec::new(),
        }
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        Self::new(SpellforgeGuestErrorKind::Runtime, message)
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new(SpellforgeGuestErrorKind::Protocol, message)
    }

    pub fn with_invocation(mut self, invocation: SpellforgeInvocation) -> Self {
        self.invocation = Some(invocation);
        self
    }
}

impl From<String> for SpellforgeGuestError {
    fn from(message: String) -> Self {
        Self::runtime(message)
    }
}

impl From<&str> for SpellforgeGuestError {
    fn from(message: &str) -> Self {
        Self::runtime(message)
    }
}

/// Non-recursive depth-first transcript for callbacks dispatched while an
/// outer Lua native is suspended.
///
/// Keeping begin/request/return/complete as a flat grammar avoids recursive
/// snapshot-codec layouts while preserving the exact point at which every
/// nested callback ran. A consumer validates the grammar and VM step at each
/// entry; malformed or divergent transcripts are fatal.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum SpellforgeNestedTapeEntry {
    Begin(SpellforgeInvocation),
    NativeRequest {
        native_index: u32,
        arguments: Vec<i32>,
    },
    NativeReturn {
        return_word: i32,
    },
    Complete {
        result: i32,
    },
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SpellforgeNativeCall {
    pub native_index: u32,
    pub arguments: Vec<i32>,
    pub return_word: i32,
    /// Flat depth-first transcript of Lua callbacks synchronously dispatched
    /// by this engine native. Keeping it at the call site lets heap
    /// reconstruction run callbacks while the parent coroutine is suspended.
    #[serde(default)]
    pub nested_transcript: Vec<SpellforgeNestedTapeEntry>,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SpellforgeEventRecord {
    pub invocation: SpellforgeInvocation,
    pub native_calls: Vec<SpellforgeNativeCall>,
    pub result: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpellforgeTapeUsage {
    pub events: u32,
    pub native_calls: u32,
    pub transcript_entries: u32,
    /// Exact byte accounting for the canonical retained values, independent
    /// of allocator layout and snapshot-codec implementation details.
    pub retained_bytes: u64,
}

impl SpellforgeTapeUsage {
    fn saturating_add(self, other: Self) -> Self {
        Self {
            events: self.events.saturating_add(other.events),
            native_calls: self.native_calls.saturating_add(other.native_calls),
            transcript_entries: self
                .transcript_entries
                .saturating_add(other.transcript_entries),
            retained_bytes: self.retained_bytes.saturating_add(other.retained_bytes),
        }
    }

    fn validate_total(self) -> Result<(), String> {
        if self.events > SPELLFORGE_TAPE_EVENT_LIMIT {
            return Err(format!(
                "Spellforge tape contains {} events; limit is {SPELLFORGE_TAPE_EVENT_LIMIT}",
                self.events
            ));
        }
        if self.native_calls > SPELLFORGE_TAPE_NATIVE_CALL_LIMIT {
            return Err(format!(
                "Spellforge tape contains {} native calls; limit is {SPELLFORGE_TAPE_NATIVE_CALL_LIMIT}",
                self.native_calls
            ));
        }
        if self.transcript_entries > SPELLFORGE_TAPE_TRANSCRIPT_ENTRY_LIMIT {
            return Err(format!(
                "Spellforge tape contains {} nested transcript entries; limit is {SPELLFORGE_TAPE_TRANSCRIPT_ENTRY_LIMIT}",
                self.transcript_entries
            ));
        }
        if self.retained_bytes > SPELLFORGE_TAPE_BYTE_LIMIT {
            return Err(format!(
                "Spellforge tape retains {} bytes; limit is {SPELLFORGE_TAPE_BYTE_LIMIT}",
                self.retained_bytes
            ));
        }
        Ok(())
    }
}

struct SpellforgeJournalNode {
    previous: Option<Arc<SpellforgeJournalNode>>,
    event: Arc<SpellforgeEventRecord>,
}

/// Append-only persistent Spellforge event journal.
///
/// A rollback/checkpoint clone shares one immutable tail pointer instead of
/// copying every historical `Arc`. Serialization deliberately flattens the
/// chain to chronological records, keeping the authoritative wire layout
/// non-recursive and independent of this in-memory optimization.
#[derive(Clone, Default)]
pub struct SpellforgeJournal {
    tail: Option<Arc<SpellforgeJournalNode>>,
    usage: SpellforgeTapeUsage,
    digest: [u8; 32],
    validation_error: Option<String>,
}

impl Drop for SpellforgeJournal {
    fn drop(&mut self) {
        // A one-node-per-event persistent chain would otherwise recurse while
        // dropping a long unique journal. Shared rollback prefixes stop this
        // loop safely; whichever journal releases the last tail later resumes
        // iterative reclamation from there.
        let mut tail = self.tail.take();
        while let Some(node) = tail {
            match Arc::try_unwrap(node) {
                Ok(mut node) => tail = node.previous.take(),
                Err(_) => break,
            }
        }
    }
}

impl SpellforgeJournal {
    pub fn len(&self) -> usize {
        self.usage.events as usize
    }

    pub fn is_empty(&self) -> bool {
        self.usage.events == 0
    }

    pub fn usage(&self) -> SpellforgeTapeUsage {
        self.usage
    }

    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }

    pub fn iter(&self) -> SpellforgeJournalIter<'_> {
        let mut nodes = Vec::with_capacity(self.len());
        let mut current = self.tail.as_deref();
        while let Some(node) = current {
            nodes.push(node);
            current = node.previous.as_deref();
        }
        SpellforgeJournalIter { nodes }
    }

    pub fn get(&self, index: usize) -> Option<&Arc<SpellforgeEventRecord>> {
        if index >= self.len() {
            return None;
        }
        let mut reverse_index = self.len() - index - 1;
        let mut current = self.tail.as_deref();
        while reverse_index != 0 {
            current = current?.previous.as_deref();
            reverse_index -= 1;
        }
        current.map(|node| &node.event)
    }

    pub fn shares_tail_with(&self, other: &Self) -> bool {
        match (&self.tail, &other.tail) {
            (None, None) => true,
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    pub fn is_direct_extension_of(&self, previous: &Self) -> bool {
        if self.len() != previous.len().saturating_add(1) {
            return false;
        }
        match (
            self.tail.as_deref().and_then(|tail| tail.previous.as_ref()),
            previous.tail.as_ref(),
        ) {
            (None, None) => true,
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(error) = &self.validation_error {
            return Err(error.clone());
        }
        self.usage.validate_total()
    }

    pub fn append(&mut self, event: SpellforgeEventRecord) -> Result<(), SpellforgeGuestError> {
        self.validate().map_err(resource_limit)?;
        let event = Arc::new(event);
        let measured = measure_event(&event).map_err(resource_limit)?;
        let prospective = self.usage.saturating_add(measured);
        prospective.validate_total().map_err(resource_limit)?;
        self.append_measured(event, prospective);
        Ok(())
    }

    pub fn from_records(
        records: Vec<Arc<SpellforgeEventRecord>>,
    ) -> Result<Self, SpellforgeGuestError> {
        let journal = Self::from_decoded_records(records);
        journal.validate().map_err(resource_limit)?;
        Ok(journal)
    }

    fn from_decoded_records(records: Vec<Arc<SpellforgeEventRecord>>) -> Self {
        let mut journal = Self::default();
        for event in records {
            let measured = match measure_event(&event) {
                Ok(measured) => measured,
                Err(error) => {
                    if journal.validation_error.is_none() {
                        journal.validation_error = Some(error);
                    }
                    measure_event_unchecked(&event)
                }
            };
            let prospective = journal.usage.saturating_add(measured);
            if let Err(error) = prospective.validate_total()
                && journal.validation_error.is_none()
            {
                journal.validation_error = Some(error);
            }
            journal.append_measured(event, prospective);
        }
        journal
    }

    fn append_measured(
        &mut self,
        event: Arc<SpellforgeEventRecord>,
        prospective: SpellforgeTapeUsage,
    ) {
        self.digest = extend_journal_digest(self.digest, &event);
        self.tail = Some(Arc::new(SpellforgeJournalNode {
            previous: self.tail.clone(),
            event,
        }));
        self.usage = prospective;
    }
}

pub struct SpellforgeJournalIter<'a> {
    nodes: Vec<&'a SpellforgeJournalNode>,
}

impl<'a> Iterator for SpellforgeJournalIter<'a> {
    type Item = &'a Arc<SpellforgeEventRecord>;

    fn next(&mut self) -> Option<Self::Item> {
        self.nodes.pop().map(|node| &node.event)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.nodes.len(), Some(self.nodes.len()))
    }
}

impl ExactSizeIterator for SpellforgeJournalIter<'_> {}

impl Index<usize> for SpellforgeJournal {
    type Output = Arc<SpellforgeEventRecord>;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index)
            .unwrap_or_else(|| panic!("Spellforge journal index {index} out of bounds"))
    }
}

impl fmt::Debug for SpellforgeJournal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpellforgeJournal")
            .field("events", &self.iter().collect::<Vec<_>>())
            .field("usage", &self.usage)
            .field("digest", &hex_hash(&self.digest))
            .field("validation_error", &self.validation_error)
            .finish()
    }
}

impl PartialEq for SpellforgeJournal {
    fn eq(&self, other: &Self) -> bool {
        self.usage == other.usage
            && self.digest == other.digest
            && self.validation_error == other.validation_error
            && (self.shares_tail_with(other) || self.iter().eq(other.iter()))
    }
}

impl Eq for SpellforgeJournal {}

impl Serialize for SpellforgeJournal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_seq(self.iter())
    }
}

impl<'de> Deserialize<'de> for SpellforgeJournal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let records = Vec::<Arc<SpellforgeEventRecord>>::deserialize(deserializer)?;
        let journal = Self::from_decoded_records(records);
        journal.validate().map_err(serde::de::Error::custom)?;
        Ok(journal)
    }
}

impl crate::bitcode_adapters::NativeBitcode for SpellforgeJournal {
    type Wire = Vec<Arc<SpellforgeEventRecord>>;

    fn to_wire(&self) -> Self::Wire {
        self.iter().cloned().collect()
    }

    fn from_wire(records: Self::Wire) -> Self {
        // NativeBitcode's conversion hook is infallible. Preserve every exact
        // decoded record and retain the first limit error; snapshot preflight
        // rejects it before state hashing or level attachment.
        Self::from_decoded_records(records)
    }
}

crate::bitcode_adapters::impl_native_bitcode!(SpellforgeJournal);

impl StateHash for SpellforgeJournal {
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        self.usage.events.state_hash(state);
        self.usage.native_calls.state_hash(state);
        self.usage.transcript_entries.state_hash(state);
        self.usage.retained_bytes.state_hash(state);
        self.digest.state_hash(state);
        self.validation_error.state_hash(state);
    }
}

#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SpellforgeTape {
    pub contract_version: u32,
    /// Shared across rollback clones; native snapshot codecs still encode the
    /// exact bytes once per serialized snapshot.
    pub package: Option<Arc<SpellforgePackage>>,
    /// Arc keeps per-frame rollback clones proportional to new journal data
    /// instead of copying the mission's complete Lua history every frame.
    pub events: SpellforgeJournal,
    /// First fatal guest failure. Once set, authoritative frame admission
    /// aborts the mission rather than panicking inside a callback.
    #[serde(default)]
    pub failure: Option<SpellforgeGuestError>,
}

impl SpellforgeTape {
    pub fn initialize(&mut self, package: SpellforgePackage) -> Result<(), String> {
        package.validate_wire()?;
        match self.package.as_deref() {
            Some(installed) if installed != &package => Err(format!(
                "Spellforge package identity changed from {} to {} (exact contract metadata and bytes must match)",
                hex_hash(&installed.sha256),
                hex_hash(&package.sha256)
            )),
            Some(_) => Ok(()),
            None if self.events.is_empty() => {
                self.contract_version = SPELLFORGE_CONTRACT_VERSION;
                self.package = Some(Arc::new(package));
                Ok(())
            }
            None => Err("Spellforge tape contains events without package bytes".to_owned()),
        }
    }

    pub fn fail(&mut self, failure: SpellforgeGuestError) {
        if self.failure.is_none() {
            self.failure = Some(failure);
        }
    }

    pub fn append_event(
        &mut self,
        event: SpellforgeEventRecord,
    ) -> Result<(), SpellforgeGuestError> {
        let mut prospective = self.events.clone();
        prospective.append(event)?;
        self.validate_snapshot_with_journal(&prospective)
            .map_err(resource_limit)?;
        self.events = prospective;
        Ok(())
    }

    pub fn validate_snapshot(&self) -> Result<(), String> {
        self.validate_snapshot_with_journal(&self.events)
    }

    fn validate_snapshot_with_journal(&self, journal: &SpellforgeJournal) -> Result<(), String> {
        journal.validate()?;
        match self.package.as_deref() {
            None if journal.is_empty() => return Ok(()),
            None => return Err("Spellforge tape contains events without package bytes".to_owned()),
            Some(package) => {
                if self.contract_version != SPELLFORGE_CONTRACT_VERSION {
                    return Err(format!(
                        "unsupported Spellforge snapshot contract {}; expected {SPELLFORGE_CONTRACT_VERSION}",
                        self.contract_version
                    ));
                }
                package.validate_wire()?;
                let snapshot_bytes = package
                    .accounted_bytes()
                    .saturating_add(journal.usage.retained_bytes);
                if snapshot_bytes > SPELLFORGE_SNAPSHOT_BYTE_LIMIT {
                    return Err(format!(
                        "Spellforge package and tape retain {snapshot_bytes} bytes; snapshot limit is {SPELLFORGE_SNAPSHOT_BYTE_LIMIT}"
                    ));
                }
            }
        }
        Ok(())
    }
}

fn resource_limit(message: impl Into<String>) -> SpellforgeGuestError {
    SpellforgeGuestError::new(SpellforgeGuestErrorKind::ResourceLimit, message)
}

mod accounting;
use accounting::{accounted_string_bytes, measure_event, measure_event_unchecked};
pub use accounting::{validate_arguments, validate_invocation};

struct Sha256StateHasher(Sha256);

impl Hasher for Sha256StateHasher {
    fn finish(&self) -> u64 {
        let digest = self.0.clone().finalize();
        u64::from_le_bytes(
            digest[..8]
                .try_into()
                .expect("SHA-256 prefix is eight bytes"),
        )
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
}

fn extend_journal_digest(previous: [u8; 32], event: &SpellforgeEventRecord) -> [u8; 32] {
    let mut hasher = Sha256StateHasher(Sha256::new());
    hasher.write(b"robin-spellforge-journal-v1\0");
    hasher.write(&previous);
    event.state_hash(&mut hasher);
    hasher.0.finalize().into()
}

pub fn hex_hash(hash: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(64);
    for byte in hash {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpellforgeStep {
    Complete {
        activation: u64,
        result: i32,
    },
    Native {
        activation: u64,
        native_index: u32,
        arguments: Vec<i32>,
    },
}

/// Process-local VM implementation.  Every method uses `&self` so a concrete
/// runtime can synchronize its interpreter behind a mutex and be shared by
/// cloned immutable [`crate::engine::LevelAssets`] handles.
pub trait SpellforgeRuntime: Send + Sync {
    fn package(&self) -> &SpellforgePackage;

    /// Attach the immutable RHM name table after level parsing and before the
    /// first Initialize callback. This is process-local data, but its contents
    /// are already part of the loaded level compatibility boundary.
    fn set_name_bindings(&self, names: crate::natives::ScriptNameBindings);

    fn has_handler(
        &self,
        invocation: &SpellforgeInvocation,
        tape: &SpellforgeTape,
    ) -> Result<bool, SpellforgeGuestError>;

    /// Synchronize the process-local heap to `tape`, start one event, and run
    /// until it returns or requests an engine native.
    fn begin(
        &self,
        invocation: SpellforgeInvocation,
        tape: &SpellforgeTape,
    ) -> Result<SpellforgeStep, SpellforgeGuestError>;

    /// Resume an activation after the engine has executed and recorded the
    /// native result word.
    fn resume(
        &self,
        activation: u64,
        return_word: i32,
        tape: &SpellforgeTape,
    ) -> Result<SpellforgeStep, SpellforgeGuestError>;

    /// Finish the active event and append its complete native transcript.
    fn commit(
        &self,
        activation: u64,
        result: i32,
        tape: &mut SpellforgeTape,
    ) -> Result<(), SpellforgeGuestError>;

    fn abort(&self, activation: u64);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package() -> SpellforgePackage {
        let mut package = SpellforgePackage {
            contract_version: SPELLFORGE_CONTRACT_VERSION,
            vm_abi: format!("{SPELLFORGE_VM_ABI_SCHEME}{}", "5a".repeat(32)),
            script_mode: SpellforgeScriptMode::Replace,
            entrypoint: "mission.lua".to_owned(),
            files: BTreeMap::from([(
                "mission.lua".to_owned(),
                b"function Initialize() end".to_vec(),
            )]),
            sha256: [0; 32],
        };
        package.sha256 = package.computed_sha256();
        package
    }

    #[test]
    fn package_and_nested_tape_survive_authoritative_codec() {
        let mut tape = SpellforgeTape::default();
        tape.initialize(package()).unwrap();
        let invocation = SpellforgeInvocation {
            target: SpellforgeTarget::Actor {
                class: "Guard".to_owned(),
                handle: 7,
            },
            event: "ProcessMessage".to_owned(),
            args: vec![4, 5, 6],
            script_this: 7,
            current_scroll: 0,
        };
        let nested_invocation = SpellforgeInvocation {
            target: SpellforgeTarget::Global,
            event: "ProcessMessage".to_owned(),
            args: vec![99],
            script_this: 0,
            current_scroll: 0,
        };
        tape.append_event(SpellforgeEventRecord {
            invocation,
            native_calls: vec![SpellforgeNativeCall {
                native_index: 42,
                arguments: vec![1, 2],
                return_word: 8,
                nested_transcript: vec![
                    SpellforgeNestedTapeEntry::Begin(nested_invocation),
                    SpellforgeNestedTapeEntry::Complete { result: 3 },
                ],
            }],
            result: 11,
        })
        .unwrap();
        tape.fail(
            SpellforgeGuestError::new(
                SpellforgeGuestErrorKind::Budget,
                "instruction budget exceeded",
            )
            .with_invocation(SpellforgeInvocation {
                target: SpellforgeTarget::Global,
                event: "Timer".to_owned(),
                args: Vec::new(),
                script_this: 0,
                current_scroll: 0,
            }),
        );

        let rollback_clone = tape.clone();
        assert!(Arc::ptr_eq(
            tape.package.as_ref().unwrap(),
            rollback_clone.package.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(&tape.events[0], &rollback_clone.events[0]));
        assert!(tape.events.shares_tail_with(&rollback_clone.events));

        let encoded = bitcode::encode(&tape);
        let decoded: SpellforgeTape = bitcode::decode(&encoded).unwrap();
        assert_eq!(decoded, tape);
        assert_eq!(
            decoded.failure.as_ref().unwrap().kind,
            SpellforgeGuestErrorKind::Budget
        );
        assert_ne!(robin_util::state_hash::compute(&decoded), 0);
    }

    #[test]
    fn package_contract_and_abi_mismatches_are_fatal() {
        let mut invalid_version = package();
        invalid_version.contract_version += 1;
        assert!(
            SpellforgeTape::default()
                .initialize(invalid_version)
                .unwrap_err()
                .contains("unsupported Spellforge package contract")
        );

        let mut invalid_abi = package();
        invalid_abi.vm_abi = "other-vm".to_owned();
        assert!(
            SpellforgeTape::default()
                .initialize(invalid_abi)
                .unwrap_err()
                .contains("invalid Spellforge VM ABI identifier")
        );

        let mut tape = SpellforgeTape::default();
        tape.initialize(package()).unwrap();
        let mut changed_bytes = package();
        changed_bytes.files.insert(
            "mission.lua".to_owned(),
            b"function Finalize() end".to_vec(),
        );
        changed_bytes.sha256 = changed_bytes.computed_sha256();
        assert!(
            tape.initialize(changed_bytes)
                .unwrap_err()
                .contains("exact contract metadata and bytes must match")
        );
    }

    #[test]
    fn persistent_journal_rejects_oversized_vectors_and_shares_history() {
        let mut tape = SpellforgeTape::default();
        tape.initialize(package()).unwrap();
        let event = SpellforgeEventRecord {
            invocation: SpellforgeInvocation {
                target: SpellforgeTarget::Global,
                event: "Timer".to_owned(),
                args: Vec::new(),
                script_this: 0,
                current_scroll: 0,
            },
            native_calls: Vec::new(),
            result: 0,
        };
        tape.append_event(event.clone()).unwrap();
        let checkpoint = tape.clone();
        assert!(tape.events.shares_tail_with(&checkpoint.events));
        tape.append_event(event).unwrap();
        assert_eq!(checkpoint.events.len(), 1);
        assert_eq!(tape.events.len(), 2);
        assert!(!tape.events.shares_tail_with(&checkpoint.events));

        let oversized = SpellforgeEventRecord {
            invocation: SpellforgeInvocation {
                target: SpellforgeTarget::Global,
                event: "Timer".to_owned(),
                args: vec![0; SPELLFORGE_ARGUMENT_WORD_LIMIT + 1],
                script_this: 0,
                current_scroll: 0,
            },
            native_calls: Vec::new(),
            result: 0,
        };
        let error = tape.append_event(oversized).unwrap_err();
        assert_eq!(error.kind, SpellforgeGuestErrorKind::ResourceLimit);
        assert_eq!(tape.events.len(), 2, "failed append must be transactional");

        let invalid_wire = bitcode::encode(&vec![Arc::new(SpellforgeEventRecord {
            invocation: SpellforgeInvocation {
                target: SpellforgeTarget::Global,
                event: "Timer".to_owned(),
                args: vec![0; SPELLFORGE_ARGUMENT_WORD_LIMIT + 1],
                script_this: 0,
                current_scroll: 0,
            },
            native_calls: Vec::new(),
            result: 0,
        })]);
        let decoded: SpellforgeJournal = bitcode::decode(&invalid_wire).unwrap();
        assert!(
            decoded.validate().unwrap_err().contains("argument words"),
            "infallible native decoding must retain a preflight rejection"
        );
    }

    #[test]
    fn package_hash_tampering_is_rejected_before_snapshot_hashing() {
        let mut tampered = package();
        tampered.files.get_mut("mission.lua").unwrap().push(b' ');
        let error = SpellforgeTape::default().initialize(tampered).unwrap_err();
        assert!(error.contains("package hash mismatch"), "{error}");
    }

    #[test]
    fn long_mission_journal_clones_and_hashes_constant_sized_roots() {
        #[derive(Default)]
        struct ByteCountingHasher(usize);
        impl std::hash::Hasher for ByteCountingHasher {
            fn finish(&self) -> u64 {
                self.0 as u64
            }

            fn write(&mut self, bytes: &[u8]) {
                self.0 += bytes.len();
            }
        }

        let event = || SpellforgeEventRecord {
            invocation: SpellforgeInvocation {
                target: SpellforgeTarget::Global,
                event: "Timer".to_owned(),
                args: vec![1],
                script_this: 0,
                current_scroll: 0,
            },
            native_calls: Vec::new(),
            result: 0,
        };
        let mut journal = SpellforgeJournal::default();
        journal.append(event()).unwrap();
        let mut short_hash = ByteCountingHasher::default();
        journal.state_hash(&mut short_hash);

        for _ in 1..8_192 {
            let checkpoint = journal.clone();
            assert!(journal.shares_tail_with(&checkpoint));
            journal.append(event()).unwrap();
            assert!(journal.is_direct_extension_of(&checkpoint));
        }
        assert_eq!(journal.len(), 8_192);
        let mut long_hash = ByteCountingHasher::default();
        journal.state_hash(&mut long_hash);
        assert_eq!(
            long_hash.0, short_hash.0,
            "state hashing must touch only journal counters and digest"
        );

        let encoded = bitcode::encode(&journal);
        let decoded: SpellforgeJournal = bitcode::decode(&encoded).unwrap();
        assert_eq!(decoded, journal);
        assert_eq!(decoded.usage(), journal.usage());
        assert_eq!(decoded.digest(), journal.digest());
    }
}
