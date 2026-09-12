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

mod effects;
mod transport;
mod viewport;
pub(crate) use effects::SherwoodTradingAccess;
pub use effects::{
    DeferredAudioRequest, HostAudio, HostEffectBatches, HostModalRequest, HostSignal,
};
pub(crate) use transport::CommittedSnapshotTransition;
pub use transport::{
    HostTransport, PendingSnapshotTransition, PendingSnapshotTransitionPayload, SnapshotSave,
};
pub use viewport::ViewportState;

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
        let preferences = self.with_active_profile_and_keys(|profile, keys| {
            FrontendPreferences::new(
                keys.active.clone(),
                keys.custom.clone(),
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

robin_util::deny_deserialize!(
    FrontendResources,
    "frontend resources require mission preparation"
);

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

robin_util::deny_deserialize!(
    FrontendPresentation,
    "frontend presentation is rebuilt by live phases"
);

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

robin_util::deny_deserialize!(
    FrontendInteraction,
    "interaction feedback requires a live mission/input sequence"
);

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

robin_util::deny_deserialize!(
    HostDraw<'_>,
    "draw authority must be borrowed from the live presentation host"
);

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
            local_seat: self.transport.local_seat(),
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
mod tests;

#[cfg(test)]
pub(crate) mod test_support;
