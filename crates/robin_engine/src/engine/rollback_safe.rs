//! Cross-crate `Engine` facade that makes the
//! "mutations-only-inside-the-tick" invariant mechanical.
//!
//! Downstream crates only ever see the [`Engine`] wrapper defined here.
//! It gives read-only access to the underlying [`EngineInner`] via
//! `Deref`; there is no `DerefMut` and no accessor returning
//! `&mut EngineInner`, so the only way to mutate simulation state from
//! outside `robin_engine` is through an explicit method on this type.
//!
//! [`Engine::advance_frame`] is the canonical runtime transaction. Remaining
//! exposed mutators are compatibility/setup seams and fall into one of these
//! categories:
//!
//! * legacy command/hourglass calls retained for engine fixtures and migration,
//! * a one-shot setup / level-load / lifecycle hook, or
//! * a drain of a side-effect queue filled during the tick and consumed
//!   host-side.
//!
//! Anything that doesn't fit one of those buckets should be pushed into
//! the sim via `SimulationFrameInput`, not added here.

use std::collections::BTreeMap;
use std::ops::Deref;

#[path = "parity_state.rs"]
mod parity_state;

use super::SimConfig;
use super::commands::SelectionCommandBatchMode;
use super::{
    ConsoleResponse, DevState, EngineError, EngineInner, ExternalAction, ExternalActionResult,
    ExternalFacts, FrameAdvanceError, FrameConsoleResponse, LevelAssets, LevelLoadStaging,
    RecordedDropAleRoute, SideEffects, SimEvents, SimulationCommandPhase, SimulationFrameInput,
    SimulationFrameOutput, SimulationRng, SoundBoundaryPolicy,
};
#[cfg(test)]
use super::{DirectorCompletion, InputState, SoundBoundary};
use crate::campaign::Campaign;
use crate::element::EntityId;
use crate::minimap::HitMask;
#[cfg(test)]
use crate::player_command::PlayerCommand;
use crate::player_command::PlayerInput;

/// Spatial data copied from one authoritative fixed tick for host-side
/// presentation interpolation.
///
/// This value is deliberately detached from [`Engine`]. Reading it cannot
/// mutate simulation state, and applying it is only supported on an owned
/// presentation clone through [`PresentationEngine::apply_spatial_presentation`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpatialPresentationSnapshot {
    poses: BTreeMap<EntityId, SpatialPresentationPose>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SpatialPresentationPose {
    position: crate::coordinates::WorldPoint3D,
    map_position: crate::coordinates::MapPoint,
    jump_z_offset: f32,
    kind: crate::element::ElementKind,
    active: bool,
    hidden_in_building: bool,
    in_honolulu: bool,
    layer: u16,
    sector: Option<crate::position_interface::SectorHandle>,
    obstacle: Option<crate::position_interface::ObstacleHandle>,
    in_door_transit: bool,
    posture: crate::element::Posture,
    carrier: Option<EntityId>,
    carried: Option<EntityId>,
    display_order_ref: Option<EntityId>,
    behind_display_order_ref: bool,
    mobile_index: Option<u16>,
    delayed_teleport_queued: bool,
    pc_teleport_counter: Option<u16>,
    pc_teleport_origin: Option<crate::coordinates::MapPoint>,
}

impl SpatialPresentationPose {
    fn from_entity(entity: &crate::element::Entity) -> Self {
        let element = entity.element_data();
        Self {
            position: element.position(),
            map_position: element.position_map(),
            jump_z_offset: entity.actor_data().map_or(0.0, |actor| actor.jump_z_offset),
            kind: element.kind,
            active: element.active,
            hidden_in_building: element.hidden_in_building,
            in_honolulu: element.in_honolulu,
            layer: element.layer(),
            sector: element.sector(),
            obstacle: element.obstacle_index(),
            in_door_transit: element.is_in_door_transit(),
            posture: element.posture(),
            carrier: entity.human_data().and_then(|human| human.carrier),
            carried: entity.pc_data().and_then(|pc| pc.carried),
            display_order_ref: element.sprite.display_order_ref,
            behind_display_order_ref: element.sprite.behind_display_order_ref,
            mobile_index: entity.fx_data().and_then(|fx| fx.mobile_index),
            delayed_teleport_queued: element.position_delayed || element.position_map_delayed,
            pc_teleport_counter: entity.pc_data().map(|pc| pc.teleport_counter),
            pc_teleport_origin: entity.pc_data().map(|pc| pc.position_before_teleport),
        }
    }

    fn requires_snap_to(&self, next: &Self) -> bool {
        // Normal actor/projectile motion is far below this in one 40 ms fixed
        // tick. The guard also catches same-sector scripted teleports, which
        // otherwise have no retained command tag after their immediate
        // sequence element terminates. TODO: replace this final distance guard
        // with a presentation-only teleport epoch once every non-PC teleport
        // path exposes an explicit retained transition marker.
        const MAX_CONTINUOUS_MAP_DISTANCE_PER_TICK: f32 = 128.0;
        let dx = next.map_position.x - self.map_position.x;
        let dy = next.map_position.y - self.map_position.y;
        let implausibly_large_step = dx.abs().max(dy.abs()) > MAX_CONTINUOUS_MAP_DISTANCE_PER_TICK;
        let pc_teleported = match (
            self.pc_teleport_counter,
            next.pc_teleport_counter,
            next.pc_teleport_origin,
        ) {
            (Some(previous), Some(current), Some(origin)) => {
                current > previous
                    || (current > 0
                        && origin.x.to_bits() == self.map_position.x.to_bits()
                        && origin.y.to_bits() == self.map_position.y.to_bits()
                        && (dx != 0.0 || dy != 0.0))
            }
            _ => false,
        };

        self.kind != next.kind
            || self.active != next.active
            || self.hidden_in_building != next.hidden_in_building
            || self.in_honolulu != next.in_honolulu
            || self.layer != next.layer
            || self.sector != next.sector
            || self.obstacle != next.obstacle
            || self.in_door_transit != next.in_door_transit
            || self.posture != next.posture
            || self.carrier != next.carrier
            || self.carried != next.carried
            || self.display_order_ref != next.display_order_ref
            || self.behind_display_order_ref != next.behind_display_order_ref
            || self.mobile_index != next.mobile_index
            || self.delayed_teleport_queued
            || next.delayed_teleport_queued
            || pc_teleported
            || implausibly_large_step
    }
}

/// Canonical gameplay-authoritative engine scalars emitted by schema-13
/// Original parity traces. Presentation camera/surface/backend state is
/// deliberately absent.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, bitcode::Encode, bitcode::Decode,
)]
pub struct ParityEngineState {
    pub cheat_used_flags: u32,
    pub next_creation_order: u32,
    pub chorus_timer: u16,
    pub force_check: bool,
    pub men_to_blazon_conversion: bool,
    pub lock_engine: bool,
    pub freeze_all: bool,
    pub locker: bool,
    pub speed: f32,
    pub speed_int: u16,
    pub mission_won: bool,
    pub mission_won_first_time: bool,
    pub quit_won: bool,
    pub quit_lost: bool,
    pub quit_interrupted: bool,
    pub script_globals: Vec<i32>,
}

/// Parallel runtime array whose length must match the loaded level geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotGridComponent {
    Lines,
    Sectors,
    Masks,
}

impl std::fmt::Display for SnapshotGridComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Lines => "lines",
            Self::Sectors => "sectors",
            Self::Masks => "masks",
        })
    }
}

/// A decoded snapshot is incompatible with the already-loaded mission.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotRestoreError {
    #[error(
        "snapshot fast-grid {component} runtime length {snapshot_len} does not match loaded level length {level_len}"
    )]
    FastGridLengthMismatch {
        component: SnapshotGridComponent,
        snapshot_len: usize,
        level_len: usize,
    },
    #[error("snapshot world invariant failed: {detail}")]
    WorldInvariantViolation { detail: String },
    #[error("snapshot order invariant failed: {detail}")]
    OrderInvariantViolation { detail: String },
    #[error("snapshot campaign-history invariant failed: {detail}")]
    CampaignHistoryInvariantViolation { detail: String },
    #[error("snapshot fog-of-war invariant failed: {detail}")]
    FogOfWarInvariantViolation { detail: String },
    #[error("snapshot level attachment failed: {detail}")]
    AttachmentFailure { detail: String },
}

/// Cross-crate owner of the simulation engine.
///
/// Downstream crates get `&EngineInner` via `Deref` and may only mutate
/// through the methods below.  There is no `DerefMut`, no accessor
/// returning `&mut EngineInner`, and `EngineInner::new` is
/// `pub(crate)`, so no alternative construction path leaks out either.
///
/// Internally (inside `robin_engine`) code still uses `EngineInner`
/// directly — the safety invariant is between the crate and its
/// downstream consumers, not a per-module check.
#[doc = include_str!("../../tests/contracts/engine_capabilities.md")]
#[cfg_attr(
    not(feature = "original-parity"),
    doc = "Ordinary builds cannot acquire Original reconstruction authority:\n```compile_fail,E0599\nuse robin_engine::engine::Engine;\nlet _ = Engine::parity_replay_setup;\n```"
)]
pub struct Engine {
    inner: EngineInner,
    /// Process-local, single-use authority minted only by fresh construction.
    /// It is deliberately absent from snapshots and the simulation hash.
    bootstrap_open: bool,
}

impl serde::Serialize for Engine {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Preserve the historical transparent wire shape without granting the
        // read-only EngineInner projection the power to encode snapshots.
        super::snapshot::serialize_engine_inner(&self.inner, serializer)
    }
}

// Preserve the historical transparent facade hash exactly. Deriving StateHash
// with a skipped authority field would append a skipped-field marker, changing
// every replay hash despite this process-local flag not being simulation state.
impl robin_util::state_hash::StateHash for Engine {
    fn state_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        robin_util::state_hash::StateHash::state_hash(&self.inner, state);
    }
}

impl Clone for Engine {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone_authoritative_state(),
            bootstrap_open: false,
        }
    }
}

impl<'de> serde::Deserialize<'de> for Engine {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        super::snapshot::deserialize_engine_inner(deserializer).map(|inner| Self {
            inner,
            bootstrap_open: false,
        })
    }
}

impl Engine {
    /// Borrow the explicit rendering query surface of the current fixed world.
    pub fn presentation_view(&self) -> super::PresentationView<'_> {
        super::PresentationView::new(&self.inner)
    }

    /// Capture only persisted mission state; unlike `Clone`, this deliberately
    /// excludes runtime continuations, caches, capabilities and attachments.
    pub fn capture_persisted_state(&self) -> Result<super::PersistedEngineState, String> {
        super::PersistedEngineState::capture(&self.inner)
    }

    /// Reconstruct an unattached persisted runtime. Loaded level resources are
    /// still preflighted and attached by `restore_from_snapshot` before use.
    pub fn from_persisted_state(state: super::PersistedEngineState) -> Self {
        Self {
            inner: state.into_engine_inner(),
            bootstrap_open: false,
        }
    }

    /// Capture the world transforms required for smooth host presentation.
    /// The authoritative engine and all sprite/gameplay animation state are
    /// left untouched.
    pub fn spatial_presentation_snapshot(&self) -> SpatialPresentationSnapshot {
        SpatialPresentationSnapshot {
            poses: self
                .inner
                .entities_with_ids_iter()
                .map(|(id, entity)| (id, SpatialPresentationPose::from_entity(entity)))
                .collect(),
        }
    }
}

/// Host-only world copy with interpolation authority, but no simulation authority.
///
/// The projection exposes only explicit rendering queries: callers cannot clone
/// it into an authoritative engine or acquire general simulation APIs.
/// Serialized diagnostics deliberately have no live-state decoder.
///
/// ```no_run
/// use robin_engine::engine::{Engine, PresentationView, PresentationEngine};
/// fn render(source: &Engine) {
///     let presentation = PresentationEngine::new(source);
///     let view: PresentationView<'_> = presentation.view();
///     let _ = view.frame_counter();
/// }
/// ```
/// ```compile_fail,E0599
/// use robin_engine::engine::Engine;
/// let _ = Engine::apply_spatial_presentation;
/// ```
/// ```compile_fail,E0599
/// use robin_engine::engine::PresentationEngine;
/// let _ = PresentationEngine::advance_frame;
/// ```
/// ```compile_fail,E0599
/// use robin_engine::engine::PresentationEngine;
/// let _ = PresentationEngine::restore_from_snapshot;
/// ```
/// ```compile_fail,E0308
/// use robin_engine::engine::{Engine, PresentationEngine};
/// fn forbidden(view: &PresentationEngine) -> Engine { view.view().clone() }
/// ```
/// ```compile_fail,E0308
/// use robin_engine::engine::{EngineInner, PresentationEngine};
/// fn forbidden(view: &mut PresentationEngine) -> &mut EngineInner { view.view() }
/// ```
/// The read surface does not expose simulation snapshot operations:
/// ```compile_fail,E0599
/// use robin_engine::engine::PresentationEngine;
/// fn forbidden(view: &PresentationEngine) {
///     let _ = view.view().capture_persisted_state();
/// }
/// ```
pub struct PresentationEngine {
    presentation: EngineInner,
}

impl serde::Serialize for PresentationEngine {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        // Diagnostics deliberately omit persisted simulation state. Do not
        // serialize the inner world under a tag: extracting that tag would
        // otherwise yield a restorable engine snapshot with interpolated poses.
        let mut diagnostic = serializer.serialize_struct("PresentationEngine", 2)?;
        diagnostic.serialize_field("frame", &self.presentation.frame_counter())?;
        diagnostic.serialize_field("entity_count", &self.presentation.entities_iter().count())?;
        diagnostic.end()
    }
}

impl<'de> serde::Deserialize<'de> for PresentationEngine {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Err(serde::de::Error::custom(
            "presentation engines must be copied from an authoritative engine",
        ))
    }
}

impl PresentationEngine {
    pub fn new(authoritative: &Engine) -> Self {
        Self {
            presentation: authoritative.inner.clone_authoritative_state(),
        }
    }

    pub fn view(&self) -> super::PresentationView<'_> {
        super::PresentationView::new(&self.presentation)
    }

    /// Apply an absolute interpolation sample to an owned presentation clone.
    ///
    /// Only entities present at both fixed-tick endpoints are interpolated.
    /// Spawned/despawned entities therefore snap with the current fixed state.
    /// Teleports and attachment/layer/topology transitions also snap to their
    /// new transform instead of sweeping through invalid world space.
    ///
    /// Calling this repeatedly with the same snapshots and `alpha` is
    /// idempotent; no result depends on the clone's previously sampled pose.
    pub fn apply_spatial_presentation(
        &mut self,
        previous: &SpatialPresentationSnapshot,
        current: &SpatialPresentationSnapshot,
        alpha: f32,
    ) {
        let alpha = if alpha.is_finite() {
            alpha.clamp(0.0, 1.0)
        } else {
            tracing::warn!(
                alpha,
                "non-finite spatial presentation alpha; snapping to current tick"
            );
            1.0
        };

        for (&id, next) in &current.poses {
            let Some(before) = previous.poses.get(&id) else {
                continue;
            };
            let sampled = if before.requires_snap_to(next) {
                next.clone()
            } else {
                let lerp = |a: f32, b: f32| a + (b - a) * alpha;
                let mut pose = next.clone();
                pose.position = crate::coordinates::WorldPoint3D::new(
                    lerp(before.position.x, next.position.x),
                    lerp(before.position.y, next.position.y),
                    lerp(before.position.z, next.position.z),
                );
                pose.map_position = crate::coordinates::MapPoint::new(
                    lerp(before.map_position.x, next.map_position.x),
                    lerp(before.map_position.y, next.map_position.y),
                );
                pose.jump_z_offset = lerp(before.jump_z_offset, next.jump_z_offset);
                pose
            };

            let entity = self.presentation.get_entity_mut(id).unwrap_or_else(|| {
                panic!("presentation clone lost current entity {id:?} while sampling")
            });
            let element = entity.element_data_mut();
            element.set_position(sampled.position);
            // Some authored targets intentionally keep an interaction point
            // distinct from their visible 3D anchor. Preserve both channels.
            element.set_position_map_preserving_3d(sampled.map_position);
            if let Some(actor) = entity.actor_data_mut() {
                actor.jump_z_offset = sampled.jump_z_offset;
            }
        }
    }
}

impl Engine {
    /// Encode a native engine snapshot through the bounded-stack facade codec.
    pub fn encode_native_snapshot(&self) -> Vec<u8> {
        self.try_encode_native_snapshot().expect(
            "authoritative engine reached snapshot encoding with an invalid Spellforge tape",
        )
    }

    /// Validate the package/tape resource boundary before allocating a native
    /// save or multiplayer snapshot.
    pub fn try_encode_native_snapshot(&self) -> Result<Vec<u8>, String> {
        self.inner
            .scripts
            .spellforge
            .validate_snapshot()
            .map_err(|error| format!("invalid Spellforge snapshot: {error}"))?;
        Ok(super::snapshot::encode_native_engine_inner(&self.inner))
    }

    /// Decode an owned native engine snapshot without exposing an owned
    /// [`EngineInner`] to downstream crates.
    ///
    /// The returned engine still has to cross the normal save/network adoption
    /// boundary before replacing a live engine, so level attachment and world
    /// invariants are validated separately from byte decoding.
    pub fn decode_native_snapshot(bytes: &[u8]) -> Result<Self, String> {
        let inner = super::snapshot::decode_native_engine_inner(bytes)
            .map_err(|error| format!("invalid native engine snapshot: {error}"))?;
        inner
            .scripts
            .spellforge
            .validate_snapshot()
            .map_err(|error| format!("invalid Spellforge snapshot: {error}"))?;
        Ok(Self {
            inner,
            bootstrap_open: false,
        })
    }

    /// Return the exact Spellforge package embedded in this authoritative
    /// state, if the mission uses one.
    ///
    /// Network peers use this before snapshot adoption to construct their
    /// process-local VM from the host's versioned package bytes. The package
    /// remains part of the hashed snapshot; this accessor does not expose or
    /// manufacture any mutable simulation state.
    pub fn spellforge_package(
        &self,
    ) -> Option<std::sync::Arc<crate::spellforge::SpellforgePackage>> {
        self.inner.scripts.spellforge.package.clone()
    }
}

/// Explicit capability for parity-tool reconstruction before/during replay.
///
/// This borrow is deliberately not serializable: it is a short-lived setup
/// authority over an `Engine`, not simulation state. Its private owner can
/// only be acquired when the explicit `original-parity` feature is enabled
/// (or inside engine unit tests). Ordinary client builds cannot construct it.
#[must_use = "parity replay setup must be used immediately and not retained"]
pub struct ParityReplaySetup<'a> {
    engine: &'a mut Engine,
}

/// Level-load parameters for [`EngineArgs::level`].
///
/// The host is responsible for pre-loading the mission binaries
/// ([`crate::engine::level_loading::load_mission_for_campaign`]) and
/// pre-decoding the background bitmap (via the host-side
/// `pre_decode_background_map` helper) *before* calling
/// [`Engine::new`].  This lets the constructor size the grid
/// (`set_level_size`), ingest motion data, and run AI init with a
/// fully-populated `fast_grid` — instead of the previous split-init
/// pattern where `map_bbox` was zero and every patrol path failed
/// `TestIfPathIsFine`.
pub struct LevelLoadArgs<'a> {
    pub assets: &'a mut LevelAssets,
    pub level_directory: &'a str,
    pub progress: &'a mut dyn FnMut(f32),
    /// Pre-parsed mission + proto-level binaries.  See
    /// [`crate::engine::level_loading::load_mission_for_campaign`].
    pub loaded: crate::level_data::LoadedLevel,
    /// Background bitmap pixel dimensions, derived from the host's
    /// pre-decoded `PreDecodedBackground`.  Drives
    /// `FastFindGrid::size_map` and `CameraState::set_level_size` —
    /// both need real dims so `is_position_authorized` /
    /// `TestIfPathIsFine` work during `init_ai`.
    pub bg_pixel_dims: (f32, f32),
}

/// Ground-mark sprite metadata: sprite half-diagonal (half-width,
/// half-height) in world pixels and per-frame `(w, h)` sizes, used to
/// build the marker's move-box and on-screen culling rectangle.
///
/// `per_frame_offsets` is the per-frame `(x_min, y_min)` of the opaque
/// region, recorded when each frame is auto-cropped against the
/// `0x07C0` colour key.  The on-screen test adds this to the sprite's
/// top-left before testing the AABB, so plumbing it keeps the cull
/// rectangle aligned with the opaque pixels instead of biased by the
/// transparent border of the uncropped surface.
///
/// Host pre-computes this from the `RHID_GROUND_FOCUS` resource in
/// DEFAULT.RES and hands it to [`Engine::new`] so the sim can place
/// destination markers during the very first tick.
#[derive(Default, Clone)]
pub struct GroundMarkSpriteData {
    pub half_w: f32,
    pub half_h: f32,
    pub frame_sizes: Vec<(u16, u16)>,
    pub per_frame_offsets: Vec<(i16, i16)>,
}

/// Minimap corner-button widget setup: corner-sprite dimensions plus
/// the pixel-level hit mask built from frame 1 of `RHMAP_CORNER`.
/// Engine uses the canonical director view for this legacy minimap
/// setup; local widget placement lives host-side.
pub struct MinimapWidgetSetup {
    pub corner_size: crate::coordinates::ScreenSize,
    pub button_hit_mask: Option<HitMask>,
}

/// Arguments for [`Engine::new`].
///
/// Every field is required: a live `Engine` is defined as
/// "fully initialised for mission play", so construction requires the
/// host to already have loaded mission binaries, pre-decoded the
/// background bitmap, and gathered HUD/widget sprite metadata.  Test
/// and save-restore code paths that want a bare engine construct
/// `EngineInner` directly (it stays `pub(crate)` for that internal
/// use) or pass a test-fixture level through this same path.
pub struct EngineArgs<'a> {
    pub campaign: Campaign,
    pub level: LevelLoadArgs<'a>,
    /// Sprite metadata for the destination-marker ground mark (read
    /// from `RHID_GROUND_FOCUS`).  Used by `add_mark` to offset the
    /// click position and by the per-frame animation tick.  `None`
    /// when the host didn't find the resource (leaves the marker
    /// disabled).
    pub ground_mark_sprite: Option<GroundMarkSpriteData>,
    /// Per-row frame counts for the titbit sprite table.  Indexed by
    /// `SpriteRow` discriminant.  Used by `TitbitManager::num_frames_for_row`
    /// during animation.  Host pre-computes from DEFAULT.RES.  Empty
    /// when the resource is absent.
    pub titbit_row_frame_counts: Vec<u16>,
    /// Initial RNG seed.  Applied as the *first* mutation inside
    /// `Engine::new`, before any setup that draws from the engine's
    /// PRNG (entity spawn, AI init, mission script `StartUp`).  In
    /// single-player this is `0` (the historical default); in
    /// multiplayer it's the host-negotiated `mp_mission_seed`; under
    /// `--replay` it's the recording's header seed.  Threading the
    /// final seed through the constructor — instead of restoring it
    /// post-`Engine::new` — guarantees the engine's frame-0 state is
    /// a deterministic function of `EngineArgs` alone, with no
    /// SP↔MP-host divergence from RNG-consuming work between the
    /// two restore points.
    pub rng_seed: u64,
    /// Optional raw libc `rand()` prefix for original-game parity tooling.
    /// Normal game, replay, save, and multiplayer construction must use `None`.
    pub original_rng_replay: Option<Vec<u32>>,
    /// Complete deterministic configuration captured before level setup.
    /// Keeping the existing [`SimConfig`] intact prevents construction,
    /// rollback, replay, and network adoption from rebuilding only a subset
    /// of gameplay-affecting options.
    pub sim_config: SimConfig,
}

/// Narrow capability for parsed developer commands that are guaranteed not
/// to mutate authoritative simulation state.
pub struct HostConsoleDispatch<'a> {
    engine: &'a mut Engine,
}

impl HostConsoleDispatch<'_> {
    pub fn dispatch(
        &mut self,
        assets: &LevelAssets,
        dev: &mut DevState,
        selected_view_element: &mut Option<EntityId>,
        command: &crate::console::ConsoleCommand,
    ) -> ConsoleResponse {
        assert!(
            command.is_host_only(),
            "simulation console command bypassed frame admission"
        );
        let sim = self.engine.inner.control.simulation_context();
        self.engine.inner.dispatch_console_command(
            &sim,
            assets,
            dev,
            selected_view_element,
            command,
        )
    }
}

/// How a newly assembled mission enters runtime. A lost Sherwood mission enters
/// debriefing without starting the campaign clock, but still closes bootstrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MissionBootstrapCompletion {
    StartClock,
    DebriefOnly,
}

impl Engine {
    /// Complete fresh mission bootstrap exactly once. Only pre-hourglass setup
    /// admissions may precede this boundary. Original save capture may import
    /// a nonzero initial frame before entry; the authority tracks this process'
    /// lifecycle, not the imported absolute frame number. Cloning, decoding,
    /// or restoring a running engine never recreates bootstrap authority.
    pub fn finish_mission_bootstrap(&mut self, completion: MissionBootstrapCompletion) {
        assert!(self.bootstrap_open, "mission bootstrap authority is closed");
        self.bootstrap_open = false;
        if completion == MissionBootstrapCompletion::StartClock {
            self.campaign_reset_mission_length();
        }
    }

    /// Open the capability used exclusively by Original parity replay tools.
    #[cfg(any(test, feature = "original-parity"))]
    pub fn parity_replay_setup(&mut self) -> ParityReplaySetup<'_> {
        ParityReplaySetup { engine: self }
    }

    /// Open the host-only developer-console capability.
    pub fn host_console(&mut self) -> HostConsoleDispatch<'_> {
        HostConsoleDispatch { engine: self }
    }

    /// Snapshot item stock and worker allocation from the live Sherwood map.
    ///
    /// This is presentation-only and does not mutate authoritative campaign
    /// state. It intentionally uses the same capture routine as the mission
    /// exit command so a production report opened after moving a worker is
    /// never based on the previous visit's allocation.
    pub fn live_production_sectors(
        &self,
        profiles: &crate::profiles::ProfileManager,
    ) -> Vec<crate::sector_production::SectorProduction> {
        self.inner.live_production_sectors(profiles)
    }

    /// Presentation view of exactly the inventory stacks Sherwood sale
    /// commands can remove. Unlike mission-exit harvesting, this excludes
    /// active in-flight arrow projectiles.
    pub fn live_tradable_production_sectors(
        &self,
        profiles: &crate::profiles::ProfileManager,
    ) -> Vec<crate::sector_production::SectorProduction> {
        self.inner.live_tradable_production_sectors(profiles)
    }

    fn has_pending_recorded_drop_ale_route(
        &self,
        actor: EntityId,
        destination: crate::coordinates::MapPoint,
    ) -> bool {
        self.inner
            .orders
            .sequence_manager
            .has_pending_drop_ale_route_candidate(actor, destination)
    }

    /// Restore an Original schema-16 session-boundary transient before the
    /// first replay frame. The v48 save payload does not carry this field.
    fn restore_parity_npc_maximal_visibility(&mut self, id: EntityId, value: u16) {
        self.inner.restore_parity_npc_maximal_visibility(id, value);
    }

    fn restore_parity_npc_dormant_macro_cursor(
        &mut self,
        id: EntityId,
        path_id: crate::ai::PathId,
        waypoint_index: u8,
        offset: usize,
        assets: &LevelAssets,
    ) -> bool {
        self.inner.restore_parity_npc_dormant_macro_cursor(
            id,
            path_id,
            waypoint_index,
            offset,
            assets,
        )
    }

    /// Crate-internal access for the validated Original-save adoption
    /// coordinator. Downstream callers cannot bypass `Engine` construction or
    /// replace a partially converted mission.
    pub(crate) fn legacy_adoption_inner(&self) -> &EngineInner {
        &self.inner
    }

    /// Install a fully preflighted detached Original-save candidate.
    ///
    /// This remains crate-internal until every authoritative v48 section is
    /// represented by the coordinator.
    pub(crate) fn install_legacy_adoption_inner(&mut self, inner: EngineInner) {
        // Preserve, never mint, the receiving process' bootstrap authority.
        // Original viewport capture imports its initial save before mission
        // entry, whereas a live replay/save replacement has already closed
        // this authority at bootstrap completion or its first hourglass.
        self.inner = inner;
    }

    /// Run pre-level campaign mission selection on the same authoritative
    /// stream that the selected mission will receive at construction.
    ///
    /// The original process uses one `rand()` sequence across campaign and
    /// mission code. This temporary bare engine owns that sequence while no
    /// loaded mission engine exists; the returned seed is the complete next
    /// RNG state and must be passed to [`EngineArgs::rng_seed`].
    pub fn select_next_mission(
        campaign: Campaign,
        profiles: &crate::profiles::ProfileManager,
        rng_seed: u64,
        sim_config: SimConfig,
    ) -> (Campaign, usize, u64, SimConfig) {
        let mut inner = EngineInner::new_with_campaign(campaign);
        inner.control.sim_config = sim_config;
        inner.restore_rng_from_seed(rng_seed);
        let mission_idx = inner.with_simulation_context(|inner, sim| {
            inner
                .mission_domain
                .campaign_mut()
                .determine_next_mission(sim, profiles)
        });
        let rng_seed = inner.rng_seed();
        let sim_config = inner.control.sim_config;
        (inner.into_campaign(), mission_idx, rng_seed, sim_config)
    }

    /// Consume the exact ranked capability inspected and signed during
    /// preflight. The admitted simulation policy is installed only here, at
    /// the single-use capability boundary.
    pub fn new_ranked(prepared: crate::simulation_inputs::RankedPreparedMissionInputs) -> Self {
        let (prepared, simulation_policy) = prepared.into_prepared_and_policy();
        let mut engine = prepared.into_engine();
        engine
            .inner
            .control
            .install_ranked_simulation_policy(simulation_policy);
        engine
    }

    /// Consume an ordinary sealed mission preparation exactly once.
    pub fn from_prepared(prepared: crate::simulation_inputs::PreparedMissionInputs) -> Self {
        prepared.into_engine()
    }

    pub fn prepare_ranked(
        args: EngineArgs,
        admission: crate::simulation_inputs::RankedContentAdmissionV1<'_>,
    ) -> Result<crate::simulation_inputs::RankedPreparedMissionInputs, EngineError> {
        Self::prepare_ranked_preserving_campaign(args, admission).map_err(|(error, _)| error)
    }

    pub fn prepare_ranked_preserving_campaign(
        args: EngineArgs,
        admission: crate::simulation_inputs::RankedContentAdmissionV1<'_>,
    ) -> Result<
        crate::simulation_inputs::RankedPreparedMissionInputs,
        (EngineError, crate::campaign::Campaign),
    > {
        let prepared = Self::prepare_preserving_campaign(args)?;
        match crate::simulation_inputs::RankedPreparedMissionInputs::admit(prepared, admission) {
            Ok(ranked) => Ok(ranked),
            Err((error, prepared)) => {
                let campaign = prepared.into_engine().into_campaign();
                Err((
                    EngineError::MissionLevelStage {
                        stage: "ranked prepared-content admission",
                        reason: error.to_string(),
                    },
                    campaign,
                ))
            }
        }
    }

    /// Create a fully-initialised engine for mission play.
    ///
    /// The host is expected to have:
    ///
    /// 1. Built `Campaign` + selected the current mission.
    /// 2. Loaded the mission binaries via
    ///    [`crate::engine::level_loading::load_mission_for_campaign`].
    /// 3. Pre-decoded the background bitmap (host-side helper) and
    ///    recorded its pixel dimensions.
    /// 4. Optionally pre-decoded the minimap bitmap.
    ///
    /// With those in hand, this constructor runs every step the old
    /// split `Engine::new` + `apply_level_bitmaps_loaded` pair used to
    /// do — `initialize_from_campaign` (entity spawn, mission script),
    /// `set_level_size`, the motion stage (pathfinder
    /// graph + grid sector registration), `initialize` (mission-script init
    /// followed by AI init, both of which now see a real `map_bbox` +
    /// half-diagonals table), and — for Sherwood —
    /// `apply_production_sector_data`.
    ///
    /// Returns `Err` only when mission data fails to ingest.
    pub fn new(args: EngineArgs) -> Result<Self, EngineError> {
        Self::new_preserving_campaign(args).map_err(|(error, _campaign)| error)
    }

    /// Append one original frame's raw RNG values to an active parity replay.
    fn append_original_rng_replay(&mut self, draws: Vec<u32>) {
        self.inner.control.rng.append_original_replay(draws);
    }

    /// Supply one frame's captured results for Original's undefined stale
    /// sprite action-point read. This is a parity-tool boundary, analogous to
    /// the captured Original RNG stream; live simulation leaves it empty.
    fn set_original_impossible_action_done_deadlines(
        &mut self,
        deadlines: impl IntoIterator<Item = (u32, u32, i16)>,
    ) {
        let mut captured = std::collections::BTreeMap::new();
        for (proposer_creation_order, target_creation_order, deadline) in deadlines {
            captured
                .entry((proposer_creation_order, target_creation_order))
                .or_insert_with(std::collections::VecDeque::new)
                .push_back(deadline);
        }
        self.inner.control.original_impossible_action_done_deadlines = captured;
    }

    /// Replace and rewind the raw Original RNG stream used by parity tools.
    ///
    /// Loaded saves restore a serialized engine and RNG seed after mission
    /// construction. A reconstruction tool may therefore need one copy of
    /// the seeded stream for fresh Rust construction, then rewind to the
    /// post-load stream boundary recorded by the Original.
    fn replace_original_rng_replay(&mut self, draws: Vec<u32>) {
        self.inner.control.sim_config.item_gameplay =
            crate::gameplay_config::ItemGameplayConfig::classic();
        self.inner.control.mission_start_sim_config.item_gameplay =
            crate::gameplay_config::ItemGameplayConfig::classic();
        self.inner.control.sim_config.noise_distraction_feedback = false;
        self.inner
            .control
            .mission_start_sim_config
            .noise_distraction_feedback = false;
        for (_, entity) in self.inner.world.entities.actors_mut() {
            if let Some(enemy) = entity.enemy_ai_mut() {
                enemy.ale_reliable_distraction = false;
            }
        }
        self.inner.control.rng.replace_original_replay(draws);
    }

    /// Rust RNG sites which consumed a selected interval of original draws.
    pub fn original_rng_replay_sites(
        &self,
        range: std::ops::Range<usize>,
    ) -> Option<Vec<crate::sim_rng::RngSite>> {
        self.inner.control.rng.original_replay_sites(range)
    }

    /// Clone the complete engine for structured diagnostics while omitting
    /// the Original parity replay capability, which intentionally cannot be
    /// serialized as an ordinary save/rollback snapshot.
    ///
    /// Diagnostic callers must record [`Self::original_rng_replay_cursor`]
    /// alongside the returned snapshot. All other engine state is unchanged.
    pub fn diagnostic_snapshot_without_original_rng_replay(&self) -> Self {
        let mut snapshot = self.clone();
        snapshot.inner.control.rng = self.inner.control.rng.clone_without_original_replay();
        snapshot
    }

    /// Create a fully-initialised engine while preserving ownership of the
    /// supplied campaign if mission ingestion fails.
    ///
    /// `EngineArgs` consumes its campaign. Callers which are themselves
    /// ownership boundaries (notably mission bootstrap) must use this variant
    /// so an initialization error cannot silently drop the one live campaign.
    /// [`Engine::new`] remains the convenience wrapper for callers which do
    /// not need to recover that value.
    pub fn new_preserving_campaign(
        args: EngineArgs,
    ) -> Result<Self, (EngineError, crate::campaign::Campaign)> {
        Self::construct_preserving_campaign(args)
    }

    /// Prepare and seal an engine while preserving the supplied campaign on
    /// every construction or projection failure.
    pub fn prepare_preserving_campaign(
        args: EngineArgs,
    ) -> Result<
        crate::simulation_inputs::PreparedMissionInputs,
        (EngineError, crate::campaign::Campaign),
    > {
        let EngineArgs {
            campaign,
            level:
                LevelLoadArgs {
                    assets,
                    level_directory,
                    progress,
                    loaded,
                    bg_pixel_dims,
                },
            ground_mark_sprite,
            titbit_row_frame_counts,
            rng_seed,
            original_rng_replay,
            sim_config,
        } = args;
        let starting_campaign = campaign.clone();
        let projection_loaded_level = loaded.clone();
        let projection_ground_mark = ground_mark_sprite.clone();
        let projection_original_rng_replay = original_rng_replay.clone();

        let engine = Self::construct_preserving_campaign(EngineArgs {
            campaign,
            level: LevelLoadArgs {
                assets: &mut *assets,
                level_directory,
                progress,
                loaded,
                bg_pixel_dims,
            },
            ground_mark_sprite,
            titbit_row_frame_counts: titbit_row_frame_counts.clone(),
            rng_seed,
            original_rng_replay,
            sim_config,
        })?;

        let static_projection = match crate::simulation_inputs::SimulationContentProjectionV1::from_prepared_engine_inputs(
            &projection_loaded_level,
            assets,
            bg_pixel_dims,
            projection_ground_mark.as_ref(),
            &titbit_row_frame_counts,
        ) {
            Ok(projection) => projection,
            Err(error) => {
                let campaign = engine.into_campaign();
                return Err((
                    EngineError::MissionLevelStage {
                        stage: "simulation input projection",
                        reason: error.to_string(),
                    },
                    campaign,
                ));
            }
        };
        let run_projection = match crate::simulation_inputs::PreparedMissionRunProjectionV1::new(
            &static_projection,
            &starting_campaign,
            rng_seed,
            &sim_config,
            projection_original_rng_replay.as_deref(),
            assets,
        ) {
            Ok(projection) => projection,
            Err(error) => {
                let campaign = engine.into_campaign();
                return Err((
                    EngineError::MissionLevelStage {
                        stage: "prepared mission run projection",
                        reason: error.to_string(),
                    },
                    campaign,
                ));
            }
        };
        Ok(crate::simulation_inputs::PreparedMissionInputs::seal(
            engine,
            static_projection,
            run_projection,
        ))
    }

    fn construct_preserving_campaign(
        args: EngineArgs,
    ) -> Result<Self, (EngineError, crate::campaign::Campaign)> {
        let original_parity = args.original_rng_replay.is_some();
        let mut sim_config = if original_parity {
            super::cloak::preserve_original_gameplay_behavior(args.sim_config)
        } else {
            args.sim_config
        };
        if original_parity {
            sim_config.item_gameplay = crate::gameplay_config::ItemGameplayConfig::classic();
            sim_config.noise_distraction_feedback = false;
        }
        let mut inner = EngineInner::new_with_campaign(args.campaign);
        inner.control.sim_config = sim_config;
        inner.control.mission_start_rng_seed = args.rng_seed;
        inner.control.mission_start_sim_config = sim_config;
        // Seed the PRNG and apply engine-global cheat flags FIRST,
        // before any setup that might draw from the RNG or branch on
        // the cheat flag.  See `EngineArgs::rng_seed` /
        // `EngineArgs::sim_config` docs for the rationale.
        inner.restore_rng_from_seed(args.rng_seed);
        if let Some(draws) = args.original_rng_replay {
            inner.control.rng = SimulationRng::with_original_replay(draws);
        }
        inner.set_golden_eye_mode(sim_config.golden_eye);
        if let Some(gm) = args.ground_mark_sprite {
            inner.set_ground_mark_sprite_data(
                gm.half_w,
                gm.half_h,
                gm.frame_sizes,
                gm.per_frame_offsets,
            );
        }
        if !args.titbit_row_frame_counts.is_empty() {
            inner.set_titbit_row_frame_counts(args.titbit_row_frame_counts);
        }
        let LevelLoadArgs {
            assets,
            level_directory,
            progress,
            loaded,
            bg_pixel_dims,
        } = args.level;
        assets.entities.mobile_element_count = 0;
        assets.scripts.mission_name = None;
        // The proto-level (motion sectors) loads before the mission
        // file (beam-mes / soldiers / civilians).  We thread
        // `bg_pixel_dims` into `initialize_from_campaign`, which calls
        // `set_level_size` + the motion stage mid-load
        // (right after the proto data is stashed in constructor-local
        // pending data, but before any entity that references a sector
        // spawns) so that beam-me sector validation and downstream
        // sector-handle resolution see the populated grid.
        let mut staging = LevelLoadStaging::default();
        // Process-only diagnostic timing: never retained in engine state or
        // consulted by deterministic initialization.
        let load_started = web_time::Instant::now();
        if let Err(error) = inner.with_simulation_context(|inner, sim| {
            inner.initialize_from_campaign(
                sim,
                assets,
                &mut staging,
                loaded,
                level_directory,
                bg_pixel_dims,
                progress,
            )
        }) {
            let campaign = inner.into_campaign();
            return Err((error, campaign));
        }
        tracing::debug!(
            elapsed_ms = load_started.elapsed().as_secs_f64() * 1000.0,
            "engine construction: mission level stages"
        );
        let topology_started = web_time::Instant::now();
        inner.populate_sector_gates_from_doors();
        let original_topology =
            match crate::legacy_save::topology_adapter::derive_static_element_topology(
                &inner, assets,
            ) {
                Ok(topology) => topology,
                Err(error) => {
                    let campaign = inner.into_campaign();
                    return Err((
                        EngineError::MissionLevelStage {
                            stage: "Original element identity",
                            reason: error.to_string(),
                        },
                        campaign,
                    ));
                }
            };
        inner.world.install_original_creation_orders(
            original_topology.creation_order_by_entity,
            original_topology.static_creation_order_boundary,
        );
        // Mission-script init and then AI init run HERE — after pathfinder +
        // grid are fully populated. This preserves engine initialization's
        // script-before-AI order while still letting TestIfPathIsFine /
        // is_position_authorized see the real map and motion lines.
        tracing::debug!(
            elapsed_ms = topology_started.elapsed().as_secs_f64() * 1000.0,
            "engine construction: gate and Original topology"
        );
        let initialize_started = web_time::Instant::now();
        inner.initialize(assets);
        tracing::debug!(
            elapsed_ms = initialize_started.elapsed().as_secs_f64() * 1000.0,
            "engine construction: script and AI initialization"
        );
        assets.navigation.level_grid = inner.world.fast_grid.level.clone();
        assets.entities.mobile_element_count = inner.world.mobile_elements.len();
        assets.scripts.mission_name = inner
            .scripts
            .mission
            .as_ref()
            .map(|script| script.script_name.clone());

        // Sherwood-only: spawn production bonuses at the registered
        // points.
        let campaign = inner.campaign();
        let is_sherwood = campaign.current_mission_idx.is_some_and(|i| {
            campaign.missions[i]
                .profile(&assets.profile_manager)
                .location
                == crate::profiles::MissionLocation::Sherwood
        });
        if is_sherwood {
            inner.with_simulation_context(|inner, sim| {
                inner.apply_production_sector_data(sim, assets);
                // Fire the "production-sector data is ready" hook
                // (`SendMessage(0, 1001)`) the Sherwood StartUp script
                // listens for on fresh Sherwood entry.  The LevelLoad twin
                // is handled via the post-load fixup path; this arm covers
                // fresh entry only.
                inner.dispatch_startup_message(sim, assets, 1001, 0, 0);
            });
        }
        // Startup scripts and Sherwood setup may intentionally create or kill
        // actors. Only the fully settled world is the Clean Hands baseline.
        inner.initialize_achievement_tracking(assets);
        inner
            .world
            .validate_level_attachments(assets, inner.script_domains.zones.scripts.len());
        Ok(Self {
            inner,
            bootstrap_open: true,
        })
    }

    /// Test-only shortcut: build an `Engine` with an empty fixture
    /// level.  Equivalent to the old
    /// `Engine::new(EngineArgs { ..Default::default() })` spelling
    /// that disappeared when `Engine::new` went RAII.
    ///
    /// Used from unit tests that want an engine for serde round-trip,
    /// command-pipeline, or HUD testing without loading a real
    /// mission from disk.  Not suitable for anything that touches the
    /// pathfinder, motion grid, or AI — the fixture level has no
    /// entities, no motion data, and no pathfinder graph.
    pub fn new_for_test(
        screen_width: f32,
        screen_height: f32,
        campaign: Campaign,
        assets: &mut LevelAssets,
    ) -> Result<Self, super::EngineError> {
        Self::new_for_test_with_level_size_and_simulation(
            screen_width,
            screen_height,
            campaign,
            assets,
            0.0,
            0.0,
            0,
            SimConfig {
                // This helper deliberately constructs a scriptless empty level.
                script_enabled: false,
                ..SimConfig::default()
            },
        )
    }

    /// Variant of [`Engine::new_for_test`] that lets the caller set
    /// non-zero map dimensions — needed by tests that touch the
    /// cutscene camera's zoom / scroll clamps, which key off `level_size`.
    pub fn new_for_test_with_level_size(
        _screen_width: f32,
        _screen_height: f32,
        campaign: Campaign,
        assets: &mut LevelAssets,
        map_width: f32,
        map_height: f32,
    ) -> Result<Self, super::EngineError> {
        Self::new_for_test_with_level_size_and_simulation(
            _screen_width,
            _screen_height,
            campaign,
            assets,
            map_width,
            map_height,
            0,
            SimConfig {
                script_enabled: false,
                ..SimConfig::default()
            },
        )
    }

    /// Test fixture variant that supplies the exact mission-construction
    /// seed/config used by replay, save preflight, and multiplayer tests.
    pub fn new_for_test_with_simulation(
        screen_width: f32,
        screen_height: f32,
        campaign: Campaign,
        assets: &mut LevelAssets,
        rng_seed: u64,
        sim_config: SimConfig,
    ) -> Result<Self, super::EngineError> {
        Self::new_for_test_with_level_size_and_simulation(
            screen_width,
            screen_height,
            campaign,
            assets,
            0.0,
            0.0,
            rng_seed,
            sim_config,
        )
    }

    fn new_for_test_with_level_size_and_simulation(
        _screen_width: f32,
        _screen_height: f32,
        campaign: Campaign,
        assets: &mut LevelAssets,
        map_width: f32,
        map_height: f32,
        rng_seed: u64,
        sim_config: SimConfig,
    ) -> Result<Self, super::EngineError> {
        use crate::mission::Mission;
        use crate::profiles::MissionProfile;

        let mut campaign = campaign;

        // `initialize_from_campaign` expects `current_mission_idx`,
        // `campaign.missions[idx]`, and `profiles.missions[profile_idx]`
        // all to resolve.  When the caller hasn't populated any of
        // those (the common test case), plant a minimal fixture entry
        // at index 0.  We mutate `assets.profile_manager` via
        // `Arc::make_mut` so callers that share the same profiles Arc
        // pick up the fixture.
        if assets.profile_manager.missions.is_empty() {
            let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
            profiles.missions.push(MissionProfile::default());
        }
        if campaign.missions.is_empty() {
            campaign.missions.push(Mission {
                profile_idx: Some(0),
                ..Mission::default()
            });
        }
        if campaign.current_mission_idx.is_none() {
            campaign.current_mission_idx = Some(0);
        }

        let loaded = crate::level_data::LoadedLevel::empty();
        Self::new(EngineArgs {
            campaign,
            level: LevelLoadArgs {
                assets,
                level_directory: "",
                progress: &mut |_| {},
                loaded,
                bg_pixel_dims: (map_width, map_height),
            },
            ground_mark_sprite: None,
            titbit_row_frame_counts: Vec::new(),
            rng_seed,
            original_rng_replay: None,
            sim_config,
        })
    }

    // ── Tick ────────────────────────────────────────────────────────

    fn apply_external_action(
        &mut self,
        assets: &LevelAssets,
        action: ExternalAction,
    ) -> ExternalActionResult {
        match action {
            ExternalAction::Native {
                name,
                args,
                this_actor,
            } => ExternalActionResult::Native(
                self.call_external_native_with_this(assets, &name, &args, this_actor),
            ),
            ExternalAction::ConsoleCommand {
                command,
                mut selected_view_element,
            } => {
                assert!(
                    !command.is_host_only(),
                    "host-only console command crossed the authoritative frame boundary"
                );
                let sim = self.inner.control.simulation_context();
                let response = self.inner.dispatch_sim_console_command(
                    &sim,
                    assets,
                    &mut selected_view_element,
                    &command,
                );
                ExternalActionResult::ConsoleCommand {
                    response: FrameConsoleResponse::from(response),
                    selected_view_element,
                }
            }
            ExternalAction::SimpleMessage { message } => {
                self.inner.send_simple_message(message);
                ExternalActionResult::SimpleMessage
            }
            ExternalAction::EzekielInstakill { target } => {
                ExternalActionResult::EzekielInstakill(self.inner.try_ezekiel_instakill(target))
            }
            ExternalAction::ReplaceCampaign { campaign } => {
                self.inner.replace_campaign(campaign);
                ExternalActionResult::ReplaceCampaign
            }
            ExternalAction::CampaignBuyBlazon { mission_index } => {
                let mission_index = usize::try_from(mission_index)
                    .expect("campaign mission index does not fit usize");
                let profiles = std::sync::Arc::clone(&assets.profile_manager);
                let closed_by_cascade = self.campaign_buy_blazon(mission_index, profiles.as_ref());
                ExternalActionResult::CampaignBuyBlazon { closed_by_cascade }
            }
            ExternalAction::AcknowledgePseudoMissionDebrief => {
                self.campaign_reset_last_pseudo_mission_status();
                ExternalActionResult::AcknowledgePseudoMissionDebrief
            }
        }
    }

    /// Advance one authoritative host-admitted engine frame.
    ///
    /// This is the migration target for drivers that currently call external
    /// replay hooks, `apply_commands`, and `perform_hourglass` separately. It
    /// preserves the Original's boundary ordering:
    ///
    /// 1. the preceding tick's pending presentation refresh when this
    ///    iteration advances simulation,
    /// 2. between-frame director/sound facts,
    /// 3. admitted pre-hourglass host actions,
    /// 4. resolved player commands in recorded order,
    /// 5. the explicitly gated simulation tick,
    /// 6. admitted post-hourglass developer/native actions,
    /// 7. post-hourglass commands,
    /// 8. the optional one-shot post-initialization stage,
    /// 9. side-effect drain and post-frame state hash.
    ///
    /// Rendering, widgets, input latches, developer overlays, and audio
    /// playback remain outside this call at their Original boundaries.
    /// Graphical play can cross `PostInitialize` with a second no-hourglass
    /// admission after presentation without exposing a separate mutation API.
    pub fn advance_frame(
        &mut self,
        assets: &LevelAssets,
        frame: SimulationFrameInput,
    ) -> Result<SimulationFrameOutput, FrameAdvanceError> {
        self.advance_frame_with_command_batch_mode(
            assets,
            frame,
            SelectionCommandBatchMode::InferNestedSelection,
        )
    }

    fn advance_frame_with_command_batch_mode(
        &mut self,
        assets: &LevelAssets,
        frame: SimulationFrameInput,
        command_batch_mode: SelectionCommandBatchMode,
    ) -> Result<SimulationFrameOutput, FrameAdvanceError> {
        if frame.run_hourglass {
            self.bootstrap_open = false;
        }

        if let Some(failure) = self.inner.scripts.spellforge.failure.clone() {
            let frame_counter = self.inner.control.frame_counter;
            let state_hash = crate::replay::state_hash(&self.inner);
            return Ok(SimulationFrameOutput {
                frame_before: frame_counter,
                frame_after: frame_counter,
                hourglass_ran: frame.run_hourglass,
                events: SimEvents::from(SideEffects {
                    code: crate::game_operation::GameCode::LevelInterrupted,
                    ..Default::default()
                }),
                post_boundary_events: SimEvents::default(),
                post_initialize_events: None,
                external_action_results: Vec::new(),
                state_hash,
                spellforge_abort: Some(failure),
            });
        }

        let frame_before = self.inner.control.frame_counter;
        let SimulationFrameInput {
            external_facts,
            external_actions,
            commands,
            post_external_actions,
            post_commands,
            run_hourglass,
            simulation_body_allowed,
            run_post_initialize,
        } = frame;

        let ranked_policy = self.inner.control.ranked_simulation_policy();
        if let Some(policy) = ranked_policy {
            Self::validate_ranked_simulation_config(policy, self.inner.control.sim_config)?;
            Self::validate_ranked_simulation_commands(
                &commands,
                SimulationCommandPhase::PreHourglass,
            )?;
            Self::validate_ranked_simulation_commands(
                &post_commands,
                SimulationCommandPhase::PostHourglass,
            )?;
        }

        // The game loop runs refresh after the preceding
        // simulation tick and before it admits the next input message. The
        // old fallback in perform_hourglass_authoritative crossed this
        // boundary only after `commands` had already run.  That changes the
        // global rand() ownership whenever both a falling-arrow Refresh and a
        // randomising input command occur in one frame. Cross it at tick admission. A
        // no-hourglass admission represents a callback/modal boundary rather
        // than another GameLoop tick, so it must leave the refresh pending;
        // the retained fallback remains useful to low-level tests which call
        // simulation tick directly.
        if run_hourglass {
            let sim = self.inner.control.simulation_context();
            self.inner.apply_pending_presentation_refresh(&sim);
        }

        // Facts are fallible authoritative inputs. Stage the complete ordered
        // prefix so a corrupt later completion or sound resolution cannot
        // leave earlier director/sound mutations partially committed.
        if !external_facts.is_empty() {
            let mut staged_inner = self.inner.clone_authoritative_state();
            Self::apply_frame_external_facts(&mut staged_inner, assets, external_facts)?;
            self.inner = staged_inner;
        } else {
            Self::apply_frame_external_facts(&mut self.inner, assets, external_facts)?;
        }

        let mut external_action_results = external_actions
            .into_iter()
            .map(|action| self.apply_external_action(assets, action))
            .collect::<Vec<_>>();

        let commands: Vec<PlayerInput> = commands.into_iter().map(Into::into).collect();
        let sim = self.inner.control.simulation_context();
        self.inner
            .apply_frame_commands_with_mode(&sim, assets, &commands, command_batch_mode);

        let mut side_effects = if run_hourglass {
            self.inner
                .perform_frame_hourglass(assets, simulation_body_allowed)
        } else {
            SideEffects {
                code: crate::game_operation::GameCode::LevelInProgress,
                ..Default::default()
            }
        };

        external_action_results.extend(
            post_external_actions
                .into_iter()
                .map(|action| self.apply_external_action(assets, action)),
        );

        let post_commands: Vec<PlayerInput> = post_commands.into_iter().map(Into::into).collect();
        let sim = self.inner.control.simulation_context();
        self.inner
            .apply_frame_commands_with_mode(&sim, assets, &post_commands, command_batch_mode);

        // Post-boundary commands are admitted after the main hourglass has
        // already drained its effects. Drain their effects explicitly before
        // the optional PostInitialize stage so acknowledgements are observable
        // on this transaction even on paused/no-hourglass frames.
        let post_boundary_events = SimEvents::from(self.inner.feedback.drain_side_effects());

        let post_initialize_events = run_post_initialize
            .then(|| self.inner.perform_frame_post_initialize(assets))
            .flatten()
            .map(SimEvents::from);
        if let Some(policy) = ranked_policy {
            Self::validate_ranked_simulation_config(policy, self.inner.control.sim_config)?;
        }
        let frame_after = self.inner.control.frame_counter;
        let state_hash = crate::replay::state_hash(&self.inner);

        let spellforge_abort = self.inner.scripts.spellforge.failure.clone();
        if spellforge_abort.is_some() {
            side_effects.code = crate::game_operation::GameCode::LevelInterrupted;
        }

        Ok(SimulationFrameOutput {
            frame_before,
            frame_after,
            hourglass_ran: run_hourglass,
            events: SimEvents::from(side_effects),
            post_boundary_events,
            post_initialize_events,
            external_action_results,
            state_hash,
            spellforge_abort,
        })
    }

    fn validate_ranked_simulation_config(
        policy: super::RankedSimulationPolicy,
        observed: SimConfig,
    ) -> Result<(), FrameAdvanceError> {
        policy
            .validate_config(observed)
            .map_err(|error| match error {
                super::RankedSimulationPolicyError::ConfigMismatch { field } => {
                    tracing::error!(
                        field = field.config_field(),
                        "ranked frame rejected because its immutable simulation config drifted"
                    );
                    FrameAdvanceError::RankedSimulationConfigViolation { field }
                }
                error @ (super::RankedSimulationPolicyError::InvalidIdentity(_)
                | super::RankedSimulationPolicyError::MissingCustomConfiguration
                | super::RankedSimulationPolicyError::InvalidCustomConfiguration(_)) => {
                    unreachable!("installed ranked policy was already validated: {error}")
                }
            })
    }

    fn validate_ranked_simulation_commands(
        commands: &[super::SimCommand],
        phase: SimulationCommandPhase,
    ) -> Result<(), FrameAdvanceError> {
        for (index, command) in commands.iter().enumerate() {
            let Some(field) = command
                .player_input()
                .command
                .ranked_simulation_setting_mutation()
            else {
                continue;
            };
            tracing::error!(
                ?phase,
                index,
                player_id = command.player_input().player_id.0,
                command = ?command.player_input().command,
                field = field.config_field(),
                "rejected command which attempted to edit an immutable ranked setting"
            );
            return Err(FrameAdvanceError::RankedSimulationSettingCommandRejected {
                phase,
                index,
                field,
            });
        }
        Ok(())
    }

    fn apply_frame_external_facts(
        inner: &mut EngineInner,
        assets: &LevelAssets,
        external_facts: ExternalFacts,
    ) -> Result<(), FrameAdvanceError> {
        let ExternalFacts {
            director_completions,
            sound_boundary,
            recorded_drop_ale_routes,
        } = external_facts;
        for (index, completion) in director_completions.into_iter().enumerate() {
            inner
                .apply_frame_external_director_completion(completion, assets)
                .map_err(|reason| FrameAdvanceError::DirectorCompletionRejected {
                    index,
                    completion,
                    reason,
                })?;
        }
        if let Some(sound_boundary) = sound_boundary {
            let policy = sound_boundary.policy;
            if inner.control.ranked_simulation_policy().is_some()
                && policy == SoundBoundaryPolicy::Replay
            {
                return Err(FrameAdvanceError::SoundBoundaryRejected {
                    policy,
                    reason: "ranked simulation accepts only live sound boundaries validated against the sealed speech timing catalog".into(),
                });
            }
            inner
                .try_queue_resolved_exclamations(
                    sound_boundary.resolutions,
                    policy == SoundBoundaryPolicy::Replay,
                )
                .map_err(|reason| FrameAdvanceError::SoundBoundaryRejected { policy, reason })?;
            let sim = inner.control.simulation_context();
            inner
                .hourglass_phase_sound_boundary(&sim, assets)
                .map_err(|reason| FrameAdvanceError::SoundBoundaryRejected { policy, reason })?;
        }
        for (index, route) in recorded_drop_ale_routes.into_iter().enumerate() {
            let RecordedDropAleRoute {
                actor,
                destination,
                goal_sector,
                goal_sector_index,
                goal_layer,
                recorded_gate_path,
            } = route;
            let goal_sector = u16::try_from(goal_sector.get())
                .ok()
                .and_then(crate::position_interface::SectorHandle::new)
                .map(|sector| sector.with_arena_index(goal_sector_index))
                .ok_or_else(|| FrameAdvanceError::RecordedDropAleRouteRejected {
                    index,
                    actor,
                    reason: format!("invalid goal sector {}", goal_sector.get()),
                })?;
            inner
                .orders
                .sequence_manager
                .inject_recorded_drop_ale_route(
                    actor,
                    destination,
                    goal_sector,
                    goal_layer,
                    recorded_gate_path,
                )
                .map_err(|reason| FrameAdvanceError::RecordedDropAleRouteRejected {
                    index,
                    actor,
                    reason,
                })?;
        }
        Ok(())
    }

    /// Select whether recorded between-frame director events own completion
    /// timing for camera sequence elements.
    fn set_external_director_completion_replay(&mut self, enabled: bool) {
        self.inner.set_external_director_completion_replay(enabled);
    }

    /// The per-frame simulation tick. The ONLY per-frame sim-state
    /// mutation point; rollback replay re-runs this on a cloned engine
    /// and must see bit-identical results.
    #[cfg(test)]
    pub(crate) fn perform_hourglass(
        &mut self,
        display: &mut super::HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        dev: &mut DevState,
    ) -> SideEffects {
        self.inner.perform_hourglass(display, input, assets, dev)
    }

    /// Apply a batch of player commands, as used by the replay driver
    /// and the rollback checker.
    #[cfg(test)]
    pub(crate) fn apply_commands(
        &mut self,
        display: &mut super::HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        cmds: &[PlayerInput],
    ) {
        let sim = self.inner.control.simulation_context();
        self.inner
            .apply_commands(&sim, display, input, assets, cmds);
    }

    /// Read-only predicate used by the host to choose between its view-cone
    /// selection and an admitted `EzekielInstakill` frame action.
    pub fn can_ezekiel_instakill(&self, id: EntityId) -> bool {
        self.inner.can_ezekiel_instakill(id)
    }

    // ── Setup / lifecycle ──────────────────────────────────────────

    /// Test-fixture adapter for a mission-script extension against live effects
    /// while the Engine-owned simulation RNG is installed.
    ///
    /// Production startup is owned by engine construction. Fixture native
    /// shims can still draw from `sim_rng`; using this boundary advances the
    /// one authoritative stream instead of panicking for lack of a scope or
    /// inventing a second RNG. The closure must not retain the host reference.
    #[cfg(any(test, feature = "test-helpers"))]
    pub fn test_with_mission_script_effects_and_rng<R>(
        &mut self,
        assets: &LevelAssets,
        f: impl FnOnce(
            &crate::sim_rng::SimulationContext,
            Option<(
                &mut crate::natives::ScriptEffects,
                &mut crate::natives::ScriptState,
                &mut crate::engine::ScriptDomains,
                &crate::natives::AttachedScriptBindings,
                &crate::natives::NativeSessionCapabilities<'_>,
            )>,
        ) -> R,
    ) -> R {
        self.inner.with_simulation_context(|inner, sim| {
            if inner.scripts.mission.is_none() {
                return f(sim, None);
            }
            inner
                .with_script_session(sim, assets, |script, script_domains, capabilities| {
                    f(
                        sim,
                        Some((
                            &mut script.script_effects,
                            &mut script.state,
                            script_domains,
                            &script.bindings,
                            capabilities,
                        )),
                    )
                })
                .expect("mission script disappeared while opening the Lua script session")
        })
    }

    /// Consume a finished mission engine and return its campaign allocation.
    pub fn into_campaign(self) -> Campaign {
        self.inner.into_campaign()
    }

    /// Consume a finished mission engine while preserving the complete next
    /// RNG state for campaign selection before the following engine exists.
    pub fn into_campaign_and_simulation(self) -> (Campaign, u64, SimConfig) {
        let rng_seed = self.inner.rng_seed();
        let sim_config = self.inner.control.sim_config;
        (self.inner.into_campaign(), rng_seed, sim_config)
    }

    /// Attach host-only run eligibility to the exact terminal campaign
    /// attempt after its deterministic quit command has been admitted.
    pub fn promote_mission_achievement_results(
        &mut self,
        policy: crate::achievement::AchievementUnlockPolicy,
        context: crate::achievement::AchievementRunContext,
        profiles: &crate::profiles::ProfileManager,
    ) -> Result<Option<crate::achievement::AchievementHistoryUpdate>, String> {
        self.inner
            .promote_mission_achievement_results(policy, context, profiles)
    }

    /// Live deterministic achievement evidence for optional host HUDs.
    pub fn achievement_progress(&self) -> crate::achievement::AchievementProgressSnapshot {
        self.inner.achievement_progress()
    }

    /// Frozen successful-attempt evidence used by the terminal debrief.
    pub fn mission_achievement_results(
        &self,
    ) -> Option<&crate::achievement::MissionAchievementResults> {
        self.inner.mission_achievement_results()
    }

    /// Exact selected-character skill evidence for the detailed XP HUD.
    pub fn pc_experience_snapshot(
        &self,
        entity: crate::element::EntityId,
    ) -> Result<super::PcExperienceSnapshot, String> {
        self.inner.pc_experience_snapshot(entity)
    }
    /// Seed and configuration captured before this mission's frame-0 setup.
    pub fn mission_start_simulation(&self) -> (u64, SimConfig) {
        (
            self.inner.control.mission_start_rng_seed,
            self.inner.control.mission_start_sim_config,
        )
    }

    /// Invoke a script native with an explicit transient `ThisActor` receiver.
    /// Runtime callers reach this only through [`Self::advance_frame`].
    pub(crate) fn call_external_native_with_this(
        &mut self,
        assets: &LevelAssets,
        native_name: &str,
        args: &[i32],
        this_actor: Option<i32>,
    ) -> Result<i32, String> {
        let sim = self.inner.control.simulation_context();
        self.inner
            .call_external_native_with_this(&sim, assets, native_name, args, this_actor)
    }

    // ── Per-frame drains ────
    // Patch-effect bg blits now travel through `SideEffects`
    // (`apply_side_effects` moves them into `Host::pending_bg_blits`)
    // so the engine no longer owns the queue between tick and render.

    // `mission_script_script_effects_mut` is no longer exposed — the
    // host-side callers use typed frame actions / `PlayerCommand::*` instead.

    // `campaign_mut` is no longer exposed — cross-crate callers use
    // the narrow methods below, or read through `campaign()` and
    // dispatch mutations via `PlayerCommand`.  `Campaign` is part of
    // the rollback hash; any future mutator added here must run on a
    // mission-lifecycle boundary (campaign map, save/load, quit) where
    // the sim is paused.

    /// Commit a blazon purchase on the owned campaign.  Pure menu-time
    /// operation — runs on the mission-description screen while the sim
    /// is paused.  Returns `true` when the Sherwood consume-cascade
    /// closed the buy screen (blazon mission fully funded), matching
    /// `Campaign::buy_blazon`.  `None` when no campaign is installed.
    fn campaign_buy_blazon(
        &mut self,
        mission_index: usize,
        profiles: &crate::profiles::ProfileManager,
    ) -> bool {
        let sim = self.inner.control.simulation_context();
        self.inner
            .mission_domain
            .campaign
            .buy_blazon(&sim, mission_index, profiles)
    }

    /// Reset the campaign's `last_pseudo_mission_status` flag after the
    /// campaign-map host has displayed the pseudo-mission debriefing.
    /// Runs on a mission-lifecycle boundary (sim paused) — `Campaign` is
    /// part of the rollback hash.  No-op when no campaign is installed.
    fn campaign_reset_last_pseudo_mission_status(&mut self) {
        self.inner
            .mission_domain
            .campaign
            .reset_last_pseudo_mission_status();
    }

    /// Reset the campaign's `MissionLength` accumulator to 0 before
    /// the mission begins.
    fn campaign_reset_mission_length(&mut self) {
        self.inner
            .mission_domain
            .campaign
            .set_value(crate::campaign::CampaignValue::MissionLength, 0);
    }

    pub fn is_peasant_name_registered(&self, name: &str) -> bool {
        self.inner.is_peasant_name_registered(name)
    }

    // ── Test-only helpers (round-trip save/load tests) ────────────
    //
    // Gated behind the `test-helpers` Cargo feature so production
    // builds of the facade do not expose direct sim-state setters.
    // `robin_rs` enables the feature in its `[dev-dependencies]`
    // block so its round-trip tests compile.

    /// Build populated achievement evidence via the real mission initializer
    /// and tracking operations, without exposing mutable tracker collections.
    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_seed_achievement_persistence(&mut self, assets: &LevelAssets) {
        use crate::element::*;
        let mut pc_element = ElementData::default();
        pc_element.kind = ElementKind::ActorPc;
        pc_element.active = true;
        let pc = self.inner.add_entity(Entity::Pc(ActorPc {
            element: pc_element,
            actor: Default::default(),
            human: Default::default(),
            pc: PcData {
                life_points: 100,
                mission_role: crate::human_control::MissionRole::PlayerParty,
                kind: Some(crate::character_kind::CharacterKind::MerryManA),
                ..Default::default()
            },
        }));
        let mut soldier_element = ElementData::default();
        soldier_element.kind = ElementKind::ActorSoldier;
        soldier_element.active = true;
        let soldier = self.inner.add_entity(Entity::Soldier(ActorSoldier {
            element: soldier_element,
            actor: Default::default(),
            human: Default::default(),
            npc: NpcData {
                life_points: 100,
                ..Default::default()
            },
            soldier: SoldierData {
                cached_camp: Camp::Royalists,
                ..Default::default()
            },
        }));
        let nest = self.inner.add_entity(Entity::Projectile(ElementProjectile {
            element: Default::default(),
            object: Default::default(),
            projectile: Default::default(),
        }));
        self.inner.initialize_achievement_tracking(assets);
        let state = &mut self.inner.mission_domain.achievements;
        state.record_party_health(pc, 90);
        state.record_wasp_nest_throw(nest);
        state.queue_wasp_sting(soldier, nest);
        state.begin_quick_action_execution();
        state.record_quick_action_launch(pc, soldier);
        state.record_qa_success(pc, soldier);
        state.record_quick_action_launch(pc, soldier);
        state.end_quick_action_execution();
    }

    /// Insert a fully-formed entity into a test engine. Input-resolution
    /// tests need live entities to click on; the blank `new_for_test`
    /// level has none and the production spawn path requires proto data.
    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_add_entity(&mut self, entity: crate::element::Entity) -> EntityId {
        self.inner.add_test_entity(entity)
    }

    /// Seed real sequence-manager insertion order for persistence regressions.
    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_launch_sequence(
        &mut self,
        sequence: crate::sequence::Sequence,
    ) -> crate::sequence::SequenceId {
        self.inner.orders.sequence_manager.launch_sequence(sequence)
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_mission_flags(&mut self, quit_won: bool, quit_lost: bool, mission_won: bool) {
        self.inner
            .test_set_mission_flags(quit_won, quit_lost, mission_won);
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_frame_counter(&mut self, frame: u32) {
        self.inner.test_set_frame_counter(frame);
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_diplomacy(
        &mut self,
        enabled: bool,
        npc_faction_wars: bool,
        definition: crate::diplomacy::DiplomacyDefinition,
    ) {
        self.inner.mission_domain.diplomacy = crate::diplomacy::DiplomacyState::from_definition(
            enabled,
            npc_faction_wars,
            Some(&definition),
        )
        .expect("test diplomacy definition must be valid");
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_engine_scalars(
        &mut self,
        cheat_used_flags: u32,
        speed: f32,
        speed_int: u16,
        lock_engine: bool,
        freeze_all: bool,
        script_globals: Vec<i32>,
    ) {
        self.inner.test_set_engine_scalars(
            cheat_used_flags,
            speed,
            speed_int,
            lock_engine,
            freeze_all,
            script_globals,
        );
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_mission_stat(&mut self, stat: crate::mission_stat::MissionStat) {
        self.inner.test_set_mission_stat(stat);
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_camera_transition_inputs(
        &mut self,
        zoom_init_done: bool,
        mechanized_zoom: bool,
        displacement: crate::coordinates::MapVec,
        displacement_counter: u16,
        pending_zoom_mouse_screen: Option<crate::coordinates::ScreenPoint>,
    ) {
        let camera = &mut self.inner.feedback.cutscene_camera;
        camera.zoom_init_done = zoom_init_done;
        camera.mechanized_zoom = mechanized_zoom;
        camera.displacement = displacement;
        camera.displacement_counter = displacement_counter;
        camera.pending_zoom_mouse_screen = pending_zoom_mouse_screen;
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_camera_transition_inputs(
        &self,
    ) -> (
        bool,
        bool,
        crate::coordinates::MapVec,
        u16,
        Option<crate::coordinates::ScreenPoint>,
    ) {
        let camera = &self.inner.feedback.cutscene_camera;
        (
            camera.zoom_init_done,
            camera.mechanized_zoom,
            camera.displacement,
            camera.displacement_counter,
            camera.pending_zoom_mouse_screen,
        )
    }

    /// Return the authoritative director camera while a script owns the
    /// player's view. The host mirrors this into its local viewport for
    /// cutscenes but remains independent during ordinary player scrolling.
    ///
    /// The view position is the top-left corner of the fixed virtual view
    /// returned by [`Self::director_camera_view_size`]; a host whose canvas
    /// has a different size must shift it to keep the framed focal point
    /// centred.
    pub fn director_camera_view(&self) -> Option<(crate::coordinates::MapPoint, f32)> {
        let camera = &self.inner.feedback.cutscene_camera;
        let script_owns_view = self.inner.players.user_locked
            || camera.sequence_element.is_some()
            || self
                .inner
                .players
                .seats
                .first()
                .is_some_and(|seat| seat.locker_active);
        script_owns_view.then_some((camera.view_position, camera.zoom_factor))
    }

    /// Size of the virtual view the shared script/director camera frames
    /// its targets in. It is deliberately independent of any peer's canvas
    /// so cutscene camera state stays deterministic across resolutions.
    pub fn director_camera_view_size(&self) -> crate::coordinates::ScreenSize {
        EngineInner::director_camera_view_size()
    }

    /// Sample before and after a tick to present scripted motion from the
    /// host's current view, without feeding local scrolling into simulation.
    pub fn director_camera_frame(&self) -> super::DirectorCameraFrame {
        let camera = &self.inner.feedback.cutscene_camera;
        super::DirectorCameraFrame {
            view_position: camera.view_position,
            zoom_factor: camera.zoom_factor,
            slide_target: camera.is_sliding().then_some(camera.camera_slide),
            owns_view: self.director_camera_view().is_some(),
        }
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_assert_level_assets_attached(&self, assets: &LevelAssets) {
        assert!(std::sync::Arc::ptr_eq(
            &self.inner.world.fast_grid.level,
            &assets.navigation.level_grid
        ));
        self.inner.scripts.assert_native_attachments_ready();
    }

    // ── Save / restore ────────────────────────────────────────────

    /// Prepare a decoded save snapshot as a complete replacement Engine.
    ///
    /// The returned value is installed by the timeline/save coordinator only
    /// after validation and post-load fixups succeed. Unlike a live `&mut
    /// self` restore method, this API cannot partially mutate the current
    /// authoritative Engine or conceal the whole-state replacement at the
    /// call site.
    pub fn restore_from_snapshot(
        display: &mut super::HostDisplayState,
        saved: Engine,
        assets: &LevelAssets,
    ) -> Result<Self, SnapshotRestoreError> {
        Self::restore_from_snapshot_with_observer(display, saved, assets, |_| {})
    }

    fn restore_from_snapshot_with_observer(
        display: &mut super::HostDisplayState,
        saved: Engine,
        assets: &LevelAssets,
        post_fixup_observer: impl FnOnce(&EngineInner),
    ) -> Result<Self, SnapshotRestoreError> {
        let mut inner = Self::prepare_snapshot(saved, assets)?;
        inner.post_load_fixups(display);
        post_fixup_observer(&inner);
        inner.queue_update_information_bars();
        Ok(Self {
            inner,
            bootstrap_open: false,
        })
    }

    /// Prepare an exact authoritative network snapshot as a complete
    /// replacement Engine.
    ///
    /// Network adoption deliberately skips save-load transient repair and
    /// preserves every serialized queue exactly as supplied by the host.
    pub fn adopt_authoritative_snapshot(
        snapshot: Engine,
        assets: &LevelAssets,
    ) -> Result<Self, SnapshotRestoreError> {
        Self::prepare_snapshot(snapshot, assets).map(|inner| Self {
            inner,
            bootstrap_open: false,
        })
    }

    fn prepare_snapshot(
        snapshot: Engine,
        assets: &LevelAssets,
    ) -> Result<EngineInner, SnapshotRestoreError> {
        Self::validate_snapshot_compatibility(&snapshot, assets)?;
        let mut inner = snapshot.inner;

        // Attachment preflight above covers every static lookup, so this phase
        // cannot partially fail. The candidate is still detached from `self`.
        inner.attach_preflighted_level_assets(assets);
        inner.orders.sequence_manager.rebuild_indices();
        Ok(inner)
    }

    fn validate_snapshot_compatibility(
        saved: &Engine,
        assets: &LevelAssets,
    ) -> Result<(), SnapshotRestoreError> {
        let level = &assets.navigation.level_grid;
        let lengths = [
            (
                SnapshotGridComponent::Lines,
                saved.inner.world.fast_grid.line_active.len(),
                level.lines.len(),
            ),
            (
                SnapshotGridComponent::Sectors,
                saved.inner.world.fast_grid.sector_active.len(),
                level.sectors.len(),
            ),
            (
                SnapshotGridComponent::Masks,
                saved.inner.world.fast_grid.mask_active.len(),
                level.masks.len(),
            ),
        ];

        // Preserve the original game's path rollback behavior.
        // serializes runtime patch/door/sector state against the already
        // loaded grid and propagates failure. It never invents an all-active
        // replacement when the save and level topology disagree.
        for (component, snapshot_len, level_len) in lengths {
            if snapshot_len != level_len {
                return Err(SnapshotRestoreError::FastGridLengthMismatch {
                    component,
                    snapshot_len,
                    level_len,
                });
            }
        }
        saved
            .inner
            .world
            .preflight_level_assets(assets, saved.inner.script_domains.zones.scripts.len())
            .map_err(|detail| SnapshotRestoreError::WorldInvariantViolation { detail })?;
        saved
            .inner
            .scripts
            .preflight_level_assets(assets)
            .map_err(|detail| SnapshotRestoreError::AttachmentFailure { detail })?;
        saved
            .inner
            .orders
            .validate_invariants()
            .map_err(|detail| SnapshotRestoreError::OrderInvariantViolation { detail })?;
        saved
            .inner
            .mission_domain
            .campaign
            .validate_history_schema()
            .map_err(|detail| SnapshotRestoreError::CampaignHistoryInvariantViolation { detail })?;
        saved
            .inner
            .players
            .fog_of_war
            .validate()
            .map_err(|detail| SnapshotRestoreError::FogOfWarInvariantViolation { detail })?;
        Ok(())
    }
}

impl ParityReplaySetup<'_> {
    /// Whether the original game's ordinary pre-update orientation pass
    /// would have emitted a resolved record for this PC/action pair.
    pub fn orientation_would_emit_before_hourglass(
        &self,
        actor: EntityId,
        action: crate::profiles::Action,
    ) -> bool {
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType;
        use crate::profiles::Action;

        let entity = self
            .engine
            .inner
            .get_entity(actor)
            .unwrap_or_else(|| panic!("orientation actor {actor:?} is missing"));
        let animation = self.engine.inner.live_actor_animation(actor);
        match action {
            Action::Bow => {
                matches!(
                    entity.actor_data().map(|actor| actor.action_state),
                    Some(ActionState::AimingWithBow | ActionState::AimingWithBowUp)
                ) && matches!(
                    animation,
                    Some(
                        OrderType::AimingWithBow
                            | OrderType::AimingWithBowUp
                            | OrderType::AimingWithBowAnonymous
                            | OrderType::AimingWithBowUpAnonymous
                    )
                ) && entity
                    .human_data()
                    .is_some_and(|human| human.pending_shoots.is_empty())
            }
            Action::Apple | Action::Stone | Action::Net | Action::WaspNest | Action::Purse => {
                !matches!(
                    animation,
                    Some(
                        OrderType::ThrowingPurse
                            | OrderType::ThrowingStone
                            | OrderType::ThrowingNet
                            | OrderType::ThrowingWaspNest
                            | OrderType::ThrowingApple
                    )
                )
            }
            Action::HelpToClimb => {
                let posture = entity.posture();
                let action_state = entity.actor_data().map(|actor| actor.action_state);
                if posture == Posture::CarryingOnShoulders {
                    matches!(
                        action_state,
                        Some(ActionState::Waiting | ActionState::Bored)
                    ) && !matches!(
                        animation,
                        Some(
                            OrderType::TransitionHelpingClimbingDown
                                | OrderType::TransitionHelpingClimbingUp
                        )
                    )
                } else {
                    !matches!(
                        posture,
                        Posture::CarryingOnShoulders | Posture::HelpingToClimb
                    )
                }
            }
            Action::Beggar => {
                entity.posture() == Posture::Upright
                    && matches!(
                        entity.actor_data().map(|actor| actor.action_state),
                        Some(ActionState::Waiting | ActionState::Bored)
                    )
            }
            other => {
                panic!("pre-update orientation ownership is not implemented for action {other:?}")
            }
        }
    }

    /// Number of retained scrolls in the replayed world.
    pub fn retained_scroll_count(&self) -> usize {
        self.engine.inner.world.entities.scrolls().count()
    }

    /// Number of falling-arrow RNG draws made by the pending ordinary
    /// presentation pass.
    ///
    /// Legacy parity traces do not retain every host refresh boundary.  The
    /// runner uses this count together with the retained original-game event positions
    /// to distinguish repeated refreshes from a single refresh containing
    /// several arrows.
    pub fn pending_falling_arrow_refresh_draw_count(&self) -> usize {
        if !self.engine.inner.control.arrow_refresh_pending {
            return 0;
        }
        self.engine
            .inner
            .compute_display_order()
            .ids
            .into_iter()
            .filter_map(|id| match self.engine.inner.world.entities.get(id) {
                Some(crate::element::Entity::Projectile(projectile))
                    if projectile.object.object_type == crate::element::ObjectType::Arrow =>
                {
                    Some(projectile)
                }
                _ => None,
            })
            .filter(|projectile| {
                projectile.element.active
                    && projectile.projectile.falling
                    && (!projectile.projectile.trajectory.is_empty()
                        || projectile.element.sprite.position_iface.old_position()
                            != projectile.element.sprite.position_iface.get_position())
            })
            .count()
    }

    /// Replay additional Original presentation passes omitted by a legacy
    /// trace, leaving the ordinary pending pass armed for `advance_frame`.
    ///
    /// Each pass uses normal arrow refresh so
    /// both tumble orientation and RNG consumption advance together.  The
    /// exact expected draw count is retained by the trace and an inconsistent
    /// reconstructed lifecycle fails loudly.
    pub fn replay_legacy_additional_arrow_refreshes(&mut self, expected_draws: usize) {
        let start = self
            .engine
            .inner
            .control
            .rng
            .original_replay_cursor()
            .expect("legacy arrow refresh replay requires Original RNG");
        let mut consumed = 0;
        while consumed < expected_draws {
            let before_pass = consumed;
            let sim = self.engine.inner.control.simulation_context();
            self.engine.inner.refresh_arrows_for_presentation(&sim);
            let cursor = self
                .engine
                .inner
                .control
                .rng
                .original_replay_cursor()
                .expect("legacy arrow refresh replay lost Original RNG");
            consumed = cursor - start;
            assert!(
                consumed <= expected_draws,
                "legacy arrow refresh reconstruction consumed {consumed} draws, expected {expected_draws}"
            );
            assert!(
                consumed > before_pass,
                "legacy arrow refresh reconstruction made no progress"
            );
        }
    }

    /// Consume a proven legacy burst of presentation-only sprite RNG draws.
    ///
    /// Schema 16 retained the values and callsites but omitted the host-side
    /// lifecycle event which caused these draws. The parity runner gates this
    /// narrowly from that evidence. No retained game state is changed here.
    pub fn consume_legacy_presentation_sprite_rng(&mut self, draw_count: usize) {
        let sim = self.engine.inner.control.simulation_context();
        for _ in 0..draw_count {
            let _ = crate::sim_rng::u32(
                &sim,
                crate::sim_rng::RngSite::ScrollInitialFrame,
                0..u32::MAX,
            );
        }
    }

    /// Cross the game's post-recording presentation boundary.
    ///
    /// Target-sprite creation overwrites the serialized width/height
    /// cache with the current bank frame while refreshing visible entities.
    /// Rust rendering is read-only, so the parity runner applies that legacy
    /// side effect explicitly after comparing each recorded frame.
    pub fn refresh_sprite_dimension_cache(
        &mut self,
        assets: &LevelAssets,
        legacy_missing_presentation_view: bool,
    ) {
        // Schema 16 does not record the engine view point, the draw-time
        // viewport origin used for visibility. It can differ from
        // the serialized simulation camera at the exact pixel where Original
        // decides whether target-sprite creation updates this cache. Leave that
        // unobservable presentation state untouched instead of inventing a
        // viewport. The parity comparator projects these cache fields out.
        // TODO: Once a trace schema records the draw viewport, replay that
        // exact value and compare the cache normally.
        if legacy_missing_presentation_view {
            return;
        }
        let Some(frames) = assets.attachments.pixel_opacity.as_ref() else {
            return;
        };
        let camera = &self.engine.inner.feedback.cutscene_camera;
        let screen = EngineInner::director_camera_view_size();
        let view = crate::sprite::BBox::from_coords(
            camera.view_position.x,
            camera.view_position.y,
            camera.view_position.x + screen.x / camera.zoom_factor,
            camera.view_position.y + (screen.y - super::PANNEL_HEIGHT) / camera.zoom_factor,
        );
        let updates = self
            .engine
            .inner
            .world
            .entities
            .occupied()
            .filter_map(|(id, entity)| {
                let element = entity.element_data();
                if !element.active || element.hidden_in_building {
                    return None;
                }
                // Scroll refresh only delegates to the ordinary object
                // renderer in these two states. Invisible and taken scrolls keep
                // their saved dimension cache even when their geometry intersects
                // the camera view.
                if matches!(entity, crate::element::Entity::Scroll(_))
                    && !matches!(
                        self.engine.inner.scroll_status(id),
                        super::ScrollStatus::Visible | super::ScrollStatus::Opened
                    )
                {
                    return None;
                }
                let sprite = entity.sprite();
                let row = sprite
                    .current_scripts_opt()
                    .and_then(|scripts| scripts.get(usize::from(sprite.current_row)))?;
                let &bank_id = row.frame_ids.get(usize::from(sprite.current_frame))?;
                let (width, height) = frames.sprite_dimensions(bank_id)?;
                // Screen visibility uses the current surface dimensions;
                // the serialized cache is not consulted until target-sprite creation
                // publishes those same dimensions. Using the stale cache here can
                // incorrectly cull a frame sitting on the viewport boundary.
                let sprite_position = entity.gameplay_sprite_position();
                let offset = sprite.current_offset();
                let sprite_box = crate::sprite::BBox::from_coords(
                    sprite_position.x + offset.x,
                    sprite_position.y + offset.y,
                    sprite_position.x + offset.x + f32::from(width),
                    sprite_position.y + offset.y + f32::from(height),
                );
                if !sprite_box.is_intersecting(&view) {
                    return None;
                }

                // Target-sprite creation resets masking before applying the current
                // grid mask list. Keep this serialized presentation cache in step
                // with the same mask query used by the renderer.
                let kind = entity.kind();
                let masked = if kind.has_valid_box_for_masking() {
                    let world_box = crate::coordinates::MapBBox::from_coords(
                        sprite_box.min.x,
                        sprite_box.min.y,
                        sprite_box.max.x,
                        sprite_box.max.y,
                    );
                    let is_flying_human = element.posture() == crate::element::Posture::Flying;
                    if is_flying_human || kind.is_projectile() {
                        !self
                            .engine
                            .inner
                            .fast_grid()
                            .get_masks_applied_to_projectile(
                                self.engine.inner.fast_grid().level.special_layer,
                                &world_box,
                                element.position(),
                                is_flying_human,
                                self.engine.inner.sight_obstacles(assets),
                            )
                            .is_empty()
                    } else {
                        !self
                            .engine
                            .inner
                            .fast_grid()
                            .get_masks_applied_to_character(
                                element.layer(),
                                &world_box,
                                element.position_map(),
                            )
                            .is_empty()
                    }
                } else {
                    false
                };
                Some((id, width, height, masked))
            })
            .collect::<Vec<_>>();

        for (id, width, height, masked) in updates {
            let sprite = self
                .engine
                .inner
                .world
                .entities
                .get_mut(id)
                .expect("presentation refresh entity disappeared");
            let sprite = sprite.sprite_mut();
            sprite.current_width = width;
            sprite.current_height = height;
            sprite.masked = masked;
        }
    }

    /// Advance a frame whose commands were independently recorded by the
    /// Original parity tracer.
    ///
    /// Nested-selection recording retains raw-mouse depth-2
    /// messages while omitting the depth-3 restitution emitted by `SelectPc`.
    /// Consequently adjacent recorded commands are siblings, not a root and
    /// its nested callback. Keeping this policy behind the explicit parity
    /// capability prevents it from changing live or rollback semantics.
    pub fn advance_frame(
        &mut self,
        assets: &LevelAssets,
        frame: SimulationFrameInput,
    ) -> Result<SimulationFrameOutput, FrameAdvanceError> {
        self.engine.advance_frame_with_command_batch_mode(
            assets,
            frame,
            SelectionCommandBatchMode::IndependentRecordedMessages,
        )
    }

    #[doc(hidden)]
    pub fn has_pending_recorded_drop_ale_route(
        &self,
        actor: EntityId,
        destination: crate::coordinates::MapPoint,
    ) -> bool {
        self.engine
            .has_pending_recorded_drop_ale_route(actor, destination)
    }

    #[doc(hidden)]
    pub fn restore_npc_maximal_visibility(&mut self, id: EntityId, value: u16) {
        self.engine.restore_parity_npc_maximal_visibility(id, value);
    }

    #[doc(hidden)]
    pub fn restore_npc_dormant_macro_cursor(
        &mut self,
        id: EntityId,
        path_id: crate::ai::PathId,
        waypoint_index: u8,
        offset: usize,
        assets: &LevelAssets,
    ) -> bool {
        self.engine.restore_parity_npc_dormant_macro_cursor(
            id,
            path_id,
            waypoint_index,
            offset,
            assets,
        )
    }

    pub fn append_rng_draws(&mut self, draws: Vec<u32>) {
        self.engine.append_original_rng_replay(draws);
    }

    pub fn replace_rng_draws(&mut self, draws: Vec<u32>) {
        self.engine.replace_original_rng_replay(draws);
    }

    pub fn set_impossible_action_done_deadlines(
        &mut self,
        deadlines: impl IntoIterator<Item = (u32, u32, i16)>,
    ) {
        self.engine
            .set_original_impossible_action_done_deadlines(deadlines);
    }

    pub fn use_external_director_completions(&mut self, enabled: bool) {
        self.engine.set_external_director_completion_replay(enabled);
    }
}

impl EngineInner {
    /// Number of original raw RNG values consumed so far, when parity replay is active.
    pub fn original_rng_replay_cursor(&self) -> Option<usize> {
        self.control.rng.original_replay_cursor()
    }

    /// Complete deterministic configuration of this read-only world view.
    pub fn sim_config(&self) -> SimConfig {
        self.control.sim_config
    }

    pub fn doors(&self) -> &[crate::gate::Door] {
        &self.script_domains.interactables.doors
    }

    pub fn patches(&self) -> &[crate::patch::Patch] {
        &self.script_domains.interactables.patches
    }
}

impl Deref for Engine {
    type Target = EngineInner;

    fn deref(&self) -> &EngineInner {
        &self.inner
    }
}

// `Default for Engine` is intentionally not implemented: the RAII
// contract says an `Engine` exists only when it's a fully-initialised
// mission engine, and the required mission data can't be conjured
// from defaults.  Tests that want a blank engine should construct
// `EngineInner` directly (it stays `pub(crate)` for that use), or
// fabricate a test-fixture level and go through `Engine::new`.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::SimCommand;

    #[test]
    fn engine_serde_facade_preserves_exact_wire_shape_and_roundtrip_hash() {
        let (engine, assets) = frame_api_fixture();
        let historical_bytes = serde_json::to_vec(&engine.inner).expect("historical inner codec");
        let bytes = serde_json::to_vec(&engine).expect("authoritative facade codec");
        assert_eq!(bytes, historical_bytes);
        let decoded: Engine = serde_json::from_slice(&bytes).expect("decode facade snapshot");
        assert!(!decoded.bootstrap_open);
        let restored = Engine::adopt_authoritative_snapshot(decoded, &assets)
            .expect("attach decoded snapshot resources");
        assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
        assert_eq!(
            crate::replay::state_hash(&restored),
            crate::replay::state_hash(&engine)
        );
    }

    #[test]
    fn bootstrap_authority_is_not_snapshot_state() {
        let (mut engine, assets) = frame_api_fixture();
        assert!(engine.bootstrap_open);
        assert!(!engine.clone().bootstrap_open);
        let decoded = Engine::decode_native_snapshot(&engine.encode_native_snapshot()).unwrap();
        assert!(!decoded.bootstrap_open);
        let restored = Engine::from_persisted_state(engine.capture_persisted_state().unwrap());
        assert!(!restored.bootstrap_open);
        // Adoption must also close authority when handed a freshly constructed
        // engine directly, rather than relying on the decoder to have done so.
        let (fresh, _) = frame_api_fixture();
        let adopted = Engine::adopt_authoritative_snapshot(fresh, &assets).unwrap();
        assert!(!adopted.bootstrap_open);
        let (fresh, _) = frame_api_fixture();
        let restored = Engine::restore_from_snapshot(
            &mut super::super::HostDisplayState::default(),
            fresh,
            &assets,
        )
        .unwrap();
        assert!(!restored.bootstrap_open);
        let bytes = serde_json::to_vec(&engine).unwrap();
        let decoded: Engine = serde_json::from_slice(&bytes).unwrap();
        assert!(!decoded.bootstrap_open);
        let hash = crate::replay::state_hash(&engine);
        assert_eq!(hash, crate::replay::state_hash(&engine.inner));
        engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
        assert_eq!(crate::replay::state_hash(&engine), hash);
        assert_eq!(serde_json::to_vec(&engine).unwrap(), bytes);
    }

    #[test]
    fn bootstrap_allows_pre_frame_zero_setup_admission() {
        let (mut engine, assets) = frame_api_fixture();
        engine
            .advance_frame(&assets, SimulationFrameInput::no_hourglass())
            .unwrap();
        engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
        assert!(!engine.bootstrap_open);
    }

    #[test]
    fn debrief_completion_preserves_clock_snapshot_and_hash() {
        let (mut engine, _) = frame_api_fixture();
        engine
            .inner
            .mission_domain
            .campaign
            .set_value(crate::campaign::CampaignValue::MissionLength, 23);
        let bytes = serde_json::to_vec(&engine).unwrap();
        let hash = crate::replay::state_hash(&engine);
        engine.finish_mission_bootstrap(MissionBootstrapCompletion::DebriefOnly);
        assert!(!engine.bootstrap_open);
        assert_eq!(
            engine.campaign().values[crate::campaign::CampaignValue::MissionLength],
            23
        );
        assert_eq!(serde_json::to_vec(&engine).unwrap(), bytes);
        assert_eq!(crate::replay::state_hash(&engine), hash);
        for completion in [
            MissionBootstrapCompletion::StartClock,
            MissionBootstrapCompletion::DebriefOnly,
        ] {
            let json = serde_json::to_string(&completion).unwrap();
            assert_eq!(
                serde_json::from_str::<MissionBootstrapCompletion>(&json).unwrap(),
                completion
            );
        }
    }

    #[test]
    #[should_panic(expected = "mission bootstrap authority is closed")]
    fn debrief_completion_cannot_later_start_clock() {
        let (mut engine, _) = frame_api_fixture();
        engine.finish_mission_bootstrap(MissionBootstrapCompletion::DebriefOnly);
        engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
    }

    #[test]
    #[should_panic(expected = "mission bootstrap authority is closed")]
    fn bootstrap_cannot_be_finished_twice() {
        let (mut engine, _) = frame_api_fixture();
        engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
        engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
    }

    #[test]
    #[should_panic(expected = "mission bootstrap authority is closed")]
    fn bootstrap_cannot_be_reopened_by_clone() {
        let (engine, _) = frame_api_fixture();
        engine
            .clone()
            .finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
    }

    #[test]
    #[should_panic(expected = "mission bootstrap authority is closed")]
    fn bootstrap_cannot_be_reopened_by_deserialization() {
        let (engine, _) = frame_api_fixture();
        let mut decoded: Engine =
            serde_json::from_value(serde_json::to_value(engine).unwrap()).unwrap();
        decoded.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
    }

    #[test]
    #[should_panic(expected = "mission bootstrap authority is closed")]
    fn bootstrap_cannot_be_reopened_by_snapshot_adoption() {
        let (engine, assets) = frame_api_fixture();
        Engine::adopt_authoritative_snapshot(engine, &assets)
            .unwrap()
            .finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
    }

    #[test]
    #[should_panic(expected = "mission bootstrap authority is closed")]
    fn first_hourglass_closes_bootstrap_even_without_explicit_finish() {
        let (mut engine, assets) = frame_api_fixture();
        engine
            .advance_frame(&assets, SimulationFrameInput::new(Vec::new()))
            .unwrap();
        engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
    }

    #[test]
    fn post_initialize_without_vm_records_the_stage_for_replay() {
        let (mut live, assets) = frame_api_fixture();
        assert!(live.inner.scripts.mission.is_none());
        live.inner.control.sim_config.script_enabled = true;
        let mut replay = live.clone();
        let output = live
            .advance_frame(
                &assets,
                SimulationFrameInput::no_hourglass().with_post_initialize(true),
            )
            .unwrap();
        let recorded_stage = output.post_initialize_events.is_some();
        assert!(
            recorded_stage,
            "the latch mutation must be recorded even without VM effects"
        );
        assert_eq!(live.parity_game_ui_state()["post_initialized"], true);
        let replay_output = replay
            .advance_frame(
                &assets,
                SimulationFrameInput::no_hourglass().with_post_initialize(recorded_stage),
            )
            .unwrap();
        assert_eq!(replay_output.state_hash, output.state_hash);
        assert_eq!(replay.parity_game_ui_state(), live.parity_game_ui_state());
        assert!(
            live.advance_frame(
                &assets,
                SimulationFrameInput::no_hourglass().with_post_initialize(true)
            )
            .unwrap()
            .post_initialize_events
            .is_none()
        );
    }

    #[test]
    fn bootstrap_accepts_an_imported_nonzero_initial_frame() {
        let (mut engine, _) = frame_api_fixture();
        let mut imported = engine.inner.clone_authoritative_state();
        imported.control.frame_counter = 98_765;
        imported
            .mission_domain
            .campaign
            .set_value(crate::campaign::CampaignValue::MissionLength, 23);
        engine.install_legacy_adoption_inner(imported);
        engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
        assert_eq!(engine.frame_counter(), 98_765);
        assert_eq!(
            engine.campaign().values[crate::campaign::CampaignValue::MissionLength],
            0
        );
        assert!(!engine.bootstrap_open);
    }

    #[test]
    #[should_panic(expected = "mission bootstrap authority is closed")]
    fn legacy_adoption_cannot_reopen_finished_bootstrap() {
        let (mut engine, _) = frame_api_fixture();
        engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
        let (fresh, _) = frame_api_fixture();
        engine.install_legacy_adoption_inner(fresh.inner);
        engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
    }

    fn frame_api_fixture() -> (Engine, LevelAssets) {
        let mut assets = LevelAssets::new();
        let sim_config = SimConfig {
            script_enabled: false,
            ignore_default_loose: true,
            ..Default::default()
        };
        let engine = Engine::new_for_test_with_simulation(
            1024.0,
            768.0,
            Campaign::default(),
            &mut assets,
            0xF4A6_E001,
            sim_config,
        )
        .expect("construct frame API fixture");
        (engine, assets)
    }

    #[test]
    fn tick_admission_crosses_pending_arrow_refresh_before_hourglass() {
        use crate::coordinates::WorldPoint3D;
        use crate::element::{
            ElementData, ElementKind, ElementProjectile, Entity, ObjectData, ObjectType,
            ProjectileData,
        };

        let (mut engine, assets) = frame_api_fixture();
        let mut element = {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ObjectProjectile;
            initial_element.active = true;
            initial_element
        };
        element
            .sprite
            .position_iface
            .set_old_position(WorldPoint3D::new(-1.0, 0.0, 0.0));
        let arrow = engine
            .inner
            .add_entity(Entity::Projectile(ElementProjectile {
                element,
                object: ObjectData {
                    object_type: ObjectType::Arrow,
                    ..Default::default()
                },
                projectile: ProjectileData {
                    flying: true,
                    falling: true,
                    falling_direction: 6,
                    ..Default::default()
                },
            }));
        engine.inner.control.arrow_refresh_pending = true;

        engine
            .advance_frame(&assets, SimulationFrameInput::default())
            .expect("admit simulation tick");

        let Entity::Projectile(arrow) = engine.inner.get_entity(arrow).unwrap() else {
            unreachable!()
        };
        // The admitted refresh was crossed (the sprite assertions below see
        // it); this tick then schedules the following presentation refresh.
        assert!(engine.inner.control.arrow_refresh_pending);
        assert_eq!(arrow.element.sprite.current_row, 6);
        assert!((3..=5).contains(&arrow.element.sprite.current_frame));
        assert_eq!(arrow.projectile.falling_direction, 4);
    }

    #[test]
    fn no_hourglass_admission_leaves_arrow_refresh_pending() {
        let (mut engine, assets) = frame_api_fixture();
        engine.inner.control.arrow_refresh_pending = true;

        engine
            .advance_frame(&assets, SimulationFrameInput::no_hourglass())
            .expect("admit no-hourglass boundary");

        assert!(engine.inner.control.arrow_refresh_pending);
    }

    fn typed_sentinel_snapshot_fixture() -> (Engine, EntityId) {
        let mut inner = EngineInner::new();
        let mut ai = crate::ai_enemy::EnemyAi::new(0);
        ai.base.primary_target = Some(crate::ai::AiEntityHandle::new(0));
        ai.base.seek_position.sector = Some(crate::position_interface::SectorHandle::from_number(
            crate::sector::SectorNumber::new(-1),
        ));
        ai.base.initial_position.sector = Some(
            crate::position_interface::SectorHandle::new(23)
                .unwrap()
                .with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap()),
        );
        ai.base.detached_patrol_path_status = crate::ai::DetachedPatrolPathStatus {
            hiking_path_index: crate::ai::PathId::new(3),
            current_waypoint_index: 5,
            last_waypoint_index: 7,
            forward: false,
            history: vec![crate::ai::PathHistoryEntry {
                position: crate::ai::Position::default(),
                direction: 9,
                distance: 11,
            }],
        };
        let id = inner.add_entity(crate::element::Entity::Soldier(
            crate::element::ActorSoldier {
                element: {
                    let mut initial_element = crate::element::ElementData::default();
                    initial_element.kind = crate::element::ElementKind::ActorSoldier;
                    initial_element
                },
                actor: Default::default(),
                human: Default::default(),
                npc: {
                    crate::element::NpcData {
                        ai: crate::element::AiActorData {
                            ai_brain: crate::element::AiBrain::Enemy(Box::new(ai)),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                },
                soldier: Default::default(),
            },
        ));
        assert_eq!(id.index(), 0, "fixture must occupy live arena slot zero");
        (
            Engine {
                inner,
                bootstrap_open: false,
            },
            id,
        )
    }

    fn assert_typed_sentinel_snapshot(engine: &Engine, id: EntityId) {
        let ai = engine
            .inner
            .get_entity(id)
            .and_then(crate::element::Entity::enemy_ai)
            .expect("typed sentinel fixture retains EnemyAi");
        assert_eq!(
            ai.base.primary_target,
            Some(crate::ai::AiEntityHandle::new(0))
        );
        let signed_sector = ai.base.seek_position.sector.unwrap();
        assert_eq!(signed_sector.number().get(), -1);
        assert_eq!(signed_sector.arena_index(), None);
        let exact_sector = ai.base.initial_position.sector.unwrap();
        assert_eq!(exact_sector.number().get(), 23);
        assert_eq!(
            exact_sector.arena_index(),
            crate::fast_find_grid::SectorIndex::new(0)
        );
        let detached = &ai.base.detached_patrol_path_status;
        assert_eq!(detached.hiking_path_index, crate::ai::PathId::new(3));
        assert_eq!(detached.current_waypoint_index, 5);
        assert_eq!(detached.last_waypoint_index, 7);
        assert!(!detached.forward);
        assert_eq!(detached.history.len(), 1);
        assert_eq!(detached.history[0].direction, 9);
        assert_eq!(detached.history[0].distance, 11);
    }

    fn sherwood_trading_frame_fixture() -> (Engine, LevelAssets) {
        use crate::element::{ElementBonus, ElementData, ElementKind, Entity, ObjectData};
        use crate::mission::Mission;
        use crate::profiles::{Action, MissionLocation, MissionProfile, ProfileManager};

        let mut profiles = ProfileManager::default();
        profiles.missions.push(MissionProfile {
            location: MissionLocation::Sherwood,
            ..MissionProfile::default()
        });
        let mut assets = LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..LevelAssets::default()
        };
        let mut campaign = Campaign::default();
        campaign.missions.push(Mission {
            profile_idx: Some(0),
            ..Mission::default()
        });
        campaign.current_mission_idx = Some(0);
        let mut engine = Engine::new_for_test_with_simulation(
            1024.0,
            768.0,
            campaign,
            &mut assets,
            0x7A4D_E001,
            SimConfig {
                sherwood_trading: true,
                script_enabled: false,
                ignore_default_loose: true,
                ..SimConfig::default()
            },
        )
        .expect("construct Sherwood trading frame fixture");
        engine
            .inner
            .world
            .entities
            .push(Some(Entity::Bonus(ElementBonus {
                element: {
                    let mut initial_element = ElementData::default();
                    initial_element.kind = ElementKind::ObjectBonus;
                    initial_element.active = true;
                    initial_element
                },
                object: ObjectData {
                    associated_action: Action::Bow,
                    quantity: 1,
                    ..ObjectData::default()
                },
            })));
        (engine, assets)
    }

    #[test]
    fn paused_post_boundary_trade_delivers_its_receipt_in_the_same_transaction() {
        use crate::player_command::PlayerCommand;
        use crate::sector_production::Type;
        use crate::trading::{TradeOutcome, TradeQuantity};

        let (mut engine, assets) = sherwood_trading_frame_fixture();
        let output = engine
            .advance_frame(
                &assets,
                SimulationFrameInput::no_hourglass().with_post_commands(vec![SimCommand::host(
                    PlayerCommand::CampaignSellProductionItem {
                        request_id: 77,
                        prod_type: Type::MakeArrow,
                        quantity: TradeQuantity::One,
                    },
                )]),
            )
            .expect("admit paused modal trade");

        assert!(!output.hourglass_ran);
        assert!(output.events.side_effects().trade_receipts.is_empty());
        let receipts = &output.post_boundary_events.side_effects().trade_receipts;
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].request_id, 77);
        assert!(matches!(
            receipts[0].outcome,
            TradeOutcome::Sold {
                units: 1,
                remaining_stock: 0,
                ..
            }
        ));
    }

    #[test]
    fn item_rules_apply_on_the_command_frame_and_survive_native_snapshot() {
        let (mut engine, assets) = frame_api_fixture();
        let rules = crate::gameplay_config::ItemGameplayConfig {
            apple_combat_interrupt: true,
            wasp_reliable_acquisition: false,
            stone_ground_distraction: true,
            stone_longer_range: false,
            net_selective_immunity: true,
            ale_reliable_distraction: false,
        };
        engine
            .advance_frame(
                &assets,
                SimulationFrameInput::new(vec![
                    PlayerCommand::SetItemGameplayConfig { config: rules }.into(),
                ])
                .with_hourglass(false),
            )
            .expect("item rules command frame");
        assert_eq!(engine.sim_config().item_gameplay, rules);

        let restored = Engine::decode_native_snapshot(&engine.encode_native_snapshot())
            .expect("decode item rules snapshot");
        assert_eq!(restored.sim_config().item_gameplay, rules);
    }

    #[test]
    fn ranked_policy_rejects_timed_and_ambience_noop_commands_in_both_phases() {
        use crate::engine::{RankedSimulationConfigField, SimulationCommandPhase};

        for (command, field) in [
            (
                PlayerCommand::SetTimedMissionsEnabled { enabled: true },
                RankedSimulationConfigField::EnableTimedMissions,
            ),
            (
                PlayerCommand::SetDynamicAmbienceEnabled { enabled: true },
                RankedSimulationConfigField::EnableDynamicAmbience,
            ),
        ] {
            for phase in [
                SimulationCommandPhase::PreHourglass,
                SimulationCommandPhase::PostHourglass,
            ] {
                let (mut engine, assets) = frame_api_fixture();
                let policy = crate::engine::RankedSimulationPolicy::standard_medium();
                let config = policy.expected_config();
                engine.inner.control.sim_config = config;
                engine.inner.control.mission_start_sim_config = config;
                engine
                    .inner
                    .control
                    .install_ranked_simulation_policy(policy);
                let ranked_noop = SimCommand::host(command.clone());
                let input = match phase {
                    SimulationCommandPhase::PreHourglass => {
                        SimulationFrameInput::new(vec![ranked_noop]).with_hourglass(false)
                    }
                    SimulationCommandPhase::PostHourglass => {
                        SimulationFrameInput::no_hourglass().with_post_commands(vec![ranked_noop])
                    }
                };

                let error = engine
                    .advance_frame(&assets, input)
                    .expect_err("ranked no-op setting command must be rejected");
                assert_eq!(
                    error,
                    FrameAdvanceError::RankedSimulationSettingCommandRejected {
                        phase,
                        index: 0,
                        field,
                    }
                );
                assert_eq!(engine.sim_config(), config);
            }
        }
    }

    #[test]
    fn installing_original_parity_replay_forces_classic_item_rules() {
        let (mut engine, assets) = frame_api_fixture();
        engine.inner.control.sim_config.item_gameplay =
            crate::gameplay_config::ItemGameplayConfig::default();
        engine.inner.control.sim_config.noise_distraction_feedback = true;
        engine
            .parity_replay_setup()
            .replace_rng_draws(vec![0x1234_5678]);

        assert_eq!(
            engine.sim_config().item_gameplay,
            crate::gameplay_config::ItemGameplayConfig::classic()
        );
        assert!(!engine.sim_config().noise_distraction_feedback);

        engine
            .advance_frame(
                &assets,
                SimulationFrameInput::new(vec![
                    PlayerCommand::SetItemGameplayConfig {
                        config: crate::gameplay_config::ItemGameplayConfig::default(),
                    }
                    .into(),
                    PlayerCommand::SetNoiseDistractionFeedback { enabled: true }.into(),
                ])
                .with_hourglass(false),
            )
            .expect("Original-parity settings command frame");
        assert_eq!(
            engine.sim_config().item_gameplay,
            crate::gameplay_config::ItemGameplayConfig::classic()
        );
        assert!(!engine.sim_config().noise_distraction_feedback);
    }

    #[test]
    fn native_snapshot_decodes_through_the_engine_facade() {
        let (mut engine, _) = frame_api_fixture();
        engine.inner.feedback.cutscene_camera.view_position =
            crate::coordinates::MapPoint::new(73.0, 91.0);
        engine.inner.feedback.cutscene_camera.zoom_factor = 2.0;
        let expected_hash = crate::replay::state_hash(&engine);
        let bytes = engine.encode_native_snapshot();

        let decoded = Engine::decode_native_snapshot(&bytes)
            .expect("decode the native Engine wire layout through the facade");

        assert_eq!(crate::replay::state_hash(&decoded), expected_hash);
        assert_eq!(
            decoded.inner.feedback.cutscene_camera.view_position,
            engine.inner.feedback.cutscene_camera.view_position
        );
        assert_eq!(decoded.inner.feedback.cutscene_camera.zoom_factor, 2.0);
    }

    #[test]
    fn rollback_native_snapshot_round_trips_typed_slot_zero_and_spatial_provenance() {
        std::thread::Builder::new()
            .name("typed-sentinel-rollback-snapshot".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(
                rollback_native_snapshot_round_trips_typed_slot_zero_and_spatial_provenance_inner,
            )
            .expect("spawn large-stack rollback snapshot test")
            .join()
            .expect("rollback snapshot test panicked");
    }

    fn rollback_native_snapshot_round_trips_typed_slot_zero_and_spatial_provenance_inner() {
        let (engine, id) = typed_sentinel_snapshot_fixture();
        let bytes = engine.encode_native_snapshot();

        let decoded = Engine::decode_native_snapshot(&bytes).expect("decode rollback snapshot");

        assert_typed_sentinel_snapshot(&decoded, id);
        let present_hash = crate::replay::state_hash(&decoded);
        let mut absent = decoded;
        absent
            .inner
            .get_entity_mut(id)
            .and_then(crate::element::Entity::enemy_ai_mut)
            .unwrap()
            .base
            .primary_target = None;
        assert_ne!(
            present_hash,
            crate::replay::state_hash(&absent),
            "rollback hashing must distinguish live slot zero from absence"
        );
    }

    #[test]
    fn network_initial_snapshot_round_trips_typed_slot_zero_and_spatial_provenance() {
        std::thread::Builder::new()
            .name("typed-sentinel-network-snapshot".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(
                network_initial_snapshot_round_trips_typed_slot_zero_and_spatial_provenance_inner,
            )
            .expect("spawn large-stack network snapshot test")
            .join()
            .expect("network snapshot test panicked");
    }

    fn network_initial_snapshot_round_trips_typed_slot_zero_and_spatial_provenance_inner() {
        let (engine, id) = typed_sentinel_snapshot_fixture();
        let message = crate::multiplayer::NetMsg::InitialSnapshot {
            frame: 37,
            engine_bytes: engine.encode_native_snapshot(),
        };

        let decoded_message =
            crate::multiplayer::decode_msg(&crate::multiplayer::encode_msg(&message))
                .expect("decode network message");
        let crate::multiplayer::NetMsg::InitialSnapshot {
            frame,
            engine_bytes,
        } = decoded_message
        else {
            panic!("network message changed variant")
        };
        assert_eq!(frame, 37);
        let decoded = Engine::decode_native_snapshot(&engine_bytes)
            .expect("decode network-carried engine snapshot");
        assert_typed_sentinel_snapshot(&decoded, id);
    }

    fn pending_drop_ale_seek(
        owner: EntityId,
        destination: crate::coordinates::MapPoint,
        fallback_sector: crate::position_interface::SectorHandle,
    ) -> crate::sequence::SequenceElement {
        use crate::element::Command;
        use crate::sequence::{MoveFlags, Sequence, SequenceElement, SequenceElementData};

        let mut seek = SequenceElement::new_movement(
            1,
            Command::Seek,
            Some(owner),
            crate::order::OrderType::WalkingUpright,
        );
        seek.point_seek_route_provenance =
            crate::sequence::PointSeekRouteProvenance::OriginalReplay;
        let SequenceElementData::Movement {
            destination: seek_destination,
            layer,
            sector,
            flags,
            post_seek_sequence,
            ..
        } = &mut seek.data
        else {
            unreachable!()
        };
        *seek_destination = destination;
        *layer = 2;
        *sector = Some(fallback_sector);
        *flags |= MoveFlags::SEEK;
        let mut post_seek = Sequence::new();
        post_seek.append_element(SequenceElement::new(1, Command::DropAle, Some(owner)));
        *post_seek_sequence = Some(post_seek.into_post_seek());
        seek
    }

    fn recorded_drop_ale_fact(
        actor: EntityId,
        destination: crate::coordinates::MapPoint,
    ) -> RecordedDropAleRoute {
        RecordedDropAleRoute {
            actor,
            destination,
            goal_sector: crate::sector::SectorNumber::new(0),
            goal_sector_index: crate::fast_find_grid::SectorIndex::new(0)
                .expect("sector index zero is valid"),
            goal_layer: 0,
            recorded_gate_path: crate::gate::RecordedGatePath {
                source_sector: crate::sector::SectorNumber::new(133),
                source_sector_index: crate::fast_find_grid::SectorIndex::new(57),
                source_layer: 11,
                outcome: crate::gate::RecordedGateOutcome::Failure,
            },
        }
    }

    fn selection_boundary_fixture() -> (Engine, LevelAssets, EntityId, crate::sequence::SequenceId)
    {
        let (mut engine, mut assets) = frame_api_fixture();
        let mut actions =
            [crate::profiles::Action::NoAction; crate::profiles::NUMBER_OF_PC_ACTIONS];
        actions[0] = crate::profiles::Action::Net;
        let mut maximum_ammo = [0; crate::profiles::NUMBER_OF_PC_ACTIONS];
        maximum_ammo[0] = 1;
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles
            .missions
            .push(crate::profiles::MissionProfile::default());
        profiles.characters.push(crate::profiles::CharacterProfile {
            actions,
            action_max_ammo: maximum_ammo,
            ..Default::default()
        });
        assets.profile_manager = std::sync::Arc::new(profiles);

        engine
            .inner
            .mission_domain
            .campaign
            .characters
            .push(crate::campaign::PcDescription {
                character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
                instanced: true,
                ..Default::default()
            });
        let pc_id = engine
            .inner
            .add_entity(crate::element::Entity::Pc(crate::element::ActorPc {
                element: {
                    let mut initial_element = crate::element::ElementData::from_initial_posture(
                        crate::element::Posture::Upright,
                    );
                    initial_element.kind = crate::element::ElementKind::ActorPc;
                    initial_element.active = true;
                    initial_element
                },
                actor: crate::element::ActorData::default(),
                human: crate::element::HumanData::default(),
                pc: crate::element::PcData {
                    profile_index: crate::profiles::CharacterProfileIdx(0),
                    campaign_description_index: Some(0),
                    life_points: 50,
                    current_action: crate::profiles::Action::Net,
                    ..Default::default()
                },
            }));

        let mut wait = crate::sequence::SequenceElement::new_generic(
            1,
            crate::element::Command::WaitTimer,
            Some(pc_id),
        );
        wait.priority = crate::sequence::SequencePriority::Wait;
        let wait_sequence = engine.inner.orders.sequence_manager.launch_element(wait);
        engine
            .inner
            .orders
            .sequence_manager
            .element_in_progress(wait_sequence, 0);
        (engine, assets, pc_id, wait_sequence)
    }

    #[test]
    fn spatial_presentation_sampling_is_absolute_and_authoritative_hashes_are_unchanged() {
        let (mut previous, _, pc_id, _) = selection_boundary_fixture();
        previous
            .inner
            .get_entity_mut(pc_id)
            .expect("presentation test PC")
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::ZERO);
        let mut current = previous.clone();
        {
            let entity = current
                .inner
                .get_entity_mut(pc_id)
                .expect("presentation test PC");
            entity
                .element_data_mut()
                .set_position(crate::coordinates::WorldPoint3D::new(40.0, 60.0, 10.0));
            entity
                .actor_data_mut()
                .expect("presentation test actor")
                .jump_z_offset = 8.0;
        }
        let previous_hash = crate::replay::state_hash(&previous);
        let current_hash = crate::replay::state_hash(&current);
        let previous_spatial = previous.spatial_presentation_snapshot();
        let current_spatial = current.spatial_presentation_snapshot();
        let mut presentation = PresentationEngine::new(&current);

        presentation.apply_spatial_presentation(&previous_spatial, &current_spatial, 0.25);
        let first_sample_hash = crate::replay::state_hash(&presentation.presentation);
        let sampled = presentation.view().get_entity(pc_id).expect("sampled PC");
        assert_eq!(
            sampled.element_data().position(),
            crate::coordinates::WorldPoint3D::new(10.0, 15.0, 2.5)
        );
        assert_eq!(
            sampled.element_data().position_map(),
            crate::coordinates::MapPoint::new(10.0, 12.5)
        );
        assert_eq!(
            sampled.actor_data().expect("sampled actor").jump_z_offset,
            2.0
        );

        presentation.apply_spatial_presentation(&previous_spatial, &current_spatial, 0.25);
        assert_eq!(
            crate::replay::state_hash(&presentation.presentation),
            first_sample_hash,
            "repeating one display sample must be idempotent"
        );
        assert_eq!(crate::replay::state_hash(&previous), previous_hash);
        assert_eq!(crate::replay::state_hash(&current), current_hash);
    }

    #[test]
    fn spatial_presentation_snaps_layer_transitions_and_new_entities() {
        let (previous, _, pc_id, _) = selection_boundary_fixture();
        let mut current = previous.clone();
        {
            let element = current
                .inner
                .get_entity_mut(pc_id)
                .expect("presentation test PC")
                .element_data_mut();
            element.set_position_map(crate::coordinates::MapPoint::new(64.0, 96.0));
            element.set_layer(1);
        }
        let spawned_id =
            current
                .inner
                .add_entity(crate::element::Entity::Fx(crate::element::ElementFx {
                    element: {
                        let mut initial_element = crate::element::ElementData::default();
                        initial_element.kind = crate::element::ElementKind::Fx;
                        initial_element
                    },
                    fx: Default::default(),
                }));
        current
            .inner
            .get_entity_mut(spawned_id)
            .expect("spawned presentation FX")
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint::new(12.0, 34.0));
        let previous_spatial = previous.spatial_presentation_snapshot();
        let current_spatial = current.spatial_presentation_snapshot();
        let mut presentation = PresentationEngine::new(&current);

        presentation.apply_spatial_presentation(&previous_spatial, &current_spatial, 0.0);

        assert_eq!(
            presentation
                .view()
                .get_entity(pc_id)
                .expect("sampled PC")
                .element_data()
                .position_map(),
            crate::coordinates::MapPoint::new(64.0, 96.0),
            "layer transition must snap rather than sweep"
        );
        assert_eq!(
            presentation
                .view()
                .get_entity(spawned_id)
                .expect("sampled spawned FX")
                .element_data()
                .position_map(),
            crate::coordinates::MapPoint::new(12.0, 34.0),
            "spawned entity must use its current fixed-tick transform"
        );
    }

    #[test]
    fn presentation_diagnostics_cannot_restore_live_presentation_or_simulation() {
        let (engine, _, _, _) = selection_boundary_fixture();
        let presentation = PresentationEngine::new(&engine);
        let diagnostic = serde_json::to_value(&presentation).expect("presentation diagnostic");
        assert_eq!(diagnostic["frame"], engine.frame_counter());
        assert_eq!(diagnostic.as_object().expect("diagnostic object").len(), 2);
        assert!(serde_json::from_value::<PresentationEngine>(diagnostic.clone()).is_err());
        assert!(serde_json::from_value::<Engine>(diagnostic).is_err());
        for view in [engine.presentation_view(), presentation.view()] {
            let diagnostic = serde_json::to_value(view).expect("read-view diagnostic");
            assert_eq!(diagnostic.as_object().expect("diagnostic object").len(), 2);
            assert!(
                serde_json::from_value::<super::super::PresentationView<'_>>(diagnostic.clone())
                    .is_err()
            );
            assert!(serde_json::from_value::<Engine>(diagnostic).is_err());
        }
    }

    #[test]
    fn presentation_queries_preserve_fixed_world_results_and_snapshot_bytes() {
        use crate::player_command::PlayerId;
        let (engine, assets, pc, _) = selection_boundary_fixture();
        let snapshot = engine.encode_native_snapshot();
        let hash = crate::replay::state_hash(&engine);
        let presentation = PresentationEngine::new(&engine);
        for view in [engine.presentation_view(), presentation.view()] {
            assert_eq!(view.frame_counter(), engine.frame_counter());
            assert_eq!(view.pc_ids(), engine.pc_ids());
            assert_eq!(view.npc_ids(), engine.npc_ids());
            assert_eq!(view.displayed_pc_ids(), engine.displayed_pc_ids());
            assert_eq!(view.sort_for_minimap(), engine.sort_for_minimap());
            assert_eq!(view.fog_entity_visible(pc), engine.fog_entity_visible(pc));
            assert_eq!(
                view.fog_entity_is_hostile(pc),
                engine.fog_entity_is_hostile(pc)
            );
            assert_eq!(
                view.has_mission_geometry(),
                engine.mission_script().is_some()
            );
            assert_eq!(view.mission_won(), engine.mission().mission_won);
            assert_eq!(
                view.more_combat_gestures(),
                engine.sim_config().more_combat_gestures
            );
            assert_eq!(
                view.timed_missions_enabled(),
                engine.sim_config().enable_timed_missions
            );
            assert_eq!(
                view.uses_original_rng_replay(),
                engine.original_rng_replay_cursor().is_some()
            );
            assert_eq!(
                view.get_entity(pc).unwrap().element_data().position(),
                engine.get_entity(pc).unwrap().element_data().position()
            );
            assert_eq!(
                view.active_entity_positions().collect::<Vec<_>>(),
                engine.active_entity_positions().collect::<Vec<_>>()
            );
            assert_eq!(
                view.hero_selection(PlayerId::HOST),
                engine.hero_selection(PlayerId::HOST)
            );
            assert_eq!(
                view.tactical_selection(PlayerId::HOST),
                engine.tactical_selection(PlayerId::HOST)
            );
            assert_eq!(
                view.selected_action_for_seat(PlayerId::HOST),
                engine.selected_action_for_seat(PlayerId::HOST)
            );
            assert_eq!(
                view.planned_action_for_seat(PlayerId::HOST),
                engine.planned_action_for_seat(PlayerId::HOST)
            );
            assert_eq!(
                view.compute_display_order().ids,
                engine.compute_display_order().ids
            );
            assert_eq!(
                format!("{:?}", view.minimap_dot_info(pc, &assets)),
                format!("{:?}", engine.minimap_dot_info(pc, &assets))
            );
            assert_eq!(
                view.compute_display_order().depths,
                engine.compute_display_order().depths
            );
            assert_eq!(
                serde_json::to_value(view.campaign()).unwrap(),
                serde_json::to_value(engine.campaign()).unwrap()
            );
        }
        assert_eq!(engine.encode_native_snapshot(), snapshot);
        assert_eq!(crate::replay::state_hash(&engine), hash);
    }

    fn adjacent_select_and_cancel(pc_id: EntityId) -> Vec<SimCommand> {
        vec![
            SimCommand::from(PlayerCommand::SelectPc {
                pc_id,
                append: false,
            }),
            SimCommand::from(PlayerCommand::CancelAction { pc_id }),
        ]
    }

    #[test]
    fn original_parity_frame_preserves_pre_and_post_command_boundaries() {
        for post_hourglass in [false, true] {
            let (mut engine, assets, pc_id, wait_sequence) = selection_boundary_fixture();
            let commands = adjacent_select_and_cancel(pc_id);
            let frame = if post_hourglass {
                SimulationFrameInput::no_hourglass().with_post_commands(commands)
            } else {
                SimulationFrameInput::new(commands).with_hourglass(false)
            };
            engine
                .parity_replay_setup()
                .advance_frame(&assets, frame)
                .expect("advance Original parity frame");

            assert_eq!(
                engine
                    .inner
                    .orders
                    .sequence_manager
                    .get_element(wait_sequence, 0)
                    .expect("interrupted wait remains inspectable")
                    .state,
                crate::sequence::SequenceState::Interrupted,
                "{}-hourglass commands must remain independent recorded siblings",
                if post_hourglass { "post" } else { "pre" },
            );
        }
    }

    #[test]
    fn ordinary_frame_keeps_live_nested_selection_inference() {
        let (mut engine, assets, pc_id, wait_sequence) = selection_boundary_fixture();

        engine
            .advance_frame(
                &assets,
                SimulationFrameInput::new(adjacent_select_and_cancel(pc_id)).with_hourglass(false),
            )
            .expect("advance ordinary frame");

        assert_eq!(
            engine
                .inner
                .orders
                .sequence_manager
                .get_element(wait_sequence, 0)
                .expect("live wait remains inspectable")
                .state,
            crate::sequence::SequenceState::InProgress,
            "ordinary admission must retain its existing nested-selection heuristic",
        );
    }

    #[test]
    fn frame_api_matches_legacy_command_then_hourglass_boundary() {
        let (engine, assets) = frame_api_fixture();
        let mut legacy = engine.clone();
        let mut framed = engine;
        let commands = vec![PlayerInput::host(
            PlayerCommand::SetMenToBlazonConversionMode { on: true },
        )];

        let mut legacy_display = super::super::HostDisplayState::default();
        let mut legacy_input = InputState::default();
        let mut legacy_dev = DevState::default();
        legacy.apply_commands(&mut legacy_display, &mut legacy_input, &assets, &commands);
        let legacy_events = legacy.perform_hourglass(
            &mut legacy_display,
            &mut legacy_input,
            &assets,
            &mut legacy_dev,
        );
        let legacy_hash = crate::replay::state_hash(&legacy);

        let output = framed
            .advance_frame(&assets, SimulationFrameInput::from_player_inputs(commands))
            .expect("advance frame");

        assert_eq!(output.frame_before, 0);
        assert_eq!(output.frame_after, 1);
        assert_eq!(output.state_hash, legacy_hash);
        assert_eq!(crate::replay::state_hash(&framed), legacy_hash);
        assert_eq!(
            serde_json::to_value(output.events.side_effects()).expect("serialize frame events"),
            serde_json::to_value(&legacy_events).expect("serialize legacy side effects"),
        );
        assert_eq!(
            output.events.side_effects().pending_minimap_position,
            legacy_events.pending_minimap_position,
            "this host-local effect is serde-skipped and must be compared explicitly"
        );
        assert!(framed.is_men_to_blazon_conversion_mode());
    }

    #[test]
    fn recorded_drop_ale_facts_round_trip_and_reject_atomically() {
        let (mut engine, assets) = frame_api_fixture();
        let owner = EntityId::Pc(crate::entity_id::PcId(36));
        let destination = crate::coordinates::MapPoint::new(778.0, 1714.0);
        let fallback_sector =
            crate::position_interface::SectorHandle::new(25).expect("fallback sector is valid");
        engine
            .inner
            .orders
            .sequence_manager
            .launch_element(pending_drop_ale_seek(owner, destination, fallback_sector));

        let fact = recorded_drop_ale_fact(owner, destination);
        let input = SimulationFrameInput::no_hourglass().with_external_facts(
            ExternalFacts::default().with_recorded_drop_ale_routes(vec![fact.clone()]),
        );
        let encoded = bitcode::encode(&input);
        let decoded: SimulationFrameInput =
            bitcode::decode(&encoded).expect("decode typed frame fact");
        let mut direct = engine.clone();
        let mut replayed = engine.clone();
        direct
            .advance_frame(&assets, input)
            .expect("admit recorded DropAle route");
        replayed
            .advance_frame(&assets, decoded)
            .expect("replay recorded DropAle route");
        assert_eq!(
            crate::replay::state_hash(&direct),
            crate::replay::state_hash(&replayed),
            "the full frame journal must retain delayed route resolution"
        );

        let before = crate::replay::state_hash(&engine);
        let rejected = engine
            .advance_frame(
                &assets,
                SimulationFrameInput::no_hourglass().with_external_facts(
                    ExternalFacts::default().with_recorded_drop_ale_routes(vec![
                        fact,
                        recorded_drop_ale_fact(
                            EntityId::Pc(crate::entity_id::PcId(37)),
                            destination,
                        ),
                    ]),
                ),
            )
            .expect_err("a route without a pending DropAle seek must be rejected");
        assert!(matches!(
            rejected,
            FrameAdvanceError::RecordedDropAleRouteRejected { index: 1, .. }
        ));
        assert_eq!(
            crate::replay::state_hash(&engine),
            before,
            "a rejected later fact must not publish the accepted prefix"
        );
    }

    #[test]
    fn paused_campaign_actions_are_typed_no_hourglass_transactions() {
        let (mut engine, assets) = frame_api_fixture();
        engine
            .inner
            .mission_domain
            .campaign
            .last_pseudo_mission_status = crate::mission::MissionStatus::Won;
        let blazons_before = engine
            .campaign()
            .get_value(crate::campaign::CampaignValue::Blazon);

        let output = engine
            .advance_frame(
                &assets,
                SimulationFrameInput::no_hourglass().with_external_actions(vec![
                    ExternalAction::CampaignBuyBlazon { mission_index: 0 },
                    ExternalAction::AcknowledgePseudoMissionDebrief,
                ]),
            )
            .expect("admit paused campaign actions");

        assert!(!output.hourglass_ran);
        assert_eq!(output.frame_before, output.frame_after);
        assert!(matches!(
            output.external_action_results.as_slice(),
            [
                ExternalActionResult::CampaignBuyBlazon {
                    closed_by_cascade: false
                },
                ExternalActionResult::AcknowledgePseudoMissionDebrief
            ]
        ));
        assert_eq!(
            engine
                .campaign()
                .get_value(crate::campaign::CampaignValue::Blazon),
            blazons_before + 1
        );
        assert_eq!(
            engine.campaign().get_last_pseudo_mission_status(),
            crate::mission::MissionStatus::Available
        );
    }

    #[test]
    fn frame_api_applies_sound_external_fact_at_pre_hourglass_boundary() {
        use crate::sound::{ExclamationGroup, PendingExclamation, ResolvedExclamation};

        let (mut framed, assets) = frame_api_fixture();
        let profile_id = 0x4651_0000;
        framed
            .inner
            .feedback
            .sound_sim
            .pending_exclamations
            .push(PendingExclamation {
                actor_id: 191,
                group: ExclamationGroup::Civilian,
                profile_id,
                exclamation_id: 62,
                variant: -1,
            });
        let resolution = ResolvedExclamation {
            actor_id: 191,
            identifier: profile_id | 62,
            exclamation_id: 62,
            duration_frames: 24,
        };

        framed
            .advance_frame(
                &assets,
                SimulationFrameInput::new(vec![SimCommand::from(PlayerCommand::Noop)])
                    .with_external_facts(
                        ExternalFacts::default()
                            .with_sound_boundary(SoundBoundary::live(vec![resolution])),
                    ),
            )
            .expect("advance frame with sound fact");

        assert!(framed.sound_sim().resolved_exclamations.is_empty());
        assert_eq!(framed.sound_sim().playing_exclamations.len(), 1);
        assert_eq!(framed.sound_sim().playing_exclamations[0].actor_id, 191);
        assert_eq!(
            framed.sound_sim().playing_exclamations[0].finish_frame,
            24,
            "the fact is resolved at the frame-0 boundary before the hourglass increments the clock"
        );
    }

    fn ranked_sound_boundary_fixture(variant: i32) -> (Engine, LevelAssets) {
        use std::collections::BTreeMap;
        use std::sync::Arc;

        use crate::sound::{ExclamationGroup, PendingExclamation};

        let (mut engine, mut assets) = frame_api_fixture();
        let policy = crate::engine::RankedSimulationPolicy::standard_medium();
        let config = policy.expected_config();
        engine.inner.control.sim_config = config;
        engine.inner.control.mission_start_sim_config = config;
        engine
            .inner
            .control
            .install_ranked_simulation_policy(policy);

        let profile_id = 0x4651_0000;
        let exclamation_id = 62;
        engine
            .inner
            .feedback
            .sound_sim
            .pending_exclamations
            .push(PendingExclamation {
                actor_id: 191,
                group: ExclamationGroup::Civilian,
                profile_id,
                exclamation_id,
                variant,
            });
        assets.audio.speech_timing_catalog = Arc::new(crate::engine::SpeechTimingCatalog {
            groups: BTreeMap::from([(
                profile_id | u32::from(exclamation_id),
                crate::engine::SpeechTimingGroup {
                    gaps: 1,
                    variants: vec![
                        crate::engine::SpeechTimingVariant {
                            sample_identity: "speech-a.wav".into(),
                            duration_frames: Some(17),
                        },
                        crate::engine::SpeechTimingVariant {
                            sample_identity: "speech-b.wav".into(),
                            duration_frames: Some(29),
                        },
                    ],
                },
            )]),
        });
        (engine, assets)
    }

    fn ranked_sound_resolution(duration_frames: u32) -> crate::sound::ResolvedExclamation {
        crate::sound::ResolvedExclamation {
            actor_id: 191,
            identifier: 0x4651_003e,
            exclamation_id: 62,
            duration_frames,
        }
    }

    #[test]
    fn ranked_sound_boundary_accepts_only_sealed_authored_timing() {
        let (mut random_engine, random_assets) = ranked_sound_boundary_fixture(-1);
        random_engine
            .advance_frame(
                &random_assets,
                SimulationFrameInput::no_hourglass().with_external_facts(
                    ExternalFacts::default().with_sound_boundary(SoundBoundary::live(vec![
                        ranked_sound_resolution(29),
                    ])),
                ),
            )
            .expect("random playback uses the canonical maximum English duration");

        let (mut explicit_engine, explicit_assets) = ranked_sound_boundary_fixture(0);
        explicit_engine
            .advance_frame(
                &explicit_assets,
                SimulationFrameInput::no_hourglass().with_external_facts(
                    ExternalFacts::default().with_sound_boundary(SoundBoundary::live(vec![
                        ranked_sound_resolution(17),
                    ])),
                ),
            )
            .expect("an explicit speech choice must use its ordered sealed variant");
    }

    #[test]
    fn ranked_sound_boundary_rejects_forged_duration_and_variant() {
        for (variant, duration, expected_reason) in [
            (-1, 23, "unauthoritative duration"),
            (-1, 17, "unauthoritative duration"),
            (0, 29, "unauthoritative duration"),
            (2, 17, "outside the 2 authored variants"),
            (-2, 17, "invalid variant -2"),
        ] {
            let (mut engine, assets) = ranked_sound_boundary_fixture(variant);
            let before = crate::replay::state_hash(&engine);
            let error = engine
                .advance_frame(
                    &assets,
                    SimulationFrameInput::no_hourglass().with_external_facts(
                        ExternalFacts::default().with_sound_boundary(SoundBoundary::live(vec![
                            ranked_sound_resolution(duration),
                        ])),
                    ),
                )
                .expect_err("forged ranked speech timing must fail closed");
            let FrameAdvanceError::SoundBoundaryRejected { reason, .. } = error else {
                panic!("unexpected ranked sound error: {error:?}");
            };
            assert!(
                reason.contains(expected_reason),
                "unexpected rejection for variant {variant}: {reason}"
            );
            assert_eq!(crate::replay::state_hash(&engine), before);
        }
    }

    #[test]
    fn ranked_sound_boundary_rejects_replay_policy() {
        let (mut engine, assets) = ranked_sound_boundary_fixture(-1);
        let before = crate::replay::state_hash(&engine);
        let error = engine
            .advance_frame(
                &assets,
                SimulationFrameInput::no_hourglass().with_external_facts(
                    ExternalFacts::default().with_sound_boundary(SoundBoundary::replay(vec![
                        ranked_sound_resolution(17),
                    ])),
                ),
            )
            .expect_err("ranked runs must reject Original-trace sound authority");
        assert!(matches!(
            error,
            FrameAdvanceError::SoundBoundaryRejected {
                policy: SoundBoundaryPolicy::Replay,
                ..
            }
        ));
        assert_eq!(crate::replay::state_hash(&engine), before);
    }

    #[test]
    fn rejected_live_sound_boundary_is_atomic() {
        use crate::sound::{ExclamationGroup, PendingExclamation, ResolvedExclamation};

        let (mut engine, assets) = frame_api_fixture();
        engine
            .inner
            .feedback
            .sound_sim
            .pending_exclamations
            .push(PendingExclamation {
                actor_id: 191,
                group: ExclamationGroup::Civilian,
                profile_id: 0x4651_0000,
                exclamation_id: 62,
                variant: -1,
            });
        let invalid_resolution = ResolvedExclamation {
            actor_id: 192,
            identifier: 0x4651_003f,
            exclamation_id: 63,
            duration_frames: 24,
        };
        let engine_hash_before = crate::replay::state_hash(&engine);
        let error = engine
            .advance_frame(
                &assets,
                SimulationFrameInput::new(vec![SimCommand::from(
                    PlayerCommand::SetMenToBlazonConversionMode { on: true },
                )])
                .with_external_facts(
                    ExternalFacts::default()
                        .with_sound_boundary(SoundBoundary::live(vec![invalid_resolution])),
                ),
            )
            .expect_err("a live sound resolution must match the pending FIFO");

        assert!(matches!(
            error,
            FrameAdvanceError::SoundBoundaryRejected {
                policy: SoundBoundaryPolicy::Live,
                ..
            }
        ));
        assert_eq!(crate::replay::state_hash(&engine), engine_hash_before);
        assert_eq!(engine.frame_counter(), 0);
        assert!(!engine.is_men_to_blazon_conversion_mode());
    }

    #[test]
    fn rejected_external_fact_prevents_command_and_hourglass() {
        use crate::element::Command;
        use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

        let (mut engine, assets) = frame_api_fixture();
        engine.set_external_director_completion_replay(true);

        // Launch a real camera command. The first completion therefore mutates
        // the Engine before the second, invalid completion is rejected.
        let mut camera = SequenceElement::new_generic(1, Command::CameraGoto, None);
        camera.set_property(
            Field::CameraPoint,
            FieldValue::GeoPoint2D { x: 100.0, y: 100.0 },
        );
        camera.set_property(Field::CameraSpeed, FieldValue::Integer(0));
        let mut sequence = Sequence::new();
        sequence.append_element(camera);
        let sequence_id = engine
            .inner
            .orders
            .sequence_manager
            .launch_sequence(sequence);
        engine
            .inner
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
        engine.inner.feedback.cutscene_camera.sequence_element =
            Some(crate::sequence::SequenceElementRef::new(sequence_id, 0));
        assert!(
            engine
                .inner
                .feedback
                .cutscene_camera
                .sequence_element
                .is_some(),
            "fixture must have an active CameraGoto"
        );

        let engine_hash_before = crate::replay::state_hash(&engine);
        let mut accepted_engine = engine.clone();
        accepted_engine
            .advance_frame(
                &assets,
                SimulationFrameInput::no_hourglass().with_external_facts(
                    ExternalFacts::default()
                        .with_director_completions(vec![DirectorCompletion::CameraGoto]),
                ),
            )
            .expect("the first director fact must be independently valid");
        assert_ne!(
            crate::replay::state_hash(&accepted_engine),
            engine_hash_before,
            "the accepted prefix must mutate the staged engine"
        );
        let error = engine
            .advance_frame(
                &assets,
                SimulationFrameInput::new(vec![
                    SimCommand::from(PlayerCommand::SetMenToBlazonConversionMode { on: true }),
                    SimCommand::from(PlayerCommand::MouseRightUp),
                ])
                .with_external_facts(
                    ExternalFacts::default().with_director_completions(vec![
                        DirectorCompletion::CameraGoto,
                        DirectorCompletion::CameraGoto,
                    ]),
                ),
            )
            .expect_err("the second completion has no active camera command");

        assert!(matches!(
            error,
            FrameAdvanceError::DirectorCompletionRejected {
                index: 1,
                completion: DirectorCompletion::CameraGoto,
                ..
            }
        ));
        assert_eq!(crate::replay::state_hash(&engine), engine_hash_before);
        assert_eq!(engine.frame_counter(), 0);
        assert!(!engine.is_men_to_blazon_conversion_mode());
    }

    #[test]
    fn no_hourglass_director_prefix_exposes_new_delayed_drop_ale_seek() {
        use crate::element::Command;
        use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

        let (mut engine, assets) = frame_api_fixture();
        engine.set_external_director_completion_replay(true);
        let owner = EntityId::Pc(crate::entity_id::PcId(36));
        let destination = crate::coordinates::MapPoint::new(778.0, 1714.0);
        let fallback_sector =
            crate::position_interface::SectorHandle::new(25).expect("valid fallback sector");

        let mut camera = SequenceElement::new_generic(1, Command::CameraGoto, None);
        camera.set_property(
            Field::CameraPoint,
            FieldValue::GeoPoint2D { x: 100.0, y: 100.0 },
        );
        camera.set_property(Field::CameraSpeed, FieldValue::Integer(0));
        let mut seek = pending_drop_ale_seek(owner, destination, fallback_sector);
        seek.command_level = 2;
        let mut sequence = Sequence::new();
        sequence.append_element(camera);
        sequence.append_element(seek);
        let sequence_id = engine
            .inner
            .orders
            .sequence_manager
            .launch_sequence(sequence);
        engine
            .inner
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
        engine.inner.feedback.cutscene_camera.sequence_element =
            Some(crate::sequence::SequenceElementRef::new(sequence_id, 0));

        assert!(
            !engine.has_pending_recorded_drop_ale_route(owner, destination),
            "the later command level must not be claimable before the prefix releases it"
        );
        engine
            .advance_frame(
                &assets,
                SimulationFrameInput::no_hourglass().with_external_facts(
                    ExternalFacts::default()
                        .with_director_completions(vec![DirectorCompletion::CameraGoto]),
                ),
            )
            .expect("stage director prefix");
        assert!(
            engine.has_pending_recorded_drop_ale_route(owner, destination),
            "the delayed route collector must inspect state after the director/sound prefix"
        );
    }

    #[test]
    fn external_facts_are_part_of_the_authoritative_frame_journal() {
        use crate::sound::{ExclamationGroup, PendingExclamation, ResolvedExclamation};

        let (mut initial, assets) = frame_api_fixture();
        let profile_id = 0x4651_0000;
        initial
            .inner
            .feedback
            .sound_sim
            .pending_exclamations
            .push(PendingExclamation {
                actor_id: 191,
                group: ExclamationGroup::Civilian,
                profile_id,
                exclamation_id: 62,
                variant: -1,
            });
        let mut complete_journal = initial.clone();
        let mut command_only_journal = initial;
        let command = SimCommand::from(PlayerCommand::Noop);
        let resolution = ResolvedExclamation {
            actor_id: 191,
            identifier: profile_id | 62,
            exclamation_id: 62,
            duration_frames: 24,
        };

        let complete_output = complete_journal
            .advance_frame(
                &assets,
                SimulationFrameInput::new(vec![command.clone()]).with_external_facts(
                    ExternalFacts::default()
                        .with_sound_boundary(SoundBoundary::live(vec![resolution])),
                ),
            )
            .expect("advance complete frame journal");

        let command_only_output = command_only_journal
            .advance_frame(&assets, SimulationFrameInput::new(vec![command]))
            .expect("advance command-only frame journal");

        assert_ne!(
            complete_output.state_hash, command_only_output.state_hash,
            "replaying commands without the recorded host sound fact must not be treated as equivalent"
        );
        assert_eq!(complete_journal.sound_sim().playing_exclamations.len(), 1);
        assert!(complete_journal.sound_sim().pending_exclamations.is_empty());
        assert!(
            command_only_journal
                .sound_sim()
                .playing_exclamations
                .is_empty()
        );
        assert_eq!(
            command_only_journal.sound_sim().pending_exclamations.len(),
            1
        );
    }

    #[test]
    fn closed_body_gate_is_not_a_paused_presentation_boundary() {
        let (mut engine, assets) = frame_api_fixture();
        let output = engine
            .advance_frame(
                &assets,
                SimulationFrameInput::default().with_simulation_body_allowed(false),
            )
            .expect("advance with only the actor/world body gated");

        assert_eq!(output.frame_before, 0);
        assert_eq!(output.frame_after, 1);
        assert_eq!(engine.frame_counter(), 1);
    }

    #[test]
    fn parity_engine_state_preserves_next_original_creation_order() {
        let mut inner = EngineInner::new();
        inner.world.next_original_creation_order = 417;
        inner.control.chorus_timer = 23;
        inner.script_domains.mission_ui.force_check = true;
        inner
            .script_domains
            .mission_ui
            .men_to_blazon_conversion_mode = true;
        let state = Engine {
            inner,
            bootstrap_open: false,
        }
        .parity_engine_state();

        assert_eq!(state.next_creation_order, 417);
        assert_eq!(state.chorus_timer, 23);
        assert!(state.force_check);
        assert!(state.men_to_blazon_conversion);
    }

    fn parity_position_sprite(state: &serde_json::Value) -> (u32, u32) {
        (
            state["position"]["sprite"]["x"]["bits"]
                .as_u64()
                .expect("sprite x bits") as u32,
            state["position"]["sprite"]["y"]["bits"]
                .as_u64()
                .expect("sprite y bits") as u32,
        )
    }

    #[test]
    fn parity_runtime_projects_current_sprite_top_left_for_ordinary_entities() {
        let mut inner = EngineInner::new();
        let mut element = {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::Fx;
            initial_element
        };
        element.sprite.center = crate::coordinates::SpriteAnchor::new(150.0, 150.0);
        element
            .sprite
            .position_iface
            .set_cached_sprite_position(crate::coordinates::MapPoint::new(1688.0, 150.0));
        element.set_position_map(crate::coordinates::MapPoint::new(1836.2246, 301.3214));
        let id = inner.add_entity(crate::element::Entity::Fx(crate::element::ElementFx {
            element,
            fx: Default::default(),
        }));

        let state = Engine {
            inner,
            bootstrap_open: false,
        }
        .parity_entity_runtime_state(id, &LevelAssets::new());

        assert_eq!(
            parity_position_sprite(&state),
            (1686.0_f32.to_bits(), 151.0_f32.to_bits())
        );
    }

    #[test]
    fn parity_runtime_preserves_target_cached_sprite_anchor() {
        let mut inner = EngineInner::new();
        let mut element = {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::Target;
            initial_element
        };
        element.sprite.center = crate::coordinates::SpriteAnchor::new(30.0, 140.0);
        element
            .sprite
            .position_iface
            .set_cached_sprite_position(crate::coordinates::MapPoint::new(2791.0, 171.0));
        element.set_position_map_preserving_3d(crate::coordinates::MapPoint::new(2823.0, 312.0));
        let id = inner.add_entity(crate::element::Entity::Target(
            crate::element::ElementTarget {
                element,
                fx: Default::default(),
                target: Default::default(),
            },
        ));

        let state = Engine {
            inner,
            bootstrap_open: false,
        }
        .parity_entity_runtime_state(id, &LevelAssets::new());

        assert_eq!(
            parity_position_sprite(&state),
            (2791.0_f32.to_bits(), 171.0_f32.to_bits())
        );
    }

    #[test]
    fn parity_runtime_refreshes_bank_dimensions_only_after_recorded_boundary() {
        struct Frames;
        impl crate::engine::PixelOpacityLookup for Frames {
            fn sprite_dimensions(&self, bank_id: u32) -> Option<(u16, u16)> {
                (bank_id == 73).then_some((48, 19))
            }

            fn is_pixel_opaque(
                &self,
                _bank_id: u32,
                _x: u16,
                _y: u16,
                _blue_pixels_are_in: bool,
            ) -> bool {
                false
            }
        }

        let mut inner = EngineInner::new();
        let mut sprite = crate::sprite::Sprite {
            current_width: 44,
            current_height: 20,
            current_row: 0,
            current_frame: 1,
            masked: true,
            ..Default::default()
        };
        // The stale 44-pixel cache ends before the viewport, while the
        // current 48-pixel bank surface intersects it. The original game's visibility test
        // uses the latter before target-sprite creation publishes the cache.
        sprite
            .position_iface
            .set_map_position(crate::coordinates::MapPoint::new(-47.0, 0.0));
        sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            frame_ids: vec![72, 73],
            offsets: vec![
                crate::coordinates::SpriteFrameOffset::ZERO,
                crate::coordinates::SpriteFrameOffset::ZERO,
            ],
            ..Default::default()
        }]);
        let id = inner.add_entity(crate::element::Entity::Fx(crate::element::ElementFx {
            element: {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.kind = crate::element::ElementKind::Fx;
                initial_element.active = true;
                initial_element.sprite = sprite;
                initial_element
            },
            fx: Default::default(),
        }));
        let assets = LevelAssets {
            attachments: crate::engine::LevelRuntimeAttachments {
                pixel_opacity: Some(std::sync::Arc::new(Frames)),
                ..Default::default()
            },
            ..LevelAssets::new()
        };

        let mut engine = Engine {
            inner,
            bootstrap_open: false,
        };
        let before_refresh = engine.parity_entity_runtime_state(id, &assets);
        engine
            .parity_replay_setup()
            .refresh_sprite_dimension_cache(&assets, false);
        let after_refresh = engine.parity_entity_runtime_state(id, &assets);

        assert_eq!(before_refresh["sprite"]["width"], 44);
        assert_eq!(before_refresh["sprite"]["height"], 20);
        assert_eq!(before_refresh["sprite"]["masked"], true);
        assert_eq!(after_refresh["sprite"]["width"], 48);
        assert_eq!(after_refresh["sprite"]["height"], 19);
        assert_eq!(after_refresh["sprite"]["masked"], false);
    }

    #[test]
    fn missing_draw_view_skips_presentation_cache_refresh() {
        struct Frames;
        impl crate::engine::PixelOpacityLookup for Frames {
            fn sprite_dimensions(&self, bank_id: u32) -> Option<(u16, u16)> {
                (bank_id == 73).then_some((24, 55))
            }

            fn is_pixel_opaque(
                &self,
                _bank_id: u32,
                _x: u16,
                _y: u16,
                _blue_pixels_are_in: bool,
            ) -> bool {
                false
            }
        }

        let mut inner = EngineInner::new();
        let mut sprite = crate::sprite::Sprite {
            current_width: 20,
            current_height: 53,
            current_row: 0,
            current_frame: 0,
            ..Default::default()
        };
        // The reconstructed view ends at x=1024. Original passes that exact
        // box to sprite visibility; DrawManager's separate +/-1 draw range
        // must not leak into the element refresh visibility test.
        sprite.center = crate::coordinates::SpriteAnchor::new(-1025.0, -20.0);
        sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            frame_ids: vec![73],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            ..Default::default()
        }]);
        let id = inner.add_entity(crate::element::Entity::Fx(crate::element::ElementFx {
            element: {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.kind = crate::element::ElementKind::Fx;
                initial_element.active = true;
                initial_element.sprite = sprite;
                initial_element
            },
            fx: Default::default(),
        }));
        let assets = LevelAssets {
            attachments: crate::engine::LevelRuntimeAttachments {
                pixel_opacity: Some(std::sync::Arc::new(Frames)),
                ..Default::default()
            },
            ..LevelAssets::new()
        };
        let mut engine = Engine {
            inner,
            bootstrap_open: false,
        };
        let gameplay_position = engine.inner.world.entities[id]
            .as_ref()
            .expect("test entity must remain occupied")
            .element_data()
            .position_map();

        engine
            .parity_replay_setup()
            .refresh_sprite_dimension_cache(&assets, false);
        let without_legacy_edge = engine.parity_entity_runtime_state(id, &assets);
        assert_eq!(without_legacy_edge["sprite"]["width"], 20);
        assert_eq!(without_legacy_edge["sprite"]["height"], 53);

        engine
            .parity_replay_setup()
            .refresh_sprite_dimension_cache(&assets, true);
        let with_legacy_edge = engine.parity_entity_runtime_state(id, &assets);
        assert_eq!(with_legacy_edge["sprite"]["width"], 20);
        assert_eq!(with_legacy_edge["sprite"]["height"], 53);
        assert_eq!(
            engine.inner.world.entities[id]
                .as_ref()
                .expect("test entity must remain occupied")
                .element_data()
                .position_map(),
            gameplay_position,
            "presentation compatibility must not mutate gameplay position"
        );

        let sprite = engine.inner.world.entities[id]
            .as_mut()
            .expect("test entity must remain occupied")
            .sprite_mut();
        sprite.current_width = 20;
        sprite.current_height = 53;
        sprite.center = crate::coordinates::SpriteAnchor::new(-1024.0, -20.0);
        engine
            .parity_replay_setup()
            .refresh_sprite_dimension_cache(&assets, true);
        let touching_view_edge = engine.parity_entity_runtime_state(id, &assets);
        assert_eq!(touching_view_edge["sprite"]["width"], 20);
        assert_eq!(touching_view_edge["sprite"]["height"], 53);

        engine
            .parity_replay_setup()
            .refresh_sprite_dimension_cache(&assets, false);
        let with_exact_view = engine.parity_entity_runtime_state(id, &assets);
        assert_eq!(with_exact_view["sprite"]["width"], 24);
        assert_eq!(with_exact_view["sprite"]["height"], 55);

        // Original also leaves an off-screen sprite's cached dimensions
        // untouched when its current frame ends one unit above the viewport
        // (interactive session 003, session 0003, frame 780).
        let sprite = engine.inner.world.entities[id]
            .as_mut()
            .expect("test entity must remain occupied")
            .sprite_mut();
        sprite.current_width = 20;
        sprite.current_height = 53;
        sprite.center = crate::coordinates::SpriteAnchor::new(0.0, 56.0);
        engine
            .parity_replay_setup()
            .refresh_sprite_dimension_cache(&assets, true);
        let above_legacy_near_edge = engine.parity_entity_runtime_state(id, &assets);
        assert_eq!(above_legacy_near_edge["sprite"]["width"], 20);
        assert_eq!(above_legacy_near_edge["sprite"]["height"], 53);
    }

    #[test]
    fn parity_runtime_projection_ordinal_includes_original_default_ground_slot() {
        use crate::sight_obstacle::{SIGHTOBSTACLE_PROJECTION_AREA, SightObstacle};

        let mut assets = LevelAssets::new();
        assets.environment.static_sight_obstacles = std::sync::Arc::new(vec![
            SightObstacle::new_default(10),
            SightObstacle::new(11, SIGHTOBSTACLE_PROJECTION_AREA),
            SightObstacle::new_default(12),
            SightObstacle::new(13, SIGHTOBSTACLE_PROJECTION_AREA),
        ]);

        for (handle, expected_ordinal) in [(1_u32, 1_u64), (3, 2)] {
            let mut inner = EngineInner::new();
            let mut element = {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.kind = crate::element::ElementKind::Fx;
                initial_element
            };
            element.set_layer(0);
            element.set_obstacle_index(
                crate::position_interface::ObstacleHandle::new(handle),
                Some(crate::position_interface::PlaneZCoeffs {
                    az: 0.0,
                    bz: 0.0,
                    dz: 0.0,
                }),
            );
            let id = inner.add_entity(crate::element::Entity::Fx(crate::element::ElementFx {
                element,
                fx: Default::default(),
            }));

            let state = Engine {
                inner,
                bootstrap_open: false,
            }
            .parity_entity_runtime_state(id, &assets);
            assert_eq!(
                state["position"]["obstacle"],
                serde_json::json!({ "kind": "projection", "index": expected_ordinal })
            );
        }
    }

    #[test]
    fn parity_game_ui_state_preserves_serialized_latches() {
        let mut inner = EngineInner::new();
        let ui = &mut inner.script_domains.mission_ui;
        ui.campaign_map = true;
        ui.campaign_map_displayed = true;
        ui.game_post_initialized = true;
        ui.start_mission_disabled_temp = true;
        ui.quit_mission_disabled_temp = false;
        ui.start_mission_enabled = true;
        ui.quit_mission_enabled = false;

        assert_eq!(
            Engine {
                inner,
                bootstrap_open: false
            }
            .parity_game_ui_state(),
            serde_json::json!({
                "campaign_map": true,
                "campaign_map_displayed": true,
                "post_initialized": true,
                "start_mission_disabled_temp": true,
                "quit_mission_disabled_temp": false,
                "start_mission_enabled": true,
                "quit_mission_enabled": false,
            })
        );
    }

    #[test]
    fn parity_messenger_controller_is_independent_of_camera_locker() {
        let mut inner = EngineInner::new();
        inner.players.view_locked = true;
        inner.players.seats[0].locker_active = false;
        inner.players.seats[0].selected_action = crate::profiles::Action::Bow;

        let engine = Engine {
            inner,
            bootstrap_open: false,
        };
        assert_eq!(
            engine.parity_messenger_controller_state(),
            serde_json::json!({ "view_locked": true, "selected_action": 1 })
        );
        assert!(!engine.locker_active());
        assert!(engine.view_locked());
    }

    #[test]
    fn parity_shield_controller_preserves_global_protocol_state() {
        let mut inner = EngineInner::new();
        inner.world.shield.is_protected = false;
        inner.world.shield.protected_pc = Some(EntityId::new(7, crate::element::EntityIdKind::Pc));
        inner.world.shield.danger_point = crate::coordinates::WorldPoint3D {
            x: 1.25,
            y: -2.5,
            z: 3.75,
        };

        assert_eq!(
            Engine {
                inner,
                bootstrap_open: false
            }
            .parity_shield_controller_state(),
            serde_json::json!({
                "is_protected": false,
                "protected_pc": { "kind": "pc", "index": 7 },
                "danger_point": {
                    "x": { "bits": 1.25_f32.to_bits() },
                    "y": { "bits": (-2.5_f32).to_bits() },
                    "z": { "bits": 3.75_f32.to_bits() },
                },
            })
        );
    }

    #[test]
    fn parity_sound_sources_preserves_sparse_slots_and_authoritative_fields() {
        let mut inner = EngineInner::new();
        inner.feedback.sound_sim.sources.sources_push_none();
        let mut source = crate::sound_source::SoundSource::new();
        source.source_kind = crate::sound_source::SoundSourceKind::Delayed;
        source.id = 73;
        source.inner_distance = 12;
        source.outer_distance = 34;
        source.noise_covering_distance = 56;
        source.inner_volume = 78;
        source.outer_volume = 9;
        source
            .shape
            .push(crate::coordinates::MapPoint::new(1.5, -2.0));
        source.altitude = crate::sound_geometry::SoundSourceAltitude::Top;
        source.min_delay = 4;
        source.max_delay = 18;
        source.delay_stepping = 5;
        source.timer = 11;
        source.active = true;
        inner.feedback.sound_sim.sources.sources_push_some(source);
        let engine = Engine {
            inner,
            bootstrap_open: false,
        };

        let state = engine.parity_sound_sources_state();
        assert!(state[0].is_null());
        assert_eq!(state[1]["kind"], 2);
        assert_eq!(state[1]["id"], 73);
        assert_eq!(state[1]["noise_covering_distance"], 56);
        assert_eq!(state[1]["shape"][0]["x"]["bits"], 1.5f32.to_bits());
        assert_eq!(state[1]["altitude"], 2);
        assert_eq!(state[1]["timer"], 11);
        assert_eq!(state[1]["active"], true);
        assert_eq!(state[1]["ambience_enabled"], true);
    }

    #[test]
    fn parity_sound_completion_frontier_preserves_pending_order() {
        let mut inner = EngineInner::new();
        inner
            .feedback
            .sound_sim
            .sources
            .sources_push_some(crate::sound_source::SoundSource::new());
        inner
            .feedback
            .sound_sim
            .sources
            .sources_push_some(crate::sound_source::SoundSource::new());
        inner
            .feedback
            .sound_sim
            .playing_sources
            .push(crate::sound::PlayingSource {
                source_index: 1,
                finish_frame: 73,
            });
        inner
            .feedback
            .sound_sim
            .playing_sources
            .push(crate::sound::PlayingSource {
                source_index: 0,
                finish_frame: 91,
            });

        let state = Engine {
            inner,
            bootstrap_open: false,
        }
        .parity_sound_completion_frontier_state();
        assert_eq!(state[0]["source_index"], 1);
        assert_eq!(state[0]["finish_frame"], 73);
        assert_eq!(state[1]["source_index"], 0);
        assert_eq!(state[1]["finish_frame"], 91);
    }

    #[test]
    fn parity_ai_global_preserves_ordered_statuses_reservations_and_alerts() {
        let mut inner = EngineInner::new();
        inner.ai.global.stupid_soldiers_cheat = true;
        inner.ai.global.green_alert_soldiers = 3;
        inner.ai.global.yellow_alert_soldiers = 4;
        inner.ai.global.red_alert_soldiers = 5;
        inner.ai.global.overall_alert_status = crate::ai::AlertLevel::Yellow;
        inner.ai.global.overall_villain_alert_status = crate::ai::AlertLevel::Red;
        inner.ai.global.saved_random_seed = -73;
        inner.ai.global.current_speech_variant = 2;
        inner
            .ai
            .global
            .forbidden_remarks
            .push(crate::ai::ForbiddenRemark {
                remark: crate::ai::Remark::Warcry,
                flags: crate::ai::RemarkTargetFlags::THIS_GUY.bits(),
                speech_id: 91,
                guy_index: 47,
                bad_guy: true,
                forbidden_till_frame: 1234,
            });
        let mut seek = crate::ai::SeekPoint::from_position(
            &crate::sim_rng::SimulationContext::with_seed(1),
            crate::ai::Position::default(),
        );
        seek.frame_when_full_interest = 99;
        seek.last_calculated_interest = 41;
        seek.locked = true;
        inner.ai.global.seek_points.push(seek);
        inner
            .ai
            .global
            .archery_sectors
            .push(crate::ai::SectorArchery {
                points: vec![crate::ai::PointArchery {
                    position: crate::ai::Position::default(),
                    direction: 7,
                    is_shooting_point: true,
                    sector_index: crate::sector::SectorNumber(2),
                    owner: None,
                }],
                polygon: Vec::new(),
                layer: 0,
                index_first_shooting_point: Some(crate::sector::ArcheryPointIdx(0)),
                index_last_shooting_point: Some(crate::sector::ArcheryPointIdx(0)),
                num_shooting_points: 1,
                num_owners: 0,
            });
        let engine = Engine {
            inner,
            bootstrap_open: false,
        };

        let state = engine.parity_ai_global_state();
        assert_eq!(state["stupid_soldiers_cheat"], true);
        assert_eq!(state["seek_points"][0]["frame_when_full_interest"], 99);
        assert_eq!(state["seek_points"][0]["last_calculated_interest"], 41);
        assert_eq!(state["seek_points"][0]["locked"], true);
        assert_eq!(state["archery_sectors"][0]["num_owners"], 0);
        assert!(state["archery_sectors"][0]["point_owners"][0].is_null());
        assert_eq!(state["overall_alert_status"], 1);
        assert_eq!(state["overall_villain_alert_status"], 2);
        assert_eq!(state["saved_random_seed"], -73);
        assert_eq!(state["forbidden_remarks"][0]["remark"], 9);
        assert_eq!(state["forbidden_remarks"][0]["flags"], 8);
        assert_eq!(state["forbidden_remarks"][0]["speech_id"], 91);
        assert_eq!(state["forbidden_remarks"][0]["guy_index"], 47);
        assert_eq!(state["forbidden_remarks"][0]["bad_guy"], true);
        assert_eq!(state["forbidden_remarks"][0]["forbidden_till_frame"], 1234);
        assert_eq!(state["current_speech_variant"], 2);
    }

    #[test]
    fn parity_pc_registry_preserves_original_order_not_portrait_order() {
        let mut inner = EngineInner::new();
        let new_pc = || {
            crate::element::Entity::Pc(crate::element::ActorPc {
                element: crate::element::ElementData::default(),
                actor: crate::element::ActorData::default(),
                human: crate::element::HumanData::default(),
                pc: crate::element::PcData::default(),
            })
        };
        let first = inner.add_entity(new_pc());
        let second = inner.add_entity(new_pc());
        inner.world.pc_ids = vec![first, second];
        inner.world.original_pc_registry_ids = vec![second, first];

        let state = Engine {
            inner,
            bootstrap_open: false,
        }
        .parity_pc_registry_state();
        assert_eq!(state[0]["kind"], "pc");
        assert_eq!(state[0]["index"], second.index());
        assert_eq!(state[1]["index"], first.index());
    }

    #[test]
    fn parity_runtime_roots_preserves_mission_stat_and_empty_reference_roots() {
        struct MenuText;
        impl crate::sherwood_stat::MenuTextLookup for MenuText {
            fn get(&self, id: usize) -> String {
                format!("menu-{id}")
            }
        }

        let mut inner = EngineInner::new();
        inner.players.user_locked = true;
        inner.mission_domain.mission_stat.collected_money = 73;
        inner.mission_domain.mission_stat.added_score = 91;
        inner
            .mission_domain
            .mission_stat
            .pc_names
            .push(crate::mission_stat::PcStatName::new(
                "fallback".into(),
                Some(crate::pc_status::SpecialPeasantName::B),
            ));
        let engine = Engine {
            inner,
            bootstrap_open: false,
        };

        let state = engine.parity_engine_runtime_roots_state(&MenuText);
        assert_eq!(state["timer_elements"].as_array().unwrap().len(), 0);
        assert!(state["camera_sequence"].is_null());
        assert!(state["dead_pc"].is_null());
        assert_eq!(state["mission_stat"]["collected_money"], 73);
        assert_eq!(state["mission_stat"]["added_score"], 91);
        assert_eq!(state["mission_stat"]["pc_names"][0], "menu-251");
        assert_eq!(state["user_locked"], true);
        assert_eq!(
            state["selection_before_user_lock"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert!(state["follow_element"].is_null());
    }

    #[test]
    fn parity_world_interactables_preserves_dynamic_patch_and_door_fields() {
        let mut inner = EngineInner::new();
        let patch = crate::patch::Patch {
            active: true,
            locked: true,
            applied: true,
            in_transition: true,
            ..Default::default()
        };
        inner.script_domains.interactables.patches.push(patch);
        let door = crate::gate::Door {
            active: false,
            locked_pc: true,
            locked_npc_villain: true,
            unlockable: true,
            locked_pc_after_patch: true,
            locked_npc_civilian_after_patch: true,
            unlockable_after_patch: true,
            special_authorisation_pc: true,
            authorised_pc_direct: 0x12,
            authorised_pc_indirect: 0x34,
            ..Default::default()
        };
        inner.script_domains.interactables.doors.push(door);
        let engine = Engine {
            inner,
            bootstrap_open: false,
        };

        let state = engine.parity_world_interactables_state(&LevelAssets::new());
        assert_eq!(state["patches"][0]["active"], true);
        assert_eq!(state["patches"][0]["locked"], true);
        assert_eq!(state["patches"][0]["applied"], true);
        assert_eq!(state["patches"][0]["in_transition"], true);
        assert_eq!(
            state["patches"][0]["occupants"].as_array().unwrap().len(),
            0
        );
        assert_eq!(state["doors"][0]["kind"], "door");
        assert_eq!(state["doors"][0]["active"], false);
        assert_eq!(state["doors"][0]["locked_pc"], true);
        assert_eq!(state["doors"][0]["locked_npc_villain"], true);
        assert_eq!(state["doors"][0]["unlockable"], true);
        assert_eq!(state["doors"][0]["locked_pc_after_patch"], true);
        assert_eq!(state["doors"][0]["locked_npc_civilian_after_patch"], true);
        assert_eq!(state["doors"][0]["unlockable_after_patch"], true);
        assert_eq!(state["doors"][0]["special_authorisation_pc"], true);
        assert_eq!(state["doors"][0]["authorised_pc_direct"], 0x12);
        assert_eq!(state["doors"][0]["authorised_pc_indirect"], 0x34);
        assert_eq!(state["sector_doors"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn parity_world_interactables_preserves_lift_runtime_state() {
        let mut inner = EngineInner::new();
        let sector_number = crate::sector::SectorNumber::new(47);
        let level = std::sync::Arc::make_mut(&mut inner.world.fast_grid_mut().level);
        level.sector_number_map.insert(sector_number, 0);
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::LIFT,
            layer: 0,
            sector_number,
            door_index: None,
            lift_type: Some(crate::sector::LiftType::Ladder),
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        });
        inner.world.fast_grid_mut().lift_state.insert(
            0,
            crate::fast_find_grid::LiftRuntimeState {
                occupants_pc: 2,
                occupants: 3,
                occupied_upwards: true,
                occupied_downwards: false,
                wait_time: 71,
            },
        );

        let state = Engine {
            inner,
            bootstrap_open: false,
        }
        .parity_world_interactables_state(&LevelAssets::new());
        assert_eq!(state["lifts"][0]["sector"], 47);
        assert_eq!(state["lifts"][0]["occupants_pc"], 2);
        assert_eq!(state["lifts"][0]["occupants"], 3);
        assert_eq!(state["lifts"][0]["occupied_upwards"], true);
        assert_eq!(state["lifts"][0]["occupied_downwards"], false);
        assert_eq!(state["lifts"][0]["wait_time"], 71);
    }

    #[test]
    fn parity_world_interactables_preserves_ordered_building_and_zone_state() {
        let mut inner = EngineInner::new();
        let new_pc = || {
            crate::element::Entity::Pc(crate::element::ActorPc {
                element: crate::element::ElementData::default(),
                actor: crate::element::ActorData::default(),
                human: crate::element::HumanData::default(),
                pc: crate::element::PcData::default(),
            })
        };
        let first = inner.add_entity(new_pc());
        let second = inner.add_entity(new_pc());
        inner.script_domains.buildings.occupants.push(vec![
            crate::natives::ScriptHandleCodec::actor_handle(second),
            crate::natives::ScriptHandleCodec::actor_handle(first),
        ]);
        inner.script_domains.buildings.arrow_reserves.push(true);

        inner
            .script_domains
            .zones
            .scripts
            .push(crate::sector::ScriptSectorData {
                sector_index: crate::fast_find_grid::SectorIndex::new(0),
                transformed_to_apex: true,
                max_throwing_apex_height: 12.5,
                occupant_indices: vec![first, second],
                ..Default::default()
            });
        let level = std::sync::Arc::make_mut(&mut inner.world.fast_grid_mut().level);
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::SCRIPT,
            layer: 0,
            sector_number: crate::sector::SectorNumber::new(47),
            door_index: None,
            lift_type: None,
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        });
        inner
            .world
            .fast_grid_mut()
            .or_sector_type_overlay(0, crate::sector::SectorType::APEX);
        let mut assets = LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.scripts.zone_grid_indices).push(0);

        let state = Engine {
            inner,
            bootstrap_open: false,
        }
        .parity_world_interactables_state(&assets);
        assert_eq!(
            state["buildings"][0]["occupants"][0]["index"],
            second.index()
        );
        assert_eq!(
            state["buildings"][0]["occupants"][1]["index"],
            first.index()
        );
        assert_eq!(state["buildings"][0]["arrow_reserve"], true);
        assert_eq!(
            state["script_zones"][0]["occupants"][0]["index"],
            first.index()
        );
        assert_eq!(
            state["script_zones"][0]["occupants"][1]["index"],
            second.index()
        );
        assert_eq!(state["script_zones"][0]["transformed_to_apex"], true);
        assert_eq!(
            state["script_zones"][0]["max_apex_height"]["bits"],
            12.5f32.to_bits()
        );
    }

    #[test]
    fn parity_repulsive_points_preserves_serialized_fields_order_and_next_id() {
        let mut inner = EngineInner::new();
        inner.world.original_repulsive_point_counter = 42;
        let mut first = crate::ai::RepulsivePoint::new(
            17,
            crate::ai::Position {
                x: 1.25,
                y: -2.5,
                sector: None,
                level: 3,
            },
            4.0,
            5.0,
            1 | 4 | 8,
        );
        first.concave = true;
        first.limit_left = crate::coordinates::MapVec::new(6.0, 7.0);
        first.limit_right = crate::coordinates::MapVec::new(8.0, 9.0);
        inner.ai.global.repulsive_points.push(first);
        inner
            .ai
            .global
            .repulsive_points
            .push(crate::ai::RepulsivePoint::new(
                18,
                crate::ai::Position {
                    level: 5,
                    ..Default::default()
                },
                10.0,
                11.0,
                2,
            ));
        let engine = Engine {
            inner,
            bootstrap_open: false,
        };

        let state = engine.parity_repulsive_points_state();
        assert_eq!(state["next_id"], 42);
        assert_eq!(state["points"][0]["id"], 17);
        assert_eq!(state["points"][1]["id"], 18);
        assert_eq!(
            state["points"][0]["position"]["x"]["bits"],
            1.25f32.to_bits()
        );
        assert_eq!(
            state["points"][0]["position"]["y"]["bits"],
            (-2.5f32).to_bits()
        );
        assert_eq!(state["points"][0]["concave"], true);
        assert_eq!(
            state["points"][0]["limit_left"]["x"]["bits"],
            6.0f32.to_bits()
        );
        assert_eq!(
            state["points"][0]["limit_right"]["y"]["bits"],
            9.0f32.to_bits()
        );
        assert_eq!(state["points"][0]["radius"]["bits"], 4.0f32.to_bits());
        assert_eq!(
            state["points"][0]["action_radius"]["bits"],
            9.0f32.to_bits()
        );
        assert_eq!(state["points"][0]["affects_pcs"], true);
        assert_eq!(state["points"][0]["affects_soldiers"], false);
        assert_eq!(state["points"][0]["affects_civilians"], true);
        assert_eq!(state["points"][0]["affects_animals"], true);
        assert_eq!(state["points"][0]["layer"], 3);
    }

    #[test]
    fn parity_titbits_preserves_serialized_manager_and_live_entry_fields() {
        let mut inner = EngineInner::new();
        let id = inner.feedback.titbit_manager.add_titbit(
            crate::coordinates::WorldPoint3D::new(1.5, -2.0, 3.25),
            4,
            crate::titbit::TitbitKind::DangerPoint,
            crate::titbit::ElementHandle::INVALID,
            7,
            crate::titbit::ElementHandle::INVALID,
            false,
            crate::titbit::INVALID_ID,
            true,
            None,
            None,
        );
        let titbit = &mut inner.feedback.titbit_manager.titbits_mut()[0];
        titbit.sprite_row = 8;
        titbit.sprite_frame = 9;
        titbit.frame_count = 10;
        titbit.display_order = 11.5;
        titbit.blinking = true;
        let engine = Engine {
            inner,
            bootstrap_open: false,
        };

        let state = engine.parity_titbit_manager_state();
        assert_eq!(state["current_id"], 1);
        assert_eq!(state["titbits"][0]["kind"], 10);
        assert_eq!(state["titbits"][0]["phase"], 7);
        assert_eq!(state["titbits"][0]["sprite_row"], 8);
        assert_eq!(state["titbits"][0]["sprite_frame"], 9);
        assert_eq!(state["titbits"][0]["frame_count"], 10);
        assert_eq!(
            state["titbits"][0]["display_order"]["bits"],
            11.5f32.to_bits()
        );
        assert_eq!(state["titbits"][0]["layer"], 4);
        assert_eq!(state["titbits"][0]["blinking"], true);
        assert_eq!(
            state["titbits"][0]["id"],
            id.expect("titbit allocation succeeds").get()
        );
        assert!(state["titbits"][0]["element_supplier"].is_null());
        assert!(state["titbits"][0]["element_manager"].is_null());
        assert_eq!(
            state["titbits"][0]["position"]["x"]["bits"],
            1.5f32.to_bits()
        );
    }

    #[test]
    fn diagnostic_snapshot_omits_only_nonserializable_original_rng_replay() {
        let mut inner = EngineInner::new();
        inner.control.rng = SimulationRng::with_original_replay(vec![11, 22]);
        let engine = Engine {
            inner,
            bootstrap_open: false,
        };

        assert!(serde_json::to_value(&engine).is_err());
        let diagnostic = engine.diagnostic_snapshot_without_original_rng_replay();

        assert_eq!(engine.original_rng_replay_cursor(), Some(0));
        assert_eq!(diagnostic.original_rng_replay_cursor(), None);
        serde_json::to_value(&diagnostic).expect("diagnostic engine must serialize");
    }

    #[test]
    fn legacy_additional_arrow_refreshes_advance_real_sprite_state() {
        let mut inner = EngineInner::new();
        inner.control.rng = SimulationRng::with_original_replay(vec![11, 22, 33, 44]);
        inner.control.arrow_refresh_pending = true;
        let projectile = crate::element::ElementProjectile {
            element: {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.kind = crate::element::ElementKind::ObjectProjectile;
                initial_element.active = true;
                initial_element
            },
            object: crate::element::ObjectData {
                object_type: crate::element::ObjectType::Arrow,
                ..Default::default()
            },
            projectile: crate::element::ProjectileData {
                falling: true,
                falling_direction: 8,
                trajectory: vec![crate::element::TrajectoryPoint {
                    position: crate::coordinates::WorldPoint3D::new(1.0, 0.0, 0.0),
                    time: 1,
                }],
                ..Default::default()
            },
        };
        let id = inner.add_entity(crate::element::Entity::Projectile(projectile));
        let mut engine = Engine {
            inner,
            bootstrap_open: false,
        };

        assert_eq!(
            engine
                .parity_replay_setup()
                .pending_falling_arrow_refresh_draw_count(),
            1
        );
        engine
            .parity_replay_setup()
            .replay_legacy_additional_arrow_refreshes(3);

        assert_eq!(engine.original_rng_replay_cursor(), Some(3));
        assert!(engine.inner.control.arrow_refresh_pending);
        let crate::element::Entity::Projectile(arrow) = engine.get_entity(id).unwrap() else {
            panic!("test arrow changed entity kind");
        };
        assert_eq!(arrow.projectile.falling_direction, 2);
        assert_eq!(arrow.element.sprite.current_row, 4);
        assert!((3..=5).contains(&arrow.element.sprite.current_frame));
    }

    #[test]
    fn campaign_selection_transfers_one_rng_sequence_to_mission_construction() {
        let mut profiles = crate::profiles::ProfileManager::default();
        profiles.missions.push(crate::profiles::MissionProfile {
            id: 0,
            location: crate::profiles::MissionLocation::Sherwood,
            life_time: 100,
            max_ransom: 200_000,
            max_gang_size: u16::MAX,
            ..Default::default()
        });
        for id in 1..=2 {
            profiles.missions.push(crate::profiles::MissionProfile {
                id,
                mission_type: crate::profiles::MissionType::Rescue,
                location: crate::profiles::MissionLocation::York,
                life_time: 100,
                access_probability: 50,
                max_ransom: 200_000,
                max_gang_size: u16::MAX,
                ..Default::default()
            });
        }

        let mut campaign = Campaign::default();
        for profile_idx in 0..3 {
            campaign.missions.push(crate::mission::Mission {
                profile_idx: Some(profile_idx),
                ..Default::default()
            });
        }
        campaign.accessible_mission_indices = vec![1, 2];

        let seed = 0xCA11_AB1E;
        let config = SimConfig::default();
        let reference_context =
            crate::sim_rng::SimulationContext::with_seed_and_config(seed, config);
        let mut reference_campaign = campaign.clone();
        let expected_mission =
            reference_campaign.determine_next_mission(&reference_context, &profiles);
        let expected_next_seed = reference_context.seed();
        let expected_next_draw = crate::sim_rng::u32(
            &reference_context,
            crate::sim_rng::RngSite::TitbitUpdate,
            ..,
        );

        let (_campaign, mission, next_seed, next_config) =
            Engine::select_next_mission(campaign, &profiles, seed, config);
        let mission_context =
            crate::sim_rng::SimulationContext::with_seed_and_config(next_seed, config);
        let actual_next_draw =
            crate::sim_rng::u32(&mission_context, crate::sim_rng::RngSite::TitbitUpdate, ..);

        assert_eq!(mission, expected_mission);
        assert_eq!(next_seed, expected_next_seed);
        assert_eq!(next_config, config);
        assert_eq!(actual_next_draw, expected_next_draw);
    }

    fn scripted_snapshot_fixture() -> (
        Engine,
        LevelAssets,
        std::sync::Arc<crate::script_manager::ScriptProgram>,
        crate::sequence::SequenceId,
    ) {
        let scb = crate::scb::ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![crate::scb::ClassEntry {
                source_file: "snapshot_attachment_test.scs".to_owned(),
                class_name: "StartUp".to_owned(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        };
        let program = std::sync::Arc::new(
            crate::script_manager::ScriptProgram::from_scb(scb).expect("prepare test bytecode"),
        );
        let script_name = "snapshot_attachment_test".to_owned();
        let script =
            crate::engine::MissionScript::from_program(script_name.clone(), program.clone())
                .expect("minimal StartUp script");

        let mut assets = LevelAssets::new();
        assets.scripts.mission_name = Some(script_name.clone());
        std::sync::Arc::make_mut(&mut assets.scripts.mission_programs)
            .insert(script_name, program.clone());

        let mut inner = EngineInner::new();
        inner.scripts.install_mission(script);
        inner.scripts.attach_native_capabilities(&assets);
        inner
            .scripts
            .mission
            .as_mut()
            .expect("fixture mission script")
            .script_effects
            .emit_engine(crate::natives::EngineCommand::UpdateInformationBars);
        {
            let effects = &mut inner
                .scripts
                .mission
                .as_mut()
                .expect("fixture mission script")
                .script_effects;
            effects.emit_sound(crate::natives::SoundCommand::SuspendAll);
            effects.emit_engine(crate::natives::EngineCommand::ChooseVictoryDefeatText { id: 17 });
            effects.emit_barrier(crate::natives::DeferredCommand::FreezeAll { freeze: true });
        }

        let mut sequence = crate::sequence::Sequence::new();
        sequence.append_element(crate::sequence::SequenceElement::new(
            1,
            crate::element::Command::Generic,
            None,
        ));
        let sequence_id = inner.orders.sequence_manager.launch_sequence(sequence);

        (
            Engine {
                inner,
                bootstrap_open: false,
            },
            assets,
            program,
            sequence_id,
        )
    }

    fn decoded_engine(engine: &Engine) -> Engine {
        serde_json::from_str(&serde_json::to_string(engine).expect("serialize engine snapshot"))
            .expect("decode engine snapshot")
    }

    #[test]
    fn persisted_capture_rejects_nonfinite_state_and_original_parity_rng() {
        let (mut source, _, _, _) = scripted_snapshot_fixture();
        source.inner.feedback.cutscene_camera.zoom_factor = f32::NAN;
        assert!(source.capture_persisted_state().is_err());
        let serialized = serde_json::to_vec(&source).unwrap();
        assert!(serde_json::from_slice::<Engine>(&serialized).is_err());

        source.inner.feedback.cutscene_camera.zoom_factor = 1.0;
        source.inner.control.rng = super::super::SimulationRng::with_original_replay(vec![1]);
        assert!(source.capture_persisted_state().is_err());
        assert!(serde_json::to_vec(&source).is_err());
    }

    #[test]
    fn persisted_projection_matches_disk_reconstruction_and_reattaches_script_resources() {
        let (mut source, assets, program, _) = scripted_snapshot_fixture();
        source
            .inner
            .ai
            .global
            .primary_target_multiplicity_initialized = true;
        let raw = source.clone();
        let projection = source.capture_persisted_state().unwrap();
        assert_eq!(
            serde_json::to_vec(&projection).unwrap(),
            serde_json::to_vec(&source).unwrap()
        );
        let projected = Engine::from_persisted_state(projection);
        let disk = decoded_engine(&source);
        assert_eq!(
            projected.encode_native_snapshot(),
            disk.encode_native_snapshot()
        );
        assert_eq!(
            crate::replay::state_hash(&projected),
            crate::replay::state_hash(&disk)
        );
        assert!(
            !projected
                .inner
                .ai
                .global
                .primary_target_multiplicity_initialized
        );
        assert!(raw.inner.ai.global.primary_target_multiplicity_initialized);
        assert!(std::sync::Arc::ptr_eq(
            &raw.inner.scripts.mission.as_ref().unwrap().manager.program,
            &program
        ));
        assert!(!std::sync::Arc::ptr_eq(
            &projected
                .inner
                .scripts
                .mission
                .as_ref()
                .unwrap()
                .manager
                .program,
            &program
        ));
        let mut projected_display = super::super::HostDisplayState::default();
        let mut disk_display = super::super::HostDisplayState::default();
        let projected =
            Engine::restore_from_snapshot(&mut projected_display, projected, &assets).unwrap();
        let disk = Engine::restore_from_snapshot(&mut disk_display, disk, &assets).unwrap();
        projected.inner.scripts.assert_native_attachments_ready();
        assert!(std::sync::Arc::ptr_eq(
            &projected
                .inner
                .scripts
                .mission
                .as_ref()
                .unwrap()
                .manager
                .program,
            &program
        ));
        assert_eq!(
            projected.encode_native_snapshot(),
            disk.encode_native_snapshot()
        );
        assert_eq!(
            crate::replay::state_hash(&projected),
            crate::replay::state_hash(&disk)
        );
    }

    #[test]
    fn failed_construction_returns_the_same_campaign_allocation() {
        let mut profiles = crate::profiles::ProfileManager::default();
        profiles
            .missions
            .push(crate::profiles::MissionProfile::default());
        profiles.soldiers.push(crate::profiles::SoldierProfile {
            filename: "missing-construction-test-sprite".to_owned(),
            profile_name: "missing-construction-test-profile".to_owned(),
            ..crate::profiles::SoldierProfile::default()
        });

        let mut campaign = crate::campaign::Campaign::default();
        campaign.missions.push(crate::mission::Mission {
            profile_idx: Some(0),
            ..crate::mission::Mission::default()
        });
        campaign.current_mission_idx = Some(0);
        campaign.missions.reserve_exact(257);
        assert!(!campaign.missions.is_empty());
        let missions = campaign.missions.as_ptr();
        let mission_capacity = campaign.missions.capacity();

        let mut assets = LevelAssets::new();
        assets.profile_manager = std::sync::Arc::new(profiles);
        let mut loaded = crate::level_data::LoadedLevel::empty();
        loaded.mission.soldiers.push(crate::level_data::RawSoldier {
            position_x: 0,
            position_y: 0,
            direction: 0,
            action: 0,
            obstacle_index: 0,
            sector: 0,
            layer: 0,
            material: 0,
            profile_number: 0,
            profile_id: None,
            allegiance: None,
            command_interface: crate::human_control::CommandInterface::None,
            mission_role: crate::human_control::MissionRole::Combatant,
            combat_stance: crate::human_control::CombatStance::Aggressive,
            revealed: false,
            tower_guard: false,
            company_number: 0,
            drunk_level: 0,
            money: 0,
            subordinate_ids: Vec::new(),
            path_id: 0,
            alert_path_id: 0,
            script_class: None,
        });

        let result = Engine::new_preserving_campaign(EngineArgs {
            campaign,
            level: LevelLoadArgs {
                assets: &mut assets,
                level_directory: "",
                progress: &mut |_| {},
                loaded,
                bg_pixel_dims: (0.0, 0.0),
            },
            ground_mark_sprite: None,
            titbit_row_frame_counts: Vec::new(),
            rng_seed: 0,
            original_rng_replay: None,
            sim_config: SimConfig::default(),
        });

        let (error, returned) = match result {
            Ok(_) => panic!("missing sprite must fail construction"),
            Err(failure) => failure,
        };
        assert!(matches!(error, EngineError::ProfileSpriteLoadFailed { .. }));
        assert_eq!(returned.missions.as_ptr(), missions);
        assert_eq!(returned.missions.capacity(), mission_capacity);
        assert_eq!(returned.current_mission_idx, Some(0));
    }

    /// Serialized camera state belongs to the snapshot; the previous live
    /// engine is not an attachment source. Only immutable `LevelAssets` are
    /// admitted by the preparation path.
    #[test]
    fn restore_uses_snapshot_camera_state_not_previous_engine() {
        let mut source_inner = EngineInner::new();

        source_inner.feedback.cutscene_camera.level_size =
            crate::coordinates::MapSize::new(1234.0, 5678.0);

        let source = Engine {
            inner: source_inner,
            bootstrap_open: false,
        };

        let json = serde_json::to_string(&source).expect("serialize");
        let decoded: Engine = serde_json::from_str(&json).expect("deserialize");

        let mut display = crate::engine::HostDisplayState::default();
        let restored = Engine::restore_from_snapshot(&mut display, decoded, &LevelAssets::new())
            .expect("restore compatible snapshot");

        assert_eq!(
            restored.inner.feedback.cutscene_camera.level_size,
            crate::coordinates::MapSize::new(1234.0, 5678.0)
        );
    }

    #[test]
    fn try_restore_rejects_mismatched_runtime_lengths_without_mutating_live_engine() {
        let mut live_inner = EngineInner::new();
        live_inner.feedback.cutscene_camera.level_size =
            crate::coordinates::MapSize::new(1234.0, 5678.0);
        let live = Engine {
            inner: live_inner,
            bootstrap_open: false,
        };

        let mut malformed_inner = EngineInner::new();
        malformed_inner.world.fast_grid_mut().line_active.push(true);
        let malformed = Engine {
            inner: malformed_inner,
            bootstrap_open: false,
        };

        let mut display = crate::engine::HostDisplayState::default();
        let error = Engine::restore_from_snapshot(&mut display, malformed, &LevelAssets::new())
            .err()
            .expect("malformed snapshot must be rejected");
        assert_eq!(
            error,
            SnapshotRestoreError::FastGridLengthMismatch {
                component: SnapshotGridComponent::Lines,
                snapshot_len: 1,
                level_len: 0,
            }
        );
        assert_eq!(
            live.inner.feedback.cutscene_camera.level_size,
            crate::coordinates::MapSize::new(1234.0, 5678.0),
            "validation must happen before replacing the live engine"
        );
    }

    #[test]
    fn try_restore_rejects_world_parallel_mismatch_before_mutating_live_engine() {
        let live = Engine {
            inner: EngineInner::new(),
            bootstrap_open: false,
        };
        let mut malformed_inner = EngineInner::new();
        malformed_inner
            .script_domains
            .zones
            .scripts
            .push(crate::sector::ScriptSectorData::new());
        let malformed = Engine {
            inner: malformed_inner,
            bootstrap_open: false,
        };

        let mut display = crate::engine::HostDisplayState::default();
        let error = Engine::restore_from_snapshot(&mut display, malformed, &LevelAssets::new())
            .err()
            .expect("malformed snapshot must be rejected");
        assert_eq!(
            error,
            SnapshotRestoreError::WorldInvariantViolation {
                detail: "script-zone runtime length 1 does not match level zone-index length 0"
                    .to_owned(),
            }
        );
        assert!(live.inner.script_domains.zones.scripts.is_empty());
    }

    #[test]
    fn network_adoption_is_fully_attached_and_preserves_hash_and_script_queue() {
        let (source, assets, program, sequence_id) = scripted_snapshot_fixture();
        let source_hash = crate::replay::state_hash(&source);
        let snapshot = decoded_engine(&source);
        let live = Engine::adopt_authoritative_snapshot(snapshot, &assets)
            .expect("adopt compatible snapshot");

        assert_eq!(crate::replay::state_hash(&live), source_hash);
        live.inner.scripts.assert_native_attachments_ready();
        let script = live.inner.scripts.mission.as_ref().expect("adopted script");
        assert!(std::sync::Arc::ptr_eq(&script.manager.program, &program));
        assert!(std::sync::Arc::ptr_eq(
            &script.bindings.profile_manager,
            &assets.profile_manager
        ));
        assert!(matches!(
            script.script_effects.ordered.as_slices(),
            (
                [
                    crate::natives::ScriptEffect::Presentation(
                        crate::natives::EngineCommand::UpdateInformationBars
                    ),
                    crate::natives::ScriptEffect::ExternalSound(
                        crate::natives::SoundCommand::SuspendAll
                    ),
                    crate::natives::ScriptEffect::Simulation(
                        crate::natives::SimulationEffect::Engine(
                            crate::natives::EngineCommand::ChooseVictoryDefeatText { id: 17 }
                        )
                    ),
                    crate::natives::ScriptEffect::Simulation(
                        crate::natives::SimulationEffect::Deferred(
                            crate::natives::DeferredCommand::FreezeAll { freeze: true }
                        )
                    )
                ],
                []
            )
        ));
        assert_eq!(
            live.inner
                .orders
                .sequence_manager
                .get_sequence(sequence_id)
                .map(|sequence| sequence.id),
            Some(sequence_id),
            "serialized sequences must be addressable after lookup indices rebuild"
        );
    }

    #[test]
    fn save_restore_attaches_before_fixups_and_appends_save_only_hud_repair() {
        let (mut source, assets, program, _) = scripted_snapshot_fixture();
        source.inner.feedback.cutscene_camera.level_size =
            crate::coordinates::MapSize::new(4096.0, 4096.0);
        source
            .inner
            .feedback
            .cutscene_camera
            .display
            .background_transform
            .zoom_to_up = true;
        source.inner.feedback.cutscene_camera.zoom_init_done = true;
        let queued_engine_commands = source
            .inner
            .scripts
            .mission
            .as_ref()
            .expect("fixture script")
            .script_effects
            .engine_commands()
            .len();
        let snapshot = decoded_engine(&source);
        let mut display = super::super::HostDisplayState::default();

        let observed_fixups_before_hud_repair = std::cell::Cell::new(false);
        let mut live =
            Engine::restore_from_snapshot_with_observer(&mut display, snapshot, &assets, |inner| {
                observed_fixups_before_hud_repair.set(true);
                assert_eq!(
                    inner.orders.messenger.count(),
                    3,
                    "zoom-end, stature, and select-action must already be queued"
                );
                assert_eq!(
                    inner
                        .scripts
                        .mission
                        .as_ref()
                        .expect("restored script during fixup observation")
                        .script_effects
                        .engine_commands()
                        .len(),
                    queued_engine_commands,
                    "save-only HUD repair must not be queued until engine fixups finish"
                );
            })
            .expect("restore compatible save snapshot");
        assert!(observed_fixups_before_hud_repair.get());

        live.inner.scripts.assert_native_attachments_ready();
        let script = live
            .inner
            .scripts
            .mission
            .as_ref()
            .expect("restored script");
        assert!(std::sync::Arc::ptr_eq(&script.manager.program, &program));
        assert_eq!(
            script.script_effects.engine_commands().len(),
            queued_engine_commands + 1,
            "saved queue must survive and save-load must append one HUD repair"
        );
        let messages = live.inner.orders.messenger.drain();
        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[0].msg_type,
            crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::ZoomUpEnd)
        );
        assert_eq!(
            messages[1].msg_type,
            crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature)
        );
        assert!(matches!(
            messages[2].msg_type,
            crate::messenger::MessageType::Pc(crate::messenger::PcMessage::SelectAction, _)
        ));
        assert_eq!(display.display_op, crate::engine::DisplayOpCode::Redraw);
    }

    #[test]
    fn failed_attachment_preflight_does_not_mutate_live_engine() {
        let (source, mut assets, _, _) = scripted_snapshot_fixture();
        let snapshot = decoded_engine(&source);
        assets.scripts.mission_programs = std::sync::Arc::new(std::collections::BTreeMap::new());

        let mut live_inner = EngineInner::new();
        live_inner.control.frame_counter = 77;
        let live = Engine {
            inner: live_inner,
            bootstrap_open: false,
        };
        let before_hash = crate::replay::state_hash(&live);

        let error = Engine::adopt_authoritative_snapshot(snapshot, &assets)
            .err()
            .expect("snapshot with missing attachment must be rejected");
        assert!(matches!(
            error,
            SnapshotRestoreError::AttachmentFailure { ref detail }
                if detail.contains("missing mission script program 'snapshot_attachment_test'")
        ));
        assert_eq!(crate::replay::state_hash(&live), before_hash);
        assert_eq!(live.frame_counter(), 77);
    }

    #[test]
    fn adoption_rejects_wrong_loaded_mission_identity() {
        let (source, mut assets, _, _) = scripted_snapshot_fixture();
        let snapshot = decoded_engine(&source);
        assets.scripts.mission_name = Some("different_mission".to_owned());
        let live = Engine {
            inner: EngineInner::new(),
            bootstrap_open: false,
        };

        let error = Engine::adopt_authoritative_snapshot(snapshot, &assets)
            .err()
            .expect("snapshot for wrong mission must be rejected");
        assert!(matches!(
            error,
            SnapshotRestoreError::AttachmentFailure { ref detail }
                if detail.contains("does not match loaded mission script 'different_mission'")
        ));
        assert!(live.inner.scripts.mission.is_none());
    }

    #[test]
    fn adoption_rejects_mobile_count_from_level_assets_atomically() {
        let mut assets = LevelAssets::new();
        assets.entities.mobile_element_count = 1;
        let snapshot = Engine {
            inner: EngineInner::new(),
            bootstrap_open: false,
        };
        let mut live_inner = EngineInner::new();
        live_inner.control.frame_counter = 91;
        let live = Engine {
            inner: live_inner,
            bootstrap_open: false,
        };
        let before_hash = crate::replay::state_hash(&live);

        let error = Engine::adopt_authoritative_snapshot(snapshot, &assets)
            .err()
            .expect("snapshot with wrong mobile count must be rejected");
        assert_eq!(
            error,
            SnapshotRestoreError::WorldInvariantViolation {
                detail: "snapshot mobile-element count 0 does not match loaded level count 1"
                    .to_owned()
            }
        );
        assert_eq!(crate::replay::state_hash(&live), before_hash);
        assert_eq!(live.frame_counter(), 91);
    }

    #[test]
    fn adoption_rejects_malformed_fog_grid_atomically() {
        let assets = LevelAssets::new();
        let mut malformed_inner = EngineInner::new();
        malformed_inner.set_level_size(192.0, 144.0);
        malformed_inner
            .players
            .fog_of_war
            .corrupt_visible_region_for_test();
        let snapshot = Engine {
            inner: malformed_inner,
            bootstrap_open: false,
        };

        let mut live_inner = EngineInner::new();
        live_inner.control.frame_counter = 92;
        let live = Engine {
            inner: live_inner,
            bootstrap_open: false,
        };
        let before_hash = crate::replay::state_hash(&live);

        let error = Engine::adopt_authoritative_snapshot(snapshot, &assets)
            .err()
            .expect("malformed fog grid must be rejected");
        assert_eq!(
            error,
            SnapshotRestoreError::FogOfWarInvariantViolation {
                detail: "uninitialized fog state must be the exact empty 0x0 state".to_owned(),
            }
        );
        assert_eq!(crate::replay::state_hash(&live), before_hash);
        assert_eq!(live.frame_counter(), 92);
    }
}
