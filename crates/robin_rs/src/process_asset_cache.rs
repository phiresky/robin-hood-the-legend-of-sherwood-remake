//! Process-global asset cache warmed on a background thread.
//!
//! The original game paid for the sprite bank, sound banks, and the
//! exclamation cache once at application startup, so its loading screen
//! only covered per-level data. The port rebuilds `Host` per mission,
//! which used to re-read and re-parse all of that on every mission
//! load. This cache holds the parsed, GPU-/audio-free products of that
//! work for the lifetime of the process (the datadir is fixed per
//! process).
//!
//! `start_background_warmup` builds it on a plain thread while the main
//! menu runs, so the menu is visually unaffected; mission load consumes
//! it via `get_or_build`, which waits for a running warm-up or builds
//! synchronously when none was started (wasm, tests).

use std::sync::{Arc, Mutex};

use robin_assets::frame_holder::FrameHolder;
use robin_assets::resource_manager::ResourceManager;
use robin_assets::shipping_datadir as assets_shipping_datadir;
use robin_engine::profiles::ProfileManager;
use robin_engine::sbfile::SbFile;
use robin_engine::sound_cache::FxBankElement;

pub struct ProcessAssetCache {
    shipping_mission: Option<String>,
    shipping_exclamation_ids: Vec<u32>,
    /// Pristine sprite bank (mmap-backed spans). Missions clone it and
    /// then append their runtime overlay sprites; the clone is cheap
    /// because bank sprites carry spans, not pixel data.
    pub sprite_bank: Option<FrameHolder>,
    pub fx_bank: Option<Vec<FxBankElement>>,
    pub menu_bank: Option<Vec<(u32, String)>>,
    /// One entry per active-mission exclamation profile file: resolved
    /// `(action_id, wav paths)` lists ready for
    /// `SoundCache::initialize_exclamations_for_profile`.
    pub exclamations: Vec<Vec<(u32, Vec<String>)>>,
}

enum State {
    Idle,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Warming(std::thread::JoinHandle<Arc<ProcessAssetCache>>),
    Ready(Arc<ProcessAssetCache>),
}

static STATE: Mutex<State> = Mutex::new(State::Idle);

/// Kick off the warm-up thread. No-op if already started or on wasm
/// (no threads there; `get_or_build` falls back to a synchronous
/// build, and shipping datadirs make that cheap anyway).
pub fn start_background_warmup(
    shipping: Option<Arc<assets_shipping_datadir::ShippingDatadir>>,
    profiles: Arc<ProfileManager>,
) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (shipping, profiles);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut state = STATE.lock().unwrap();
        if !matches!(*state, State::Idle) {
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("asset-warmup".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let cache = Arc::new(build(shipping.as_deref(), &profiles));
                tracing::info!(
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "process asset cache warmed in background"
                );
                cache
            });
        match spawned {
            Ok(handle) => *state = State::Warming(handle),
            Err(e) => tracing::warn!("asset warm-up thread failed to spawn: {e}"),
        }
    }
}

/// The process asset cache: waits for a running warm-up, or builds
/// synchronously when none was started. Cheap once warmed.
pub fn get_or_build(
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    profiles: &ProfileManager,
) -> Arc<ProcessAssetCache> {
    let mut state = STATE.lock().unwrap();
    match std::mem::replace(&mut *state, State::Idle) {
        State::Ready(cache) => {
            let active = shipping.and_then(|datadir| datadir.active_mission_name());
            let active_exclamations = shipping
                .map(|datadir| datadir.active_exclamation_ids())
                .unwrap_or_default();
            if cache.shipping_mission != active
                || cache.shipping_exclamation_ids != active_exclamations
            {
                drop(state);
                let cache = Arc::new(build(shipping, profiles));
                *STATE.lock().unwrap() = State::Ready(cache.clone());
                return cache;
            }
            *state = State::Ready(cache.clone());
            cache
        }
        State::Warming(handle) => {
            // Join without holding the lock (a concurrent caller then
            // builds redundantly instead of deadlocking; the game runs
            // single-threaded here so that never happens in practice).
            drop(state);
            let mut cache = handle.join().unwrap_or_else(|_| {
                tracing::warn!("asset warm-up thread panicked; rebuilding synchronously");
                Arc::new(build(shipping, profiles))
            });
            let active = shipping.and_then(|datadir| datadir.active_mission_name());
            let active_exclamations = shipping
                .map(|datadir| datadir.active_exclamation_ids())
                .unwrap_or_default();
            if cache.shipping_mission != active
                || cache.shipping_exclamation_ids != active_exclamations
            {
                cache = Arc::new(build(shipping, profiles));
            }
            *STATE.lock().unwrap() = State::Ready(cache.clone());
            cache
        }
        State::Idle => {
            drop(state);
            let cache = Arc::new(build(shipping, profiles));
            *STATE.lock().unwrap() = State::Ready(cache.clone());
            cache
        }
    }
}

fn build(
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    profiles: &ProfileManager,
) -> ProcessAssetCache {
    let sprite_bank = {
        let mut holder = FrameHolder::new();
        match holder.initialize_sprite_bank_with_progress(".", &mut |_| {}, shipping) {
            Ok(()) => Some(holder),
            Err(e) => {
                tracing::warn!("Failed to load sprite bank: {e}");
                None
            }
        }
    };

    let fx_bank_path = "Data/Sounds/robin hood.fxg";
    let fx_bank = match SbFile::read_all(fx_bank_path) {
        Ok(data) => match robin_engine::sound_cache::parse_fx_bank(&data) {
            Ok(elements) => Some(elements),
            Err(e) => {
                tracing::warn!("Failed to parse FX bank: {e}");
                None
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read FX bank '{fx_bank_path}': error {e}");
            None
        }
    };

    let menu_bank_path = "Data/Sounds/Menu/menu.fxg";
    let menu_bank = match SbFile::read_all(menu_bank_path) {
        Ok(data) => match robin_engine::sound_cache::parse_menu_bank(&data) {
            Ok(entries) => Some(entries),
            Err(e) => {
                tracing::warn!("Failed to parse menu sound bank: {e}");
                None
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read menu sound bank '{menu_bank_path}': error {e}");
            None
        }
    };

    let exclamations = build_exclamations(shipping, profiles);

    ProcessAssetCache {
        shipping_mission: shipping.and_then(|datadir| datadir.active_mission_name()),
        shipping_exclamation_ids: shipping
            .map(|datadir| datadir.active_exclamation_ids())
            .unwrap_or_default(),
        sprite_bank,
        fx_bank,
        menu_bank,
        exclamations,
    }
}

/// Load actors.res for variant-index → WAV-filename resolution, then
/// parse each active profile's .dat file into resolved speech entry lists.
/// Split shipping datadirs publish an exact mission/team closure; loose
/// datadirs preserve the original eager all-profile behavior.
fn build_exclamations(
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    profiles: &ProfileManager,
) -> Vec<Vec<(u32, Vec<String>)>> {
    let mut excl_res = ResourceManager::new();
    if let Err(error) =
        excl_res.attach_or_from_shipping("Data/Sounds/Exclamations/actors.res", shipping)
    {
        if shipping.is_some() {
            panic!("shipping mission is missing authoritative actors.res: {error}");
        }
        tracing::warn!("Failed to load actors.res — exclamation cache not initialized: {error}");
        return Vec::new();
    }

    // Collect unique exclamation IDs from all profile types. The id's
    // non-zero LE bytes spell the actor file's name suffix.
    let mut files_needed = std::collections::BTreeMap::<u32, String>::new();
    let mut add = |excl_id: u32| {
        if excl_id != 0 {
            let name: String = excl_id
                .to_le_bytes()
                .iter()
                .filter(|&&b| b != 0)
                .map(|&b| b as char)
                .collect();
            files_needed.insert(excl_id, format!("actor{name}.dat"));
        }
    };
    if let Some(datadir) = shipping {
        for excl_id in datadir.active_exclamation_ids() {
            add(excl_id);
        }
    } else {
        for ch in &profiles.characters {
            add(ch.exclamation_id);
        }
        for s in &profiles.soldiers {
            add(s.exclamation_id);
        }
        for c in &profiles.civilians {
            add(c.exclamation_id);
        }
    }

    let mut result = Vec::new();
    let mut total_exclamations = 0usize;
    for (&excl_id, dat_filename) in &files_needed {
        let dat_path = format!("Data/Sounds/Exclamations/{dat_filename}");
        let data = match SbFile::read_all(&dat_path) {
            Ok(d) => d,
            Err(e) => {
                if shipping.is_some() {
                    panic!(
                        "shipping mission selected exclamation profile {excl_id:#010x}, but {dat_path} is unavailable: {e}"
                    );
                }
                tracing::warn!("Failed to read exclamation file '{dat_path}': error {e}");
                continue;
            }
        };

        let prefix_id = excl_id & 0xFFFF_0000;
        let (table_id, exclamations) = match robin_engine::sound_cache::parse_exclamation_file(
            &data, prefix_id,
        ) {
            Ok(r) => r,
            Err(e) => {
                if shipping.is_some() {
                    panic!(
                        "shipping exclamation profile {excl_id:#010x} has invalid metadata in {dat_filename}: {e}"
                    );
                }
                tracing::warn!("Failed to parse exclamation file '{dat_filename}': {e}");
                continue;
            }
        };

        // Resolve variant indices to WAV file paths via resource manager
        let resolved: Vec<(u32, Vec<String>)> = exclamations
            .into_iter()
            .map(|(action_id, variant_indices)| {
                let paths: Vec<String> = variant_indices
                    .into_iter()
                    .filter_map(|vi| match excl_res.get_sample(table_id as i32, vi as usize) {
                        Ok(sample) => Some(sample.to_string()),
                        Err(error) if shipping.is_some() => panic!(
                            "shipping exclamation profile {excl_id:#010x} cannot resolve table {table_id} variant {vi}: {error}"
                        ),
                        Err(error) => {
                            tracing::warn!(
                                "Failed to resolve exclamation profile {excl_id:#010x}, table {table_id}, variant {vi}: {error}"
                            );
                            None
                        }
                    })
                    .collect();
                (action_id, paths)
            })
            .collect();

        total_exclamations += resolved.len();
        result.push(resolved);
    }

    tracing::info!(
        "Loaded exclamation cache: {} profiles, {} exclamations",
        files_needed.len(),
        total_exclamations,
    );
    result
}
