//! Host state — non-sim, non-serialized per-client transient.
//!
//! Lived as `robin_engine::engine::Host` during the early Rust port,
//! moved to robin_rs once engine code stopped depending on it. Each
//! client owns one `Host`; rollback snapshots ignore it (each client
//! reconstructs its own from hardware context). Engine reaches host
//! state only through input parameters and `SideEffects` outputs.

use robin_assets::frame_holder::{FrameHolder, PublishedFrameHolder};
use robin_assets::shipping_datadir::ShippingDatadir;
use robin_engine::coordinates::{
    MapPoint, MapSize, ScreenPoint, ScreenSize, ScreenVec, WorldPoint3D,
};
use robin_engine::element::EntityId;
use robin_engine::engine as engine_api;
use robin_engine::engine::{
    DrawOrder, FadeToBlack, GroundMarkSpriteData, InputState, PendingBgBlit, SideEffects,
    SoundCommand,
};
use robin_engine::game_operation::GameCode;
use robin_engine::markers as engine_markers;
use robin_engine::player_command as engine_player_command;
use robin_engine::player_profile::{PlayerProfile, PlayerProfileManager};
use robin_engine::sprite_variant::SpriteVariant;
#[cfg(test)]
use robin_engine::tactical_control::TacticalFormation;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::bg_cache::BackgroundDecal;
#[cfg(not(target_arch = "wasm32"))]
use crate::distributed_mod_cache::DistributedModCache;
use crate::draw_manager::DrawManager;
use crate::key_config::KeyConfig;
use crate::key_config_store::{KeyConfigStore, ProfileKeyConfig};
use crate::localization::{
    LanguageChange, LanguagePack, LanguageSelection, LocalizationPreferences, LocalizationService,
    PortTextKey,
};
use crate::mouse_way::MouseWay;
use crate::pc_info_overlay::PcInfoOverlay;
use crate::sound::SoundManager;
use crate::spellforge_trust::{
    SpellforgeTrustGrant, SpellforgeTrustKey, SpellforgeTrustMetadata, SpellforgeTrustStore,
};

const PANNEL_HEIGHT: f32 = engine_api::PANNEL_HEIGHT;
const DISPLAY_INFO_SAMPLES: usize = 16;

#[derive(Debug, Clone, Copy, Default)]
pub struct QueueStripAnimation {
    pub previous_count: usize,
    pub fall_offset: i32,
}

/// Mutable application services shared by clones of one
/// [`ApplicationContext`]. Separate contexts allocate separate service sets,
/// which makes tests, headless sessions, and future multi-instance hosts
/// independent instead of routing through process-wide singletons.
#[derive(Debug, Serialize, Deserialize)]
struct ApplicationServices {
    #[cfg(all(target_arch = "wasm32", feature = "audio"))]
    #[serde(skip)]
    browser_audio: std::cell::RefCell<Option<crate::web_audio_backend::BrowserAudioSession>>,
    #[serde(skip)]
    asset_cache: Option<crate::process_asset_cache::ApplicationAssetCache>,
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
    #[serde(skip, default = "unavailable_distributed_mod_cache")]
    distributed_mod_cache: Mutex<Result<DistributedModCache, String>>,
    localization: Mutex<LocalizationService>,
    shipping: Option<Arc<ShippingDatadir>>,
    /// Process/application-lifetime owner for queued verification. This is
    /// host-only asynchronous state; the durable stores, not a serialized
    /// `ApplicationContext`, are its recovery boundary.
    #[serde(skip)]
    leaderboard_receipts: Mutex<crate::leaderboard_receipt_watcher::ApplicationReceiptWatcher>,
}

#[cfg(all(target_arch = "wasm32", feature = "audio"))]
impl Drop for ApplicationServices {
    fn drop(&mut self) {
        if let Some(audio) = self.browser_audio.get_mut().as_ref() {
            audio.retire();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn unavailable_distributed_mod_cache() -> Mutex<Result<DistributedModCache, String>> {
    Mutex::new(Err(
        "distributed-mod cache is unavailable in a deserialized host context".to_owned(),
    ))
}

/// Explicit application-owned configuration and persistence context.
///
/// `CliArgs` initially carries a bootstrap context containing only parsed
/// options. `rust_init` supplies the required profile/key/shipping services
/// before an async game loop begins. Service accessors take snapshots while
/// holding a lock and return owned data, so no lock guard can cross an
/// `.await`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationContext {
    options: engine_api::GlobalOptions,
    sim_config: Arc<Mutex<engine_api::SimConfig>>,
    services: Option<Arc<ApplicationServices>>,
}

/// Initialization proof required by application run loops. The transparent
/// wire representation stays identical to ApplicationContext, while decoding
/// refuses a launcher-only context without the required services.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "ApplicationContext", into = "ApplicationContext")]
pub struct ReadyApplicationContext(ApplicationContext);

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
    pub fn with_options(self, options: engine_api::GlobalOptions) -> Self {
        Self(self.0.with_options(options))
    }
}

/// Owned host-facing snapshot copied out of an [`ApplicationContext`].
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HostContextSnapshot {
    shipping: Option<Arc<ShippingDatadir>>,
    key_config: KeyConfig,
    custom_key_config: KeyConfig,
    #[serde(alias = "control_allied_soldiers")]
    control_tactical_units: bool,
    plan_quick_actions: bool,
    touch_camera_gestures: bool,
    native_refresh_presentation: bool,
    quick_action_cursor_pulse: bool,
    diplomacy_visuals: bool,
    gameplay_config: robin_engine::gameplay_config::GameplayConfig,
}

impl ApplicationContext {
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
        active.gameplay_config.item_gameplay = sim_config.item_gameplay;
        active.gameplay_config.noise_distraction_feedback = sim_config.noise_distraction_feedback;
        active.gameplay_config.sherwood_trading = sim_config.sherwood_trading;

        let mut key_configs = KeyConfigStore::new(MEMORY_ONLY_SAVE_DIRECTORY.to_owned());
        key_configs.entry_or_default(active.id);
        Ok(Self {
            options,
            sim_config: Arc::new(Mutex::new(sim_config)),
            services: Some(Arc::new(ApplicationServices {
                asset_cache: Some(Default::default()),
                cache_maintenance: Default::default(),
                #[cfg(all(target_arch = "wasm32", feature = "audio"))]
                browser_audio: Default::default(),
                preparation_files,
                profile_store: crate::player_profile_store::PlayerProfileStore::unavailable(
                    "profiles disabled in closed official projection",
                ),
                player_profiles: Mutex::new(player_profiles),
                key_configs: Mutex::new(key_configs),
                spellforge_trust: Mutex::new(SpellforgeTrustStore::unavailable(
                    MEMORY_ONLY_SAVE_DIRECTORY.to_owned(),
                    "Spellforge trust is disabled in the closed official projection context",
                )),
                #[cfg(not(target_arch = "wasm32"))]
                distributed_mod_cache: Mutex::new(Err(
                    "distributed-mod cache is disabled in the closed official projection context"
                        .to_owned(),
                )),
                localization: Mutex::new(LocalizationService::disabled()),
                shipping,
                leaderboard_receipts: Mutex::new(Default::default()),
            })),
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

        let trust_directory = key_configs.save_directory.clone();
        let spellforge_trust = SpellforgeTrustStore::load(&trust_directory).unwrap_or_else(|error| {
            tracing::error!(
                "Spellforge trust persistence is unavailable and remote code admission will fail closed: {error}"
            );
            SpellforgeTrustStore::unavailable(trust_directory.clone(), error)
        });
        #[cfg(not(target_arch = "wasm32"))]
        let distributed_mod_cache = DistributedModCache::open(&trust_directory).map_err(|error| {
            tracing::error!(
                "distributed-mod cache is unavailable and host-distributed content admission will fail closed: {error}"
            );
            error
        });

        let sim_config =
            profile_sim_config(&options, difficulty, amount_of_speaking, gameplay_config);
        Ok(Self {
            sim_config: Arc::new(Mutex::new(sim_config)),
            options,
            services: Some(Arc::new(ApplicationServices {
                asset_cache: Some(Default::default()),
                cache_maintenance: crate::cache_maintenance::CacheMaintenance::new(),
                #[cfg(all(target_arch = "wasm32", feature = "audio"))]
                browser_audio: Default::default(),
                preparation_files,
                player_profiles: Mutex::new(player_profiles),
                key_configs: Mutex::new(key_configs),
                profile_store,
                spellforge_trust: Mutex::new(spellforge_trust),
                #[cfg(not(target_arch = "wasm32"))]
                distributed_mod_cache: Mutex::new(distributed_mod_cache),
                localization: Mutex::new(localization),
                shipping,
                leaderboard_receipts: Mutex::new(Default::default()),
            })),
        })
    }

    pub fn with_options(mut self, options: engine_api::GlobalOptions) -> Self {
        let mut sim_config = self.sim_config();
        let launcher = engine_api::SimConfig::from_options(&options, sim_config.difficulty);
        sim_config.script_enabled = launcher.script_enabled;
        sim_config.highlander = launcher.highlander;
        sim_config.highlander2 = launcher.highlander2;
        sim_config.golden_eye = launcher.golden_eye;
        sim_config.ignore_default_loose = launcher.ignore_default_loose;
        sim_config.bypass_fog_sprites_crash = launcher.bypass_fog_sprites_crash;
        *self
            .sim_config
            .lock()
            .expect("ApplicationContext sim-config lock poisoned") = sim_config;
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
        self.required_services()?
            .asset_cache
            .as_ref()
            .ok_or_else(|| {
                "application asset cache is unavailable in a deserialized host context".to_owned()
            })
    }

    pub fn options(&self) -> &engine_api::GlobalOptions {
        &self.options
    }

    pub fn sim_config(&self) -> engine_api::SimConfig {
        *self
            .sim_config
            .lock()
            .expect("ApplicationContext sim-config lock poisoned")
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
        // Validate runtime ownership before publishing any locale change. Decoded
        // services also lack localization file authority, but must never silently
        // skip cache invalidation if that service's recovery rules change later.
        let cache = services.asset_cache.as_ref().ok_or_else(|| {
            "application asset cache is unavailable in a deserialized host context".to_owned()
        })?;
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
        let (result, difficulty, amount_of_speaking, gameplay_config) = {
            let mut profiles = self
                .required_services()?
                .player_profiles
                .lock()
                .map_err(|_| "ApplicationContext player-profile lock poisoned".to_string())?;
            let result = update(&mut profiles);
            let active = profiles.get_active().ok_or_else(|| {
                "ApplicationContext profile mutation must leave an active profile".to_string()
            })?;
            (
                result,
                active.difficulty,
                active.sound_config.amount_of_speaking,
                active.gameplay_config,
            )
        };
        self.refresh_profile_derived_state(difficulty, amount_of_speaking, gameplay_config)?;
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
        let (profile_id, difficulty, amount_of_speaking, gameplay_config) = {
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
            (profile_id, difficulty, amount_of_speaking, gameplay_config)
        };

        self.refresh_profile_derived_state(difficulty, amount_of_speaking, gameplay_config)?;
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

    fn host_snapshot(&self) -> Result<HostContextSnapshot, String> {
        let services = self.required_services()?;
        let (key_config, custom_key_config) = self.active_key_configs()?;
        let active_profile = self.active_profile_snapshot()?;
        Ok(HostContextSnapshot {
            shipping: services.shipping.clone(),
            key_config,
            custom_key_config,
            control_tactical_units: active_profile.gameplay_config.control_tactical_units,
            plan_quick_actions: active_profile.gameplay_config.plan_quick_actions,
            touch_camera_gestures: active_profile.gameplay_config.touch_camera_gestures,
            native_refresh_presentation: active_profile.graphic_config.native_refresh_presentation,
            quick_action_cursor_pulse: active_profile.graphic_config.quick_action_cursor_pulse,
            diplomacy_visuals: active_profile.graphic_config.diplomacy_visuals,
            gameplay_config: active_profile.gameplay_config,
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

    pub(crate) fn take_leaderboard_receipt_notice(
        &self,
    ) -> Result<Option<crate::leaderboard_receipt_watcher::ReceiptWatcherNotice>, String> {
        Ok(self
            .required_services()?
            .leaderboard_receipts
            .lock()
            .map_err(|_| "ApplicationContext leaderboard-receipt lock poisoned".to_owned())?
            .take_notice())
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
        let sim_config = profile_sim_config(
            &self.options,
            difficulty,
            amount_of_speaking,
            gameplay_config,
        );
        *self
            .sim_config
            .lock()
            .map_err(|_| "ApplicationContext sim-config lock poisoned".to_string())? = sim_config;

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

/// Deferred PrintScreen request, including the modifier branch that was
/// active when the key edge fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PrintScreenRequest {
    Plain,
    Median3x3,
    WideSnapshot,
}

/// Host-only titbit-like previews emitted by cursor/hover code.
///
/// These are intentionally not inserted into `Engine::titbit_manager`:
/// they are local UI feedback and must not affect rollback state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HostTitbitPreview {
    JumpHelperGhost {
        position: WorldPoint3D,
        layer: u16,
        sector_dir: u16,
        display_order: f32,
    },
}

impl HostTitbitPreview {
    #[inline]
    pub fn display_order(self) -> f32 {
        match self {
            Self::JumpHelperGhost { display_order, .. } => display_order,
        }
    }
}

/// Host-local viewport state. This is deliberately outside
/// `robin_engine`: screen size, mouse anchoring, render culling, and
/// local scroll/zoom are presentation concerns and may differ on every
/// multiplayer peer.
#[derive(Debug, Clone)]
pub struct ViewportState {
    pub view_position: MapPoint,
    pub old_view_position: MapPoint,
    pub zoom_factor: f32,
    pub old_zoom_factor: f32,
    pub screen_size: ScreenSize,
    pub level_size: MapSize,
    touch_motion: TouchCameraMotion,
}

/// Host-only touch-camera state. Velocities are expressed in screen pixels
/// per second so momentum feels consistent at every zoom level.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct TouchCameraMotion {
    transform_active: bool,
    velocity_x: f32,
    velocity_y: f32,
    last_inertia_ms: u32,
}

impl ViewportState {
    pub fn new(screen_width: f32, screen_height: f32) -> Self {
        Self {
            view_position: MapPoint::ZERO,
            old_view_position: MapPoint::ZERO,
            zoom_factor: 1.0,
            old_zoom_factor: 1.0,
            screen_size: ScreenSize::new(screen_width, screen_height),
            level_size: MapSize::ZERO,
            touch_motion: TouchCameraMotion::default(),
        }
    }

    pub fn set_screen_size(&mut self, width: f32, height: f32) {
        self.screen_size = ScreenSize::new(width, height);
        self.clip_view();
    }

    pub fn set_level_size(&mut self, width: f32, height: f32) {
        self.level_size = MapSize::new(width, height);
        self.clip_view();
    }

    pub fn center_on_point(&mut self, point: MapPoint) {
        self.view_position = MapPoint::new(
            (point.x - self.screen_size.x / (2.0 * self.zoom_factor)).floor(),
            (point.y - self.screen_size.y / (2.0 * self.zoom_factor)).floor(),
        );
        self.clip_view();
    }

    /// Mirror the shared script/director camera while a cutscene owns input.
    ///
    /// The director camera is deterministic shared state, so the engine
    /// frames its focal point inside a fixed virtual view (`view_size`)
    /// rather than any peer's canvas. Shift the top-left by half the size
    /// difference so that same focal point lands in the centre of this
    /// host's (possibly widescreen) view. The shift is exactly zero when
    /// both sizes agree, which keeps the classic 1024x768 path bit-identical.
    pub fn adopt_director_camera(
        &mut self,
        view_position: MapPoint,
        view_size: ScreenSize,
        zoom_factor: f32,
    ) {
        assert!(
            zoom_factor.is_finite() && zoom_factor > 0.0,
            "director camera supplied invalid zoom factor {zoom_factor}"
        );
        self.old_view_position = self.view_position;
        self.old_zoom_factor = self.zoom_factor;
        self.zoom_factor = zoom_factor;
        let shift_x = (view_size.x - self.screen_size.x) / (2.0 * zoom_factor);
        let shift_y = (view_size.y - self.screen_size.y) / (2.0 * zoom_factor);
        self.view_position = MapPoint::new(view_position.x + shift_x, view_position.y + shift_y);
        self.clip_view();
    }

    pub fn sound_listen_point(&self) -> MapPoint {
        MapPoint::new(
            self.view_position.x + self.screen_size.x * 0.5 / self.zoom_factor,
            self.view_position.y + (self.screen_size.y - PANNEL_HEIGHT) * 0.5 / self.zoom_factor,
        )
    }

    pub fn scroll_by(&mut self, delta: ScreenVec) {
        self.old_view_position = self.view_position;
        self.view_position.x += delta.x / self.zoom_factor;
        self.view_position.y += delta.y / self.zoom_factor;
        self.clip_view();
    }

    pub fn zoom_by(&mut self, factor: f32, mouse_screen: Option<ScreenPoint>) {
        let next = (self.zoom_factor * factor).clamp(0.5, 2.0);
        if (next - self.zoom_factor).abs() < f32::EPSILON {
            return;
        }
        let anchor = mouse_screen.unwrap_or_else(|| {
            ScreenPoint::new(self.screen_size.x * 0.5, self.screen_size.y * 0.5)
        });
        let before = self.screen_to_map_unchecked(anchor);
        self.old_zoom_factor = self.zoom_factor;
        self.zoom_factor = next;
        self.view_position = MapPoint::new(
            before.x - anchor.x / self.zoom_factor,
            before.y - anchor.y / self.zoom_factor,
        );
        self.clip_view();
    }

    /// Begin a two-finger camera transform after the gameplay layer has
    /// decided whether both fingers originated in the world viewport.
    pub fn begin_touch_transform(&mut self, accepted: bool) {
        self.touch_motion = TouchCameraMotion {
            transform_active: accepted,
            ..TouchCameraMotion::default()
        };
    }

    /// Atomically apply centroid translation and pinch scaling. The map point
    /// beneath the previous centroid remains beneath the current centroid,
    /// avoiding the order-dependent wobble caused by separate pan/zoom calls.
    pub fn apply_touch_transform(
        &mut self,
        centroid: ScreenPoint,
        pan: ScreenVec,
        scale: f32,
    ) -> bool {
        if !self.touch_motion.transform_active {
            return false;
        }
        if !scale.is_finite()
            || scale <= 0.0
            || !centroid.x.is_finite()
            || !centroid.y.is_finite()
            || !pan.x.is_finite()
            || !pan.y.is_finite()
        {
            tracing::warn!(?centroid, ?pan, scale, "ignored non-finite touch transform");
            return false;
        }

        let previous_centroid = ScreenPoint::new(centroid.x - pan.x, centroid.y - pan.y);
        let anchor = self.screen_to_map_unchecked(previous_centroid);
        self.old_view_position = self.view_position;
        self.old_zoom_factor = self.zoom_factor;
        self.zoom_factor = (self.zoom_factor * scale).clamp(0.5, 2.0);
        self.view_position = MapPoint::new(
            anchor.x - centroid.x / self.zoom_factor,
            anchor.y - centroid.y / self.zoom_factor,
        );
        self.clip_view();
        true
    }

    pub fn end_touch_transform(&mut self, velocity: ScreenVec, cancelled: bool, now_ms: u32) {
        const MAX_INERTIA_SPEED: f32 = 5_000.0;

        if !self.touch_motion.transform_active {
            self.touch_motion = TouchCameraMotion::default();
            return;
        }
        self.touch_motion.transform_active = false;
        if cancelled || !velocity.x.is_finite() || !velocity.y.is_finite() {
            self.touch_motion.velocity_x = 0.0;
            self.touch_motion.velocity_y = 0.0;
        } else {
            self.touch_motion.velocity_x = velocity.x;
            self.touch_motion.velocity_y = velocity.y;
            let speed = velocity.x.hypot(velocity.y);
            if speed > MAX_INERTIA_SPEED {
                let scale = MAX_INERTIA_SPEED / speed;
                self.touch_motion.velocity_x *= scale;
                self.touch_motion.velocity_y *= scale;
            }
        }
        self.touch_motion.last_inertia_ms = now_ms;
    }

    pub fn cancel_touch_motion(&mut self) {
        self.touch_motion = TouchCameraMotion::default();
    }

    /// Advance hard-clamped pan inertia using wall time. Returns whether the
    /// camera moved, allowing future display-rate render loops to skip static
    /// recomposition without coupling momentum to the 25 Hz simulation.
    pub fn advance_touch_inertia(&mut self, now_ms: u32) -> bool {
        const DECAY_PER_SECOND: f32 = 6.5;
        const STOP_SPEED: f32 = 18.0;
        const MAX_STEP_SECONDS: f32 = 0.050;

        if self.touch_motion.transform_active {
            self.touch_motion.last_inertia_ms = now_ms;
            return false;
        }
        let speed = self
            .touch_motion
            .velocity_x
            .hypot(self.touch_motion.velocity_y);
        if speed < STOP_SPEED {
            self.touch_motion.velocity_x = 0.0;
            self.touch_motion.velocity_y = 0.0;
            self.touch_motion.last_inertia_ms = now_ms;
            return false;
        }
        let elapsed = now_ms.wrapping_sub(self.touch_motion.last_inertia_ms) as f32 / 1000.0;
        let dt = elapsed.min(MAX_STEP_SECONDS);
        self.touch_motion.last_inertia_ms = now_ms;
        if dt <= 0.0 {
            return false;
        }

        let before = self.view_position;
        self.scroll_by(ScreenVec::new(
            -self.touch_motion.velocity_x * dt,
            -self.touch_motion.velocity_y * dt,
        ));
        if (self.view_position.x - before.x).abs() < f32::EPSILON {
            self.touch_motion.velocity_x = 0.0;
        }
        if (self.view_position.y - before.y).abs() < f32::EPSILON {
            self.touch_motion.velocity_y = 0.0;
        }
        let decay = (-DECAY_PER_SECOND * dt).exp();
        self.touch_motion.velocity_x *= decay;
        self.touch_motion.velocity_y *= decay;
        self.view_position != before
    }

    pub fn screen_to_map(&self, screen_pt: ScreenPoint) -> Option<MapPoint> {
        let map_pt = self.screen_to_map_unchecked(screen_pt);
        if map_pt.x > 0.0
            && map_pt.y > 0.0
            && map_pt.x <= self.level_size.x
            && map_pt.y <= self.level_size.y
        {
            Some(map_pt)
        } else {
            None
        }
    }

    pub fn screen_to_map_unchecked(&self, screen_pt: ScreenPoint) -> MapPoint {
        MapPoint::new(
            self.view_position.x + screen_pt.x / self.zoom_factor,
            self.view_position.y + screen_pt.y / self.zoom_factor,
        )
    }

    pub fn map_to_screen(&self, map_pt: MapPoint) -> Option<ScreenPoint> {
        let screen_pt = self.map_to_screen_unclamped(map_pt);
        if screen_pt.x >= 0.0
            && screen_pt.y >= 0.0
            && screen_pt.x <= self.screen_size.x
            && screen_pt.y <= self.screen_size.y
        {
            Some(screen_pt)
        } else {
            None
        }
    }

    pub fn map_to_screen_unclamped(&self, map_pt: MapPoint) -> ScreenPoint {
        ScreenPoint::new(
            (map_pt.x - self.view_position.x) * self.zoom_factor,
            (map_pt.y - self.view_position.y) * self.zoom_factor,
        )
    }

    fn clip_view(&mut self) {
        if self.view_position.x < 0.0 {
            self.view_position.x = 0.0;
        }
        if self.view_position.y < 0.0 {
            self.view_position.y = 0.0;
        }
        if self.level_size.x > 0.0 {
            let max_x = (self.level_size.x - self.screen_size.x / self.zoom_factor).max(0.0);
            self.view_position.x = self.view_position.x.min(max_x);
        }
        if self.level_size.y > 0.0 {
            let max_y = (self.level_size.y
                - (self.screen_size.y - PANNEL_HEIGHT) / self.zoom_factor)
                .max(0.0);
            self.view_position.y = self.view_position.y.min(max_y);
        }
    }
}

impl Default for ViewportState {
    fn default() -> Self {
        Self::new(1024.0, 768.0)
    }
}

/// Local rendering and interaction state. Kept behind the small [`Host`]
/// facade so it can be borrowed independently from transport, audio, and
/// ordered post-tick work.
#[derive(Default)]
pub struct HostFrontend {
    // ── Rendering / GPU surfaces ─────────────────────────────────
    pub(crate) mission_surfaces: crate::mission_render_resources::MissionRenderResources,
    pub viewport: ViewportState,
    pub engine_display: engine_api::HostDisplayState,

    // ── Input ────────────────────────────────────────────────────
    pub input: InputState,

    /// Paired pointer-event ownership, retired together at interaction resets.
    pub pointer_capture: crate::frontend_input::FrontendPointerCapture,

    /// Active profile's opt-in tactical-unit control setting. Host-local
    /// because resolved player commands, rather than UI preferences, cross
    /// replay and multiplayer boundaries.
    pub control_tactical_units: bool,

    /// Planning preference, sticky touch mode and non-overridable session policy.
    pub planning: crate::frontend_input::FrontendPlanning,

    /// Per-portrait cosmetic easing for the independent automatic queue strip.
    pub queue_strip_animations: HashMap<EntityId, QueueStripAnimation>,

    /// Active profile's local relationship colour/legend preference.
    pub diplomacy_visuals: bool,

    /// Host-local presentation settings copied from the active profile.
    /// Deterministic settings are separately mirrored into `SimConfig`.
    pub gameplay_config: robin_engine::gameplay_config::GameplayConfig,

    /// Host-local targeting prompt armed by the tactical patrol portrait button.
    pub tactical_targeting: crate::frontend_targeting::TacticalTargeting,

    /// Active profile's touch-camera gesture setting. Host-local because
    /// camera pan/zoom/inertia never enters deterministic simulation state.
    pub touch_camera_gestures: bool,

    /// Opt-in display-rate re-presentation. Host-local and intentionally
    /// absent from deterministic save/replay state.
    pub native_refresh_presentation: bool,

    /// Show the original cursor-shadow recording pulse while the manual
    /// quick-action recorder is active. Cached from the profile so high-rate
    /// presentation samples never clone or lock the full profile history.
    pub quick_action_cursor_pulse: bool,

    /// Last positive duration of a display-rate presentation sample, in
    /// microseconds. The fixed-step presentation scheduler uses this host-only
    /// observation to avoid beginning a vsync wait that would cross the
    /// simulation deadline. Zero means no blocking sample has been observed.
    pub native_refresh_present_cost_us: u64,

    /// Back-to-front entity draw order.  Host-cached derived state —
    /// recomputed from [`Engine::compute_display_order`] once per frame
    /// after the tick, before the input-dispatch and render passes.
    /// Consumed by the render loop (iteration), input hit-test
    /// (`find_focusable_entity`), and titbit Z flush (depth lookup).
    /// Not sim state: never serialized, never hashed.
    pub draw_order: DrawOrder,

    /// Ping-pong animation phase for the PC selection ring.  Advanced
    /// once per frame inside `Game::run_engine_tick`, gated on the same
    /// `should_run_hourglass` check as the sim tick (so pause / console
    /// freeze the ring).  Only `SelectionMarkRenderer` reads it —
    /// purely cosmetic, lives host-side.
    pub selection_mark: engine_markers::SelectionMark,

    /// Entity whose vision cone is currently displayed as an overlay.
    /// Set when the player alt-hovers an NPC (or an ally via a cheat).
    ///
    /// UI-mode state: read by the render-phase vision-cone overlay,
    /// the alt-key UI handler, and the console cheats that target
    /// "the NPC you're currently looking at" (Honolulu, Morpheus,
    /// Hades, LastManStanding).  Not sim state: nothing inside the
    /// tick reads it, so it's excluded from the rollback hash by
    /// virtue of living on Host.
    pub selected_view_element: Option<EntityId>,

    // ── Trajectory preview (transient) ───────────────────────────
    pub trajectory_preview: crate::frontend_preview::FrontendTrajectoryPreview,
    /// Host-only explanation rendered at the current item target.
    pub item_effect_preview: Option<ItemEffectPreview>,
    /// Host-local titbit-like hover preview.  Currently only the
    /// helper-needed jump ghost from the original mouse-hover path.
    pub host_titbit_preview: Option<HostTitbitPreview>,

    // ── Assets that live only on the host side ───────────────────
    /// Decoded sprite frame bank. Host-only because `FrameHolder`
    /// lives in `robin_assets`, which depends on `robin_engine` — so
    /// engine's `LevelAssets` can't carry it. Shared via `Arc` so
    /// `Engine::clone` stays cheap.
    pub frame_holder: Arc<FrameHolder>,

    /// Engine-side opacity view of [`Self::frame_holder`]. Installed only after
    /// variant generation and the initial Arno-law bind are complete. Runtime
    /// ambiance rebinds publish a new immutable generation through this shared
    /// handle so cloned `LevelAssets` never retain a detached COW dictionary.
    frame_holder_opacity: Option<Arc<PublishedFrameHolder>>,

    /// Shipping-datadir handle. Host-only (asset-layer type). Holds the
    /// path/resource layout for the currently-loaded shipping build so
    /// the resource manager can resolve relative lookups.
    pub shipping: Option<Arc<ShippingDatadir>>,

    /// Active key bindings for the current player profile. Host-only because
    /// physical `winit` key codes and local input policy do not belong in the
    /// deterministic, platform-neutral engine `PlayerProfile`.
    pub key_config: KeyConfig,

    /// User's custom key bindings (the "User Defined" slot in the
    /// shortcuts menu). The active set is whatever the user picked
    /// last (preset or custom), while this slot preserves their
    /// personal bindings so the User Defined button can restore them.
    pub custom_key_config: KeyConfig,

    /// Physical key bound to the `DisplayMap` shortcut.  The game loop
    /// reads this on each frame and emits a minimap-toggle command on
    /// key release.  `None` means no accelerator bound.  Lives host-side
    /// — the engine has no reason to know which key the UI is bound to.
    pub minimap_fast_key: Option<winit::keyboard::KeyCode>,

    // ── Host-side managers ───────────────────────────────────────
    /// Immediate-mode draw helper (line segments, ellipses, gauges).
    pub draw_manager: DrawManager,
    /// PC info hover popup (HP, equipment). Populated from sim's
    /// `SideEffects.overlay`.
    pub pc_info_overlay: PcInfoOverlay,
    /// Mouse gesture / way-point tracker for "draw-path-to-target"
    /// movement. Pure host UI state.
    pub mouse_way: MouseWay,

    /// Last released sword gesture for the optional, host-only coach overlay.
    pub gesture_coach_feedback: Option<crate::mouse_way::GestureCoachFeedback>,

    // ── Pixel-level fade (script opcode `FADE_TO_BLACK`) ─────────
    /// Active fade-to-black ramp driven by the `FADE_TO_BLACK` script
    /// opcode.  When set, the renderer draws a black overlay with a
    /// per-frame alpha ramp — alpha climbs from 0→255 over `speed`
    /// frames (fade out), then falls 255→0 over the next `speed`
    /// frames (fade back in).
    pub fade_to_black: Option<FadeToBlack>,

    /// Last tick's `SideEffects.skip_render` decision. Read by the
    /// per-frame render loop in `game_session` to short-circuit the
    /// GPU pass when fast-forward mode wants to skip.
    pub skip_render: bool,

    /// Set when the PrintScreen keybind fires.  Drained in the render
    /// loop after `render_frame` (before `present()`) which reads back
    /// the composited frame and writes it to disk as `screen%03u.png`
    /// in the save directory. Ctrl requests a wide snapshot; Shift
    /// applies the historical 3x3 median filter to the captured frame.
    pub pending_print_screen: Option<PrintScreenRequest>,

    /// Debug-info overlay toggle. Toggled by the bound `RequestInfo`
    /// / `DisplayInfo` key (typically `Home`); read by the per-frame
    /// debug-overlay renderer.  Not serialized — debug state, not sim
    /// state.
    pub info_displayed: bool,
    /// Rolling frame-duration samples used by the DisplayInfo overlay.
    pub display_info_frame_samples: [u32; DISPLAY_INFO_SAMPLES],
    pub display_info_sample_cursor: usize,
    pub display_info_last_tick_ms: u32,
    pub display_info_max_pending_sounds: usize,

    /// Slow-motion pacing toggle. Toggled by `MSG_SLOW_MOTION` (the
    /// bound SlowMotion key — Pause by default).  Consumed by the
    /// frame pacing block at the bottom of `run_mission`: when set
    /// (and neither console nor engine fast-forward are active), the
    /// 40 ms frame target is multiplied by 10.
    pub slow_motion: bool,

    /// One-frame "a UI widget stole input focus" latch.  Set by
    /// `MSG_UI_HAS_FOCUS` during the frame's message dispatch and
    /// cleared every frame during input-state reset. The sole consumer
    /// (`RHDISPLAY_INITZOOM`) is itself unported, so this field
    /// currently only tracks the flag for future consumers.  Not sim
    /// state — purely transient per-frame input gating.
    pub ui_focus: bool,

    /// Deferred console-overlay output lines produced by host-side work
    /// that can't reach the overlay directly. Drained by the overlay
    /// at the start of each frame via
    /// [`crate::console_overlay::ConsoleOverlay::drain_pending_host_output`].
    pub pending_console_output: Vec<String>,

    // ── Persistent background decals ─
    /// Per-FX-entity persistent background decals replacing the legacy
    /// map-patch bake/restore surface pipeline. A queued map-patch insertion
    /// inserts or replaces the entity's decal; a queued restore removes it.
    pub background_decals: HashMap<EntityId, BackgroundDecal>,
    /// Stable draw order for [`Self::background_decals`], preserving the
    /// order in which patch effects became permanent.
    pub background_decal_order: Vec<EntityId>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemEffectPreview {
    pub center: MapPoint,
    pub radius: Option<u16>,
    pub localization_key: &'static str,
    pub fallback_text: &'static str,
    pub blocked: bool,
}

/// Why a host interaction sequence ended. Camera pose, resource handles and
/// profile preferences survive all resets; snapshot restoration additionally
/// invalidates every entity-bound cache and sticky action-planning selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractionReset {
    ModalClosed,
    EngineRequested,
    SnapshotRestored,
}

impl HostFrontend {
    pub fn reset_interaction(&mut self, reason: InteractionReset) {
        self.reset_pointer_sequence();
        self.reset_targeting_preview();
        if reason == InteractionReset::SnapshotRestored {
            self.input = InputState::default();
            self.planning.cancel_touch();
            self.queue_strip_animations.clear();
            self.selected_view_element = None;
            self.selection_mark = engine_markers::SelectionMark::default();
        }
    }

    fn reset_pointer_sequence(&mut self) {
        self.input.reset_pointer_sequence();
        self.pointer_capture.cancel_sequence();
        self.input.portrait_action_countdown = 0;
        self.input.portrait_action_pc = None;
        self.mouse_way.clear();
        self.viewport.cancel_touch_motion();
        self.ui_focus = false;
    }

    fn reset_targeting_preview(&mut self) {
        self.tactical_targeting.cancel();
        self.trajectory_preview.reset_after_restore();
        self.item_effect_preview = None;
        self.host_titbit_preview = None;
        self.gesture_coach_feedback = None;
    }
}

#[derive(Default)]
pub struct HostTransport {
    pub local_seat: engine_player_command::PlayerId,
    pub net: Option<crate::multiplayer::NetChannels>,
    pub mission_seed: Option<u64>,
    pub mission_sim_config: Option<engine_api::SimConfig>,
    pub speech_timing_locale: Option<String>,
    pub mission_id: Option<String>,
    pub reconnecting: bool,
    /// Verified full-mod bytes, VFS overlays, and cache lease for a
    /// host-distributed mission. Field order makes the network runtime stop
    /// before this mount is dropped with the enclosing transport.
    pub distributed_mod: Option<crate::distributed_mod_admission::AdmittedDistributedMod>,
    pub snapshot_transition: Option<PendingSnapshotTransition>,
    /// Delayed Sherwood command boundary belongs to this transport lifetime.
    pub(crate) pending_campaign_exit: Option<crate::main_entry::PendingMultiplayerCampaignExit>,
}

pub struct PendingSnapshotTransition {
    id: robin_engine::multiplayer::SnapshotTransitionId,
    payload: PendingSnapshotTransitionPayload,
    committed: bool,
}

impl PendingSnapshotTransition {
    pub(crate) fn new(
        id: robin_engine::multiplayer::SnapshotTransitionId,
        payload: PendingSnapshotTransitionPayload,
    ) -> Self {
        Self {
            id,
            payload,
            committed: false,
        }
    }
    /// Called by the authenticated transport event drain; the prepared payload
    /// and instance remain private and cannot be swapped after this admission.
    pub(crate) fn commit_authenticated(
        &mut self,
        id: robin_engine::multiplayer::SnapshotTransitionId,
    ) -> Result<(), String> {
        if self.id != id {
            return Err("snapshot transition commit does not match prepared payload".into());
        }
        if self.committed {
            return Err("snapshot transition was already committed".into());
        }
        self.committed = true;
        Ok(())
    }
}

pub enum PendingSnapshotTransitionPayload {
    Save {
        load: SnapshotSave,
    },
    CampaignExit {
        exit_code: robin_engine::game_operation::GameCode,
        /// Clients retain the exact decoded host engine so their campaign is
        /// identical before all participants enter the next mission. The
        /// host already owns that engine and therefore stores `None`.
        engine: Option<Box<robin_engine::engine::Engine>>,
    },
}

pub enum SnapshotSave {
    Local(crate::main_entry::PreparedLoad),
    Remote(Box<crate::save_file::GameSaveFile>),
}

/// Only the transport's committed take can create this process-local token.
#[derive(serde::Serialize)]
pub(crate) struct CommittedSnapshotTransition(#[serde(skip)] PendingSnapshotTransition);

impl<'de> serde::Deserialize<'de> for CommittedSnapshotTransition {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "committed transition cannot be deserialized",
        ))
    }
}

impl CommittedSnapshotTransition {
    pub(crate) fn id(&self) -> robin_engine::multiplayer::SnapshotTransitionId {
        self.0.id
    }
    pub(crate) fn is_save(&self) -> bool {
        matches!(
            self.0.payload,
            PendingSnapshotTransitionPayload::Save { .. }
        )
    }
    pub(crate) fn into_payload(self) -> PendingSnapshotTransitionPayload {
        self.0.payload
    }
}

impl HostTransport {
    pub fn authoritative_transition_actions_enabled(&self) -> bool {
        !self.reconnecting
            && self.snapshot_transition.is_none()
            && (self.net.is_none()
                || self.local_seat == robin_engine::player_command::PlayerId::HOST)
    }

    pub(crate) fn take_committed_snapshot_transition(
        &mut self,
    ) -> Option<CommittedSnapshotTransition> {
        if self
            .snapshot_transition
            .as_ref()
            .is_some_and(|transition| transition.committed)
        {
            self.snapshot_transition
                .take()
                .map(CommittedSnapshotTransition)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeferredAudioRequest {
    PlayDelayedSource(usize),
    ResumeAllSources,
    ActivateSource(usize),
    RefreshAmbienceSources,
    StopExclamation(u32),
    StopExclamationChannel(u32),
}

#[derive(Default)]
pub struct HostAudio {
    pub sound: SoundManager,
    pub deferred: Vec<DeferredAudioRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostModalRequest {
    Dialogue(i32),
    PopupText(i32),
    Debriefing(engine_player_command::DebriefingTextId),
    SherwoodReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSignal {
    ShowConsole,
    SilentWinWidgetSwap,
    MissionStateNotice,
    MissionStatePopup,
    ResetInput,
    PromoteFpsCheat,
    SherwoodTrading,
}

/// Live presentation facts required to admit a Sherwood trading-panel request.
///
/// These checks intentionally mirror the authoritative sale-command ordering:
/// host ownership, feature rule, then mission location. The engine repeats the
/// same checks when a sale reaches the deterministic command frame, so a stale
/// or forged presentation request cannot mutate campaign state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SherwoodTradingAccess {
    pub(crate) local_is_host: bool,
    pub(crate) enabled: bool,
    pub(crate) in_sherwood: bool,
}

impl SherwoodTradingAccess {
    pub(crate) fn validate(self) -> Result<(), robin_engine::trading::TradeRejectReason> {
        use robin_engine::trading::TradeRejectReason;
        if !self.local_is_host {
            return Err(TradeRejectReason::HostOnly);
        }
        if !self.enabled {
            return Err(TradeRejectReason::TradingDisabled);
        }
        if !self.in_sherwood {
            return Err(TradeRejectReason::NotInSherwood);
        }
        Ok(())
    }
}

/// Ordered, typed work emitted at the post-tick boundary. Variant-specific
/// drains preserve the existing host phase priority and simulation timing.
#[derive(Default)]
pub struct HostEffectBatches {
    modals: Vec<HostModalRequest>,
    signals: Vec<HostSignal>,
    trade_receipts: Vec<robin_engine::trading::TradeReceipt>,
    next_trade_request_id: u64,
    pub background_blits: Vec<PendingBgBlit>,
}

impl HostEffectBatches {
    pub fn pending_modal_kinds(&self) -> Vec<engine_player_command::ModalKind> {
        self.modals
            .iter()
            .map(|request| match *request {
                HostModalRequest::Dialogue(dialog_id) => {
                    engine_player_command::ModalKind::Dialog { dialog_id }
                }
                HostModalRequest::PopupText(text_id) => {
                    engine_player_command::ModalKind::PopupText { text_id }
                }
                HostModalRequest::Debriefing(text_id) => {
                    engine_player_command::ModalKind::Debriefing { text_id }
                }
                HostModalRequest::SherwoodReport => {
                    engine_player_command::ModalKind::SherwoodReport
                }
            })
            .collect()
    }

    pub fn extend_dialogues(&mut self, ids: impl IntoIterator<Item = i32>) {
        self.modals
            .extend(ids.into_iter().map(HostModalRequest::Dialogue));
    }

    pub fn extend_popup_texts(&mut self, ids: impl IntoIterator<Item = i32>) {
        self.modals
            .extend(ids.into_iter().map(HostModalRequest::PopupText));
    }

    pub fn extend_debriefings(
        &mut self,
        ids: impl IntoIterator<Item = engine_player_command::DebriefingTextId>,
    ) {
        self.modals
            .extend(ids.into_iter().map(HostModalRequest::Debriefing));
    }

    pub fn request_sherwood_report(&mut self) {
        if !self.has_sherwood_report() {
            self.modals.push(HostModalRequest::SherwoodReport);
        }
    }

    pub fn has_sherwood_report(&self) -> bool {
        self.modals.contains(&HostModalRequest::SherwoodReport)
    }

    pub fn take_sherwood_report(&mut self) -> bool {
        let Some(index) = self
            .modals
            .iter()
            .position(|request| *request == HostModalRequest::SherwoodReport)
        else {
            return false;
        };
        self.modals.remove(index);
        true
    }

    pub fn extend_trade_receipts(
        &mut self,
        receipts: impl IntoIterator<Item = robin_engine::trading::TradeReceipt>,
    ) {
        self.trade_receipts.extend(receipts);
    }

    pub fn take_trade_receipts(&mut self) -> Vec<robin_engine::trading::TradeReceipt> {
        std::mem::take(&mut self.trade_receipts)
    }

    /// Allocate a process-session correlation id for one authoritative sale.
    /// This counter deliberately survives panel close/reopen and effect-queue
    /// clears so a delayed network receipt cannot alias a newer request.
    pub(crate) fn allocate_trade_request_id(&mut self) -> u64 {
        self.next_trade_request_id = self
            .next_trade_request_id
            .checked_add(1)
            .expect("Sherwood trade request id exhausted");
        self.next_trade_request_id
    }

    pub fn dialogue_count(&self) -> usize {
        self.modals
            .iter()
            .filter(|request| matches!(request, HostModalRequest::Dialogue(_)))
            .count()
    }

    pub fn popup_text_count(&self) -> usize {
        self.modals
            .iter()
            .filter(|request| matches!(request, HostModalRequest::PopupText(_)))
            .count()
    }

    pub fn debriefing_count(&self) -> usize {
        self.modals
            .iter()
            .filter(|request| matches!(request, HostModalRequest::Debriefing(_)))
            .count()
    }

    pub fn take_dialogues(&mut self) -> Vec<i32> {
        take_modal_payloads(&mut self.modals, |request| match request {
            HostModalRequest::Dialogue(id) => Some(id),
            _ => None,
        })
    }

    pub fn take_popup_texts(&mut self) -> Vec<i32> {
        take_modal_payloads(&mut self.modals, |request| match request {
            HostModalRequest::PopupText(id) => Some(id),
            _ => None,
        })
    }

    pub fn take_debriefings(&mut self) -> Vec<engine_player_command::DebriefingTextId> {
        take_modal_payloads(&mut self.modals, |request| match request {
            HostModalRequest::Debriefing(id) => Some(id),
            _ => None,
        })
    }

    pub fn request_signal(&mut self, signal: HostSignal) {
        if !self.signals.contains(&signal) {
            self.signals.push(signal);
        }
    }

    /// Queue a player-facing trading-panel request only after all live access
    /// checks pass. This is the single producer used by keyboard and menu UI.
    pub(crate) fn request_sherwood_trading(
        &mut self,
        access: SherwoodTradingAccess,
    ) -> Result<(), robin_engine::trading::TradeRejectReason> {
        access.validate()?;
        self.request_signal(HostSignal::SherwoodTrading);
        Ok(())
    }

    /// Consume and revalidate a queued request immediately before modal
    /// construction. A settings/location/seat transition between input and
    /// presentation therefore fails closed, while an empty queue is ordinary.
    pub(crate) fn take_sherwood_trading(
        &mut self,
        access: SherwoodTradingAccess,
    ) -> Result<bool, robin_engine::trading::TradeRejectReason> {
        if !self.take_signal(HostSignal::SherwoodTrading) {
            return Ok(false);
        }
        access.validate()?;
        Ok(true)
    }

    pub fn has_signal(&self, signal: HostSignal) -> bool {
        self.signals.contains(&signal)
    }

    pub fn take_signal(&mut self, signal: HostSignal) -> bool {
        let Some(index) = self.signals.iter().position(|queued| *queued == signal) else {
            return false;
        };
        self.signals.remove(index);
        true
    }

    pub fn clear(&mut self) {
        self.modals.clear();
        self.signals.clear();
        self.trade_receipts.clear();
        self.background_blits.clear();
    }
}

fn take_modal_payloads<T>(
    requests: &mut Vec<HostModalRequest>,
    take: impl Fn(HostModalRequest) -> Option<T>,
) -> Vec<T> {
    let mut payloads = Vec::new();
    requests.retain(|request| {
        if let Some(payload) = take(*request) {
            payloads.push(payload);
            false
        } else {
            true
        }
    });
    payloads
}

#[derive(Default)]
pub struct HostScripting {
    pub lua_session: Option<crate::lua_session::LuaSession>,
}

/// Small process-host facade. Deterministic state remains in `Engine`; these
/// owners can be borrowed independently at the existing async/tick barriers.
///
/// Frontend state is deliberately explicit: `Host` must not dereference to
/// `HostFrontend`, or callers can accidentally regain process-wide authority.
///
/// Independent owners can be borrowed without granting the whole host:
///
/// ```
/// use robin_rs::host::Host;
/// let mut host = Host::scratch(1024.0, 768.0);
/// let frontend = &mut host.frontend;
/// let effects = &mut host.effects;
/// frontend.clear_background_decals();
/// robin_rs::blit_to_map::drain_pending_bg_blits(frontend, effects);
/// ```
///
/// ```compile_fail,E0609
/// use robin_rs::host::Host;
/// let mut host = Host::scratch(1024.0, 768.0);
/// host.input.has_focus = false;
/// ```
///
/// ```compile_fail,E0308
/// use robin_rs::host::{Host, HostFrontend};
/// fn frontend_only(_: &mut HostFrontend) {}
/// let mut host = Host::scratch(1024.0, 768.0);
/// frontend_only(&mut host);
/// ```
#[derive(Default)]
pub struct Host {
    application_context: ApplicationContext,
    /// Launch policy belongs to this mission host, never to its rendering state.
    /// Unbound scratch/bootstrap hosts have no achievement-persistence authority.
    session_achievement_eligibility:
        Option<crate::session_achievement::SessionAchievementEligibility>,
    pub frontend: HostFrontend,
    pub transport: HostTransport,
    pub audio: HostAudio,
    pub effects: HostEffectBatches,
    pub scripting: HostScripting,
}

/// Borrowed presentation authority. Unlike `Host`, this cannot submit network
/// traffic, run scripts, enqueue application effects, or mutate audio state.
/// Process-local borrows are deliberately not serialized.
pub(crate) struct HostPresentation<'a> {
    pub(crate) frontend: &'a mut HostFrontend,
    pub(crate) sound: &'a crate::sound::SoundManager,
    pub(crate) options: &'a engine_api::GlobalOptions,
    pub(crate) local_seat: robin_engine::player_command::PlayerId,
    application: &'a ApplicationContext,
}

impl HostPresentation<'_> {
    /// Read only the active presentation settings, without granting storage,
    /// profile mutation, asset preparation, or other application authority.
    pub(crate) fn graphic_config(&self) -> robin_engine::graphic_config::GraphicConfig {
        self.application
            .with_player_profiles(|profiles| {
                profiles
                    .get_active()
                    .map(|profile| profile.graphic_config.clone())
            })
            .unwrap_or_else(|error| panic!("rendering requires an active profile: {error}"))
            .expect("rendering requires an active profile")
    }
}

impl Host {
    pub(crate) fn presentation(&mut self) -> HostPresentation<'_> {
        HostPresentation {
            frontend: &mut self.frontend,
            sound: &self.audio.sound,
            options: self.application_context.options(),
            local_seat: self.transport.local_seat,
            application: &self.application_context,
        }
    }

    pub(crate) fn bind_session_achievement_eligibility(
        &mut self,
        eligibility: crate::session_achievement::SessionAchievementEligibility,
    ) -> Result<(), String> {
        if self.session_achievement_eligibility.is_some() {
            return Err("achievement eligibility is already bound to this mission host".into());
        }
        self.session_achievement_eligibility = Some(eligibility);
        Ok(())
    }

    pub(crate) fn session_achievement_eligibility(
        &self,
    ) -> Result<crate::session_achievement::SessionAchievementEligibility, String> {
        self.session_achievement_eligibility
            .ok_or_else(|| "mission host has no achievement eligibility launch policy".into())
    }

    pub fn preparation_files(&self) -> Result<&Arc<robin_engine::sbfile::SbFileSystem>, String> {
        self.application_context.preparation_files()
    }

    /// Read-only access preserves the initialized context of production hosts.
    /// Scratch hosts deliberately expose a bootstrap context with no storage authority.
    ///
    /// ```compile_fail
    /// use robin_rs::host::{ApplicationContext, Host};
    /// fn replace_context(host: &mut Host) {
    ///     host.application_context = ApplicationContext::default();
    /// }
    /// ```
    pub fn application_context(&self) -> &ApplicationContext {
        &self.application_context
    }

    /// Construct a production host only after application initialization.
    /// Snapshot failures (such as a poisoned service lock) remain recoverable.
    ///
    /// ```compile_fail
    /// use robin_rs::host::{ApplicationContext, Host};
    /// let _ = Host::new(ApplicationContext::default(), 800.0, 600.0);
    /// ```
    ///
    /// ```no_run
    /// use robin_rs::host::{ApplicationContext, Host, ReadyApplicationContext};
    /// fn construct(context: ApplicationContext) -> Result<Host, String> {
    ///     Host::new(ReadyApplicationContext::try_from(context)?, 800.0, 600.0)
    /// }
    /// ```
    pub fn new(
        application_context: ReadyApplicationContext,
        screen_width: f32,
        screen_height: f32,
    ) -> Result<Self, String> {
        let snapshot = application_context.host_snapshot()?;
        Ok(Self {
            application_context: application_context.into(),
            frontend: HostFrontend {
                viewport: ViewportState::new(screen_width, screen_height),
                input: InputState::focused(),
                shipping: snapshot.shipping,
                key_config: snapshot.key_config,
                custom_key_config: snapshot.custom_key_config,
                control_tactical_units: snapshot.control_tactical_units,
                planning: crate::frontend_input::FrontendPlanning::new(snapshot.plan_quick_actions),
                touch_camera_gestures: snapshot.touch_camera_gestures,
                native_refresh_presentation: snapshot.native_refresh_presentation,
                quick_action_cursor_pulse: snapshot.quick_action_cursor_pulse,
                diplomacy_visuals: snapshot.diplomacy_visuals,
                gameplay_config: snapshot.gameplay_config,
                ..Default::default()
            },
            ..Default::default()
        })
    }

    /// Construct a host for deterministic replay/test paths which never read
    /// application persistence or shipping resources.
    pub fn scratch(screen_width: f32, screen_height: f32) -> Self {
        Self {
            application_context: ApplicationContext::default(),
            frontend: HostFrontend {
                viewport: ViewportState::new(screen_width, screen_height),
                input: InputState::focused(),
                planning: crate::frontend_input::FrontendPlanning::new(true),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Reset host-side transient state after a save-load.  Mirrors the
    /// engine-side [`robin_engine::engine::Engine::restore`] fix-ups
    /// for the host half: a stale mid-drag rubber-band, a stale
    /// `focused_entity_id` pointing at a now-missing entity, or a
    /// UI-request queue partially drained before the load would all
    /// survive into the restored session without this wipe.  Called by
    /// [`crate::save_file::GameSaveFile::apply_to`] and by any future
    /// caller that swaps engines under a live host.
    ///
    /// Does NOT touch `SoundManager` — callers replace that wholesale
    /// from the save payload.
    pub fn post_load_reset(&mut self) {
        // Clear mouse/keyboard input state — otherwise a mid-drag
        // quick-load would leave the rubber-band box active with stale
        // screen coordinates, or keep a stale `focused_entity_id` that
        // no longer exists in the reloaded entity array.
        self.frontend
            .reset_interaction(InteractionReset::SnapshotRestored);

        // Drop any UI-request queues that were in flight before the
        // load.  They live host-side now — accumulated from per-tick
        // `SideEffects.pending_*` by `Host::apply_side_effects`.
        self.effects.clear();
        self.frontend.pending_console_output.clear();
        self.frontend.pending_print_screen = None;
    }

    /// Apply the engine-local outputs of a tick.  Consumes the
    /// [`SideEffects`] struct by value so owned sub-vectors
    /// can be moved directly into host accumulators without clones.
    /// Returns the tick's game-state code.
    pub fn apply_side_effects(&mut self, fx: SideEffects) -> GameCode {
        if let Some(fade) = fx.fade_to_black {
            self.frontend.fade_to_black = fade;
        }
        if let Some(show) = fx.set_draw_hidden {
            self.frontend.input.draw_hidden = show;
        }
        if fx.invalidate_trajectory_preview {
            // `SelectAction` trajectory cleanup: clear the jumper and
            // jumped trajectories, the valid flag, and the projectile
            // arc.  We fold all four trajectory overlays (jump-line
            // preview, projectile arc, valid flag, crumpled-net tint)
            // into the single host-side preview since there is only
            // ever one visible arc; clearing them together here is an
            // immediate wipe before the next mouse-update frame.
            self.frontend.trajectory_preview.invalidate_action();
            self.frontend.host_titbit_preview = None;
        }
        if fx.reset_input {
            // MSG_RESET_INPUT clears the rubber-band selection flags
            // and suppresses any pending drag / click so a modal popup
            // / dialog entered from a sequence command doesn't leave
            // input state armed.  Also zeroes the per-frame modifier
            // cache and the swordfight mouse-way polyline (modifier
            // keys, drag, UI focus, info overlay, mouse-way).
            self.frontend.input.reset_modal_input();
            // Reset does the swap `info_displayed = fps_cheat;
            // fps_cheat = false`: the FPS-cheat flag is consumed and
            // promoted into `info_displayed`, so toggling the FPS
            // cheat arms the next reset to leave the debug-info
            // overlay visible.  The cheat flag lives on
            // `DevState::debug.fps_display`, which is not reachable
            // from here — hand off via a typed host signal for
            // the game-loop site that owns `&mut DevState` to apply.
            self.effects.request_signal(HostSignal::PromoteFpsCheat);
            self.frontend.ui_focus = false;
            self.frontend.mouse_way.clear();
            // Zero the no-mouse-move accumulator so the
            // hover-trajectory gate (`TIME_TRAJECTORY_DISPLAY`)
            // doesn't re-arm immediately after a modal dialog or task
            // switch.
            self.frontend.trajectory_preview.interrupt_hover();
        }
        if fx.cancel_multi_selection {
            self.frontend.input.cancel_selection_gestures();
        }
        if let Some(top_left) = fx.pending_minimap_position {
            // Write the new minimap top-left back to the active player
            // profile on every accepted move. Persist through this host's
            // explicit application context and save to disk; failures are
            // logged after the sim has already accepted the new position.
            let context = self.application_context.clone();
            context
                .with_player_profiles_mut(|mgr| {
                    let profile = mgr
                        .get_active_mut()
                        .expect("ApplicationContext lost its required active player profile");
                    profile.minimap_x = top_left.x;
                    profile.minimap_y = top_left.y;
                    if let Err(e) = context.persist_player_profiles(mgr) {
                        tracing::warn!("failed to persist minimap position to profile: {e}");
                    }
                })
                .unwrap_or_else(|error| panic!("failed to persist minimap position: {error}"));
        }
        if fx.pending_swordfight_drag_ignore && self.frontend.input.is_dragging() {
            // Selected PC left Swordfighting this tick; if a drag was
            // in flight, raise `IgnoreMouseEvent(true, true, true)` so
            // the drag doesn't bleed into a click-release or a
            // subsequent double-click.
            self.frontend.input.ignore_mouse_event(true, true, true);
        }
        self.frontend.skip_render = fx.skip_render;
        // Dispatch sim-emitted sound commands onto the SoundManager.
        // Most variants queue into `SoundManager::pending_sounds` and
        // are played out by `SoundManager::hourglass`; the two that
        // need access to `engine.sound_sim.sources` (ResumeAllSources,
        // ActivateSource) are stashed on host and drained by
        // game_session before the hourglass call.
        for cmd in fx.sounds {
            match cmd {
                SoundCommand::StopExclamation { actor_id } => {
                    self.audio
                        .deferred
                        .push(DeferredAudioRequest::StopExclamation(actor_id.index()));
                }
                SoundCommand::Exclamation {
                    group,
                    profile_id,
                    exclamation_id,
                    variant,
                    position,
                    actor_id,
                } => {
                    if let Some(actor_id) = actor_id {
                        let had_deferred_stop = self.audio.deferred.iter().any(|request| {
                            *request == DeferredAudioRequest::StopExclamation(actor_id.index())
                        });
                        if had_deferred_stop {
                            self.audio.deferred.retain(|request| {
                                *request != DeferredAudioRequest::StopExclamation(actor_id.index())
                            });
                            self.audio.sound.drop_pending_exclamations(actor_id.index());
                            self.audio
                                .deferred
                                .push(DeferredAudioRequest::StopExclamationChannel(
                                    actor_id.index(),
                                ));
                        }
                    }
                    self.audio.sound.play_exclamation(
                        group,
                        profile_id,
                        exclamation_id,
                        variant,
                        position,
                        actor_id.map(|id| id.index()),
                    );
                }
                SoundCommand::Fx {
                    fx_id,
                    position,
                    material,
                } => {
                    self.audio.sound.queue_fx(fx_id, position, material);
                }
                SoundCommand::StrikeFx {
                    strike_kind,
                    weapon1,
                    weapon2,
                    position,
                } => {
                    self.audio
                        .sound
                        .queue_strike_fx(strike_kind, weapon1, weapon2, position);
                }
                SoundCommand::ImpactFx {
                    impact_kind,
                    weapon,
                    armor,
                    position,
                } => {
                    self.audio
                        .sound
                        .queue_impact_fx(impact_kind, weapon, armor, position);
                }
                SoundCommand::Jingle(jingle) => {
                    self.audio.sound.queue_jingle(jingle);
                }
                SoundCommand::SetMusicMode(mode) => {
                    self.audio.sound.set_music_mode(mode);
                }
                SoundCommand::ForceMusicMode(mode) => {
                    self.audio.sound.force_music_mode(mode);
                }
                SoundCommand::PlayDelayedSource(idx) => {
                    self.audio
                        .deferred
                        .push(DeferredAudioRequest::PlayDelayedSource(idx));
                }
                SoundCommand::ResumeAllSources { .. } => {
                    if !self
                        .audio
                        .deferred
                        .contains(&DeferredAudioRequest::ResumeAllSources)
                    {
                        self.audio
                            .deferred
                            .push(DeferredAudioRequest::ResumeAllSources);
                    }
                }
                SoundCommand::ActivateSource(idx) => {
                    self.audio
                        .deferred
                        .push(DeferredAudioRequest::ActivateSource(idx));
                }
                SoundCommand::RefreshAmbienceSources => {
                    if !self
                        .audio
                        .deferred
                        .contains(&DeferredAudioRequest::RefreshAmbienceSources)
                    {
                        self.audio
                            .deferred
                            .push(DeferredAudioRequest::RefreshAmbienceSources);
                    }
                }
            }
        }
        // Accumulate UI-request queues — the host drives the widgets
        // asynchronously so signals outlive a single tick.
        self.effects.extend_dialogues(fx.pending_dialogues);
        self.effects.extend_popup_texts(fx.pending_popup_texts);
        self.effects.extend_debriefings(fx.pending_debriefings);
        if fx.pending_sherwood_report {
            self.effects.request_sherwood_report();
        }
        if self.transport.local_seat == engine_player_command::PlayerId::HOST {
            self.effects.extend_trade_receipts(fx.trade_receipts);
        } else if !fx.trade_receipts.is_empty() {
            tracing::trace!(
                count = fx.trade_receipts.len(),
                "discarding host-only Sherwood trade receipts on a client"
            );
        }
        if fx.pending_show_console {
            self.effects.request_signal(HostSignal::ShowConsole);
        }
        if fx.pending_silent_win_widget_swap {
            self.effects.request_signal(HostSignal::SilentWinWidgetSwap);
        }
        if fx.pending_mission_state_notice {
            self.effects.request_signal(HostSignal::MissionStateNotice);
            self.effects.request_signal(HostSignal::MissionStatePopup);
        }
        if fx.pending_reset_input {
            self.effects.request_signal(HostSignal::ResetInput);
        }
        self.frontend.ui_focus |= fx.ui_has_focus;
        // Per-frame mark requests from sim-side Mark() calls (currently
        // scripted mission-team insertion → `EngineCommand::MarkPc`).
        // Accumulates with host-side mark sources (requirements-bar
        // hover, portrait guard hover); the render loop drains the
        // buffer right after the outline pass.
        self.frontend
            .input
            .marked_pc_ids
            .extend(fx.pending_mark_pc_ids);
        // Patch-effect background decal changes are accumulated across
        // frames until the next render pass drains them.
        self.effects.background_blits.extend(fx.bg_blits);
        fx.code
    }

    pub fn sync_sound_listener(&mut self) {
        self.audio.sound.set_listen_point(
            self.frontend.viewport.sound_listen_point(),
            self.frontend.viewport.zoom_factor,
        );
    }
}

impl HostFrontend {
    /// Mutable access to the frame holder before its opacity view is published.
    /// Post-publication mutations must use
    /// [`Self::rebind_frame_holder_shadow_color`] so the engine and renderer
    /// switch generations together.
    pub fn frame_holder_mut(&mut self) -> &mut FrameHolder {
        assert!(
            self.frame_holder_opacity.is_none(),
            "published frame holder cannot be mutated without synchronizing pixel opacity"
        );
        Arc::make_mut(&mut self.frame_holder)
    }

    /// Publish the fully initialized frame-holder generation for engine hit
    /// testing. This is a one-way loading boundary: subsequent dictionary
    /// changes must go through [`Self::rebind_frame_holder_shadow_color`].
    pub fn publish_frame_holder_opacity(&mut self) -> Arc<PublishedFrameHolder> {
        assert!(
            self.frame_holder_opacity.is_none(),
            "frame-holder opacity was already published"
        );
        let published = Arc::new(PublishedFrameHolder::new(Arc::clone(&self.frame_holder)));
        self.frame_holder_opacity = Some(Arc::clone(&published));
        published
    }

    /// Apply an ambiance shadow-key change and publish the resulting immutable
    /// generation to every engine-side opacity reader.
    pub fn rebind_frame_holder_shadow_color(&mut self, shadow_color: u16) {
        let published = Arc::clone(
            self.frame_holder_opacity
                .as_ref()
                .expect("frame-holder opacity must be published before runtime rebinding"),
        );
        Arc::make_mut(&mut self.frame_holder).apply_arno_law(shadow_color);
        published.publish(Arc::clone(&self.frame_holder));
    }

    /// Rebuild ambience dictionaries and the shadow key as one published
    /// generation. This extends Feature 14's single-generation rebinding
    /// boundary so render pixels and engine hit testing stay synchronized.
    pub fn rebind_frame_holder_ambiance(
        &mut self,
        ambiance: engine_api::Ambiance,
        bypass_fog_sprites_crash: bool,
        shadow_color: u16,
    ) {
        let published = Arc::clone(
            self.frame_holder_opacity
                .as_ref()
                .expect("frame-holder opacity must be published before runtime rebinding"),
        );
        let holder = Arc::make_mut(&mut self.frame_holder);
        if bypass_fog_sprites_crash {
            holder.drop_variant_dictionaries(SpriteVariant::Night);
            holder.drop_variant_dictionaries(SpriteVariant::Fog);
        } else {
            match ambiance {
                engine_api::Ambiance::Fog => {
                    holder.drop_variant_dictionaries(SpriteVariant::Night);
                    holder.generate_fog_dictionaries();
                    holder.set_global_shadow(10);
                    holder.set_global_blip_shadow(40);
                }
                engine_api::Ambiance::Night => {
                    holder.drop_variant_dictionaries(SpriteVariant::Fog);
                    holder.generate_night_dictionaries();
                    holder.set_global_shadow(40);
                    holder.set_global_blip_shadow(60);
                }
                _ => {
                    holder.drop_variant_dictionaries(SpriteVariant::Night);
                    holder.drop_variant_dictionaries(SpriteVariant::Fog);
                    holder.set_global_shadow(40);
                    holder.set_global_blip_shadow(60);
                }
            }
        }
        holder.apply_arno_law(shadow_color);
        published.publish(Arc::clone(&self.frame_holder));
    }

    /// Clear persistent decals that belonged to the previous level.
    pub fn clear_background_decals(&mut self) {
        self.background_decals.clear();
        self.background_decal_order.clear();
    }

    pub fn install_trajectory_ground_mark_sprite(&mut self, data: &GroundMarkSpriteData) {
        self.trajectory_preview.install_mark_sprite(data);
    }
}

#[cfg(test)]
mod viewport_touch_tests {
    use super::*;

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.001,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn combined_touch_transform_preserves_anchor() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(500.0, 400.0);
        let previous_centroid = ScreenPoint::new(300.0, 250.0);
        let anchor = viewport.screen_to_map_unchecked(previous_centroid);

        viewport.begin_touch_transform(true);
        viewport.apply_touch_transform(
            ScreenPoint::new(340.0, 270.0),
            ScreenVec::new(40.0, 20.0),
            1.5,
        );

        close(viewport.zoom_factor, 1.5);
        let transformed_anchor = viewport.screen_to_map_unchecked(ScreenPoint::new(340.0, 270.0));
        close(transformed_anchor.x, anchor.x);
        close(transformed_anchor.y, anchor.y);
    }

    #[test]
    fn director_camera_is_identity_on_matching_canvas() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.adopt_director_camera(
            MapPoint::new(1234.5678, 987.6543),
            ScreenSize::new(1024.0, 768.0),
            0.5,
        );
        assert_eq!(viewport.view_position, MapPoint::new(1234.5678, 987.6543));
        assert_eq!(viewport.zoom_factor, 0.5);
    }

    #[test]
    fn director_camera_recentres_focal_point_on_widescreen_canvas() {
        // The engine framed focal point (1512, 1384) as top-left (1000, 1000)
        // in its 1024x768 virtual view. A 1280x720 host must show that same
        // point in the middle of its own canvas.
        let mut viewport = ViewportState::new(1280.0, 720.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.adopt_director_camera(
            MapPoint::new(1000.0, 1000.0),
            ScreenSize::new(1024.0, 768.0),
            1.0,
        );
        assert_eq!(viewport.view_position, MapPoint::new(872.0, 1024.0));
        let centre = viewport.screen_to_map_unchecked(ScreenPoint::new(640.0, 360.0));
        assert_eq!(centre, MapPoint::new(1512.0, 1384.0));

        // Zoomed out, the shift is measured in map pixels, so it doubles.
        viewport.adopt_director_camera(
            MapPoint::new(1000.0, 1000.0),
            ScreenSize::new(1024.0, 768.0),
            0.5,
        );
        assert_eq!(viewport.view_position, MapPoint::new(744.0, 1048.0));
    }

    #[test]
    fn rejected_touch_transform_does_not_move_camera() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(500.0, 400.0);
        viewport.begin_touch_transform(false);
        viewport.apply_touch_transform(
            ScreenPoint::new(400.0, 300.0),
            ScreenVec::new(50.0, 20.0),
            1.2,
        );
        assert_eq!(viewport.view_position, MapPoint::new(500.0, 400.0));
        assert_eq!(viewport.zoom_factor, 1.0);
    }

    #[test]
    fn touch_zoom_and_inertia_hard_clamp_to_map() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(1200.0, 900.0);
        viewport.begin_touch_transform(true);
        viewport.apply_touch_transform(
            ScreenPoint::new(0.0, 0.0),
            ScreenVec::new(1000.0, 1000.0),
            0.01,
        );
        assert_eq!(viewport.zoom_factor, 0.5);
        assert_eq!(viewport.view_position, MapPoint::ZERO);

        viewport.end_touch_transform(ScreenVec::new(2000.0, 1200.0), false, 100);
        assert!(!viewport.advance_touch_inertia(116));
        assert_eq!(viewport.view_position, MapPoint::ZERO);
        assert!(!viewport.advance_touch_inertia(132));
    }

    #[test]
    fn cancelling_transform_disables_momentum() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(1000.0, 1000.0);
        viewport.begin_touch_transform(true);
        viewport.end_touch_transform(ScreenVec::new(1000.0, 0.0), true, 100);
        assert!(!viewport.advance_touch_inertia(150));
        assert_eq!(viewport.view_position, MapPoint::new(1000.0, 1000.0));
    }

    #[test]
    fn touch_inertia_clamps_implausible_release_velocity() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(1000.0, 1000.0);
        viewport.begin_touch_transform(true);
        viewport.end_touch_transform(ScreenVec::new(1_000_000.0, 0.0), false, 100);

        assert!(viewport.advance_touch_inertia(116));
        close(viewport.view_position.x, 920.0);
        close(viewport.view_position.y, 1000.0);
    }
}

#[cfg(test)]
mod interaction_reset_tests {
    use super::*;

    #[test]
    fn ready_context_rejects_bootstrap_and_its_serialized_form() {
        let bootstrap = ApplicationContext::default();
        let bytes = serde_json::to_vec(&bootstrap).unwrap();
        assert!(ReadyApplicationContext::try_from(bootstrap).is_err());
        assert!(serde_json::from_slice::<ReadyApplicationContext>(&bytes).is_err());
    }

    #[test]
    fn snapshot_restore_discards_entity_and_pointer_state_but_preserves_preferences_and_pose() {
        let mut host = Host::scratch(640.0, 480.0);
        host.frontend
            .input
            .press_left_pointer(Default::default(), 1);
        host.frontend.pointer_capture.capture_touch_plan();
        host.frontend.pointer_capture.right_button_down(2);
        host.frontend.planning.update_preference(true);
        host.frontend.planning.toggle_touch();
        host.frontend
            .trajectory_preview
            .apply(robin_engine::engine::input::TrajectoryPreview::HitNoArc);
        host.frontend.item_effect_preview = Some(ItemEffectPreview {
            center: MapPoint::ZERO,
            radius: Some(20),
            localization_key: "test",
            fallback_text: "test",
            blocked: false,
        });
        host.frontend
            .tactical_targeting
            .arm_patrol(Vec::new(), TacticalFormation::default());
        host.frontend.viewport.view_position = MapPoint::new(100.0, 200.0);
        host.frontend.viewport.zoom_factor = 2.0;
        host.frontend.planning.update_preference(true);

        host.post_load_reset();

        assert!(!host.frontend.input.is_dragging());
        assert!(!host.frontend.pointer_capture.touch_plan_captured());
        assert!(!host.frontend.pointer_capture.take_right_double_click());
        assert!(!host.frontend.planning.touch_latched());
        assert!(!host.frontend.trajectory_preview.is_valid());
        assert!(!host.frontend.tactical_targeting.is_armed());
        assert!(host.frontend.item_effect_preview.is_none());
        assert_eq!(
            host.frontend.viewport.view_position,
            MapPoint::new(100.0, 200.0)
        );
        assert_eq!(host.frontend.viewport.zoom_factor, 2.0);
        assert!(host.frontend.planning.enabled());
        assert!(host.frontend.mission_surfaces.map().is_none());
    }

    #[test]
    fn modal_and_engine_resets_preserve_sticky_planning_but_cancel_pointer_capture() {
        for reason in [
            InteractionReset::ModalClosed,
            InteractionReset::EngineRequested,
        ] {
            let mut host = Host::scratch(640.0, 480.0);
            host.frontend.planning.update_preference(true);
            host.frontend.planning.toggle_touch();
            host.frontend.pointer_capture.capture_touch_plan();
            host.frontend
                .input
                .press_left_pointer(Default::default(), 1);
            host.frontend.viewport.begin_touch_transform(true);
            host.frontend.reset_interaction(reason);
            assert!(host.frontend.planning.touch_latched());
            assert!(!host.frontend.pointer_capture.touch_plan_captured());
            assert!(!host.frontend.input.left_mouse_down());
            assert!(!host.frontend.viewport.advance_touch_inertia(100));
        }
    }

    #[test]
    fn action_and_input_effects_keep_their_distinct_preview_reset_scopes() {
        use robin_engine::engine::input::TrajectoryPreview;
        let mut host = Host::scratch(640.0, 480.0);
        host.frontend
            .trajectory_preview
            .observe_hover(false, Default::default(), MapPoint::ZERO);
        host.frontend
            .trajectory_preview
            .observe_hover(false, Default::default(), MapPoint::ZERO);
        host.frontend
            .trajectory_preview
            .apply(TrajectoryPreview::HitNoArc);
        host.frontend
            .tactical_targeting
            .arm_patrol(Vec::new(), TacticalFormation::Line);
        host.apply_side_effects(SideEffects {
            invalidate_trajectory_preview: true,
            ..Default::default()
        });
        assert!(!host.frontend.trajectory_preview.is_valid());
        assert_eq!(host.frontend.trajectory_preview.hover_ticks(), 2);
        assert!(host.frontend.tactical_targeting.is_armed());
        host.frontend
            .trajectory_preview
            .apply(TrajectoryPreview::HitNoArc);
        host.apply_side_effects(SideEffects {
            reset_input: true,
            ..Default::default()
        });
        assert_eq!(host.frontend.trajectory_preview.hover_ticks(), 0);
        assert!(host.frontend.trajectory_preview.is_valid());
        assert!(host.frontend.tactical_targeting.is_armed());
        host.post_load_reset();
        assert!(!host.frontend.trajectory_preview.is_valid());
        assert!(!host.frontend.tactical_targeting.is_armed());
    }
}

#[cfg(test)]
mod application_context_tests {
    use super::*;
    use robin_assets::frame_holder::{SHADOW_KEY, SpriteVariant, TRANSPARENT_COLOR_16};
    use robin_assets::shipping_datadir::{ShippingSprite, ShippingSpriteBank};
    use robin_engine::campaign::Campaign;
    use robin_engine::coordinates::{SpriteAnchor, SpriteFrameOffset};
    use robin_engine::element::{ElementData, ElementFx, ElementKind, Entity};
    use robin_engine::player_profile::DifficultyLevel;
    use robin_engine::sprite::Sprite;
    use robin_engine::sprite_script::SpriteScript;
    use winit::keyboard::KeyCode;

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
        let decoded: ApplicationContext = serde_json::from_value(encoded).unwrap();
        assert!(decoded.cache_clear_status().is_err());
        assert!(decoded.begin_distributed_mod_cache_clear().is_err());
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
        let decoded: ApplicationContext = serde_json::from_value(encoded).unwrap();
        assert!(decoded.asset_cache().is_err());
        assert!(decoded.preparation_files().is_err());
        assert!(
            decoded
                .set_language(LanguageSelection::Auto)
                .unwrap_err()
                .contains("asset cache")
        );
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
        let restored: ApplicationContext = serde_json::from_value(encoded).unwrap();
        let error = restored
            .with_player_profiles(|profiles| restored.persist_player_profiles(profiles))
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("deserialized context"));
        assert!(
            restored
                .active_profile_save_directory()
                .unwrap_err()
                .contains("deserialized context")
        );
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
                .key_config
                .get_binding("ZoomIn")
                .unwrap()
                .primary_key,
            Some(KeyCode::F2)
        );
        assert_eq!(
            hard_host
                .frontend
                .key_config
                .get_binding("ZoomIn")
                .unwrap()
                .primary_key,
            Some(KeyCode::F3)
        );
        assert!(
            easy_host
                .frontend
                .shipping
                .as_ref()
                .unwrap()
                .raw
                .contains_key("easy.marker")
        );
        assert!(
            !easy_host
                .frontend
                .shipping
                .as_ref()
                .unwrap()
                .raw
                .contains_key("hard.marker")
        );
        assert!(
            hard_host
                .frontend
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
    fn decoded_ready_host_keeps_profile_storage_unavailable() {
        let ready = ReadyApplicationContext::try_from(context(
            0,
            DifficultyLevel::Medium,
            KeyCode::F2,
            "ready.marker",
        ))
        .unwrap();
        let bytes = serde_json::to_vec(&ready).unwrap();
        let decoded: ReadyApplicationContext = serde_json::from_slice(&bytes).unwrap();
        let host = Host::new(decoded, 800.0, 600.0).unwrap();
        let error = host
            .application_context()
            .with_player_profiles(|profiles| {
                host.application_context().persist_player_profiles(profiles)
            })
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("unavailable"), "{error}");
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
        let decoded: ReadyApplicationContext = serde_json::from_value(encoded).unwrap();
        assert!(Host::new(decoded, 800.0, 600.0).is_err());
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
        let mut options = engine_api::GlobalOptions::default();
        options.script_enabled = false;
        options.highlander = true;
        let context =
            ApplicationContext::complete_official_projection(options, sim_config, None).unwrap();

        assert_eq!(context.sim_config(), sim_config);
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
                    .key_config
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
        assert_eq!(host.frontend.key_config.key_type, active_keys.key_type);
        assert_eq!(
            host.frontend.custom_key_config.key_type,
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

    #[test]
    fn effect_batches_preserve_domain_order_and_coalesce_signals() {
        let mut effects = HostEffectBatches::default();
        effects.extend_dialogues([7]);
        effects.extend_popup_texts([11]);
        effects.extend_dialogues([8, 9]);
        effects.request_sherwood_report();
        effects.request_sherwood_report();
        effects.request_signal(HostSignal::ResetInput);
        effects.request_signal(HostSignal::ShowConsole);
        effects.request_signal(HostSignal::ResetInput);

        assert_eq!(effects.take_dialogues(), vec![7, 8, 9]);
        assert_eq!(effects.take_popup_texts(), vec![11]);
        assert!(effects.take_sherwood_report());
        assert!(!effects.take_sherwood_report());
        assert!(effects.take_signal(HostSignal::ResetInput));
        assert!(!effects.take_signal(HostSignal::ResetInput));
        assert!(effects.take_signal(HostSignal::ShowConsole));
    }

    #[test]
    fn trading_signal_requires_host_enabled_rule_and_sherwood() {
        use robin_engine::trading::TradeRejectReason;

        let allowed = SherwoodTradingAccess {
            local_is_host: true,
            enabled: true,
            in_sherwood: true,
        };
        let cases = [
            (
                SherwoodTradingAccess {
                    local_is_host: false,
                    ..allowed
                },
                TradeRejectReason::HostOnly,
            ),
            (
                SherwoodTradingAccess {
                    enabled: false,
                    ..allowed
                },
                TradeRejectReason::TradingDisabled,
            ),
            (
                SherwoodTradingAccess {
                    in_sherwood: false,
                    ..allowed
                },
                TradeRejectReason::NotInSherwood,
            ),
        ];

        for (access, reason) in cases {
            let mut effects = HostEffectBatches::default();
            assert_eq!(effects.request_sherwood_trading(access), Err(reason));
            assert!(!effects.has_signal(HostSignal::SherwoodTrading));
        }

        let mut effects = HostEffectBatches::default();
        assert_eq!(effects.request_sherwood_trading(allowed), Ok(()));
        assert!(effects.has_signal(HostSignal::SherwoodTrading));
        assert_eq!(effects.take_sherwood_trading(allowed), Ok(true));
        assert_eq!(effects.take_sherwood_trading(allowed), Ok(false));
    }

    #[test]
    fn queued_trading_signal_is_revalidated_and_drained_before_modal_dispatch() {
        use robin_engine::trading::TradeRejectReason;

        let allowed = SherwoodTradingAccess {
            local_is_host: true,
            enabled: true,
            in_sherwood: true,
        };
        let mut effects = HostEffectBatches::default();
        effects.request_sherwood_trading(allowed).unwrap();

        let disabled = SherwoodTradingAccess {
            enabled: false,
            ..allowed
        };
        assert_eq!(
            effects.take_sherwood_trading(disabled),
            Err(TradeRejectReason::TradingDisabled)
        );
        assert!(!effects.has_signal(HostSignal::SherwoodTrading));
    }

    fn dictionary_frame_holder(shadow_color: u16) -> FrameHolder {
        let mut shipping = ShippingDatadir::default();
        shipping.sprite_bank = Some(ShippingSpriteBank {
            signature: 0x51A0_0001,
            dictionaries: vec![robin_assets::frame_holder::FrameDictionary::from_raw(
                1,
                vec![SHADOW_KEY, 0x0841, TRANSPARENT_COLOR_16, 0x1234],
            )],
            sprite_count: 1,
            sprites: vec![(
                0,
                ShippingSprite {
                    width: 4,
                    height: 1,
                    dictionary_index: 0,
                    packed_data: std::sync::Arc::new(vec![0]),
                    raster: None,
                },
            )],
            vq_chunks: Vec::new(),
            rle_jxl_chunks: Vec::new(),
        });

        let mut holder = FrameHolder::new();
        holder
            .initialize_sprite_bank_with_progress(".", &mut |_| {}, Some(&shipping))
            .expect("load synthetic dictionary bank");
        holder.generate_night_dictionaries();
        holder.apply_arno_law(shadow_color);
        holder
    }

    fn rendered_dictionary_pixel_is_opaque(
        holder: &FrameHolder,
        variant: SpriteVariant,
        shadow_color: u16,
        x: usize,
    ) -> bool {
        let mut pixels = [TRANSPARENT_COLOR_16; 4];
        holder.uncompress_frame(&mut pixels, 4, 0, variant, shadow_color, 16);
        let pixel = pixels[x];
        pixel != TRANSPARENT_COLOR_16 && pixel != SHADOW_KEY && pixel != shadow_color
    }

    fn dictionary_sprite_entity() -> Entity {
        let script = SpriteScript {
            frame_ids: vec![0],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
            ..Default::default()
        };
        let mut element = {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::Fx;
            initial_element.sprite = Sprite {
                current_width: 4,
                current_height: 1,
                scripts: Arc::new(vec![script]),
                center: SpriteAnchor::ZERO,
                ..Default::default()
            };
            initial_element
        };
        element.set_position_map(MapPoint::new(100.0, 100.0));
        Entity::Fx(ElementFx {
            element,
            fx: Default::default(),
        })
    }

    #[test]
    fn ambiance_rebind_publishes_renderer_dictionary_generation_to_engine_hit_testing() {
        const INITIAL_NIGHT_COLOR: u16 = 0x0040;
        const REBOUND_NIGHT_COLOR: u16 = 0x0841;

        let mut host = Host::scratch(1024.0, 768.0);
        host.frontend.frame_holder = Arc::new(dictionary_frame_holder(INITIAL_NIGHT_COLOR));
        let published = host.frontend.publish_frame_holder_opacity();

        let mut assets = engine_api::LevelAssets::new();
        assets.attachments.pixel_opacity = Some(published.clone());
        let engine =
            engine_api::Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut assets)
                .expect("construct sprite-hit-test engine");
        let entity = dictionary_sprite_entity();
        let shadow_point = MapPoint::new(100.0, 100.0);
        let solid_point = MapPoint::new(101.0, 100.0);

        assert!(Arc::ptr_eq(
            &host.frontend.frame_holder,
            &published.snapshot()
        ));
        assert!(!rendered_dictionary_pixel_is_opaque(
            &host.frontend.frame_holder,
            SpriteVariant::Day,
            INITIAL_NIGHT_COLOR,
            0,
        ));
        assert!(!engine.is_point_on_sprite(&assets, &entity, shadow_point, false));
        assert!(engine.is_point_on_sprite(&assets, &entity, shadow_point, true));
        let cloned_assets = assets.clone();

        // Mirrors a scripted Weather::night_color change observed by the
        // runtime visual refresh: COW-rebind the renderer generation, then
        // publish that exact Arc to the original and cloned LevelAssets
        // opacity handles.
        host.frontend
            .rebind_frame_holder_shadow_color(REBOUND_NIGHT_COLOR);

        assert!(Arc::ptr_eq(
            &host.frontend.frame_holder,
            &published.snapshot()
        ));
        for variant in [SpriteVariant::Day, SpriteVariant::Night] {
            let renderer_shadow = rendered_dictionary_pixel_is_opaque(
                &host.frontend.frame_holder,
                variant,
                REBOUND_NIGHT_COLOR,
                0,
            );
            let engine_shadow = engine.is_point_on_sprite(&assets, &entity, shadow_point, false);
            assert_eq!(renderer_shadow, engine_shadow);
            assert_eq!(
                renderer_shadow,
                engine.is_point_on_sprite(&cloned_assets, &entity, shadow_point, false)
            );
            assert!(!renderer_shadow);
        }
        assert!(rendered_dictionary_pixel_is_opaque(
            &host.frontend.frame_holder,
            SpriteVariant::Day,
            REBOUND_NIGHT_COLOR,
            1,
        ));
        assert!(engine.is_point_on_sprite(&assets, &entity, solid_point, false));
        assert!(engine.is_point_on_sprite(&cloned_assets, &entity, solid_point, false));
    }
}
