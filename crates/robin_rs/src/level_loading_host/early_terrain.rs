//! Single-use preparation handoff; no worker can publish into another mission.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct TerrainKey {
    installation: u64,
    mission: String,
    map: String,
    ambiance: String,
    mission_generation: u64,
}

#[cfg(any(
    all(test, not(target_arch = "wasm32")),
    all(target_arch = "wasm32", feature = "wasm-threads")
))]
#[derive(serde::Serialize, serde::Deserialize)]
struct FinishOnDrop(#[serde(skip)] Arc<AtomicBool>);

#[cfg(any(
    all(test, not(target_arch = "wasm32")),
    all(target_arch = "wasm32", feature = "wasm-threads")
))]
impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Transient worker result, deliberately excluded from persisted asset state.
/// Dropping an unconsumed handoff cancels queued work and discards late output.
pub struct EarlyTerrainDecode {
    key: TerrainKey,
    dimensions: (u16, u16),
    source: Arc<Vec<u8>>,
    receiver: Option<futures::channel::oneshot::Receiver<Result<Picture, String>>>,
    cancelled: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}

impl Drop for EarlyTerrainDecode {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl EarlyTerrainDecode {
    /// Only the exact authored initial ambiance is eligible: a later part may
    /// still supply a higher-priority image than a Day/bare fallback.
    pub fn try_start(
        mission: &str,
        datadir: &assets_shipping_datadir::ShippingDatadir,
        merged: &assets_shipping_datadir::ShippingMission,
    ) -> Option<Self> {
        #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
        {
            if robin_assets::wasm_threads::pool_threads() == 0 {
                return None;
            }
            let disabled = web_sys::window()
                .and_then(|window| window.location().search().ok())
                .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
                .and_then(|params| params.get("early-terrain"))
                .is_some_and(|value| value == "0");
            if disabled {
                return None;
            }
            let level = merged.levels.get(mission)?;
            let authored = Ambiance::from_raw(level.mission.header.ambiance);
            let ambiance = level
                .mission
                .ambience_schedule
                .iter()
                .take_while(|cue| cue.at_seconds == 0)
                .last()
                .map_or(authored, |cue| cue.ambiance)
                .directory()
                .to_owned();
            let map = level.mission.header.map_filename.clone();
            let path = format!("levels/{ambiance}/{map}.map").to_ascii_lowercase();
            // Boot raw has precedence in ShippingDatadir::raw_asset too.
            let bytes = datadir
                .raw_asset(&path)
                .or_else(|| merged.raw.get(&path).map(Vec::as_slice))?;
            let dimensions = Picture::terrain_dimensions(bytes).ok()?;
            let bytes = Arc::new(bytes.to_vec());
            let source = bytes.clone();
            let (sender, receiver) = futures::channel::oneshot::channel();
            let cancelled = Arc::new(AtomicBool::new(false));
            let finished = Arc::new(AtomicBool::new(false));
            let worker_cancelled = cancelled.clone();
            let worker_finished = finished.clone();
            // One serial decoder occupies exactly one scheduler slot.
            // Dispatch is eager; EarlyTerrainDecode owns the separate result
            // channel above, so this unit-completion receiver is unnecessary.
            let _ = robin_assets::wasm_threads::start_on_pool(move || {
                let _finished = FinishOnDrop(worker_finished);
                if !worker_cancelled.load(Ordering::Acquire) {
                    let started = web_time::Instant::now();
                    let result = Picture::load_terrain_from_bytes(&bytes)
                        .map_err(|error| format!("failed to decode shipped map '{path}': {error}"));
                    tracing::info!(
                        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
                        "startup timing: early terrain pixels"
                    );
                    if !worker_cancelled.load(Ordering::Acquire) {
                        let _ = sender.send(result);
                    }
                }
            });
            tracing::info!(mission, "early terrain decode started");
            return Some(Self {
                key: TerrainKey {
                    installation: datadir.installation_id(),
                    mission: mission.into(),
                    map,
                    ambiance,
                    mission_generation: 0,
                },
                dimensions,
                source,
                receiver: Some(receiver),
                cancelled,
                finished,
            });
        }
        #[cfg(not(all(target_arch = "wasm32", feature = "wasm-threads")))]
        {
            let _ = (mission, datadir, merged);
            None
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub(super) fn dimensions(&self) -> (u16, u16) {
        self.dimensions
    }

    pub(crate) fn publish(
        &mut self,
        datadir: &assets_shipping_datadir::ShippingDatadir,
    ) -> Result<(), String> {
        let selection = datadir.selection_snapshot();
        if self.key.installation != datadir.installation_id()
            || selection.mission.as_deref() != Some(&self.key.mission)
        {
            return Err(
                "early terrain handoff does not belong to selected shipping mission".into(),
            );
        }
        // Exclamation selection/localization invalidates the broad asset
        // generation immediately after shipping installation. Terrain belongs
        // to the installed mission; final reader bytes are checked separately.
        self.key.mission_generation = selection.mission_generation;
        Ok(())
    }

    pub(crate) fn matches(
        &self,
        datadir: &assets_shipping_datadir::ShippingDatadir,
        mission: &str,
        map: &str,
        ambiance: &str,
    ) -> bool {
        let selection = datadir.selection_snapshot();
        self.key.installation == datadir.installation_id()
            && self.key.mission_generation == selection.mission_generation
            && selection.mission.as_deref() == Some(mission)
            && self.key.mission == mission
            && self.key.map == map
            && self.key.ambiance == ambiance
    }

    pub(crate) fn matches_source(
        &self,
        datadir: &assets_shipping_datadir::ShippingDatadir,
        files: &sbfile::SbFileSystem,
        level_directory: &str,
    ) -> Result<bool, String> {
        let key = format!("levels/{}/{}.map", self.key.ambiance, self.key.map).to_ascii_lowercase();
        for candidate in [
            key,
            format!("levels/day/{}.map", self.key.map).to_ascii_lowercase(),
            format!("levels/{}.map", self.key.map).to_ascii_lowercase(),
        ] {
            if let Some(bytes) = datadir.raw_asset(&candidate) {
                return Ok(bytes == self.source.as_slice());
            }
        }
        let path = format!(
            "{}/{}/{}.map",
            level_directory, self.key.ambiance, self.key.map
        );
        let png = format!("{path}.png");
        if files.try_exists(&png).map_err(|error| error.to_string())? {
            return Ok(false);
        }
        let bytes = files.read_all(&path).map_err(|error| error.to_string())?;
        Ok(bytes.as_slice() == self.source.as_slice())
    }

    pub(super) async fn finish(
        mut self,
        level_directory: String,
        shipping: Arc<assets_shipping_datadir::ShippingDatadir>,
        files: Arc<sbfile::SbFileSystem>,
    ) -> DecodedTerrainBitmaps {
        let picture = self
            .receiver
            .take()
            .expect("early terrain result consumed twice")
            .await
            .expect("early terrain worker dropped its result");
        let map = self.key.map.clone();
        let ambiance = self.key.ambiance.clone();
        let finish = move || {
            let background = picture
                .and_then(|picture| {
                    super::finish_background_picture(
                        picture,
                        &map,
                        &ambiance,
                        &level_directory,
                        &files,
                    )
                })
                .map(Some);
            let minimap = super::pre_decode_minimap_with_files(
                &map,
                &ambiance,
                &level_directory,
                Some(&shipping),
                &mut |_| {},
                &files,
            );
            DecodedTerrainBitmaps {
                background,
                minimap,
            }
        };
        #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
        {
            robin_assets::wasm_threads::run_on_pool(finish)
                .await
                .expect("terrain finalization worker failed")
        }
        #[cfg(not(all(target_arch = "wasm32", feature = "wasm-threads")))]
        {
            finish()
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn datadir() -> Arc<assets_shipping_datadir::ShippingDatadir> {
        let installed = assets_shipping_datadir::ShippingAssets::install(
            Arc::new(assets_shipping_datadir::ShippingDatadir::default()),
            Arc::new(robin_util::asset_fs::AssetVfs::new()),
        )
        .unwrap();
        installed
            .vfs()
            .select_mission(Some("first".into()), Arc::default())
            .unwrap();
        installed.datadir().clone()
    }

    fn job(datadir: &assets_shipping_datadir::ShippingDatadir) -> EarlyTerrainDecode {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let picture = Picture {
            width: 2,
            height: 1,
            pitch: 4,
            pixel_format: robin_assets::picture::PixelFormat::Rgb16,
            data: vec![0x12, 0x34, 0x56, 0x78],
            palette: None,
        };
        assert!(sender.send(Ok(picture)).is_ok());
        EarlyTerrainDecode {
            key: TerrainKey {
                installation: datadir.installation_id(),
                mission: "first".into(),
                map: "map".into(),
                ambiance: "Day".into(),
                mission_generation: 0,
            },
            dimensions: (2, 1),
            source: Arc::new(vec![1, 2, 3]),
            receiver: Some(receiver),
            cancelled: Arc::new(AtomicBool::new(false)),
            finished: Arc::new(AtomicBool::new(true)),
        }
    }

    #[test]
    fn completed_worker_releases_scheduler_reservation() {
        let finished = Arc::new(AtomicBool::new(false));
        let completion = FinishOnDrop(finished.clone());
        drop(completion);
        assert!(finished.load(Ordering::Acquire));
    }

    #[test]
    #[cfg(panic = "unwind")]
    #[ignore = "requires LLVM codegen to exercise unwind cleanup; see docs/TESTING.md"]
    fn unwound_worker_releases_scheduler_reservation() {
        let finished = Arc::new(AtomicBool::new(false));
        let completion = FinishOnDrop(finished.clone());
        let result = std::panic::catch_unwind(move || {
            let _completion = completion;
            panic!("injected worker failure");
        });
        assert!(result.is_err());
        assert!(finished.load(Ordering::Acquire));
    }

    #[test]
    fn handoff_is_single_use_and_rejects_changed_catalog_or_selection() {
        let first = datadir();
        let second = datadir();
        let cache = crate::process_asset_cache::ApplicationAssetCache::default();
        cache.publish_early_terrain(job(&first), &first).unwrap();
        assert!(
            cache
                .take_early_terrain(&second, "first", "map", "Day")
                .is_none()
        );
        cache.publish_early_terrain(job(&first), &first).unwrap();
        let taken = cache
            .take_early_terrain(&first, "first", "map", "Day")
            .unwrap();
        assert!(
            cache
                .take_early_terrain(&first, "first", "map", "Day")
                .is_none()
        );
        drop(taken);
        let stale = job(&first);
        let cancelled = stale.cancelled.clone();
        cache.publish_early_terrain(stale, &first).unwrap();
        first
            .asset_vfs()
            .select_mission(Some("first".into()), Arc::default())
            .unwrap();
        assert!(
            cache
                .take_early_terrain(&first, "first", "map", "Day")
                .is_none()
        );
        assert!(cancelled.load(Ordering::Acquire));
    }

    #[test]
    fn shipping_exclamation_publication_preserves_terrain_handoff() {
        let datadir = datadir();
        let cache = crate::process_asset_cache::ApplicationAssetCache::default();
        cache
            .publish_early_terrain(job(&datadir), &datadir)
            .unwrap();
        let installed = datadir.selection_snapshot();
        // ensure_loaded performs this immediately after publishing terrain.
        datadir.set_active_exclamation_ids([1, 2].into_iter().collect());
        let selected = datadir.selection_snapshot();
        assert_ne!(selected.generation, installed.generation);
        assert_eq!(selected.mission_generation, installed.mission_generation);
        let pending = cache
            .take_early_terrain(&datadir, "first", "map", "Day")
            .unwrap();
        let files = Arc::new(sbfile::SbFileSystem::new(datadir.asset_vfs().clone()));
        let decoded = futures::executor::block_on(pending.finish("Levels".into(), datadir, files));
        assert_eq!(
            decoded.background.unwrap().unwrap().pixels,
            vec![0x3412, 0x7856]
        );
    }

    #[test]
    fn cancellation_and_failed_publication_do_not_affect_other_job() {
        let first = datadir();
        let second = datadir();
        let pending = job(&first);
        let cancelled = pending.cancelled.clone();
        let other = job(&second);
        let cache = crate::process_asset_cache::ApplicationAssetCache::default();
        assert!(cache.publish_early_terrain(pending, &second).is_err());
        assert!(cancelled.load(Ordering::Acquire));
        assert!(!other.cancelled.load(Ordering::Acquire));
    }

    #[test]
    fn installed_reader_override_discards_speculative_pixels() {
        let datadir = datadir();
        let vfs = datadir.asset_vfs().clone();
        vfs.install_preloaded_asset("Levels/Day/map.map", vec![1, 2, 3])
            .unwrap();
        let files = sbfile::SbFileSystem::new(vfs.clone());
        let pending = job(&datadir);
        assert!(pending.matches_source(&datadir, &files, "Levels").unwrap());
        vfs.install_preloaded_asset("Levels/Day/map.map.png", vec![4])
            .unwrap();
        assert!(!pending.matches_source(&datadir, &files, "Levels").unwrap());
    }

    #[test]
    fn consumed_pixels_are_reused_without_reading_or_decoding_map_again() {
        let datadir = datadir();
        let files = Arc::new(sbfile::SbFileSystem::new(datadir.asset_vfs().clone()));
        let decoded =
            futures::executor::block_on(job(&datadir).finish("Levels".into(), datadir, files));
        let background = decoded.background.unwrap().unwrap();
        assert_eq!((background.width, background.height), (2, 1));
        assert_eq!(background.pixels, vec![0x3412, 0x7856]);
        assert!(background.occlusion_depth.is_none());
    }
}
