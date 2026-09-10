//! Decoded sample residency, separate from mixer channels and device lifetime.
use std::collections::HashMap;

use kira::sound::static_sound::StaticSoundData;
use serde::{Deserialize, Serialize};

/// Cache retention is bounded; active Kira voices own independent Arc clones.
/// Eviction releases cache ownership without interrupting playback.
#[derive(Serialize, Deserialize)]
pub(super) struct SampleCache {
    #[serde(skip)]
    samples: HashMap<String, (StaticSoundData, u64)>,
    #[serde(skip)]
    clock: u64,
    #[serde(skip)]
    resident_bytes: usize,
    budget_bytes: usize,
}

impl SampleCache {
    pub(super) fn new(budget_bytes: usize) -> Self {
        Self {
            samples: HashMap::new(),
            clock: 0,
            resident_bytes: 0,
            budget_bytes,
        }
    }

    pub(super) fn get(&mut self, key: &str) -> Option<StaticSoundData> {
        self.clock = self.clock.wrapping_add(1);
        self.samples.get_mut(key).map(|(sample, used)| {
            *used = self.clock;
            sample.clone()
        })
    }

    pub(super) fn insert(&mut self, key: String, sample: StaticSoundData) {
        let bytes = std::mem::size_of_val(sample.frames.as_ref());
        if bytes > self.budget_bytes {
            tracing::debug!(
                bytes,
                budget_bytes = self.budget_bytes,
                "audio sample remains voice-owned"
            );
            return;
        }
        if let Some((old, _)) = self.samples.remove(&key) {
            self.resident_bytes -= std::mem::size_of_val(old.frames.as_ref());
        }
        while self.resident_bytes + bytes > self.budget_bytes {
            let oldest = self
                .samples
                .iter()
                .min_by_key(|(_, (_, used))| used)
                .map(|(key, _)| key.clone())
                .expect("nonzero audio residency has a cache entry");
            let (old, _) = self.samples.remove(&oldest).unwrap();
            self.resident_bytes -= std::mem::size_of_val(old.frames.as_ref());
        }
        self.clock = self.clock.wrapping_add(1);
        self.resident_bytes += bytes;
        self.samples.insert(key, (sample, self.clock));
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
}
