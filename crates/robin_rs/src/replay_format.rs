//! Compact replay sharing format: `rhrec-{versionhash}-{base64}`.
//!
//! Designed to be as small as possible so a finished replay fits in a
//! URL / bug-report paste: the engine's `ReplayData` is flattened into
//! a [`ReplayFile`], bitcode-serialized, zstd-compressed, then
//! base64-encoded (URL-safe, no padding). A short engine git hash is
//! prepended so the loader can reject a replay produced on a
//! different build.
//!
//! The canonical recording format on disk is still JSONL (see
//! [`robin_engine::replay`]) — that streams incrementally and is
//! crash-safe. This module converts between the two: encode produces a
//! sharing string from an in-memory replay, decode parses one back.
//!
//! # Acceptance from the CLI / JSON API
//!
//! [`load_replay_spec`] accepts three flavours:
//!
//! 1. A bare `rhrec-…` string (the compact format, inline).
//! 2. A filesystem path to a file whose contents are a `rhrec-…`
//!    string (any extension; convenient for shell redirection).
//! 3. A filesystem path to a legacy `*.rhrec.jsonl` file.
//!
//! The version hash is checked on decode; mismatches are rejected because
//! state hashes and deterministic behavior are build-specific contracts.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64;
use robin_engine::replay::{REPLAY_SCHEMA_VERSION, ReplayData, ReplayFile};

/// Git short-hash of the engine at build time (see `build.rs`).
/// Falls back to `"unknown"` when built outside a git checkout.
pub const ENGINE_VERSION_HASH: &str = env!("ROBIN_GIT_HASH");

/// Prefix byte sequence for the compact format; also the file-format
/// magic when a `rhrec-…` string is written to disk.
pub const COMPACT_PREFIX: &str = "rhrec-";

/// Zstd compression level. 19 is near-max ratio; replays are tiny so
/// the extra CPU cost is invisible compared to a mission run.
const ZSTD_LEVEL: i32 = 19;

/// Error from compact-format encode / decode.
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("missing `rhrec-` prefix")]
    MissingPrefix,
    #[error("missing version/payload separator")]
    MissingSeparator,
    #[error("base64 decode failed: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("zstd decode failed: {0}")]
    Zstd(std::io::Error),
    #[error("bitcode decode failed: {0}")]
    Bitcode(#[from] bitcode::Error),
    #[error("bitcode encode failed: {0}")]
    Encode(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("jsonl decode failed: {0}")]
    Jsonl(String),
    #[error(
        "unsupported replay schema version {version}; supported version is {REPLAY_SCHEMA_VERSION}"
    )]
    UnsupportedVersion { version: u32 },
    #[error("replay engine hash `{actual}` does not match this build `{expected}`")]
    EngineHashMismatch { actual: String, expected: String },
}

/// Encode an in-memory [`ReplayData`] as a `rhrec-{hash}-{base64}`
/// string. `hash` is the engine version tag to stamp into the output;
/// callers normally pass [`ENGINE_VERSION_HASH`].
pub fn encode_compact(data: &ReplayData, hash: &str) -> Result<String, FormatError> {
    let file: ReplayFile = data.into();
    let bytes = bitcode::serialize(&file).map_err(|e| FormatError::Encode(format!("{e}")))?;
    let zbytes = zstd::encode_all(&bytes[..], ZSTD_LEVEL).map_err(FormatError::Zstd)?;
    let b64 = BASE64.encode(&zbytes);
    Ok(format!("{COMPACT_PREFIX}{hash}-{b64}"))
}

/// Parse a `rhrec-{hash}-{base64}` string back into a [`ReplayData`],
/// returning the embedded version hash so the caller can compare it
/// against [`ENGINE_VERSION_HASH`] and reject a mismatch.
pub fn decode_compact(text: &str) -> Result<(String, ReplayData), FormatError> {
    let rest = text
        .trim()
        .strip_prefix(COMPACT_PREFIX)
        .ok_or(FormatError::MissingPrefix)?;
    let (hash, payload) = rest.split_once('-').ok_or(FormatError::MissingSeparator)?;
    let zbytes = BASE64.decode(payload.as_bytes())?;
    let bytes = zstd::decode_all(&zbytes[..]).map_err(FormatError::Zstd)?;
    let file: ReplayFile = bitcode::deserialize(&bytes)?;
    let data = file.into();
    validate_replay_data(&data)?;
    Ok((hash.to_string(), data))
}

/// Reject replay schemas that this build cannot interpret reliably.
///
/// Serde alone cannot reject a header whose payload happens to deserialize
/// into today's structs but belongs to a different hash contract.
pub fn validate_replay_data(data: &ReplayData) -> Result<(), FormatError> {
    if data.header.version != REPLAY_SCHEMA_VERSION {
        return Err(FormatError::UnsupportedVersion {
            version: data.header.version,
        });
    }
    Ok(())
}

/// Load a replay from either the compact inline format, a file
/// containing the compact format, or a `*.rhrec.jsonl` file.
///
/// Compact replays whose engine hash differs from this build are rejected.
pub fn load_replay_spec(spec: &str) -> Result<ReplayData, FormatError> {
    // Inline `rhrec-…` wins first so `--replay rhrec-…` works without
    // shell escaping headaches on a token that might also look like a
    // relative path.
    if spec.trim_start().starts_with(COMPACT_PREFIX) {
        let (hash, data) = decode_compact(spec)?;
        require_matching_hash(&hash)?;
        return Ok(data);
    }
    // Otherwise read the file. If its contents start with `rhrec-`,
    // treat it as a dumped compact string; otherwise try JSONL.
    let contents = std::fs::read_to_string(spec)?;
    let trimmed = contents.trim_start();
    if trimmed.starts_with(COMPACT_PREFIX) {
        let (hash, data) = decode_compact(trimmed)?;
        require_matching_hash(&hash)?;
        Ok(data)
    } else {
        let data = ReplayData::from_file(spec).map_err(FormatError::Jsonl)?;
        validate_replay_data(&data)?;
        Ok(data)
    }
}

fn require_matching_hash(hash: &str) -> Result<(), FormatError> {
    if hash != ENGINE_VERSION_HASH {
        return Err(FormatError::EngineHashMismatch {
            actual: hash.to_string(),
            expected: ENGINE_VERSION_HASH.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::player_command::{PlayerCommand, PlayerInput};
    use robin_engine::replay::{ReplayFile, ReplayHeader};
    use std::collections::BTreeMap;

    fn sample_data() -> ReplayData {
        let mut frames = BTreeMap::new();
        frames.insert(0, vec![PlayerInput::host(PlayerCommand::CrouchDown)]);
        frames.insert(
            7,
            vec![
                PlayerInput::host(PlayerCommand::SelectAllPcs),
                PlayerInput::host(PlayerCommand::CrouchDown),
            ],
        );
        let hashes = BTreeMap::new();
        ReplayFile {
            header: ReplayHeader {
                mission_id: "Dem_Lei_MP".into(),
                rng_seed: 0xdead_beef,
                sim_config: robin_engine::engine::SimConfig::default(),
                version: REPLAY_SCHEMA_VERSION,
                total_frames: 8,
                campaign: vec![1, 2, 3, 4],
            },
            frames,
            hashes,
        }
        .into()
    }

    #[test]
    fn compact_roundtrip() {
        let data = sample_data();
        let s = encode_compact(&data, "abc123").unwrap();
        assert!(s.starts_with("rhrec-abc123-"));
        let (hash, back) = decode_compact(&s).unwrap();
        assert_eq!(hash, "abc123");
        assert_eq!(back.header.mission_id, "Dem_Lei_MP");
        assert_eq!(back.header.rng_seed, 0xdead_beef);
        assert_eq!(back.header.campaign, vec![1, 2, 3, 4]);
        assert_eq!(back.frame_count(), 8);
        assert_eq!(back.commands_for_frame(0).len(), 1);
        assert_eq!(back.commands_for_frame(7).len(), 2);
    }

    #[test]
    fn compact_prefix_required() {
        assert!(matches!(
            decode_compact("not-a-replay"),
            Err(FormatError::MissingPrefix)
        ));
    }

    #[test]
    fn load_spec_accepts_inline_string() {
        let data = sample_data();
        let s = encode_compact(&data, ENGINE_VERSION_HASH).unwrap();
        let back = load_replay_spec(&s).unwrap();
        assert_eq!(back.header.mission_id, "Dem_Lei_MP");
    }

    #[test]
    fn load_spec_rejects_a_different_engine_hash() {
        let data = sample_data();
        // Compact hashes use `-` as their field separator; production hashes
        // are hexadecimal git IDs, so keep the deliberately wrong fixture in
        // that same token domain.
        let s = encode_compact(&data, "deadbeefdeadbeef").unwrap();
        assert!(matches!(
            load_replay_spec(&s),
            Err(FormatError::EngineHashMismatch { .. })
        ));
    }
}
