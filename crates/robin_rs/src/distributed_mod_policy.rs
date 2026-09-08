//! Platform-independent admission transitions and cache bounds.
//! Storage adapters retain durable writes, leases and browser scheduling.
use crate::distributed_mod::DISTRIBUTED_MOD_ENCODED_LIMIT;
use serde::{Deserialize, Serialize};

pub const DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION: u32 = 1;
pub const DISTRIBUTED_MOD_CACHE_ENTRY_LIMIT: usize = 64;
pub const DISTRIBUTED_MOD_CACHE_BYTE_LIMIT: u64 = 512 * 1024 * 1024;
pub const DISTRIBUTED_MOD_TRANSFER_CHUNK_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CacheIndexEntry {
    pub encoded_bytes: u64,
    pub last_used: u64,
}

pub(crate) fn validate_transfer_total(total_bytes: u64) -> Result<(), String> {
    if total_bytes == 0 || total_bytes > DISTRIBUTED_MOD_ENCODED_LIMIT as u64 {
        return Err(format!(
            "distributed-mod transfer declares {total_bytes} bytes; expected 1..={DISTRIBUTED_MOD_ENCODED_LIMIT}"
        ));
    }
    Ok(())
}

/// Cache metadata is never authority: validate the canonical envelope on every read.
pub(crate) fn validate_cached_package(
    encoded: &[u8],
    expected_hash: [u8; 32],
    declared_bytes: u64,
) -> Result<crate::distributed_mod::ValidatedDistributedMod, String> {
    validate_transfer_total(declared_bytes)?;
    if encoded.len() as u64 != declared_bytes {
        return Err(format!(
            "cached distributed mod is {} bytes; index declares {declared_bytes}",
            encoded.len()
        ));
    }
    let validated = crate::distributed_mod::DistributedModPackage::decode(encoded)
        .map_err(|error| format!("validate cached distributed mod: {error}"))?;
    if validated.package.manifest.full_mod_sha256 != expected_hash {
        return Err("cached distributed mod has a different embedded hash".to_owned());
    }
    Ok(validated)
}

pub(crate) fn validate_chunk(total: u64, offset: u64, length: usize) -> Result<u64, String> {
    validate_transfer_total(total)?;
    if length == 0 || length > DISTRIBUTED_MOD_TRANSFER_CHUNK_LIMIT {
        return Err(format!(
            "distributed-mod chunk is {length} bytes; expected 1..={DISTRIBUTED_MOD_TRANSFER_CHUNK_LIMIT}"
        ));
    }
    let end = offset
        .checked_add(length as u64)
        .ok_or_else(|| "distributed-mod chunk offset overflow".to_owned())?;
    if end > total {
        return Err(format!(
            "distributed-mod chunk [{offset},{end}) exceeds declared total {total}"
        ));
    }
    Ok(end)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum AdmissionPhase {
    Receiving,
    Validated,
    Mounted,
}

/// State advances only after the adapter completes its durable operation.
/// Even an entirely cached package starts in Receiving and needs validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AdmissionState {
    hash: [u8; 32],
    total: u64,
    durable_offset: u64,
    phase: AdmissionPhase,
}

impl AdmissionState {
    pub fn new(hash: [u8; 32], total: u64, durable_offset: u64) -> Result<Self, String> {
        validate_transfer_total(total)?;
        if durable_offset > total {
            return Err("cache resume offset exceeds offered content length".to_owned());
        }
        Ok(Self {
            hash,
            total,
            durable_offset,
            phase: AdmissionPhase::Receiving,
        })
    }
    pub fn check_chunk(
        &self,
        hash: [u8; 32],
        total: u64,
        offset: u64,
        length: usize,
    ) -> Result<(), String> {
        if self.phase != AdmissionPhase::Receiving
            || hash != self.hash
            || total != self.total
            || offset != self.durable_offset
        {
            return Err(format!(
                "transport delivered mismatched content chunk at {offset}/{total}; expected {}/{}",
                self.durable_offset, self.total
            ));
        }
        validate_chunk(total, offset, length)?;
        Ok(())
    }
    pub fn committed(&mut self, offset: u64) -> Result<(), String> {
        if self.phase != AdmissionPhase::Receiving
            || offset <= self.durable_offset
            || offset > self.total
        {
            return Err("cache returned an invalid durable content offset".to_owned());
        }
        self.durable_offset = offset;
        Ok(())
    }
    pub fn validated(&mut self) -> Result<(), String> {
        if self.phase != AdmissionPhase::Receiving || self.durable_offset != self.total {
            return Err("cannot validate incomplete content transfer".to_owned());
        }
        self.phase = AdmissionPhase::Validated;
        Ok(())
    }
    pub fn mounted(&mut self) -> Result<(), String> {
        if self.phase != AdmissionPhase::Validated {
            return Err("cannot mount content before package validation".to_owned());
        }
        self.phase = AdmissionPhase::Mounted;
        Ok(())
    }
    pub fn acknowledge(&self) -> Result<(), String> {
        if self.phase != AdmissionPhase::Mounted {
            return Err("cannot acknowledge content before mounting".to_owned());
        }
        Ok(())
    }
}

/// Pure LRU selection. Adapters supply platform-specific pins and staged bytes,
/// then publish the resulting index with their own storage transaction.
pub(crate) fn select_evictions(
    entries: &std::collections::BTreeMap<String, CacheIndexEntry>,
    protected: &std::collections::BTreeSet<String>,
    staged_bytes: u64,
) -> Result<Vec<String>, String> {
    let mut total = entries
        .values()
        .map(|entry| entry.encoded_bytes)
        .try_fold(staged_bytes, u64::checked_add)
        .ok_or_else(|| "distributed-mod cache byte accounting overflow".to_owned())?;
    let mut count = entries.len();
    let mut candidates: Vec<_> = entries
        .iter()
        .filter(|(hash, _)| !protected.contains(*hash))
        .collect();
    candidates.sort_by_key(|(hash, entry)| (entry.last_used, *hash));
    let mut evicted = Vec::new();
    for (hash, entry) in candidates {
        if count <= DISTRIBUTED_MOD_CACHE_ENTRY_LIMIT && total <= DISTRIBUTED_MOD_CACHE_BYTE_LIMIT {
            break;
        }
        total -= entry.encoded_bytes;
        count -= 1;
        evicted.push(hash.clone());
    }
    if count > DISTRIBUTED_MOD_CACHE_ENTRY_LIMIT || total > DISTRIBUTED_MOD_CACHE_BYTE_LIMIT {
        return Err(
            "distributed-mod cache exceeds its limits with no unprotected complete entry to evict"
                .to_owned(),
        );
    }
    Ok(evicted)
}

pub(crate) fn check_consent(
    offer: &robin_engine::multiplayer::DistributedModOffer,
    spellforge_enabled: bool,
    trusted: bool,
) -> Result<(), String> {
    if offer.spellforge_package_sha256.is_some() && !spellforge_enabled {
        return Err("host content requires Spellforge, but Allow Spellforge Missions is disabled in Gameplay settings".to_owned());
    }
    if !trusted {
        return Err(format!(
            "host content {} is not approved for this player; review it in the multiplayer lobby",
            robin_engine::spellforge::hex_hash(&offer.full_mod_sha256)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_content_still_requires_consent_and_spellforge_setting() {
        let mut offer = robin_engine::multiplayer::DistributedModOffer {
            schema_version: 1,
            full_mod_sha256: [1; 32],
            spellforge_package_sha256: Some([2; 32]),
            spellforge_vm_abi: Some("test".into()),
            encoded_bytes: 4,
            mission_basename: "mission".into(),
            mission_rhm_entry: "Data/Levels/mission.rhm".into(),
            map_filename: "map".into(),
            title: "title".into(),
            claimed_author: "author".into(),
            version: "1".into(),
            source_url: "https://example.invalid".into(),
            license: "CC0-1.0".into(),
            host_endpoint_id: "host".into(),
        };
        let cached = AdmissionState::new(offer.full_mod_sha256, 4, 4).unwrap();
        assert!(cached.acknowledge().is_err());
        assert!(check_consent(&offer, true, false).is_err());
        assert!(check_consent(&offer, false, true).is_err());
        assert!(check_consent(&offer, true, true).is_ok());
        offer.spellforge_package_sha256 = None;
        assert!(check_consent(&offer, false, false).is_err());
        assert!(check_consent(&offer, false, true).is_ok());
    }
    #[test]
    fn eviction_preserves_pins_and_accounts_for_staging_without_mutating_index() {
        use std::collections::{BTreeMap, BTreeSet};
        let entries = BTreeMap::from([
            (
                "a".to_owned(),
                CacheIndexEntry {
                    encoded_bytes: DISTRIBUTED_MOD_CACHE_BYTE_LIMIT / 2,
                    last_used: 1,
                },
            ),
            (
                "b".to_owned(),
                CacheIndexEntry {
                    encoded_bytes: DISTRIBUTED_MOD_CACHE_BYTE_LIMIT / 2,
                    last_used: 1,
                },
            ),
        ]);
        assert_eq!(
            select_evictions(&entries, &BTreeSet::new(), 1).unwrap(),
            vec!["a"]
        );
        assert_eq!(
            select_evictions(&entries, &BTreeSet::from(["a".to_owned()]), 1).unwrap(),
            vec!["b"]
        );
        assert!(select_evictions(&entries, &entries.keys().cloned().collect(), 1).is_err());
        assert!(select_evictions(&entries, &BTreeSet::new(), u64::MAX).is_err());
        assert_eq!(entries.len(), 2);
    }
    #[test]
    fn both_platforms_follow_the_same_transfer_scenarios() {
        // Native and browser adapters use the same durable-prefix contract;
        // their synchronous vs asynchronous writes do not alter transitions.
        for resume in [0, 2, 4] {
            let mut state = AdmissionState::new([1; 32], 4, resume).unwrap();
            assert!(state.acknowledge().is_err());
            assert!(state.mounted().is_err());
            if resume < 4 {
                assert!(state.validated().is_err());
                assert!(state.check_chunk([2; 32], 4, resume, 1).is_err());
                assert!(state.check_chunk([1; 32], 5, resume, 1).is_err());
                assert!(state.check_chunk([1; 32], 4, resume + 1, 1).is_err());
                state
                    .check_chunk([1; 32], 4, resume, (4 - resume) as usize)
                    .unwrap();
                state.committed(4).unwrap();
            }
            state.validated().unwrap();
            assert!(state.acknowledge().is_err());
            state.mounted().unwrap();
            state.acknowledge().unwrap();
            assert!(state.check_chunk([1; 32], 4, 4, 1).is_err());
        }
    }
    #[test]
    fn cache_bounds_reject_empty_oversized_and_overflowing_chunks() {
        assert!(validate_chunk(4, 0, 0).is_err());
        assert!(validate_chunk(4, 4, 1).is_err());
        assert!(validate_chunk(4, u64::MAX, 1).is_err());
        assert!(validate_chunk(4, 0, DISTRIBUTED_MOD_TRANSFER_CHUNK_LIMIT + 1).is_err());
        assert!(AdmissionState::new([1; 32], 4, 5).is_err());
        assert_eq!(validate_chunk(4, 2, 2).unwrap(), 4);
    }
}
