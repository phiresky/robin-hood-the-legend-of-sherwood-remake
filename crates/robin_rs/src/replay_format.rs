//! Game-facing replay loading facade.
//!
//! The canonical codec and server/verifier admission contract live in
//! [`robin_replay_format`]. This module adds two application-only concerns:
//!
//! - local developer JSONL/path loading, visibly separate from production;
//! - isolated public compact validation before the game process/wasm instance
//!   decodes the exact bytes a second time.

pub use robin_replay_format::*;

#[cfg(not(target_arch = "wasm32"))]
mod native;

/// Exact source commit used by multiplayer artifact selection.
pub const ENGINE_SOURCE_COMMIT: &str = env!("ROBIN_GIT_COMMIT");

/// Validate the game-runtime part of the exact executable replay contract.
///
/// The format crate validates the canonical wire graph and package digest. The
/// game facade additionally rejects a structurally valid package authored for
/// another Spellforge VM ABI before it reaches playback.
fn validate_spellforge_runtime_package(
    data: &robin_engine::replay::ReplayData,
) -> Result<(), robin_replay_format::FormatError> {
    if let Some(package) = &data.header().spellforge_package {
        robin_spellforge::validate_package(package).map_err(|error| {
            robin_replay_format::FormatError::InvalidLayout(format!(
                "invalid replay Spellforge package: {error}"
            ))
        })?;
    }
    Ok(())
}

/// Validate both the canonical replay layout and this build's exact
/// Spellforge executable ABI.
pub fn validate_replay_data(
    data: &robin_engine::replay::ReplayData,
) -> Result<(), robin_replay_format::FormatError> {
    robin_replay_format::validate_replay_data(data)?;
    validate_spellforge_runtime_package(data)
}

/// Encode only replays whose embedded executable package can run in this
/// build's Spellforge VM.
pub fn encode_compact(
    data: &robin_engine::replay::ReplayData,
    hash: &str,
) -> Result<String, robin_replay_format::FormatError> {
    validate_replay_data(data)?;
    robin_replay_format::encode_compact(data, hash)
}

/// Decode a trusted compact replay and apply the game-runtime executable gate.
pub fn decode_compact(
    text: &str,
) -> Result<(String, robin_engine::replay::ReplayData), robin_replay_format::FormatError> {
    let decoded = robin_replay_format::trusted_local::decode(text)?;
    validate_spellforge_runtime_package(&decoded.1)?;
    Ok(decoded)
}

/// Decode the current public format under the canonical resource limits and
/// reject packages targeting another Spellforge VM ABI.
pub fn decode_compact_for_admission(
    text: &str,
) -> Result<(String, robin_engine::replay::ReplayData), robin_replay_format::FormatError> {
    let decoded = robin_replay_format::contained_worker::admit(text)?;
    validate_spellforge_runtime_package(&decoded.1)?;
    Ok(decoded)
}

/// Decode the bounded local-custom lane after/inside isolated containment.
pub fn decode_compact_for_local_playback(
    text: &str,
) -> Result<(String, robin_engine::replay::ReplayData), robin_replay_format::FormatError> {
    let decoded = robin_replay_format::decode_compact_for_local_playback(text)?;
    validate_spellforge_runtime_package(&decoded.1)?;
    Ok(decoded)
}

#[derive(Debug, thiserror::Error)]
pub enum ReplayLoadError {
    #[error(transparent)]
    Compact(#[from] robin_replay_format::FormatError),
    #[error("local replay I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[cfg(not(target_arch = "wasm32"))]
    #[error("local JSONL replay decode failed: {0}")]
    LocalJsonl(String),
    #[cfg(not(target_arch = "wasm32"))]
    #[error("isolated replay admission rejected the artifact: {0}")]
    AdmissionRejected(String),
    #[cfg(not(target_arch = "wasm32"))]
    #[error("replay admission exhausted its {stage} resource limit: {detail}")]
    ResourceLimit { stage: &'static str, detail: String },
    #[cfg(not(target_arch = "wasm32"))]
    #[error("this platform cannot securely contain replay admission: {0}")]
    ContainmentUnavailable(String),
    #[cfg(not(target_arch = "wasm32"))]
    #[error("isolated replay admission protocol failed: {0}")]
    WorkerProtocol(String),
    #[cfg(target_arch = "wasm32")]
    #[error("browser compact replay was not validated by the isolated Web Worker")]
    BrowserWorkerValidationRequired,
    #[cfg(target_arch = "wasm32")]
    #[error("browser replay loading accepts only an inline compact replay")]
    BrowserCompactOnly,
}

/// Load user-supplied compact bytes only after an isolated validator has
/// accepted the exact digest. Native uses a short-lived child process with
/// OS memory/CPU limits. Browser builds require the shell's dedicated Worker
/// to install a one-shot digest proof before this call.
pub fn decode_compact_for_public_playback(
    text: &str,
) -> Result<(String, robin_engine::replay::ReplayData), ReplayLoadError> {
    AdmittedReplay::admit(text)?.decode()
}

/// Process-local capability for these exact immutable bytes. Deliberately
/// neither serializable nor publicly constructible: a deserialized digest is
/// not evidence that the isolated worker ran.
struct AdmittedReplay<'a> {
    text: &'a str,
}

impl<'a> AdmittedReplay<'a> {
    fn admit(text: &'a str) -> Result<Self, ReplayLoadError> {
        #[cfg(not(target_arch = "wasm32"))]
        native::validate_in_native_child(text)?;
        #[cfg(target_arch = "wasm32")]
        consume_browser_worker_proof(text)?;
        Ok(Self { text })
    }

    // The isolated worker validated and canonically re-encoded these exact
    // immutable bytes. Repeating typed decode here is safe: collection/string
    // sizes and total work were already proven under external containment.
    fn decode(self) -> Result<(String, robin_engine::replay::ReplayData), ReplayLoadError> {
        decode_compact_for_local_playback(self.text).map_err(Into::into)
    }
}

/// Explicitly local CLI/developer loader. Production network/server code must
/// call the canonical crate's bounded admission API and has no JSONL branch.
pub fn load_replay_spec(spec: &str) -> Result<robin_engine::replay::ReplayData, ReplayLoadError> {
    if spec.starts_with(COMPACT_PREFIX) {
        return decode_compact_for_public_playback(spec).map(|(_, replay)| replay);
    }

    #[cfg(target_arch = "wasm32")]
    return Err(ReplayLoadError::BrowserCompactOnly);

    #[cfg(not(target_arch = "wasm32"))]
    {
        // JSONL is a visibly named, trusted local developer lane. It is never
        // auto-detected from bytes and is unavailable to network admission.
        let path = std::path::Path::new(spec);
        let archive = if path.is_dir() {
            Some(path)
        } else {
            path.parent()
                .filter(|parent| parent.join("mission.json").is_file())
        };
        if let Some(directory) = archive {
            let replay = if path.is_dir() {
                crate::replay_archive::load_directory(directory)
            } else {
                crate::replay_archive::load_through_chunk(path)
            }
            .map_err(|error| ReplayLoadError::LocalJsonl(format!("{error:#}")))?;
            validate_replay_data(&replay)?;
            return Ok(replay);
        }
        if spec.ends_with(".rhrec.jsonl") {
            let replay = robin_engine::replay::ReplayData::from_file(spec)
                .map_err(ReplayLoadError::LocalJsonl)?;
            validate_replay_data(&replay)?;
            return Ok(replay);
        }

        // Every other path is a production compact artifact. Bound acquisition
        // before allocating a String so hostile local/URL-derived paths cannot
        // bypass the codec's transport preflight with `read_to_string`.
        use std::io::Read as _;
        let limit = LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS.max_input_bytes;
        let file = std::fs::File::open(spec)?;
        if file.metadata()?.len() > u64::try_from(limit).unwrap_or(u64::MAX) {
            return Err(robin_replay_format::FormatError::LimitExceeded {
                kind: ReplayLimitKind::CompactInputBytes,
                observed: limit.saturating_add(1),
                limit,
            }
            .into());
        }
        let mut bytes = Vec::with_capacity(
            usize::try_from(file.metadata()?.len())
                .unwrap_or(limit)
                .min(limit),
        );
        file.take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(robin_replay_format::FormatError::LimitExceeded {
                kind: ReplayLimitKind::CompactInputBytes,
                observed: bytes.len(),
                limit,
            }
            .into());
        }
        let contents = std::str::from_utf8(&bytes).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("compact replay is not UTF-8: {error}"),
            )
        })?;
        decode_compact_for_public_playback(contents).map(|(_, replay)| replay)
    }
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static BROWSER_WORKER_PROOF: std::cell::RefCell<Option<[u8; 32]>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Install a one-shot digest after the shell's isolated wasm Worker accepted
/// the exact canonical compact replay. This performs transport preflight only;
/// it never zstd/bitcode-decodes in the main wasm instance.
#[cfg(target_arch = "wasm32")]
pub fn mark_browser_worker_validated(text: &str) -> Result<(), ReplayLoadError> {
    use sha2::Digest as _;

    preflight_compact_transport(text, &LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS)?;
    let digest: [u8; 32] = sha2::Sha256::digest(text.as_bytes()).into();
    BROWSER_WORKER_PROOF.with(|proof| *proof.borrow_mut() = Some(digest));
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn consume_browser_worker_proof(text: &str) -> Result<(), ReplayLoadError> {
    use sha2::Digest as _;

    let digest: [u8; 32] = sha2::Sha256::digest(text.as_bytes()).into();
    let accepted = BROWSER_WORKER_PROOF.with(|proof| proof.borrow_mut().take()) == Some(digest);
    if accepted {
        Ok(())
    } else {
        Err(ReplayLoadError::BrowserWorkerValidationRequired)
    }
}
