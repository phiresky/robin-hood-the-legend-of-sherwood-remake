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
    distributed_mod_cache: Mutex<Result<DistributedModCache, String>>,
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
            distributed_mod_cache: Mutex::new(distributed_mod_cache),
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
        if let Err(error) = self.recording_index.shutdown() {
            tracing::error!("recording index shutdown failed: {error}");
        }
    }
}

/// Explicit application-owned configuration and persistence context.
///
/// `CliArgs` initially carries a bootstrap context containing only parsed
/// options. `rust_init` supplies the required profile/key/shipping services
/// before an async game loop begins. Service accessors take snapshots while
/// holding a lock and return owned data, so no lock guard can cross an
/// `.await`.
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
        let errors: Vec<_> = [transport, recordings, exports]
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
        active.gameplay_config.fix_hard_reaction_times = sim_config.fix_hard_reaction_times;
        active.gameplay_config.enable_unbinding = sim_config.enable_unbinding;
        active.gameplay_config.clean_hands_npc_kills_invalidate =
            sim_config.clean_hands_npc_kills_invalidate;
        active.gameplay_config.reusable_cloaks = sim_config.reusable_cloaks;
        active.gameplay_config.reversible_background_patches =
            sim_config.reversible_background_patches;
        active.gameplay_config.item_gameplay = sim_config.item_gameplay;
        active.gameplay_config.noise_distraction_feedback = sim_config.noise_distraction_feedback;
        active.gameplay_config.sherwood_trading = sim_config.sherwood_trading;

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
            .map_err(|error| error.to_string())?;
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

    pub fn player_profiles_snapshot(&self) -> Result<PlayerProfileManager, String> {
        self.with_player_profiles(Clone::clone)
    }

    pub fn active_profile_snapshot(&self) -> Result<PlayerProfile, String> {
        self.with_player_profiles(|profiles| {
            profiles
                .active_index
                .and_then(|index| profiles.profiles.get(index))
                .cloned()
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

    pub(crate) fn with_player_profiles_mut<R>(
        &self,
        update: impl FnOnce(&mut PlayerProfileManager) -> R,
    ) -> Result<R, String> {
        let result = {
            let mut profiles = self
                .required_services()?
                .player_profiles
                .lock()
                .map_err(|_| "ApplicationContext player-profile lock poisoned".to_string())?;
            let result = update(&mut profiles);
            let active = profiles.get_active().ok_or_else(|| {
                "ApplicationContext profile mutation must leave an active profile".to_string()
            })?;
            // Publish before releasing the profile lock: concurrent updates
            // through sibling contexts must not publish snapshots out of order.
            self.refresh_profile_derived_state(
                active.difficulty,
                active.sound_config.amount_of_speaking,
                active.gameplay_config,
            )?;
            result
        };
        Ok(result)
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
            // Keep this lock order (profiles, then keys) consistent for the
            // only operation that must update both services as one domain
            // transition. No guard escapes this synchronous method.
            let mut profiles = services
                .player_profiles
                .lock()
                .map_err(|_| "ApplicationContext player-profile lock poisoned".to_string())?;
            let mut key_configs = services
                .key_configs
                .lock()
                .map_err(|_| "ApplicationContext key-config lock poisoned".to_string())?;
            let mut spellforge_trust = services
                .spellforge_trust
                .lock()
                .map_err(|_| "ApplicationContext Spellforge-trust lock poisoned".to_string())?;

            if !profiles.default_profiles {
                return Err("first-launch profile transition was already completed".to_string());
            }
            let profiles_before = profiles.clone();
            let key_configs_before = key_configs.clone();

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
                if let Err(error) = services.profile_store.remove_profile_saves(placeholder_id) {
                    tracing::warn!("failed to remove placeholder saves: {error}");
                }
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
            let difficulty = active.difficulty;
            let amount_of_speaking = active.sound_config.amount_of_speaking;
            let gameplay_config = active.gameplay_config;

            let persistence = services
                .profile_store
                .save(&profiles)
                .map_err(|error| format!("persist player profile: {error}"))
                .and_then(|()| {
                    key_configs
                        .save()
                        .map_err(|error| format!("persist key configuration: {error}"))
                });
            if let Err(error) = persistence {
                *profiles = profiles_before;
                *key_configs = key_configs_before;
                let profile_rollback = services.profile_store.save(&profiles);
                let key_rollback = key_configs.save();
                return Err(format!(
                    "failed to complete durable first-launch profile transition: {error}; rollback profile={profile_rollback:?}, keys={key_rollback:?}"
                ));
            }
            self.refresh_profile_derived_state(difficulty, amount_of_speaking, gameplay_config)?;
            profile_id
        };

        Ok(profile_id)
    }

    pub(crate) fn persist_player_profiles(
        &self,
        profiles: &PlayerProfileManager,
    ) -> std::io::Result<()> {
        self.required_services()
            .map_err(std::io::Error::other)?
            .profile_store
            .save(profiles)
    }

    pub(crate) fn remove_profile_saves(&self, id: u32) -> std::io::Result<()> {
        self.required_services()
            .map_err(std::io::Error::other)?
            .profile_store
            .remove_profile_saves(id)
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
                &self
                    .required_services()?
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
        let profile_id = self.active_profile_snapshot()?.id;
        self.with_spellforge_trust(|trust| {
            trust.require_available()?;
            Ok(trust.grants_for_profile(profile_id).to_vec())
        })?
    }

    pub fn is_spellforge_content_trusted(&self, key: SpellforgeTrustKey) -> Result<bool, String> {
        let profile_id = self.active_profile_snapshot()?.id;
        self.with_spellforge_trust(|trust| trust.is_trusted(profile_id, key))?
    }

    pub fn grant_spellforge_content_trust(
        &self,
        key: SpellforgeTrustKey,
        metadata: SpellforgeTrustMetadata,
        approved_unix_seconds: u64,
    ) -> Result<(), String> {
        let profile_id = self.active_profile_snapshot()?.id;
        self.with_spellforge_trust_mut(|trust| {
            trust.grant(profile_id, key, metadata, approved_unix_seconds)
        })?
    }

    pub fn revoke_spellforge_content_trust(&self, key: SpellforgeTrustKey) -> Result<bool, String> {
        let profile_id = self.active_profile_snapshot()?.id;
        self.with_spellforge_trust_mut(|trust| trust.revoke(profile_id, key))?
    }

    pub fn revoke_all_spellforge_content_trust(&self) -> Result<usize, String> {
        let profile_id = self.active_profile_snapshot()?.id;
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
        self.required_services()?
            .cache_maintenance
            .begin(self.clone())
    }

    pub fn active_key_configs(&self) -> Result<(KeyConfig, KeyConfig), String> {
        let profile_id = self.active_profile_snapshot()?.id;
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

    fn refresh_profile_derived_state(
        &self,
        difficulty: robin_engine::player_profile::DifficultyLevel,
        amount_of_speaking: u16,
        gameplay_config: robin_engine::gameplay_config::GameplayConfig,
    ) -> Result<(), String> {
        let mut state = self
            .sim_config
            .lock()
            .map_err(|_| "ApplicationContext sim-config lock poisoned".to_string())?;
        let mut sim_config = profile_sim_config(
            &engine_api::GlobalOptions::default(),
            difficulty,
            amount_of_speaking,
            gameplay_config,
        );
        // This is an explicit simulation-construction setting, not a profile
        // preference. Profile updates must not reset official/parity authority.
        sim_config.synchronous_pathfinding = state.synchronous_pathfinding;
        *state = sim_config;

        Ok(())
    }
}

fn profile_sim_config(
    options: &engine_api::GlobalOptions,
    difficulty: robin_engine::player_profile::DifficultyLevel,
    amount_of_speaking: u16,
    gameplay_config: robin_engine::gameplay_config::GameplayConfig,
) -> engine_api::SimConfig {
    let mut sim_config = engine_api::SimConfig::from_options(options, difficulty);
    sim_config.amount_of_speaking = amount_of_speaking;
    sim_config.fix_hard_reaction_times = gameplay_config.fix_hard_reaction_times;
    sim_config.enable_unbinding = gameplay_config.enable_unbinding;
    sim_config.clean_hands_npc_kills_invalidate = gameplay_config.clean_hands_npc_kills_invalidate;
    sim_config.reusable_cloaks = gameplay_config.reusable_cloaks;
    sim_config.reversible_background_patches = gameplay_config.reversible_background_patches;
    sim_config.item_gameplay = gameplay_config.item_gameplay;
    sim_config.noise_distraction_feedback = gameplay_config.noise_distraction_feedback;
    sim_config.sherwood_trading = gameplay_config.sherwood_trading;
    sim_config.enable_timed_missions = gameplay_config.enable_timed_missions;
    sim_config.enable_dynamic_ambience = gameplay_config.enable_dynamic_ambience;
    sim_config.diplomacy = gameplay_config.diplomacy;
    sim_config.npc_faction_wars = gameplay_config.npc_faction_wars;
    sim_config.more_combat_gestures = gameplay_config.more_combat_gestures;
    sim_config.gesture_quality_damage = gameplay_config.gesture_quality_damage;
    sim_config.fog_of_war = gameplay_config.fog_of_war;
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
mod application_context_tests {
    use super::*;
    use crate::host::{FrontendPreferenceEffects, FrontendPreferences, Host, HostFrontend};
    use robin_engine::player_profile::DifficultyLevel;
    use winit::keyboard::KeyCode;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recording_index_is_shared_only_within_application_and_retired_on_exit() {
        let directory = tempfile::tempdir().unwrap();
        let mut application = context(0, DifficultyLevel::Medium, KeyCode::F2, "index");
        let index = Arc::new(crate::mission_replays::RecordingIndex::native(
            directory.path().join("attempts"),
        ));
        Arc::get_mut(application.services.as_mut().unwrap())
            .unwrap()
            .recording_index = index.clone();
        let sibling = application.clone();
        let independent = context(0, DifficultyLevel::Medium, KeyCode::F2, "other-index");
        assert!(Arc::ptr_eq(
            application.recording_index(),
            sibling.recording_index()
        ));
        assert!(!Arc::ptr_eq(
            application.recording_index(),
            independent.recording_index()
        ));
        let diagnostic = serde_json::to_value(&application).unwrap();
        assert!(diagnostic["services"].get("recording_index").is_none());
        assert!(serde_json::from_value::<ApplicationContext>(diagnostic).is_err());
        index.refresh_index().unwrap();
        pollster::block_on(application.shutdown()).unwrap();
        assert!(
            sibling
                .recording_index()
                .refresh_index()
                .unwrap_err()
                .contains("shut down")
        );
        drop(application);
        drop(sibling);
        assert!(index.refresh_index().is_err());

        // Unstructured exits must retire the service even when a view retains
        // the narrow index capability beyond the last application context.
        let application = context(0, DifficultyLevel::Medium, KeyCode::F2, "drop-index");
        let index = application.recording_index().clone();
        drop(application);
        assert!(index.refresh_index().is_err());
    }

    #[test]
    fn cache_maintenance_is_application_owned_and_not_deserialized() {
        let first = context(42, DifficultyLevel::Medium, KeyCode::KeyA, "maintenance");
        let cloned = first.clone();
        let independent = context(42, DifficultyLevel::Medium, KeyCode::KeyA, "maintenance");
        assert!(std::ptr::eq(
            &first.required_services().unwrap().cache_maintenance,
            &cloned.required_services().unwrap().cache_maintenance,
        ));
        assert!(!std::ptr::eq(
            &first.required_services().unwrap().cache_maintenance,
            &independent.required_services().unwrap().cache_maintenance,
        ));
        assert_eq!(
            first.cache_clear_status().unwrap(),
            crate::cache_maintenance::CacheClearStatus::Idle
        );
        let encoded = serde_json::to_value(&first).unwrap();
        assert!(encoded["services"].get("cache_maintenance").is_none());
        let _: ApplicationContextDiagnostic = serde_json::from_value(encoded.clone()).unwrap();
        assert!(serde_json::from_value::<ApplicationContext>(encoded).is_err());
        assert!(ApplicationContext::default().cache_clear_status().is_err());
    }

    #[test]
    fn application_asset_cache_is_shared_by_clones_only_and_not_decoded() {
        let make_context = || {
            let mut application = context(42, DifficultyLevel::Medium, KeyCode::KeyA, "cache");
            let files = Arc::new(robin_engine::sbfile::SbFileSystem::new(Arc::new(
                robin_util::asset_fs::AssetVfs::new(),
            )));
            Arc::get_mut(application.services.as_mut().unwrap())
                .unwrap()
                .preparation_files = Some(files);
            application
        };
        let first = make_context();
        let cloned = first.clone();
        let independent = make_context();
        assert!(std::ptr::eq(
            first.asset_cache().unwrap(),
            cloned.asset_cache().unwrap()
        ));
        assert!(!std::ptr::eq(
            first.asset_cache().unwrap(),
            independent.asset_cache().unwrap()
        ));
        let encoded = serde_json::to_value(&first).unwrap();
        assert!(encoded["services"].get("asset_cache").is_none());
        let _: ApplicationContextDiagnostic = serde_json::from_value(encoded.clone()).unwrap();
        assert!(serde_json::from_value::<ApplicationContext>(encoded).is_err());
        assert!(ApplicationContext::default().asset_cache().is_err());
    }

    #[test]
    fn explicit_context_store_ignores_archive_directory_metadata() {
        let selected = tempfile::tempdir().unwrap();
        let redirected = tempfile::tempdir().unwrap();
        let store = crate::player_profile_store::PlayerProfileStore::for_directory(
            selected.path().to_str().unwrap(),
        );
        let mut profiles = store.load().unwrap();
        profiles.save_directory = redirected.path().to_str().unwrap().into();
        let archive = serde_json::to_string(&profiles).unwrap();
        let decoded = serde_json::from_str(&archive).unwrap();
        let context = ApplicationContext::complete(
            store,
            engine_api::GlobalOptions::default(),
            decoded,
            KeyConfigStore::new(selected.path().to_str().unwrap().into()),
            None,
        )
        .unwrap();
        context
            .with_player_profiles(|profiles| context.persist_player_profiles(profiles))
            .unwrap()
            .unwrap();
        assert!(
            context
                .active_profile_save_directory()
                .unwrap()
                .starts_with(selected.path())
        );
        assert!(!redirected.path().join("profiles.json").exists());
    }

    #[test]
    fn deserialized_context_cannot_recover_profile_storage_authority() {
        let original = context(
            42,
            DifficultyLevel::Medium,
            KeyCode::KeyA,
            "store-authority",
        );
        let encoded = serde_json::to_value(&original).unwrap();
        assert!(encoded["services"].get("profile_store").is_none());
        let diagnostic: ApplicationContextDiagnostic =
            serde_json::from_value(encoded.clone()).unwrap();
        assert!(
            diagnostic
                .services
                .unwrap()
                .player_profiles
                .get_active()
                .is_some()
        );
        assert!(serde_json::from_value::<ApplicationContext>(encoded.clone()).is_err());
        assert!(serde_json::from_value::<ReadyApplicationContext>(encoded).is_err());
    }

    fn spellforge_trust_key(value: u8) -> SpellforgeTrustKey {
        SpellforgeTrustKey {
            full_mod_sha256: [value; 32],
            package_sha256: Some([value.wrapping_add(1); 32]),
        }
    }

    fn spellforge_trust_metadata() -> SpellforgeTrustMetadata {
        SpellforgeTrustMetadata {
            mission: "Mission".into(),
            title: "Mission".into(),
            claimed_author: "Author".into(),
            version: "1".into(),
            source_url: "https://example.invalid/mod".into(),
            license: "CC0".into(),
            host_endpoint_id: "endpoint-public-key".into(),
            package_vm_abi: Some("spellforge-v1-sha256:00".into()),
            compressed_bytes: 123,
        }
    }

    fn context(
        profile_id: u32,
        difficulty: DifficultyLevel,
        key: KeyCode,
        shipping_marker: &str,
    ) -> ApplicationContext {
        let mut profiles = PlayerProfileManager::new(format!("/tmp/context-{profile_id}"));
        let profile_idx = profiles.create_profile(format!("Profile {profile_id}"), difficulty);
        profiles.set_active(profile_idx);

        let mut keys = KeyConfigStore::new(format!("/tmp/context-{profile_id}"));
        keys.entry_or_default(profile_id)
            .active
            .set_binding("ZoomIn", Some(key), None);

        let mut shipping = ShippingDatadir::default();
        shipping
            .raw
            .insert(shipping_marker.to_string(), vec![profile_id as u8]);

        ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&format!(
                "/tmp/context-{profile_id}"
            )),
            engine_api::GlobalOptions::default(),
            profiles,
            keys,
            Some(Arc::new(shipping)),
        )
        .unwrap()
    }

    #[test]
    fn independent_contexts_do_not_cross_talk() {
        let easy = context(0, DifficultyLevel::Easy, KeyCode::F2, "easy.marker");
        let hard = context(0, DifficultyLevel::Hard, KeyCode::F3, "hard.marker");

        let easy_host = Host::new(easy.clone().try_into().unwrap(), 1024.0, 768.0).unwrap();
        let hard_host = Host::new(hard.clone().try_into().unwrap(), 1024.0, 768.0).unwrap();

        assert_eq!(easy.sim_config().difficulty, DifficultyLevel::Easy);
        assert_eq!(hard.sim_config().difficulty, DifficultyLevel::Hard);
        assert_eq!(
            easy_host
                .frontend
                .preferences()
                .key_config()
                .get_binding("ZoomIn")
                .unwrap()
                .primary_key,
            Some(KeyCode::F2)
        );
        assert_eq!(
            hard_host
                .frontend
                .preferences()
                .key_config()
                .get_binding("ZoomIn")
                .unwrap()
                .primary_key,
            Some(KeyCode::F3)
        );
        assert!(
            easy_host
                .frontend
                .resources
                .shipping
                .as_ref()
                .unwrap()
                .raw
                .contains_key("easy.marker")
        );
        assert!(
            !easy_host
                .frontend
                .resources
                .shipping
                .as_ref()
                .unwrap()
                .raw
                .contains_key("hard.marker")
        );
        assert!(
            hard_host
                .frontend
                .resources
                .shipping
                .as_ref()
                .unwrap()
                .raw
                .contains_key("hard.marker")
        );

        easy.with_player_profiles_mut(|profiles| {
            profiles.get_active_mut().unwrap().minimap_x = 123.0;
        })
        .unwrap();
        let hard_x = hard
            .with_player_profiles_mut(|profiles| profiles.get_active().unwrap().minimap_x)
            .unwrap();
        assert_eq!(hard_x, 65536.0);

        easy.with_player_profiles_mut(|profiles| {
            profiles.get_active_mut().unwrap().difficulty = DifficultyLevel::Medium;
        })
        .unwrap();
        assert_eq!(easy.sim_config().difficulty, DifficultyLevel::Medium);
        assert_eq!(hard.sim_config().difficulty, DifficultyLevel::Hard);
    }

    #[test]
    fn production_host_reports_service_failure_after_readiness_validation() {
        let context = context(0, DifficultyLevel::Medium, KeyCode::F2, "ready.marker");
        let ready = ReadyApplicationContext::try_from(context.clone()).unwrap();
        let services = context.required_services().unwrap();
        services.player_profiles.lock().unwrap().active_index = None;
        let error = Host::new(ready, 800.0, 600.0).err().unwrap();
        assert!(error.contains("no active player profile"), "{error}");
    }

    #[test]
    fn replay_composition_precedes_sharing_and_diagnostics_do_not_restore_authority() {
        use std::io::Write;
        let early = Arc::new(crate::replay_service::ReplayService::default());
        let mut writer = early.recording().begin_recording();
        writer.write_all(b"early browser recording\n").unwrap();
        writer.flush().unwrap();
        let application = context(0, DifficultyLevel::Medium, KeyCode::F2, "replay.marker")
            .with_replay_service(early)
            .unwrap();
        let sibling = application.clone();
        assert_eq!(
            sibling.replay_exports().snapshot_bytes().unwrap(),
            b"early browser recording\n"
        );
        let independent = context(
            0,
            DifficultyLevel::Medium,
            KeyCode::F2,
            "independent-replay.marker",
        );
        assert!(
            independent
                .replay_exports()
                .snapshot_bytes()
                .unwrap()
                .is_empty()
        );
        assert!(
            application
                .with_replay_service(Arc::new(Default::default()))
                .unwrap_err()
                .contains("precede sharing")
        );
        let diagnostic = serde_json::to_value(&sibling).unwrap();
        assert!(diagnostic["services"].get("replay").is_none());
        let _: ApplicationContextDiagnostic = serde_json::from_value(diagnostic.clone()).unwrap();
        assert!(serde_json::from_value::<ApplicationContext>(diagnostic).is_err());
        assert_eq!(
            sibling.replay_exports().snapshot_bytes().unwrap(),
            b"early browser recording\n"
        );
    }

    #[test]
    fn ready_diagnostics_cannot_reconstruct_live_authority() {
        let ready = ReadyApplicationContext::try_from(context(
            0,
            DifficultyLevel::Medium,
            KeyCode::F2,
            "ready.marker",
        ))
        .unwrap();
        let bytes = serde_json::to_vec(&ready).unwrap();
        let diagnostic: ApplicationContextDiagnostic = serde_json::from_slice(&bytes).unwrap();
        assert!(diagnostic.services.is_some());
        assert!(serde_json::from_slice::<ReadyApplicationContext>(&bytes).is_err());
        assert!(serde_json::from_slice::<ApplicationContext>(&bytes).is_err());
        assert!(Host::new(ready, 800.0, 600.0).is_ok());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn replay_authority_cannot_change_after_transport_configuration() {
        let application = context(0, DifficultyLevel::Medium, KeyCode::F2, "transport.marker");
        // Port zero configures the disabled native endpoint without opening a
        // socket. Even that endpoint owns the original ingress capabilities.
        application.start_http_transport(0).unwrap();
        let error = application
            .with_replay_service(Arc::new(Default::default()))
            .unwrap_err();
        assert!(error.contains("precede starting"), "{error}");
    }

    #[test]
    fn bootstrap_diagnostics_preserve_options_without_granting_services() {
        let options = engine_api::GlobalOptions {
            highlander: true,
            script_enabled: false,
            ..Default::default()
        };
        let bootstrap = ApplicationContext::bootstrap(options);
        let encoded = serde_json::to_value(&bootstrap).unwrap();
        let diagnostic: ApplicationContextDiagnostic =
            serde_json::from_value(encoded.clone()).unwrap();
        assert!(diagnostic.services.is_none());
        assert_eq!(diagnostic.sim_config(), bootstrap.sim_config());
        assert_eq!(serde_json::to_value(&diagnostic).unwrap(), encoded);
        assert!(serde_json::from_value::<ApplicationContext>(encoded).is_err());

        // Retaining launcher options is an explicit construction operation,
        // not a restore of the diagnostic's recorded application authority.
        let launch = ApplicationContext::bootstrap(diagnostic.options);
        assert!(launch.options().highlander);
        assert!(ReadyApplicationContext::try_from(launch).is_err());
    }

    #[test]
    fn scratch_hosts_do_not_gain_application_service_authority() {
        for host in [Host::default(), Host::scratch(800.0, 600.0)] {
            assert!(
                host.application_context()
                    .active_profile_snapshot()
                    .is_err()
            );
            assert!(ReadyApplicationContext::try_from(host.application_context().clone()).is_err());
        }
    }

    #[test]
    fn production_host_rejects_decoded_services_without_an_active_profile() {
        let context = context(0, DifficultyLevel::Medium, KeyCode::F2, "ready.marker");
        let mut encoded = serde_json::to_value(context).unwrap();
        encoded["services"]["player_profiles"]["profiles"] = serde_json::json!([]);
        assert!(serde_json::from_value::<ReadyApplicationContext>(encoded).is_err());
    }

    #[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
    #[test]
    fn official_projection_context_preserves_the_exact_current_sim_config() {
        let sim_config = engine_api::SimConfig {
            script_enabled: false,
            highlander: true,
            amount_of_speaking: 2,
            synchronous_pathfinding: true,
            item_gameplay: robin_engine::gameplay_config::ItemGameplayConfig::classic(),
            ..engine_api::SimConfig::default()
        };
        let options = engine_api::GlobalOptions {
            script_enabled: false,
            highlander: true,
            ..Default::default()
        };
        let context =
            ApplicationContext::complete_official_projection(options, sim_config, None).unwrap();

        assert_eq!(context.sim_config(), sim_config);
        assert!(
            context
                .persist_player_profiles(&context.player_profiles_snapshot().unwrap())
                .is_err()
        );
        assert!(context.cache_clear_status().is_err());
        assert_eq!(
            serde_json::to_value(context.recording_index()).unwrap()["directory"],
            serde_json::Value::Null
        );
        assert_eq!(
            context
                .clone()
                .with_options(context.options().clone())
                .sim_config(),
            sim_config,
            "ordinary headless handoff and exporter must seal byte-identical rules"
        );
        let profiles = context.player_profiles_snapshot().unwrap();
        assert_eq!(profiles.save_directory, "official-projection-memory-only");
        assert_eq!(profiles.profiles.len(), 1);
        assert_eq!(
            profiles
                .get_active()
                .unwrap()
                .sound_config
                .amount_of_speaking,
            2
        );
        assert!(
            context
                .active_spellforge_trust_grants()
                .unwrap_err()
                .contains("disabled in the closed official projection context")
        );
        assert!(
            context
                .clear_distributed_mod_cache()
                .unwrap_err()
                .contains("disabled in the closed official projection context")
        );

        let mut mismatched = context.options().clone();
        mismatched.script_enabled = true;
        assert!(
            ApplicationContext::complete_official_projection(mismatched, sim_config, None).is_err()
        );
    }

    #[test]
    fn replacing_launcher_options_preserves_profile_speech_amount() {
        let context = context(0, DifficultyLevel::Medium, KeyCode::F2, "speech.marker");
        context
            .with_player_profiles_mut(|profiles| {
                profiles
                    .get_active_mut()
                    .unwrap()
                    .sound_config
                    .amount_of_speaking = 9;
            })
            .unwrap();

        let options = engine_api::GlobalOptions {
            highlander2: true,
            ..Default::default()
        };
        let replaced = context.with_options(options);

        assert_eq!(replaced.sim_config().amount_of_speaking, 9);
        assert!(replaced.sim_config().highlander2);
    }

    #[test]
    fn cloned_launch_options_stay_local_while_profile_updates_are_shared() {
        let original = context(
            0,
            DifficultyLevel::Easy,
            KeyCode::F2,
            "clone-options.marker",
        );
        let changed = original.clone().with_options(engine_api::GlobalOptions {
            script_enabled: false,
            highlander: true,
            highlander2: true,
            golden_eye: true,
            ignore_default_loose: true,
            bypass_fog_sprites_crash: true,
            ..Default::default()
        });
        let original_options = original.options().clone();
        let changed_options = changed.options().clone();
        let sealed = changed.sim_config();

        for (updater, difficulty, speech) in [
            (&original, DifficultyLevel::Hard, 9),
            (&changed, DifficultyLevel::Medium, 2),
        ] {
            updater
                .with_player_profiles_mut(|profiles| {
                    let active = profiles.get_active_mut().unwrap();
                    active.difficulty = difficulty;
                    active.sound_config.amount_of_speaking = speech;
                    active.gameplay_config.enable_unbinding = false;
                })
                .unwrap();
            for (context, options) in [(&original, &original_options), (&changed, &changed_options)]
            {
                let gameplay = context.active_profile_snapshot().unwrap().gameplay_config;
                assert_eq!(
                    context.sim_config(),
                    profile_sim_config(options, difficulty, speech, gameplay),
                );
                assert_eq!(
                    serde_json::to_value(context.options()).unwrap(),
                    serde_json::to_value(options).unwrap(),
                );
            }
        }
        assert_eq!(sealed.difficulty, DifficultyLevel::Easy);
        assert!(
            sealed.highlander,
            "already sealed simulation values are independent copies"
        );
        // The diagnostic wire snapshot must retain the same effective contract.
        let decoded: ApplicationContextDiagnostic =
            serde_json::from_value(serde_json::to_value(&changed).unwrap()).unwrap();
        assert_eq!(decoded.sim_config(), changed.sim_config());
    }

    #[test]
    fn startup_and_options_use_the_same_frontend_projection() {
        let context = context(0, DifficultyLevel::Hard, KeyCode::F4, "projection.marker");
        context
            .with_player_profiles_mut(|profiles| {
                let profile = profiles.get_active_mut().unwrap();
                profile.gameplay_config.control_tactical_units = false;
                profile.gameplay_config.plan_quick_actions = false;
                profile.gameplay_config.touch_camera_gestures = false;
                profile.graphic_config.native_refresh_presentation = true;
                profile.graphic_config.quick_action_cursor_pulse = false;
                profile.graphic_config.diplomacy_visuals = true;
                profile.sound_config.amount_of_speaking = 7;
            })
            .unwrap();
        let startup = Host::new(context.clone().try_into().unwrap(), 800.0, 600.0).unwrap();
        let profile = context.active_profile_snapshot().unwrap();
        let (keys, custom_keys) = context.active_key_configs().unwrap();
        let mut live = Host::scratch(800.0, 600.0);
        FrontendPreferences::new(
            KeyConfig::default(),
            KeyConfig::default(),
            robin_engine::gameplay_config::GameplayConfig {
                plan_quick_actions: true,
                ..Default::default()
            },
            &Default::default(),
        )
        .apply(&mut live.frontend);
        live.frontend.route_touch_plan_event(
            &crate::gfx_types::GameEvent::MouseDown(0, 0, 1, 1),
            true,
            |_, _| true,
        );
        assert!(live.frontend.planning().touch_latched());
        let sealed = engine_api::SimConfig {
            difficulty: DifficultyLevel::Easy,
            amount_of_speaking: 1,
            ..Default::default()
        };
        live.transport.mission_sim_config = Some(sealed);
        let effects = FrontendPreferences::new(
            keys,
            custom_keys,
            profile.gameplay_config,
            &profile.graphic_config,
        )
        .apply(&mut live.frontend);
        assert_eq!(
            effects,
            FrontendPreferenceEffects {
                cancel_planned_action: true,
                native_refresh_presentation: true,
                release_tactical_control: true,
            }
        );
        assert!(!live.frontend.planning().touch_latched());
        assert_eq!(live.transport.mission_sim_config, Some(sealed));
        assert_eq!(context.sim_config().amount_of_speaking, 7);
        for frontend in [&startup.frontend, &live.frontend] {
            assert_eq!(
                serde_json::to_value(FrontendPreferences::new(
                    frontend.preferences().key_config().clone(),
                    frontend.preferences().custom_key_config().clone(),
                    frontend.preferences().gameplay_config(),
                    &profile.graphic_config,
                ))
                .unwrap(),
                serde_json::to_value(context.host_snapshot().unwrap().preferences).unwrap(),
            );
            assert!(!frontend.preferences().control_tactical_units());
            assert!(!frontend.planning().enabled());
            assert!(!frontend.preferences().touch_camera_gestures());
            assert!(frontend.preferences().native_refresh_presentation());
            assert!(!frontend.preferences().quick_action_cursor_pulse());
            assert!(frontend.preferences().diplomacy_visuals());
        }
    }

    #[test]
    fn profile_updates_preserve_explicit_pathfinding_construction_policy() {
        let context = context(
            0,
            DifficultyLevel::Medium,
            KeyCode::F2,
            "pathfinding.marker",
        );
        context.sim_config.lock().unwrap().synchronous_pathfinding = true;
        let sibling = context.clone().with_options(engine_api::GlobalOptions {
            highlander: true,
            ..Default::default()
        });
        sibling
            .with_player_profiles_mut(|profiles| {
                profiles
                    .get_active_mut()
                    .unwrap()
                    .sound_config
                    .amount_of_speaking = 9;
            })
            .unwrap();
        for snapshot in [context.sim_config(), sibling.sim_config()] {
            assert!(snapshot.synchronous_pathfinding);
            assert_eq!(snapshot.amount_of_speaking, 9);
            assert_eq!(snapshot.difficulty, DifficultyLevel::Medium);
        }
        assert!(!context.sim_config().highlander);
        assert!(sibling.sim_config().highlander);
    }

    #[test]
    fn frontend_projection_preserves_session_planning_policy() {
        let mut frontend = HostFrontend::default();
        frontend.force_planning_off_for_session();
        let gameplay = robin_engine::gameplay_config::GameplayConfig {
            plan_quick_actions: true,
            control_tactical_units: true,
            ..Default::default()
        };
        let effects = FrontendPreferences::new(
            KeyConfig::default(),
            KeyConfig::default(),
            gameplay,
            &Default::default(),
        )
        .apply(&mut frontend);
        assert!(effects.cancel_planned_action);
        assert!(!effects.release_tactical_control);
        assert!(!frontend.planning().enabled());
    }

    #[test]
    fn context_snapshots_release_locks_before_await() {
        let context = context(0, DifficultyLevel::Medium, KeyCode::F4, "lock.marker");

        pollster::block_on(async {
            let snapshot = context.host_snapshot().unwrap();
            std::future::ready(()).await;

            let services = context.required_services().unwrap();
            assert!(services.player_profiles.try_lock().is_ok());
            assert!(services.key_configs.try_lock().is_ok());
            assert_eq!(
                snapshot
                    .preferences
                    .key_config()
                    .get_binding("ZoomIn")
                    .unwrap()
                    .primary_key,
                Some(KeyCode::F4)
            );
        });
    }

    #[test]
    fn first_launch_replacement_installs_keys_host_and_save_target_for_new_id() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().to_string_lossy().into_owned();
        let mut profiles = PlayerProfileManager::new(root_path.clone());
        let placeholder = profiles.create_profile("Robin".into(), DifficultyLevel::Medium);
        profiles.set_active(placeholder);
        profiles.default_profiles = true;
        crate::player_profile_store::PlayerProfileStore::for_directory(&profiles.save_directory)
            .save(&profiles)
            .unwrap();

        let mut keys = KeyConfigStore::new(root_path.clone());
        keys.entry_or_default(0);
        keys.save().unwrap();
        let context = ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&root_path),
            engine_api::GlobalOptions::default(),
            profiles,
            keys,
            None,
        )
        .unwrap();
        context
            .grant_spellforge_content_trust(
                spellforge_trust_key(9),
                spellforge_trust_metadata(),
                10,
            )
            .unwrap();

        let new_id = context
            .complete_first_launch_profile(
                Some(("Marian".into(), DifficultyLevel::Hard)),
                (1280, 720),
            )
            .unwrap();
        assert_eq!(new_id, 1);
        assert_eq!(context.active_profile_snapshot().unwrap().id, new_id);
        context
            .with_spellforge_trust(|trust| {
                assert!(trust.grants_for_profile(0).is_empty());
            })
            .unwrap();
        assert!(
            !SpellforgeTrustStore::load(&root_path)
                .unwrap()
                .is_trusted(0, spellforge_trust_key(9))
                .unwrap()
        );

        let (active_keys, custom_keys) = context.active_key_configs().unwrap();
        assert!(!active_keys.bindings.is_empty());
        assert!(!custom_keys.bindings.is_empty());
        context
            .with_key_configs(|store| {
                assert!(store.get(0).is_none());
                assert!(store.get(new_id).is_some());
            })
            .unwrap();

        let host = Host::new(context.clone().try_into().unwrap(), 1280.0, 720.0).unwrap();
        assert_eq!(
            host.frontend.preferences().key_config().key_type,
            active_keys.key_type
        );
        assert_eq!(
            host.frontend.preferences().custom_key_config().key_type,
            custom_keys.key_type
        );

        let mut saves = crate::savegame::SaveGameManager::open_for_context(&context).unwrap();
        let slot = saves.create("First save".into(), 7);
        let expected_save_root = root.path().join("Profile_001");
        assert_eq!(
            std::path::Path::new(saves.save_directory()),
            expected_save_root
        );
        assert!(saves.save_path(slot).starts_with(&expected_save_root));
        assert!(!std::path::Path::new(saves.save_directory()).ends_with("Profile_000"));
    }

    #[test]
    fn unavailable_trust_persistence_blocks_first_launch_profile_replacement() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().to_string_lossy().into_owned();
        let mut profiles = PlayerProfileManager::new(root_path.clone());
        let placeholder = profiles.create_profile("Robin".into(), DifficultyLevel::Medium);
        profiles.set_active(placeholder);
        profiles.default_profiles = true;

        let mut keys = KeyConfigStore::new(root_path.clone());
        keys.entry_or_default(0);
        let context = ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&root_path),
            engine_api::GlobalOptions::default(),
            profiles,
            keys,
            None,
        )
        .unwrap();
        context
            .with_spellforge_trust_mut(|store| {
                *store = SpellforgeTrustStore::unavailable(
                    root_path,
                    "corrupt first-launch trust store",
                );
            })
            .unwrap();

        let error = context
            .complete_first_launch_profile(
                Some(("Marian".into(), DifficultyLevel::Hard)),
                (1280, 720),
            )
            .unwrap_err();
        assert!(error.contains("persistence is unavailable"), "{error}");
        context
            .with_player_profiles(|profiles| {
                assert_eq!(profiles.profile_count(), 1);
                assert_eq!(profiles.get_active().unwrap().id, 0);
                assert!(profiles.default_profiles);
            })
            .unwrap();
        context
            .with_key_configs(|keys| {
                assert!(keys.get(0).is_some());
                assert!(keys.get(1).is_none());
            })
            .unwrap();
    }

    #[test]
    fn failed_profile_recovery_revocation_keeps_remote_trust_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().to_string_lossy().into_owned();
        let mut profiles = PlayerProfileManager::new(root_path.clone());
        let profile = profiles.create_profile("Robin".into(), DifficultyLevel::Medium);
        profiles.set_active(profile);
        let mut keys = KeyConfigStore::new(root_path.clone());
        keys.entry_or_default(0);
        let context = ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&root_path),
            engine_api::GlobalOptions::default(),
            profiles,
            keys,
            None,
        )
        .unwrap();
        context
            .grant_spellforge_content_trust(
                spellforge_trust_key(5),
                spellforge_trust_metadata(),
                10,
            )
            .unwrap();

        let moved = root.path().with_extension("moved");
        std::fs::rename(root.path(), &moved).unwrap();
        std::fs::write(root.path(), b"blocks trust directory recreation").unwrap();
        let error = context
            .reset_spellforge_trust_after_profile_recovery()
            .unwrap_err();
        assert!(
            error.contains("create Spellforge trust directory"),
            "{error}"
        );
        let unavailable = context.active_spellforge_trust_grants().unwrap_err();
        assert!(
            unavailable.contains("persistence is unavailable"),
            "{unavailable}"
        );

        std::fs::remove_file(root.path()).unwrap();
        std::fs::rename(moved, root.path()).unwrap();
    }
}
