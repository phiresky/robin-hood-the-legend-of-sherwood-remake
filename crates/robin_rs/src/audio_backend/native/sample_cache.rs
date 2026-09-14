//! Decoded sample residency, separate from mixer channels and device lifetime.
use crate::byte_budget_lru::ByteBudgetLru;
use kira::sound::static_sound::StaticSoundData;
use serde::{Deserialize, Serialize};

/// Cache retention is bounded; active Kira voices own independent Arc clones.
/// Eviction releases cache ownership without interrupting playback.
/// Diagnostic serialization retains only the configured budget; restored caches
/// begin empty and cannot recreate mixer resources or extend voice lifetimes.
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct SampleCache {
    samples: ByteBudgetLru<String, StaticSoundData>,
}

impl SampleCache {
    pub(super) fn new(budget_bytes: u64) -> Self {
        Self {
            samples: ByteBudgetLru::new(budget_bytes),
        }
    }

    pub(super) fn get(&mut self, key: &str) -> Option<StaticSoundData> {
        self.samples.get(key).cloned()
    }

    pub(super) fn insert(&mut self, key: String, sample: StaticSoundData) {
        let bytes = std::mem::size_of_val(sample.frames.as_ref()) as u64;
        // A replacement invalidates the old value even when the new sample
        // is too large to retain. Never serve stale audio under the same key.
        // Unlike browser content-addressed buffers, native path keys can name
        // changed content, so a duplicate cannot simply return the old sample.
        // `ByteBudgetLru::insert` removes the old entry before the size check.
        let insertion = self.samples.insert(key, bytes, sample);
        // Stale same-key sample: the cache clone is released here.
        drop(insertion.replaced);
        if insertion.rejected.is_some() {
            tracing::debug!(
                bytes,
                budget_bytes = self.samples.budget_bytes(),
                "audio sample remains voice-owned"
            );
            return;
        }
        for victim in insertion.evicted {
            tracing::debug!(
                key = victim.key,
                bytes = victim.bytes,
                "evicted native decoded audio under byte budget"
            );
            // Releases only the cache's Arc clone; playing voices keep theirs.
            drop(victim.value);
        }
        tracing::debug!(
            resident_bytes = self.samples.resident_bytes(),
            entries = self.samples.len(),
            budget_bytes = self.samples.budget_bytes(),
            "native decoded audio cache residency"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> StaticSoundData {
        StaticSoundData {
            sample_rate: 1,
            frames: vec![kira::Frame::ZERO].into(),
            settings: Default::default(),
            slice: None,
        }
    }
    #[test]
    fn eviction_and_cache_drop_preserve_voice_owned_samples() {
        let bytes = std::mem::size_of::<kira::Frame>() as u64;
        let mut cache = SampleCache::new(bytes * 2);
        cache.insert("a".into(), sample());
        cache.insert("b".into(), sample());
        let active = cache.get("a").unwrap();
        cache.insert("c".into(), sample());
        assert!(cache.get("b").is_none());
        assert_eq!(cache.samples.resident_bytes(), bytes * 2);
        drop(cache);
        assert_eq!(std::sync::Arc::strong_count(&active.frames), 1);
        assert_eq!(active.frames.len(), 1);
    }
    #[test]
    fn oversized_sample_is_not_retained() {
        let mut cache = SampleCache::new(0);
        cache.insert("a".into(), sample());
        assert!(cache.get("a").is_none());
    }
    #[test]
    fn oversized_replacement_retires_old_sample_without_evicting_other_keys() {
        let bytes = std::mem::size_of::<kira::Frame>() as u64;
        let mut cache = SampleCache::new(bytes * 2);
        cache.insert("a".into(), sample());
        cache.insert("b".into(), sample());
        let active = cache.get("a").unwrap();
        let mut replacement = sample();
        replacement.frames = vec![kira::Frame::ZERO; 3].into();
        cache.insert("a".into(), replacement);
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_some());
        assert_eq!(cache.samples.resident_bytes(), bytes);
        assert_eq!(active.frames.len(), 1);
        assert_eq!(std::sync::Arc::strong_count(&active.frames), 1);
    }

    #[test]
    fn retained_replacement_counts_bytes_once_and_becomes_most_recent() {
        let bytes = std::mem::size_of::<kira::Frame>() as u64;
        let mut cache = SampleCache::new(bytes * 2);
        cache.insert("a".into(), sample());
        cache.insert("b".into(), sample());
        let mut replacement = sample();
        replacement.sample_rate = 2;
        cache.insert("a".into(), replacement);
        assert_eq!(cache.samples.resident_bytes(), bytes * 2);
        cache.insert("c".into(), sample());
        assert!(cache.get("b").is_none());
        assert_eq!(cache.get("a").unwrap().sample_rate, 2);
        assert!(cache.get("c").is_some());
    }

    #[test]
    fn larger_insert_evicts_multiple_least_recent_samples_to_meet_byte_budget() {
        let bytes = std::mem::size_of::<kira::Frame>() as u64;
        let mut cache = SampleCache::new(bytes * 4);
        for key in ["a", "b", "c", "d"] {
            cache.insert(key.into(), sample());
        }
        let active = cache.get("a").unwrap();
        let mut larger = sample();
        larger.frames = vec![kira::Frame::ZERO; 3].into();
        cache.insert("large".into(), larger);
        assert_eq!(cache.samples.resident_bytes(), bytes * 4);
        assert_eq!(cache.samples.len(), 2);
        for evicted in ["b", "c", "d"] {
            assert!(cache.get(evicted).is_none());
        }
        assert!(cache.get("a").is_some());
        assert_eq!(cache.get("large").unwrap().frames.len(), 3);
        drop(cache);
        assert_eq!(std::sync::Arc::strong_count(&active.frames), 1);
    }

    #[test]
    fn diagnostic_roundtrip_restores_budget_without_sample_residency() {
        let bytes = std::mem::size_of::<kira::Frame>() as u64;
        let mut cache = SampleCache::new(bytes);
        cache.insert("a".into(), sample());
        let mut restored: SampleCache =
            serde_json::from_value(serde_json::to_value(&cache).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(&cache).unwrap(),
            serde_json::json!({ "budget_bytes": bytes })
        );
        assert_eq!(restored.samples.budget_bytes(), bytes);
        assert_eq!(restored.samples.resident_bytes(), 0);
        assert!(restored.get("a").is_none());
        restored.insert("b".into(), sample());
        assert_eq!(restored.samples.resident_bytes(), bytes);
        assert!(restored.get("b").is_some());
    }
}
