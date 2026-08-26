//! Persistent cache of audio sample durations.
//!
//! Mission load builds deterministic duration tables for every
//! exclamation and sound source. Deriving a duration means reading the
//! sample file and parsing its header, and a loose datadir turns that
//! into thousands of small file reads per mission load — expensive on a
//! cold page cache even though the results never change. This cache
//! keeps every duration ever derived in one JSON file under the user
//! data dir, keyed by resolved native path and revalidated by file
//! size + mtime, so subsequent loads skip the sample reads entirely.
//!
//! Durations on a cache miss still come from the regular sample loader,
//! so cached and uncached loads produce byte-identical tables — the
//! tables feed the deterministic sim, so this must stay true.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use robin_engine::sound_cache::SampleLoader;

/// Identity + result for one cached sample file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedDuration {
    pub size: u64,
    pub mtime_ms: u64,
    pub duration_ms: u32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AudioDurationCache {
    /// Resolved native sample path → duration, revalidated by size+mtime.
    entries: BTreeMap<String, CachedDuration>,
    #[serde(skip)]
    dirty: bool,
}

impl AudioDurationCache {
    /// Load the cache from disk; any missing or unreadable file just
    /// starts an empty cache that gets rebuilt as durations are derived.
    pub fn load() -> Self {
        let Some(path) = cache_file_path() else {
            return Self::default();
        };
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(cache) => cache,
                Err(e) => {
                    tracing::warn!(
                        "audio duration cache {}: unreadable ({e}); rebuilding",
                        path.display()
                    );
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                tracing::warn!(
                    "audio duration cache {}: read failed ({e}); rebuilding",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Duration of `file_name` in milliseconds, via the cache when
    /// possible.
    ///
    /// The sample is identified the way the sample loader resolves it
    /// on disk; a hit requires matching size + mtime. On miss the
    /// duration comes from `loader` (a full sample read — exactly the
    /// value the uncached path produced) and is remembered. Samples
    /// that don't resolve to a native file (e.g. bundle-backed
    /// datadirs) bypass the cache — those reads are already in-memory.
    pub fn duration_ms(
        &mut self,
        base_dir: &Path,
        file_name: &str,
        loader: &SampleLoader,
    ) -> Option<u32> {
        let Some((native, size, mtime_ms)) = resolve_native_sample(base_dir, file_name) else {
            return loader(file_name).map(|(_, _, duration_ms)| duration_ms);
        };
        let key = native.to_string_lossy().into_owned();
        if let Some(hit) = self.entries.get(&key)
            && hit.size == size
            && hit.mtime_ms == mtime_ms
        {
            return Some(hit.duration_ms);
        }
        let (_, _, duration_ms) = loader(file_name)?;
        self.entries.insert(
            key,
            CachedDuration {
                size,
                mtime_ms,
                duration_ms,
            },
        );
        self.dirty = true;
        Some(duration_ms)
    }

    /// Write the cache back if any entry was added or refreshed.
    pub fn save_if_dirty(&self) {
        if !self.dirty {
            return;
        }
        let Some(path) = cache_file_path() else {
            return;
        };
        match save_atomically(&path, self) {
            Ok(()) => tracing::info!(
                entries = self.entries.len(),
                "audio duration cache saved to {}",
                path.display()
            ),
            Err(e) => tracing::warn!("audio duration cache {}: save failed: {e}", path.display()),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn save_atomically(path: &Path, cache: &AudioDurationCache) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("cache file path has no parent directory"))?;
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(cache).map_err(std::io::Error::other)?;
    // Unique temp name so concurrent game instances can't clobber each
    // other's half-written file; the rename makes the swap atomic.
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, path)
}

/// `<user data dir>/robin_hood/cache/audio_durations.json`, mirroring the
/// save-directory layout. `ROBINHOOD_SAVE_DIR` relocates it next to the
/// saves for portable installs; `None` disables persistence (wasm).
fn cache_file_path() -> Option<PathBuf> {
    if let Ok(save_dir) = std::env::var("ROBINHOOD_SAVE_DIR") {
        return Some(
            PathBuf::from(save_dir)
                .join("cache")
                .join("audio_durations.json"),
        );
    }
    #[cfg(feature = "native-fs")]
    if let Some(data_dir) = dirs::data_dir() {
        return Some(
            data_dir
                .join("robin_hood")
                .join("cache")
                .join("audio_durations.json"),
        );
    }
    None
}

/// Mirror the sample loader's candidate order on the native filesystem
/// and return the winning file's identity for cache keying: the name
/// under `base_dir`, then under `base_dir/Exclamations`, each with
/// case-insensitive fallback.
fn resolve_native_sample(base_dir: &Path, file_name: &str) -> Option<(PathBuf, u64, u64)> {
    let normalised = file_name.replace('\\', "/");
    let direct = Path::new(&normalised);
    let candidates = if direct.is_absolute() {
        vec![direct.to_path_buf()]
    } else {
        vec![
            base_dir.join(&normalised),
            base_dir.join("Exclamations").join(&normalised),
        ]
    };
    let resolved = candidates.into_iter().find_map(|candidate| {
        if candidate.is_file() {
            return Some(candidate);
        }
        robin_engine::sbfile::resolve_case_insensitive(&candidate)
            .or_else(|| {
                candidate
                    .to_str()
                    .and_then(robin_engine::sbfile::resolve_data_path)
            })
            .filter(|p| p.is_file())
    })?;
    let meta = std::fs::metadata(&resolved).ok()?;
    let mtime_ms = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some((resolved, meta.len(), mtime_ms))
}
