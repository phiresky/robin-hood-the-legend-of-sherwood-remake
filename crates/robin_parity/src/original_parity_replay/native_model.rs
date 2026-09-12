//! Native trace envelopes, storage policy and reader state shared by codec and storage.
use super::{
    Deserialize, PathBuf, Read, Serialize, TraceFrame, TraceHeader, TraceRngBatch, TraceRngPrefix,
    VecDeque, bitcode,
};

pub(super) const TRACE_NATIVE_VERSION: u32 = 68;

/// The native parity trace is the authoritative artifact once its JSONL
/// source has been converted (and possibly deleted), so its name carries no
/// version: compatibility is enforced through the versioned header/footer,
/// and an incompatible file must be migrated, never silently regenerated.
/// The suffix appends to the full recording name (`X.jsonl.zst` becomes
/// `X.jsonl.zst.parity.bitcode.zst`) because the `.jsonl.zst` path is the
/// stable trace identity used by sweep status keys, EOF ledgers, and
/// completion markers.
pub(super) const TRACE_NATIVE_SUFFIX: &str = ".parity.bitcode.zst";
pub(super) const TRACE_CONVERSION_QUARANTINE_SUFFIX: &str = ".parity-conversion-source";
pub(super) const TRACE_REBLOCK_SOURCE_SUFFIX: &str = ".parity-reblock-source-v67";
pub(super) const TRACE_REBLOCK_BINDING_SUFFIX: &str = ".parity-reblock-binding-v67.json";
pub(super) const TRACE_NATIVE_FOOTER_MAGIC: [u8; 16] = *b"RHPRTRACEFOOTER!";
pub(super) const TRACE_NATIVE_FOOTER_LEN: u64 = 16 + 4 + 8 + 8;
// Full-session JSONL recordings are compressed as a single zstd frame. Some
// encoders select a frame window from the total uncompressed size, so long
// recordings legitimately exceed zstd's conservative 128 MiB decoder default.
// Keep the reader bounded at zstd's platform maximum while accepting those
// valid trace frames.
pub(super) const TRACE_ZSTD_WINDOW_LOG_MAX: u32 = if usize::BITS >= 64 { 31 } else { 30 };
// Prefer maximum archival density for parity recordings. A representative
// min/median/max corpus benchmark made level 19 16-19% smaller than level 9,
// at the cost of substantially slower conversion. Long-distance matching was
// neutral at level 19, so the native writer deliberately leaves it disabled.
// Rechecked with the production 512 MiB window on interactive session 21
// (11,594 frames, 8,028.68 MiB raw bitcode): LDM off and on both rounded to
// 18.78 MiB (less than 0.01 MiB apart). The single-pass timings, which also
// included native decode and bitcode encode, were 233.75s off and 225.13s on;
// that is not a compression-density reason to pay LDM's extra working state.
pub(super) const TRACE_NATIVE_ZSTD_LEVEL: i32 = 19;
pub(super) const TRACE_NATIVE_LONG_DISTANCE_MATCHING: bool = false;
/// Frames per newly-written on-disk block. A current-schema frame contains a
/// complete Original state envelope, so the former 1,000-record policy could
/// require 2-3.6 GiB of live Rust allocations. Readers have always accepted any
/// non-empty block size, so this storage-only change remains compatible with
/// every existing version-68 reader and does not change the wire layout.
pub(super) const TRACE_NATIVE_BLOCK_RECORDS: usize = 32;
/// Bound the zstd history retained by every replay process. Cross-frame
/// repetition is already captured inside bitcode blocks. A 64 MiB history keeps
/// replay lanes bounded while preserving useful cross-block compression.
pub(super) const TRACE_NATIVE_WINDOW_LOG: u32 = 26;
pub(super) const TRACE_NATIVE_MIN_WINDOW_LOG: u32 = 20;
pub(super) const TRACE_NATIVE_MAX_REBLOCK_WINDOW_LOG: u32 = 29;
pub(super) const TRACE_NATIVE_MAX_REBLOCK_RECORDS: usize = 1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NativeStoragePolicy {
    pub(super) block_records: usize,
    pub(super) window_log: u32,
}

impl Default for NativeStoragePolicy {
    fn default() -> Self {
        Self {
            block_records: TRACE_NATIVE_BLOCK_RECORDS,
            window_log: TRACE_NATIVE_WINDOW_LOG,
        }
    }
}

impl NativeStoragePolicy {
    pub(super) fn new(block_records: usize, window_log: u32) -> Self {
        assert!(
            (1..=TRACE_NATIVE_MAX_REBLOCK_RECORDS).contains(&block_records),
            "--reblock-records must be between 1 and {TRACE_NATIVE_MAX_REBLOCK_RECORDS}"
        );
        assert!(
            (TRACE_NATIVE_MIN_WINDOW_LOG..=TRACE_NATIVE_MAX_REBLOCK_WINDOW_LOG)
                .contains(&window_log),
            "--reblock-window-log must be between {TRACE_NATIVE_MIN_WINDOW_LOG} and {TRACE_NATIVE_MAX_REBLOCK_WINDOW_LOG}"
        );
        Self {
            block_records,
            window_log,
        }
    }
}

/// Native trace header layout for version 68. Do not change its bitcode shape
/// without bumping `TRACE_NATIVE_VERSION` and retaining this type as the v68
/// compatibility decoder.
#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct BinaryTraceHeaderV68 {
    pub(super) version: u32,
    pub(super) source_fingerprint: String,
    pub(super) trace: TraceHeader,
    pub(super) rng_prefix: TraceRngPrefix,
}

/// Native record layout for version 68.
///
/// ON-DISK FORMAT INVARIANT: this enum and every transitively encoded child
/// type are immutable for version 68. Shape changes require a version bump and
/// an explicit offline migration from the frozen previous layout.
#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) enum BinaryTraceRecord {
    Frame(TraceFrame),
    End {
        rng_suffix: Option<TraceRngBatch>,
        final_frame: Option<u64>,
        frame_count: Option<u64>,
    },
}

pub(super) struct BinaryTraceReader {
    pub(super) path: PathBuf,
    pub(super) reader: Box<dyn Read>,
    pub(super) footer: BinaryTraceFooter,
    /// Records of the current block not yet handed out by [`Self::read_record`].
    pub(super) pending: VecDeque<BinaryTraceRecord>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BinaryTraceFooter {
    pub(super) version: u32,
    pub(super) frame_count: u64,
    pub(super) final_frame: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct NativeReblockBinding {
    pub(super) version: u32,
    pub(super) canonical_path: PathBuf,
    pub(super) source_content_sha256: String,
    pub(super) source_bytes: u64,
    pub(super) source_semantic_sha256: String,
    pub(super) frame_count: u64,
    pub(super) final_frame: u64,
    #[cfg(unix)]
    pub(super) source_device: u64,
    #[cfg(unix)]
    pub(super) source_inode: u64,
}
