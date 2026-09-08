//! Durable native cache and resumable staging for validated distributed mods.
//!
//! Cache entries are disposable downloaded content, never an authority: every
//! hit is decoded and revalidated against its exact full-mod hash before use.
//! The trust store remains separate and a cache hit cannot imply consent.

use crate::distributed_mod::{
    DISTRIBUTED_MOD_ENCODED_LIMIT, DistributedModPackage, ValidatedDistributedMod,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::distributed_mod_policy::{CacheIndexEntry, validate_chunk, validate_transfer_total};
pub use crate::distributed_mod_policy::{
    DISTRIBUTED_MOD_CACHE_BYTE_LIMIT, DISTRIBUTED_MOD_CACHE_ENTRY_LIMIT,
    DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION, DISTRIBUTED_MOD_TRANSFER_CHUNK_LIMIT,
};

const CACHE_DIRECTORY: &str = "distributed-mod-cache";
const CACHE_INDEX_FILE: &str = "index.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheIndex {
    schema_version: u32,
    access_counter: u64,
    entries: BTreeMap<String, CacheIndexEntry>,
}

impl Default for CacheIndex {
    fn default() -> Self {
        Self {
            schema_version: DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION,
            access_counter: 0,
            entries: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Default)]
struct CachePins {
    hashes: Mutex<BTreeMap<[u8; 32], usize>>,
}

impl CachePins {
    fn pin(&self, hash: [u8; 32]) {
        *self
            .hashes
            .lock()
            .expect("distributed-mod cache pin lock poisoned")
            .entry(hash)
            .or_default() += 1;
    }

    fn unpin(&self, hash: [u8; 32]) {
        let mut pins = self
            .hashes
            .lock()
            .expect("distributed-mod cache pin lock poisoned");
        let count = pins
            .get_mut(&hash)
            .expect("distributed-mod cache lease dropped without a pin");
        *count -= 1;
        if *count == 0 {
            pins.remove(&hash);
        }
    }

    fn contains(&self, hash: &[u8; 32]) -> bool {
        self.hashes
            .lock()
            .expect("distributed-mod cache pin lock poisoned")
            .contains_key(hash)
    }
}

/// An exact validated package pinned against cache eviction for its lifetime.
#[derive(Debug)]
pub struct DistributedModCacheLease {
    pub validated: ValidatedDistributedMod,
    encoded: Arc<[u8]>,
    full_mod_sha256: [u8; 32],
    pins: Arc<CachePins>,
}

impl DistributedModCacheLease {
    pub fn encoded(&self) -> &[u8] {
        &self.encoded
    }

    pub fn encoded_arc(&self) -> Arc<[u8]> {
        Arc::clone(&self.encoded)
    }
}

impl Drop for DistributedModCacheLease {
    fn drop(&mut self) {
        self.pins.unpin(self.full_mod_sha256);
    }
}

#[derive(Debug)]
pub struct DistributedModCache {
    root: PathBuf,
    index: CacheIndex,
    pins: Arc<CachePins>,
}

impl DistributedModCache {
    pub fn open(save_directory: &str) -> Result<Self, String> {
        let root = Path::new(save_directory).join(CACHE_DIRECTORY);
        std::fs::create_dir_all(&root)
            .map_err(|error| format!("create distributed-mod cache {}: {error}", root.display()))?;
        let index_path = root.join(CACHE_INDEX_FILE);
        let index = match std::fs::read(&index_path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| format!("parse {}: {error}", index_path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => CacheIndex::default(),
            Err(error) => return Err(format!("read {}: {error}", index_path.display())),
        };
        let mut cache = Self {
            root,
            index,
            pins: Arc::new(CachePins::default()),
        };
        cache.validate_index()?;
        cache.remove_unindexed_complete_files()?;
        cache.prune_stale_partials_to_limits()?;
        Ok(cache)
    }

    pub fn contains(&self, full_mod_sha256: [u8; 32]) -> bool {
        self.index.entries.contains_key(&hex_hash(&full_mod_sha256))
    }

    pub fn acquire(
        &mut self,
        full_mod_sha256: [u8; 32],
    ) -> Result<Option<DistributedModCacheLease>, String> {
        let key = hex_hash(&full_mod_sha256);
        let Some(entry) = self.index.entries.get(&key) else {
            return Ok(None);
        };
        let declared_len = entry.encoded_bytes;
        let bytes = read_bounded(
            &self.complete_path(&full_mod_sha256),
            DISTRIBUTED_MOD_ENCODED_LIMIT,
        )?;
        let validated = crate::distributed_mod_policy::validate_cached_package(
            &bytes,
            full_mod_sha256,
            declared_len,
        )?;
        self.touch(&key)?;
        self.pins.pin(full_mod_sha256);
        Ok(Some(DistributedModCacheLease {
            validated,
            encoded: Arc::from(bytes),
            full_mod_sha256,
            pins: Arc::clone(&self.pins),
        }))
    }

    pub fn install(
        &mut self,
        encoded: Vec<u8>,
        expected_full_mod_sha256: [u8; 32],
    ) -> Result<DistributedModCacheLease, String> {
        let validated = DistributedModPackage::decode(&encoded)
            .map_err(|error| format!("validate downloaded distributed mod: {error}"))?;
        if validated.package.manifest.full_mod_sha256 != expected_full_mod_sha256 {
            return Err(format!(
                "downloaded full-mod hash is {}, expected {}",
                hex_hash(&validated.package.manifest.full_mod_sha256),
                hex_hash(&expected_full_mod_sha256)
            ));
        }
        let key = hex_hash(&expected_full_mod_sha256);
        let had_prior = self.index.entries.contains_key(&key);
        atomic_write(
            &self.root,
            &self.complete_path(&expected_full_mod_sha256),
            &encoded,
        )?;
        let prior = self.index.clone();
        self.index.access_counter = self.index.access_counter.saturating_add(1);
        self.index.entries.insert(
            key,
            CacheIndexEntry {
                encoded_bytes: encoded.len() as u64,
                last_used: self.index.access_counter,
            },
        );
        let evicted = match self.evict_to_limits(Some(expected_full_mod_sha256)) {
            Ok(evicted) => evicted,
            Err(error) => {
                self.index = prior;
                if !had_prior {
                    let _ = std::fs::remove_file(self.complete_path(&expected_full_mod_sha256));
                }
                return Err(error);
            }
        };
        if let Err(error) = self.save_index() {
            self.index = prior;
            if !had_prior {
                let _ = std::fs::remove_file(self.complete_path(&expected_full_mod_sha256));
            }
            return Err(error);
        }
        for path in evicted {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => tracing::warn!(
                    "failed to remove unindexed distributed-mod cache entry {}: {error}",
                    path.display()
                ),
            }
        }
        let _ = std::fs::remove_file(self.partial_path(&expected_full_mod_sha256));
        self.pins.pin(expected_full_mod_sha256);
        Ok(DistributedModCacheLease {
            validated,
            encoded: Arc::from(encoded),
            full_mod_sha256: expected_full_mod_sha256,
            pins: Arc::clone(&self.pins),
        })
    }

    /// Return the exact durable prefix already staged for this immutable hash.
    /// Disposable corrupt/oversized staging is discarded so the exact transfer
    /// can restart, while a valid complete object rejects a contradictory offer.
    pub fn resume_offset(
        &mut self,
        full_mod_sha256: [u8; 32],
        total_bytes: u64,
    ) -> Result<u64, String> {
        validate_transfer_total(total_bytes)?;
        let key = hex_hash(&full_mod_sha256);
        if let Some(encoded_bytes) = self
            .index
            .entries
            .get(&key)
            .map(|entry| entry.encoded_bytes)
        {
            let complete_path = self.complete_path(&full_mod_sha256);
            let validation =
                read_bounded(&complete_path, DISTRIBUTED_MOD_ENCODED_LIMIT).and_then(|bytes| {
                    crate::distributed_mod_policy::validate_cached_package(
                        &bytes,
                        full_mod_sha256,
                        encoded_bytes,
                    )
                    .map(|_| ())
                });
            if let Err(error) = validation {
                self.evict_corrupt_complete(&key, full_mod_sha256, &error)?;
                return Ok(0);
            }
            if encoded_bytes != total_bytes {
                return Err(format!(
                    "validated complete cache entry has {encoded_bytes} bytes, but the host offer for the same immutable hash declares {total_bytes}"
                ));
            }
            let _ = std::fs::remove_file(self.partial_path(&full_mod_sha256));
            return Ok(total_bytes);
        }
        let partial_path = self.partial_path(&full_mod_sha256);
        match std::fs::symlink_metadata(&partial_path) {
            Ok(metadata) if metadata.file_type().is_file() && metadata.len() <= total_bytes => {
                Ok(metadata.len())
            }
            Ok(metadata) => {
                let reason = if metadata.file_type().is_file() {
                    format!(
                        "partial distributed mod is {} bytes; offer declares {total_bytes}",
                        metadata.len()
                    )
                } else {
                    "partial distributed-mod cache entry is not a regular file".to_owned()
                };
                discard_partial_path(&partial_path).map_err(|remove_error| {
                    format!(
                        "{reason}; additionally failed to discard {}: {remove_error}",
                        partial_path.display()
                    )
                })?;
                tracing::warn!(
                    hash = %key,
                    "discarded unusable distributed-mod partial and restarted from zero: {reason}"
                );
                Ok(0)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(error) => Err(format!("inspect partial distributed mod: {error}")),
        }
    }

    /// Append one exact sequential transfer chunk and sync it before
    /// acknowledging the new offset. Returns the durable end offset.
    pub fn append_chunk(
        &mut self,
        full_mod_sha256: [u8; 32],
        total_bytes: u64,
        offset: u64,
        chunk: &[u8],
    ) -> Result<u64, String> {
        let end = validate_chunk(total_bytes, offset, chunk.len())?;
        let path = self.partial_path(&full_mod_sha256);
        let current = match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => metadata.len(),
            Ok(_) => {
                return Err(format!(
                    "partial distributed-mod cache entry {} is not a regular file",
                    path.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => {
                return Err(format!(
                    "inspect partial distributed mod {}: {error}",
                    path.display()
                ));
            }
        };
        if current != offset {
            return Err(format!(
                "distributed-mod chunk starts at {offset}, durable partial ends at {current}"
            ));
        }
        self.reserve_staging_capacity(&path, current, end)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| format!("open partial distributed mod {}: {error}", path.display()))?;
        let opened_len = file
            .metadata()
            .map_err(|error| format!("inspect {}: {error}", path.display()))?
            .len();
        if opened_len != current {
            return Err(format!(
                "partial distributed mod changed from {current} to {opened_len} bytes while opening"
            ));
        }
        file.seek(SeekFrom::Start(offset))
            .and_then(|_| file.write_all(chunk))
            .and_then(|_| file.sync_data())
            .map_err(|error| {
                format!("append partial distributed mod {}: {error}", path.display())
            })?;
        Ok(end)
    }

    pub fn finish_partial(
        &mut self,
        full_mod_sha256: [u8; 32],
        total_bytes: u64,
    ) -> Result<DistributedModCacheLease, String> {
        validate_transfer_total(total_bytes)?;
        let path = self.partial_path(&full_mod_sha256);
        let encoded = read_bounded(&path, DISTRIBUTED_MOD_ENCODED_LIMIT)?;
        if encoded.len() as u64 != total_bytes {
            return Err(format!(
                "partial distributed mod is {} bytes; expected {total_bytes}",
                encoded.len()
            ));
        }
        let validation = DistributedModPackage::decode(&encoded)
            .map_err(|error| format!("validate staged distributed mod: {error}"))
            .and_then(|validated| {
                if validated.package.manifest.full_mod_sha256 == full_mod_sha256 {
                    Ok(())
                } else {
                    Err(format!(
                        "staged full-mod hash is {}, expected {}",
                        hex_hash(&validated.package.manifest.full_mod_sha256),
                        hex_hash(&full_mod_sha256)
                    ))
                }
            });
        if let Err(error) = validation {
            match std::fs::remove_file(&path) {
                Ok(()) => return Err(error),
                Err(remove_error) if remove_error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(error);
                }
                Err(remove_error) => {
                    return Err(format!(
                        "{error}; additionally failed to discard invalid partial {}: {remove_error}",
                        path.display()
                    ));
                }
            }
        }
        self.install(encoded, full_mod_sha256)
    }

    pub fn clear(&mut self) -> Result<usize, String> {
        if !self.pins.hashes.lock().expect("cache pin lock").is_empty() {
            return Err("cannot clear distributed-mod cache while content is mounted".to_owned());
        }
        let removed = self.index.entries.len();
        for key in self.index.entries.keys() {
            let hash = parse_hash(key)?;
            let path = self.complete_path(&hash);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("remove {}: {error}", path.display())),
            }
        }
        for entry in std::fs::read_dir(&self.root)
            .map_err(|error| format!("read cache directory {}: {error}", self.root.display()))?
        {
            let path = entry
                .map_err(|error| format!("read cache directory entry: {error}"))?
                .path();
            if path
                .extension()
                .is_some_and(|extension| extension == "part")
            {
                std::fs::remove_file(&path)
                    .map_err(|error| format!("remove partial cache {}: {error}", path.display()))?;
            }
        }
        self.index = CacheIndex::default();
        self.save_index()?;
        Ok(removed)
    }

    fn touch(&mut self, key: &str) -> Result<(), String> {
        let prior_counter = self.index.access_counter;
        let prior_last_used = self.index.entries[key].last_used;
        self.index.access_counter = self.index.access_counter.saturating_add(1);
        self.index
            .entries
            .get_mut(key)
            .expect("cache key disappeared while touching")
            .last_used = self.index.access_counter;
        if let Err(error) = self.save_index() {
            self.index.access_counter = prior_counter;
            self.index
                .entries
                .get_mut(key)
                .expect("cache key disappeared while rolling back touch")
                .last_used = prior_last_used;
            return Err(error);
        }
        Ok(())
    }

    fn reserve_staging_capacity(
        &mut self,
        current_path: &Path,
        current_bytes: u64,
        new_bytes: u64,
    ) -> Result<(), String> {
        let mut complete_bytes = self
            .index
            .entries
            .values()
            .try_fold(0u64, |sum, entry| sum.checked_add(entry.encoded_bytes))
            .ok_or_else(|| "distributed-mod cache byte accounting overflow".to_owned())?;
        let mut partials = Vec::new();
        let mut partial_bytes = 0u64;
        let mut current_seen = false;
        for entry in std::fs::read_dir(&self.root)
            .map_err(|error| format!("read cache directory {}: {error}", self.root.display()))?
        {
            let path = entry
                .map_err(|error| format!("read cache directory entry: {error}"))?
                .path();
            if !path
                .extension()
                .is_some_and(|extension| extension == "part")
            {
                continue;
            }
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("inspect partial cache {}: {error}", path.display()))?;
            if !metadata.file_type().is_file() {
                return Err(format!(
                    "partial distributed-mod cache entry {} is not a regular file",
                    path.display()
                ));
            }
            partial_bytes = partial_bytes
                .checked_add(metadata.len())
                .ok_or_else(|| "distributed-mod partial byte accounting overflow".to_owned())?;
            current_seen |= path == current_path;
            partials.push((
                path,
                metadata.len(),
                metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
            ));
        }
        let mut staged_after = partial_bytes
            .checked_sub(current_bytes)
            .and_then(|bytes| bytes.checked_add(new_bytes))
            .ok_or_else(|| "distributed-mod partial byte accounting overflow".to_owned())?;
        let mut entry_count = self
            .index
            .entries
            .len()
            .checked_add(partials.len())
            .and_then(|count| count.checked_add(usize::from(!current_seen)))
            .ok_or_else(|| "distributed-mod cache entry count overflow".to_owned())?;
        let mut partial_evictions = Vec::new();
        let mut complete_evictions = Vec::new();
        let prior_index = self.index.clone();

        loop {
            let total_after = complete_bytes
                .checked_add(staged_after)
                .ok_or_else(|| "distributed-mod cache byte accounting overflow".to_owned())?;
            if entry_count <= DISTRIBUTED_MOD_CACHE_ENTRY_LIMIT
                && total_after <= DISTRIBUTED_MOD_CACHE_BYTE_LIMIT
            {
                break;
            }

            // Incomplete transfers are disposable and use filesystem mtime as
            // their durable LRU stamp. Preserve the transfer being appended.
            if let Some((index, _)) = partials
                .iter()
                .enumerate()
                .filter(|(_, (path, _, _))| path != current_path)
                .min_by(|(_, a), (_, b)| a.2.cmp(&b.2).then_with(|| a.0.cmp(&b.0)))
            {
                let (path, bytes, _) = partials.remove(index);
                staged_after = staged_after.checked_sub(bytes).ok_or_else(|| {
                    "distributed-mod partial byte accounting underflow".to_owned()
                })?;
                entry_count -= 1;
                partial_evictions.push(path);
                continue;
            }

            // Then reclaim the least-recently-used validated complete object.
            // Mounted objects are pinned and never eligible.
            let candidate = self
                .index
                .entries
                .iter()
                .filter_map(|(key, entry)| {
                    let hash = parse_hash(key).ok()?;
                    (!self.pins.contains(&hash)).then_some((entry.last_used, key.clone(), hash))
                })
                .min_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            let Some((_, key, hash)) = candidate else {
                self.index = prior_index;
                return Err(format!(
                    "distributed-mod cache cannot reserve {new_bytes} staged bytes: no unpinned complete or older partial entry remains"
                ));
            };
            let removed = self
                .index
                .entries
                .remove(&key)
                .expect("selected cache reservation candidate disappeared");
            complete_bytes = complete_bytes
                .checked_sub(removed.encoded_bytes)
                .ok_or_else(|| "distributed-mod complete byte accounting underflow".to_owned())?;
            entry_count -= 1;
            complete_evictions.push(self.complete_path(&hash));
        }

        let index_changed = !complete_evictions.is_empty();
        if index_changed && let Err(error) = self.save_index() {
            self.index = prior_index;
            return Err(error);
        }
        for path in partial_evictions.into_iter().chain(complete_evictions) {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "remove LRU distributed-mod cache entry {}: {error}",
                        path.display()
                    ));
                }
            }
        }
        Ok(())
    }

    fn evict_corrupt_complete(
        &mut self,
        key: &str,
        full_mod_sha256: [u8; 32],
        reason: &str,
    ) -> Result<(), String> {
        // Complete cache objects are disposable, never authority. A corrupt
        // object must not permanently poison every reconnect for this
        // immutable hash: remove it transactionally and ask the host for the
        // exact bytes again from offset zero.
        let prior = self.index.clone();
        self.index.entries.remove(key);
        if let Err(save_error) = self.save_index() {
            self.index = prior;
            return Err(format!(
                "{reason}; additionally failed to evict corrupt cache entry: {save_error}"
            ));
        }
        let complete_path = self.complete_path(&full_mod_sha256);
        match std::fs::remove_file(&complete_path) {
            Ok(()) => {}
            Err(remove_error) if remove_error.kind() == std::io::ErrorKind::NotFound => {}
            Err(remove_error) => tracing::warn!(
                "failed to remove corrupt distributed-mod cache entry {}: {remove_error}",
                complete_path.display()
            ),
        }
        let partial_path = self.partial_path(&full_mod_sha256);
        if let Err(remove_error) = discard_partial_path(&partial_path)
            && remove_error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                "failed to remove stale partial for corrupt cache entry {}: {remove_error}",
                partial_path.display()
            );
        }
        tracing::warn!(hash = %key, "evicted corrupt distributed-mod cache entry: {reason}");
        Ok(())
    }

    fn evict_to_limits(&mut self, protected: Option<[u8; 32]>) -> Result<Vec<PathBuf>, String> {
        let mut protected_keys = BTreeSet::new();
        for key in self.index.entries.keys() {
            let hash = parse_hash(key)?;
            if protected == Some(hash) || self.pins.contains(&hash) {
                protected_keys.insert(key.clone());
            }
        }
        let selected = crate::distributed_mod_policy::select_evictions(
            &self.index.entries,
            &protected_keys,
            0,
        )?;
        let mut evicted = Vec::new();
        for key in selected {
            let hash = parse_hash(&key)?;
            evicted.push(self.complete_path(&hash));
            self.index.entries.remove(&key);
        }
        Ok(evicted)
    }

    fn validate_index(&mut self) -> Result<(), String> {
        if self.index.schema_version != DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported distributed-mod cache schema {}; expected {DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION}",
                self.index.schema_version
            ));
        }
        if self.index.entries.len() > DISTRIBUTED_MOD_CACHE_ENTRY_LIMIT {
            return Err(format!(
                "distributed-mod cache index has {} entries; limit is {DISTRIBUTED_MOD_CACHE_ENTRY_LIMIT}",
                self.index.entries.len()
            ));
        }
        let mut invalid_files = Vec::new();
        for (key, entry) in &self.index.entries {
            let hash = parse_hash(key)?;
            if entry.encoded_bytes as usize > DISTRIBUTED_MOD_ENCODED_LIMIT {
                return Err(format!("cache entry {key} exceeds the package byte limit"));
            }
            let path = self.complete_path(&hash);
            match std::fs::symlink_metadata(&path) {
                Ok(metadata)
                    if metadata.file_type().is_file() && metadata.len() == entry.encoded_bytes => {}
                Ok(metadata) => invalid_files.push((
                    key.clone(),
                    format!(
                        "{} is {} bytes and has type {:?}; index declares {} regular-file bytes",
                        path.display(),
                        metadata.len(),
                        metadata.file_type(),
                        entry.encoded_bytes
                    ),
                )),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    invalid_files.push((key.clone(), format!("{} is missing", path.display())))
                }
                Err(error) => return Err(format!("inspect cache entry {key}: {error}")),
            }
        }
        if !invalid_files.is_empty() {
            let prior = self.index.clone();
            for (key, reason) in &invalid_files {
                self.index.entries.remove(key);
                tracing::warn!(hash = %key, "discarding unusable distributed-mod cache index entry: {reason}");
            }
            if let Err(error) = self.save_index() {
                self.index = prior;
                return Err(error);
            }
        }
        let total = self
            .index
            .entries
            .values()
            .try_fold(0u64, |sum, entry| sum.checked_add(entry.encoded_bytes))
            .ok_or_else(|| "distributed-mod cache index byte overflow".to_owned())?;
        if total > DISTRIBUTED_MOD_CACHE_BYTE_LIMIT {
            return Err(format!(
                "distributed-mod cache index declares {total} bytes; limit is {DISTRIBUTED_MOD_CACHE_BYTE_LIMIT}"
            ));
        }
        Ok(())
    }

    fn remove_unindexed_complete_files(&mut self) -> Result<(), String> {
        let indexed = self.index.entries.keys().cloned().collect::<BTreeSet<_>>();
        for entry in std::fs::read_dir(&self.root)
            .map_err(|error| format!("read cache directory {}: {error}", self.root.display()))?
        {
            let path = entry
                .map_err(|error| format!("read cache directory entry: {error}"))?
                .path();
            if path
                .extension()
                .is_some_and(|extension| extension == "rhmod")
            {
                let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                    return Err(format!("non-UTF-8 cache filename {}", path.display()));
                };
                if !indexed.contains(stem) {
                    std::fs::remove_file(&path).map_err(|error| {
                        format!("remove unindexed cache entry {}: {error}", path.display())
                    })?;
                }
            }
        }
        Ok(())
    }

    fn prune_stale_partials_to_limits(&self) -> Result<(), String> {
        let complete_bytes = self
            .index
            .entries
            .values()
            .try_fold(0u64, |sum, entry| sum.checked_add(entry.encoded_bytes))
            .ok_or_else(|| "distributed-mod cache byte accounting overflow".to_owned())?;
        let mut partials = Vec::new();
        let mut partial_bytes = 0u64;
        for entry in std::fs::read_dir(&self.root)
            .map_err(|error| format!("read cache directory {}: {error}", self.root.display()))?
        {
            let path = entry
                .map_err(|error| format!("read cache directory entry: {error}"))?
                .path();
            if !path
                .extension()
                .is_some_and(|extension| extension == "part")
            {
                continue;
            }
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("inspect partial cache {}: {error}", path.display()))?;
            if !metadata.file_type().is_file() {
                discard_partial_path(&path).map_err(|error| {
                    format!(
                        "discard non-regular partial cache {}: {error}",
                        path.display()
                    )
                })?;
                continue;
            }
            partial_bytes = partial_bytes
                .checked_add(metadata.len())
                .ok_or_else(|| "distributed-mod partial byte accounting overflow".to_owned())?;
            partials.push((
                path,
                metadata.len(),
                metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
            ));
        }
        partials.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.0.cmp(&b.0)));
        while self
            .index
            .entries
            .len()
            .checked_add(partials.len())
            .ok_or_else(|| "distributed-mod cache entry count overflow".to_owned())?
            > DISTRIBUTED_MOD_CACHE_ENTRY_LIMIT
            || complete_bytes
                .checked_add(partial_bytes)
                .ok_or_else(|| "distributed-mod cache byte accounting overflow".to_owned())?
                > DISTRIBUTED_MOD_CACHE_BYTE_LIMIT
        {
            if partials.is_empty() {
                return Err(
                    "distributed-mod cache exceeds limits without a disposable partial entry"
                        .to_owned(),
                );
            }
            let (path, bytes, _) = partials.remove(0);
            discard_partial_path(&path).map_err(|error| {
                format!("remove stale partial cache {}: {error}", path.display())
            })?;
            partial_bytes = partial_bytes
                .checked_sub(bytes)
                .ok_or_else(|| "distributed-mod partial byte accounting underflow".to_owned())?;
        }
        Ok(())
    }

    fn save_index(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&self.index)
            .map_err(|error| format!("encode distributed-mod cache index: {error}"))?;
        atomic_write(&self.root, &self.root.join(CACHE_INDEX_FILE), &bytes)
    }

    fn complete_path(&self, hash: &[u8; 32]) -> PathBuf {
        self.root.join(format!("{}.rhmod", hex_hash(hash)))
    }

    fn partial_path(&self, hash: &[u8; 32]) -> PathBuf {
        self.root.join(format!("{}.part", hex_hash(hash)))
    }
}

fn discard_partial_path(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        // Never recursively remove an unexpected directory at a cache-file
        // path. An empty directory is safe to discard; a non-empty one fails
        // closed and needs explicit user cleanup.
        std::fs::remove_dir(path)
    } else {
        // `remove_file` removes a symlink itself rather than following it.
        std::fs::remove_file(path)
    }
}

fn atomic_write(directory: &Path, destination: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut temporary = tempfile::NamedTempFile::new_in(directory)
        .map_err(|error| format!("create temporary cache file: {error}"))?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| format!("write temporary cache file: {error}"))?;
    temporary.persist(destination).map_err(|error| {
        format!(
            "atomically replace cache file {}: {}",
            destination.display(),
            error.error
        )
    })?;
    Ok(())
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("open cached distributed mod {}: {error}", path.display()))?;
    let len = file
        .metadata()
        .map_err(|error| format!("inspect cached distributed mod {}: {error}", path.display()))?
        .len();
    if len > limit as u64 {
        return Err(format!(
            "cached distributed mod {} is {len} bytes; limit is {limit}",
            path.display()
        ));
    }
    let mut bytes = Vec::with_capacity(len as usize);
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read cached distributed mod {}: {error}", path.display()))?;
    if bytes.len() > limit {
        return Err(format!(
            "cached distributed mod {} grew past {limit} bytes while reading",
            path.display()
        ));
    }
    Ok(bytes)
}

fn hex_hash(hash: &[u8; 32]) -> String {
    robin_engine::spellforge::hex_hash(hash)
}

fn parse_hash(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid distributed-mod cache hash `{value}`"));
    }
    let mut hash = [0u8; 32];
    for (index, byte) in hash.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|error| format!("parse cache hash `{value}`: {error}"))?;
    }
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distributed_mod::DistributedModPackage;
    use std::io::Cursor;

    fn rhm(map: &str) -> Vec<u8> {
        let mut bytes = vec![0; 34];
        bytes[..4].copy_from_slice(b"RHMI");
        bytes[32..34].copy_from_slice(&(map.len() as u16).to_le_bytes());
        bytes.extend_from_slice(map.as_bytes());
        bytes
    }

    fn package(title: &str) -> (Vec<u8>, [u8; 32]) {
        let mut archive_cursor = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut archive_cursor);
            writer
                .start_file(
                    "Data/Levels/Mission.rhm",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            writer.write_all(&rhm("Map")).unwrap();
            writer.finish().unwrap();
        }
        let validated = DistributedModPackage::build(
            title.to_ascii_lowercase(),
            title.into(),
            "Author".into(),
            "1".into(),
            "https://example.invalid".into(),
            "CC0-1.0".into(),
            "Mission".into(),
            "Data/Levels/Mission.rhm".into(),
            "Map".into(),
            false,
            archive_cursor.into_inner(),
            None,
        )
        .unwrap();
        let hash = validated.package.manifest.full_mod_sha256;
        (validated.package.encode().unwrap(), hash)
    }

    #[test]
    fn install_reopen_acquire_and_clear_are_exact() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy();
        let (encoded, hash) = package("One");
        {
            let mut cache = DistributedModCache::open(&root).unwrap();
            let lease = cache.install(encoded.clone(), hash).unwrap();
            assert_eq!(lease.encoded(), encoded);
            assert!(cache.clear().is_err(), "mounted content must be pinned");
            drop(lease);
        }
        let mut reopened = DistributedModCache::open(&root).unwrap();
        let lease = reopened.acquire(hash).unwrap().unwrap();
        assert_eq!(lease.encoded(), encoded);
        drop(lease);
        assert_eq!(reopened.clear().unwrap(), 1);
        assert!(!reopened.contains(hash));
    }

    #[test]
    fn staged_transfer_is_sequential_resumable_and_revalidated() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy();
        let (encoded, hash) = package("Resume");
        let split = encoded.len() / 2;
        {
            let mut cache = DistributedModCache::open(&root).unwrap();
            assert_eq!(cache.resume_offset(hash, encoded.len() as u64).unwrap(), 0);
            assert_eq!(
                cache
                    .append_chunk(hash, encoded.len() as u64, 0, &encoded[..split])
                    .unwrap(),
                split as u64
            );
            assert!(
                cache
                    .append_chunk(hash, encoded.len() as u64, 0, &encoded[split..])
                    .is_err()
            );
        }
        let mut reopened = DistributedModCache::open(&root).unwrap();
        assert_eq!(
            reopened.resume_offset(hash, encoded.len() as u64).unwrap(),
            split as u64
        );
        reopened
            .append_chunk(hash, encoded.len() as u64, split as u64, &encoded[split..])
            .unwrap();
        let lease = reopened.finish_partial(hash, encoded.len() as u64).unwrap();
        assert_eq!(lease.validated.package.manifest.full_mod_sha256, hash);
    }

    #[test]
    fn wrong_expected_hash_never_enters_cache() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy();
        let (encoded, hash) = package("Wrong");
        let mut cache = DistributedModCache::open(&root).unwrap();
        assert!(cache.install(encoded, [7; 32]).is_err());
        assert!(!cache.contains(hash));
    }

    #[test]
    fn invalid_finished_partial_is_discarded_before_retry() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy();
        let (_, hash) = package("Expected");
        let (encoded, _) = package("Different valid package");
        let mut cache = DistributedModCache::open(&root).unwrap();
        cache
            .append_chunk(hash, encoded.len() as u64, 0, &encoded)
            .unwrap();
        assert!(cache.finish_partial(hash, encoded.len() as u64).is_err());
        assert_eq!(cache.resume_offset(hash, encoded.len() as u64).unwrap(), 0);
    }

    #[test]
    fn corrupt_complete_entry_is_evicted_and_restarts_from_zero() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy();
        let (encoded, hash) = package("Corrupt");
        let mut cache = DistributedModCache::open(&root).unwrap();
        drop(cache.install(encoded.clone(), hash).unwrap());
        let path = cache.complete_path(&hash);
        let mut corrupt = encoded.clone();
        let corrupt_index = corrupt.len() / 2;
        corrupt[corrupt_index] ^= 0xa5;
        std::fs::write(&path, corrupt).unwrap();

        assert_eq!(cache.resume_offset(hash, encoded.len() as u64).unwrap(), 0);
        assert!(!cache.contains(hash));
        assert!(!path.exists());
    }

    #[test]
    fn valid_complete_entry_rejects_a_changed_length_for_the_same_hash() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy();
        let (encoded, hash) = package("Exact");
        let mut cache = DistributedModCache::open(&root).unwrap();
        drop(cache.install(encoded.clone(), hash).unwrap());

        let error = cache
            .resume_offset(hash, encoded.len() as u64 + 1)
            .unwrap_err();
        assert!(error.contains("same immutable hash"));
        assert!(cache.contains(hash));
        assert!(cache.complete_path(&hash).is_file());
    }

    #[test]
    fn reopening_repairs_a_truncated_complete_entry() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy();
        let (encoded, hash) = package("Truncated");
        let mut cache = DistributedModCache::open(&root).unwrap();
        drop(cache.install(encoded.clone(), hash).unwrap());
        let complete = cache.complete_path(&hash);
        std::fs::File::options()
            .write(true)
            .open(&complete)
            .unwrap()
            .set_len(encoded.len() as u64 - 1)
            .unwrap();
        drop(cache);

        let mut reopened = DistributedModCache::open(&root).unwrap();
        assert!(!reopened.contains(hash));
        assert!(!complete.exists());
        assert_eq!(
            reopened.resume_offset(hash, encoded.len() as u64).unwrap(),
            0
        );
    }

    #[test]
    fn oversized_partial_is_discarded_and_restarts_from_zero() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy();
        let mut cache = DistributedModCache::open(&root).unwrap();
        let hash = [3; 32];
        let partial = cache.partial_path(&hash);
        std::fs::File::create(&partial).unwrap().set_len(2).unwrap();
        assert_eq!(cache.resume_offset(hash, 1).unwrap(), 0);
        assert!(!partial.exists());
    }

    #[cfg(unix)]
    #[test]
    fn partial_symlink_is_removed_without_touching_its_target() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), b"outside").unwrap();
        let root = temp.path().to_string_lossy();
        let mut cache = DistributedModCache::open(&root).unwrap();
        let hash = [4; 32];
        let partial = cache.partial_path(&hash);
        symlink(outside.path(), &partial).unwrap();

        assert_eq!(cache.resume_offset(hash, 1).unwrap(), 0);
        assert!(std::fs::symlink_metadata(&partial).is_err());
        assert_eq!(std::fs::read(outside.path()).unwrap(), b"outside");
    }

    #[test]
    fn partial_staging_uses_bounded_lru_space_reclamation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy();
        let mut cache = DistributedModCache::open(&root).unwrap();
        for value in 0..DISTRIBUTED_MOD_CACHE_ENTRY_LIMIT {
            cache
                .append_chunk([value as u8; 32], 1, 0, &[value as u8])
                .unwrap();
        }
        cache
            .append_chunk([0xff; 32], 1, 0, &[0xff])
            .expect("new transfer evicts the oldest resumable partial");
        assert!(!cache.partial_path(&[0; 32]).exists());
        assert!(cache.partial_path(&[0xff; 32]).exists());

        let bytes_temp = tempfile::tempdir().unwrap();
        let bytes_root = bytes_temp.path().to_string_lossy();
        let mut bytes_cache = DistributedModCache::open(&bytes_root).unwrap();
        let full = bytes_cache.partial_path(&[1; 32]);
        std::fs::File::create(&full)
            .unwrap()
            .set_len(DISTRIBUTED_MOD_CACHE_BYTE_LIMIT)
            .unwrap();
        bytes_cache
            .append_chunk([2; 32], 1, 0, &[1])
            .expect("new transfer evicts an older partial to stay under the byte cap");
        assert!(!full.exists());
        assert!(bytes_cache.partial_path(&[2; 32]).exists());
    }

    #[test]
    fn reopening_prunes_legacy_partials_to_the_aggregate_byte_limit() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy();
        let cache = DistributedModCache::open(&root).unwrap();
        let oldest = cache.partial_path(&[1; 32]);
        let newest = cache.partial_path(&[2; 32]);
        std::fs::File::create(&oldest)
            .unwrap()
            .set_len(DISTRIBUTED_MOD_CACHE_BYTE_LIMIT)
            .unwrap();
        std::fs::write(&newest, [1]).unwrap();
        drop(cache);

        let _reopened = DistributedModCache::open(&root).unwrap();
        assert!(!oldest.exists());
        assert!(newest.exists());
    }

    #[test]
    fn staging_evicts_the_unpinned_lru_complete_entry() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy();
        let mut cache = DistributedModCache::open(&root).unwrap();
        let mut installed = Vec::new();
        let mut pinned = None;
        for value in 0..DISTRIBUTED_MOD_CACHE_ENTRY_LIMIT {
            let (encoded, hash) = package(&format!("Complete {value:02}"));
            let lease = cache.install(encoded, hash).unwrap();
            if value == 0 {
                pinned = Some(lease);
            } else {
                drop(lease);
            }
            installed.push(hash);
        }
        assert!(cache.contains(installed[0]));

        cache.append_chunk([0xfe; 32], 1, 0, &[1]).unwrap();
        assert!(cache.contains(installed[0]), "mounted entry remains pinned");
        assert!(
            !cache.contains(installed[1]),
            "oldest unpinned entry is LRU"
        );
        assert!(cache.partial_path(&[0xfe; 32]).exists());
        drop(pinned);
    }
}
