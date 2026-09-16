//! EngineInner-related type definitions.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

// BTreeMap (not HashMap) so iteration order is deterministic — per-actor
// script state is part of the rollback simulation snapshot, and any
// iteration during updates / native callbacks must produce the same
// order on every client.
use std::collections::{BTreeMap, BTreeSet};

use crate::coordinates::{MapPoint, MapSize, MapVec, ScreenPoint};
use crate::natives::{NativeContext, ScriptState};
use crate::script_manager::{ScriptInstance, ScriptManager};

use super::{
    DEFAULT_SCROLLING_ACCELERATION, DEFAULT_SCROLLING_LIMIT, DEFAULT_SCROLLING_START,
    PANNEL_HEIGHT, SCROLLING_TABLE_SIZE, ZOOM_LEVEL_COUNT,
};

#[path = "simulation_rng.rs"]
mod simulation_rng;
pub(crate) use simulation_rng::*;

// ─── Display operation codes ─────────────────────────────────────────

/// What the renderer should do this frame with the background.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u8)]
pub enum DisplayOpCode {
    /// No operation needed.
    Nothing = 0,
    /// Background didn't move — just refresh elements.
    NoBackgroundMove = 1,
    /// Scroll the background by the current vector.
    Scroll = 2,
    /// Begin a zoom transition (prepare surfaces).
    InitZoom = 3,
    /// In the middle of a zoom transition.
    InZoom = 4,
    /// Full redraw required (cache invalid, first frame, etc.).
    #[default]
    Redraw = 5,
}

// ─── Scroll direction ────────────────────────────────────────────────

/// Cardinal directions for scrolling.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(usize)]
pub enum ScrollDirection {
    Up = 0,
    Left = 1,
    Right = 2,
    Down = 3,
}

impl ScrollDirection {
    pub const ALL: [ScrollDirection; 4] = [
        ScrollDirection::Up,
        ScrollDirection::Left,
        ScrollDirection::Right,
        ScrollDirection::Down,
    ];
}

// ─── EngineInner state changes ────────────────────────────────────────────

/// State change requests that can be sent to the engine.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(i32)]
pub enum EngineStateRequest {
    BeaconOff = 0,
    BeaconNext = 1,
    BeaconPrev = 2,
    Beacon = 3,
    BeaconViewerIndex = 4,
    LockerOn = 5,
    LockerOff = 6,
    TriangleOn = 7,
    TriangleOff = 8,
    NumberOfDynamite = 9,
    NumberOfHealingDose = 10,
    ZoomingUp = 11,
    ZoomingDown = 12,
    IsReloading = 13,
    NightDimish = 14,
    NightShadowColor = 15,
    IsSettingTimer = 16,
    EnterMenu = 17,
}

// ─── Ambiance ────────────────────────────────────────────────────────

pub use robin_engine_types::mission_environment::Ambiance;

#[path = "camera_state.rs"]
mod camera_state;
pub use camera_state::*;

#[path = "level_assets.rs"]
mod level_assets;
pub use level_assets::*;

// ─── Level-load staging data ────────────────────────────────────────

/// Raw data stashed during `initialize_from_mission` and consumed later
/// by `load_background_map` / `initialize_motion_from_level_data`.
///
/// These fields are transient: populated during the level load sequence,
/// fully drained before the first tick runs, and empty for the rest of
/// the mission. They are not simulation state and are never serialized.
#[derive(
    Clone, Default, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct LevelLoadStaging {
    /// Proto geometry that must wait until map dimensions are known.
    pub motion: MotionStageInput,
    /// Attachments produced while building geometry and consumed after the
    /// canonical authored door table exists.
    pub attachments: DeferredLevelAttachments,
}

/// Typed input to the motion/grid construction stage.
#[derive(
    Clone, Default, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct MotionStageInput {
    /// Motion data loaded from proto level, processed when background is loaded.
    pub motion_data: Option<crate::level_data::RawMotionData>,
    /// Lift proto data, consumed alongside motion data for sector fixup.
    pub lifts: Vec<crate::level_data::RawLift>,
    /// Raw mask chunk from the proto level, stashed until
    /// `initialize_motion_from_level_data` sizes + allocates the grid.
    pub masks: Vec<crate::level_data::RawMask>,
    /// Raw elevation-line chunk from the proto level (`BOND/007`), stashed
    /// until `initialize_motion_from_level_data` has sized and allocated
    /// the grid.
    pub elevation_lines: Vec<crate::level_data::RawElevationLine>,
    /// Raw jump zones from the JZ/PPPP proto chunk.
    pub jump_zones: Vec<crate::level_data::RawJumpZone>,
    /// Raw jump line pairs from the JZ/PPPP proto chunk.
    pub jump_line_pairs: Vec<crate::level_data::RawJumpLinePair>,
    /// Building sector_numbers allocated by `rewire_building_doors` during
    /// the initial level load.  Consumed by `initialize_motion_from_level_data`.
    pub building_sector_numbers: Vec<i16>,
    /// Raw light/shadow sectors from the LIGHT/DARK proto chunk.  Consumed by
    /// `initialize_motion_from_level_data` after the grid is sized and layers
    /// are allocated — each sector becomes a `SectorType::SHADOW` grid sector
    /// iff its ambience bitmask overlaps the mission's ambience.
    pub light_sectors: Vec<crate::level_data::RawLightSector>,
}

/// Typed late attachments that depend on both grid geometry and script domains.
#[derive(
    Clone, Default, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct DeferredLevelAttachments {
    /// Jump gates produced by proto geometry in exact jump-pair order.
    pub jump_gates: Vec<JumpGateAttachment>,
}

/// Deferred jump-gate attachment produced by the motion stage.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct JumpGateAttachment {
    pub point_out: crate::coordinates::MapPoint,
    pub point_in: crate::coordinates::MapPoint,
    pub layer_out: u16,
    pub layer_in: u16,
    pub sector_out: crate::sector::SectorNumber,
    pub sector_in: crate::sector::SectorNumber,
    pub sector_out_index: crate::fast_find_grid::SectorIndex,
    pub sector_in_index: crate::fast_find_grid::SectorIndex,
    pub jump_line_out: u32,
    pub jump_line_in: u32,
    pub jump_line_in_helper_needed: bool,
    pub jump_line_out_helper_needed: bool,
    pub penalty: f32,
}

#[path = "mission_script.rs"]
mod mission_script;
pub use mission_script::*;

// ─── Mission state ───────────────────────────────────────────────────

/// Tracks win/lose/interrupted conditions for the current mission.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MissionState {
    /// The mission has been won (objective completed).
    pub mission_won: bool,
    /// First frame where mission_won became true (triggers UI message).
    pub mission_won_first_time: bool,
    /// EngineInner should transition to "won" result this frame.
    pub quit_won: bool,
    /// EngineInner should transition to "lost" result this frame.
    pub quit_lost: bool,
    /// EngineInner should transition to "interrupted" result this frame.
    pub quit_interrupted: bool,

    /// Map filename from the mission header (e.g. "lincoln").
    pub map_name: String,

    /// Victory/defeat dialogue ID.
    pub victory_defeat_id: u32,

    /// Optional Rust-authored mission clocks and runtime ambience schedule.
    /// This state is authoritative: saves, rollback snapshots and network
    /// state hashes all retain the exact elapsed tick and transition phase.
    #[serde(default)]
    pub runtime_features: MissionRuntimeFeatures,
}

pub use robin_engine_types::mission_environment::MissionCountdownMode;

/// Deterministic state for a mission time limit.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct TimedMissionRuntime {
    pub limit_ticks: u32,
    pub warning_ticks: u32,
    pub countdown_mode: MissionCountdownMode,
    pub elapsed_ticks: u32,
    pub expired: bool,
}

/// One compiled ambience cue in active-play ticks.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct AmbienceRuntimeCue {
    pub at_tick: u32,
    pub ambiance: Ambiance,
    pub transition_ticks: u32,
}

/// In-progress deterministic lighting crossfade.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct AmbienceTransitionRuntime {
    pub started_at_tick: u32,
    pub duration_ticks: u32,
    pub from_color: u16,
    pub to_color: u16,
}

/// Compiled runtime ambience schedule and transition cursor.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct AmbienceScheduleRuntime {
    pub initial_ambiance: Ambiance,
    pub current_ambiance: Ambiance,
    pub elapsed_ticks: u32,
    pub cues: Vec<AmbienceRuntimeCue>,
    pub next_cue: u32,
    pub transition: Option<AmbienceTransitionRuntime>,
}

/// Optional mission extensions driven only by completed, interactive
/// simulation ticks. The Original's universal frame still advances through
/// some engine locks; this separate clock deliberately does not.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MissionRuntimeFeatures {
    pub active_elapsed_ticks: u32,
    pub timed_mission: Option<TimedMissionRuntime>,
    pub ambience_schedule: Option<AmbienceScheduleRuntime>,
}

/// Read-only countdown data used by local HUD presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionCountdownStatus {
    pub remaining_ticks: u32,
    pub limit_ticks: u32,
    pub warning_ticks: u32,
    pub mode: MissionCountdownMode,
    pub expired: bool,
}

#[path = "input_state.rs"]
mod input_state;
pub use input_state::*;

// ─── Weather ─────────────────────────────────────────────────────────

/// Weather and environmental state.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct WeatherState {
    /// Night shadow color (16-bit packed).
    pub night_color: u16,
    /// Whether this is a forest level.
    pub is_forest_level: bool,
    /// Current ambiance.
    pub ambiance: Ambiance,
}

impl WeatherState {
    pub fn new() -> Self {
        Self::default()
    }
}

// ─── Shield protection state ─────────────────────────────────────────

/// State for the shield protection mechanic.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ShieldState {
    pub is_protected: bool,
    /// The PC whose defensive arc is being honoured. `None` means no
    /// PC is protecting.
    pub protected_pc: Option<crate::element::EntityId>,
    /// Last danger point resolved by the two-click shield protocol. Original
    /// serializes this even while the first click is pending.
    pub danger_point: crate::coordinates::WorldPoint3D,
    /// Selected map layer paired with `danger_point`. Original resets its
    /// selected-layer scratch to zero after loading a save.
    pub danger_point_layer: u16,
}

// ─── Element index ───────────────────────────────────────────────────

/// Opaque handle into the element arrays.
/// TODO: replace with proper entity handles.
pub type ElementIndex = u32;

// ─── The EngineInner ──────────────────────────────────────────────────────

/// An anonymous countdown timer tracked by the engine.
///
/// One entry per sequence element with a timer countdown property that
/// decrements each frame.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct TimerEntry {
    /// Frames remaining, mirroring the original game's **signed** timer
    /// `int` property. Decremented every frame; the entry is removed only when
    /// the value is exactly 1, so a timer that starts at
    /// 0 counts down through negative values and never expires.
    pub remaining: i32,
    /// Back-reference to the sequence element driving this timer. On expiry
    /// the engine calls `SequenceManager::element_terminated(sequence_id,
    /// element_index)`, terminating the underlying sequence element.
    pub element_ref: crate::sequence::SequenceElementRef,
}

/// A sound playback command enqueued by simulation logic, drained by the
/// audio layer after the tick completes. Keeping audio out of the sim tick
/// lets rollback replay the tick N times without triggering duplicate
/// playback — the queue is cleared each frame regardless of replay count.
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum SoundCommand {
    /// Stop any currently-playing or queued exclamation for this actor.
    ///
    /// Emergency-priority speech issues this before starting the new
    /// line; without it, a death/ouch emergency can get stuck behind the
    /// previous wounded or combat remark.
    StopExclamation { actor_id: crate::element::EntityId },
    /// NPC/PC exclamation (speech bubble with localized audio).
    Exclamation {
        group: crate::sound::ExclamationGroup,
        profile_id: u32,
        exclamation_id: u16,
        /// `-1` = random variant.
        variant: i32,
        position: crate::coordinates::MapPoint,
        actor_id: Option<crate::element::EntityId>,
    },
    /// Positional FX (footsteps, impacts, etc.).
    Fx {
        fx_id: u32,
        position: crate::coordinates::MapPoint,
        material: Option<crate::sound_cache::Material>,
    },
    /// Sword-vs-sword clang.
    StrikeFx {
        strike_kind: crate::sound::StrikeKind,
        weapon1: crate::profiles::WeaponMaterial,
        weapon2: crate::profiles::WeaponMaterial,
        position: crate::coordinates::MapPoint,
    },
    /// Weapon-vs-armor impact.
    ImpactFx {
        impact_kind: crate::sound::ImpactKind,
        weapon: crate::profiles::WeaponMaterial,
        armor: crate::profiles::ArmorMaterial,
        position: crate::coordinates::MapPoint,
    },
    /// Camera-relative resume of all sound sources (level enter / wake).
    ResumeAllSources {
        position: crate::coordinates::MapPoint,
        zoom: f32,
    },
    /// Activate a previously-idle sound source by index.
    ActivateSource(usize),
    /// Reconcile host source channels after an authoritative ambience cue.
    RefreshAmbienceSources,
    /// Start playback for a delayed sound source whose engine-side
    /// countdown timer just hit zero. EngineInner immediately re-rolls the
    /// timer for the next play (using `sim_rng`) so the host doesn't
    /// touch sim state. Host just kicks off the audio playback.
    PlayDelayedSource(usize),
    /// Play a UI jingle.
    Jingle(crate::sound::Jingle),
    /// Update overall music mode (Quiet/Alert/Fight) based on villain alerts.
    /// Additive: bumps the target-mode weight but waits for the current track
    /// to finish before switching.
    SetMusicMode(crate::sound::MusicMode),
    /// Force the music mode immediately (resets weights + reloads track).
    /// Fired when `set_alert_status` carries the instant-music-change
    /// flag — notably on soldier death and when the overall villain
    /// alert drops back to Green, so combat music doesn't keep looping
    /// over an empty battlefield.
    ForceMusicMode(crate::sound::MusicMode),
}

// ─── Side effects ────────────────────────────────────────────────────

/// Ordered request to change minimap visibility and optionally restore its position.
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MinimapDisplayRequest {
    pub show: bool,
    pub restore_position: bool,
}

/// Changes the PC-info hover overlay applied post-tick by the host.
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum OverlayChange {
    Show { pc_id: crate::element::EntityId },
    Hide,
}

/// Ordered host-only work emitted by authoritative command/tick handlers.
/// These events never feed back into simulation decisions.
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostEvent {
    SetRightMouseDown { down: bool },
    ClearInputFocus,
    CancelMultiSelection { suppress_next_double: bool },
    Minimap(MinimapHostEvent),
    MacroUi(MacroUiHostEvent),
}

#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MinimapHostEvent {
    Resize {
        base: crate::coordinates::ScreenPoint,
        corner_size: crate::coordinates::ScreenSize,
        screen_width: f32,
        screen_height: f32,
    },
    MouseDown {
        click_pt: crate::coordinates::ScreenPoint,
        screen_width: f32,
        screen_height: f32,
    },
    MouseMove {
        mouse_pt: crate::coordinates::ScreenPoint,
        left_mouse_down: bool,
        screen_width: f32,
        screen_height: f32,
    },
    MouseUp {
        on_minimap: bool,
    },
    RightClick,
    Toggle,
    DisplayMap {
        show: bool,
        restore_position: bool,
    },
    HighlightDelayed {
        element_ids: Vec<u32>,
        screen_width: f32,
        screen_height: f32,
    },
    Tick,
}

#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MacroUiHostEvent {
    Tick {
        slots: Vec<MacroSlotLengths>,
        pc_ids: Vec<crate::element::EntityId>,
    },
    RearmTetris {
        slot: usize,
        slots: Vec<MacroSlotLengths>,
    },
    BlinkQa {
        pc_id: crate::element::EntityId,
        slot: usize,
    },
}

#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MacroSlotLengths {
    pub pc_id: crate::element::EntityId,
    pub lengths: [u16; crate::macro_store::NUMBER_OF_QA_MEMORY],
}

/// Outputs produced by one simulation tick that must be applied to the
/// host *after* the sim has finished. The sim never writes to the host
/// directly — it pushes into its feedback output, which is drained at the
/// admitted frame boundary. Game status and sound commands retain their
/// authoritative frame timing; presentation requests have explicit host phases.
///
/// This is the only channel through which sim-originated state reaches
/// the host. Rollback replay discards the produced `HostEffects` so
/// audio/UI aren't duplicated when a frame is re-simulated.
#[derive(
    Debug,
    Default,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct HostEffects {
    /// The game-state code returned by the tick (in-progress / succeeded / failed / interrupted).
    pub code: crate::game_operation::GameCode,
    /// Exclamations and other sim-originated sound triggers.
    pub sounds: Vec<SoundCommand>,
    /// Broadcast noises emitted this tick for the developer noise
    /// overlay. Host/game-session code drains these into `DevState`.
    pub displayed_noises: Vec<crate::ai::Noise>,
    /// Presentation/input work in exact production order.
    pub host_events: Vec<HostEvent>,
    /// PC-info hover overlay show/hide requested by the sim this tick.
    pub overlay: Option<OverlayChange>,
    /// Fade-to-black overlay transition requested this tick.
    /// `Some(..)` = start/replace fade. `Some(None)` = clear fade.
    /// `None` = no change.
    pub fade_to_black: Option<Option<FadeToBlack>>,
    /// Toggle the masked / outline "draw hidden" display mode. `None` = no change.
    pub set_draw_hidden: Option<bool>,
    /// Whether the host should skip the render pass this frame.
    /// Used by fast-forward mode (render only every 32nd frame).
    pub skip_render: bool,
    /// Modal requests are consumed by priority, preserving order within a phase.
    pub modals: Vec<crate::player_command::ModalKind>,
    /// Coalesced requests retain their own consumption phase.
    pub signals: Vec<super::HostSignal>,
    pub trade_receipts: Vec<crate::trading::TradeReceipt>,
    pub background_blits: Vec<super::PendingBgBlit>,
    /// Entities the sim asked to render a one-frame full-alpha outline
    /// on this tick.  Currently only populated by the
    /// `AddPCToMissionTeam` native, marking the PC after it is added.
    /// Host merges into [`CursorFeedback::marked_pc_ids`] each frame.
    pub pending_mark_pc_ids: Vec<crate::element::EntityId>,
    /// New top-left of the deployed minimap when an accepted drag /
    /// resize / setup-time validation moved it this tick. The host
    /// drains this by writing the top-left into the active
    /// `PlayerProfile`'s `minimap_x` / `minimap_y` and persisting the
    /// profile.
    #[state_hash(skip)]
    #[serde(skip)]
    pub pending_minimap_position: Option<crate::coordinates::ScreenPoint>,
    /// Script/sequence-driven minimap show/hide requests produced this
    /// tick. The minimap itself is host-owned, so the game loop applies
    /// these to `HostDisplayState`.
    pub pending_minimap_display_maps: Vec<MinimapDisplayRequest>,
}

impl HostEffects {
    /// In-memory equivalent of a save round trip, without running a codec.
    ///
    /// This exhaustive destructure is the single persistence decision point:
    /// `_` fields survive a save verbatim, bound fields are process-local and
    /// reset to what deserialization reconstructs.
    pub(crate) fn persisted_clone(&self) -> Self {
        let mut clone = self.clone();
        let Self {
            code: _,
            sounds: _,
            displayed_noises: _,
            host_events: _,
            overlay: _,
            fade_to_black: _,
            set_draw_hidden: _,
            skip_render: _,
            modals: _,
            signals: _,
            trade_receipts: _,
            background_blits: _,
            pending_mark_pc_ids: _,
            pending_minimap_position,
            pending_minimap_display_maps: _,
        } = &mut clone;
        *pending_minimap_position = None;
        clone
    }
}

// ─── Errors ──────────────────────────────────────────────────────────

/// Errors that can occur during engine operations.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("Failed to open proto-level file: {0}")]
    ProtoLevelNotFound(String),

    #[error("Failed to open mission file: {0}")]
    MissionNotFound(String),

    #[error("Proto-level and mission files do not match (CRC mismatch)")]
    ProtoMissionMismatch,

    #[error("Unknown chunk '{0}' in proto-level file")]
    UnknownProtoChunk(String),

    #[error("Unknown chunk '{0}' in mission file")]
    UnknownMissionChunk(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to load sprite for {kind} profile {profile_id}: {reason}")]
    ProfileSpriteLoadFailed {
        kind: &'static str,
        profile_id: u32,
        reason: String,
    },

    #[error("mission level stage '{stage}' failed: {reason}")]
    MissionLevelStage { stage: &'static str, reason: String },

    #[error(transparent)]
    MissionLevelBuild(#[from] MissionLevelBuildError),
}

/// Validation failures raised by the staged mission-level builder.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    thiserror::Error,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum MissionLevelBuildError {
    #[error(
        "mission '{mission}' has scripting enabled, but its mission VM with StartUp binding was not loaded"
    )]
    MissingMissionScript { mission: String },

    #[error(
        "standalone door {door_index} in building entry {entry_index} has illegal type {door_type} at ({x}, {y})"
    )]
    InvalidStandaloneDoorType {
        entry_index: usize,
        door_index: usize,
        door_type: u8,
        x: i16,
        y: i16,
    },

    #[error("proto level has {building_count} buildings but no motion data to allocate sectors")]
    MissingBuildingMotionData { building_count: usize },

    #[error("building {building_index} has no door; tenant attachment requires its first door")]
    BuildingWithoutDoor { building_index: usize },

    #[error(
        "building {building_index} first authored door {door_index} is missing from the canonical door table"
    )]
    MissingCanonicalBuildingDoor {
        building_index: usize,
        door_index: u32,
    },

    #[error(
        "mission tenant table has {tenant_count} entries but the proto level has {building_count} buildings"
    )]
    BuildingTenantCountMismatch {
        tenant_count: usize,
        building_count: usize,
    },

    #[error(
        "building {building_index} tenant references missing legacy element slot {element_index}"
    )]
    MissingBuildingTenant {
        building_index: usize,
        element_index: u16,
    },

    #[error("building {building_index} tenant at legacy element slot {element_index} is not human")]
    NonHumanBuildingTenant {
        building_index: usize,
        element_index: u16,
    },

    #[error(
        "patch {patch_index} references door {door_index}, but only {door_count} non-lift doors were authored"
    )]
    PatchDoorOutOfRange {
        patch_index: usize,
        door_index: u16,
        door_count: usize,
    },

    #[error("patch {patch_index} references missing {state} mask ({layer}, {mask_index})")]
    MissingPatchMask {
        patch_index: usize,
        state: String,
        layer: u16,
        mask_index: u16,
    },

    #[error(
        "patch attachment table has {attachment_count} entries but {patch_count} patches were authored"
    )]
    PatchAttachmentCountMismatch {
        attachment_count: usize,
        patch_count: usize,
    },

    #[error(
        "cannot retain exact legacy grid topology: {stream} source chunk order is missing for non-empty authored grid data"
    )]
    MissingGridChunkOrder { stream: String },

    #[error(
        "cannot retain exact legacy grid topology: {stream} contains duplicate {chunk} construction chunks"
    )]
    DuplicateGridConstructionChunk { stream: String, chunk: String },

    #[error(
        "patch {patch_index} has no retained effect entity, but the original game always constructs and serializes one"
    )]
    MissingPatchFxIdentity { patch_index: usize },

    #[error(
        "{lift_type} lift sector {sector_number} is missing a {endpoint} authored door endpoint"
    )]
    MissingLiftEndpoint {
        lift_type: String,
        sector_number: i16,
        endpoint: String,
    },
}
