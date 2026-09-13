//! Platform-selected durable cache implementation.
//!
//! The index header (schema version, LRU access counter, complete entries)
//! and the hash-key parser are shared; each backend adds its own index
//! extension through [`CacheIndex::extension`].
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
pub use browser::*;
#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

// `DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION` arrives through the backend glob
// re-export above; importing it again here would shadow that public name.
use crate::distributed_mod_policy::CacheIndexEntry;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Persisted cache index. Serialized as one flat JSON object: the shared
/// header fields followed by the backend extension's fields (native: none,
/// browser: `partials`), exactly matching the pre-unification layouts.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheIndex<Extension> {
    schema_version: u32,
    access_counter: u64,
    entries: BTreeMap<String, CacheIndexEntry>,
    #[serde(flatten)]
    extension: Extension,
}

impl<Extension: Default> Default for CacheIndex<Extension> {
    fn default() -> Self {
        Self {
            schema_version: DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION,
            access_counter: 0,
            entries: BTreeMap::new(),
            extension: Extension::default(),
        }
    }
}

/// Parse a cache index key. Existing indexes accept either hex case; unlike
/// wire digests, this parser must not silently become lowercase-only.
fn parse_hash(value: &str) -> Result<[u8; 32], String> {
    let mut hash = [0u8; 32];
    hex::decode_to_slice(value, &mut hash)
        .map_err(|_| format!("invalid distributed-mod cache hash `{value}`"))?;
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hash_parser_preserves_case_compatibility_and_rejects_malformed_input() {
        let hash = std::array::from_fn(|index| index as u8 * 7);
        let lower = robin_engine::spellforge::hex_hash(&hash);
        assert_eq!(parse_hash(&lower).unwrap(), hash);
        assert_eq!(parse_hash(&lower.to_ascii_uppercase()).unwrap(), hash);
        let mixed = lower[..32].to_ascii_uppercase() + &lower[32..];
        assert_eq!(parse_hash(&mixed).unwrap(), hash);
        for invalid in [
            String::new(),
            "0".repeat(63),
            "0".repeat(65),
            "g".repeat(64),
            "é".repeat(32),
        ] {
            assert_eq!(
                parse_hash(&invalid).unwrap_err(),
                format!("invalid distributed-mod cache hash `{invalid}`")
            );
        }
    }

    /// Stand-in with the same shape as the browser extension so the flattened
    /// layout is exercised on every target.
    #[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
    struct PartialsExtension {
        partials: BTreeMap<String, u64>,
    }

    #[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
    struct NoExtension {}

    #[test]
    fn flattened_index_matches_the_legacy_flat_json_layouts() {
        let key = "ab".repeat(32);
        let mut native = CacheIndex::<NoExtension>::default();
        native.access_counter = 7;
        native.entries.insert(
            key.clone(),
            CacheIndexEntry {
                encoded_bytes: 11,
                last_used: 3,
            },
        );
        let native_json = format!(
            r#"{{"schema_version":{DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION},"access_counter":7,"entries":{{"{key}":{{"encoded_bytes":11,"last_used":3}}}}}}"#
        );
        assert_eq!(serde_json::to_string(&native).unwrap(), native_json);
        let reparsed: CacheIndex<NoExtension> = serde_json::from_str(&native_json).unwrap();
        assert_eq!(serde_json::to_string(&reparsed).unwrap(), native_json);

        let mut browser = CacheIndex::<PartialsExtension>::default();
        browser.extension.partials.insert(key.clone(), 5);
        let browser_json = format!(
            r#"{{"schema_version":{DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION},"access_counter":0,"entries":{{}},"partials":{{"{key}":5}}}}"#
        );
        assert_eq!(serde_json::to_string(&browser).unwrap(), browser_json);
        let reparsed: CacheIndex<PartialsExtension> = serde_json::from_str(&browser_json).unwrap();
        assert_eq!(reparsed.extension, browser.extension);
        // A browser index without its extension field stays rejected.
        let missing = format!(
            r#"{{"schema_version":{DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION},"access_counter":0,"entries":{{}}}}"#
        );
        assert!(serde_json::from_str::<CacheIndex<PartialsExtension>>(&missing).is_err());
    }
}
