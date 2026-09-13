//! Platform-neutral least-recently-used retention bounded by caller-reported bytes.
//!
//! Shared by the native decoded-sample cache and the browser decoded-buffer
//! cache. The type only governs cache ownership: values handed out by `get`
//! are expected to be cheap handles (Arc clones, JS references) whose lifetime
//! is independent of retention, so no entry is ever pinned against eviction.
use lru::LruCache;
use serde::{Deserialize, Serialize};
use std::{borrow::Borrow, hash::Hash};

/// Entry count is unbounded; residency is governed only by the byte budget.
/// Serialization keeps the budget policy, never resident entries: a restored
/// cache starts empty.
#[derive(Serialize, Deserialize)]
pub(crate) struct ByteBudgetLru<K: Hash + Eq, V> {
    budget_bytes: u64,
    #[serde(skip, default = "LruCache::unbounded")]
    entries: LruCache<K, Entry<V>>,
    #[serde(skip)]
    resident_bytes: u64,
}

struct Entry<V> {
    bytes: u64,
    value: V,
}

/// Everything an insert released from the cache, so callers can log or free
/// platform resources.
#[must_use]
pub(crate) struct Insertion<K, V> {
    /// Previous value stored under the same key; always removed, even when the
    /// new value is rejected.
    pub replaced: Option<V>,
    /// Least-recently-used entries evicted to make room, oldest first.
    pub evicted: Vec<Evicted<K, V>>,
    /// The new value when it exceeds the whole budget and was not retained.
    pub rejected: Option<V>,
}

pub(crate) struct Evicted<K, V> {
    pub key: K,
    pub bytes: u64,
    pub value: V,
}

impl<K: Hash + Eq, V> ByteBudgetLru<K, V> {
    pub(crate) fn new(budget_bytes: u64) -> Self {
        Self {
            budget_bytes,
            entries: LruCache::unbounded(),
            resident_bytes: 0,
        }
    }

    pub(crate) fn budget_bytes(&self) -> u64 {
        self.budget_bytes
    }

    pub(crate) fn resident_bytes(&self) -> u64 {
        self.resident_bytes
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the value and marks it most recently used.
    pub(crate) fn get<Q>(&mut self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.entries.get(key).map(|entry| &entry.value)
    }

    /// Keys from most to least recently used.
    #[cfg(test)]
    pub(crate) fn keys_most_recent_first(&self) -> impl Iterator<Item = &K> {
        self.entries.iter().map(|(key, _)| key)
    }

    /// Inserts `value` as most recently used. Any existing value under `key`
    /// is removed first. A value larger than the whole budget is rejected
    /// without evicting anything else; otherwise least-recently-used entries
    /// are evicted until it fits.
    pub(crate) fn insert(&mut self, key: K, bytes: u64, value: V) -> Insertion<K, V> {
        let replaced = self.entries.pop(&key).map(|old| {
            self.resident_bytes -= old.bytes;
            old.value
        });
        if bytes > self.budget_bytes {
            return Insertion {
                replaced,
                evicted: Vec::new(),
                rejected: Some(value),
            };
        }
        let mut evicted = Vec::new();
        while self.resident_bytes > self.budget_bytes - bytes {
            let (key, old) = self
                .entries
                .pop_lru()
                .expect("nonzero byte residency has a cache entry");
            self.resident_bytes -= old.bytes;
            evicted.push(Evicted {
                key,
                bytes: old.bytes,
                value: old.value,
            });
        }
        self.resident_bytes += bytes;
        self.entries.put(key, Entry { bytes, value });
        Insertion {
            replaced,
            evicted,
            rejected: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn keys(cache: &ByteBudgetLru<&'static str, u32>) -> Vec<&'static str> {
        cache.keys_most_recent_first().copied().collect()
    }

    #[test]
    fn get_refreshes_order_and_eviction_is_least_recent_first() {
        let mut cache = ByteBudgetLru::new(3);
        for (index, key) in ["a", "b", "c"].into_iter().enumerate() {
            let insertion = cache.insert(key, 1, index as u32);
            assert!(insertion.evicted.is_empty());
        }
        assert_eq!(cache.get("a"), Some(&0));
        assert_eq!(keys(&cache), ["a", "c", "b"]);
        let insertion = cache.insert("d", 2, 3);
        let evicted: Vec<_> = insertion
            .evicted
            .iter()
            .map(|entry| (entry.key, entry.bytes, entry.value))
            .collect();
        assert_eq!(evicted, [("b", 1, 1), ("c", 1, 2)]);
        assert_eq!(keys(&cache), ["d", "a"]);
        assert_eq!(cache.resident_bytes(), 3);
    }

    #[test]
    fn budget_is_exact_and_never_exceeded() {
        let mut cache = ByteBudgetLru::new(10);
        for round in 0..200u32 {
            let key = ["a", "b", "c", "d", "e"][(round % 5) as usize];
            let bytes = u64::from(round % 7);
            let _ = cache.insert(key, bytes, round);
            assert!(cache.resident_bytes() <= cache.budget_bytes());
        }
        let _ = cache.insert("full", 10, 0);
        assert_eq!(cache.resident_bytes(), 10);
        assert_eq!(keys(&cache), ["full"]);
    }

    #[test]
    fn replacement_counts_once_and_becomes_most_recent() {
        let mut cache = ByteBudgetLru::new(4);
        let _ = cache.insert("a", 2, 1);
        let _ = cache.insert("b", 2, 2);
        let insertion = cache.insert("a", 2, 3);
        assert_eq!(insertion.replaced, Some(1));
        assert!(insertion.evicted.is_empty());
        assert_eq!(cache.resident_bytes(), 4);
        assert_eq!(keys(&cache), ["a", "b"]);
        assert_eq!(cache.get("a"), Some(&3));
    }

    #[test]
    fn oversized_entry_is_rejected_without_evicting_others_but_retires_old_value() {
        let mut cache = ByteBudgetLru::new(4);
        let _ = cache.insert("a", 2, 1);
        let _ = cache.insert("b", 2, 2);
        let insertion = cache.insert("a", 5, 3);
        assert_eq!(insertion.replaced, Some(1));
        assert_eq!(insertion.rejected, Some(3));
        assert!(insertion.evicted.is_empty());
        assert!(cache.get("a").is_none());
        assert_eq!(cache.get("b"), Some(&2));
        assert_eq!(cache.resident_bytes(), 2);
        let mut disabled = ByteBudgetLru::new(0);
        assert_eq!(disabled.insert("a", 1, 1).rejected, Some(1));
        assert!(disabled.is_empty());
        // Zero-byte entries fit any budget, including a disabled one.
        assert!(disabled.insert("empty", 0, 2).rejected.is_none());
        assert_eq!(disabled.len(), 1);
    }

    #[test]
    fn evicted_values_are_returned_to_the_caller_and_outside_handles_survive() {
        let mut cache = ByteBudgetLru::new(1);
        let shared = Rc::new(7);
        let _ = cache.insert("a", 1, shared.clone());
        let insertion = cache.insert("b", 1, Rc::new(8));
        let [evicted] = insertion.evicted.as_slice() else {
            panic!("expected exactly one eviction");
        };
        assert!(Rc::ptr_eq(&evicted.value, &shared));
        drop(insertion);
        drop(cache);
        assert_eq!(Rc::strong_count(&shared), 1);
    }

    #[test]
    fn serialization_keeps_budget_but_not_residency() {
        let mut cache = ByteBudgetLru::new(12);
        let _ = cache.insert("a".to_owned(), 8, 1u32);
        let restored: ByteBudgetLru<String, u32> =
            serde_json::from_value(serde_json::to_value(&cache).unwrap()).unwrap();
        assert_eq!(restored.budget_bytes(), 12);
        assert_eq!(restored.resident_bytes(), 0);
        assert!(restored.is_empty());
    }
}
