//! IndexedDB holds replay history in fixed-size blocks. Synchronous recorder
//! flushes publish only a bounded write-ahead journal, so an urgent autosave
//! never references a marker that exists solely in an unawaited transaction.
//! Journal batches and the IndexedDB watermark commit together: recovery is
//! idempotent even if the page closes between commit and journal retirement.

use super::{MANIFEST, MAX_BYTES};
use anyhow::{Context, Result, ensure};
use base64::Engine as _;
use js_sys::Uint8Array;
use rexie::{ObjectStore, Rexie, Store, TransactionMode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    rc::Rc,
};
use wasm_bindgen::{JsCast, JsValue};

#[cfg(not(test))]
const DATABASE: &str = "robin-replays-v1";
#[cfg(test)]
const DATABASE: &str = "robin-replays-tests-v1";
const FILES: &str = "files";
const BLOCKS: &str = "blocks";
const COMMITS: &str = "commits";
const BLOCK_BYTES: usize = 64 * 1024;
const JOURNAL_BYTES: usize = 1024 * 1024;
const CACHE_BYTES: usize = 2 * MAX_BYTES;
#[cfg(not(test))]
const JOURNAL_PREFIX: &str = "robin:replay:journal:v1:";
#[cfg(test)]
const JOURNAL_PREFIX: &str = "robin:replay:testjournal:v1:";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileIndex {
    len: usize,
    revision: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    path: String,
    expected: Option<FileIndex>,
    offset: usize,
    encoded: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    sequence: u64,
    patches: Vec<Patch>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    writer: String,
    batches: Vec<Batch>,
}

#[derive(Serialize, Deserialize)]
struct CachedFile {
    bytes: Vec<u8>,
    index: Option<FileIndex>,
    dirty: Option<usize>,
}

// Live browser resources, not a serializable application snapshot.
struct Session {
    db: Rc<Rexie>,
    files: BTreeMap<PathBuf, CachedFile>,
    leases: BTreeMap<PathBuf, usize>,
    journal: Journal,
    next_sequence: u64,
    committing: bool,
    mission_sequence: u64,
    failure: Option<String>,
}

thread_local! { static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) }; }

fn db_error(error: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("browser replay IndexedDB: {error}")
}
fn storage() -> Result<web_sys::Storage> {
    web_sys::window()
        .context("replay journal requires a browser window")?
        .local_storage()
        .map_err(|e| anyhow::anyhow!("replay journal unavailable: {e:?}"))?
        .context("replay journal storage is disabled")
}
fn with_session<T>(f: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
    SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot
            .as_mut()
            .context("browser replay storage was not initialized")?;
        if let Some(error) = &session.failure {
            anyhow::bail!("browser replay recording failed: {error}");
        }
        f(session)
    })
}
fn journal_key(journal: &Journal) -> String {
    format!("{JOURNAL_PREFIX}{}", journal.writer)
}
fn revision(writer: &str, sequence: u64) -> String {
    format!("{writer}:{sequence}")
}
fn block_key(path: &str, index: usize) -> JsValue {
    JsValue::from_str(&format!("{path}\0{index}"))
}
fn path_key(path: &Path) -> Result<String> {
    let key = path.to_str().context("replay path is not UTF-8")?;
    ensure!(
        !key.is_empty() && !key.contains('\0'),
        "invalid replay path"
    );
    Ok(key.to_owned())
}
async fn get_json<T: DeserializeOwned>(store: &Store, key: &str) -> Result<Option<T>> {
    store
        .get(JsValue::from_str(key))
        .await
        .map_err(db_error)?
        .map(|v| {
            serde_json::from_str(&v.as_string().context("replay metadata is not JSON text")?)
                .map_err(Into::into)
        })
        .transpose()
}
async fn put_json(store: &Store, key: &str, value: &impl Serialize) -> Result<()> {
    store
        .put(
            &JsValue::from_str(&serde_json::to_string(value)?),
            Some(&JsValue::from_str(key)),
        )
        .await
        .map_err(db_error)?;
    Ok(())
}
async fn block(store: &Store, path: &str, index: usize, expected: usize) -> Result<Vec<u8>> {
    let value = store
        .get(block_key(path, index))
        .await
        .map_err(db_error)?
        .context("missing replay storage block")?;
    let array = value
        .dyn_into::<Uint8Array>()
        .map_err(|_| anyhow::anyhow!("replay block is not bytes"))?;
    ensure!(
        array.length() as usize == expected && expected <= BLOCK_BYTES,
        "invalid replay block length"
    );
    Ok(array.to_vec())
}

impl Journal {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1
                && self.writer.len() == 32
                && self.writer.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid replay journal identity"
        );
        let mut previous = None;
        for batch in &self.batches {
            ensure!(
                batch.sequence > 0
                    && previous.is_none_or(|p: u64| p.checked_add(1) == Some(batch.sequence)),
                "replay journal sequence gap"
            );
            ensure!(!batch.patches.is_empty(), "empty replay journal batch");
            previous = Some(batch.sequence);
            for patch in &batch.patches {
                path_key(Path::new(&patch.path))?;
            }
        }
        Ok(())
    }
    fn persist(&self) -> Result<()> {
        self.validate()?;
        let key = journal_key(self);
        if self.batches.is_empty() {
            storage()?
                .remove_item(&key)
                .map_err(|e| anyhow::anyhow!("retire replay journal: {e:?}"))?;
        } else {
            let encoded = serde_json::to_string(self)?;
            ensure!(
                encoded.len() <= JOURNAL_BYTES,
                "pending browser replay journal exceeds {JOURNAL_BYTES} bytes; previous durable history is preserved"
            );
            storage()?
                .set_item(&key, &encoded)
                .map_err(|e| anyhow::anyhow!("persist replay recovery journal: {e:?}"))?;
        }
        Ok(())
    }
}

/// Open storage and recover interrupted commits before any recorder is created.
pub async fn initialize() -> Result<()> {
    if SESSION.with(|s| s.borrow().is_some()) {
        return Ok(());
    }
    let db = Rc::new(
        Rexie::builder(DATABASE)
            .version(1)
            .add_object_store(ObjectStore::new(FILES))
            .add_object_store(ObjectStore::new(BLOCKS))
            .add_object_store(ObjectStore::new(COMMITS))
            .build()
            .await
            .map_err(db_error)?,
    );
    let storage = storage()?;
    let mut pending = Vec::new();
    let mut legacy = Vec::new();
    for i in 0..storage
        .length()
        .map_err(|e| anyhow::anyhow!("enumerate replay journals: {e:?}"))?
    {
        if let Some(key) = storage
            .key(i)
            .map_err(|e| anyhow::anyhow!("read replay journal key: {e:?}"))?
        {
            if key.starts_with(JOURNAL_PREFIX) {
                pending.push(key);
            } else if let Some(path) = key.strip_prefix("robin:replay:")
                && (path.ends_with(".rhrec.jsonl")
                    || matches!(
                        Path::new(path).file_name().and_then(|p| p.to_str()),
                        Some(MANIFEST | "ranked.json")
                    ))
            {
                legacy.push(key);
            }
        }
    }
    // Stream legacy files directly into IndexedDB before needing any journal
    // quota. Do not preload every historical mission into the live cache.
    for key in legacy {
        let Some(encoded) = storage
            .get_item(&key)
            .map_err(|e| anyhow::anyhow!("read legacy replay: {e:?}"))?
        else {
            continue;
        };
        ensure!(encoded.len() <= MAX_BYTES, "oversized legacy replay file");
        let path = key
            .strip_prefix("robin:replay:")
            .expect("filtered legacy key");
        path_key(Path::new(path))?;
        match import_legacy(&db, path, encoded.as_bytes()).await {
            Ok(_) => retire_legacy(&key, &encoded)?,
            Err(error) => {
                // A newer IndexedDB writer or a failed import must not erase
                // the only remaining legacy copy or block unrelated missions.
                tracing::warn!(
                    path,
                    "Legacy replay retained after failed import: {error:#}"
                );
            }
        }
    }
    for key in pending {
        let Some(encoded) = storage
            .get_item(&key)
            .map_err(|e| anyhow::anyhow!("read replay journal: {e:?}"))?
        else {
            continue;
        };
        ensure!(
            encoded.len() <= JOURNAL_BYTES,
            "oversized replay recovery journal"
        );
        let journal: Journal = serde_json::from_str(&encoded)?;
        ensure!(
            journal_key(&journal) == key,
            "replay journal key disagrees with its identity"
        );
        commit(&db, &journal).await?;
        // Another tab may have appended while IndexedDB was committing.
        if storage
            .get_item(&key)
            .map_err(|e| anyhow::anyhow!("reread replay journal: {e:?}"))?
            .as_deref()
            == Some(&encoded)
        {
            storage
                .remove_item(&key)
                .map_err(|e| anyhow::anyhow!("retire recovered replay journal: {e:?}"))?;
        }
    }
    let mut nonce = [0u8; 16];
    getrandom_04::fill(&mut nonce).map_err(|e| anyhow::anyhow!("replay journal identity: {e}"))?;
    SESSION.with(|s| {
        *s.borrow_mut() = Some(Session {
            db,
            files: BTreeMap::new(),
            leases: BTreeMap::new(),
            journal: Journal {
                version: 1,
                writer: hex::encode(nonce),
                batches: Vec::new(),
            },
            next_sequence: 1,
            committing: false,
            mission_sequence: 0,
            failure: None,
        })
    });
    Ok(())
}

/// The synchronous durability boundary used by recorder flush and save capture.
pub(super) fn checkpoint() -> Result<()> {
    with_session(|s| {
        let mut patches = Vec::new();
        for (path, file) in &s.files {
            if let Some(offset) = file.dirty {
                patches.push(Patch {
                    path: path_key(path)?,
                    expected: file.index.clone(),
                    offset,
                    encoded: base64::engine::general_purpose::STANDARD
                        .encode(&file.bytes[offset..]),
                });
            }
        }
        if patches.is_empty() {
            return Ok(());
        }
        let next = s
            .next_sequence
            .checked_add(1)
            .context("replay journal sequence exhausted")?;
        let mut staged = s.journal.clone();
        staged.batches.push(Batch {
            sequence: s.next_sequence,
            patches,
        });
        staged.persist()?;
        for file in s.files.values_mut().filter(|file| file.dirty.is_some()) {
            file.index = Some(FileIndex {
                len: file.bytes.len(),
                revision: revision(&s.journal.writer, s.next_sequence),
            });
            file.dirty = None;
        }
        s.journal = staged;
        s.next_sequence = next;
        Ok(())
    })
}

/// Drain between frames; never retain a RefCell or engine lock across await.
pub async fn flush_pending() -> Result<()> {
    checkpoint()?;
    let (db, journal) = with_session(|s| {
        ensure!(!s.committing, "overlapping browser replay commits");
        s.committing = true;
        Ok((s.db.clone(), s.journal.clone()))
    })?;
    let _reset = CommitGuard;
    let result = if journal.batches.is_empty() {
        Ok(())
    } else {
        commit(&db, &journal).await
    };
    with_session(|s| {
        result?;
        if let Some(last) = journal.batches.last() {
            let mut remaining = s.journal.clone();
            remaining
                .batches
                .retain(|batch| batch.sequence > last.sequence);
            remaining.persist()?;
            s.journal = remaining;
        }
        Ok(())
    })
}

// Cancellation leaves the durable journal available for an idempotent retry.
struct CommitGuard;
impl Drop for CommitGuard {
    fn drop(&mut self) {
        SESSION.with(|s| {
            if let Some(session) = s.borrow_mut().as_mut() {
                session.committing = false;
            }
        });
    }
}

pub(crate) fn next_directory() -> Result<String> {
    with_session(|s| {
        let number = s.mission_sequence;
        s.mission_sequence = number
            .checked_add(1)
            .context("browser mission identity exhausted")?;
        Ok(format!("mission-{}-{number}", s.journal.writer))
    })
}

async fn commit(db: &Rexie, journal: &Journal) -> Result<()> {
    journal.validate()?;
    let tx = db
        .transaction(&[FILES, BLOCKS, COMMITS], TransactionMode::ReadWrite)
        .map_err(db_error)?;
    let result: Result<()> = async {
        let files = tx.store(FILES).map_err(db_error)?;
        let blocks = tx.store(BLOCKS).map_err(db_error)?;
        let commits = tx.store(COMMITS).map_err(db_error)?;
        let mut committed: u64 = get_json(&commits, &journal.writer).await?.unwrap_or(0);
        for batch in &journal.batches {
            if batch.sequence <= committed {
                continue;
            }
            ensure!(
                committed.checked_add(1) == Some(batch.sequence),
                "missing replay journal predecessor"
            );
            for patch in &batch.patches {
                apply_patch(
                    &files,
                    &blocks,
                    patch,
                    &revision(&journal.writer, batch.sequence),
                )
                .await?;
            }
            committed = batch.sequence;
        }
        put_json(&commits, &journal.writer, &committed).await
    }
    .await;
    if let Err(error) = result {
        tx.abort()
            .await
            .map_err(db_error)
            .context("abort rejected replay transaction")?;
        return Err(error);
    }
    ensure!(
        tx.done().await.map_err(db_error)?.is_committed(),
        "browser replay transaction was aborted"
    );
    Ok(())
}

async fn apply_patch(files: &Store, blocks: &Store, patch: &Patch, revision: &str) -> Result<()> {
    let old: Option<FileIndex> = get_json(files, &patch.path).await?;
    ensure!(
        old == patch.expected,
        "replay file changed in another writer: {}",
        patch.path
    );
    let old_len = old.as_ref().map_or(0, |old| old.len);
    let metadata = matches!(
        Path::new(&patch.path).file_name().and_then(|p| p.to_str()),
        Some(MANIFEST | "ranked.json")
    );
    ensure!(
        old_len <= MAX_BYTES && (patch.offset == old_len || metadata && patch.offset == 0),
        "invalid replay patch boundary"
    );
    let bytes = base64::engine::general_purpose::STANDARD.decode(&patch.encoded)?;
    let new_len = patch
        .offset
        .checked_add(bytes.len())
        .context("replay patch size overflow")?;
    ensure!(
        new_len <= MAX_BYTES,
        "replay file exceeds {MAX_BYTES} bytes"
    );
    let start = patch.offset / BLOCK_BYTES;
    let mut tail = if !patch.offset.is_multiple_of(BLOCK_BYTES) {
        let mut prefix = block(
            blocks,
            &patch.path,
            start,
            (old_len - start * BLOCK_BYTES).min(BLOCK_BYTES),
        )
        .await?;
        prefix.truncate(patch.offset % BLOCK_BYTES);
        prefix
    } else {
        Vec::new()
    };
    tail.extend_from_slice(&bytes);
    for (i, chunk) in tail.chunks(BLOCK_BYTES).enumerate() {
        blocks
            .put(
                &Uint8Array::from(chunk).into(),
                Some(&block_key(&patch.path, start + i)),
            )
            .await
            .map_err(db_error)?;
    }
    for i in new_len.div_ceil(BLOCK_BYTES)..old_len.div_ceil(BLOCK_BYTES) {
        blocks
            .delete(block_key(&patch.path, i))
            .await
            .map_err(db_error)?;
    }
    put_json(
        files,
        &patch.path,
        &FileIndex {
            len: new_len,
            revision: revision.into(),
        },
    )
    .await
}

pub(super) fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    with_session(|s| {
        let file = s.files.get(path).context(
            "replay file is not prepared; load browser history asynchronously before restoring",
        )?;
        ensure!(
            file.bytes.len() <= limit,
            "mission recording exceeds its byte limit"
        );
        Ok(file.bytes.clone())
    })
}

fn change(path: &Path, bytes: &[u8], append: bool, create: bool) -> Result<()> {
    path_key(path)?;
    let result = with_session(|s| {
        let old = s.files.get(path);
        ensure!(!create || old.is_none(), "replay file already exists");
        ensure!(!append || old.is_some(), "missing replay chunk for append");
        let old_len = old.map_or(0, |f| f.bytes.len());
        let len = if append {
            old_len
                .checked_add(bytes.len())
                .context("replay size overflow")?
        } else {
            bytes.len()
        };
        ensure!(len <= MAX_BYTES, "replay chunk capacity exceeded");
        let total: usize = s.files.values().map(|f| f.bytes.len()).sum();
        ensure!(
            total - old_len + len <= CACHE_BYTES,
            "browser replay cache capacity exceeded"
        );
        let file = s.files.entry(path.into()).or_insert(CachedFile {
            bytes: Vec::new(),
            index: None,
            dirty: Some(0),
        });
        if append {
            file.dirty = Some(file.dirty.unwrap_or(old_len));
            file.bytes.extend_from_slice(bytes);
        } else {
            file.dirty = Some(0);
            file.bytes = bytes.to_vec();
        }
        Ok(())
    });
    if let Err(error) = &result {
        SESSION.with(|slot| {
            if let Some(session) = slot.borrow_mut().as_mut() {
                session.failure = Some(format!("{error:#}"));
            }
        });
    }
    result
}
pub(super) fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    change(path, bytes, false, false)?;
    checkpoint()
}
pub(super) fn create_chunk(path: &Path) -> Result<()> {
    change(path, b"", false, true)
}
pub(super) fn open_chunk_writer(path: &Path) -> Result<Box<dyn Write + Send>> {
    with_session(|s| {
        ensure!(s.files.contains_key(path), "missing replay chunk");
        Ok(())
    })?;
    let directory = path.parent().context("replay chunk has no directory")?;
    Ok(Box::new(BrowserChunk(
        path.into(),
        pin_directory(directory)?,
    )))
}
#[derive(Serialize)]
struct BrowserChunk(PathBuf, DirectoryLease);
impl<'de> Deserialize<'de> for BrowserChunk {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> std::result::Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "browser replay writers require live archive ownership",
        ))
    }
}
impl Write for BrowserChunk {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        change(&self.0, bytes, true, false).map_err(std::io::Error::other)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        checkpoint().map_err(std::io::Error::other)
    }
}

async fn load_file(path: &Path, required: bool) -> Result<()> {
    if with_session(|s| Ok(s.files.contains_key(path)))? {
        return Ok(());
    }
    let key = path_key(path)?;
    let db = with_session(|s| Ok(s.db.clone()))?;
    let tx = db
        .transaction(&[FILES, BLOCKS], TransactionMode::ReadOnly)
        .map_err(db_error)?;
    let mut index: Option<FileIndex> = get_json(&tx.store(FILES).map_err(db_error)?, &key).await?;
    let bytes = if let Some(index) = &index {
        ensure!(index.len <= MAX_BYTES, "oversized replay file index");
        let blocks = tx.store(BLOCKS).map_err(db_error)?;
        let mut bytes = Vec::with_capacity(index.len);
        for i in 0..index.len.div_ceil(BLOCK_BYTES) {
            bytes.extend_from_slice(
                &block(&blocks, &key, i, (index.len - bytes.len()).min(BLOCK_BYTES)).await?,
            );
        }
        Some(bytes)
    } else {
        None
    };
    ensure!(
        tx.done().await.map_err(db_error)?.is_committed(),
        "browser replay transaction was aborted"
    );
    // Read compatibility also handles a legacy tab creating a file after this
    // page initialized. Retire the old key only after the atomic import commits.
    let bytes = match bytes {
        Some(bytes) => Some(bytes),
        None => storage()?
            .get_item(&format!("robin:replay:{key}"))
            .map_err(|e| anyhow::anyhow!("read legacy replay: {e:?}"))?
            .map(String::into_bytes),
    };
    let Some(bytes) = bytes else {
        ensure!(!required, "missing replay file {key}");
        return Ok(());
    };
    ensure!(bytes.len() <= MAX_BYTES, "oversized legacy replay file");
    if index.is_none() {
        index = Some(import_legacy(&db, &key, &bytes).await?);
        retire_legacy(&format!("robin:replay:{key}"), std::str::from_utf8(&bytes)?)?;
    }
    with_session(|s| {
        let total: usize = s.files.values().map(|f| f.bytes.len()).sum();
        ensure!(
            total + bytes.len() <= CACHE_BYTES,
            "browser replay cache capacity exceeded"
        );
        let dirty = None;
        s.files.insert(
            path.into(),
            CachedFile {
                bytes,
                index,
                dirty,
            },
        );
        Ok(())
    })
}

// Legacy bytes are already durable. Import them in one transaction without
// copying an entire old recording into the small synchronous recovery journal.
async fn import_legacy(db: &Rexie, path: &str, bytes: &[u8]) -> Result<FileIndex> {
    use sha2::{Digest, Sha256};
    let index = FileIndex {
        len: bytes.len(),
        revision: format!("legacy:{}", hex::encode(Sha256::digest(bytes))),
    };
    let tx = db
        .transaction(&[FILES, BLOCKS], TransactionMode::ReadWrite)
        .map_err(db_error)?;
    let result: Result<()> = async {
        let files = tx.store(FILES).map_err(db_error)?;
        let blocks = tx.store(BLOCKS).map_err(db_error)?;
        if let Some(existing) = get_json::<FileIndex>(&files, path).await? {
            ensure!(
                existing == index,
                "legacy replay was modified by another writer"
            );
            return Ok(());
        }
        for (i, bytes) in bytes.chunks(BLOCK_BYTES).enumerate() {
            blocks
                .put(&Uint8Array::from(bytes).into(), Some(&block_key(path, i)))
                .await
                .map_err(db_error)?;
        }
        put_json(&files, path, &index).await
    }
    .await;
    if let Err(error) = result {
        tx.abort().await.map_err(db_error)?;
        return Err(error);
    }
    ensure!(
        tx.done().await.map_err(db_error)?.is_committed(),
        "browser replay transaction was aborted"
    );
    Ok(index)
}

fn retire_legacy(key: &str, imported: &str) -> Result<()> {
    let storage = storage()?;
    // Never delete a legacy tab's newer write while an import was awaiting I/O.
    if storage
        .get_item(key)
        .map_err(|e| anyhow::anyhow!("reread legacy replay: {e:?}"))?
        .as_deref()
        == Some(imported)
    {
        storage
            .remove_item(key)
            .map_err(|e| anyhow::anyhow!("retire imported replay: {e:?}"))?;
    }
    Ok(())
}

/// Archive and writer owners keep their complete history resident, including
/// clean prefix chunks that synchronous save validation may still need.
#[derive(Serialize)]
pub(super) struct DirectoryLease {
    directory: PathBuf,
    writer: String,
}
impl<'de> Deserialize<'de> for DirectoryLease {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> std::result::Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "replay cache leases require live ownership",
        ))
    }
}
pub(super) fn pin_directory(directory: &Path) -> Result<DirectoryLease> {
    with_session(|s| {
        *s.leases.entry(directory.into()).or_default() += 1;
        Ok(DirectoryLease {
            directory: directory.into(),
            writer: s.journal.writer.clone(),
        })
    })
}
impl Drop for DirectoryLease {
    fn drop(&mut self) {
        SESSION.with(|slot| {
            if let Some(s) = slot.borrow_mut().as_mut()
                && s.journal.writer == self.writer
            {
                let count = s
                    .leases
                    .get_mut(&self.directory)
                    .expect("live replay cache lease");
                *count -= 1;
                if *count == 0 {
                    s.leases.remove(&self.directory);
                }
            }
        });
    }
}

/// Evict only durable, unowned histories. Keep entire directories with pending
/// writes, not just dirty files: journal retirement and synchronous archive
/// validation still need the associated metadata and prefix.
fn release_unreferenced(keep: Option<&Path>) -> Result<()> {
    with_session(|s| {
        let mut retained: std::collections::BTreeSet<PathBuf> = s.leases.keys().cloned().collect();
        retained.extend(keep.map(Path::to_path_buf));
        for (path, file) in &s.files {
            if file.dirty.is_some()
                && let Some(directory) = path.parent()
            {
                retained.insert(directory.into());
            }
        }
        for patch in s.journal.batches.iter().flat_map(|batch| &batch.patches) {
            if let Some(directory) = Path::new(&patch.path).parent() {
                retained.insert(directory.into());
            }
        }
        s.files
            .retain(|path, _| path.parent().is_some_and(|dir| retained.contains(dir)));
        Ok(())
    })
}

/// Prepare only the selected archive, never every historical recording.
pub async fn prepare_directory(directory: &Path) -> Result<()> {
    // A temporary lease also keeps concurrent preparation from evicting this
    // history while IndexedDB reads yield. Drop it before failure cleanup.
    release_unreferenced(Some(directory))?;
    let lease = pin_directory(directory)?;
    let result = async {
        load_file(&directory.join(MANIFEST), true).await?;
        let manifest = super::read_manifest(directory)?;
        for chunk in manifest.chunks {
            load_file(&directory.join(chunk.file), true).await?;
        }
        load_file(&directory.join("ranked.json"), false).await?;
        flush_pending().await
    }
    .await;
    drop(lease);
    if result.is_err() {
        release_unreferenced(None)?;
    }
    result
}

/// The mission loop has retired, but the application save-capture service may
/// still own its recorder until the next mission installs a replacement.
/// Honor those leases instead of invalidating a live writer's backing cache.
pub async fn retire_mission() -> Result<()> {
    flush_pending().await?;
    release_unreferenced(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    async fn restart() {
        SESSION.with(|s| *s.borrow_mut() = None);
        initialize().await.unwrap();
    }

    // One owner runs the complete storage lifecycle to avoid competing with
    // another fixture for the page-local cache. Every path has a random prefix.
    #[wasm_bindgen_test]
    async fn indexed_replay_blocks_recover_atomic_batches_and_preserve_legacy_history() {
        initialize().await.unwrap();
        let directory = next_directory().unwrap();
        let path = PathBuf::from(&directory).join("00000000.rhrec.jsonl");
        create_chunk(&path).unwrap();
        let mut writer = open_chunk_writer(&path).unwrap();
        let first = vec![b'a'; BLOCK_BYTES + 17];
        writer.write_all(&first).unwrap();
        writer.flush().unwrap();
        let (db, first_journal) = with_session(|s| Ok((s.db.clone(), s.journal.clone()))).unwrap();
        // A page close before the first IndexedDB commit must retain all bytes.
        restart().await;
        load_file(&path, true).await.unwrap();
        assert_eq!(read_bounded(&path, MAX_BYTES).unwrap(), first);
        assert!(
            storage()
                .unwrap()
                .get_item(&journal_key(&first_journal))
                .unwrap()
                .is_none()
        );

        let mut writer = open_chunk_writer(&path).unwrap();
        writer.write_all(b"second").unwrap();
        writer.flush().unwrap();
        let snapshot = with_session(|s| Ok(s.journal.clone())).unwrap();
        assert_eq!(snapshot.batches.len(), 1);
        assert_eq!(snapshot.batches[0].patches[0].offset, first.len());
        assert!(
            serde_json::to_vec(&snapshot).unwrap().len() < 1024,
            "journal size must not grow with committed history"
        );
        commit(&db, &snapshot).await.unwrap();
        // Simulate an urgent capture during the awaited transaction: it uses
        // the next sequence even though the old batch has not been retired.
        writer.write_all(b"urgent").unwrap();
        writer.flush().unwrap();
        restart().await;
        load_file(&path, true).await.unwrap();
        let mut expected = first;
        expected.extend_from_slice(b"secondurgent");
        assert_eq!(read_bounded(&path, MAX_BYTES).unwrap(), expected);
        commit(&db, &snapshot).await.unwrap(); // Already committed: no duplicate append.

        // A stale writer aborts the entire transaction, including an earlier
        // valid patch in the same batch and its commit watermark.
        let current = with_session(|s| Ok(s.files[&path].index.clone())).unwrap();
        let orphan = PathBuf::from(&directory).join("orphan.rhrec.jsonl");
        let bad = Journal {
            version: 1,
            writer: "f".repeat(32),
            batches: vec![Batch {
                sequence: 1,
                patches: vec![
                    Patch {
                        path: path_key(&orphan).unwrap(),
                        expected: None,
                        offset: 0,
                        encoded: "YQ==".into(),
                    },
                    Patch {
                        path: path_key(&path).unwrap(),
                        expected: None,
                        offset: 0,
                        encoded: "Yg==".into(),
                    },
                ],
            }],
        };
        assert!(
            commit(&db, &bad)
                .await
                .unwrap_err()
                .to_string()
                .contains("another writer")
        );
        let tx = db
            .transaction(&[FILES, COMMITS], TransactionMode::ReadOnly)
            .unwrap();
        assert!(
            get_json::<FileIndex>(&tx.store(FILES).unwrap(), &path_key(&orphan).unwrap())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            get_json::<u64>(&tx.store(COMMITS).unwrap(), &bad.writer)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            get_json::<FileIndex>(&tx.store(FILES).unwrap(), &path_key(&path).unwrap())
                .await
                .unwrap(),
            current
        );
        assert!(tx.done().await.unwrap().is_committed());

        // Replacements shrink metadata without retaining obsolete tail blocks.
        let metadata = PathBuf::from(&directory).join(MANIFEST);
        write(&metadata, &vec![b'x'; BLOCK_BYTES * 2 + 7]).unwrap();
        flush_pending().await.unwrap();
        write(&metadata, b"{}").unwrap();
        flush_pending().await.unwrap();
        restart().await;
        load_file(&metadata, true).await.unwrap();
        assert_eq!(read_bounded(&metadata, MAX_BYTES).unwrap(), b"{}");

        // Import larger-than-journal legacy files atomically, keeping the old
        // history intact rather than truncating or guessing data.
        let legacy = PathBuf::from(&directory).join("legacy.rhrec.jsonl");
        let legacy_key = format!("robin:replay:{}", legacy.display());
        let legacy_bytes = "l".repeat(JOURNAL_BYTES + 17);
        storage()
            .unwrap()
            .set_item(&legacy_key, &legacy_bytes)
            .unwrap();
        restart().await; // Startup migrates old files without filling the cache.
        assert!(with_session(|s| Ok(s.files.is_empty())).unwrap());
        load_file(&legacy, true).await.unwrap();
        restart().await;
        load_file(&legacy, true).await.unwrap();
        assert_eq!(
            read_bounded(&legacy, MAX_BYTES).unwrap(),
            legacy_bytes.as_bytes()
        );
        assert!(storage().unwrap().get_item(&legacy_key).unwrap().is_none());

        // Capacity failure cannot advance the journaled offset or replace the
        // last durable recovery record. Errors ignored by Write consumers stay
        // visible at the awaited frame barrier.
        let huge = PathBuf::from(&directory).join("capacity.rhrec.jsonl");
        create_chunk(&huge).unwrap();
        checkpoint().unwrap();
        let before = with_session(|s| Ok(s.journal.clone())).unwrap();
        let key = journal_key(&before);
        let saved = storage().unwrap().get_item(&key).unwrap();
        change(&huge, &vec![0; JOURNAL_BYTES], true, false).unwrap();
        assert!(
            checkpoint()
                .unwrap_err()
                .to_string()
                .contains("journal exceeds")
        );
        assert_eq!(storage().unwrap().get_item(&key).unwrap(), saved);
        assert_eq!(
            with_session(|s| Ok(s.files[&huge].index.as_ref().unwrap().len)).unwrap(),
            0
        );
        restart().await;
        load_file(&huge, true).await.unwrap();
        assert!(read_bounded(&huge, MAX_BYTES).unwrap().is_empty());
        assert!(change(Path::new("not-prepared"), b"bad", true, false).is_err());
        assert!(
            flush_pending()
                .await
                .unwrap_err()
                .to_string()
                .contains("recording failed")
        );
        restart().await;
        exercise_archive_save_and_cold_continuation().await;
        exercise_cache_lifetimes().await;
    }

    async fn exercise_cache_lifetimes() {
        use crate::replay_archive::MissionArchive;
        let active_dir = PathBuf::from(next_directory().unwrap());
        let active = MissionArchive::create(&active_dir).unwrap();
        let active_path = active_dir.join(active.current_chunk());
        let mut writer = open_chunk_writer(&active_path).unwrap();
        writer.write_all(b"active").unwrap();
        flush_pending().await.unwrap();
        // The application capture service can retain the recorder across the
        // mission-loop return. Retirement must honor that still-live owner.
        retire_mission().await.unwrap();
        assert_eq!(read_bounded(&active_path, MAX_BYTES).unwrap(), b"active");

        // Repeated same-mission selections must not accumulate unrelated
        // clean histories, while a live archive keeps its complete prefix.
        let mut previous: Option<PathBuf> = None;
        for _ in 0..8 {
            let directory = PathBuf::from(next_directory().unwrap());
            let archive = MissionArchive::create(&directory).unwrap();
            flush_pending().await.unwrap();
            drop(archive);
            prepare_directory(&directory).await.unwrap();
            with_session(|s| {
                assert!(s.files.contains_key(&active_path));
                if let Some(previous) = &previous {
                    assert!(!s.files.keys().any(|p| p.starts_with(previous)));
                }
                assert!(
                    s.files
                        .keys()
                        .all(|p| p.starts_with(&active_dir) || p.starts_with(&directory))
                );
                Ok(())
            })
            .unwrap();
            previous = Some(directory);
        }

        // A writer may outlive its archive handle; its prefix stays resident.
        drop(active);
        release_unreferenced(None).unwrap();
        assert_eq!(read_bounded(&active_path, MAX_BYTES).unwrap(), b"active");
        writer.write_all(b"-pending").unwrap();
        drop(writer);
        release_unreferenced(None).unwrap(); // Dirty bytes retain the history.
        assert!(read_bounded(&active_path, MAX_BYTES).is_ok());
        checkpoint().unwrap();
        release_unreferenced(None).unwrap(); // Journaled but uncommitted too.
        assert!(read_bounded(&active_path, MAX_BYTES).is_ok());
        flush_pending().await.unwrap();
        release_unreferenced(None).unwrap();
        assert!(with_session(|s| Ok(s.files.is_empty())).unwrap());
        load_file(&active_path, true).await.unwrap();
        assert_eq!(
            read_bounded(&active_path, MAX_BYTES).unwrap(),
            b"active-pending"
        );

        // A valid manifest with a missing chunk must not leave its partially
        // loaded history behind, or evict an unrelated live archive.
        let active = MissionArchive::open(&active_dir);
        assert!(active.is_err()); // Only the chunk, not its manifest, is loaded.
        prepare_directory(&active_dir).await.unwrap();
        let active = MissionArchive::open(&active_dir).unwrap();
        let broken_dir = PathBuf::from(next_directory().unwrap());
        write(&broken_dir.join(MANIFEST), br#"{"version":1,"chunks":[{"file":"00000000.rhrec.jsonl","first_ordinal":0,"previous":null,"loaded_save":null}]}"#).unwrap();
        flush_pending().await.unwrap();
        assert!(prepare_directory(&broken_dir).await.is_err());
        with_session(|s| {
            assert!(s.files.contains_key(&active_path));
            assert!(!s.files.keys().any(|p| p.starts_with(&broken_dir)));
            Ok(())
        })
        .unwrap();
        drop(active);
        retire_mission().await.unwrap();
    }

    async fn exercise_archive_save_and_cold_continuation() {
        use crate::replay_archive::MissionArchive;
        use robin_engine::replay::{ReplayRecorder, ReplaySaveMarker};
        let directory = PathBuf::from(next_directory().unwrap());
        let archive = MissionArchive::create(&directory).unwrap();
        let assets = robin_engine::mission_assets::MissionAssetDescriptor::built_in(
            "browser", "browser", "browser",
        )
        .unwrap();
        let mut recorder = ReplayRecorder::with_writer(
            archive.writer().unwrap(),
            "browser".into(),
            assets,
            0,
            Default::default(),
            &Default::default(),
        )
        .unwrap();
        let marker = ReplaySaveMarker {
            state_hash: 123,
            timeline_frame: 0,
        };
        recorder.write_save_marker(0, marker);
        let link = archive.marker_link(0, marker, [7; 32]);
        recorder.write_frame(0, 0, 1, Default::default(), Vec::new(), None);
        recorder.flush().unwrap();
        archive.sync_current().unwrap();
        drop(recorder);
        drop(archive);
        // No asynchronous flush: this mirrors closing the page immediately
        // after a synchronous autosave publishes its replay marker link.
        restart().await;
        prepare_directory(&directory).await.unwrap();
        let mut archive = MissionArchive::open(&directory).unwrap();
        let (_, history, root) = archive.assembled_replay().unwrap();
        archive.validate_link(&link, &history, [7; 32]).unwrap();
        assert_eq!(history.frame_count(), 1);
        archive.append_chunk(1, Some(link)).unwrap();
        let mut recorder =
            ReplayRecorder::continue_recording(archive.writer().unwrap(), root, 1).unwrap();
        recorder.write_load_back(1, 0, false);
        recorder.write_frame(1, 0, 1, Default::default(), Vec::new(), None);
        recorder.flush().unwrap();
        drop(recorder);
        drop(archive);
        retire_mission().await.unwrap();
        prepare_directory(&directory).await.unwrap();
        let history = crate::replay_archive::load_directory(&directory).unwrap();
        assert_eq!(history.frame_count(), 2);
        assert_eq!(history.load_back_for_frame(1).unwrap().to_frame, 0);
        retire_mission().await.unwrap();
    }
}
