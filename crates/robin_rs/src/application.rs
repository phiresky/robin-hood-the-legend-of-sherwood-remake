//! Application composition, shared service ownership and runtime authority.
//!
//! Game-host state consumes this context; application services are never restored
//! from serialized diagnostics. Normal and closed projection policies stay explicit.

use robin_assets::shipping_datadir::ShippingDatadir;
use robin_engine::engine as engine_api;
use robin_engine::player_profile::{PlayerProfile, PlayerProfileManager};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[cfg(not(target_arch = "wasm32"))]
use crate::distributed_mod_cache::DistributedModCache;
use crate::key_config::KeyConfig;
use crate::key_config_store::{KeyConfigStore, ProfileKeyConfig};
use crate::localization::{
    LanguageChange, LanguagePack, LanguageSelection, LocalizationPreferences, LocalizationService,
    PortTextKey,
};
use crate::spellforge_trust::{
    SpellforgeTrustGrant, SpellforgeTrustKey, SpellforgeTrustMetadata, SpellforgeTrustStore,
};

/// Mutable application services shared by clones of one
/// [`ApplicationContext`]. Separate contexts allocate separate service sets,
/// which makes tests, headless sessions, and future multi-instance hosts
/// independent instead of routing through process-wide singletons.
#[derive(Debug, Serialize)]
struct ApplicationServices {
    #[serde(skip)]
    http: Mutex<crate::http_server::HttpTransport>,
    #[serde(skip)]
    replay: Arc<crate::replay_service::ReplayService>,
    #[serde(skip)]
    recording_index: Arc<crate::mission_replays::RecordingIndex>,
    #[cfg(all(target_arch = "wasm32", feature = "audio"))]
    #[serde(skip)]
    browser_audio: std::cell::RefCell<Option<crate::web_audio_backend::BrowserAudioSession>>,
    #[serde(skip)]
    asset_cache: crate::process_asset_cache::ApplicationAssetCache,
    #[serde(skip)]
    cache_maintenance: crate::cache_maintenance::CacheMaintenance,
    #[serde(skip)]
    preparation_files: Option<Arc<robin_engine::sbfile::SbFileSystem>>,
    #[serde(skip)]
    profile_store: crate::player_profile_store::PlayerProfileStore,
    player_profiles: Mutex<PlayerProfileManager>,
    key_configs: Mutex<KeyConfigStore>,
    spellforge_trust: Mutex<SpellforgeTrustStore>,
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(skip)]
    distributed_mod_cache: Arc<Mutex<Result<DistributedModCache, String>>>,
    localization: Mutex<LocalizationService>,
    shipping: Option<Arc<ShippingDatadir>>,
    /// Process/application-lifetime owner for queued verification. This is
    /// host-only asynchronous state; the durable stores, not a serialized
    /// `ApplicationContext`, are its recovery boundary.
    #[serde(skip)]
    leaderboard_receipts: Mutex<crate::leaderboard_receipt_watcher::ApplicationReceiptWatcher>,
}

/// Construction policy is explicit: a closed projection must not load local trust,
/// caches, profiles or the recording index as a side effect of sharing assembly.
#[derive(Debug, Serialize, Deserialize)]
enum PersistencePolicy {
    Local(crate::player_profile_store::PlayerProfileStore),
    #[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
    ClosedOfficialProjection,
}

impl ApplicationServices {
    fn compose(
        policy: PersistencePolicy,
        player_profiles: PlayerProfileManager,
        key_configs: KeyConfigStore,
        shipping: Option<Arc<ShippingDatadir>>,
        localization: LocalizationService,
        preparation_files: Option<Arc<robin_engine::sbfile::SbFileSystem>>,
    ) -> Self {
        let trust_directory = key_configs.save_directory.clone();
        let (profile_store, persistence_enabled) = match policy {
            PersistencePolicy::Local(store) => (store, true),
            #[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
            PersistencePolicy::ClosedOfficialProjection => (
                crate::player_profile_store::PlayerProfileStore::unavailable(
                    "profiles disabled in closed official projection",
                ),
                false,
            ),
        };
        let spellforge_trust = if persistence_enabled {
            SpellforgeTrustStore::load(&trust_directory).unwrap_or_else(|error| {
                tracing::error!(
                    "Spellforge trust persistence is unavailable and remote code admission will fail closed: {error}"
                );
                SpellforgeTrustStore::unavailable(trust_directory.clone(), error)
            })
        } else {
            SpellforgeTrustStore::unavailable(
                trust_directory.clone(),
                "Spellforge trust is disabled in the closed official projection context",
            )
        };
        #[cfg(not(target_arch = "wasm32"))]
        let distributed_mod_cache = if persistence_enabled {
            DistributedModCache::open(&trust_directory).map_err(|error| {
                tracing::error!(
                    "distributed-mod cache is unavailable and host-distributed content admission will fail closed: {error}"
                );
                error
            })
        } else {
            Err(
                "distributed-mod cache is disabled in the closed official projection context"
                    .to_owned(),
            )
        };
        #[cfg(not(target_arch = "wasm32"))]
        let recording_index = if persistence_enabled {
            crate::mission_replays::RecordingIndex::native(
                crate::mission_replays::default_directory(),
            )
        } else {
            crate::mission_replays::RecordingIndex::disabled()
        };
        #[cfg(target_arch = "wasm32")]
        let recording_index = crate::mission_replays::RecordingIndex::disabled();
        Self {
            http: Mutex::new(Default::default()),
            replay: Arc::new(Default::default()),
            recording_index: Arc::new(recording_index),
            asset_cache: Default::default(),
            cache_maintenance: if persistence_enabled {
                crate::cache_maintenance::CacheMaintenance::new()
            } else {
                Default::default()
            },
            #[cfg(all(target_arch = "wasm32", feature = "audio"))]
            browser_audio: Default::default(),
            preparation_files,
            profile_store,
            player_profiles: Mutex::new(player_profiles),
            key_configs: Mutex::new(key_configs),
            spellforge_trust: Mutex::new(spellforge_trust),
            #[cfg(not(target_arch = "wasm32"))]
            distributed_mod_cache: Arc::new(Mutex::new(distributed_mod_cache)),
            localization: Mutex::new(localization),
            shipping,
            leaderboard_receipts: Mutex::new(Default::default()),
        }
    }
}

impl Drop for ApplicationServices {
    fn drop(&mut self) {
        #[cfg(all(target_arch = "wasm32", feature = "audio"))]
        if let Some(audio) = self.browser_audio.get_mut().as_ref() {
            audio.retire();
        }
        // Stop admission before releasing the application's replay owner. Its
        // Drop drains native work even when normal async shutdown was skipped.
        match self.http.get_mut() {
            Ok(http) => http.stop(),
            Err(error) => {
                tracing::error!("application HTTP transport poisoned during shutdown");
                error.into_inner().stop();
            }
        }
        self.replay.shutdown_on_drop();
        self.cache_maintenance.shutdown_on_drop();
        if let Err(error) = self.recording_index.shutdown() {
            tracing::error!("recording index shutdown failed: {error}");
        }
    }
}

/// Explicit application-owned configuration and persistence context.
///
/// `MissionLaunch` initially carries a bootstrap context containing only parsed
/// options. `rust_init` supplies the required profile/key/shipping services
/// before an async game loop begins. Service accessors project the required
/// owned data while holding a lock, so no lock guard can cross an `.await`.
/// Whole-profile snapshots are test helpers, not production service APIs.
///
/// ```compile_fail
/// use robin_rs::application::ApplicationContext;
/// fn snapshot(context: &ApplicationContext) {
///     let _ = context.active_profile_snapshot();
/// }
/// ```
///
/// ```compile_fail
/// use robin_rs::application::ApplicationContext;
/// fn snapshot(context: &ApplicationContext) {
///     let _ = context.player_profiles_snapshot();
/// }
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct ApplicationContext {
    // Launch overrides belong to this context; profiles and services are shared.
    options: engine_api::GlobalOptions,
    // Profile-derived state. sim_config() overlays this context's launch options
    // so changing one clone cannot silently change a sibling's launch authority.
    sim_config: Arc<Mutex<engine_api::SimConfig>>,
    services: Option<Arc<ApplicationServices>>,
}

/// Initialization proof required by application run loops. Serialized diagnostics
/// are not a recovery boundary: only explicit service composition grants authority.
#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct ReadyApplicationContext(ApplicationContext);

/// Data-only view of an application. Decode this instead of a live context.
/// There is deliberately no conversion into [`ReadyApplicationContext`].
///
/// ```compile_fail
/// use robin_rs::host::{ApplicationContextDiagnostic, ReadyApplicationContext};
/// fn restore(snapshot: ApplicationContextDiagnostic) -> ReadyApplicationContext {
///     snapshot.into()
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationContextDiagnostic {
    pub options: engine_api::GlobalOptions,
    pub sim_config: engine_api::SimConfig,
    pub services: Option<ApplicationServicesDiagnostic>,
}

impl ApplicationContextDiagnostic {
    /// Effective configuration including this clone's launch overrides.
    pub fn sim_config(&self) -> engine_api::SimConfig {
        effective_sim_config(&self.options, self.sim_config)
    }
}

/// Durable-looking fields are observations, not filesystem or runtime handles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationServicesDiagnostic {
    pub player_profiles: PlayerProfileManager,
    pub key_configs: KeyConfigStore,
    pub spellforge_trust: SpellforgeTrustStore,
    pub localization: LocalizationService,
    pub shipping: Option<Arc<ShippingDatadir>>,
}

fn effective_sim_config(
    options: &engine_api::GlobalOptions,
    mut config: engine_api::SimConfig,
) -> engine_api::SimConfig {
    let launcher = engine_api::SimConfig::from_options(options, config.difficulty);
    config.script_enabled = launcher.script_enabled;
    config.highlander = launcher.highlander;
    config.highlander2 = launcher.highlander2;
    config.golden_eye = launcher.golden_eye;
    config.ignore_default_loose = launcher.ignore_default_loose;
    config.bypass_fog_sprites_crash = launcher.bypass_fog_sprites_crash;
    config
}

impl<'de> Deserialize<'de> for ApplicationContext {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "application authority requires explicit composition; decode ApplicationContextDiagnostic instead",
        ))
    }
}

impl<'de> Deserialize<'de> for ReadyApplicationContext {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "ready application authority cannot be deserialized; decode ApplicationContextDiagnostic instead",
        ))
    }
}

impl TryFrom<ApplicationContext> for ReadyApplicationContext {
    type Error = String;

    fn try_from(context: ApplicationContext) -> Result<Self, Self::Error> {
        context.required_services()?;
        Ok(Self(context))
    }
}

impl From<ReadyApplicationContext> for ApplicationContext {
    fn from(context: ReadyApplicationContext) -> Self {
        context.0
    }
}

impl std::ops::Deref for ReadyApplicationContext {
    type Target = ApplicationContext;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ReadyApplicationContext {
    pub fn adopt_http_transport(
        self,
        transport: crate::http_server::HttpTransport,
    ) -> Result<Self, String> {
        self.0.adopt_http_transport(transport)?;
        Ok(self)
    }
    pub fn with_replay_service(
        self,
        replay: Arc<crate::replay_service::ReplayService>,
    ) -> Result<Self, String> {
        self.0.with_replay_service(replay).map(Self)
    }
    pub fn with_options(self, options: engine_api::GlobalOptions) -> Self {
        Self(self.0.with_options(options))
    }
}

impl ApplicationContext {
    /// Final application exit only. Mission retirement must not cancel frozen
    /// exports belonging to the previous recording generation.
    pub async fn shutdown(&self) -> Result<(), String> {
        let transport = self.stop_http_transport();
        let recordings = self.required_services()?.recording_index.shutdown();
        let exports = self.required_services()?.replay.shutdown().await;
        let maintenance = self.required_services()?.cache_maintenance.shutdown().await;
        let errors: Vec<_> = [transport, recordings, exports, maintenance]
            .into_iter()
            .filter_map(Result::err)
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    pub(crate) fn recording_index(&self) -> &Arc<crate::mission_replays::RecordingIndex> {
        &self
            .required_services()
            .expect("recording index requires initialized application authority")
            .recording_index
    }

    pub fn start_http_transport(&self, port: u16) -> Result<(), String> {
        self.required_services()?
            .http
            .lock()
            .map_err(|_| "application HTTP transport lock poisoned".to_owned())?
            .start(port, self.replay_exports(), self.replay_launches())
    }

    pub fn stop_http_transport(&self) -> Result<(), String> {
        self.required_services()?
            .http
            .lock()
            .map_err(|_| "application HTTP transport lock poisoned".to_owned())?
            .stop();
        Ok(())
    }

    pub fn drain_http_pre_engine(&self) -> Result<(), String> {
        self.required_services()?
            .http
            .lock()
            .map_err(|_| "application HTTP transport lock poisoned".to_owned())?
            .drain_pre_engine();
        Ok(())
    }

    pub fn attach_http_ingress(&self) -> Result<crate::http_server::SessionIngress, String> {
        Ok(self
            .required_services()?
            .http
            .lock()
            .map_err(|_| "application HTTP transport lock poisoned".to_owned())?
            .attach())
    }

    /// Transfer the early browser listener without manufacturing a second binding.
    pub fn adopt_http_transport(
        &self,
        transport: crate::http_server::HttpTransport,
    ) -> Result<(), String> {
        let mut destination = self
            .required_services()?
            .http
            .lock()
            .map_err(|_| "application HTTP transport lock poisoned".to_owned())?;
        if destination.is_started() {
            return Err("application HTTP transport is already started".to_owned());
        }
        if !transport.matches_replay(&self.replay_exports(), &self.replay_launches()) {
            return Err("HTTP transport belongs to different replay capabilities".to_owned());
        }
        *destination = transport;
        Ok(())
    }

    /// Composition-only injection before this application's services are shared.
    /// Browser boot uses this to share the authority installed before async startup.
    pub fn with_replay_service(
        mut self,
        replay: Arc<crate::replay_service::ReplayService>,
    ) -> Result<Self, String> {
        let services = self.services.as_mut().ok_or_else(|| {
            "replay injection requires initialized application services".to_owned()
        })?;
        let services = Arc::get_mut(services).ok_or_else(|| {
            "replay injection must precede sharing application services".to_owned()
        })?;
        if services
            .http
            .get_mut()
            .map_err(|_| "application HTTP transport lock poisoned".to_owned())?
            .is_started()
        {
            return Err(
                "replay injection must precede starting application HTTP transport".to_owned(),
            );
        }
        services.replay = replay;
        Ok(self)
    }
    pub(crate) fn replay_recording(&self) -> crate::replay_service::ReplayRecordingControl {
        self.required_services()
            .expect("replay requires initialized application authority")
            .replay
            .recording()
    }
    pub(crate) fn replay_exports(&self) -> crate::replay_service::ReplayExports {
        self.required_services()
            .expect("replay requires initialized application authority")
            .replay
            .exports()
    }
    pub(crate) fn replay_launches(&self) -> crate::replay_service::ReplayLaunches {
        self.required_services()
            .expect("replay requires initialized application authority")
            .replay
            .launches()
    }
    /// Create the pre-initialization context used while parsing launcher
    /// arguments. Accessing profiles, keys, or shipping before completion is
    /// an error rather than a fabricated empty service.
    pub fn bootstrap(options: engine_api::GlobalOptions) -> Self {
        let sim_config = engine_api::SimConfig::from_options(
            &options,
            robin_engine::player_profile::DifficultyLevel::Medium,
        );
        Self {
            options,
            sim_config: Arc::new(Mutex::new(sim_config)),
            services: None,
        }
    }

    pub fn complete(
        profile_store: crate::player_profile_store::PlayerProfileStore,
        options: engine_api::GlobalOptions,
        player_profiles: PlayerProfileManager,
        key_configs: KeyConfigStore,
        shipping: Option<Arc<ShippingDatadir>>,
    ) -> Result<Self, String> {
        Self::complete_with_localization(
            profile_store,
            options,
            player_profiles,
            key_configs,
            shipping,
            LocalizationService::disabled(),
        )
    }

    /// Construct the closed, in-memory application authority used by the
    /// native official simulation-content exporter.
    ///
    /// Unlike production startup, this path never reads or writes player
    /// profiles, key bindings, identities, localization preferences, saves,
    /// or mods. The exact deterministic config was decoded from the signed
    /// rules document before this constructor is called.
    #[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
    pub fn complete_official_projection(
        options: engine_api::GlobalOptions,
        sim_config: engine_api::SimConfig,
        shipping: Option<Arc<ShippingDatadir>>,
    ) -> Result<Self, String> {
        Self::complete_official_projection_with_files(options, sim_config, shipping, None)
    }

    #[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
    pub fn complete_official_projection_with_files(
        options: engine_api::GlobalOptions,
        sim_config: engine_api::SimConfig,
        shipping: Option<Arc<ShippingDatadir>>,
        preparation_files: Option<Arc<robin_engine::sbfile::SbFileSystem>>,
    ) -> Result<Self, String> {
        sim_config
            .validate()
            .map_err(|error| format!("invalid official projection SimConfig: {error}"))?;
        let option_config = engine_api::SimConfig::from_options(&options, sim_config.difficulty);
        if (
            option_config.script_enabled,
            option_config.highlander,
            option_config.highlander2,
            option_config.golden_eye,
            option_config.ignore_default_loose,
            option_config.bypass_fog_sprites_crash,
        ) != (
            sim_config.script_enabled,
            sim_config.highlander,
            sim_config.highlander2,
            sim_config.golden_eye,
            sim_config.ignore_default_loose,
            sim_config.bypass_fog_sprites_crash,
        ) {
            return Err(
                "official projection options differ from the exact supplied SimConfig".to_owned(),
            );
        }

        const MEMORY_ONLY_SAVE_DIRECTORY: &str = "official-projection-memory-only";
        let mut player_profiles = PlayerProfileManager::new(MEMORY_ONLY_SAVE_DIRECTORY.to_owned());
        let active_index =
            player_profiles.create_profile("Official Projection".to_owned(), sim_config.difficulty);
        player_profiles.set_active(active_index);
        let active = player_profiles
            .get_active_mut()
            .ok_or_else(|| "official projection profile was not activated".to_owned())?;
        active.sound_config.amount_of_speaking = sim_config.amount_of_speaking;
        sim_config.copy_gameplay_to_profile(&mut active.gameplay_config);

        let mut key_configs = KeyConfigStore::new(MEMORY_ONLY_SAVE_DIRECTORY.to_owned());
        key_configs.entry_or_default(active.id);
        Ok(Self {
            options,
            sim_config: Arc::new(Mutex::new(sim_config)),
            services: Some(Arc::new(ApplicationServices::compose(
                PersistencePolicy::ClosedOfficialProjection,
                player_profiles,
                key_configs,
                shipping,
                LocalizationService::disabled(),
                preparation_files,
            ))),
        })
    }

    /// Complete a production application context with localization already
    /// installed. Keeping this separate from [`Self::complete`] prevents unit
    /// test contexts from mutating the process-wide resource search order.
    pub fn complete_with_localization(
        profile_store: crate::player_profile_store::PlayerProfileStore,
        options: engine_api::GlobalOptions,
        player_profiles: PlayerProfileManager,
        key_configs: KeyConfigStore,
        shipping: Option<Arc<ShippingDatadir>>,
        localization: LocalizationService,
    ) -> Result<Self, String> {
        Self::complete_with_localization_and_files(
            profile_store,
            options,
            player_profiles,
            key_configs,
            shipping,
            localization,
            None,
        )
    }

    /// Supply resource authority at construction, never through serialized state.
    pub fn complete_with_localization_and_files(
        profile_store: crate::player_profile_store::PlayerProfileStore,
        options: engine_api::GlobalOptions,
        mut player_profiles: PlayerProfileManager,
        mut key_configs: KeyConfigStore,
        shipping: Option<Arc<ShippingDatadir>>,
        localization: LocalizationService,
        preparation_files: Option<Arc<robin_engine::sbfile::SbFileSystem>>,
    ) -> Result<Self, String> {
        // Storage authority is selected by the caller, never decoded from archive metadata.
        player_profiles.save_directory = profile_store
            .directory()
            .map_err(|error| error.to_string())?
            .to_owned();
        // Recovery failures are initialization errors, not corrupt profiles:
        // never route them through the launcher's default-profile regeneration.
        profile_store
            .restore_interrupted_deletions(&player_profiles)
            .map_err(|error| format!("recover interrupted player deletion: {error}"))?;
        let active = player_profiles
            .get_active()
            .ok_or_else(|| "ApplicationContext requires an active player profile".to_string())?;
        let difficulty = active.difficulty;
        let amount_of_speaking = active.sound_config.amount_of_speaking;
        let gameplay_config = active.gameplay_config;

        // The original-game player profile stores
        // active and custom key configs on each player profile, and
        // The original game snapshots the active
        // profile's bindings into the mission input translator. The Rust port
        // keeps the host-side key type in a parallel store keyed by the same
        // profile id.
        // Profiles are the deletion commit point; stale bindings are cleanup.
        let previous_keys = key_configs.configs.len();
        key_configs.configs.retain(|id, _| {
            player_profiles
                .profiles
                .iter()
                .any(|profile| profile.id == *id)
        });
        if key_configs.configs.len() != previous_keys {
            if let Err(error) = key_configs.save() {
                tracing::warn!("Cannot persist orphan key-configuration cleanup: {error}");
            }
        }
        for profile in &player_profiles.profiles {
            let entry = key_configs.entry_or_default(profile.id);
            entry.active.migrate_post_port_bindings();
            entry.custom.migrate_post_port_bindings();
        }

        let sim_config =
            profile_sim_config(&options, difficulty, amount_of_speaking, gameplay_config);
        Ok(Self {
            sim_config: Arc::new(Mutex::new(sim_config)),
            options,
            services: Some(Arc::new(ApplicationServices::compose(
                PersistencePolicy::Local(profile_store),
                player_profiles,
                key_configs,
                shipping,
                localization,
                preparation_files,
            ))),
        })
    }

    /// Replace this clone's launch overrides without changing its siblings.
    /// Shared profile updates remain visible through every clone's sim_config().
    pub fn with_options(mut self, options: engine_api::GlobalOptions) -> Self {
        self.options = options;
        self
    }

    pub fn preparation_files(&self) -> Result<&Arc<robin_engine::sbfile::SbFileSystem>, String> {
        self.required_services()?
            .preparation_files
            .as_ref()
            .ok_or_else(|| "application resource preparation authority is unavailable".to_owned())
    }

    /// Parsed assets and background preparation belong to this application.
    /// Clones share the owner; decoded contexts cannot recover runtime authority.
    pub fn asset_cache(
        &self,
    ) -> Result<&crate::process_asset_cache::ApplicationAssetCache, String> {
        self.preparation_files()?;
        Ok(&self.required_services()?.asset_cache)
    }

    pub fn options(&self) -> &engine_api::GlobalOptions {
        &self.options
    }

    pub fn sim_config(&self) -> engine_api::SimConfig {
        let config = *self
            .sim_config
            .lock()
            .expect("ApplicationContext sim-config lock poisoned");
        effective_sim_config(&self.options, config)
    }

    pub fn shipping(&self) -> Result<Option<&ShippingDatadir>, String> {
        Ok(self.required_services()?.shipping.as_deref())
    }

    pub fn shipping_arc(&self) -> Result<Option<Arc<ShippingDatadir>>, String> {
        Ok(self.required_services()?.shipping.clone())
    }

    #[cfg(all(target_arch = "wasm32", feature = "audio"))]
    pub fn browser_audio(&self) -> Result<crate::web_audio_backend::BrowserAudioSession, String> {
        let services = self.required_services()?;
        let mut slot = services.browser_audio.borrow_mut();
        if let Some(session) = slot.as_ref() {
            return Ok(session.clone());
        }
        let session = crate::web_audio_backend::BrowserAudioSession::new(
            self.preparation_files()?.clone(),
            services
                .shipping
                .clone()
                .ok_or("browser audio requires an application shipping catalog")?,
        )?;
        *slot = Some(session.clone());
        Ok(session)
    }

    pub fn localization_preferences(&self) -> Result<LocalizationPreferences, String> {
        self.with_localization(|localization| localization.preferences().clone())
    }

    pub fn installed_languages(&self) -> Result<Vec<LanguagePack>, String> {
        self.with_localization(|localization| localization.installed().to_vec())
    }

    pub fn active_locale(&self) -> Result<Option<String>, String> {
        self.with_localization(|localization| localization.active_locale().map(str::to_owned))
    }

    pub fn localized_mission_name(&self, mission_id: u32, fallback: &str) -> String {
        self.with_localization(|localization| {
            localization
                .active_locale()
                .and_then(|locale| {
                    localization
                        .installed()
                        .iter()
                        .find(|pack| pack.locale == locale)
                })
                .and_then(|pack| pack.mission_names.get(&mission_id))
                .cloned()
                .unwrap_or_else(|| fallback.to_owned())
        })
        .unwrap_or_else(|error| panic!("mission title lost localization service: {error}"))
    }

    pub fn localization_generation(&self) -> Result<u64, String> {
        self.with_localization(LocalizationService::generation)
    }

    pub fn language_selector_visible(&self) -> Result<bool, String> {
        self.with_localization(LocalizationService::selector_visible)
    }

    pub fn canonical_speech_timing_locale(&self) -> Result<Option<String>, String> {
        self.with_localization(|localization| {
            localization
                .canonical_speech_timing_locale()
                .map(str::to_owned)
        })
    }

    pub fn port_text(&self, key: PortTextKey) -> Result<&'static str, String> {
        self.with_localization(|localization| {
            crate::localization::port_text(localization.active_locale(), key)
        })
    }

    pub fn format_port_text(
        &self,
        key: PortTextKey,
        arguments: &[(&str, &str)],
    ) -> Result<String, String> {
        self.with_localization(|localization| {
            crate::localization::format_port_text(localization.active_locale(), key, arguments)
        })?
        .map_err(|error| error.to_string())
    }

    pub fn set_language(&self, selection: LanguageSelection) -> Result<LanguageChange, String> {
        let services = self.required_services()?;
        let cache = &services.asset_cache;
        let mut localization = services
            .localization
            .lock()
            .map_err(|_| "ApplicationContext localization lock poisoned".to_string())?;
        let change = localization
            .set_selection(selection, services.shipping.as_deref())
            .map_err(|error| error.to_string())?;
        drop(localization);
        if change.previous_locale != change.active_locale {
            cache.invalidate_localized();
        }
        Ok(change)
    }

    fn with_localization<R>(
        &self,
        read: impl FnOnce(&LocalizationService) -> R,
    ) -> Result<R, String> {
        let localization = self
            .required_services()?
            .localization
            .lock()
            .map_err(|_| "ApplicationContext localization lock poisoned".to_string())?;
        Ok(read(&localization))
    }

    #[cfg(test)]
    pub(crate) fn player_profiles_snapshot(&self) -> Result<PlayerProfileManager, String> {
        self.with_player_profiles(Clone::clone)
    }

    #[cfg(test)]
    pub(crate) fn active_profile_snapshot(&self) -> Result<PlayerProfile, String> {
        self.with_active_profile(Clone::clone)
    }

    /// Read only the required active-profile values while holding its lock.
    /// Do not render, await, or re-enter profile services from this callback.
    pub(crate) fn with_active_profile<R>(
        &self,
        read: impl FnOnce(&PlayerProfile) -> R,
    ) -> Result<R, String> {
        self.with_player_profiles(|profiles| {
            profiles
                .active_index
                .and_then(|index| profiles.profiles.get(index))
                .map(read)
        })?
        .ok_or_else(|| "ApplicationContext has no active player profile".to_string())
    }

    pub(crate) fn with_player_profiles<R>(
        &self,
        read: impl FnOnce(&PlayerProfileManager) -> R,
    ) -> Result<R, String> {
        let profiles = self
            .required_services()?
            .player_profiles
            .lock()
            .map_err(|_| "ApplicationContext player-profile lock poisoned".to_string())?;
        Ok(read(&profiles))
    }

    /// Infallible in-memory update. Callbacks only edit the staged value;
    /// use an explicit persistence operation below to write it to storage.
    pub(crate) fn with_player_profiles_mut<R>(
        &self,
        update: impl FnOnce(&mut PlayerProfileManager) -> R,
    ) -> Result<R, String> {
        self.try_update_player_profiles(|profiles| Ok(update(profiles)))
    }

    /// Stage one fallible mutation and publish it with its derived simulation
    /// settings only on success. Callbacks must not reenter profile or simulation
    /// accessors. External side effects are not rolled back by this transaction.
    pub(crate) fn try_update_player_profiles<R>(
        &self,
        update: impl FnOnce(&mut PlayerProfileManager) -> Result<R, String>,
    ) -> Result<R, String> {
        self.update_profile_state(ProfilePersistence::MemoryOnly, update)
            .map(|publication| publication.value)
    }

    /// Validate before writing. A failure before publication discards the
    /// mutation; an already-visible replacement is published in memory too,
    /// with its durability error preserved in the returned receipt.
    pub(crate) fn try_persist_player_profiles<R>(
        &self,
        update: impl FnOnce(&mut PlayerProfileManager) -> Result<R, String>,
    ) -> Result<ProfilePublication<R>, String> {
        self.update_profile_state(ProfilePersistence::RequirePublication, update)
    }

    /// Validate and retain the change even if storage fails. The caller must
    /// surface the receipt's error; subsequent profile saves retry this state.
    pub(crate) fn update_and_retain_player_profiles<R>(
        &self,
        update: impl FnOnce(&mut PlayerProfileManager) -> R,
    ) -> Result<ProfilePublication<R>, String> {
        self.update_profile_state(ProfilePersistence::RetainOnFailure, |profiles| {
            Ok(update(profiles))
        })
    }

    fn update_profile_state<R>(
        &self,
        policy: ProfilePersistence,
        update: impl FnOnce(&mut PlayerProfileManager) -> Result<R, String>,
    ) -> Result<ProfilePublication<R>, String> {
        let mut profiles = self
            .required_services()?
            .player_profiles
            .lock()
            .map_err(|_| "ApplicationContext player-profile lock poisoned".to_string())?;
        // No callback gets persistence authority. Both validation and I/O are
        // owned here, with no opportunity to persist an unvalidated snapshot.
        let mut state = self
            .sim_config
            .lock()
            .map_err(|_| "ApplicationContext sim-config lock poisoned".to_string())?;
        let mut staged = profiles.clone();
        let result = update(&mut staged)?;
        let next = profile_derived_state(&staged, &state)?;
        let persistence = match policy {
            ProfilePersistence::MemoryOnly => Ok(()),
            ProfilePersistence::RequirePublication | ProfilePersistence::RetainOnFailure => {
                self.required_services()?.profile_store.save(&staged)
            }
        };
        if matches!(policy, ProfilePersistence::RequirePublication)
            && let Err(error) = &persistence
            && !profile_publication_visible(error)
        {
            return Err(format!("persist player profiles: {error}"));
        }
        *profiles = staged;
        *state = next;
        Ok(ProfilePublication {
            value: result,
            persistence: persistence.map_err(|error| error.to_string()),
        })
    }

    /// Replace the auto-created first-launch placeholder and its parallel key
    /// configuration while both context service locks are held. The returned
    /// id is the final active profile id that save/session construction must
    /// use. `None` keeps the placeholder but still finalizes first launch.
    pub(crate) fn complete_first_launch_profile(
        &self,
        replacement: Option<(String, robin_engine::player_profile::DifficultyLevel)>,
        screen_dims: (u32, u32),
    ) -> Result<u32, String> {
        let services = self.required_services()?;
        let profile_id = {
            // Keep this lock order (profiles, keys, trust, then simulation)
            // consistent for the operation that updates these services as one domain
            // transition. No guard escapes this synchronous method.
            let mut profiles_guard = services
                .player_profiles
                .lock()
                .map_err(|_| "ApplicationContext player-profile lock poisoned".to_string())?;
            let mut key_configs_guard = services
                .key_configs
                .lock()
                .map_err(|_| "ApplicationContext key-config lock poisoned".to_string())?;
            let mut spellforge_trust = services
                .spellforge_trust
                .lock()
                .map_err(|_| "ApplicationContext Spellforge-trust lock poisoned".to_string())?;
            let mut state = self
                .sim_config
                .lock()
                .map_err(|_| "ApplicationContext sim-config lock poisoned".to_string())?;

            if !profiles_guard.default_profiles {
                return Err("first-launch profile transition was already completed".to_string());
            }
            let profiles_before = profiles_guard.clone();
            let key_configs_before = key_configs_guard.clone();
            let mut staged_profiles = profiles_guard.clone();
            let mut staged_keys = key_configs_guard.clone();
            let profiles = &mut staged_profiles;
            let key_configs = &mut staged_keys;
            let mut removed_profile = None;

            if let Some((name, difficulty)) = replacement {
                if profiles.profiles.len() != 1 || profiles.active_index != Some(0) {
                    return Err(format!(
                        "first-launch replacement requires one active placeholder, found {} profiles with active index {:?}",
                        profiles.profiles.len(),
                        profiles.active_index,
                    ));
                }
                let placeholder_id = profiles.profiles[0].id;
                // Revoke authority before destroying or replacing the
                // profile.  If durable trust persistence is unavailable,
                // leave the complete profile/key domain untouched rather
                // than creating an orphaned approval for a deleted identity.
                spellforge_trust
                    .remove_profile(placeholder_id)
                    .map_err(|error| {
                        format!(
                            "failed to remove first-launch placeholder Spellforge trust: {error}"
                        )
                    })?;
                profiles.default_profiles = false;
                removed_profile = Some(placeholder_id);
                profiles.delete_profile(0);
                let index =
                    profiles.create_profile_with_screen_dims(name, difficulty, Some(screen_dims));
                profiles.set_active(index);
                let profile_id = profiles.profiles[index].id;

                key_configs.configs.remove(&placeholder_id);
                key_configs
                    .configs
                    .insert(profile_id, ProfileKeyConfig::fresh());
            } else {
                profiles.default_profiles = false;
            }

            let active = profiles.get_active().ok_or_else(|| {
                "first-launch transition did not leave an active profile".to_string()
            })?;
            let profile_id = active.id;
            let next = profile_derived_state(profiles, &state)?;
            if let Some(id) = removed_profile {
                if let Err(error) = services.profile_store.quarantine_profile_saves(id) {
                    let restored = services.profile_store.restore_profile_saves(id);
                    return Err(format!(
                        "quarantine placeholder saves: {error}; restore={restored:?}"
                    ));
                }
            }

            let persistence = services
                .profile_store
                .save(profiles)
                .map_err(|error| format!("persist player profile: {error}"))
                .and_then(|()| {
                    key_configs
                        .save()
                        .map_err(|error| format!("persist key configuration: {error}"))
                });
            if let Err(error) = persistence {
                let profile_rollback = services.profile_store.save(&profiles_before);
                let key_rollback = key_configs_before.save();
                let save_rollback = if profile_rollback.is_ok() {
                    removed_profile.map(|id| services.profile_store.restore_profile_saves(id))
                } else {
                    None // Retain quarantine until startup reads the actual archive.
                };
                return Err(format!(
                    "failed to complete durable first-launch profile transition: {error}; rollback profile={profile_rollback:?}, keys={key_rollback:?}, saves={save_rollback:?}"
                ));
            }
            // Durable profile/key writes above retain their explicit best-effort
            // rollback contract; trust revocation stays fail-closed. Renamed saves
            // remain recoverable and startup uses profiles.json to restore them.
            *profiles_guard = staged_profiles;
            *key_configs_guard = staged_keys;
            *state = next;
            profile_id
        };

        Ok(profile_id)
    }

    /// Retry the current validated profile state, including retained changes.
    pub(crate) fn save_player_profiles(&self) -> Result<ProfilePublication<()>, String> {
        self.with_player_profiles(|profiles| {
            profiles
                .validate_archive()
                .map_err(|error| format!("invalid player profiles: {error}"))?;
            Ok(ProfilePublication {
                value: (),
                persistence: self
                    .required_services()?
                    .profile_store
                    .save(profiles)
                    .map_err(|error| error.to_string()),
            })
        })?
    }

    /// Profile metadata is the commit point. Save quarantine is reversible
    /// until that publication; stale key bindings are harmless cleanup afterward.
    pub(crate) fn delete_player_profile(&self, index: usize) -> Result<bool, String> {
        let services = self.required_services()?;
        // Same lock order as first-launch replacement.
        let mut profiles = services
            .player_profiles
            .lock()
            .map_err(|_| "ApplicationContext player-profile lock poisoned")?;
        let mut keys = services
            .key_configs
            .lock()
            .map_err(|_| "ApplicationContext key-config lock poisoned")?;
        let mut trust = services
            .spellforge_trust
            .lock()
            .map_err(|_| "ApplicationContext Spellforge-trust lock poisoned")?;
        let mut state = self
            .sim_config
            .lock()
            .map_err(|_| "ApplicationContext sim-config lock poisoned")?;
        let Some(profile) = profiles.profiles.get(index) else {
            return Ok(false);
        };
        if profiles.profiles.len() == 1 {
            tracing::warn!("Refusing to delete the final player profile");
            return Ok(false);
        }
        let id = profile.id;
        let mut staged = profiles.clone();
        staged.delete_profile(index);
        staged.set_active(0);
        let next = profile_derived_state(&staged, &state)?;
        services
            .profile_store
            .restore_profile_saves(id)
            .map_err(|error| format!("recover interrupted player deletion: {error}"))?;
        // Revocation deliberately fails closed and is never rolled back.
        trust.remove_profile(id)?;
        if let Err(error) = services.profile_store.quarantine_profile_saves(id) {
            let restored = services.profile_store.restore_profile_saves(id);
            return Err(format!(
                "quarantine player saves: {error}; restore={restored:?}"
            ));
        }
        if let Err(error) = services.profile_store.save(&staged) {
            if !profile_publication_visible(&error) {
                let restored = services.profile_store.restore_profile_saves(id);
                return Err(format!(
                    "persist player deletion: {error}; restore={restored:?}"
                ));
            }
            // Replacement happened: reverting only memory/saves would contradict
            // the visible archive. Keep quarantine and report durability uncertainty.
            tracing::error!("Player deletion published but durability is unconfirmed: {error}");
        }
        *profiles = staged;
        *state = next;
        keys.configs.remove(&id);
        if let Err(error) = keys.save() {
            tracing::warn!("Player deleted; obsolete key configuration cleanup failed: {error}");
        }
        Ok(true)
    }

    pub fn set_fog_tint_all_sprites_for_tool(&self, enabled: bool) -> Result<(), String> {
        self.with_player_profiles_mut(|profiles| {
            profiles
                .get_active_mut()
                .expect("context requires active profile")
                .graphic_config
                .apply_fog_to_all_sprites = enabled;
        })
    }

    pub(crate) fn active_profile_save_directory(&self) -> Result<std::path::PathBuf, String> {
        self.with_player_profiles(|profiles| {
            let profile = profiles.get_active().ok_or_else(|| {
                "ApplicationContext has no active profile for save directory".to_string()
            })?;
            Ok(std::path::Path::new(
                self.required_services()?
                    .profile_store
                    .directory()
                    .map_err(|error| error.to_string())?,
            )
            .join(robin_engine::player_profile::profile_save_subdirectory(
                profile.id,
            )))
        })?
    }

    pub(crate) fn with_key_configs<R>(
        &self,
        read: impl FnOnce(&KeyConfigStore) -> R,
    ) -> Result<R, String> {
        let keys = self
            .required_services()?
            .key_configs
            .lock()
            .map_err(|_| "ApplicationContext key-config lock poisoned".to_string())?;
        Ok(read(&keys))
    }

    pub(crate) fn with_key_configs_mut<R>(
        &self,
        update: impl FnOnce(&mut KeyConfigStore) -> R,
    ) -> Result<R, String> {
        let mut keys = self
            .required_services()?
            .key_configs
            .lock()
            .map_err(|_| "ApplicationContext key-config lock poisoned".to_string())?;
        Ok(update(&mut keys))
    }

    pub(crate) fn with_spellforge_trust<R>(
        &self,
        read: impl FnOnce(&SpellforgeTrustStore) -> R,
    ) -> Result<R, String> {
        let trust = self
            .required_services()?
            .spellforge_trust
            .lock()
            .map_err(|_| "ApplicationContext Spellforge-trust lock poisoned".to_string())?;
        Ok(read(&trust))
    }

    pub(crate) fn with_spellforge_trust_mut<R>(
        &self,
        update: impl FnOnce(&mut SpellforgeTrustStore) -> R,
    ) -> Result<R, String> {
        let mut trust = self
            .required_services()?
            .spellforge_trust
            .lock()
            .map_err(|_| "ApplicationContext Spellforge-trust lock poisoned".to_string())?;
        Ok(update(&mut trust))
    }

    pub fn active_spellforge_trust_grants(&self) -> Result<Vec<SpellforgeTrustGrant>, String> {
        let profile_id = self.with_active_profile(|profile| profile.id)?;
        self.with_spellforge_trust(|trust| {
            trust.require_available()?;
            Ok(trust.grants_for_profile(profile_id).to_vec())
        })?
    }

    pub fn is_spellforge_content_trusted(&self, key: SpellforgeTrustKey) -> Result<bool, String> {
        let profile_id = self.with_active_profile(|profile| profile.id)?;
        self.with_spellforge_trust(|trust| trust.is_trusted(profile_id, key))?
    }

    pub fn grant_spellforge_content_trust(
        &self,
        key: SpellforgeTrustKey,
        metadata: SpellforgeTrustMetadata,
        approved_unix_seconds: u64,
    ) -> Result<(), String> {
        let profile_id = self.with_active_profile(|profile| profile.id)?;
        self.with_spellforge_trust_mut(|trust| {
            trust.grant(profile_id, key, metadata, approved_unix_seconds)
        })?
    }

    pub fn revoke_spellforge_content_trust(&self, key: SpellforgeTrustKey) -> Result<bool, String> {
        let profile_id = self.with_active_profile(|profile| profile.id)?;
        self.with_spellforge_trust_mut(|trust| trust.revoke(profile_id, key))?
    }

    pub fn revoke_all_spellforge_content_trust(&self) -> Result<usize, String> {
        let profile_id = self.with_active_profile(|profile| profile.id)?;
        self.with_spellforge_trust_mut(|trust| trust.revoke_all(profile_id))?
    }

    pub fn reset_spellforge_trust_store(&self) -> Result<(), String> {
        self.with_spellforge_trust_mut(SpellforgeTrustStore::reset)?
    }

    /// Revoke all approvals after the complete player-profile identity domain
    /// was regenerated. Failure to rewrite the store must not prevent
    /// ordinary offline startup, but it must make old grants unusable until a
    /// later explicit reset succeeds.
    pub(crate) fn reset_spellforge_trust_after_profile_recovery(&self) -> Result<(), String> {
        self.with_spellforge_trust_mut(|trust| {
            if let Err(error) = trust.reset() {
                trust.fail_closed(format!(
                    "profile identities were regenerated but durable revocation failed: {error}"
                ));
                return Err(error);
            }
            Ok(())
        })?
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn with_distributed_mod_cache_mut<R>(
        &self,
        update: impl FnOnce(&mut DistributedModCache) -> Result<R, String>,
    ) -> Result<R, String> {
        let mut cache = self
            .required_services()?
            .distributed_mod_cache
            .lock()
            .map_err(|_| "ApplicationContext distributed-mod cache lock poisoned".to_string())?;
        match cache.as_mut() {
            Ok(cache) => update(cache),
            Err(error) => Err(format!("distributed-mod cache is unavailable: {error}")),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn clear_distributed_mod_cache(&self) -> Result<usize, String> {
        self.with_distributed_mod_cache_mut(DistributedModCache::clear)
    }

    pub(crate) fn cache_clear_status(
        &self,
    ) -> Result<crate::cache_maintenance::CacheClearStatus, String> {
        self.required_services()?.cache_maintenance.status()
    }

    pub(crate) fn begin_distributed_mod_cache_clear(
        &self,
    ) -> Result<crate::cache_maintenance::CacheClearStatus, String> {
        let services = self.required_services()?;
        services.cache_maintenance.begin(
            #[cfg(not(target_arch = "wasm32"))]
            services.distributed_mod_cache.clone(),
        )
    }

    pub fn active_key_configs(&self) -> Result<(KeyConfig, KeyConfig), String> {
        let profile_id = self.with_active_profile(|profile| profile.id)?;
        self.with_key_configs(|key_configs| {
            key_configs
                .get(profile_id)
                .map(|entry| (entry.active.clone(), entry.custom.clone()))
        })?
        .ok_or_else(|| {
            format!("ApplicationContext has no key config for active profile {profile_id}")
        })
    }

    /// Persist a queued-verification handoff into the application-owned
    /// watcher. Callers must not discard the mission controller until this
    /// succeeds.
    pub(crate) fn enqueue_leaderboard_receipt_watch(
        &self,
        handoff: crate::leaderboard_receipt_watcher::QueuedSubmissionReceiptWatch,
    ) -> Result<bool, String> {
        let now_unix_ms =
            crate::leaderboard_receipt_watcher::now_unix_ms().map_err(|error| error.to_string())?;
        let mut watcher = self
            .required_services()?
            .leaderboard_receipts
            .lock()
            .map_err(|_| "ApplicationContext leaderboard-receipt lock poisoned".to_owned())?;
        watcher
            .enqueue(handoff, now_unix_ms)
            .map_err(|error| error.to_string())
    }

    /// Advance one cooperative verification-watch step. Errors stay visible
    /// in the watcher and are retried under its bounded policy; a frame must
    /// never block or fabricate an empty result.
    pub(crate) fn poll_leaderboard_receipts(&self) {
        let now_unix_ms = match crate::leaderboard_receipt_watcher::now_unix_ms() {
            Ok(now) => now,
            Err(error) => {
                tracing::error!("leaderboard receipt watcher clock failed: {error}");
                return;
            }
        };
        let services = match self.required_services() {
            Ok(services) => services,
            Err(error) => {
                tracing::error!("leaderboard receipt watcher lost application services: {error}");
                return;
            }
        };
        let mut watcher = match services.leaderboard_receipts.lock() {
            Ok(watcher) => watcher,
            Err(_) => {
                tracing::error!("ApplicationContext leaderboard-receipt lock poisoned");
                return;
            }
        };
        let _ = watcher.poll(now_unix_ms);
    }

    fn required_services(&self) -> Result<&ApplicationServices, String> {
        self.services.as_deref().ok_or_else(|| {
            "ApplicationContext services requested before rust initialization".to_string()
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[must_use = "profile persistence failures must be surfaced or explicitly handled"]
pub(crate) struct ProfilePublication<R> {
    pub(crate) value: R,
    pub(crate) persistence: Result<(), String>,
}

impl<R> ProfilePublication<R> {
    pub(crate) fn log_persistence_error(self, context: &str) -> R {
        if let Err(error) = self.persistence {
            tracing::error!("{context}: {error}; profile changes retained in memory for retry");
        }
        self.value
    }
}

enum ProfilePersistence {
    MemoryOnly,
    RequirePublication,
    RetainOnFailure,
}

fn profile_publication_visible(error: &std::io::Error) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        error
            .get_ref()
            .and_then(|error| {
                error.downcast_ref::<crate::desktop_persistence::PublicationFailure>()
            })
            .is_some_and(|failure| failure.published())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = error;
        false
    }
}

fn profile_derived_state(
    profiles: &PlayerProfileManager,
    previous: &engine_api::SimConfig,
) -> Result<engine_api::SimConfig, String> {
    let active = profiles
        .active_index
        .and_then(|index| profiles.profiles.get(index))
        .ok_or_else(|| {
            "ApplicationContext profile mutation must leave an active profile".to_string()
        })?;
    profiles
        .validate_archive()
        .map_err(|error| format!("invalid staged player profiles: {error}"))?;
    let mut config = profile_sim_config(
        &engine_api::GlobalOptions::default(),
        active.difficulty,
        active.sound_config.amount_of_speaking,
        active.gameplay_config,
    );
    // Construction authority is not a profile preference.
    config.synchronous_pathfinding = previous.synchronous_pathfinding;
    Ok(config)
}

fn profile_sim_config(
    options: &engine_api::GlobalOptions,
    difficulty: robin_engine::player_profile::DifficultyLevel,
    amount_of_speaking: u16,
    gameplay_config: robin_engine::gameplay_config::GameplayConfig,
) -> engine_api::SimConfig {
    let mut sim_config = engine_api::SimConfig::from_options(options, difficulty);
    sim_config.amount_of_speaking = amount_of_speaking;
    sim_config.apply_profile_gameplay(&gameplay_config);
    sim_config
}

impl Default for ApplicationContext {
    fn default() -> Self {
        Self::bootstrap(engine_api::GlobalOptions::default())
    }
}

impl std::ops::Deref for ApplicationContext {
    type Target = engine_api::GlobalOptions;

    fn deref(&self) -> &Self::Target {
        &self.options
    }
}

#[cfg(test)]
#[path = "application/tests.rs"]
mod application_context_tests;
