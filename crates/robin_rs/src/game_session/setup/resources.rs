//! Archive acquisition boundary.
use super::error::ResourcePreparationError;
use super::{
    extract_ground_mark_sprite_data, extract_minimap_widget_setup, extract_titbit_row_frame_counts,
    init_audio_backend, localization,
};
use crate::{audio_backend::KiraAudioBackend, game::Game, host::Host};
use robin_assets::res_descr as assets_res_descr;
use robin_assets::{resource_manager::ResourceManager, shipping_datadir::ShippingDatadir};
use robin_engine::sbfile::SbFileSystem;
use robin_engine::{engine as engine_api, sbfile as engine_sbfile};

/// Preserve archive identity and the decoder's full table/entry error chain.
/// The caller, not this shared diagnostic boundary, decides optionality.
pub(super) fn attach_mission_archive(
    resources: &mut ResourceManager,
    path: &str,
    shipping: Option<&ShippingDatadir>,
    files: &SbFileSystem,
) -> Result<Option<()>, ResourcePreparationError> {
    // Let the resource manager select active shipping locales and legacy
    // fallbacks. A failed registered archive is never optional absence.
    match resources.attach_or_from_shipping(path, shipping) {
        Ok(()) => Ok(Some(())),
        Err(error) => {
            let present = files
                .try_exists(path)
                .map_err(|status| ResourcePreparationError::unavailable(path, status))?;
            if present {
                // Presence alone does not prove the bytes could be acquired
                // (permissions, truncated backing files, or VFS failures).
                files.read_all(path).map_err(|status| {
                    ResourcePreparationError::unavailable(
                        path,
                        format!("file error {status}; {error:#}"),
                    )
                })?;
                Err(ResourcePreparationError::malformed(
                    path,
                    format!("{error:#}"),
                ))
            } else {
                if shipping.is_some_and(|datadir| datadir.active_locale_name().is_some())
                    && robin_assets::shipping_datadir::is_required_locale_key(path)
                {
                    return Err(ResourcePreparationError::unavailable(
                        path,
                        format!("required selected-locale archive: {error:#}"),
                    ));
                }
                tracing::debug!(path, "Optional mission archive absent");
                Ok(None)
            }
        }
    }
}

/// Process-only resources acquired before the deterministic engine is built.
///
/// The text and interface archives provide both construction metadata and the
/// later interactive frontend caches. The optional backend owns the native
/// audio device. None of these values belongs in an engine snapshot.
pub(in crate::game_session) struct MissionProcessResources<Interface = ResourceManager> {
    pub(in crate::game_session) text: ResourceManager,
    /// Only the ready stage permits engine metadata extraction. Dispatch and
    /// collection consume their stages, so neither operation can run twice.
    interface: Interface,
    pub(in crate::game_session) audio_backend: Option<KiraAudioBackend>,
}

/// The interface archive, possibly off on a worker getting its JXL pictures
/// eagerly decoded while the level loads (see
/// [`MissionProcessResources::start_interface_decode`]). The joined pair is
/// `(cursor, menu)` — two identical fully-decoded `DEFAULT.RES` views, one
/// for the mission sprite caches and one owned by the in-game menus.
pub(in crate::game_session) enum DecodingInterfaceResources {
    #[cfg(target_arch = "wasm32")]
    Ready { cursor: ResourceManager },
    #[cfg(not(target_arch = "wasm32"))]
    Thread(std::thread::JoinHandle<(ResourceManager, ResourceManager)>),
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    Pool(robin_assets::wasm_threads::PoolReceiver<(ResourceManager, ResourceManager)>),
}

/// Worker-side body of the interface pre-decode: decode every encoded (JXL)
/// picture once, then duplicate the decoded manager for the menu owner —
/// a memcpy of decoded pixels, far cheaper than a second decode pass.
#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
fn decode_interface_managers(mut cursor: ResourceManager) -> (ResourceManager, ResourceManager) {
    let started = web_time::Instant::now();
    let decoded = cursor.decode_all_encoded_pictures();
    let menu = cursor.duplicate();
    tracing::info!(
        resources = decoded,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "interface pictures pre-decoded off the loading path"
    );
    (cursor, menu)
}

/// Shared engine-construction resources for graphical and headless missions.
/// This owner contains
/// no renderer, input device, HUD, menu, font, or native audio backend.
pub(in crate::game_session) struct MissionEngineResources {
    pub(in crate::game_session) text: ResourceManager,
    cursor: ResourceManager,
}

impl MissionEngineResources {
    pub(in crate::game_session) fn load(host: &Host) -> Result<Self, ResourcePreparationError> {
        Self::load_archives(
            host.preparation_files()
                .map_err(ResourcePreparationError::MissingAuthority)?
                .clone(),
            host.frontend.shipping.as_deref(),
        )
    }

    fn load_archives(
        files: std::sync::Arc<engine_sbfile::SbFileSystem>,
        shipping: Option<&robin_assets::shipping_datadir::ShippingDatadir>,
    ) -> Result<Self, ResourcePreparationError> {
        // Hackable/headless missions may omit these archives. Malformed
        // authored content is an error, not a request for that fallback.
        let mut text = ResourceManager::with_files(files.clone());
        attach_mission_archive(&mut text, "Data/Text/Level.res", shipping, &files)?;

        let mut cursor = ResourceManager::with_files(files.clone());
        attach_mission_archive(&mut cursor, "Data/Interface/DEFAULT.RES", shipping, &files)?;
        Ok(Self { text, cursor })
    }

    pub(in crate::game_session) fn engine_setup_resources(
        &mut self,
        host: &mut Host,
    ) -> (
        Option<engine_api::GroundMarkSpriteData>,
        Vec<u16>,
        Option<engine_api::MinimapWidgetSetup>,
    ) {
        engine_setup_resources(&mut self.cursor, host)
    }
}

fn engine_setup_resources(
    cursor: &mut ResourceManager,
    host: &mut Host,
) -> (
    Option<engine_api::GroundMarkSpriteData>,
    Vec<u16>,
    Option<engine_api::MinimapWidgetSetup>,
) {
    let ground_mark_sprite = extract_ground_mark_sprite_data(cursor);
    if let Some(data) = ground_mark_sprite.as_ref() {
        host.frontend.install_trajectory_ground_mark_sprite(data);
    }
    (
        ground_mark_sprite,
        extract_titbit_row_frame_counts(cursor),
        extract_minimap_widget_setup(cursor),
    )
}

impl MissionProcessResources {
    pub(in crate::game_session) fn load(
        host: &mut Host,
        game: &Game,
        play_loading_menu_music: bool,
    ) -> Result<Self, ResourcePreparationError> {
        let audio_backend = init_audio_backend(host, game, play_loading_menu_music);

        let MissionEngineResources { text, cursor } = MissionEngineResources::load(host)?;

        Ok(Self {
            text,
            interface: cursor,
            audio_backend,
        })
    }

    /// Move the interface archive onto a worker that eagerly decodes every
    /// encoded (JXL) picture, so frontend assembly finds them ready instead
    /// of decoding hundreds of interface images on the loading path. No-op
    /// when no worker can run it (single-threaded wasm) — the lazy per-
    /// resource decode then behaves exactly as before.
    pub(in crate::game_session) fn start_interface_decode(
        self,
    ) -> MissionProcessResources<DecodingInterfaceResources> {
        let Self {
            text,
            interface: cursor,
            audio_backend,
        } = self;
        MissionProcessResources {
            text,
            interface: DecodingInterfaceResources::start(cursor),
            audio_backend,
        }
    }

    pub(in crate::game_session) fn engine_setup_resources(
        &mut self,
        host: &mut Host,
    ) -> (
        Option<engine_api::GroundMarkSpriteData>,
        Vec<u16>,
        Option<engine_api::MinimapWidgetSetup>,
    ) {
        engine_setup_resources(&mut self.interface, host)
    }
}

impl DecodingInterfaceResources {
    fn start(cursor: ResourceManager) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let handle = std::thread::Builder::new()
                .name("interface-decode".into())
                .spawn(move || decode_interface_managers(cursor))
                .expect("failed to spawn interface decode thread");
            Self::Thread(handle)
        }
        #[cfg(target_arch = "wasm32")]
        {
            #[cfg(feature = "wasm-threads")]
            if robin_assets::wasm_threads::pool_threads() > 0 {
                return Self::Pool(robin_assets::wasm_threads::start_on_pool(move || {
                    decode_interface_managers(cursor)
                }));
            }
            Self::Ready { cursor }
        }
    }

    /// Collect the `(cursor, menu)` interface managers for frontend
    /// assembly, waiting for the pre-decode worker when one is running.
    /// Never blocks the wasm main thread (the pool variant is awaited).
    async fn collect(self) -> (ResourceManager, ResourceManager) {
        match self {
            #[cfg(target_arch = "wasm32")]
            Self::Ready { cursor } => {
                // No worker ran: hand the menus their own lazily-decoded
                // copy, exactly like the old second attach.
                let menu = cursor.duplicate();
                (cursor, menu)
            }
            #[cfg(not(target_arch = "wasm32"))]
            Self::Thread(handle) => handle.join().expect("interface decode thread panicked"),
            #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
            Self::Pool(receiver) => receiver
                .await
                .expect("interface decode worker dropped its result"),
        }
    }
}

impl MissionProcessResources<DecodingInterfaceResources> {
    pub(in crate::game_session) async fn collect(
        self,
    ) -> (
        ResourceManager,
        ResourceManager,
        ResourceManager,
        Option<KiraAudioBackend>,
    ) {
        let (cursor, menu) = self.interface.collect().await;
        (self.text, cursor, menu, self.audio_backend)
    }
}

impl<Interface> MissionProcessResources<Interface> {
    pub(in crate::game_session) fn resolve_short_briefings(
        &mut self,
        level_descriptors: Option<&assets_res_descr::LevelDescriptors>,
    ) -> Result<std::collections::HashMap<u32, String>, ResourcePreparationError> {
        localization::resolve_short_briefings(&mut self.text, level_descriptors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::resource_ids;
    #[test]
    fn required_selected_locale_archive_is_not_optional_absence() {
        use robin_assets::shipping_datadir::{ShippingAssets, ShippingLocale};
        let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        let mut datadir = ShippingDatadir::default();
        datadir
            .locales
            .insert("en-US".into(), ShippingLocale::default());
        let installed = ShippingAssets::install(std::sync::Arc::new(datadir), vfs.clone()).unwrap();
        installed
            .datadir()
            .set_active_locale(Some("en-US"))
            .unwrap();
        let files = std::sync::Arc::new(SbFileSystem::new(vfs).snapshot());
        let mut text = ResourceManager::with_files(files.clone());
        let error = attach_mission_archive(
            &mut text,
            "Data/Text/Level.res",
            Some(installed.datadir()),
            &files,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ResourcePreparationError::Unavailable { .. }
        ));
        assert!(error.to_string().contains("required selected-locale"));
    }
    fn interface_stage_fixture() -> ResourceManager {
        use std::sync::Arc;
        let picture = robin_assets::picture::Picture {
            width: 2,
            height: 1,
            pitch: 4,
            pixel_format: robin_assets::picture::PixelFormat::Rgb16,
            data: vec![0xc0, 0x07, 0xff, 0xff],
            palette: None,
        };
        // Load a real two-entry legacy archive through the owned reader. This
        // avoids relying on flattened JSON maps to decode integer resource IDs.
        let mut bytes = b"SRES".to_vec();
        bytes.extend_from_slice(&0x0100u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(b"PIC ");
        bytes.extend_from_slice(&resource_ids::RHID_GROUND_FOCUS.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend(
            picture
                .write_sixteen_to_bytes(robin_assets::picture::SixteenPacking::None)
                .unwrap(),
        );
        bytes.extend_from_slice(b"TEXT");
        bytes.extend_from_slice(&123u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        let text: Vec<_> = "stage fixture".encode_utf16().collect();
        bytes.extend_from_slice(&(text.len() as u16).to_le_bytes());
        for unit in text {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let vfs = Arc::new(robin_util::asset_fs::AssetVfs::new());
        vfs.install_preloaded_asset("stage.res", bytes).unwrap();
        let files = Arc::new(engine_sbfile::SbFileSystem::new(vfs).snapshot());
        let mut resources = ResourceManager::with_files(files);
        resources.attach_resource_file("stage.res").unwrap();
        resources
    }

    fn short_briefing_archive(entries: &[&[u16]]) -> Vec<u8> {
        let mut bytes = b"SRES".to_vec();
        bytes.extend_from_slice(&0x0100u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(b"TEXT");
        bytes.extend_from_slice(&123u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for entry in entries {
            bytes.extend_from_slice(&(entry.len() as u16).to_le_bytes());
            for unit in *entry {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
        }
        bytes
    }

    fn short_briefing_process(text: ResourceManager) -> MissionProcessResources {
        MissionProcessResources {
            text,
            interface: ResourceManager::new(),
            audio_backend: None,
        }
    }

    #[test]
    fn optional_short_briefing_absence_keeps_custom_text_ids() {
        let mut process = short_briefing_process(ResourceManager::new());
        assert!(process.resolve_short_briefings(None).unwrap().is_empty());
        let mut descriptor = assets_res_descr::LevelDescriptors::default();
        descriptor.short_briefing.text_table_id = 123;
        assert!(
            process
                .resolve_short_briefings(Some(&descriptor))
                .unwrap()
                .is_empty()
        );
        descriptor.custom_short_briefings = vec![None, Some("custom only".into())];
        assert_eq!(
            process.resolve_short_briefings(Some(&descriptor)).unwrap(),
            std::collections::HashMap::from([(1, "custom only".into())])
        );
    }

    #[test]
    fn short_briefing_overrides_preserve_legacy_entries_and_sparse_ids() {
        let mut process = short_briefing_process(interface_stage_fixture());
        let mut descriptor = assets_res_descr::LevelDescriptors::default();
        descriptor.short_briefing.text_table_id = 123;
        descriptor.custom_short_briefings = vec![None, None, Some("extra".into())];
        assert_eq!(
            process.resolve_short_briefings(Some(&descriptor)).unwrap(),
            std::collections::HashMap::from([(0, "stage fixture".into()), (2, "extra".into()),])
        );
        descriptor.custom_short_briefings[0] = Some(String::new());
        assert_eq!(
            process.resolve_short_briefings(Some(&descriptor)).unwrap(),
            std::collections::HashMap::from([(0, String::new()), (2, "extra".into())])
        );
    }

    #[test]
    fn malformed_mission_text_archive_reports_path_table_and_entry() {
        use std::sync::Arc;
        let vfs = Arc::new(robin_util::asset_fs::AssetVfs::new());
        let path = "Data/Text/Level.res";
        vfs.install_preloaded_asset(path, short_briefing_archive(&[&[65], &[0xd800]]))
            .unwrap();
        let files = Arc::new(engine_sbfile::SbFileSystem::new(vfs).snapshot());
        let mut text = ResourceManager::with_files(files.clone());
        let error = attach_mission_archive(&mut text, path, None, &files).unwrap_err();
        assert!(matches!(error, ResourcePreparationError::Malformed { .. }));
        let error = error.to_string();
        assert!(error.contains(path), "{error}");
        assert!(error.contains("resource 123 (TEXT)"), "{error}");
        assert!(error.contains("string 1: invalid UTF-16"), "{error}");
    }

    #[test]
    fn short_briefing_wrong_resource_kind_is_not_optional_absence() {
        let mut process = short_briefing_process(interface_stage_fixture());
        let mut descriptor = assets_res_descr::LevelDescriptors::default();
        descriptor.short_briefing.text_table_id = resource_ids::RHID_GROUND_FOCUS;
        // Overrides do not mask corrupt registered tables. They replace
        // valid base entries or supply text when the base is absent.
        descriptor.custom_short_briefings = vec![Some("override".into())];
        let error = process
            .resolve_short_briefings(Some(&descriptor))
            .unwrap_err();
        assert!(matches!(error, ResourcePreparationError::Malformed { .. }));
        let error = error.to_string();
        assert!(error.contains("Data/Text/Level.res: short-briefing table"));
        assert!(error.contains("not found"), "{error}");
    }

    #[test]
    fn optional_minimap_setup_accepts_absent_and_empty_picture_collections() {
        assert!(extract_minimap_widget_setup(&mut ResourceManager::new()).is_none());
        let mut bytes = b"SRES".to_vec();
        bytes.extend_from_slice(&0x0100u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(b"PICC");
        bytes.extend_from_slice(&resource_ids::RHMAP_CORNER.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
        bytes.extend_from_slice(&0u32.to_le_bytes()); // picture count
        let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        vfs.install_preloaded_asset("empty-corner.res", bytes)
            .unwrap();
        let files = std::sync::Arc::new(engine_sbfile::SbFileSystem::new(vfs).snapshot());
        let mut cursor = ResourceManager::with_files(files);
        cursor.attach_resource_file("empty-corner.res").unwrap();
        assert!(cursor.has_picture_resource(resource_ids::RHMAP_CORNER));
        assert!(extract_minimap_widget_setup(&mut cursor).is_none());
    }

    #[test]
    fn graphical_and_headless_stages_extract_identical_engine_metadata() {
        let cursor = interface_stage_fixture();
        let mut graphical = MissionProcessResources {
            text: ResourceManager::new(),
            interface: cursor.duplicate(),
            audio_backend: None,
        };
        let mut headless = MissionEngineResources {
            text: ResourceManager::new(),
            cursor,
        };
        let mut graphical_host = Host::scratch(800.0, 600.0);
        let mut headless_host = Host::scratch(800.0, 600.0);
        let graphical_metadata = graphical.engine_setup_resources(&mut graphical_host);
        let headless_metadata = headless.engine_setup_resources(&mut headless_host);
        let ground = graphical_metadata
            .0
            .as_ref()
            .expect("fixture ground geometry");
        assert_eq!(ground.frame_sizes, vec![(2, 1)]);
        assert_eq!(ground.per_frame_offsets, vec![(1, 0)]);
        let headless_ground = headless_metadata
            .0
            .as_ref()
            .expect("headless ground geometry");
        assert_eq!(ground.half_w, headless_ground.half_w);
        assert_eq!(ground.half_h, headless_ground.half_h);
        assert_eq!(ground.frame_sizes, headless_ground.frame_sizes);
        assert_eq!(ground.per_frame_offsets, headless_ground.per_frame_offsets);
        assert_eq!(graphical_metadata.1, headless_metadata.1);
        match (graphical_metadata.2, headless_metadata.2) {
            (Some(graphical), Some(headless)) => {
                assert_eq!(graphical.corner_size, headless.corner_size);
                // HitMask already has a persisted representation, including
                // its private dimensions and every opacity bit.
                assert_eq!(
                    serde_json::to_value(graphical.button_hit_mask).unwrap(),
                    serde_json::to_value(headless.button_hit_mask).unwrap()
                );
            }
            (None, None) => {}
            _ => panic!("graphical and headless minimap presence differs"),
        }
    }

    #[test]
    fn mission_archives_require_owned_preparation_authority() {
        let host = Host::scratch(800.0, 600.0);
        assert!(matches!(
            MissionEngineResources::load(&host),
            Err(ResourcePreparationError::MissingAuthority(_))
        ));
    }

    #[test]
    fn optional_mission_archives_distinguish_absence_from_malformed_content() {
        use robin_util::asset_fs::{AssetVfs, Bundle};
        use std::sync::Arc;
        for malformed in [false, true] {
            let vfs = Arc::new(AssetVfs::new());
            if malformed {
                vfs.mount_bundle_first(Arc::new(Bundle::from([
                    (
                        "Data/Text/Level.res".into(),
                        b"invalid archive".to_vec().into(),
                    ),
                    (
                        "Data/Interface/DEFAULT.RES".into(),
                        b"invalid archive".to_vec().into(),
                    ),
                ])))
                .unwrap();
            }
            let files = Arc::new(engine_sbfile::SbFileSystem::new(vfs).snapshot());
            let mut probe = ResourceManager::with_files(files.clone());
            assert!(probe.attach_resource_file("Data/Text/Level.res").is_err());
            let loaded = MissionEngineResources::load_archives(files, None);
            if malformed {
                assert!(matches!(
                    loaded,
                    Err(ResourcePreparationError::Malformed { .. })
                ));
            } else {
                let loaded = loaded.unwrap();
                assert!(loaded.text.is_empty());
                assert!(loaded.cursor.is_empty());
            }
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn interface_decode_consumes_ready_stage_and_preserves_both_frontend_views() {
        let process = MissionProcessResources {
            text: interface_stage_fixture(),
            interface: interface_stage_fixture(),
            audio_backend: None,
        };
        let decoding: MissionProcessResources<DecodingInterfaceResources> =
            process.start_interface_decode();
        let (mut text, mut cursor, mut menu, audio) = pollster::block_on(decoding.collect());
        assert!(audio.is_none());
        for manager in [&mut text, &mut cursor, &mut menu] {
            assert_eq!(manager.get_string(123, 0).unwrap(), "stage fixture");
        }
        assert_ne!(cursor.cache_identity(), menu.cache_identity());
    }
}
