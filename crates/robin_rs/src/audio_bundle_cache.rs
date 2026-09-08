//! Byte-bounded retention for encoded browser audio, independent of JS handles.
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Conservative retention ceiling, one third of the separate 96 MiB PCM cap.
/// This bounds cache-owned bytes, not live fetch/decode references or JS heap.
/// TODO: tune using real mission/locale residency and repeat-fetch traces.
pub(crate) const DEFAULT_ENCODED_BUDGET: u64 = 32 * 1024 * 1024;

#[derive(Default, Clone, Copy, Debug, Serialize, Deserialize)]
pub(crate) struct RetentionStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub bypasses: u64,
    pub resident_bytes: u64,
}

/// Oldest first. Bundle counts are small; moving entries avoids clock overflow
/// and keeps the eviction order deterministic without a second index.
#[derive(Serialize, Deserialize)]
pub(crate) struct AudioBundleCache<T> {
    budget: u64,
    #[serde(skip)]
    entries: VecDeque<(String, u64, T)>,
    #[serde(skip)]
    stats: RetentionStats,
}

impl<T> Default for AudioBundleCache<T> {
    fn default() -> Self {
        Self::new(DEFAULT_ENCODED_BUDGET)
    }
}

impl<T> AudioBundleCache<T> {
    pub fn new(budget: u64) -> Self {
        Self {
            budget,
            entries: VecDeque::new(),
            stats: RetentionStats::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&mut self, key: &str) -> Option<&T> {
        let Some(index) = self.entries.iter().position(|entry| entry.0 == key) else {
            self.stats.misses = self.stats.misses.saturating_add(1);
            self.trace("miss");
            return None;
        };
        let entry = self
            .entries
            .remove(index)
            .expect("located bundle must exist");
        self.entries.push_back(entry);
        self.stats.hits = self.stats.hits.saturating_add(1);
        self.trace("hit");
        Some(&self.entries.back().expect("just inserted bundle").2)
    }

    /// Retains a clone supplied by the caller, never invalidating other handles.
    /// Empty and oversized bundles bypass retention without evicting useful data.
    pub fn insert(&mut self, key: String, bytes: u64, value: T) {
        if let Some(index) = self.entries.iter().position(|entry| entry.0 == key) {
            let (_, size, _) = self
                .entries
                .remove(index)
                .expect("located bundle must exist");
            self.stats.resident_bytes -= size;
        }
        if bytes == 0 || bytes > self.budget {
            self.stats.bypasses = self.stats.bypasses.saturating_add(1);
            self.trace("bypass");
            return;
        }
        while self.stats.resident_bytes > self.budget - bytes {
            let (_, size, _) = self
                .entries
                .pop_front()
                .expect("positive residency requires an entry");
            self.stats.resident_bytes -= size;
            self.stats.evictions = self.stats.evictions.saturating_add(1);
        }
        self.entries.push_back((key, bytes, value));
        self.stats.resident_bytes += bytes;
        self.trace("retain");
    }

    fn trace(&self, operation: &str) {
        tracing::debug!(
            operation,
            budget_bytes = self.budget,
            resident_bytes = self.stats.resident_bytes,
            bundles = self.entries.len(),
            hits = self.stats.hits,
            misses = self.stats.misses,
            evictions = self.stats.evictions,
            bypasses = self.stats.bypasses,
            "browser encoded audio retention"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[test]
    fn hits_refresh_lru_and_eviction_keeps_borrowed_values_alive() {
        let mut cache = AudioBundleCache::new(8);
        let retained = Rc::new(vec![1; 4]);
        cache.insert("a".into(), 4, retained.clone());
        cache.insert("b".into(), 4, Rc::new(vec![2; 4]));
        let borrowed = cache.get("b").unwrap().clone();
        cache.get("a").unwrap();
        cache.insert("c".into(), 4, Rc::new(vec![3; 4]));
        assert!(cache.get("b").is_none());
        assert_eq!(*borrowed, vec![2; 4]);
        assert_eq!(cache.stats.resident_bytes, 8);
        assert_eq!(cache.stats.evictions, 1);
        assert_eq!(cache.stats.hits, 2);
        assert_eq!(cache.stats.misses, 1);
    }

    #[test]
    fn replacement_oversized_empty_and_disabled_account_exactly() {
        let mut cache = AudioBundleCache::new(8);
        cache.insert("a".into(), 8, ());
        cache.insert("a".into(), 3, ());
        cache.insert("huge".into(), u64::MAX, ());
        cache.insert("empty".into(), 0, ());
        assert_eq!(cache.stats.resident_bytes, 3);
        assert_eq!(cache.stats.evictions, 0);
        assert_eq!(cache.stats.bypasses, 2);
        let mut disabled = AudioBundleCache::new(0);
        disabled.insert("a".into(), 1, ());
        assert!(disabled.is_empty());
    }

    #[test]
    fn repeated_catalog_and_locale_workloads_stay_bounded() {
        let mut cache = AudioBundleCache::new(32);
        for round in 0..100 {
            for locale in ["en", "fr", "de"] {
                for catalog in 0..4 {
                    let key = format!("{locale}/{catalog}");
                    cache.get(&key);
                    cache.insert(key, 7 + (round % 4), ());
                    assert!(cache.stats.resident_bytes <= 32);
                    assert_eq!(
                        cache.stats.resident_bytes,
                        cache.entries.iter().map(|entry| entry.1).sum()
                    );
                }
            }
        }
        assert!(cache.stats.evictions > 1000);
    }

    #[test]
    fn serialization_preserves_policy_not_resident_authority() {
        let mut cache = AudioBundleCache::new(12);
        cache.insert("a".into(), 8, ());
        let decoded: AudioBundleCache<()> =
            serde_json::from_str(&serde_json::to_string(&cache).unwrap()).unwrap();
        assert!(decoded.is_empty());
        assert_eq!(decoded.budget, 12);
        assert_eq!(decoded.stats.resident_bytes, 0);
    }
}
