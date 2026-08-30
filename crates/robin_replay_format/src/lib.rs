//! Canonical compact replay encoding and untrusted-input admission.
//!
//! Production accepts exactly one replay representation:
//! `rhrec-{12-lowercase-hex-build}-{base64url-no-pad(zstd(bitcode(ReplayFile)))}`.
//! There is no format tag, content negotiation, JSON/JSONL fallback, or
//! compatibility decoder at this boundary. JSONL is a crash-safe *local
//! recorder* format and is deliberately kept in the game-facing developer
//! facade instead of this crate.
//!
//! # Trust boundary
//!
//! [`preflight_compact_transport`] is allocation-bounded and safe to run in an
//! API process before spooling an opaque submission. Typed bitcode decoding
//! must be performed by a resource-limited worker: bitcode validates its input
//! before construction, but attacker-controlled collection lengths can still
//! amplify a small valid payload into substantial typed heap/work before the
//! post-decode structural limits can inspect it. The ranked server therefore
//! runs [`decode_compact_for_admission`] under its verifier's address-space,
//! CPU and wall-time limits. Do not move that call into an unsandboxed request
//! handler. Browser public playback must run this decoder in the separately
//! built replay-admission wasm inside a Dedicated Worker; that artifact owns a
//! non-shared linear memory with a CI-inspected 384 MiB maximum. A normal game
//! wasm instance is not an allocation boundary.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64;
use robin_engine::campaign::Campaign;
use robin_engine::replay::{REPLAY_SCHEMA_VERSION, ReplayData, ReplayFile, ReplayFrame};
use serde::Serialize;
use serde::ser::{
    self, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant,
};
use std::fmt;
use std::io::Read as _;

/// Short source identity embedded in compact artifacts made by this build.
pub const ENGINE_VERSION_HASH: &str = env!("ROBIN_GIT_HASH");

pub const COMPACT_PREFIX: &str = "rhrec-";
pub const VERSION_HASH_BYTES: usize = 12;
const ZSTD_LEVEL: i32 = 19;

/// Independent ceilings for every untrusted expansion and typed work stage.
///
/// The defaults are intentionally much larger than the measured local corpus
/// (15 recordings; largest JSONL 1.314 MB, 3,746 frames, ~1 KB campaign), while
/// remaining small enough for the verifier worker's external resource limits
/// to terminate amplification safely. See `docs/feature-reviews/36-...md`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ReplayAdmissionLimits {
    /// Entire ASCII envelope.
    pub max_input_bytes: usize,
    /// Base64url payload before decoding.
    pub max_base64_payload_bytes: usize,
    /// Zstd frame bytes after base64url decoding.
    pub max_compressed_bytes: usize,
    /// Bitcode bytes emitted by zstd.
    pub max_decompressed_bytes: usize,
    /// Maximum zstd frame window accepted by the decoder.
    pub max_zstd_window_log: u32,
    pub max_mission_id_bytes: usize,
    /// Opaque bitcode campaign snapshot in the replay header.
    pub max_campaign_bytes: usize,
    pub max_frames: usize,
    pub max_metadata_records: usize,
    /// Direct fact/action/command/control entries in one frame.
    pub max_entries_per_frame: usize,
    /// Direct entries over the complete replay.
    pub max_total_frame_entries: usize,
    /// Entries in any typed nested sequence or map below a frame/config.
    pub max_typed_collection_entries: usize,
    /// UTF-8 bytes in any typed string below a frame/config.
    pub max_typed_string_bytes: usize,
    /// Combined typed string bytes below all frames/config.
    pub max_total_typed_string_bytes: usize,
    /// Serializer nodes plus collection elements below all frames/config.
    pub max_total_typed_work: usize,
}

pub const DEFAULT_REPLAY_ADMISSION_LIMITS: ReplayAdmissionLimits = ReplayAdmissionLimits {
    max_input_bytes: 16 * 1024 * 1024 + 64,
    max_base64_payload_bytes: 16 * 1024 * 1024,
    max_compressed_bytes: 12 * 1024 * 1024,
    max_decompressed_bytes: 16 * 1024 * 1024,
    // The decoded artifact itself is capped at 16 MiB, so a canonical stream
    // never needs a history window larger than 2^24 bytes. Zstd level 19's
    // canonical encoder currently declares <=8 MiB for measured replays.
    max_zstd_window_log: 24,
    max_mission_id_bytes: 256,
    max_campaign_bytes: 8 * 1024 * 1024,
    max_frames: 500_000,
    max_metadata_records: 500_000,
    max_entries_per_frame: 4_096,
    max_total_frame_entries: 1_000_000,
    max_typed_collection_entries: 65_536,
    max_typed_string_bytes: 16 * 1024,
    max_total_typed_string_bytes: 4 * 1024 * 1024,
    max_total_typed_work: 16_000_000,
};

impl Default for ReplayAdmissionLimits {
    fn default() -> Self {
        DEFAULT_REPLAY_ADMISSION_LIMITS
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayLimitKind {
    CompactInputBytes,
    Base64PayloadBytes,
    CompressedBytes,
    DecompressedBytes,
    ZstdWindowBytes,
    MissionIdBytes,
    CampaignBytes,
    DeclaredFrames,
    FrameRecords,
    MetadataRecords,
    FrameEntries,
    TotalFrameEntries,
    TypedCollectionEntries,
    TypedStringBytes,
    TotalTypedStringBytes,
    TotalTypedWork,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayMetadataKind {
    StateHash,
    SaveMarker,
    LoadBack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalStage {
    Bitcode,
    Zstd,
    Base64Url,
}

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("missing `rhrec-` prefix")]
    MissingPrefix,
    #[error("missing build/payload separator")]
    MissingSeparator,
    #[error("compact replay contains leading/trailing or non-ASCII bytes")]
    NonCanonicalEnvelopeText,
    #[error("build identity must be exactly 12 lowercase hexadecimal characters")]
    InvalidVersionHash,
    #[error("compact replay payload is not unpadded base64url")]
    InvalidBase64UrlText,
    #[error("base64url decode failed: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("zstd decode failed: {0}")]
    Zstd(std::io::Error),
    #[error("bitcode decode failed: {0}")]
    Bitcode(#[from] bitcode::Error),
    #[error("embedded campaign bitcode decode failed: {0}")]
    CampaignBitcode(bitcode::Error),
    #[error("embedded campaign is not canonical bitcode")]
    NonCanonicalCampaign,
    #[error("embedded campaign history is invalid: {0}")]
    InvalidCampaignHistory(String),
    #[error("invalid replay layout: {0}")]
    InvalidLayout(String),
    #[error("compact replay is not canonical at the {stage:?} stage")]
    NonCanonical { stage: CanonicalStage },
    #[error("compact replay {kind:?} observed {observed}, limit is {limit}")]
    LimitExceeded {
        kind: ReplayLimitKind,
        observed: usize,
        limit: usize,
    },
    #[error("compact replay count overflowed while checking {kind:?}")]
    CountOverflow { kind: ReplayLimitKind },
    #[error("invalid replay admission limits: {0}")]
    InvalidLimits(String),
    #[error("invalid zstd frame header: {0}")]
    InvalidZstdHeader(String),
    #[error("replay declares {declared} frames but contains {actual} frame records")]
    FrameCountMismatch { declared: u32, actual: usize },
    #[error("replay expected frame ordinal {expected} but found {actual}")]
    UnexpectedFrameOrdinal { expected: u32, actual: u32 },
    #[error(
        "replay {kind:?} metadata references frame {frame}, outside total_frames {total_frames}"
    )]
    MetadataFrameOutsideReplay {
        kind: ReplayMetadataKind,
        frame: u32,
        total_frames: u32,
    },
    #[error(
        "unsupported replay schema version {version}; supported version is {REPLAY_SCHEMA_VERSION}"
    )]
    UnsupportedVersion { version: u32 },
    #[error("replay engine version `{recorded}` does not match required build `{required}`")]
    EngineVersionMismatch { recorded: String, required: String },
    #[error("this build has no durable source identity; production replay admission is disabled")]
    BuildIdentityUnavailable,
    #[error("typed replay budget traversal failed: {0}")]
    TypedBudget(String),
}

/// Cheap, no-decompression view used by the network/API spool boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompactTransportPreflight<'a> {
    pub version_hash: &'a str,
    pub base64_payload: &'a str,
    /// Upper bound implied by base64 length; the exact byte count is checked
    /// after decoding inside the sandbox.
    pub estimated_compressed_bytes: usize,
}

/// Validate the only accepted envelope grammar without decoding attacker
/// controlled zstd/bitcode in the API process.
pub fn preflight_compact_transport<'a>(
    text: &'a str,
    limits: &ReplayAdmissionLimits,
) -> Result<CompactTransportPreflight<'a>, FormatError> {
    check_limit(
        ReplayLimitKind::CompactInputBytes,
        text.len(),
        limits.max_input_bytes,
    )?;
    if !text.is_ascii() || text.trim() != text {
        return Err(FormatError::NonCanonicalEnvelopeText);
    }
    let rest = text
        .strip_prefix(COMPACT_PREFIX)
        .ok_or(FormatError::MissingPrefix)?;
    let (version_hash, payload) = rest.split_once('-').ok_or(FormatError::MissingSeparator)?;
    validate_version_hash(version_hash)?;
    check_limit(
        ReplayLimitKind::Base64PayloadBytes,
        payload.len(),
        limits.max_base64_payload_bytes,
    )?;
    if payload.is_empty()
        || payload.len() % 4 == 1
        || !payload
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(FormatError::InvalidBase64UrlText);
    }
    let estimated_compressed_bytes =
        payload
            .len()
            .checked_mul(3)
            .ok_or(FormatError::CountOverflow {
                kind: ReplayLimitKind::CompressedBytes,
            })?
            / 4;
    check_limit(
        ReplayLimitKind::CompressedBytes,
        estimated_compressed_bytes,
        limits.max_compressed_bytes,
    )?;
    Ok(CompactTransportPreflight {
        version_hash,
        base64_payload: payload,
        estimated_compressed_bytes,
    })
}

/// Encode one canonical compact representation.
pub fn encode_compact(data: &ReplayData, hash: &str) -> Result<String, FormatError> {
    validate_version_hash(hash)?;
    validate_replay_data(data)?;
    let file = ReplayFile::from(data);
    let encoded = bitcode::encode(&file);
    let compressed = zstd::encode_all(encoded.as_slice(), ZSTD_LEVEL).map_err(FormatError::Zstd)?;
    Ok(format!(
        "{COMPACT_PREFIX}{hash}-{}",
        BASE64.encode(compressed)
    ))
}

/// Decode a trusted compact replay while still enforcing canonical bytes and
/// the current replay schema. This lane does not apply public resource limits.
pub fn decode_compact(text: &str) -> Result<(String, ReplayData), FormatError> {
    decode_compact_inner(text, None, None)
}

/// Decode under explicit limits. This must execute inside a resource-limited
/// worker when `text` came from an untrusted submitter.
pub fn decode_compact_bounded(
    text: &str,
    limits: &ReplayAdmissionLimits,
) -> Result<(String, ReplayData), FormatError> {
    decode_compact_inner(text, Some(limits), None)
}

/// Decode the current build's production format under public limits.
///
/// The build hash is rejected before base64/zstd work. Server installations
/// that route several approved build hashes should call
/// [`decode_compact_for_build`] in the selected build's sandbox.
pub fn decode_compact_for_admission(text: &str) -> Result<(String, ReplayData), FormatError> {
    if ENGINE_VERSION_HASH == "unknown" {
        return Err(FormatError::BuildIdentityUnavailable);
    }
    decode_compact_for_build(text, &DEFAULT_REPLAY_ADMISSION_LIMITS, ENGINE_VERSION_HASH)
}

pub fn decode_compact_for_build(
    text: &str,
    limits: &ReplayAdmissionLimits,
    required_hash: &str,
) -> Result<(String, ReplayData), FormatError> {
    validate_version_hash(required_hash)?;
    decode_compact_inner(text, Some(limits), Some(required_hash))
}

fn decode_compact_inner(
    text: &str,
    limits: Option<&ReplayAdmissionLimits>,
    required_hash: Option<&str>,
) -> Result<(String, ReplayData), FormatError> {
    let unbounded_limits;
    let parse_limits = if let Some(limits) = limits {
        limits
    } else {
        unbounded_limits = ReplayAdmissionLimits {
            max_input_bytes: usize::MAX,
            max_base64_payload_bytes: usize::MAX,
            max_compressed_bytes: usize::MAX,
            max_decompressed_bytes: usize::MAX,
            max_zstd_window_log: 31,
            max_mission_id_bytes: usize::MAX,
            max_campaign_bytes: usize::MAX,
            max_frames: usize::MAX,
            max_metadata_records: usize::MAX,
            max_entries_per_frame: usize::MAX,
            max_total_frame_entries: usize::MAX,
            max_typed_collection_entries: usize::MAX,
            max_typed_string_bytes: usize::MAX,
            max_total_typed_string_bytes: usize::MAX,
            max_total_typed_work: usize::MAX,
        };
        &unbounded_limits
    };
    let preflight = preflight_compact_transport(text, parse_limits)?;
    if let Some(required_hash) = required_hash
        && preflight.version_hash != required_hash
    {
        return Err(FormatError::EngineVersionMismatch {
            recorded: preflight.version_hash.to_owned(),
            required: required_hash.to_owned(),
        });
    }

    let compressed = BASE64.decode(preflight.base64_payload.as_bytes())?;
    if let Some(limits) = limits {
        check_limit(
            ReplayLimitKind::CompressedBytes,
            compressed.len(),
            limits.max_compressed_bytes,
        )?;
    }
    let encoded = decode_zstd_bounded(
        &compressed,
        limits.map(|limits| limits.max_decompressed_bytes),
        limits.map(|limits| limits.max_zstd_window_log),
    )?;

    // No discriminator and no fallback: the decompressed bytes must be the
    // current native ReplayFile bitcode graph.
    let file: ReplayFile = bitcode::decode(&encoded)?;
    if let Some(limits) = limits {
        validate_file_for_admission(&file, limits)?;
    }
    if file.header.version != REPLAY_SCHEMA_VERSION {
        return Err(FormatError::UnsupportedVersion {
            version: file.header.version,
        });
    }

    validate_canonical_bytes(&file, &encoded, &compressed, preflight.base64_payload)?;
    let data = ReplayData::from(file);
    validate_replay_data(&data)?;
    Ok((preflight.version_hash.to_owned(), data))
}

fn validate_canonical_bytes(
    file: &ReplayFile,
    encoded: &[u8],
    compressed: &[u8],
    payload: &str,
) -> Result<(), FormatError> {
    let canonical_encoded = bitcode::encode(file);
    if canonical_encoded != encoded {
        return Err(FormatError::NonCanonical {
            stage: CanonicalStage::Bitcode,
        });
    }
    let canonical_compressed =
        zstd::encode_all(canonical_encoded.as_slice(), ZSTD_LEVEL).map_err(FormatError::Zstd)?;
    if canonical_compressed != compressed {
        return Err(FormatError::NonCanonical {
            stage: CanonicalStage::Zstd,
        });
    }
    if BASE64.encode(canonical_compressed) != payload {
        return Err(FormatError::NonCanonical {
            stage: CanonicalStage::Base64Url,
        });
    }
    Ok(())
}

fn decode_zstd_bounded(
    compressed: &[u8],
    max_decompressed_bytes: Option<usize>,
    max_window_log: Option<u32>,
) -> Result<Vec<u8>, FormatError> {
    if let Some(max_window_log) = max_window_log {
        if !(10..=31).contains(&max_window_log) {
            return Err(FormatError::InvalidLimits(format!(
                "max_zstd_window_log must be in 10..=31, got {max_window_log}"
            )));
        }
        let window_bytes = zstd_frame_window_size(compressed)?;
        let limit = 1usize
            .checked_shl(max_window_log)
            .ok_or_else(|| FormatError::InvalidLimits("zstd window limit overflow".into()))?;
        check_limit(ReplayLimitKind::ZstdWindowBytes, window_bytes, limit)?;
    }
    let mut decoder = zstd::stream::read::Decoder::new(compressed).map_err(FormatError::Zstd)?;
    if let Some(max_window_log) = max_window_log {
        decoder
            .window_log_max(max_window_log)
            .map_err(FormatError::Zstd)?;
    }
    let mut output = Vec::new();
    if let Some(limit) = max_decompressed_bytes {
        let read_limit = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
        decoder
            .take(read_limit)
            .read_to_end(&mut output)
            .map_err(FormatError::Zstd)?;
        check_limit(ReplayLimitKind::DecompressedBytes, output.len(), limit)?;
    } else {
        decoder
            .read_to_end(&mut output)
            .map_err(FormatError::Zstd)?;
    }
    Ok(output)
}

/// Parse the standard zstd frame header without allocating or initializing a
/// decoder context. See RFC 8878 section 3.1.1.1. This makes a forged window a
/// deterministic resource-limit rejection rather than a backend error string.
fn zstd_frame_window_size(frame: &[u8]) -> Result<usize, FormatError> {
    const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
    if frame.get(..4) != Some(ZSTD_MAGIC.as_slice()) {
        return Err(FormatError::InvalidZstdHeader(
            "missing standard zstd magic".into(),
        ));
    }
    let descriptor = *frame
        .get(4)
        .ok_or_else(|| FormatError::InvalidZstdHeader("truncated descriptor".into()))?;
    if descriptor & 0x18 != 0 {
        return Err(FormatError::InvalidZstdHeader(
            "reserved/unused descriptor bits are set".into(),
        ));
    }
    let single_segment = descriptor & 0x20 != 0;
    let dictionary_id_len = match descriptor & 0x03 {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 4,
        _ => unreachable!(),
    };
    let content_size_flag = descriptor >> 6;
    let content_size_len = match content_size_flag {
        0 if single_segment => 1,
        0 => 0,
        1 => 2,
        2 => 4,
        3 => 8,
        _ => unreachable!(),
    };
    let mut cursor = 5usize;
    let window_from_descriptor = if single_segment {
        None
    } else {
        let window_descriptor = *frame
            .get(cursor)
            .ok_or_else(|| FormatError::InvalidZstdHeader("truncated window descriptor".into()))?;
        cursor += 1;
        let exponent = usize::from(window_descriptor >> 3);
        let mantissa = usize::from(window_descriptor & 0x07);
        let base = 1usize
            .checked_shl(u32::try_from(10 + exponent).unwrap_or(u32::MAX))
            .ok_or_else(|| FormatError::InvalidZstdHeader("window size overflow".into()))?;
        Some(base + (base / 8) * mantissa)
    };
    cursor = cursor
        .checked_add(dictionary_id_len)
        .ok_or_else(|| FormatError::InvalidZstdHeader("header offset overflow".into()))?;
    let end = cursor
        .checked_add(content_size_len)
        .ok_or_else(|| FormatError::InvalidZstdHeader("header offset overflow".into()))?;
    let content_size_bytes = frame
        .get(cursor..end)
        .ok_or_else(|| FormatError::InvalidZstdHeader("truncated content size".into()))?;
    let content_size = match content_size_len {
        0 => None,
        1 => Some(u64::from(content_size_bytes[0])),
        2 => Some(
            u64::from(u16::from_le_bytes([
                content_size_bytes[0],
                content_size_bytes[1],
            ])) + 256,
        ),
        4 => Some(u64::from(u32::from_le_bytes(
            content_size_bytes.try_into().expect("four-byte slice"),
        ))),
        8 => Some(u64::from_le_bytes(
            content_size_bytes.try_into().expect("eight-byte slice"),
        )),
        _ => unreachable!(),
    };
    let window = if single_segment {
        content_size.ok_or_else(|| {
            FormatError::InvalidZstdHeader("single-segment frame omitted content size".into())
        })?
    } else {
        u64::try_from(window_from_descriptor.expect("non-single window")).unwrap_or(u64::MAX)
    };
    usize::try_from(window)
        .map_err(|_| FormatError::InvalidZstdHeader("window does not fit this platform".into()))
}

fn validate_file_for_admission(
    file: &ReplayFile,
    limits: &ReplayAdmissionLimits,
) -> Result<(), FormatError> {
    if file.header.version != REPLAY_SCHEMA_VERSION {
        return Err(FormatError::UnsupportedVersion {
            version: file.header.version,
        });
    }
    check_limit(
        ReplayLimitKind::MissionIdBytes,
        file.header.mission_id.len(),
        limits.max_mission_id_bytes,
    )?;
    check_limit(
        ReplayLimitKind::CampaignBytes,
        file.header.campaign.len(),
        limits.max_campaign_bytes,
    )?;
    check_limit(
        ReplayLimitKind::DeclaredFrames,
        file.header.total_frames as usize,
        limits.max_frames,
    )?;
    check_limit(
        ReplayLimitKind::FrameRecords,
        file.frames.len(),
        limits.max_frames,
    )?;
    if file.frames.len() != file.header.total_frames as usize {
        return Err(FormatError::FrameCountMismatch {
            declared: file.header.total_frames,
            actual: file.frames.len(),
        });
    }

    let mut total_frame_entries = 0usize;
    let mut budget = TypedBudget::new(limits);
    // `ReplayHeader::campaign` is nested bitcode, not merely an opaque blob.
    // Decode it under the same external containment as the outer ReplayFile
    // before issuing an exact-byte admission receipt. Otherwise a canonical
    // outer replay could defer an attacker-controlled allocation until the
    // live game session decodes the campaign in its main process.
    let campaign: Campaign =
        bitcode::decode(&file.header.campaign).map_err(FormatError::CampaignBitcode)?;
    if bitcode::encode(&campaign) != file.header.campaign {
        return Err(FormatError::NonCanonicalCampaign);
    }
    campaign
        .validate_history_schema()
        .map_err(FormatError::InvalidCampaignHistory)?;
    campaign.serialize(&mut budget).map_err(FormatError::from)?;
    file.header
        .sim_config
        .serialize(&mut budget)
        .map_err(FormatError::from)?;
    for (expected, (&actual, frame)) in (0..file.header.total_frames).zip(&file.frames) {
        if expected != actual {
            return Err(FormatError::UnexpectedFrameOrdinal { expected, actual });
        }
        let entries = frame_entry_count(frame)?;
        check_limit(
            ReplayLimitKind::FrameEntries,
            entries,
            limits.max_entries_per_frame,
        )?;
        total_frame_entries =
            total_frame_entries
                .checked_add(entries)
                .ok_or(FormatError::CountOverflow {
                    kind: ReplayLimitKind::TotalFrameEntries,
                })?;
        frame.serialize(&mut budget).map_err(FormatError::from)?;
    }
    check_limit(
        ReplayLimitKind::TotalFrameEntries,
        total_frame_entries,
        limits.max_total_frame_entries,
    )?;

    let metadata_records = file
        .hashes
        .len()
        .checked_add(file.save_markers.len())
        .and_then(|count| count.checked_add(file.load_backs.len()))
        .ok_or(FormatError::CountOverflow {
            kind: ReplayLimitKind::MetadataRecords,
        })?;
    check_limit(
        ReplayLimitKind::MetadataRecords,
        metadata_records,
        limits.max_metadata_records,
    )?;
    validate_metadata_frames(
        file.hashes.keys().copied(),
        ReplayMetadataKind::StateHash,
        file.header.total_frames,
    )?;
    validate_metadata_frames(
        file.save_markers.keys().copied(),
        ReplayMetadataKind::SaveMarker,
        file.header.total_frames,
    )?;
    validate_metadata_frames(
        file.load_backs.keys().copied(),
        ReplayMetadataKind::LoadBack,
        file.header.total_frames,
    )
}

fn frame_entry_count(frame: &ReplayFrame) -> Result<usize, FormatError> {
    let input = &frame.input;
    let counts = [
        input.external_facts.director_completions.len(),
        input
            .external_facts
            .sound_boundary
            .as_ref()
            .map_or(0, |boundary| boundary.resolutions.len()),
        input.external_facts.recorded_drop_ale_routes.len(),
        input.external_actions.len(),
        input.commands.len(),
        input.post_external_actions.len(),
        input.post_commands.len(),
        frame.host_controls.len(),
    ];
    counts.into_iter().try_fold(0usize, |total, count| {
        total.checked_add(count).ok_or(FormatError::CountOverflow {
            kind: ReplayLimitKind::FrameEntries,
        })
    })
}

fn validate_metadata_frames(
    frames: impl IntoIterator<Item = u32>,
    kind: ReplayMetadataKind,
    total_frames: u32,
) -> Result<(), FormatError> {
    for frame in frames {
        if frame >= total_frames {
            return Err(FormatError::MetadataFrameOutsideReplay {
                kind,
                frame,
                total_frames,
            });
        }
    }
    Ok(())
}

pub fn validate_replay_data(data: &ReplayData) -> Result<(), FormatError> {
    if data.header.version != REPLAY_SCHEMA_VERSION {
        return Err(FormatError::UnsupportedVersion {
            version: data.header.version,
        });
    }
    data.validate_layout().map_err(FormatError::InvalidLayout)
}

pub fn validate_engine_hash(hash: &str) -> Result<(), FormatError> {
    if ENGINE_VERSION_HASH == "unknown" {
        return Err(FormatError::BuildIdentityUnavailable);
    }
    if hash != ENGINE_VERSION_HASH {
        return Err(FormatError::EngineVersionMismatch {
            recorded: hash.to_owned(),
            required: ENGINE_VERSION_HASH.to_owned(),
        });
    }
    Ok(())
}

fn validate_version_hash(hash: &str) -> Result<(), FormatError> {
    if hash.len() != VERSION_HASH_BYTES
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(FormatError::InvalidVersionHash);
    }
    Ok(())
}

fn check_limit(kind: ReplayLimitKind, observed: usize, limit: usize) -> Result<(), FormatError> {
    if observed > limit {
        Err(FormatError::LimitExceeded {
            kind,
            observed,
            limit,
        })
    } else {
        Ok(())
    }
}

#[derive(Debug)]
enum BudgetError {
    Limit {
        kind: ReplayLimitKind,
        observed: usize,
        limit: usize,
    },
    Overflow(ReplayLimitKind),
    Message(String),
}

impl fmt::Display for BudgetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit {
                kind,
                observed,
                limit,
            } => write!(formatter, "{kind:?} observed {observed}, limit is {limit}"),
            Self::Overflow(kind) => write!(formatter, "{kind:?} count overflow"),
            Self::Message(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for BudgetError {}

impl ser::Error for BudgetError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self::Message(message.to_string())
    }
}

impl From<BudgetError> for FormatError {
    fn from(error: BudgetError) -> Self {
        match error {
            BudgetError::Limit {
                kind,
                observed,
                limit,
            } => Self::LimitExceeded {
                kind,
                observed,
                limit,
            },
            BudgetError::Overflow(kind) => Self::CountOverflow { kind },
            BudgetError::Message(message) => Self::TypedBudget(message),
        }
    }
}

struct TypedBudget<'a> {
    limits: &'a ReplayAdmissionLimits,
    total_work: usize,
    total_string_bytes: usize,
}

impl<'a> TypedBudget<'a> {
    fn new(limits: &'a ReplayAdmissionLimits) -> Self {
        Self {
            limits,
            total_work: 0,
            total_string_bytes: 0,
        }
    }

    fn spend(&mut self, amount: usize) -> Result<(), BudgetError> {
        self.total_work = self
            .total_work
            .checked_add(amount)
            .ok_or_else(|| BudgetError::Overflow(ReplayLimitKind::TotalTypedWork))?;
        self.enforce(
            ReplayLimitKind::TotalTypedWork,
            self.total_work,
            self.limits.max_total_typed_work,
        )
    }

    fn collection(&mut self, len: Option<usize>) -> Result<(), BudgetError> {
        let len =
            len.ok_or_else(|| BudgetError::Message("typed collection omitted its length".into()))?;
        self.enforce(
            ReplayLimitKind::TypedCollectionEntries,
            len,
            self.limits.max_typed_collection_entries,
        )?;
        self.spend(len.saturating_add(1))
    }

    fn string(&mut self, value: &str) -> Result<(), BudgetError> {
        self.enforce(
            ReplayLimitKind::TypedStringBytes,
            value.len(),
            self.limits.max_typed_string_bytes,
        )?;
        self.total_string_bytes =
            self.total_string_bytes
                .checked_add(value.len())
                .ok_or(BudgetError::Overflow(
                    ReplayLimitKind::TotalTypedStringBytes,
                ))?;
        self.enforce(
            ReplayLimitKind::TotalTypedStringBytes,
            self.total_string_bytes,
            self.limits.max_total_typed_string_bytes,
        )?;
        self.spend(1)
    }

    fn enforce(
        &self,
        kind: ReplayLimitKind,
        observed: usize,
        limit: usize,
    ) -> Result<(), BudgetError> {
        if observed > limit {
            Err(BudgetError::Limit {
                kind,
                observed,
                limit,
            })
        } else {
            Ok(())
        }
    }
}

struct Compound<'a, 'limits> {
    budget: &'a mut TypedBudget<'limits>,
}

macro_rules! primitive {
    ($($method:ident($type:ty)),+ $(,)?) => {
        $(fn $method(self, _value: $type) -> Result<Self::Ok, Self::Error> {
            self.spend(1)
        })+
    };
}

impl<'a, 'limits> ser::Serializer for &'a mut TypedBudget<'limits> {
    type Ok = ();
    type Error = BudgetError;
    type SerializeSeq = Compound<'a, 'limits>;
    type SerializeTuple = Compound<'a, 'limits>;
    type SerializeTupleStruct = Compound<'a, 'limits>;
    type SerializeTupleVariant = Compound<'a, 'limits>;
    type SerializeMap = Compound<'a, 'limits>;
    type SerializeStruct = Compound<'a, 'limits>;
    type SerializeStructVariant = Compound<'a, 'limits>;

    primitive!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_i128(i128),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_u128(u128),
        serialize_f32(f32),
        serialize_f64(f64),
        serialize_char(char)
    );

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        self.string(value)
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<Self::Ok, Self::Error> {
        self.collection(Some(value.len()))
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        self.spend(1)
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        self.spend(1)?;
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        self.spend(1)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.spend(1)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.spend(1)
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.spend(1)?;
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.spend(1)?;
        value.serialize(self)
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        self.collection(len)?;
        Ok(Compound { budget: self })
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        self.spend(1)?;
        Ok(Compound { budget: self })
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        self.spend(1)?;
        Ok(Compound { budget: self })
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        self.spend(1)?;
        Ok(Compound { budget: self })
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        self.collection(len)?;
        Ok(Compound { budget: self })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        self.spend(1)?;
        Ok(Compound { budget: self })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        self.spend(1)?;
        Ok(Compound { budget: self })
    }

    fn collect_str<T: ?Sized + fmt::Display>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        self.string(&value.to_string())
    }
}

macro_rules! compound_element {
    ($trait:ident, $method:ident) => {
        impl<'a, 'limits> $trait for Compound<'a, 'limits> {
            type Ok = ();
            type Error = BudgetError;

            fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
                value.serialize(&mut *self.budget)
            }

            fn end(self) -> Result<Self::Ok, Self::Error> {
                Ok(())
            }
        }
    };
}

compound_element!(SerializeSeq, serialize_element);
compound_element!(SerializeTuple, serialize_element);
compound_element!(SerializeTupleStruct, serialize_field);
compound_element!(SerializeTupleVariant, serialize_field);

impl SerializeMap for Compound<'_, '_> {
    type Ok = ();
    type Error = BudgetError;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        key.serialize(&mut *self.budget)
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(&mut *self.budget)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl SerializeStruct for Compound<'_, '_> {
    type Ok = ();
    type Error = BudgetError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut *self.budget)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl SerializeStructVariant for Compound<'_, '_> {
    type Ok = ();
    type Error = BudgetError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut *self.budget)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::engine::{ExternalAction, SimulationFrameInput};
    use robin_engine::player_command::{PlayerCommand, PlayerInput};
    use robin_engine::replay::{ReplayFrame, ReplayHeader};
    use std::collections::BTreeMap;
    use std::io::Write as _;

    const TEST_HASH: &str = "0123456789ab";

    fn sample_file() -> ReplayFile {
        ReplayFile {
            header: ReplayHeader {
                mission_id: "Dem_Lei_MP".to_owned(),
                rng_seed: 0xdead_beef,
                sim_config: robin_engine::engine::SimConfig::default(),
                version: REPLAY_SCHEMA_VERSION,
                total_frames: 1,
                campaign: bitcode::encode(&robin_engine::campaign::Campaign::default()),
            },
            frames: [(
                0,
                ReplayFrame {
                    timeline_before: 0,
                    timeline_after: 1,
                    input: SimulationFrameInput::from_player_inputs(vec![PlayerInput::host(
                        PlayerCommand::CrouchDown,
                    )]),
                    host_controls: Vec::new(),
                },
            )]
            .into(),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        }
    }

    fn sample_data() -> ReplayData {
        sample_file().into()
    }

    fn envelope(hash: &str, compressed: &[u8]) -> String {
        format!("{COMPACT_PREFIX}{hash}-{}", BASE64.encode(compressed))
    }

    fn encode_file(file: &ReplayFile) -> String {
        let encoded = bitcode::encode(file);
        let compressed = zstd::encode_all(encoded.as_slice(), ZSTD_LEVEL).unwrap();
        envelope(TEST_HASH, &compressed)
    }

    fn assert_limit(error: FormatError, kind: ReplayLimitKind) {
        assert!(
            matches!(error, FormatError::LimitExceeded { kind: actual, .. } if actual == kind),
            "expected {kind:?}, got {error:?}"
        );
    }

    #[test]
    fn compact_roundtrip_is_exactly_bitcode_zstd_base64url() {
        let encoded = encode_compact(&sample_data(), TEST_HASH).unwrap();
        assert!(encoded.starts_with("rhrec-0123456789ab-"));
        assert!(!encoded.contains('='));
        let (hash, decoded) = decode_compact_bounded(&encoded, &Default::default()).unwrap();
        assert_eq!(hash, TEST_HASH);
        assert_eq!(decoded.header.mission_id, "Dem_Lei_MP");
        assert_eq!(decoded.frame_count(), 1);
    }

    #[test]
    fn transport_preflight_rejects_every_noncanonical_text_shape() {
        let valid = encode_file(&sample_file());
        for invalid in [
            format!(" {valid}"),
            format!("{valid}\n"),
            valid.replacen(TEST_HASH, "ABCDEF012345", 1),
            valid.replacen(TEST_HASH, "0123456789a", 1),
            format!("{valid}="),
            valid.replacen("rhrec-", "RHREC-", 1),
        ] {
            assert!(
                preflight_compact_transport(&invalid, &Default::default()).is_err(),
                "accepted noncanonical envelope: {invalid}"
            );
        }
    }

    #[test]
    fn disguised_jsonl_has_no_fallback_decoder() {
        let jsonl = br#"{"version":24}"#;
        let compressed = zstd::encode_all(jsonl.as_slice(), ZSTD_LEVEL).unwrap();
        assert!(matches!(
            decode_compact_bounded(&envelope(TEST_HASH, &compressed), &Default::default()),
            Err(FormatError::Bitcode(_))
        ));
    }

    #[test]
    fn alternate_valid_zstd_representation_is_rejected() {
        let encoded = bitcode::encode(&sample_file());
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), ZSTD_LEVEL).unwrap();
        encoder.include_checksum(true).unwrap();
        encoder.write_all(&encoded).unwrap();
        let compressed = encoder.finish().unwrap();
        let error = decode_compact_bounded(
            &envelope(TEST_HASH, &compressed),
            &ReplayAdmissionLimits::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            FormatError::NonCanonical {
                stage: CanonicalStage::Zstd
            }
        ));
    }

    #[test]
    fn every_binary_expansion_stage_has_an_independent_limit() {
        let valid = encode_file(&sample_file());
        for (limits, kind) in [
            (
                ReplayAdmissionLimits {
                    max_input_bytes: valid.len() - 1,
                    ..Default::default()
                },
                ReplayLimitKind::CompactInputBytes,
            ),
            (
                ReplayAdmissionLimits {
                    max_base64_payload_bytes: 1,
                    ..Default::default()
                },
                ReplayLimitKind::Base64PayloadBytes,
            ),
            (
                ReplayAdmissionLimits {
                    max_compressed_bytes: 1,
                    ..Default::default()
                },
                ReplayLimitKind::CompressedBytes,
            ),
        ] {
            assert_limit(decode_compact_bounded(&valid, &limits).unwrap_err(), kind);
        }
    }

    #[test]
    fn decompression_stops_one_byte_past_limit() {
        let compressed =
            zstd::encode_all(vec![0_u8; 2 * 1024 * 1024].as_slice(), ZSTD_LEVEL).unwrap();
        assert!(
            compressed.len() < 1024,
            "fixture must be a high-ratio stream"
        );
        let limits = ReplayAdmissionLimits {
            max_decompressed_bytes: 4096,
            ..Default::default()
        };
        assert!(matches!(
            decode_compact_bounded(&envelope(TEST_HASH, &compressed), &limits),
            Err(FormatError::LimitExceeded {
                kind: ReplayLimitKind::DecompressedBytes,
                observed: 4097,
                limit: 4096,
            })
        ));
    }

    #[test]
    fn forged_large_zstd_window_is_a_deterministic_resource_limit() {
        let encoded = bitcode::encode(&sample_file());
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
        encoder.include_contentsize(false).unwrap();
        encoder.window_log(28).unwrap();
        encoder.write_all(&encoded).unwrap();
        let compressed = encoder.finish().unwrap();
        let window = zstd_frame_window_size(&compressed).unwrap();
        assert!(window > 1 << 24, "fixture declared only {window} bytes");
        assert_limit(
            decode_compact_bounded(
                &envelope(TEST_HASH, &compressed),
                &ReplayAdmissionLimits::default(),
            )
            .unwrap_err(),
            ReplayLimitKind::ZstdWindowBytes,
        );
    }

    #[test]
    fn zero_window_log_never_selects_the_zstd_default() {
        let valid = encode_file(&sample_file());
        let limits = ReplayAdmissionLimits {
            max_zstd_window_log: 0,
            ..Default::default()
        };
        assert!(matches!(
            decode_compact_bounded(&valid, &limits),
            Err(FormatError::InvalidLimits(_))
        ));
    }

    #[test]
    fn declared_materialized_and_sparse_record_counts_are_bounded() {
        let mut declared = sample_file();
        declared.header.total_frames = 2;
        let limits = ReplayAdmissionLimits {
            max_frames: 1,
            ..Default::default()
        };
        assert_limit(
            decode_compact_bounded(&encode_file(&declared), &limits).unwrap_err(),
            ReplayLimitKind::DeclaredFrames,
        );

        let mut metadata = sample_file();
        metadata.hashes.insert(0, 42);
        let limits = ReplayAdmissionLimits {
            max_metadata_records: 0,
            ..Default::default()
        };
        assert_limit(
            decode_compact_bounded(&encode_file(&metadata), &limits).unwrap_err(),
            ReplayLimitKind::MetadataRecords,
        );
    }

    #[test]
    fn non_dense_and_out_of_range_records_fail_without_declared_range_scan() {
        let mut file = sample_file();
        let frame = file.frames.remove(&0).unwrap();
        file.frames.insert(1, frame);
        assert!(matches!(
            decode_compact_bounded(&encode_file(&file), &Default::default()),
            Err(FormatError::UnexpectedFrameOrdinal {
                expected: 0,
                actual: 1,
            })
        ));

        let mut file = sample_file();
        file.hashes.insert(1, 42);
        assert!(matches!(
            decode_compact_bounded(&encode_file(&file), &Default::default()),
            Err(FormatError::MetadataFrameOutsideReplay {
                kind: ReplayMetadataKind::StateHash,
                frame: 1,
                total_frames: 1,
            })
        ));
    }

    #[test]
    fn frame_and_total_entry_work_are_bounded() {
        let mut file = sample_file();
        file.frames
            .get_mut(&0)
            .unwrap()
            .input
            .commands
            .push(PlayerInput::host(PlayerCommand::StandUp).into());
        let per_frame = ReplayAdmissionLimits {
            max_entries_per_frame: 1,
            ..Default::default()
        };
        assert_limit(
            decode_compact_bounded(&encode_file(&file), &per_frame).unwrap_err(),
            ReplayLimitKind::FrameEntries,
        );
        let total = ReplayAdmissionLimits {
            max_total_frame_entries: 1,
            ..Default::default()
        };
        assert_limit(
            decode_compact_bounded(&encode_file(&file), &total).unwrap_err(),
            ReplayLimitKind::TotalFrameEntries,
        );
    }

    #[test]
    fn nested_collections_strings_and_total_typed_work_are_bounded() {
        let mut collection = sample_file();
        collection
            .frames
            .get_mut(&0)
            .unwrap()
            .input
            .commands
            .push(PlayerInput::host(PlayerCommand::StandUp).into());
        let limits = ReplayAdmissionLimits {
            max_typed_collection_entries: 1,
            max_entries_per_frame: usize::MAX,
            ..Default::default()
        };
        assert_limit(
            decode_compact_bounded(&encode_file(&collection), &limits).unwrap_err(),
            ReplayLimitKind::TypedCollectionEntries,
        );

        let mut string = sample_file();
        string.frames.get_mut(&0).unwrap().input.external_actions = vec![ExternalAction::Native {
            name: "long_native_name".to_owned(),
            args: Vec::new(),
            this_actor: None,
        }];
        let limits = ReplayAdmissionLimits {
            max_typed_string_bytes: 4,
            ..Default::default()
        };
        assert_limit(
            decode_compact_bounded(&encode_file(&string), &limits).unwrap_err(),
            ReplayLimitKind::TypedStringBytes,
        );

        let limits = ReplayAdmissionLimits {
            max_total_typed_work: 1,
            ..Default::default()
        };
        assert_limit(
            decode_compact_bounded(&encode_file(&sample_file()), &limits).unwrap_err(),
            ReplayLimitKind::TotalTypedWork,
        );
    }

    #[test]
    fn campaign_mission_and_schema_are_fail_closed() {
        let mut file = sample_file();
        file.header.campaign = vec![0; 17];
        let limits = ReplayAdmissionLimits {
            max_campaign_bytes: 16,
            ..Default::default()
        };
        assert_limit(
            decode_compact_bounded(&encode_file(&file), &limits).unwrap_err(),
            ReplayLimitKind::CampaignBytes,
        );

        let mut file = sample_file();
        file.header.mission_id = "mission-name".into();
        let limits = ReplayAdmissionLimits {
            max_mission_id_bytes: 4,
            ..Default::default()
        };
        assert_limit(
            decode_compact_bounded(&encode_file(&file), &limits).unwrap_err(),
            ReplayLimitKind::MissionIdBytes,
        );

        let mut file = sample_file();
        file.header.version = REPLAY_SCHEMA_VERSION - 1;
        assert!(matches!(
            decode_compact_bounded(&encode_file(&file), &Default::default()),
            Err(FormatError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn embedded_campaign_is_typed_canonical_and_semantically_valid() {
        let mut malformed = sample_file();
        malformed.header.campaign = vec![0xff];
        assert!(matches!(
            decode_compact_bounded(&encode_file(&malformed), &Default::default()),
            Err(FormatError::CampaignBitcode(_))
        ));

        let mut alternate = sample_file();
        alternate.header.campaign.extend_from_slice(&[0, 0, 0, 0]);
        assert!(matches!(
            decode_compact_bounded(&encode_file(&alternate), &Default::default()),
            Err(FormatError::NonCanonicalCampaign) | Err(FormatError::CampaignBitcode(_))
        ));

        let mut invalid_history = robin_engine::campaign::Campaign::default();
        invalid_history.mission_attempt_sequence = 1;
        let mut file = sample_file();
        file.header.campaign = bitcode::encode(&invalid_history);
        assert!(matches!(
            decode_compact_bounded(&encode_file(&file), &Default::default()),
            Err(FormatError::InvalidCampaignHistory(_))
        ));
    }

    #[test]
    fn required_build_is_rejected_before_payload_decode() {
        let malformed_payload = format!("rhrec-{TEST_HASH}-AA");
        let error =
            decode_compact_for_build(&malformed_payload, &Default::default(), "abcdef012345")
                .unwrap_err();
        assert!(matches!(error, FormatError::EngineVersionMismatch { .. }));
    }

    #[test]
    fn canonical_encoder_window_fits_the_output_bound() {
        let valid = encode_file(&sample_file());
        let preflight = preflight_compact_transport(&valid, &Default::default()).unwrap();
        let compressed = BASE64.decode(preflight.base64_payload).unwrap();
        let window = zstd_frame_window_size(&compressed).unwrap();
        assert!(window <= 1 << DEFAULT_REPLAY_ADMISSION_LIMITS.max_zstd_window_log);
        assert!(window <= DEFAULT_REPLAY_ADMISSION_LIMITS.max_decompressed_bytes);
    }
}
