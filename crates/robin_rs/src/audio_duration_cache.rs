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
        resolver: &SampleResolver,
        file_name: &str,
        loader: &SampleLoader,
    ) -> Option<u32> {
        let Some((native, size, mtime_ms)) = resolver.resolve(file_name) else {
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

    /// Batch variant of [`Self::duration_ms`]: derive the duration of every
    /// distinct sample name once, fanning cache misses out across a thread
    /// pool on native (wasm derives sequentially — no threads there).
    ///
    /// Returns `name → duration_ms` for every name whose sample resolved;
    /// missing samples are simply absent, matching `duration_ms`'s `None`.
    /// The probe must derive durations exactly like the sample loader so
    /// cached and uncached loads stay byte-identical.
    pub fn durations_for(
        &mut self,
        resolver: &SampleResolver,
        names: impl IntoIterator<Item = String>,
        probe: &(dyn Fn(&str) -> Option<u32> + Sync),
    ) -> BTreeMap<String, u32> {
        let names: std::collections::BTreeSet<String> = names.into_iter().collect();
        let mut out = BTreeMap::new();
        // (name, native identity if the sample resolved to a file)
        let mut misses: Vec<(String, Option<(String, u64, u64)>)> = Vec::new();
        for name in names {
            match resolver.resolve(&name) {
                Some((path, size, mtime_ms)) => {
                    let key = path.to_string_lossy().into_owned();
                    match self.entries.get(&key) {
                        Some(hit) if hit.size == size && hit.mtime_ms == mtime_ms => {
                            out.insert(name, hit.duration_ms);
                        }
                        _ => misses.push((name, Some((key, size, mtime_ms)))),
                    }
                }
                // No native file (e.g. bundle-backed datadirs) — derive
                // through the probe; those reads are already in-memory.
                None => misses.push((name, None)),
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        let derived: Vec<_> = {
            use rayon::prelude::*;
            misses
                .into_par_iter()
                .map(|(name, identity)| {
                    let duration = probe(&name);
                    (name, identity, duration)
                })
                .collect()
        };
        #[cfg(target_arch = "wasm32")]
        let derived: Vec<_> = misses
            .into_iter()
            .map(|(name, identity)| {
                let duration = probe(&name);
                (name, identity, duration)
            })
            .collect();

        for (name, identity, duration) in derived {
            let Some(duration_ms) = duration else {
                continue;
            };
            if let Some((key, size, mtime_ms)) = identity {
                self.entries.insert(
                    key,
                    CachedDuration {
                        size,
                        mtime_ms,
                        duration_ms,
                    },
                );
                self.dirty = true;
            }
            out.insert(name, duration_ms);
        }
        out
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

/// Memoizes the expensive per-directory half of sample path resolution:
/// the case-insensitive / overlay-aware lookup of the sample base
/// directories runs once, after which each sample costs one `stat` of
/// `<resolved dir>/<name>` instead of a full fallback walk. The rare
/// per-file case mismatch still falls back to the full resolution.
pub struct SampleResolver {
    /// Resolved native candidate roots, in loader candidate order
    /// (`base_dir`, then `base_dir/Exclamations`).
    roots: Vec<PathBuf>,
}

impl SampleResolver {
    pub fn new(base_dir: &Path) -> Self {
        // Datadir layering (overlays, primary, language-folder
        // alternates, case folding) is sbfile's domain — resolve each
        // loader candidate directory across the layers once, in loader
        // candidate order (the name under the base dir first, then
        // under base/Exclamations). After that each sample costs one
        // stat.
        let roots = if base_dir.is_absolute() {
            vec![base_dir.to_path_buf()]
        } else {
            let rel = base_dir.to_string_lossy();
            let mut roots = robin_engine::sbfile::SbFile::resolve_data_dir_layers(&rel);
            roots.extend(robin_engine::sbfile::SbFile::resolve_data_dir_layers(
                &format!("{rel}/Exclamations"),
            ));
            roots
        };
        Self { roots }
    }

    /// Mirror the sample loader's candidate order on the native
    /// filesystem and return the winning file's identity for cache
    /// keying.
    fn resolve(&self, file_name: &str) -> Option<(PathBuf, u64, u64)> {
        let normalised = file_name.replace('\\', "/");
        let direct = Path::new(&normalised);
        let resolved = if direct.is_absolute() {
            direct.is_file().then(|| direct.to_path_buf())
        } else {
            self.roots.iter().find_map(|root| {
                let candidate = root.join(&normalised);
                if candidate.is_file() {
                    return Some(candidate);
                }
                // Case mismatch below the resolved root — rare, so the
                // full per-file walk is acceptable here.
                robin_engine::sbfile::resolve_case_insensitive(&candidate).filter(|p| p.is_file())
            })
        }?;
        let meta = std::fs::metadata(&resolved).ok()?;
        let mtime_ms = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis() as u64;
        Some((resolved, meta.len(), mtime_ms))
    }
}
