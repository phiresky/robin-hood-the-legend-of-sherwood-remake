//! Minimal, memory-capped browser replay admission module.
//!
//! This is intentionally not part of the game wasm module. The shell loads it
//! in a short-lived Dedicated Worker whose separate linear memory declares a
//! hard 384 MiB maximum. CI inspects that declaration after wasm-bindgen and
//! optimization; a missing/shared/imported/oversized memory fails publishing.

#[wasm_bindgen::prelude::wasm_bindgen]
pub fn validate_replay_seek_sidecar(
    compact: &[u8],
    sidecar: &[u8],
) -> Result<(), wasm_bindgen::JsValue> {
    use sha2::{Digest, Sha256};
    let result = (|| -> Result<(), String> {
        let (_, replay) = robin_replay_format::decode_compact_bounded(
            compact,
            &robin_replay_format::LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS,
        )
        .map_err(|e| e.to_string())?;
        robin_replay_format::seek::ReplaySeekSidecar::decode(
            sidecar,
            Sha256::digest(compact).into(),
            &replay,
        )?
        .validate_engines()
    })();
    result.map_err(|error| wasm_bindgen::JsValue::from_str(&error))
}

#[wasm_bindgen::prelude::wasm_bindgen]
pub fn validate_compact_replay(compact: &[u8]) -> Result<(), wasm_bindgen::JsValue> {
    // The recorded source hash is provenance; this validates schema and limits.
    robin_replay_format::decode_compact_bounded(
        compact,
        &robin_replay_format::LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS,
    )
    .map(|_| ())
    .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
}
