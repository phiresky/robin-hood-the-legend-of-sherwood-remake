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
                .required_campaign_mut("selecting the next mission")
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

        let loaded = crate::level_data::LoadedLevel::empty_for_test();
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
        self.require_live_campaign("advancing a simulation frame");

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
        self.require_live_campaign("performing an engine tick");
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
        self.require_live_campaign("applying replay or network commands");
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

    fn require_live_campaign(&self, context: &str) {
        self.inner.mission_domain.required_campaign(context);
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
#[path = "rollback_safe/tests.rs"]
mod tests;
