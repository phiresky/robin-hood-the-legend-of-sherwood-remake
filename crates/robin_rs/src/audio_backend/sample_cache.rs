//! Decoded sample residency, separate from mixer channels and device lifetime.
use kira::sound::static_sound::StaticSoundData;
use lru::LruCache;
use serde::{Deserialize, Serialize};

/// Cache retention is bounded; active Kira voices own independent Arc clones.
/// Eviction releases cache ownership without interrupting playback.
#[derive(Serialize, Deserialize)]
pub(super) struct SampleCache {
    // Entry count is unbounded here because residency is governed by bytes.
    #[serde(skip, default = "LruCache::unbounded")]
    samples: LruCache<String, StaticSoundData>,
    #[serde(skip)]
    resident_bytes: usize,
    budget_bytes: usize,
}

impl SampleCache {
    pub(super) fn new(budget_bytes: usize) -> Self {
        Self {
            samples: LruCache::unbounded(),
            resident_bytes: 0,
            budget_bytes,
        }
    }

    pub(super) fn get(&mut self, key: &str) -> Option<StaticSoundData> {
        self.samples.get(key).cloned()
    }

    pub(super) fn insert(&mut self, key: String, sample: StaticSoundData) {
        let bytes = std::mem::size_of_val(sample.frames.as_ref());
        // A replacement invalidates the old value even when the new sample
        // is too large to retain. Never serve stale audio under the same key.
        if let Some(old) = self.samples.pop(&key) {
            self.resident_bytes -= std::mem::size_of_val(old.frames.as_ref());
        }
        if bytes > self.budget_bytes {
            tracing::debug!(
                bytes,
                budget_bytes = self.budget_bytes,
                "audio sample remains voice-owned"
            );
            return;
        }
        while self.resident_bytes > self.budget_bytes - bytes {
            let (_, old) = self
                .samples
                .pop_lru()
                .expect("nonzero audio residency has a cache entry");
            self.resident_bytes -= std::mem::size_of_val(old.frames.as_ref());
        }
        self.resident_bytes += bytes;
        self.samples.put(key, sample);
        tracing::debug!(
            resident_bytes = self.resident_bytes,
            entries = self.samples.len(),
            budget_bytes = self.budget_bytes,
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
        let bytes = std::mem::size_of::<kira::Frame>();
        let mut cache = SampleCache::new(bytes * 2);
        cache.insert("a".into(), sample());
        cache.insert("b".into(), sample());
        let active = cache.get("a").unwrap();
        cache.insert("c".into(), sample());
        assert!(cache.get("b").is_none());
        assert_eq!(cache.resident_bytes, bytes * 2);
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
        let bytes = std::mem::size_of::<kira::Frame>();
        let mut cache = SampleCache::new(bytes * 2);
        cache.insert("a".into(), sample());
        cache.insert("b".into(), sample());
        let active = cache.get("a").unwrap();
        let mut replacement = sample();
        replacement.frames = vec![kira::Frame::ZERO; 3].into();
        cache.insert("a".into(), replacement);
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_some());
        assert_eq!(cache.resident_bytes, bytes);
        assert_eq!(active.frames.len(), 1);
        assert_eq!(std::sync::Arc::strong_count(&active.frames), 1);
    }

    #[test]
    fn retained_replacement_counts_bytes_once_and_becomes_most_recent() {
        let bytes = std::mem::size_of::<kira::Frame>();
        let mut cache = SampleCache::new(bytes * 2);
        cache.insert("a".into(), sample());
        cache.insert("b".into(), sample());
        let mut replacement = sample();
        replacement.sample_rate = 2;
        cache.insert("a".into(), replacement);
        assert_eq!(cache.resident_bytes, bytes * 2);
        cache.insert("c".into(), sample());
        assert!(cache.get("b").is_none());
        assert_eq!(cache.get("a").unwrap().sample_rate, 2);
        assert!(cache.get("c").is_some());
    }

    #[test]
    fn larger_insert_evicts_multiple_least_recent_samples_to_meet_byte_budget() {
        let bytes = std::mem::size_of::<kira::Frame>();
        let mut cache = SampleCache::new(bytes * 4);
        for key in ["a", "b", "c", "d"] {
            cache.insert(key.into(), sample());
        }
        let active = cache.get("a").unwrap();
        let mut larger = sample();
        larger.frames = vec![kira::Frame::ZERO; 3].into();
        cache.insert("large".into(), larger);
        assert_eq!(cache.resident_bytes, bytes * 4);
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
        let bytes = std::mem::size_of::<kira::Frame>();
        let mut cache = SampleCache::new(bytes);
        cache.insert("a".into(), sample());
        let mut restored: SampleCache =
            serde_json::from_value(serde_json::to_value(&cache).unwrap()).unwrap();
        assert_eq!(restored.budget_bytes, bytes);
        assert_eq!(restored.resident_bytes, 0);
        assert!(restored.get("a").is_none());
        restored.insert("b".into(), sample());
        assert_eq!(restored.resident_bytes, bytes);
        assert!(restored.get("b").is_some());
    }
}
