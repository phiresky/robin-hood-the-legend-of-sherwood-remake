//! Durable browser cache and resumable staging for host-distributed mods.
//!
//! IndexedDB stores immutable canonical envelopes and independent sequential
//! chunks. Every complete read is decoded and hash-validated before use; cache
//! presence never grants trust. Chunk records make resumption durable without
//! repeatedly rewriting an ever-growing blob.

use crate::distributed_mod::{
    DISTRIBUTED_MOD_ENCODED_LIMIT, DistributedModPackage, ValidatedDistributedMod,
};
use js_sys::Uint8Array;
use rexie::{ObjectStore, Rexie, Store, TransactionMode};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Arc;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use crate::distributed_mod_policy::{CacheIndexEntry, validate_chunk, validate_transfer_total};
pub use crate::distributed_mod_policy::{
    DISTRIBUTED_MOD_CACHE_BYTE_LIMIT, DISTRIBUTED_MOD_CACHE_ENTRY_LIMIT,
    DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION, DISTRIBUTED_MOD_TRANSFER_CHUNK_LIMIT,
};
const PARTIAL_ENTRY_LIMIT: usize = 4;
const PARTIAL_CHUNK_LIMIT: usize =
    DISTRIBUTED_MOD_ENCODED_LIMIT.div_ceil(DISTRIBUTED_MOD_TRANSFER_CHUNK_LIMIT) + 1;

const DATABASE: &str = "robinhood-distributed-mod-cache";
const PACKAGES: &str = "packages";
const CHUNKS: &str = "chunks";
const METADATA: &str = "metadata";
const INDEX_KEY: &str = "index-v1";

thread_local! {
    static PERSIST_REQUESTED: Cell<bool> = const { Cell::new(false) };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PartialIndexEntry {
    total_bytes: u64,
    next_offset: u64,
    last_used: u64,
    chunk_offsets: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheIndex {
    schema_version: u32,
    access_counter: u64,
    entries: BTreeMap<String, CacheIndexEntry>,
    partials: BTreeMap<String, PartialIndexEntry>,
}

impl Default for CacheIndex {
    fn default() -> Self {
        Self {
            schema_version: DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION,
            access_counter: 0,
            entries: BTreeMap::new(),
            partials: BTreeMap::new(),
        }
    }
}

impl CacheIndex {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported browser distributed-mod cache schema {}; expected {DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        if self.entries.len() > DISTRIBUTED_MOD_CACHE_ENTRY_LIMIT
            || self.partials.len() > PARTIAL_ENTRY_LIMIT
        {
            return Err("browser distributed-mod cache index exceeds its entry limits".to_owned());
        }
        let mut total = 0_u64;
        for (hash, entry) in &self.entries {
            validate_hash(hash)?;
            validate_transfer_total(entry.encoded_bytes)?;
            total = total
                .checked_add(entry.encoded_bytes)
                .ok_or_else(|| "browser cache byte accounting overflow".to_owned())?;
        }
        for (hash, partial) in &self.partials {
            validate_hash(hash)?;
            validate_transfer_total(partial.total_bytes)?;
            if partial.next_offset > partial.total_bytes
                || partial.chunk_offsets.len() > PARTIAL_CHUNK_LIMIT
                || !partial
                    .chunk_offsets
                    .windows(2)
                    .all(|pair| pair[0] < pair[1])
            {
                return Err(format!("invalid browser cache partial metadata for {hash}"));
            }
            total = total
                .checked_add(partial.next_offset)
                .ok_or_else(|| "browser cache byte accounting overflow".to_owned())?;
        }
        if total > DISTRIBUTED_MOD_CACHE_BYTE_LIMIT {
            return Err(format!(
                "browser distributed-mod cache declares {total} bytes; limit is {DISTRIBUTED_MOD_CACHE_BYTE_LIMIT}"
            ));
        }
        Ok(())
    }
}

/// Exact validated package retained in memory for the admission/mount lifetime.
#[derive(Debug)]
pub struct DistributedModCacheLease {
    pub validated: ValidatedDistributedMod,
    encoded: Arc<[u8]>,
}

impl DistributedModCacheLease {
    pub fn encoded(&self) -> &[u8] {
        &self.encoded
    }

    pub fn encoded_arc(&self) -> Arc<[u8]> {
        Arc::clone(&self.encoded)
    }
}

async fn database() -> Result<Rexie, String> {
    request_persistent_storage_once().await;
    Rexie::builder(DATABASE)
        .version(DISTRIBUTED_MOD_CACHE_SCHEMA_VERSION)
        .add_object_store(ObjectStore::new(PACKAGES))
        .add_object_store(ObjectStore::new(CHUNKS))
        .add_object_store(ObjectStore::new(METADATA))
        .build()
        .await
        .map_err(|error| format!("open browser distributed-mod IndexedDB: {error}"))
}

/// Ask the user agent to keep this origin's IndexedDB durable when its policy
/// permits it. `false` is not an admission failure—browser persistence is a
/// best-effort hint—but every actual IndexedDB/quota failure still propagates
/// and fails the transfer closed.
async fn request_persistent_storage_once() {
    let already_requested = PERSIST_REQUESTED.with(|requested| requested.replace(true));
    if already_requested {
        return;
    }
    let Some(window) = web_sys::window() else {
        tracing::warn!("cannot request persistent browser mod storage without a Window");
        return;
    };
    let request = match window.navigator().storage().persist() {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!(
                ?error,
                "browser refused to start persistent-storage request; continuing with fail-closed IndexedDB operations"
            );
            return;
        }
    };
    match JsFuture::from(request).await {
        Ok(value) if value.as_bool() == Some(true) => {
            tracing::info!("browser granted persistent distributed-mod cache storage")
        }
        Ok(_) => tracing::warn!(
            "browser kept distributed-mod cache as best-effort storage; validated entries may be evicted"
        ),
        Err(error) => tracing::warn!(
            ?error,
            "browser persistent-storage request failed; continuing with fail-closed IndexedDB operations"
        ),
    }
}

async fn read_index(store: &Store) -> Result<CacheIndex, String> {
    let index = match store
        .get(JsValue::from_str(INDEX_KEY))
        .await
        .map_err(|error| format!("read browser cache index: {error}"))?
    {
        None => CacheIndex::default(),
        Some(value) => {
            let json = value
                .as_string()
                .ok_or_else(|| "browser cache index is not a JSON string".to_owned())?;
            serde_json::from_str(&json)
                .map_err(|error| format!("parse browser cache index: {error}"))?
        }
    };
    index.validate()?;
    Ok(index)
}

async fn write_index(store: &Store, index: &CacheIndex) -> Result<(), String> {
    index.validate()?;
    let json = serde_json::to_string(index)
        .map_err(|error| format!("encode browser cache index: {error}"))?;
    store
        .put(
            &JsValue::from_str(&json),
            Some(&JsValue::from_str(INDEX_KEY)),
        )
        .await
        .map_err(|error| format!("write browser cache index: {error}"))?;
    Ok(())
}

fn bytes_to_js(bytes: &[u8]) -> JsValue {
    Uint8Array::from(bytes).into()
}

fn bytes_from_js(value: JsValue, label: &str) -> Result<Vec<u8>, String> {
    let array = value
        .dyn_into::<Uint8Array>()
        .map_err(|_| format!("{label} is not an IndexedDB byte array"))?;
    let len = usize::try_from(array.length())
        .map_err(|_| format!("{label} length does not fit this platform"))?;
    if len > DISTRIBUTED_MOD_ENCODED_LIMIT {
        return Err(format!(
            "{label} is {len} bytes; limit is {DISTRIBUTED_MOD_ENCODED_LIMIT}"
        ));
    }
    let mut bytes = vec![0; len];
    array.copy_to(&mut bytes);
    Ok(bytes)
}

fn hex_hash(hash: &[u8; 32]) -> String {
    robin_engine::spellforge::hex_hash(hash)
}

fn validate_hash(value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid distributed-mod cache hash `{value}`"));
    }
    Ok(())
}

fn chunk_key(hash: &str, offset: u64) -> String {
    format!("{hash}:{offset:020}")
}

/// Validate and acquire one complete immutable cache entry.
pub async fn acquire(
    full_mod_sha256: [u8; 32],
) -> Result<Option<DistributedModCacheLease>, String> {
    match acquire_inner(full_mod_sha256).await {
        Ok(lease) => Ok(lease),
        Err(corruption) => {
            let key = hex_hash(&full_mod_sha256);
            purge_cached_hash(&key).await.map_err(|cleanup| {
                format!(
                    "browser cache entry {key} is unusable ({corruption}); atomic cleanup failed: {cleanup}"
                )
            })?;
            tracing::warn!(%corruption, %key, "discarded unusable browser distributed-mod cache entry");
            Ok(None)
        }
    }
}

async fn acquire_inner(
    full_mod_sha256: [u8; 32],
) -> Result<Option<DistributedModCacheLease>, String> {
    let key = hex_hash(&full_mod_sha256);
    let db = database().await?;
    let transaction = db
        .transaction(&[PACKAGES, METADATA], TransactionMode::ReadWrite)
        .map_err(|error| format!("start browser cache acquire transaction: {error}"))?;
    let packages = transaction
        .store(PACKAGES)
        .map_err(|error| format!("open browser package store: {error}"))?;
    let metadata = transaction
        .store(METADATA)
        .map_err(|error| format!("open browser cache metadata: {error}"))?;
    let mut index = read_index(&metadata).await?;
    let Some(entry) = index.entries.get(&key).cloned() else {
        drop(packages);
        drop(metadata);
        transaction
            .done()
            .await
            .map_err(|error| browser_storage_error("finish empty cache acquire", error))?;
        return Ok(None);
    };
    let value = packages
        .get(JsValue::from_str(&key))
        .await
        .map_err(|error| format!("read browser cached package {key}: {error}"))?
        .ok_or_else(|| format!("browser cache index references missing package {key}"))?;
    let encoded = bytes_from_js(value, "browser cached distributed mod")?;
    let validated = crate::distributed_mod_policy::validate_cached_package(
        &encoded,
        full_mod_sha256,
        entry.encoded_bytes,
    )?;
    index.access_counter = index.access_counter.saturating_add(1);
    index
        .entries
        .get_mut(&key)
        .expect("cache entry disappeared during acquire")
        .last_used = index.access_counter;
    write_index(&metadata, &index).await?;
    transaction
        .done()
        .await
        .map_err(|error| format!("commit browser cache acquire: {error}"))?;
    Ok(Some(DistributedModCacheLease {
        validated,
        encoded: Arc::from(encoded),
    }))
}

/// Return a byte prefix only after verifying every indexed durable chunk.
pub async fn resume_offset(full_mod_sha256: [u8; 32], total_bytes: u64) -> Result<u64, String> {
    validate_transfer_total(total_bytes)?;
    match inspect_resume_offset(full_mod_sha256, total_bytes).await {
        Ok(offset) => Ok(offset),
        Err(corruption) => {
            let key = hex_hash(&full_mod_sha256);
            purge_cached_hash(&key).await.map_err(|cleanup| {
                format!(
                    "browser cache state for {key} is inconsistent ({corruption}); atomic reset failed: {cleanup}"
                )
            })?;
            tracing::warn!(%corruption, %key, "reset inconsistent browser distributed-mod cache state");
            Ok(0)
        }
    }
}

async fn inspect_resume_offset(full_mod_sha256: [u8; 32], total_bytes: u64) -> Result<u64, String> {
    let key = hex_hash(&full_mod_sha256);
    let db = database().await?;
    let transaction = db
        .transaction(&[PACKAGES, CHUNKS, METADATA], TransactionMode::ReadOnly)
        .map_err(|error| format!("start browser cache resume transaction: {error}"))?;
    let packages = transaction
        .store(PACKAGES)
        .map_err(|error| error.to_string())?;
    let chunks = transaction
        .store(CHUNKS)
        .map_err(|error| error.to_string())?;
    let metadata = transaction
        .store(METADATA)
        .map_err(|error| error.to_string())?;
    let index = read_index(&metadata).await?;
    if let Some(entry) = index.entries.get(&key) {
        if entry.encoded_bytes != total_bytes {
            return Err(format!(
                "complete browser cache entry has {} bytes; offer declares {total_bytes}",
                entry.encoded_bytes
            ));
        }
        let value = packages
            .get(JsValue::from_str(&key))
            .await
            .map_err(|error| format!("inspect complete browser cache entry: {error}"))?
            .ok_or_else(|| format!("browser cache index references missing package {key}"))?;
        let encoded = bytes_from_js(value, "complete browser cached distributed mod")?;
        crate::distributed_mod_policy::validate_cached_package(
            &encoded,
            full_mod_sha256,
            total_bytes,
        )?;
        drop(packages);
        drop(chunks);
        drop(metadata);
        transaction
            .done()
            .await
            .map_err(|error| browser_storage_error("finish cache resume read", error))?;
        return Ok(total_bytes);
    }
    let Some(partial) = index.partials.get(&key) else {
        drop(packages);
        drop(chunks);
        drop(metadata);
        transaction
            .done()
            .await
            .map_err(|error| browser_storage_error("finish empty cache resume read", error))?;
        return Ok(0);
    };
    if partial.total_bytes != total_bytes {
        return Err(format!(
            "partial browser cache declares {} bytes; offer declares {total_bytes}",
            partial.total_bytes
        ));
    }
    let mut expected = 0_u64;
    for &offset in &partial.chunk_offsets {
        if offset != expected {
            return Err(format!("browser cache partial has a gap at {expected}"));
        }
        let value = chunks
            .get(JsValue::from_str(&chunk_key(&key, offset)))
            .await
            .map_err(|error| format!("read browser cache chunk at {offset}: {error}"))?
            .ok_or_else(|| format!("browser cache partial is missing chunk at {offset}"))?;
        let bytes = bytes_from_js(value, "browser cache partial chunk")?;
        let len = bytes.len() as u64;
        if len == 0 || len > DISTRIBUTED_MOD_TRANSFER_CHUNK_LIMIT as u64 {
            return Err(format!(
                "browser cache chunk at {offset} has invalid length {len}"
            ));
        }
        expected = expected
            .checked_add(len)
            .ok_or_else(|| "browser cache partial offset overflow".to_owned())?;
    }
    if expected != partial.next_offset {
        return Err(format!(
            "browser cache chunks end at {expected}; index declares {}",
            partial.next_offset
        ));
    }
    drop(packages);
    drop(chunks);
    drop(metadata);
    transaction
        .done()
        .await
        .map_err(|error| browser_storage_error("finish partial cache resume read", error))?;
    Ok(expected)
}

/// Atomically discard one unusable hash. If the global index itself cannot be
/// trusted, the whole disposable cache is reset so corruption cannot poison
/// every future admission attempt.
async fn purge_cached_hash(key: &str) -> Result<(), String> {
    let db = database().await?;
    let transaction = db
        .transaction(&[PACKAGES, CHUNKS, METADATA], TransactionMode::ReadWrite)
        .map_err(|error| format!("start browser cache repair: {error}"))?;
    let packages = transaction
        .store(PACKAGES)
        .map_err(|error| error.to_string())?;
    let chunks = transaction
        .store(CHUNKS)
        .map_err(|error| error.to_string())?;
    let metadata = transaction
        .store(METADATA)
        .map_err(|error| error.to_string())?;

    match read_index(&metadata).await {
        Ok(mut index) => {
            index.entries.remove(key);
            let offsets = index
                .partials
                .remove(key)
                .map(|partial| partial.chunk_offsets)
                .unwrap_or_default();
            packages
                .delete(JsValue::from_str(key))
                .await
                .map_err(|error| browser_storage_error("remove corrupt cached package", error))?;
            for offset in offsets {
                chunks
                    .delete(JsValue::from_str(&chunk_key(key, offset)))
                    .await
                    .map_err(|error| browser_storage_error("remove corrupt cache chunk", error))?;
            }
            write_index(&metadata, &index).await?;
        }
        Err(index_error) => {
            tracing::warn!(%index_error, "browser distributed-mod cache index is corrupt; resetting disposable cache");
            packages
                .clear()
                .await
                .map_err(|error| browser_storage_error("reset corrupt package store", error))?;
            chunks
                .clear()
                .await
                .map_err(|error| browser_storage_error("reset corrupt chunk store", error))?;
            metadata
                .clear()
                .await
                .map_err(|error| browser_storage_error("reset corrupt metadata store", error))?;
            write_index(&metadata, &CacheIndex::default()).await?;
        }
    }
    transaction
        .done()
        .await
        .map(|_| ())
        .map_err(|error| browser_storage_error("commit browser cache repair", error))
}

/// Append one sequential chunk in a single IndexedDB transaction.
pub async fn append_chunk(
    full_mod_sha256: [u8; 32],
    total_bytes: u64,
    offset: u64,
    chunk: &[u8],
) -> Result<u64, String> {
    let end = validate_chunk(total_bytes, offset, chunk.len())?;
    let key = hex_hash(&full_mod_sha256);
    let db = database().await?;
    let transaction = db
        .transaction(&[PACKAGES, CHUNKS, METADATA], TransactionMode::ReadWrite)
        .map_err(|error| format!("start browser cache append transaction: {error}"))?;
    let packages = transaction
        .store(PACKAGES)
        .map_err(|error| error.to_string())?;
    let chunks = transaction
        .store(CHUNKS)
        .map_err(|error| error.to_string())?;
    let metadata = transaction
        .store(METADATA)
        .map_err(|error| error.to_string())?;
    let mut index = read_index(&metadata).await?;
    if index.entries.contains_key(&key) {
        return Err(format!(
            "cannot append partial bytes over complete browser cache entry {key}"
        ));
    }
    let stale_partial =
        if !index.partials.contains_key(&key) && index.partials.len() >= PARTIAL_ENTRY_LIMIT {
            let stale = index
                .partials
                .iter()
                .min_by_key(|(hash, entry)| (entry.last_used, *hash))
                .map(|(hash, _)| hash.clone())
                .expect("non-empty partial cache must have an LRU candidate");
            let stale_entry = index.partials.remove(&stale).expect("stale partial exists");
            Some((stale, stale_entry.chunk_offsets))
        } else {
            None
        };
    index.access_counter = index.access_counter.saturating_add(1);
    let access_counter = index.access_counter;
    let partial = index
        .partials
        .entry(key.clone())
        .or_insert_with(|| PartialIndexEntry {
            total_bytes,
            next_offset: 0,
            last_used: access_counter,
            chunk_offsets: Vec::new(),
        });
    if partial.total_bytes != total_bytes || partial.next_offset != offset {
        return Err(format!(
            "distributed-mod chunk starts at {offset}/{total_bytes}; durable browser partial ends at {}/{}",
            partial.next_offset, partial.total_bytes
        ));
    }
    if partial.chunk_offsets.len() >= PARTIAL_CHUNK_LIMIT {
        return Err("browser distributed-mod partial exceeds its chunk limit".to_owned());
    }
    partial.chunk_offsets.push(offset);
    partial.next_offset = end;
    partial.last_used = access_counter;
    let evicted_packages = select_complete_evictions_to_limits(&mut index, Some(&key))?;
    index.validate()?;
    if let Some((stale, offsets)) = stale_partial {
        for stale_offset in offsets {
            chunks
                .delete(JsValue::from_str(&chunk_key(&stale, stale_offset)))
                .await
                .map_err(|error| browser_storage_error("evict stale cache chunk", error))?;
        }
    }
    for candidate in evicted_packages {
        packages
            .delete(JsValue::from_str(&candidate))
            .await
            .map_err(|error| browser_storage_error("evict cached package", error))?;
    }
    chunks
        .put(
            &bytes_to_js(chunk),
            Some(&JsValue::from_str(&chunk_key(&key, offset))),
        )
        .await
        .map_err(|error| browser_storage_error("persist cache chunk", error))?;
    write_index(&metadata, &index).await?;
    transaction
        .done()
        .await
        .map_err(|error| browser_storage_error("commit cache chunk", error))?;
    Ok(end)
}

fn select_complete_evictions_to_limits(
    index: &mut CacheIndex,
    protected: Option<&str>,
) -> Result<Vec<String>, String> {
    let staged_bytes = index
        .partials
        .values()
        .map(|entry| entry.next_offset)
        .try_fold(0u64, u64::checked_add)
        .ok_or_else(|| "browser cache byte accounting overflow".to_owned())?;
    let protected = protected.into_iter().map(str::to_owned).collect();
    let evicted =
        crate::distributed_mod_policy::select_evictions(&index.entries, &protected, staged_bytes)?;
    for hash in &evicted {
        index.entries.remove(hash);
    }
    Ok(evicted)
}

fn browser_storage_error(operation: &str, error: impl std::fmt::Display) -> String {
    let detail = error.to_string();
    if detail.contains("QuotaExceededError") || detail.to_ascii_lowercase().contains("quota") {
        format!(
            "browser storage quota was exceeded while attempting to {operation}; the mod was not admitted ({detail})"
        )
    } else {
        format!("browser IndexedDB failed to {operation}; the mod was not admitted ({detail})")
    }
}

/// Assemble, validate, and atomically promote a complete partial.
pub async fn finish_partial(
    full_mod_sha256: [u8; 32],
    total_bytes: u64,
) -> Result<DistributedModCacheLease, String> {
    validate_transfer_total(total_bytes)?;
    match finish_partial_inner(full_mod_sha256, total_bytes).await {
        Ok(lease) => Ok(lease),
        Err(corruption) => {
            let key = hex_hash(&full_mod_sha256);
            purge_cached_hash(&key).await.map_err(|cleanup| {
                format!(
                    "downloaded browser cache partial {key} is unusable ({corruption}); atomic cleanup failed: {cleanup}"
                )
            })?;
            Err(format!(
                "downloaded browser cache partial {key} was discarded: {corruption}"
            ))
        }
    }
}

async fn finish_partial_inner(
    full_mod_sha256: [u8; 32],
    total_bytes: u64,
) -> Result<DistributedModCacheLease, String> {
    let key = hex_hash(&full_mod_sha256);
    let db = database().await?;
    let read = db
        .transaction(&[CHUNKS, METADATA], TransactionMode::ReadOnly)
        .map_err(|error| format!("start browser cache finish read: {error}"))?;
    let chunks = read.store(CHUNKS).map_err(|error| error.to_string())?;
    let metadata = read.store(METADATA).map_err(|error| error.to_string())?;
    let index = read_index(&metadata).await?;
    let partial = index
        .partials
        .get(&key)
        .cloned()
        .ok_or_else(|| format!("no browser cache partial exists for {key}"))?;
    if partial.total_bytes != total_bytes || partial.next_offset != total_bytes {
        return Err(format!(
            "browser cache partial ends at {}/{}; expected {total_bytes}",
            partial.next_offset, partial.total_bytes
        ));
    }
    let capacity = usize::try_from(total_bytes)
        .map_err(|_| "distributed mod is too large for this browser".to_owned())?;
    let mut encoded = Vec::with_capacity(capacity);
    for &offset in &partial.chunk_offsets {
        if encoded.len() as u64 != offset {
            return Err(format!(
                "browser cache partial has a gap at byte {}",
                encoded.len()
            ));
        }
        let value = chunks
            .get(JsValue::from_str(&chunk_key(&key, offset)))
            .await
            .map_err(|error| format!("read browser cache chunk at {offset}: {error}"))?
            .ok_or_else(|| format!("missing browser cache chunk at {offset}"))?;
        encoded.extend_from_slice(&bytes_from_js(value, "browser cache chunk")?);
    }
    if encoded.len() as u64 != total_bytes {
        return Err(format!(
            "assembled browser cache package is {} bytes; expected {total_bytes}",
            encoded.len()
        ));
    }
    drop(chunks);
    drop(metadata);
    read.done()
        .await
        .map_err(|error| browser_storage_error("finish cache promotion read", error))?;
    let validated = DistributedModPackage::decode(&encoded)
        .map_err(|error| format!("validate downloaded browser distributed mod: {error}"))?;
    if validated.package.manifest.full_mod_sha256 != full_mod_sha256 {
        return Err("downloaded browser full-mod hash differs from its offer".to_owned());
    }

    let write = db
        .transaction(&[PACKAGES, CHUNKS, METADATA], TransactionMode::ReadWrite)
        .map_err(|error| format!("start browser cache promotion: {error}"))?;
    let packages = write.store(PACKAGES).map_err(|error| error.to_string())?;
    let chunks = write.store(CHUNKS).map_err(|error| error.to_string())?;
    let metadata = write.store(METADATA).map_err(|error| error.to_string())?;
    let mut current = read_index(&metadata).await?;
    let current_partial = current
        .partials
        .get(&key)
        .ok_or_else(|| "browser cache partial changed during validation".to_owned())?;
    if current_partial.next_offset != total_bytes
        || current_partial.chunk_offsets != partial.chunk_offsets
    {
        return Err("browser cache partial changed during validation".to_owned());
    }
    current.partials.remove(&key);
    current.access_counter = current.access_counter.saturating_add(1);
    current.entries.insert(
        key.clone(),
        CacheIndexEntry {
            encoded_bytes: total_bytes,
            last_used: current.access_counter,
        },
    );
    let evicted_packages = select_complete_evictions_to_limits(&mut current, Some(&key))?;
    current.validate()?;
    packages
        .put(&bytes_to_js(&encoded), Some(&JsValue::from_str(&key)))
        .await
        .map_err(|error| browser_storage_error("persist complete mod", error))?;
    for &offset in &partial.chunk_offsets {
        chunks
            .delete(JsValue::from_str(&chunk_key(&key, offset)))
            .await
            .map_err(|error| format!("remove promoted browser cache chunk: {error}"))?;
    }
    for candidate in evicted_packages {
        packages
            .delete(JsValue::from_str(&candidate))
            .await
            .map_err(|error| browser_storage_error("evict cached package", error))?;
    }
    write_index(&metadata, &current).await?;
    write
        .done()
        .await
        .map_err(|error| browser_storage_error("commit complete mod", error))?;
    Ok(DistributedModCacheLease {
        validated,
        encoded: Arc::from(encoded),
    })
}

/// Remove every complete and partial cached package. Trust grants are separate.
pub async fn clear() -> Result<usize, String> {
    let db = database().await?;
    let transaction = db
        .transaction(&[PACKAGES, CHUNKS, METADATA], TransactionMode::ReadWrite)
        .map_err(|error| format!("start browser cache clear: {error}"))?;
    let packages = transaction
        .store(PACKAGES)
        .map_err(|error| error.to_string())?;
    let chunks = transaction
        .store(CHUNKS)
        .map_err(|error| error.to_string())?;
    let metadata = transaction
        .store(METADATA)
        .map_err(|error| error.to_string())?;
    let removed = match read_index(&metadata).await {
        Ok(index) => index.entries.len(),
        Err(error) => {
            tracing::warn!(%error, "clearing corrupt browser distributed-mod cache metadata");
            0
        }
    };
    packages.clear().await.map_err(|error| error.to_string())?;
    chunks.clear().await.map_err(|error| error.to_string())?;
    metadata.clear().await.map_err(|error| error.to_string())?;
    write_index(&metadata, &CacheIndex::default()).await?;
    transaction
        .done()
        .await
        .map_err(|error| format!("commit browser cache clear: {error}"))?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn entry(bytes: u64, last_used: u64) -> CacheIndexEntry {
        CacheIndexEntry {
            encoded_bytes: bytes,
            last_used,
        }
    }

    #[wasm_bindgen_test]
    fn lru_selection_is_deterministic_and_protects_current_hash() {
        let mut index = CacheIndex::default();
        index
            .entries
            .insert("00".repeat(32), entry(128 * 1024 * 1024, 2));
        index
            .entries
            .insert("11".repeat(32), entry(128 * 1024 * 1024, 1));
        for (value, last_used) in [(0x22, 3), (0x33, 4), (0x44, 5)] {
            index.entries.insert(
                format!("{value:02x}").repeat(32),
                entry(128 * 1024 * 1024, last_used),
            );
        }
        let protected = "00".repeat(32);
        let evicted = select_complete_evictions_to_limits(&mut index, Some(&protected)).unwrap();
        assert_eq!(evicted, vec!["11".repeat(32)]);
        assert!(index.entries.contains_key(&protected));
        index.validate().unwrap();
    }

    #[wasm_bindgen_test]
    fn cache_index_rejects_oversized_partial_accounting() {
        let mut index = CacheIndex::default();
        for value in 0..PARTIAL_ENTRY_LIMIT {
            index.partials.insert(
                format!("{value:064x}"),
                PartialIndexEntry {
                    total_bytes: DISTRIBUTED_MOD_ENCODED_LIMIT as u64,
                    next_offset: DISTRIBUTED_MOD_ENCODED_LIMIT as u64,
                    last_used: value as u64,
                    chunk_offsets: Vec::new(),
                },
            );
        }
        assert!(index.validate().unwrap_err().contains("limit"));
    }

    #[wasm_bindgen_test]
    fn quota_diagnostic_is_explicit_and_fail_closed() {
        let diagnostic = browser_storage_error("commit package", "QuotaExceededError");
        assert!(diagnostic.contains("quota"));
        assert!(diagnostic.contains("not admitted"));
    }

    async fn install_index(
        index: &CacheIndex,
        chunk: Option<(&str, u64, &[u8])>,
    ) -> Result<(), String> {
        clear().await?;
        let db = database().await?;
        let transaction = db
            .transaction(&[CHUNKS, METADATA], TransactionMode::ReadWrite)
            .map_err(|error| error.to_string())?;
        let chunks = transaction
            .store(CHUNKS)
            .map_err(|error| error.to_string())?;
        let metadata = transaction
            .store(METADATA)
            .map_err(|error| error.to_string())?;
        if let Some((hash, offset, bytes)) = chunk {
            chunks
                .put(
                    &bytes_to_js(bytes),
                    Some(&JsValue::from_str(&chunk_key(hash, offset))),
                )
                .await
                .map_err(|error| error.to_string())?;
        }
        write_index(&metadata, index).await?;
        transaction
            .done()
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    #[wasm_bindgen_test]
    async fn missing_complete_record_is_atomically_discarded_and_restarts() {
        let hash = [0x31; 32];
        let key = hex_hash(&hash);
        let mut index = CacheIndex::default();
        index.entries.insert(key, entry(8, 1));
        install_index(&index, None).await.unwrap();

        assert_eq!(resume_offset(hash, 8).await.unwrap(), 0);
        assert!(acquire(hash).await.unwrap().is_none());
        clear().await.unwrap();
    }

    #[wasm_bindgen_test]
    async fn missing_partial_chunk_is_atomically_discarded_and_restarts() {
        let hash = [0x32; 32];
        let key = hex_hash(&hash);
        let mut index = CacheIndex::default();
        index.partials.insert(
            key,
            PartialIndexEntry {
                total_bytes: 8,
                next_offset: 4,
                last_used: 1,
                chunk_offsets: vec![0],
            },
        );
        install_index(&index, None).await.unwrap();

        assert_eq!(resume_offset(hash, 8).await.unwrap(), 0);
        assert_eq!(resume_offset(hash, 8).await.unwrap(), 0);
        clear().await.unwrap();
    }

    #[wasm_bindgen_test]
    async fn invalid_completed_partial_is_removed_after_validation_failure() {
        let hash = [0x33; 32];
        let key = hex_hash(&hash);
        let mut index = CacheIndex::default();
        index.partials.insert(
            key.clone(),
            PartialIndexEntry {
                total_bytes: 4,
                next_offset: 4,
                last_used: 1,
                chunk_offsets: vec![0],
            },
        );
        install_index(&index, Some((&key, 0, &[1, 2, 3, 4])))
            .await
            .unwrap();

        assert!(finish_partial(hash, 4).await.is_err());
        assert_eq!(resume_offset(hash, 4).await.unwrap(), 0);
        clear().await.unwrap();
    }
}
