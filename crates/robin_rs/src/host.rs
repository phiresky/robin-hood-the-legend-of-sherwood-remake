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
use robin_engine::element::{EntityId, TrajectoryPoint};
use robin_engine::engine as engine_api;
use robin_engine::engine::{
    DrawOrder, FadeToBlack, GroundMarkSpriteData, InputState, PendingBgBlit, SideEffects,
    SoundCommand,
};
use robin_engine::game_operation::GameCode;
use robin_engine::markers as engine_markers;
use robin_engine::markers::GroundMark;
use robin_engine::player_command as engine_player_command;
use robin_engine::player_profile::{PlayerProfile, PlayerProfileManager};
use robin_engine::profiles::Action;
use robin_engine::tactical_control::TacticalFormation;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::bg_cache::BackgroundDecal;
use crate::draw_manager::DrawManager;
use crate::key_config::KeyConfig;
use crate::key_config_store::{KeyConfigStore, ProfileKeyConfig};
use crate::mouse_way::MouseWay;
use crate::pc_info_overlay::PcInfoOverlay;
use crate::sound::SoundManager;

const PANNEL_HEIGHT: f32 = engine_api::PANNEL_HEIGHT;
const DISPLAY_INFO_SAMPLES: usize = 16;

/// A world-space target still required by an allied portrait action.
#[derive(Debug, Clone)]
pub enum TacticalTargetMode {
    Patrol {
        soldiers: Vec<EntityId>,
        formation: TacticalFormation,
    },
}

/// Mutable application services shared by clones of one
/// [`ApplicationContext`]. Separate contexts allocate separate service sets,
/// which makes tests, headless sessions, and future multi-instance hosts
/// independent instead of routing through process-wide singletons.
#[derive(Debug, Serialize, Deserialize)]
struct ApplicationServices {
    player_profiles: Mutex<PlayerProfileManager>,
    key_configs: Mutex<KeyConfigStore>,
    shipping: Option<Arc<ShippingDatadir>>,
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

/// Owned host-facing snapshot copied out of an [`ApplicationContext`].
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HostContextSnapshot {
    shipping: Option<Arc<ShippingDatadir>>,
    key_config: KeyConfig,
    custom_key_config: KeyConfig,
    #[serde(alias = "control_allied_soldiers")]
    control_tactical_units: bool,
    touch_camera_gestures: bool,
    native_refresh_presentation: bool,
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
        options: engine_api::GlobalOptions,
        player_profiles: PlayerProfileManager,
        mut key_configs: KeyConfigStore,
        shipping: Option<Arc<ShippingDatadir>>,
    ) -> Result<Self, String> {
        let active = player_profiles
            .get_active()
            .ok_or_else(|| "ApplicationContext requires an active player profile".to_string())?;
        let difficulty = active.difficulty;
        let amount_of_speaking = active.sound_config.amount_of_speaking;
        let fix_hard_reaction_times = active.gameplay_config.fix_hard_reaction_times;
        let enable_unbinding = active.gameplay_config.enable_unbinding;

        // Original provenance: `original-code/RHPlayerProfile.h:44-45` stores
        // active and custom key configs on each player profile, and
        // `original-code/RHgameinputtranslator.cpp:326` snapshots the active
        // profile's bindings into the mission input translator. The Rust port
        // keeps the host-side key type in a parallel store keyed by the same
        // profile id.
        for profile in &player_profiles.profiles {
            key_configs.entry_or_default(profile.id);
        }

        let mut sim_config = engine_api::SimConfig::from_options(&options, difficulty);
        sim_config.amount_of_speaking = amount_of_speaking;
        sim_config.fix_hard_reaction_times = fix_hard_reaction_times;
        sim_config.enable_unbinding = enable_unbinding;
        Ok(Self {
            sim_config: Arc::new(Mutex::new(sim_config)),
            options,
            services: Some(Arc::new(ApplicationServices {
                player_profiles: Mutex::new(player_profiles),
                key_configs: Mutex::new(key_configs),
                shipping,
            })),
        })
    }

    pub fn with_options(mut self, options: engine_api::GlobalOptions) -> Self {
        let existing = self.sim_config();
        let mut sim_config = engine_api::SimConfig::from_options(&options, existing.difficulty);
        sim_config.amount_of_speaking = existing.amount_of_speaking;
        sim_config.fix_hard_reaction_times = existing.fix_hard_reaction_times;
        sim_config.enable_unbinding = existing.enable_unbinding;
        *self
            .sim_config
            .lock()
            .expect("ApplicationContext sim-config lock poisoned") = sim_config;
        self.options = options;
        self
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

    pub fn player_profiles_snapshot(&self) -> Result<PlayerProfileManager, String> {
        self.with_player_profiles(Clone::clone)
    }

    pub fn active_profile_snapshot(&self) -> Result<PlayerProfile, String> {
        self.with_player_profiles(|profiles| profiles.get_active().cloned())?
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
        let (result, difficulty, amount_of_speaking, fix_hard_reaction_times, enable_unbinding) = {
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
                active.gameplay_config.fix_hard_reaction_times,
                active.gameplay_config.enable_unbinding,
            )
        };
        self.refresh_profile_derived_state(
            difficulty,
            amount_of_speaking,
            fix_hard_reaction_times,
            enable_unbinding,
        )?;
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
        let (profile_id, difficulty, amount_of_speaking, fix_hard_reaction_times, enable_unbinding) = {
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

            if !profiles.default_profiles {
                return Err("first-launch profile transition was already completed".to_string());
            }

            if let Some((name, difficulty)) = replacement {
                if profiles.profiles.len() != 1 || profiles.active_index != Some(0) {
                    return Err(format!(
                        "first-launch replacement requires one active placeholder, found {} profiles with active index {:?}",
                        profiles.profiles.len(),
                        profiles.active_index,
                    ));
                }
                profiles.default_profiles = false;
                let placeholder_id = profiles.profiles[0].id;
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
            let fix_hard_reaction_times = active.gameplay_config.fix_hard_reaction_times;
            let enable_unbinding = active.gameplay_config.enable_unbinding;

            if let Err(error) = profiles.save() {
                #[cfg(not(target_arch = "wasm32"))]
                return Err(format!(
                    "failed to persist first-launch player profile: {error}"
                ));
                #[cfg(target_arch = "wasm32")]
                tracing::warn!(
                    "Browser profile persistence is unavailable; keeping the first-launch profile in memory for this session: {error}"
                );
            }
            if let Err(error) = key_configs.save() {
                #[cfg(not(target_arch = "wasm32"))]
                return Err(format!(
                    "failed to persist first-launch key configuration: {error}"
                ));
                #[cfg(target_arch = "wasm32")]
                tracing::warn!(
                    "Browser key-config persistence is unavailable; keeping the first-launch configuration in memory for this session: {error}"
                );
            }
            // TODO: Persist browser profiles and key configurations in
            // IndexedDB instead of keeping first-launch changes session-only.
            (
                profile_id,
                difficulty,
                amount_of_speaking,
                fix_hard_reaction_times,
                enable_unbinding,
            )
        };

        self.refresh_profile_derived_state(
            difficulty,
            amount_of_speaking,
            fix_hard_reaction_times,
            enable_unbinding,
        )?;
        Ok(profile_id)
    }

    pub(crate) fn active_profile_save_directory(&self) -> Result<std::path::PathBuf, String> {
        self.with_player_profiles(|profiles| {
            let profile = profiles.get_active().ok_or_else(|| {
                "ApplicationContext has no active profile for save directory".to_string()
            })?;
            Ok(std::path::Path::new(&profiles.save_directory).join(
                robin_engine::player_profile::profile_save_subdirectory(profile.id),
            ))
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
            touch_camera_gestures: active_profile.gameplay_config.touch_camera_gestures,
            native_refresh_presentation: active_profile.graphic_config.native_refresh_presentation,
        })
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
        fix_hard_reaction_times: bool,
        enable_unbinding: bool,
    ) -> Result<(), String> {
        let mut sim_config = engine_api::SimConfig::from_options(&self.options, difficulty);
        sim_config.amount_of_speaking = amount_of_speaking;
        sim_config.fix_hard_reaction_times = fix_hard_reaction_times;
        sim_config.enable_unbinding = enable_unbinding;
        *self
            .sim_config
            .lock()
            .map_err(|_| "ApplicationContext sim-config lock poisoned".to_string())? = sim_config;

        Ok(())
    }
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
    pub fn adopt_director_camera(&mut self, view_position: MapPoint, zoom_factor: f32) {
        assert!(
            zoom_factor.is_finite() && zoom_factor > 0.0,
            "director camera supplied invalid zoom factor {zoom_factor}"
        );
        self.old_view_position = self.view_position;
        self.old_zoom_factor = self.zoom_factor;
        self.view_position = view_position;
        self.zoom_factor = zoom_factor;
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
    pub map_surface: u32,
    pub minimap_corner_surfaces: Vec<u32>,
    pub minimap_corner_size: ScreenSize,
    /// Per-`DotType` dot sprite `(surface, width, height)`. Indexed by
    /// `DotType as usize`. Populated at mission start; empty until then.
    pub minimap_dot_surfaces: Vec<(u32, u16, u16)>,
    pub ground_mark_surfaces: Vec<(u32, u16, u16)>,
    pub viewport: ViewportState,
    pub engine_display: engine_api::HostDisplayState,

    // ── Input ────────────────────────────────────────────────────
    pub input: InputState,

    /// Platform click count captured on right-button down. The matching
    /// release event has no click-count field, so portrait input consumes it.
    pub right_double_click_pending: bool,

    /// Active profile's opt-in tactical-unit control setting. Host-local
    /// because resolved player commands, rather than UI preferences, cross
    /// replay and multiplayer boundaries.
    pub control_tactical_units: bool,

    /// Host-local targeting prompt armed by the tactical patrol portrait button.
    pub tactical_target_mode: Option<TacticalTargetMode>,

    /// Active profile's touch-camera gesture setting. Host-local because
    /// camera pan/zoom/inertia never enters deterministic simulation state.
    pub touch_camera_gestures: bool,

    /// Opt-in display-rate re-presentation. Host-local and intentionally
    /// absent from deterministic save/replay state.
    pub native_refresh_presentation: bool,

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
    pub valid_trajectory: bool,
    /// Modifier/action identity that produced the cached preview. Changing
    /// either invalidates it even when the mouse itself has not moved.
    pub trajectory_preview_shift_held: bool,
    pub trajectory_preview_action: Action,
    /// Previous live-input modifier state used to emit a deterministic
    /// planned-action cancel command on the Shift release edge.
    pub trajectory_preview_points: Vec<TrajectoryPoint>,
    pub trajectory_preview_start: WorldPoint3D,
    /// Shooter layer captured alongside `trajectory_preview_points`.
    /// Passed to `GroundMark::add_mark` by the ground-mark drop driver
    /// as the layer argument for the trajectory display.
    pub trajectory_preview_layer: u16,
    /// Set by the trajectory-preview computation when the projected
    /// shot will miss (arrows/stones) or the net will crumple
    /// (Easy-mode nets).  Read by the trajectory-preview renderer to
    /// swap the arc colour from cyan (default) to pink (crumpled).
    pub net_crumpled: bool,
    pub time_no_mouse_move: u32,
    pub mouse_map_prev: MapPoint,
    /// Rolling counter for the once-every-10-frames ground-mark drop
    /// performed by `DisplayTrajectory`.  Incremented each frame the
    /// trajectory-preview is valid.
    pub trajectory_mark_count: u16,
    /// Host-local destination markers emitted by the trajectory-preview
    /// hover path. Real move-command markers stay engine-owned; preview
    /// markers are per-seat UI feedback and must not affect rollback.
    pub trajectory_ground_mark: GroundMark,
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

    /// Gamepad / joystick state. Carries edge-detection, the QA macro
    /// timer, and the in-progress swordfight-gesture buffer across
    /// frames.  Updated from gilrs controller events each frame.
    pub gamepad: crate::gamepad::GamePadState,

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
    /// cleared every frame by `BringDownState`.  The sole consumer
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
    /// `BlitToMap` bake/restore surface pipeline. A queued `BlitToMap`
    /// inserts or replaces the entity's decal; a queued restore removes it.
    pub background_decals: HashMap<EntityId, BackgroundDecal>,
    /// Stable draw order for [`Self::background_decals`], preserving the
    /// order in which patch effects became permanent.
    pub background_decal_order: Vec<EntityId>,
}

#[derive(Default)]
pub struct HostTransport {
    pub local_seat: engine_player_command::PlayerId,
    pub net: Option<crate::multiplayer::NetChannels>,
    pub mission_seed: Option<u64>,
    pub mission_sim_config: Option<engine_api::SimConfig>,
    pub mission_id: Option<String>,
    pub reconnecting: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeferredAudioRequest {
    PlayDelayedSource(usize),
    ResumeAllSources,
    ActivateSource(usize),
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
}

/// Ordered, typed work emitted at the post-tick boundary. Variant-specific
/// drains preserve the existing host phase priority and simulation timing.
#[derive(Default)]
pub struct HostEffectBatches {
    modals: Vec<HostModalRequest>,
    signals: Vec<HostSignal>,
    pub background_blits: Vec<PendingBgBlit>,
}

impl HostEffectBatches {
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
#[derive(Default)]
pub struct Host {
    pub application_context: ApplicationContext,
    pub frontend: HostFrontend,
    pub transport: HostTransport,
    pub audio: HostAudio,
    pub effects: HostEffectBatches,
    pub scripting: HostScripting,
}

impl std::ops::Deref for Host {
    type Target = HostFrontend;

    fn deref(&self) -> &Self::Target {
        &self.frontend
    }
}

impl std::ops::DerefMut for Host {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.frontend
    }
}

impl Host {
    pub fn new(
        application_context: ApplicationContext,
        screen_width: f32,
        screen_height: f32,
    ) -> Self {
        let snapshot = application_context.host_snapshot().unwrap_or_else(|error| {
            panic!("Host construction requires a complete context: {error}")
        });
        Self {
            application_context,
            frontend: HostFrontend {
                viewport: ViewportState::new(screen_width, screen_height),
                input: InputState {
                    has_focus: true,
                    ..Default::default()
                },
                shipping: snapshot.shipping,
                key_config: snapshot.key_config,
                custom_key_config: snapshot.custom_key_config,
                control_tactical_units: snapshot.control_tactical_units,
                touch_camera_gestures: snapshot.touch_camera_gestures,
                native_refresh_presentation: snapshot.native_refresh_presentation,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Construct a host for deterministic replay/test paths which never read
    /// application persistence or shipping resources.
    pub fn scratch(screen_width: f32, screen_height: f32) -> Self {
        Self {
            application_context: ApplicationContext::default(),
            frontend: HostFrontend {
                viewport: ViewportState::new(screen_width, screen_height),
                input: InputState {
                    has_focus: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

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

    /// Clear persistent decals that belonged to the previous level.
    pub fn clear_background_decals(&mut self) {
        self.background_decals.clear();
        self.background_decal_order.clear();
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
        self.input = InputState::default();

        // Per-frame scratch flags that live host-side.
        self.valid_trajectory = false;
        self.host_titbit_preview = None;
        self.trajectory_ground_mark.clear();
        self.selected_view_element = None;
        // Restart the PC selection ring animation so every selected PC
        // comes back with a clean frame 0 (matches the pre-move
        // `Engine::restore` cleanup).
        self.selection_mark = engine_markers::SelectionMark::default();

        // Drop any UI-request queues that were in flight before the
        // load.  They live host-side now — accumulated from per-tick
        // `SideEffects.pending_*` by `Host::apply_side_effects`.
        self.effects.clear();
        self.pending_console_output.clear();
        self.pending_print_screen = None;
    }

    /// Apply the engine-local outputs of a tick.  Consumes the
    /// [`SideEffects`] struct by value so owned sub-vectors
    /// can be moved directly into host accumulators without clones.
    /// Returns the tick's game-state code.
    pub fn apply_side_effects(&mut self, fx: SideEffects) -> GameCode {
        if let Some(fade) = fx.fade_to_black {
            self.fade_to_black = fade;
        }
        if let Some(show) = fx.set_draw_hidden {
            self.input.draw_hidden = show;
        }
        if fx.invalidate_trajectory_preview {
            // `SelectAction` trajectory cleanup: clear the jumper and
            // jumped trajectories, the valid flag, and the projectile
            // arc.  We fold all four trajectory overlays (jump-line
            // preview, projectile arc, valid flag, crumpled-net tint)
            // into the single host-side preview since there is only
            // ever one visible arc; clearing them together here is an
            // immediate wipe before the next UpdateMouse frame.
            self.valid_trajectory = false;
            self.trajectory_preview_points.clear();
            self.trajectory_ground_mark.clear();
            self.trajectory_mark_count = 0;
            self.net_crumpled = false;
            self.host_titbit_preview = None;
        }
        if fx.reset_input {
            // MSG_RESET_INPUT clears the rubber-band selection flags
            // and suppresses any pending drag / click so a modal popup
            // / dialog entered from a sequence command doesn't leave
            // input state armed.  Also zeroes the per-frame modifier
            // cache and the swordfight mouse-way polyline (modifier
            // keys, drag, UI focus, info overlay, mouse-way).
            self.input.multi_selection_active = false;
            self.input.multi_unselection_active = false;
            self.input.draw_multi_selection = false;
            self.input.ignore_next_drag = false;
            self.input.ignore_next_left_click = false;
            self.input.is_dragging = false;
            self.input.is_alt = false;
            // Reset does the swap `info_displayed = fps_cheat;
            // fps_cheat = false`: the FPS-cheat flag is consumed and
            // promoted into `info_displayed`, so toggling the FPS
            // cheat arms the next reset to leave the debug-info
            // overlay visible.  The cheat flag lives on
            // `DevState::debug.fps_display`, which is not reachable
            // from here — hand off via a typed host signal for
            // the game-loop site that owns `&mut DevState` to apply.
            self.effects.request_signal(HostSignal::PromoteFpsCheat);
            self.ui_focus = false;
            self.mouse_way.clear();
            // Zero the no-mouse-move accumulator so the
            // hover-trajectory gate (`TIME_TRAJECTORY_DISPLAY`)
            // doesn't re-arm immediately after a modal dialog or task
            // switch.
            self.time_no_mouse_move = 0;
        }
        if fx.cancel_multi_selection {
            self.input.multi_selection_active = false;
            self.input.multi_unselection_active = false;
            self.input.draw_multi_selection = false;
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
                    if let Err(e) = mgr.save() {
                        tracing::warn!("failed to persist minimap position to profile: {e}");
                    }
                })
                .unwrap_or_else(|error| panic!("failed to persist minimap position: {error}"));
        }
        if fx.pending_swordfight_drag_ignore && self.input.is_dragging {
            // Selected PC left Swordfighting this tick; if a drag was
            // in flight, raise `IgnoreMouseEvent(true, true, true)` so
            // the drag doesn't bleed into a click-release or a
            // subsequent double-click.
            self.input.ignore_mouse_event(true, true, true);
        }
        self.skip_render = fx.skip_render;
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
                SoundCommand::SetListenPoint { .. } => {
                    // Local viewport state lives on Host now. The engine's
                    // shared cutscene-camera listener is still emitted for
                    // deterministic legacy plumbing, but native playback must
                    // use the actual viewport the player is looking at.
                    self.sync_sound_listener();
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
        self.ui_focus |= fx.ui_has_focus;
        // Per-frame mark requests from sim-side Mark() calls (currently
        // `RHScript::AddPCToMissionTeam` → `EngineCommand::MarkPc`).
        // Accumulates with host-side mark sources (requirements-bar
        // hover, portrait guard hover); the render loop drains the
        // buffer right after the outline pass.
        self.input.marked_pc_ids.extend(fx.pending_mark_pc_ids);
        // Patch-effect background decal changes are accumulated across
        // frames until the next render pass drains them.
        self.effects.background_blits.extend(fx.bg_blits);
        fx.code
    }

    pub fn sync_sound_listener(&mut self) {
        self.audio.sound.set_listen_point(
            self.viewport.sound_listen_point(),
            self.viewport.zoom_factor,
        );
    }

    pub fn install_trajectory_ground_mark_sprite(&mut self, data: &GroundMarkSpriteData) {
        self.trajectory_ground_mark.set_sprite_data(
            data.half_w,
            data.half_h,
            data.frame_sizes.clone(),
            data.per_frame_offsets.clone(),
        );
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

        let easy_host = Host::new(easy.clone(), 1024.0, 768.0);
        let hard_host = Host::new(hard.clone(), 1024.0, 768.0);

        assert_eq!(easy.sim_config().difficulty, DifficultyLevel::Easy);
        assert_eq!(hard.sim_config().difficulty, DifficultyLevel::Hard);
        assert_eq!(
            easy_host
                .key_config
                .get_binding("ZoomIn")
                .unwrap()
                .primary_key,
            Some(KeyCode::F2)
        );
        assert_eq!(
            hard_host
                .key_config
                .get_binding("ZoomIn")
                .unwrap()
                .primary_key,
            Some(KeyCode::F3)
        );
        assert!(
            easy_host
                .shipping
                .as_ref()
                .unwrap()
                .raw
                .contains_key("easy.marker")
        );
        assert!(
            !easy_host
                .shipping
                .as_ref()
                .unwrap()
                .raw
                .contains_key("hard.marker")
        );
        assert!(
            hard_host
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
        profiles.save().unwrap();

        let mut keys = KeyConfigStore::new(root_path.clone());
        keys.entry_or_default(0);
        keys.save().unwrap();
        let context = ApplicationContext::complete(
            engine_api::GlobalOptions::default(),
            profiles,
            keys,
            None,
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

        let (active_keys, custom_keys) = context.active_key_configs().unwrap();
        assert!(!active_keys.bindings.is_empty());
        assert!(!custom_keys.bindings.is_empty());
        context
            .with_key_configs(|store| {
                assert!(store.get(0).is_none());
                assert!(store.get(new_id).is_some());
            })
            .unwrap();

        let host = Host::new(context.clone(), 1280.0, 720.0);
        assert_eq!(host.key_config.key_type, active_keys.key_type);
        assert_eq!(host.custom_key_config.key_type, custom_keys.key_type);

        let mut saves = crate::savegame::SaveGameManager::open_for_context(&context);
        let slot = saves.create("First save".into(), 7);
        let expected_save_root = root.path().join("Profile_001");
        assert_eq!(
            std::path::Path::new(&saves.save_directory),
            expected_save_root
        );
        assert!(saves.save_path(slot).starts_with(&expected_save_root));
        assert!(!std::path::Path::new(&saves.save_directory).ends_with("Profile_000"));
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
                },
            )],
            vq_chunks: Vec::new(),
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
        let mut element = ElementData {
            kind: ElementKind::Fx,
            sprite: Sprite {
                current_width: 4,
                current_height: 1,
                scripts: Arc::new(vec![script]),
                center: SpriteAnchor::ZERO,
                ..Default::default()
            },
            ..Default::default()
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
        host.frame_holder = Arc::new(dictionary_frame_holder(INITIAL_NIGHT_COLOR));
        let published = host.publish_frame_holder_opacity();

        let mut assets = engine_api::LevelAssets::new();
        assets.pixel_opacity = Some(published.clone());
        let engine =
            engine_api::Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut assets)
                .expect("construct sprite-hit-test engine");
        let entity = dictionary_sprite_entity();
        let shadow_point = MapPoint::new(100.0, 100.0);
        let solid_point = MapPoint::new(101.0, 100.0);

        assert!(Arc::ptr_eq(&host.frame_holder, &published.snapshot()));
        assert!(!rendered_dictionary_pixel_is_opaque(
            &host.frame_holder,
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
        host.rebind_frame_holder_shadow_color(REBOUND_NIGHT_COLOR);

        assert!(Arc::ptr_eq(&host.frame_holder, &published.snapshot()));
        for variant in [SpriteVariant::Day, SpriteVariant::Night] {
            let renderer_shadow = rendered_dictionary_pixel_is_opaque(
                &host.frame_holder,
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
            &host.frame_holder,
            SpriteVariant::Day,
            REBOUND_NIGHT_COLOR,
            1,
        ));
        assert!(engine.is_point_on_sprite(&assets, &entity, solid_point, false));
        assert!(engine.is_point_on_sprite(&cloned_assets, &entity, solid_point, false));
    }
}
