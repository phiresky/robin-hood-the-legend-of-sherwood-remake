//! Minimal, memory-capped browser replay admission module.
//!
//! This is intentionally not part of the game wasm module. The shell loads it
//! in a short-lived Dedicated Worker whose separate linear memory declares a
//! hard 384 MiB maximum. CI inspects that declaration after wasm-bindgen and
//! optimization; a missing/shared/imported/oversized memory fails publishing.

#[wasm_bindgen::prelude::wasm_bindgen]
pub fn validate_compact_replay(compact: &str) -> Result<(), wasm_bindgen::JsValue> {
    robin_replay_format::decode_compact_for_build(
        compact,
        &robin_replay_format::LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS,
        robin_replay_format::ENGINE_VERSION_HASH,
    )
    .map(|_| ())
    .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
}
