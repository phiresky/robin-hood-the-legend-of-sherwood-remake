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
use robin_engine::sprite_variant::SpriteVariant;
#[cfg(test)]
use robin_engine::tactical_control::TacticalFormation;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::bg_cache::BackgroundDecals;
use crate::draw_manager::DrawManager;
use crate::key_config::KeyConfig;
use crate::pc_info_overlay::PcInfoOverlay;
use crate::sound::SoundManager;

const PANNEL_HEIGHT: f32 = engine_api::PANNEL_HEIGHT;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct QueueStripAnimation {
    pub previous_count: usize,
    pub fall_offset: i32,
}

/// Identity of the UI source, never an arbitrary representative group member.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum QueueStripIdentity {
    Pc(EntityId),
    AlliedGroup(u32),
    /// Unpinned selections have no persistent ID. Canonical membership keeps
    /// reordering stable without carrying easing into an unrelated selection.
    AlliedSelection(Vec<EntityId>),
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct QueueStripAnimations {
    seat: Option<engine_player_command::PlayerId>,
    entries: HashMap<QueueStripIdentity, QueueStripAnimation>,
}

impl QueueStripAnimations {
    pub(crate) fn clear(&mut self) {
        self.seat = None;
        self.entries.clear();
    }

    pub(crate) fn prepare_fixed_tick(
        &mut self,
        seat: engine_player_command::PlayerId,
        visible: impl IntoIterator<Item = (QueueStripIdentity, usize)>,
    ) {
        if self.seat != Some(seat) {
            self.clear();
            self.seat = Some(seat);
        }
        let mut remaining = std::mem::take(&mut self.entries);
        for (identity, count) in visible {
            let mut animation = remaining.remove(&identity).unwrap_or_default();
            animation.prepare_fixed_tick(count);
            assert!(
                self.entries.insert(identity, animation).is_none(),
                "visible queue strip identities must be unique"
            );
        }
        // Entries absent from the visible portrait set are retired, including
        // deleted groups, dead portraits and portraits paged off screen.
    }

    pub(crate) fn displayed_offset(
        &self,
        seat: engine_player_command::PlayerId,
        identity: &QueueStripIdentity,
        count: usize,
    ) -> i32 {
        if self.seat != Some(seat) {
            return 0; // First capture for this seat has no previous queue.
        }
        self.entries
            .get(identity)
            .map_or(0, |entry| entry.displayed_offset(count))
    }
}

impl QueueStripAnimation {
    pub(crate) fn prepare_fixed_tick(&mut self, count: usize) {
        self.fall_offset = if count < self.previous_count {
            10
        } else {
            self.fall_offset.saturating_sub(2).max(0)
        };
        self.previous_count = count;
    }

    /// An early thumbnail may observe a queue change before live preparation.
    /// Project its first collapse frame without advancing the live animation.
    pub(crate) fn displayed_offset(&self, count: usize) -> i32 {
        if count < self.previous_count {
            10
        } else {
            self.fall_offset
        }
    }
}

pub use crate::application::{
    ApplicationContext, ApplicationContextDiagnostic, ApplicationServicesDiagnostic,
    ReadyApplicationContext,
};

/// Owned host-facing snapshot copied out of an [`ApplicationContext`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HostContextSnapshot {
    pub(crate) shipping: Option<Arc<ShippingDatadir>>,
    pub(crate) preferences: FrontendPreferences,
}

/// A presentation-only projection of profile preferences. Applying it never
/// writes the running engine's sealed replay/multiplayer simulation config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FrontendPreferences {
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

/// Effects left to the live frame adapter, in its existing command/window order.
/// Disabled preferences are enforced even when already disabled at menu entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FrontendPreferenceEffects {
    pub(crate) cancel_planned_action: bool,
    pub(crate) native_refresh_presentation: bool,
    pub(crate) release_tactical_control: bool,
}

impl FrontendPreferences {
    pub(crate) fn new(
        key_config: KeyConfig,
        custom_key_config: KeyConfig,
        gameplay: robin_engine::gameplay_config::GameplayConfig,
        graphics: &robin_engine::graphic_config::GraphicConfig,
    ) -> Self {
        Self {
            key_config,
            custom_key_config,
            control_tactical_units: gameplay.control_tactical_units,
            plan_quick_actions: gameplay.plan_quick_actions,
            touch_camera_gestures: gameplay.touch_camera_gestures,
            native_refresh_presentation: graphics.native_refresh_presentation,
            quick_action_cursor_pulse: graphics.quick_action_cursor_pulse,
            diplomacy_visuals: graphics.diplomacy_visuals,
            gameplay_config: gameplay,
        }
    }

    pub(crate) fn apply(self, frontend: &mut HostFrontend) -> FrontendPreferenceEffects {
        frontend.planning.update_preference(self.plan_quick_actions);
        let effects = FrontendPreferenceEffects {
            cancel_planned_action: !frontend.planning.enabled(),
            native_refresh_presentation: self.native_refresh_presentation,
            release_tactical_control: !self.control_tactical_units,
        };
        frontend.preferences = self;
        effects
    }

    pub fn key_config(&self) -> &KeyConfig {
        &self.key_config
    }
    pub fn custom_key_config(&self) -> &KeyConfig {
        &self.custom_key_config
    }
    pub fn gameplay_config(&self) -> robin_engine::gameplay_config::GameplayConfig {
        self.gameplay_config
    }
    pub fn control_tactical_units(&self) -> bool {
        self.control_tactical_units
    }
    pub fn touch_camera_gestures(&self) -> bool {
        self.touch_camera_gestures
    }
    pub fn native_refresh_presentation(&self) -> bool {
        self.native_refresh_presentation
    }
    pub fn quick_action_cursor_pulse(&self) -> bool {
        self.quick_action_cursor_pulse
    }
    pub fn diplomacy_visuals(&self) -> bool {
        self.diplomacy_visuals
    }
}

impl ApplicationContext {
    pub(crate) fn host_snapshot(&self) -> Result<HostContextSnapshot, String> {
        let (key_config, custom_key_config) = self.active_key_configs()?;
        let preferences = self.with_active_profile(|profile| {
            FrontendPreferences::new(
                key_config,
                custom_key_config,
                profile.gameplay_config,
                &profile.graphic_config,
            )
        })?;
        Ok(HostContextSnapshot {
            shipping: self.shipping_arc()?,
            preferences,
        })
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

    /// Apply scripted motion from the local view. Merely locking input must
    /// not restore the stale shared view left by an earlier cutscene.
    pub fn advance_director_camera(
        &mut self,
        before: engine_api::DirectorCameraFrame,
        after: engine_api::DirectorCameraFrame,
        view_size: ScreenSize,
    ) {
        if !before.owns_view && !after.owns_view {
            return;
        }
        if before.zoom_factor != after.zoom_factor {
            // Script zooms retain the local focal point until a pan or jump
            // explicitly chooses a new one.
            self.zoom_by(after.zoom_factor / self.zoom_factor, None);
        }

        let target = after.slide_target.or_else(|| {
            before
                .slide_target
                .filter(|target| *target == after.view_position)
        });
        if let Some(target) = target {
            if self.zoom_factor != after.zoom_factor {
                self.zoom_by(after.zoom_factor / self.zoom_factor, None);
            }
            let distance = |point: MapPoint| (point.x - target.x).hypot(point.y - target.y);
            let remaining_before = distance(before.view_position);
            let progress = if remaining_before == 0.0 {
                1.0
            } else {
                (1.0 - distance(after.view_position) / remaining_before).clamp(0.0, 1.0)
            };
            // Use the shared pan's progress, but interpolate from the local
            // viewport. This preserves deterministic sequence completion and
            // makes every peer arrive at the scripted destination together.
            // TODO: a shared pan with zero distance has no duration to reuse;
            // presenting a local-only pan then needs a separate visual clock.
            self.old_view_position = self.view_position;
            let target_x =
                target.x + (view_size.x - self.screen_size.x) / (2.0 * after.zoom_factor);
            let target_y =
                target.y + (view_size.y - self.screen_size.y) / (2.0 * after.zoom_factor);
            self.view_position.x += (target_x - self.view_position.x) * progress;
            self.view_position.y += (target_y - self.view_position.y) * progress;
            self.clip_view();
        } else if before.view_position != after.view_position
            && before.zoom_factor == after.zoom_factor
        {
            // An explicit jump (including jump+unlock in one tick), or a
            // follow-camera update, still adopts the scripted framing.
            self.adopt_director_camera(after.view_position, view_size, after.zoom_factor);
        }
    }

    /// Mirror the shared script/director camera for an explicit placement.
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
///
/// Profile settings are an immutable projection, not independent cached knobs:
/// ```compile_fail,E0616
/// let mut frontend = robin_rs::host::HostFrontend::default();
/// frontend.preferences().control_tactical_units = true;
/// ```
/// Session planning policy cannot be replaced by an input consumer:
/// ```compile_fail,E0616
/// let mut frontend = robin_rs::host::HostFrontend::default();
/// frontend.planning = Default::default();
/// ```
/// Draw-side diagnostics do not grant a sampling clock:
/// ```compile_fail,E0596
/// let frontend = robin_rs::host::HostFrontend::default();
/// frontend.diagnostics().record_frame(100, 0);
/// ```
#[derive(Default)]
pub struct HostFrontend {
    /// Mission assets survive snapshot replacement, but retire with the mission.
    pub resources: FrontendResources,
    /// Host-local render state, advanced by named tick/render phases.
    pub presentation: FrontendPresentation,
    pub viewport: ViewportState,

    /// Independently owned controls, gestures, spatial hits, and cursor feedback.
    pub input: InputState,
    /// Paired pointer events and gesture trail retire together.
    pointer_sequence: crate::frontend_input::FrontendPointerSequence,
    /// Atomic profile projection, never a simulation or replay input.
    preferences: FrontendPreferences,
    /// Preference plus sticky touch mode and non-overridable session policy.
    planning: crate::frontend_input::FrontendPlanning,
    /// Cosmetic fixed-tick easing, invalidated when the entity timeline changes.
    queue_strip_animations: QueueStripAnimations,
    /// Entity-bound targeting/hover feedback, invalidated at interaction boundaries.
    interaction: FrontendInteraction,
    /// Observations survive snapshot loads; pending console output does not.
    diagnostics: crate::frontend_diagnostics::FrontendDiagnostics,
    /// Single-slot deferred capture. Drained after render_frame, before present,
    /// to write the composited frame as screen%03u.png in the save directory.
    /// Ctrl requests a wide snapshot; Shift applies the historical 3x3 median
    /// filter. Only named queue/consume operations expose this pending request.
    pending_print_screen: Option<PrintScreenRequest>,
    /// Physical DisplayMap shortcut, retained with the host's local preferences.
    pub minimap_fast_key: Option<winit::keyboard::KeyCode>,
    /// Local MSG_SLOW_MOTION pacing toggle (Pause by default). Multiplies the
    /// 40 ms frame target by ten unless console or engine fast-forward is active.
    /// Neither snapshot state nor deterministic input; survives save restoration.
    pub slow_motion: bool,
}

/// Mission-owned decoded and uploaded resources. Snapshot restoration deliberately
/// leaves these intact. GPU retirement requires the originating renderer; a new
/// mission receives a fresh owner rather than reusing the previous mission's banks.
#[derive(Default, Serialize)]
pub struct FrontendResources {
    pub(crate) mission_surfaces: crate::mission_render_resources::MissionRenderResources,
    /// Decoded sprite bank, host-only because FrameHolder is an asset-layer type.
    /// Immutable Arc generations keep clones cheap and synchronize sprite drawing
    /// with the engine's pixel-opacity reader.
    #[serde(skip)]
    frame_holder: Arc<FrameHolder>,
    /// Published only after variant generation and the initial Arno-law bind.
    /// Runtime ambiance changes publish a new immutable generation through this
    /// handle so cloned LevelAssets never retain a detached COW dictionary.
    #[serde(skip)]
    frame_holder_opacity: Option<Arc<PublishedFrameHolder>>,
    /// Resource layout bound at Host construction; snapshots cannot replace it.
    #[serde(skip)]
    pub shipping: Option<Arc<ShippingDatadir>>,
    /// Persistent FX-entity decals replacing map-patch bake/restore surfaces.
    /// Replacement keeps draw position; removal preserves survivor order;
    /// reinsertion appends. Level loading explicitly clears the previous level.
    pub(crate) background_decals: BackgroundDecals,
}

impl<'de> Deserialize<'de> for FrontendResources {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "frontend resources require mission preparation",
        ))
    }
}

/// Presentation state is not simulation state. Save restoration updates the
/// display machine through Engine::restore_from_snapshot and retires only the
/// selection animation here; camera, overlays and script fades retain their
/// historical lifetimes instead of being blanket-defaulted.
#[derive(Default, Serialize)]
pub struct FrontendPresentation {
    pub engine_display: engine_api::HostDisplayState,
    /// Back-to-front entity order, computed from Engine::compute_display_order
    /// once per frame after the tick and before input dispatch and rendering.
    /// Render iteration, focus hit-testing and titbit Z-flush share this cache.
    /// Derived host state: never serialized or included in simulation hashes.
    #[serde(skip)]
    pub draw_order: DrawOrder,
    /// PC-selection-ring ping-pong animation. Advanced once per fixed tick under
    /// the same should_run_hourglass gate as simulation, so pause and console
    /// freeze it. Only SelectionMarkRenderer reads this cosmetic phase.
    #[serde(skip)]
    pub selection_mark: engine_markers::SelectionMark,
    /// Immediate drawing helper synchronized from the viewport before rendering.
    pub draw_manager: DrawManager,
    pub pc_info_overlay: PcInfoOverlay,
    /// FADE_TO_BLACK pixel ramp: alpha rises from 0 to 255 over speed frames,
    /// then falls back over the next speed frames. Advanced only at the live
    /// presentation boundary, never by screenshot or thumbnail drawing.
    pub fade_to_black: Option<FadeToBlack>,
    /// Last tick's SideEffects.skip_render; fast-forward can skip the GPU pass.
    pub skip_render: bool,
}

impl<'de> Deserialize<'de> for FrontendPresentation {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "frontend presentation is rebuilt by live phases",
        ))
    }
}

impl FrontendPresentation {
    fn restore_snapshot(&mut self) {
        // The restored display machine is installed by the snapshot adapter.
        // Draw order is recomputed by frame preparation before input/rendering.
        // Preserve overlay/fade and draw-helper state, as the historical load
        // path does; only selection's cosmetic phase restarts at this boundary.
        self.selection_mark = engine_markers::SelectionMark::default();
    }
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

/// Private owner of entity-bound, transient interaction feedback. No mutable
/// projection exposes the collection: callers must use the matching operation.
/// Diagnostic serialization is allowed, but restoring feedback without its
/// live input sequence and mission entity identities would be invalid.
#[derive(Default)]
struct FrontendInteraction {
    tactical_targeting: crate::frontend_targeting::TacticalTargeting,
    trajectory_preview: crate::frontend_preview::FrontendTrajectoryPreview,
    /// Alt-hover vision cone and console target; never simulation state.
    selected_view_element: Option<EntityId>,
    item_effect_preview: Option<ItemEffectPreview>,
    host_titbit_preview: Option<HostTitbitPreview>,
    gesture_coach_feedback: Option<crate::mouse_way::GestureCoachFeedback>,
}

impl Serialize for FrontendInteraction {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("FrontendInteraction", 6)?;
        state.serialize_field("tactical_targeting", &self.tactical_targeting)?;
        state.serialize_field("trajectory_preview", &self.trajectory_preview)?;
        state.serialize_field("selected_view_element", &self.selected_view_element)?;
        state.serialize_field("has_item_effect", &self.item_effect_preview.is_some())?;
        state.serialize_field("has_titbit", &self.host_titbit_preview.is_some())?;
        state.serialize_field(
            "has_gesture_feedback",
            &self.gesture_coach_feedback.is_some(),
        )?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for FrontendInteraction {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "interaction feedback requires a live mission/input sequence",
        ))
    }
}

impl FrontendInteraction {
    fn reset(&mut self, reason: InteractionReset) {
        self.tactical_targeting.cancel();
        self.trajectory_preview.reset_after_restore();
        self.item_effect_preview = None;
        self.host_titbit_preview = None;
        self.gesture_coach_feedback = None;
        if reason == InteractionReset::SnapshotRestored {
            self.selected_view_element = None;
        }
    }

    fn invalidate_action(&mut self) {
        self.trajectory_preview.invalidate_action();
        self.host_titbit_preview = None;
    }
}

impl HostFrontend {
    /// Queue one capture of this live mission; a newer shortcut replaces the
    /// previous unconsumed request, matching the original single-slot behavior.
    pub fn request_print_screen(&mut self, request: PrintScreenRequest) {
        self.pending_print_screen = Some(request);
    }

    pub fn take_print_screen(&mut self) -> Option<PrintScreenRequest> {
        self.pending_print_screen.take()
    }

    pub fn take_wide_snapshot_request(&mut self) -> bool {
        if self.pending_print_screen == Some(PrintScreenRequest::WideSnapshot) {
            self.pending_print_screen = None;
            true
        } else {
            false
        }
    }

    /// One snapshot boundary for input and deferred frontend requests. Resource
    /// handles, camera pose, preferences and diagnostics history remain live.
    fn restore_snapshot(&mut self) {
        self.reset_interaction(InteractionReset::SnapshotRestored);
        self.diagnostics.clear_console_output();
        self.pending_print_screen = None;
    }

    /// Called on both success and failure when leaving an interactive mission.
    /// The owning Host is then consumed; only renderer uploads need explicit
    /// retirement, while input and pending output can no longer cross missions.
    pub(crate) fn retire_mission(&mut self, renderer: &mut crate::renderer::Renderer) {
        self.resources.retire(renderer);
        self.restore_snapshot();
    }

    pub fn selected_view_element(&self) -> Option<EntityId> {
        self.interaction.selected_view_element
    }
    pub fn set_selected_view_element(&mut self, selected: Option<EntityId>) {
        self.interaction.selected_view_element = selected;
    }
    pub fn trajectory_preview(&self) -> &crate::frontend_preview::FrontendTrajectoryPreview {
        &self.interaction.trajectory_preview
    }
    pub(crate) fn apply_trajectory_preview(
        &mut self,
        preview: engine_api::input::TrajectoryPreview,
    ) {
        self.interaction.trajectory_preview.apply(preview);
    }
    pub(crate) fn reject_trajectory_hit(&mut self) {
        self.interaction.trajectory_preview.reject_hit();
    }
    pub(crate) fn apply_trajectory_crumple_prediction(&mut self, predicted: bool) {
        self.interaction
            .trajectory_preview
            .apply_crumple_prediction(predicted);
    }
    /// Begin a hover observation by retiring last frame's explanations. Geometry
    /// and the hover timer retain their original, independently scoped lifetime.
    pub(crate) fn observe_hover_feedback(
        &mut self,
        shift: bool,
        action: robin_engine::profiles::Action,
        mouse: MapPoint,
    ) {
        self.interaction.host_titbit_preview = None;
        self.interaction.item_effect_preview = None;
        self.interaction
            .trajectory_preview
            .observe_hover(shift, action, mouse);
    }
    pub(crate) fn advance_hover_markers(&mut self, display_delay: u32) {
        self.interaction
            .trajectory_preview
            .advance_hover_markers(display_delay);
    }
    pub(crate) fn tick_trajectory_marks(
        &mut self,
        view: geo::Coord<f32>,
        zoom: f32,
        width: i32,
        height: i32,
        frame: u32,
    ) {
        self.interaction
            .trajectory_preview
            .tick_marks(view, zoom, width, height, frame);
    }
    pub fn tactical_targeting(&self) -> &crate::frontend_targeting::TacticalTargeting {
        &self.interaction.tactical_targeting
    }
    pub fn arm_tactical_patrol(
        &mut self,
        soldiers: Vec<EntityId>,
        formation: robin_engine::tactical_control::TacticalFormation,
    ) {
        self.interaction
            .tactical_targeting
            .arm_patrol(soldiers, formation);
    }
    pub fn resolve_tactical_target(
        &mut self,
        destination: MapPoint,
    ) -> Option<engine_player_command::PlayerCommand> {
        self.interaction
            .tactical_targeting
            .resolve_world_click(destination)
    }
    pub fn cancel_tactical_target(&mut self) -> bool {
        self.interaction.tactical_targeting.cancel()
    }
    pub fn item_effect_preview(&self) -> Option<ItemEffectPreview> {
        self.interaction.item_effect_preview
    }
    pub fn set_item_effect_preview(&mut self, preview: Option<ItemEffectPreview>) {
        self.interaction.item_effect_preview = preview;
    }
    pub fn host_titbit_preview(&self) -> Option<HostTitbitPreview> {
        self.interaction.host_titbit_preview
    }
    pub fn set_host_titbit_preview(&mut self, preview: Option<HostTitbitPreview>) {
        self.interaction.host_titbit_preview = preview;
    }
    pub fn gesture_coach_feedback(&self) -> Option<crate::mouse_way::GestureCoachFeedback> {
        self.interaction.gesture_coach_feedback
    }
    pub fn set_gesture_coach_feedback(
        &mut self,
        feedback: Option<crate::mouse_way::GestureCoachFeedback>,
    ) {
        self.interaction.gesture_coach_feedback = feedback;
    }
    pub(crate) fn queue_strip_animations(&self) -> &QueueStripAnimations {
        &self.queue_strip_animations
    }

    pub(crate) fn prepare_queue_strip_animations(
        &mut self,
        seat: engine_player_command::PlayerId,
        visible: impl IntoIterator<Item = (QueueStripIdentity, usize)>,
    ) {
        self.queue_strip_animations
            .prepare_fixed_tick(seat, visible);
    }

    /// Profile settings can only be replaced as one projection. Callers cannot
    /// update a cached scalar independently from the selected profile.
    pub fn preferences(&self) -> &FrontendPreferences {
        &self.preferences
    }
    pub fn diagnostics(&self) -> &crate::frontend_diagnostics::FrontendDiagnostics {
        &self.diagnostics
    }
    pub fn diagnostics_mut(&mut self) -> &mut crate::frontend_diagnostics::FrontendDiagnostics {
        &mut self.diagnostics
    }
    pub fn planning(&self) -> &crate::frontend_input::FrontendPlanning {
        &self.planning
    }
    pub fn force_planning_off_for_session(&mut self) {
        self.planning.force_off_for_session();
    }
    pub fn cancel_touch_planning(&mut self) {
        self.planning.cancel_touch();
    }
    pub fn pointer_capture(&self) -> &crate::frontend_input::FrontendPointerCapture {
        self.pointer_sequence.capture()
    }
    pub fn mouse_way(&self) -> &crate::mouse_way::MouseWay {
        self.pointer_sequence.mouse_way()
    }
    pub fn add_gesture_point(&mut self, point: ScreenPoint) {
        self.pointer_sequence.add_point(point);
    }
    pub fn clear_gesture(&mut self) {
        self.pointer_sequence.clear_gesture();
    }
    pub fn advance_gesture_trail(&mut self, trail: &crate::mouse_trail::MouseTrailRenderer) {
        self.pointer_sequence.advance_trail(trail);
    }
    pub fn begin_left_pointer(&mut self, point: ScreenPoint, clicks: u8) {
        self.pointer_sequence
            .begin_left(&mut self.input, point, clicks);
    }
    pub fn release_left_pointer(&mut self) -> bool {
        self.pointer_sequence.release_left(&mut self.input)
    }
    pub fn begin_right_pointer(&mut self, clicks: u8) {
        self.pointer_sequence.begin_right(&mut self.input, clicks);
    }
    pub fn release_right_pointer(&mut self) -> bool {
        self.pointer_sequence.release_right(&mut self.input)
    }
    pub fn cancel_left_pointer(&mut self) {
        self.pointer_sequence.cancel_left(&mut self.input);
    }
    pub fn begin_minimap_drag(&mut self, camera: bool) {
        self.pointer_sequence.begin_minimap_drag(camera);
    }
    pub fn end_minimap_drag(&mut self) {
        self.pointer_sequence.end_minimap_drag();
    }
    pub fn route_hud_event(&mut self, event: &crate::gfx_types::GameEvent, hit: bool) -> bool {
        self.pointer_sequence.route_hud_event(event, hit)
    }
    /// Modal entry disarms the current drag without inventing a release.
    pub fn reset_modal_input(&mut self) {
        self.pointer_sequence.reset_modal(&mut self.input);
        self.viewport.cancel_touch_motion();
    }
    pub fn lose_pointer_focus(&mut self) {
        self.reset_pointer_sequence();
        // Unlike modal closure, focus loss abandons any pending release.
        self.input.cancel_left_pointer();
        self.interaction.invalidate_action();
    }
    /// Route a paired touch gesture against the current session planning policy.
    /// Keeping both owners borrowed here prevents a caller replacing that policy
    /// while preserving stale capture metadata.
    pub fn route_touch_plan_event(
        &mut self,
        event: &crate::gfx_types::GameEvent,
        admit_touch: bool,
        hit_test: impl FnOnce(i32, i32) -> bool,
    ) -> crate::frontend_input::TouchPlanRoute {
        self.pointer_sequence.route_touch_plan_event(
            &mut self.planning,
            event,
            admit_touch,
            hit_test,
        )
    }

    pub fn reset_interaction(&mut self, reason: InteractionReset) {
        self.reset_pointer_sequence();
        self.interaction.reset(reason);
        if reason == InteractionReset::SnapshotRestored {
            self.input = InputState::default();
            self.planning.cancel_touch();
            self.queue_strip_animations.clear();
            self.presentation.restore_snapshot();
        }
    }

    fn reset_pointer_sequence(&mut self) {
        self.pointer_sequence.reset(&mut self.input);
        self.input.gestures.portrait_action_countdown = 0;
        self.input.gestures.portrait_action_pc = None;
        self.viewport.cancel_touch_motion();
    }
}

/// Mission-scoped transport authority. Admission installs the channel owner and
/// construction metadata together; later events can only advance named states.
///
/// ```compile_fail,E0616
/// let mut transport = robin_rs::host::HostTransport::default();
/// transport.net = None;
/// ```
///
/// ```compile_fail,E0616
/// let mut transport = robin_rs::host::HostTransport::default();
/// transport.local_seat = robin_engine::player_command::PlayerId::HOST;
/// ```
#[derive(Default)]
pub struct HostTransport {
    local_seat: engine_player_command::PlayerId,
    net: Option<crate::multiplayer::NetChannels>,
    mission_seed: Option<u64>,
    mission_sim_config: Option<engine_api::SimConfig>,
    speech_timing_locale: Option<String>,
    mission_id: Option<String>,
    synchronization: TransportSynchronization,
    /// Verified full-mod bytes, VFS overlays, and cache lease for a
    /// host-distributed mission. Field order makes the network runtime stop
    /// before this mount is dropped with the enclosing transport.
    #[cfg(feature = "multiplayer")]
    distributed_mod: Option<crate::distributed_mod_admission::AdmittedDistributedMod>,
    /// Delayed Sherwood command boundary belongs to this transport lifetime.
    pending_campaign_exit: Option<crate::main_entry::PendingMultiplayerCampaignExit>,
}

/// Prepared transitions always hold simulation. Consuming their committed
/// payload keeps that hold until the replacement mission releases BeginSim.
#[derive(Default)]
enum TransportSynchronization {
    #[default]
    Running,
    AwaitingSnapshot,
    Prepared(PendingSnapshotTransition),
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
pub(crate) struct CommittedSnapshotTransition(PendingSnapshotTransition);

impl serde::Serialize for CommittedSnapshotTransition {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Diagnostic serialization deliberately carries no payload or authority.
        serializer.serialize_unit_struct("CommittedSnapshotTransition")
    }
}

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
    pub fn local_seat(&self) -> engine_player_command::PlayerId {
        self.local_seat
    }
    pub fn net(&self) -> Option<&crate::multiplayer::NetChannels> {
        self.net.as_ref()
    }
    pub fn mission_seed(&self) -> Option<u64> {
        self.mission_seed
    }
    pub fn mission_sim_config(&self) -> Option<engine_api::SimConfig> {
        self.mission_sim_config
    }
    pub fn speech_timing_locale(&self) -> Option<&str> {
        self.speech_timing_locale.as_deref()
    }
    pub fn mission_id(&self) -> Option<&str> {
        self.mission_id.as_deref()
    }
    pub fn reconnecting(&self) -> bool {
        !matches!(self.synchronization, TransportSynchronization::Running)
    }
    pub fn has_snapshot_transition(&self) -> bool {
        matches!(self.synchronization, TransportSynchronization::Prepared(_))
    }

    /// Install one fully validated session before engine construction. Metadata
    /// and identity cannot be partially replaced by a subsequent Welcome.
    #[cfg(any(feature = "multiplayer", test))]
    pub(crate) fn install_session(
        &mut self,
        net: crate::multiplayer::NetChannels,
        seat: engine_player_command::PlayerId,
        mission_id: String,
        seed: u64,
        config: engine_api::SimConfig,
        speech_locale: Option<String>,
    ) {
        assert!(
            self.net.is_none(),
            "cannot overwrite a live multiplayer session"
        );
        assert!(
            !self.has_snapshot_transition(),
            "cannot install a session during a snapshot transition"
        );
        self.local_seat = seat;
        self.mission_id = Some(mission_id);
        self.mission_seed = Some(seed);
        self.mission_sim_config = Some(config);
        self.speech_timing_locale = speech_locale;
        self.net = Some(net);
        self.synchronization = TransportSynchronization::Running;
    }

    pub(crate) fn confirm_local_seat(&self, seat: engine_player_command::PlayerId) {
        assert_eq!(
            self.local_seat, seat,
            "assigned seat changed after session admission"
        );
    }

    /// Hold simulation without losing the admitted seat, campaign, or runtime.
    pub(crate) fn await_authoritative_snapshot(&mut self) {
        if !self.has_snapshot_transition() {
            self.synchronization = TransportSynchronization::AwaitingSnapshot;
        }
    }
    pub(crate) fn begin_simulation(&mut self) {
        assert!(
            !self.has_snapshot_transition(),
            "ordinary snapshot cannot complete a prepared transition"
        );
        self.synchronization = TransportSynchronization::Running;
    }
    pub(crate) fn prepare_snapshot_transition(&mut self, pending: PendingSnapshotTransition) {
        assert!(
            !self.has_snapshot_transition(),
            "snapshot transition already prepared"
        );
        self.synchronization = TransportSynchronization::Prepared(pending);
    }
    pub(crate) fn commit_snapshot_transition(
        &mut self,
        id: robin_engine::multiplayer::SnapshotTransitionId,
    ) -> Result<(), String> {
        match &mut self.synchronization {
            TransportSynchronization::Prepared(pending) => pending.commit_authenticated(id),
            _ => Err("snapshot transition commit has no prepared payload".into()),
        }
    }
    #[cfg(feature = "multiplayer")]
    pub(crate) fn retain_distributed_mod(
        &mut self,
        admitted: crate::distributed_mod_admission::AdmittedDistributedMod,
    ) {
        assert!(
            self.distributed_mod.is_none(),
            "distributed mission content already admitted"
        );
        self.distributed_mod = Some(admitted);
    }
    pub(crate) fn defer_campaign_exit(
        &mut self,
        pending: crate::main_entry::PendingMultiplayerCampaignExit,
    ) {
        assert!(
            self.pending_campaign_exit.is_none(),
            "campaign exit already pending"
        );
        self.pending_campaign_exit = Some(pending);
    }
    pub(crate) fn pending_campaign_exit(
        &self,
    ) -> Option<&crate::main_entry::PendingMultiplayerCampaignExit> {
        self.pending_campaign_exit.as_ref()
    }
    pub(crate) fn take_campaign_exit_at(
        &mut self,
        frame: u32,
    ) -> Option<crate::main_entry::PendingMultiplayerCampaignExit> {
        if self
            .pending_campaign_exit
            .as_ref()
            .is_some_and(|pending| frame >= pending.not_before_frame)
        {
            self.pending_campaign_exit.take()
        } else {
            None
        }
    }
    pub(crate) fn preserve_session_for_next_mission(&mut self) {
        if let Some(net) = self.net.as_mut() {
            net.preserve_session_for_next_mission();
        }
    }
    #[cfg(test)]
    pub(crate) fn test_session(
        net: crate::multiplayer::NetChannels,
        seat: engine_player_command::PlayerId,
    ) -> Self {
        Self {
            net: Some(net),
            local_seat: seat,
            ..Self::default()
        }
    }
    #[cfg(test)]
    pub(crate) fn test_local_seat(&mut self, seat: engine_player_command::PlayerId) {
        self.local_seat = seat;
    }
    #[cfg(test)]
    pub(crate) fn test_drop_channels(&mut self) {
        self.net = None;
    }

    pub fn authoritative_transition_actions_enabled(&self) -> bool {
        !self.reconnecting() && self.local_seat == robin_engine::player_command::PlayerId::HOST
    }

    pub(crate) fn take_committed_snapshot_transition(
        &mut self,
    ) -> Option<CommittedSnapshotTransition> {
        if !matches!(&self.synchronization, TransportSynchronization::Prepared(pending) if pending.committed)
        {
            return None;
        }
        let TransportSynchronization::Prepared(pending) = std::mem::replace(
            &mut self.synchronization,
            TransportSynchronization::AwaitingSnapshot,
        ) else {
            unreachable!("checked committed preparation")
        };
        Some(CommittedSnapshotTransition(pending))
    }
}

#[cfg(test)]
mod transport_lifecycle_tests {
    use super::*;
    use robin_engine::multiplayer::{MultiplayerSessionId, SnapshotTransitionId};
    use robin_engine::player_command::PlayerId;

    fn installed(seat: PlayerId) -> HostTransport {
        let (channels, _incoming, _outgoing, _, _) = crate::multiplayer::NetChannels::new();
        let mut transport = HostTransport::default();
        transport.install_session(
            channels,
            seat,
            "leicester".into(),
            42,
            engine_api::SimConfig::default(),
            Some("en".into()),
        );
        transport
    }

    fn transition_id() -> SnapshotTransitionId {
        SnapshotTransitionId {
            session_id: MultiplayerSessionId([8; 32]),
            sequence: 1,
        }
    }

    fn prepare(transport: &mut HostTransport) {
        transport.prepare_snapshot_transition(PendingSnapshotTransition::new(
            transition_id(),
            PendingSnapshotTransitionPayload::CampaignExit {
                exit_code: robin_engine::game_operation::GameCode::LevelInterrupted,
                engine: None,
            },
        ));
    }

    #[test]
    fn reconnect_retains_admitted_identity_metadata_and_channels() {
        let mut transport = installed(PlayerId(2));
        let channel_address = transport.net().unwrap() as *const _;
        transport.await_authoritative_snapshot();
        assert!(transport.reconnecting());
        assert!(!transport.authoritative_transition_actions_enabled());
        assert_eq!(transport.local_seat(), PlayerId(2));
        assert_eq!(transport.mission_id(), Some("leicester"));
        assert_eq!(transport.mission_seed(), Some(42));
        assert_eq!(
            transport.mission_sim_config(),
            Some(engine_api::SimConfig::default())
        );
        assert_eq!(transport.speech_timing_locale(), Some("en"));
        assert_eq!(transport.net().unwrap() as *const _, channel_address);
        transport.confirm_local_seat(PlayerId(2));
        transport.begin_simulation();
        assert!(!transport.reconnecting());
        assert!(
            !transport.authoritative_transition_actions_enabled(),
            "a resumed peer never becomes the host"
        );
    }

    #[test]
    fn committed_payload_is_consumed_once_and_keeps_simulation_held() {
        let mut transport = installed(PlayerId::HOST);
        prepare(&mut transport);
        assert!(transport.take_committed_snapshot_transition().is_none());
        assert!(
            transport
                .commit_snapshot_transition(SnapshotTransitionId {
                    sequence: 2,
                    ..transition_id()
                })
                .is_err()
        );
        transport.await_authoritative_snapshot();
        assert!(
            transport.has_snapshot_transition(),
            "disconnect cannot discard an authenticated preparation"
        );
        transport
            .commit_snapshot_transition(transition_id())
            .unwrap();
        assert!(
            transport
                .commit_snapshot_transition(transition_id())
                .is_err()
        );
        assert_eq!(
            transport.take_committed_snapshot_transition().unwrap().id(),
            transition_id()
        );
        assert!(transport.take_committed_snapshot_transition().is_none());
        assert!(!transport.has_snapshot_transition());
        assert!(transport.reconnecting());
        assert!(!transport.authoritative_transition_actions_enabled());
        transport.begin_simulation();
        assert!(transport.authoritative_transition_actions_enabled());
    }

    #[test]
    #[should_panic(expected = "ordinary snapshot cannot complete a prepared transition")]
    fn ordinary_barrier_cannot_discard_prepared_payload() {
        let mut transport = installed(PlayerId::HOST);
        prepare(&mut transport);
        transport.begin_simulation();
    }

    #[test]
    #[should_panic(expected = "assigned seat changed after session admission")]
    fn late_assignment_cannot_rewrite_admitted_seat() {
        installed(PlayerId(2)).confirm_local_seat(PlayerId::HOST);
    }

    #[test]
    fn losing_test_channels_does_not_promote_a_waiting_peer() {
        let mut transport = installed(PlayerId(2));
        transport.await_authoritative_snapshot();
        transport.test_drop_channels();
        assert_eq!(transport.local_seat(), PlayerId(2));
        assert!(!transport.authoritative_transition_actions_enabled());
        transport.begin_simulation();
        assert!(
            !transport.authoritative_transition_actions_enabled(),
            "missing channels cannot turn a former client into a single-player host"
        );
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
/// frontend.resources.clear_background_decals();
/// robin_rs::blit_to_map::drain_pending_bg_blits(frontend, effects);
/// ```
///
/// ```compile_fail,E0609
/// use robin_rs::host::Host;
/// let mut host = Host::scratch(1024.0, 768.0);
/// host.input.controls.has_focus = false;
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
    /// Drawing cannot advance frontend state or escape into application services.
    pub(crate) fn draw(&self) -> HostDraw<'_> {
        HostDraw {
            frontend: self.frontend,
            sound: self.sound,
            options: self.options,
            local_seat: self.local_seat,
            graphic_config: self.graphic_config(),
            viewport: &self.frontend.viewport,
            draw_manager: self.frontend.presentation.draw_manager.clone(),
        }
    }

    /// Read only the active presentation settings, without granting storage,
    /// profile mutation, asset preparation, or other application authority.
    pub(crate) fn graphic_config(&self) -> robin_engine::graphic_config::GraphicConfig {
        self.application
            .with_active_profile(|profile| profile.graphic_config.clone())
            .unwrap_or_else(|error| panic!("rendering requires an active profile: {error}"))
    }
}

/// Immutable gameplay presentation inputs; GPU command buffers remain separately
/// mutable in the renderer. Serialization is diagnostic-only and cannot
/// reconstruct borrowed frontend authority.
pub(crate) struct HostDraw<'a> {
    viewport: &'a ViewportState,
    draw_manager: crate::draw_manager::DrawManager,
    pub(crate) frontend: &'a HostFrontend,
    pub(crate) sound: &'a crate::sound::SoundManager,
    pub(crate) options: &'a engine_api::GlobalOptions,
    pub(crate) local_seat: robin_engine::player_command::PlayerId,
    graphic_config: robin_engine::graphic_config::GraphicConfig,
}

impl Serialize for HostDraw<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // No live frontend, sound, options, or application state escapes.
        serializer.serialize_unit_struct("HostDraw")
    }
}

impl<'de> Deserialize<'de> for HostDraw<'_> {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "draw authority must be borrowed from the live presentation host",
        ))
    }
}

impl HostDraw<'_> {
    pub(crate) fn viewport(&self) -> &ViewportState {
        self.viewport
    }

    pub(crate) fn draw_manager(&self) -> &crate::draw_manager::DrawManager {
        &self.draw_manager
    }

    pub(crate) fn with_viewport<'a>(&'a self, viewport: &'a ViewportState) -> HostDraw<'a> {
        let mut draw_manager = self.draw_manager.clone();
        let view = viewport.view_position;
        let screen = viewport.screen_size;
        let zoom = viewport.zoom_factor;
        assert!(zoom > 0.0, "capture viewport requires a positive zoom");
        draw_manager.update_drawing_parameters(
            0,
            robin_engine::coordinates::MapBBox::from_coords(
                view.x,
                view.y,
                view.x + (screen.x - 1.0) / zoom,
                view.y + (screen.y - engine_api::PANNEL_HEIGHT + 1.0) / zoom,
            ),
            zoom,
        );
        HostDraw {
            frontend: self.frontend,
            sound: self.sound,
            options: self.options,
            local_seat: self.local_seat,
            graphic_config: self.graphic_config.clone(),
            viewport,
            draw_manager,
        }
    }

    pub(crate) fn graphic_config(&self) -> robin_engine::graphic_config::GraphicConfig {
        self.graphic_config.clone()
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
        let mut frontend = HostFrontend {
            viewport: ViewportState::new(screen_width, screen_height),
            input: InputState::focused(),
            resources: FrontendResources {
                shipping: snapshot.shipping,
                ..Default::default()
            },
            ..Default::default()
        };
        // Startup has no pending world commands or attached presentation window.
        // The live options adapter performs the returned transition effects.
        snapshot.preferences.apply(&mut frontend);
        Ok(Self {
            application_context: application_context.into(),
            frontend,
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
        self.frontend.restore_snapshot();

        // Drop any UI-request queues that were in flight before the
        // load.  They live host-side now — accumulated from per-tick
        // `SideEffects.pending_*` by `Host::apply_side_effects`.
        self.effects.clear();
    }

    /// Apply engine outputs using only the frontend, audio and effect queues.
    pub fn apply_side_effects(&mut self, fx: SideEffects) -> GameCode {
        self.frontend.apply_side_effects(
            fx,
            &mut self.audio,
            &mut self.effects,
            &self.application_context,
            self.transport.local_seat(),
        )
    }

    pub fn sync_sound_listener(&mut self) {
        self.audio.sound.set_listen_point(
            self.frontend.viewport.sound_listen_point(),
            self.frontend.viewport.zoom_factor,
        );
    }
}

impl HostFrontend {
    /// Apply the engine-local outputs of a tick.  Consumes the
    /// [`SideEffects`] struct by value so owned sub-vectors
    /// can be moved directly into host accumulators without clones.
    /// Returns the tick's game-state code.
    pub(crate) fn apply_side_effects(
        &mut self,
        fx: SideEffects,
        audio: &mut HostAudio,
        effects: &mut HostEffectBatches,
        application_context: &ApplicationContext,
        local_seat: engine_player_command::PlayerId,
    ) -> GameCode {
        if let Some(fade) = fx.fade_to_black {
            self.presentation.fade_to_black = fade;
        }
        if let Some(show) = fx.set_draw_hidden {
            self.input.feedback.draw_hidden = show;
        }
        if fx.invalidate_trajectory_preview {
            // `SelectAction` trajectory cleanup: clear the jumper and
            // jumped trajectories, the valid flag, and the projectile
            // arc.  We fold all four trajectory overlays (jump-line
            // preview, projectile arc, valid flag, crumpled-net tint)
            // into the single host-side preview since there is only
            // ever one visible arc; clearing them together here is an
            // immediate wipe before the next mouse-update frame.
            self.interaction.invalidate_action();
        }
        if fx.reset_input {
            // MSG_RESET_INPUT clears the rubber-band selection flags
            // and suppresses any pending drag / click so a modal popup
            // / dialog entered from a sequence command doesn't leave
            // input state armed.  Also zeroes the per-frame modifier
            // cache and the swordfight mouse-way polyline (modifier
            // keys, drag, UI focus, info overlay, mouse-way).
            self.reset_modal_input();
            // Reset does the swap `info_displayed = fps_cheat;
            // fps_cheat = false`: the FPS-cheat flag is consumed and
            // promoted into `info_displayed`, so toggling the FPS
            // cheat arms the next reset to leave the debug-info
            // overlay visible.  The cheat flag lives on
            // `DevState::debug.fps_display`, which is not reachable
            // from here — hand off via a typed host signal for
            // the game-loop site that owns `&mut DevState` to apply.
            effects.request_signal(HostSignal::PromoteFpsCheat);
            // Zero the no-mouse-move accumulator so the
            // hover-trajectory gate (`TIME_TRAJECTORY_DISPLAY`)
            // doesn't re-arm immediately after a modal dialog or task
            // switch.
            self.interaction.trajectory_preview.interrupt_hover();
        }
        if fx.cancel_multi_selection {
            self.input.cancel_selection_gestures();
        }
        if let Some(top_left) = fx.pending_minimap_position {
            // Write the new minimap top-left back to the active player
            // profile on every accepted move. Persist through this host's
            // explicit application context and save to disk; failures are
            // logged after the sim has already accepted the new position.
            let context = application_context.clone();
            context
                .update_and_retain_player_profiles(|mgr| {
                    let profile = mgr
                        .get_active_mut()
                        .expect("ApplicationContext lost its required active player profile");
                    profile.minimap_x = top_left.x;
                    profile.minimap_y = top_left.y;
                })
                .unwrap_or_else(|error| panic!("failed to persist minimap position: {error}"))
                .log_persistence_error("failed to persist minimap position to profile");
        }
        if fx.pending_swordfight_drag_ignore && self.input.is_dragging() {
            // Selected PC left Swordfighting this tick; if a drag was
            // in flight, raise `IgnoreMouseEvent(true, true, true)` so
            // the drag doesn't bleed into a click-release or a
            // subsequent double-click.
            self.input.ignore_mouse_event(true, true, true);
        }
        self.presentation.skip_render = fx.skip_render;
        // Dispatch sim-emitted sound commands onto the SoundManager.
        // Most variants queue into `SoundManager::pending_sounds` and
        // are played out by `SoundManager::hourglass`; the two that
        // need access to `engine.sound_sim.sources` (ResumeAllSources,
        // ActivateSource) are stashed on host and drained by
        // game_session before the hourglass call.
        for cmd in fx.sounds {
            match cmd {
                SoundCommand::StopExclamation { actor_id } => {
                    audio
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
                        let had_deferred_stop = audio.deferred.iter().any(|request| {
                            *request == DeferredAudioRequest::StopExclamation(actor_id.index())
                        });
                        if had_deferred_stop {
                            audio.deferred.retain(|request| {
                                *request != DeferredAudioRequest::StopExclamation(actor_id.index())
                            });
                            audio.sound.drop_pending_exclamations(actor_id.index());
                            audio
                                .deferred
                                .push(DeferredAudioRequest::StopExclamationChannel(
                                    actor_id.index(),
                                ));
                        }
                    }
                    audio.sound.play_exclamation(
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
                    audio.sound.queue_fx(fx_id, position, material);
                }
                SoundCommand::StrikeFx {
                    strike_kind,
                    weapon1,
                    weapon2,
                    position,
                } => {
                    audio
                        .sound
                        .queue_strike_fx(strike_kind, weapon1, weapon2, position);
                }
                SoundCommand::ImpactFx {
                    impact_kind,
                    weapon,
                    armor,
                    position,
                } => {
                    audio
                        .sound
                        .queue_impact_fx(impact_kind, weapon, armor, position);
                }
                SoundCommand::Jingle(jingle) => {
                    audio.sound.queue_jingle(jingle);
                }
                SoundCommand::SetMusicMode(mode) => {
                    audio.sound.set_music_mode(mode);
                }
                SoundCommand::ForceMusicMode(mode) => {
                    audio.sound.force_music_mode(mode);
                }
                SoundCommand::PlayDelayedSource(idx) => {
                    audio
                        .deferred
                        .push(DeferredAudioRequest::PlayDelayedSource(idx));
                }
                SoundCommand::ResumeAllSources { .. } => {
                    if !audio
                        .deferred
                        .contains(&DeferredAudioRequest::ResumeAllSources)
                    {
                        audio.deferred.push(DeferredAudioRequest::ResumeAllSources);
                    }
                }
                SoundCommand::ActivateSource(idx) => {
                    audio
                        .deferred
                        .push(DeferredAudioRequest::ActivateSource(idx));
                }
                SoundCommand::RefreshAmbienceSources => {
                    if !audio
                        .deferred
                        .contains(&DeferredAudioRequest::RefreshAmbienceSources)
                    {
                        audio
                            .deferred
                            .push(DeferredAudioRequest::RefreshAmbienceSources);
                    }
                }
            }
        }
        // Accumulate UI-request queues — the host drives the widgets
        // asynchronously so signals outlive a single tick.
        effects.extend_dialogues(fx.pending_dialogues);
        effects.extend_popup_texts(fx.pending_popup_texts);
        effects.extend_debriefings(fx.pending_debriefings);
        if fx.pending_sherwood_report {
            effects.request_sherwood_report();
        }
        if local_seat == engine_player_command::PlayerId::HOST {
            effects.extend_trade_receipts(fx.trade_receipts);
        } else if !fx.trade_receipts.is_empty() {
            tracing::trace!(
                count = fx.trade_receipts.len(),
                "discarding host-only Sherwood trade receipts on a client"
            );
        }
        if fx.pending_show_console {
            effects.request_signal(HostSignal::ShowConsole);
        }
        if fx.pending_silent_win_widget_swap {
            effects.request_signal(HostSignal::SilentWinWidgetSwap);
        }
        if fx.pending_mission_state_notice {
            effects.request_signal(HostSignal::MissionStateNotice);
            effects.request_signal(HostSignal::MissionStatePopup);
        }
        if fx.pending_reset_input {
            effects.request_signal(HostSignal::ResetInput);
        }
        // Per-frame mark requests from sim-side Mark() calls (currently
        // scripted mission-team insertion → `EngineCommand::MarkPc`).
        // Accumulates with host-side mark sources (requirements-bar
        // hover, portrait guard hover); the render loop drains the
        // buffer right after the outline pass.
        self.input
            .feedback
            .marked_pc_ids
            .extend(fx.pending_mark_pc_ids);
        // Patch-effect background decal changes are accumulated across
        // frames until the next render pass drains them.
        effects.background_blits.extend(fx.bg_blits);
        fx.code
    }
}

impl FrontendResources {
    fn retire(&mut self, renderer: &mut crate::renderer::Renderer) {
        self.mission_surfaces.retire(renderer);
        self.background_decals.clear();
    }
    /// Current immutable rendering generation. Retaining a clone preserves that
    /// generation, but cannot replace either the renderer or opacity publisher.
    ///
    /// ```compile_fail,E0616
    /// use robin_rs::host::FrontendResources;
    /// fn replace(frontend: &mut FrontendResources) {
    ///     frontend.frame_holder = Default::default();
    /// }
    /// ```
    ///
    /// ```compile_fail,E0596
    /// use robin_rs::host::FrontendResources;
    /// fn mutate(frontend: &mut FrontendResources) {
    ///     frontend.frame_holder().apply_arno_law(0);
    /// }
    /// ```
    pub fn frame_holder(&self) -> &Arc<FrameHolder> {
        &self.frame_holder
    }

    /// Install a prepared sprite bank during loading, before any live opacity
    /// readers exist. Runtime replacement must use synchronized rebinding.
    pub fn install_frame_holder_before_publication(&mut self, holder: FrameHolder) {
        *self.frame_holder_before_publication_mut() = holder;
    }

    /// Mutable loading access before the opacity view is published.
    /// Post-publication mutations must use
    /// [`Self::rebind_frame_holder_shadow_color`] so the engine and renderer
    /// switch generations together.
    pub fn frame_holder_before_publication_mut(&mut self) -> &mut FrameHolder {
        assert!(
            self.frame_holder_opacity.is_none(),
            "published frame holder cannot be mutated without synchronizing pixel opacity"
        );
        Arc::make_mut(&mut self.frame_holder)
    }

    /// Publish the fully initialized frame-holder generation for engine hit
    /// testing. This is a one-way loading boundary: subsequent dictionary
    /// changes must go through [`Self::rebind_frame_holder_shadow_color`].
    /// The returned reader deliberately hides the publisher: consumers must not
    /// replace the engine generation independently of the renderer.
    ///
    /// ```compile_fail,E0599
    /// use robin_rs::host::FrontendResources;
    /// fn replace_opacity(frontend: &mut FrontendResources) {
    ///     let reader = frontend.publish_frame_holder_opacity();
    ///     reader.publish(frontend.frame_holder().clone());
    /// }
    /// ```
    pub fn publish_frame_holder_opacity(&mut self) -> Arc<dyn engine_api::PixelOpacityLookup> {
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
    }
}

impl HostFrontend {
    pub fn install_trajectory_ground_mark_sprite(&mut self, data: &GroundMarkSpriteData) {
        self.interaction
            .trajectory_preview
            .install_mark_sprite(data);
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
    fn director_pan_starts_at_local_view_and_finishes_at_script_target() {
        let mut viewport = ViewportState::new(1280.0, 720.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(2200.0, 1800.0);
        let view_size = ScreenSize::new(1024.0, 768.0);
        let idle = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(100.0, 200.0),
            zoom_factor: 1.0,
            slide_target: None,
            owns_view: false,
        };

        // LockUser and a timer before CameraGoto must leave the local view
        // where the player put it, even though the director is far away.
        let locked = engine_api::DirectorCameraFrame {
            owns_view: true,
            ..idle
        };
        viewport.advance_director_camera(idle, locked, view_size);
        assert_eq!(viewport.view_position, MapPoint::new(2200.0, 1800.0));
        let start = engine_api::DirectorCameraFrame {
            slide_target: Some(MapPoint::new(1100.0, 1200.0)),
            ..locked
        };
        viewport.advance_director_camera(locked, start, view_size);
        assert_eq!(viewport.view_position, MapPoint::new(2200.0, 1800.0));

        let halfway = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(600.0, 700.0),
            ..start
        };
        viewport.advance_director_camera(start, halfway, view_size);
        close(viewport.view_position.x, 1586.0);
        close(viewport.view_position.y, 1512.0);

        // Completion can clear the slide in the same tick that it arrives.
        let end = engine_api::DirectorCameraFrame {
            view_position: start.slide_target.unwrap(),
            slide_target: None,
            ..start
        };
        viewport.advance_director_camera(halfway, end, view_size);
        assert_eq!(viewport.view_position, MapPoint::new(972.0, 1224.0));
        viewport.advance_director_camera(end, end, view_size);
        assert_eq!(viewport.view_position, MapPoint::new(972.0, 1224.0));
    }

    #[test]
    fn director_jump_interrupts_pan_and_adopts_destination() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(2200.0, 1800.0);
        let before = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(100.0, 200.0),
            zoom_factor: 1.0,
            slide_target: Some(MapPoint::new(1100.0, 1200.0)),
            owns_view: true,
        };
        let after = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(500.0, 600.0),
            slide_target: None,
            owns_view: false,
            ..before
        };
        viewport.advance_director_camera(before, after, viewport.screen_size);
        assert_eq!(viewport.view_position, after.view_position);
    }

    #[test]
    fn director_zoom_keeps_local_focal_point() {
        let mut viewport = ViewportState::new(1280.0, 720.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(2200.0, 1800.0);
        let center = ScreenPoint::new(640.0, 360.0);
        let focal_point = viewport.screen_to_map_unchecked(center);
        let before = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(100.0, 200.0),
            zoom_factor: 1.0,
            slide_target: None,
            owns_view: true,
        };
        let after = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(356.0, 392.0),
            zoom_factor: 2.0,
            ..before
        };
        viewport.advance_director_camera(before, after, ScreenSize::new(1024.0, 768.0));
        assert_eq!(viewport.zoom_factor, 2.0);
        assert_eq!(viewport.screen_to_map_unchecked(center), focal_point);
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
    fn focus_loss_retires_both_buttons_captures_and_path_without_changing_planning() {
        let mut frontend = HostFrontend::default();
        frontend.planning.update_preference(true);
        frontend.planning.toggle_touch();
        frontend.begin_left_pointer(Default::default(), 2);
        frontend.begin_right_pointer(2);
        frontend.begin_minimap_drag(true);
        frontend.add_gesture_point(Default::default());
        frontend.route_hud_event(&crate::gfx_types::GameEvent::MouseDown(0, 0, 1, 1), true);
        frontend.lose_pointer_focus();
        assert!(!frontend.input.left_mouse_down());
        assert!(!frontend.input.controls.right_mouse_down);
        assert!(!frontend.release_left_pointer());
        assert!(!frontend.release_right_pointer());
        assert!(!frontend.pointer_capture().minimap_drag_active());
        assert!(frontend.mouse_way().is_empty());
        assert!(!frontend.route_hud_event(&crate::gfx_types::GameEvent::MouseUp(0, 0, 1), false));
        assert!(frontend.planning.touch_latched());
    }

    #[test]
    fn modal_entry_cancels_captures_and_gesture_but_preserves_held_button_semantics() {
        let mut frontend = HostFrontend::default();
        frontend.begin_left_pointer(Default::default(), 1);
        frontend.begin_right_pointer(2);
        frontend.begin_minimap_drag(true);
        frontend.add_gesture_point(Default::default());
        frontend.input.controls.is_alt = true;
        frontend.reset_modal_input();
        assert!(frontend.input.left_mouse_down());
        assert!(!frontend.input.is_dragging());
        assert!(!frontend.input.controls.is_alt);
        assert!(!frontend.pointer_capture().minimap_drag_active());
        assert!(!frontend.release_right_pointer());
        assert!(frontend.mouse_way().is_empty());
    }

    #[test]
    fn modal_reset_preserves_view_target_but_snapshot_reset_retires_it() {
        let mut frontend = HostFrontend::default();
        let selected = EntityId::Soldier(robin_engine::entity_id::SoldierId(7));
        frontend.set_selected_view_element(Some(selected));
        frontend.arm_tactical_patrol(vec![selected], TacticalFormation::Line);
        frontend.apply_trajectory_preview(engine_api::input::TrajectoryPreview::HitNoArc);
        frontend.reset_interaction(InteractionReset::ModalClosed);
        assert_eq!(frontend.selected_view_element(), Some(selected));
        assert!(!frontend.tactical_targeting().is_armed());
        assert!(!frontend.trajectory_preview().is_valid());
        frontend.reset_interaction(InteractionReset::SnapshotRestored);
        assert_eq!(frontend.selected_view_element(), None);
    }

    #[test]
    fn hover_observation_retires_explanations_without_cancelling_targeting() {
        let mut frontend = HostFrontend::default();
        frontend.arm_tactical_patrol(Vec::new(), TacticalFormation::Line);
        frontend.set_item_effect_preview(Some(ItemEffectPreview {
            center: MapPoint::ZERO,
            radius: None,
            localization_key: "test",
            fallback_text: "test",
            blocked: false,
        }));
        frontend.set_host_titbit_preview(Some(HostTitbitPreview::JumpHelperGhost {
            position: WorldPoint3D::new(0.0, 0.0, 0.0),
            layer: 0,
            sector_dir: 0,
            display_order: 0.0,
        }));
        frontend.observe_hover_feedback(false, Default::default(), MapPoint::ZERO);
        assert!(frontend.item_effect_preview().is_none());
        assert!(frontend.host_titbit_preview().is_none());
        assert!(frontend.tactical_targeting().is_armed());
        assert_eq!(frontend.trajectory_preview().hover_ticks(), 1);
    }

    #[test]
    fn interaction_diagnostics_cannot_restore_live_feedback() {
        let feedback = FrontendInteraction::default();
        let diagnostic = serde_json::to_vec(&feedback).unwrap();
        assert!(serde_json::from_slice::<FrontendInteraction>(&diagnostic).is_err());
    }

    #[test]
    fn frontend_snapshot_boundary_retires_requests_but_preserves_live_resources_and_presentation() {
        let mut host = Host::scratch(640.0, 480.0);
        let sprites = Arc::clone(host.frontend.resources.frame_holder());
        host.frontend
            .request_print_screen(PrintScreenRequest::Median3x3);
        host.frontend
            .diagnostics_mut()
            .queue_console_output("old mission output".into());
        host.frontend.diagnostics_mut().record_frame(100, 7);
        host.frontend.diagnostics_mut().observe_present_cost(42);
        host.frontend.presentation.fade_to_black = Some(FadeToBlack {
            speed: 20,
            frames_remaining: 13,
        });
        host.frontend.presentation.pc_info_overlay.visible = true;
        host.frontend.presentation.skip_render = true;
        host.frontend.slow_motion = true;
        let before = serde_json::to_value(&host.frontend.presentation).unwrap();

        host.post_load_reset();
        host.post_load_reset(); // Retirement is idempotent, not a second rebuild.

        assert!(host.frontend.take_print_screen().is_none());
        assert!(
            host.frontend
                .diagnostics_mut()
                .take_console_output()
                .is_empty()
        );
        assert_eq!(host.frontend.diagnostics().max_pending_sounds(), 7);
        assert_eq!(
            host.frontend.diagnostics().native_refresh_present_cost_us(),
            42
        );
        assert!(Arc::ptr_eq(
            &sprites,
            host.frontend.resources.frame_holder()
        ));
        assert_eq!(
            serde_json::to_value(&host.frontend.presentation).unwrap(),
            before
        );
        assert!(host.frontend.slow_motion);

        // Mission replacement constructs a fresh Host instead of resetting the
        // old mission's resources in place. Camera/preferences are then bound
        // explicitly by startup, and no pending output crosses that boundary.
        let next = Host::scratch(640.0, 480.0);
        assert!(!Arc::ptr_eq(
            &sprites,
            next.frontend.resources.frame_holder()
        ));
        assert!(next.frontend.presentation.fade_to_black.is_none());
        assert!(!next.frontend.presentation.pc_info_overlay.visible);
        assert!(!next.frontend.slow_motion);
    }

    #[test]
    fn capture_slot_preserves_wide_branch_and_latest_request_wins() {
        let mut frontend = HostFrontend::default();
        frontend.request_print_screen(PrintScreenRequest::Median3x3);
        assert!(!frontend.take_wide_snapshot_request());
        assert_eq!(
            frontend.take_print_screen(),
            Some(PrintScreenRequest::Median3x3)
        );
        assert!(frontend.take_print_screen().is_none());
        frontend.request_print_screen(PrintScreenRequest::Plain);
        frontend.request_print_screen(PrintScreenRequest::WideSnapshot);
        assert!(frontend.take_wide_snapshot_request());
        assert!(!frontend.take_wide_snapshot_request());
        assert!(frontend.take_print_screen().is_none());
        frontend.request_print_screen(PrintScreenRequest::Plain);
        frontend.reset_interaction(InteractionReset::ModalClosed);
        assert_eq!(
            frontend.take_print_screen(),
            Some(PrintScreenRequest::Plain)
        );
    }

    #[test]
    fn lifecycle_owner_diagnostics_cannot_restore_runtime_authority() {
        let resources = serde_json::to_value(FrontendResources::default()).unwrap();
        assert!(serde_json::from_value::<FrontendResources>(resources).is_err());
        let presentation = serde_json::to_value(FrontendPresentation::default()).unwrap();
        assert!(serde_json::from_value::<FrontendPresentation>(presentation).is_err());
    }

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
        host.frontend.begin_right_pointer(2);
        host.frontend.planning.update_preference(true);
        host.frontend.route_touch_plan_event(
            &crate::gfx_types::GameEvent::MouseDown(0, 0, 1, 1),
            true,
            |_, _| true,
        );
        host.frontend
            .interaction
            .trajectory_preview
            .apply(robin_engine::engine::input::TrajectoryPreview::HitNoArc);
        host.frontend.interaction.item_effect_preview = Some(ItemEffectPreview {
            center: MapPoint::ZERO,
            radius: Some(20),
            localization_key: "test",
            fallback_text: "test",
            blocked: false,
        });
        host.frontend
            .interaction
            .tactical_targeting
            .arm_patrol(Vec::new(), TacticalFormation::default());
        host.frontend.viewport.view_position = MapPoint::new(100.0, 200.0);
        host.frontend.viewport.zoom_factor = 2.0;
        host.frontend.planning.update_preference(true);

        host.post_load_reset();

        assert!(!host.frontend.input.is_dragging());
        assert!(!host.frontend.pointer_capture().touch_plan_captured());
        assert!(!host.frontend.release_right_pointer());
        assert!(!host.frontend.planning.touch_latched());
        assert!(!host.frontend.interaction.trajectory_preview.is_valid());
        assert!(!host.frontend.interaction.tactical_targeting.is_armed());
        assert!(host.frontend.interaction.item_effect_preview.is_none());
        assert_eq!(
            host.frontend.viewport.view_position,
            MapPoint::new(100.0, 200.0)
        );
        assert_eq!(host.frontend.viewport.zoom_factor, 2.0);
        assert!(host.frontend.planning.enabled());
        assert!(host.frontend.resources.mission_surfaces.map().is_none());
    }

    #[test]
    fn modal_and_engine_resets_preserve_sticky_planning_but_cancel_pointer_capture() {
        for reason in [
            InteractionReset::ModalClosed,
            InteractionReset::EngineRequested,
        ] {
            let mut host = Host::scratch(640.0, 480.0);
            host.frontend.planning.update_preference(true);
            host.frontend.route_touch_plan_event(
                &crate::gfx_types::GameEvent::MouseDown(0, 0, 1, 1),
                true,
                |_, _| true,
            );
            host.frontend
                .input
                .press_left_pointer(Default::default(), 1);
            host.frontend.viewport.begin_touch_transform(true);
            host.frontend.reset_interaction(reason);
            assert!(host.frontend.planning.touch_latched());
            assert!(!host.frontend.pointer_capture().touch_plan_captured());
            assert!(!host.frontend.input.left_mouse_down());
            assert!(!host.frontend.viewport.advance_touch_inertia(100));
        }
    }

    #[test]
    fn action_and_input_effects_keep_their_distinct_preview_reset_scopes() {
        use robin_engine::engine::input::TrajectoryPreview;
        let mut host = Host::scratch(640.0, 480.0);
        host.frontend.interaction.trajectory_preview.observe_hover(
            false,
            Default::default(),
            MapPoint::ZERO,
        );
        host.frontend.interaction.trajectory_preview.observe_hover(
            false,
            Default::default(),
            MapPoint::ZERO,
        );
        host.frontend
            .interaction
            .trajectory_preview
            .apply(TrajectoryPreview::HitNoArc);
        host.frontend
            .interaction
            .tactical_targeting
            .arm_patrol(Vec::new(), TacticalFormation::Line);
        host.apply_side_effects(SideEffects {
            invalidate_trajectory_preview: true,
            ..Default::default()
        });
        assert!(!host.frontend.interaction.trajectory_preview.is_valid());
        assert_eq!(
            host.frontend.interaction.trajectory_preview.hover_ticks(),
            2
        );
        assert!(host.frontend.interaction.tactical_targeting.is_armed());
        host.frontend
            .interaction
            .trajectory_preview
            .apply(TrajectoryPreview::HitNoArc);
        host.apply_side_effects(SideEffects {
            reset_input: true,
            ..Default::default()
        });
        assert_eq!(
            host.frontend.interaction.trajectory_preview.hover_ticks(),
            0
        );
        assert!(host.frontend.interaction.trajectory_preview.is_valid());
        assert!(host.frontend.interaction.tactical_targeting.is_armed());
        host.post_load_reset();
        assert!(!host.frontend.interaction.trajectory_preview.is_valid());
        assert!(!host.frontend.interaction.tactical_targeting.is_armed());
    }
}

#[cfg(test)]
mod host_resource_tests {
    use super::*;
    use robin_assets::frame_holder::{SHADOW_KEY, SpriteVariant, TRANSPARENT_COLOR_16};
    use robin_assets::shipping_datadir::{ShippingSprite, ShippingSpriteBank};
    use robin_engine::campaign::Campaign;
    use robin_engine::coordinates::{SpriteAnchor, SpriteFrameOffset};
    use robin_engine::element::{ElementData, ElementFx, ElementKind, Entity};
    use robin_engine::sprite::Sprite;
    use robin_engine::sprite_script::SpriteScript;

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

        let installed = robin_assets::shipping_datadir::ShippingAssets::install(
            std::sync::Arc::new(shipping),
            std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new()),
        )
        .expect("install synthetic dictionary bank");
        let mut holder = FrameHolder::new();
        holder
            .initialize_sprite_bank_with_progress(".", &mut |_| {}, Some(installed.datadir()))
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
        host.frontend
            .resources
            .install_frame_holder_before_publication(dictionary_frame_holder(INITIAL_NIGHT_COLOR));
        let reader = host.frontend.resources.publish_frame_holder_opacity();
        let published = host
            .frontend
            .resources
            .frame_holder_opacity
            .as_ref()
            .unwrap()
            .clone();
        let old_renderer = Arc::clone(host.frontend.resources.frame_holder());
        let old_opacity_snapshot = published.snapshot();

        let mut assets = engine_api::LevelAssets::new();
        assets.attachments.pixel_opacity = Some(reader);
        let engine =
            engine_api::Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut assets)
                .expect("construct sprite-hit-test engine");
        let entity = dictionary_sprite_entity();
        let shadow_point = MapPoint::new(100.0, 100.0);
        let solid_point = MapPoint::new(101.0, 100.0);

        assert!(Arc::ptr_eq(
            &host.frontend.resources.frame_holder,
            &published.snapshot()
        ));
        assert!(!rendered_dictionary_pixel_is_opaque(
            &host.frontend.resources.frame_holder,
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
            .resources
            .rebind_frame_holder_shadow_color(REBOUND_NIGHT_COLOR);

        // Retained readers are immutable snapshots. Live engine handles follow
        // publication, while an old render generation stays byte-for-byte old.
        assert!(Arc::ptr_eq(&old_renderer, &old_opacity_snapshot));
        assert!(!Arc::ptr_eq(
            &old_renderer,
            host.frontend.resources.frame_holder()
        ));
        assert_eq!(
            old_renderer.dictionaries()[0].shadow_color(),
            INITIAL_NIGHT_COLOR
        );
        assert_eq!(
            host.frontend.resources.frame_holder().dictionaries()[0].shadow_color(),
            REBOUND_NIGHT_COLOR
        );

        assert!(Arc::ptr_eq(
            &host.frontend.resources.frame_holder,
            &published.snapshot()
        ));
        for variant in [SpriteVariant::Day, SpriteVariant::Night] {
            let renderer_shadow = rendered_dictionary_pixel_is_opaque(
                &host.frontend.resources.frame_holder,
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
            &host.frontend.resources.frame_holder,
            SpriteVariant::Day,
            REBOUND_NIGHT_COLOR,
            1,
        ));
        assert!(engine.is_point_on_sprite(&assets, &entity, solid_point, false));
        assert!(engine.is_point_on_sprite(&cloned_assets, &entity, solid_point, false));
    }

    #[test]
    #[should_panic(expected = "published frame holder cannot be mutated")]
    fn published_sprite_bank_cannot_reopen_loading_mutation() {
        let mut frontend = HostFrontend::default();
        frontend.resources.publish_frame_holder_opacity();
        frontend.resources.frame_holder_before_publication_mut();
    }

    #[test]
    #[should_panic(expected = "frame-holder opacity was already published")]
    fn sprite_publication_cannot_replace_the_live_reader() {
        let mut frontend = HostFrontend::default();
        frontend.resources.publish_frame_holder_opacity();
        frontend.resources.publish_frame_holder_opacity();
    }

    #[test]
    #[should_panic(expected = "published frame holder cannot be mutated")]
    fn sprite_bank_installation_cannot_replace_a_published_generation() {
        let mut frontend = HostFrontend::default();
        frontend.resources.publish_frame_holder_opacity();
        frontend
            .resources
            .install_frame_holder_before_publication(FrameHolder::new());
    }

    #[test]
    fn ambiance_variant_rebind_publishes_one_new_generation_and_keeps_old_snapshot() {
        let mut frontend = HostFrontend::default();
        frontend
            .resources
            .install_frame_holder_before_publication(dictionary_frame_holder(0x0040));
        frontend.resources.publish_frame_holder_opacity();
        let old_renderer = Arc::clone(frontend.resources.frame_holder());
        let published = frontend
            .resources
            .frame_holder_opacity
            .as_ref()
            .unwrap()
            .clone();

        frontend
            .resources
            .rebind_frame_holder_ambiance(engine_api::Ambiance::Fog, false, 0x1234);

        assert!(!Arc::ptr_eq(
            &old_renderer,
            frontend.resources.frame_holder()
        ));
        assert!(Arc::ptr_eq(
            frontend.resources.frame_holder(),
            &published.snapshot()
        ));
        assert!(
            !old_renderer
                .variant_dictionaries(SpriteVariant::Night)
                .is_empty()
        );
        assert!(
            old_renderer
                .variant_dictionaries(SpriteVariant::Fog)
                .is_empty()
        );
        assert!(
            frontend
                .resources
                .frame_holder()
                .variant_dictionaries(SpriteVariant::Night)
                .is_empty()
        );
        assert!(
            !frontend
                .resources
                .frame_holder()
                .variant_dictionaries(SpriteVariant::Fog)
                .is_empty()
        );
        assert_eq!(frontend.resources.frame_holder().global_shadow(), 10);
        assert_eq!(old_renderer.dictionaries()[0].shadow_color(), 0x0040);
        assert_eq!(
            published.snapshot().dictionaries()[0].shadow_color(),
            0x1234
        );
    }
}
