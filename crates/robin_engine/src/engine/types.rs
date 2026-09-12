//! EngineInner-related type definitions.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

// BTreeMap (not HashMap) so iteration order is deterministic — per-actor
// script state is part of the rollback simulation snapshot, and any
// iteration during updates / native callbacks must produce the same
// order on every client.
use std::collections::{BTreeMap, BTreeSet};

use crate::coordinates::{MapPoint, MapSize, MapVec, ScreenPoint};
use crate::natives::{NativeContext, ScriptEffects, ScriptState};
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

/// Level ambiance type (day, night, fog, etc.).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum Ambiance {
    #[serde(alias = "day", alias = "DAY")]
    #[default]
    Day,
    #[serde(alias = "fog", alias = "FOG")]
    Fog,
    #[serde(alias = "night", alias = "NIGHT")]
    Night,
    #[serde(alias = "attack", alias = "ATTACK")]
    Attack,
    #[serde(alias = "custom1", alias = "custom_1", alias = "CUSTOM_1")]
    Custom1,
    #[serde(alias = "custom2", alias = "custom_2", alias = "CUSTOM_2")]
    Custom2,
    #[serde(alias = "custom3", alias = "custom_3", alias = "CUSTOM_3")]
    Custom3,
    #[serde(alias = "custom4", alias = "custom_4", alias = "CUSTOM_4")]
    Custom4,
}

impl Ambiance {
    /// Map from the AMBIANCE_* integer constants.
    /// DAY=1, FOG=2, NIGHT=4, ATTACK=8, CUSTOM_1=16, CUSTOM_2=32,
    /// CUSTOM_3=64, CUSTOM_4=128. These are bitflags but only one is set.
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            1 => Ambiance::Day,
            2 => Ambiance::Fog,
            4 => Ambiance::Night,
            8 => Ambiance::Attack,
            16 => Ambiance::Custom1,
            32 => Ambiance::Custom2,
            64 => Ambiance::Custom3,
            128 => Ambiance::Custom4,
            _ => {
                tracing::warn!("Unknown ambiance value {}, defaulting to Day", raw);
                Ambiance::Day
            }
        }
    }

    /// Subdirectory name for map/minimap files.
    pub fn directory(&self) -> &'static str {
        match self {
            Ambiance::Day => "Day",
            Ambiance::Fog => "Fog",
            Ambiance::Night => "Night",
            Ambiance::Attack => "Attack",
            Ambiance::Custom1 => "Custom1",
            Ambiance::Custom2 => "Custom2",
            Ambiance::Custom3 => "Custom3",
            Ambiance::Custom4 => "Custom4",
        }
    }

    /// Convert to sprite_scriptor's Ambiance enum for .rhs file resolution.
    /// Attack/Custom_* use Day sprites (the shipping game has no dedicated
    /// sprite dictionaries for those ambiances — they reuse Day/Night art).
    pub fn to_sprite_ambiance(self) -> crate::sprite_script::Ambiance {
        match self {
            Ambiance::Day
            | Ambiance::Attack
            | Ambiance::Custom1
            | Ambiance::Custom2
            | Ambiance::Custom3
            | Ambiance::Custom4 => crate::sprite_script::Ambiance::Day,
            Ambiance::Fog => crate::sprite_script::Ambiance::Fog,
            Ambiance::Night => crate::sprite_script::Ambiance::Night,
        }
    }

    /// Convert to AMBIANCE_* bitmask for sound source filtering.
    /// DAY=1, FOG=2, NIGHT=4, ATTACK=8, CUSTOM_1..4=16/32/64/128.
    pub fn to_bitmask(self) -> u32 {
        match self {
            Ambiance::Day => 1,
            Ambiance::Fog => 2,
            Ambiance::Night => 4,
            Ambiance::Attack => 8,
            Ambiance::Custom1 => 16,
            Ambiance::Custom2 => 32,
            Ambiance::Custom3 => 64,
            Ambiance::Custom4 => 128,
        }
    }

    pub fn night_color_rgb(&self) -> (u8, u8, u8) {
        // The tint colour switch only lists Day/Fog/Night; the extra
        // ambiances fall through and are tinted like Day.
        match self {
            Ambiance::Day
            | Ambiance::Attack
            | Ambiance::Custom1
            | Ambiance::Custom2
            | Ambiance::Custom3
            | Ambiance::Custom4 => (45, 45, 35),
            Ambiance::Fog => (85, 77, 90),
            Ambiance::Night => (0, 0, 0),
        }
    }

    /// Initial `standard_view_polygon_radius` derived from the ambiance
    /// at header-load time. DAY / ATTACK / CUSTOM_1..4 default to the
    /// daytime view radius (400), FOG / NIGHT to the night view radius
    /// (300).
    pub fn default_view_polygon_radius(&self) -> u16 {
        match self {
            Ambiance::Fog | Ambiance::Night => crate::ai_vision::NIGHT_VIEW_RADIUS,
            Ambiance::Day
            | Ambiance::Attack
            | Ambiance::Custom1
            | Ambiance::Custom2
            | Ambiance::Custom3
            | Ambiance::Custom4 => crate::ai_vision::DEFAULT_VIEW_RADIUS,
        }
    }
}

#[path = "camera_state.rs"]
mod camera_state;
pub use camera_state::*;

#[path = "level_assets.rs"]
mod level_assets;
pub use level_assets::*;

/// Per-simulation-stream scratch rebuilt from canonical engine state.
///
/// This is deliberately outside [`LevelAssets`] and outside serialized
/// [`super::EngineInner`] state. AI code uses these borrow-breaking
/// snapshots while dispatching a tick, but they are derived data and
/// must not be shared between live simulation and rollback replay.
#[derive(Clone, Default)]
pub struct SimScratch {
    pub ai_entity_views: crate::ai_entity_view::SharedAiEntityViews,
    pub ai_sight_obstacles: crate::sight_obstacle::SharedSightObstacles,
}

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

/// Countdown visibility requested by a mission author.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub enum MissionCountdownMode {
    /// Keep the tracker visible throughout the active mission.
    #[default]
    Always,
    /// Show only once the authored warning threshold is reached.
    FinalOnly,
    /// Do not draw a tracker. Expiry still remains authoritative.
    Hidden,
}

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

/// Ordered request to change minimap visibility. The tuple wire representation
/// is retained so naming these independent flags does not change snapshots.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
#[serde(from = "(bool, bool)", into = "(bool, bool)")]
pub struct MinimapDisplayRequest {
    pub show: bool,
    pub restore_position: bool,
}

impl robin_util::state_hash::StateHash for MinimapDisplayRequest {
    fn state_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (self.show, self.restore_position).state_hash(state);
    }
}

impl From<(bool, bool)> for MinimapDisplayRequest {
    fn from((show, restore_position): (bool, bool)) -> Self {
        Self {
            show,
            restore_position,
        }
    }
}

impl From<MinimapDisplayRequest> for (bool, bool) {
    fn from(request: MinimapDisplayRequest) -> Self {
        (request.show, request.restore_position)
    }
}

#[cfg(test)]
mod minimap_display_request_tests {
    use super::MinimapDisplayRequest;
    use robin_util::state_hash::StateHash;
    use std::hash::Hasher;

    #[test]
    fn named_flags_retain_tuple_json_and_native_bytes() {
        for show in [false, true] {
            for restore_position in [false, true] {
                let tuple = (show, restore_position);
                let request = MinimapDisplayRequest::from(tuple);
                let json = serde_json::to_string(&request).unwrap();
                assert_eq!(json, serde_json::to_string(&tuple).unwrap());
                assert_eq!(bitcode::encode(&request), bitcode::encode(&tuple));
                let mut named_hash = std::collections::hash_map::DefaultHasher::new();
                let mut tuple_hash = std::collections::hash_map::DefaultHasher::new();
                request.state_hash(&mut named_hash);
                tuple.state_hash(&mut tuple_hash);
                assert_eq!(named_hash.finish(), tuple_hash.finish());
                let decoded: MinimapDisplayRequest = serde_json::from_str(&json).unwrap();
                assert_eq!((decoded.show, decoded.restore_position), tuple);
            }
        }
    }
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
/// directly — it pushes into `EngineInner::pending_side_effects`, which is
/// drained and handed to [`Host::apply_side_effects`] every frame.
///
/// This is the only channel through which sim-originated state reaches
/// the host. Rollback replay discards the produced `SideEffects` so
/// audio/UI aren't duplicated when a frame is re-simulated.
#[derive(
    Debug, Default, Clone, robin_state_hash_derive::StateHash, bitcode::Encode, bitcode::Decode,
)]
pub struct SideEffects {
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
    /// Sim asked the host to invalidate its cached background this tick.
    pub invalidate_background: bool,
    /// Sim asked the host to drop any cached trajectory preview this tick.
    /// Emitted from the scroll handlers and other places that invalidate
    /// world-to-screen aim previews. Host clears `host.valid_trajectory`
    /// on consume.
    pub invalidate_trajectory_preview: bool,
    /// Sim consumed a `ResetInput` broadcast this tick.  Host clears
    /// the rubber-band / click-suppression flags on `InputState` so a
    /// modal popup / dialog entered from a sequence command doesn't
    /// leave a pending drag or click armed.
    pub reset_input: bool,
    /// Fade-to-black overlay transition requested this tick.
    /// `Some(..)` = start/replace fade. `Some(None)` = clear fade.
    /// `None` = no change.
    pub fade_to_black: Option<Option<FadeToBlack>>,
    /// Toggle the masked / outline "draw hidden" display mode. `None` = no change.
    pub set_draw_hidden: Option<bool>,
    /// Whether the host should skip the render pass this frame.
    /// Used by fast-forward mode (render only every 32nd frame).
    pub skip_render: bool,
    /// Dialogue IDs queued this tick by `StartDialog` script commands.
    /// Host accumulates into its own queue and displays via the
    /// dialogue menu.
    pub pending_dialogues: Vec<i32>,
    /// Popup-scroll text IDs queued this tick by `DisplayPopupText` /
    /// `DisplayAllPopupTexts`. Host accumulates and renders through the
    /// popup parchment widget.
    pub pending_popup_texts: Vec<i32>,
    /// Debriefing text IDs queued this tick by the `DisplayAllDebriefings`
    /// cheat.
    pub pending_debriefings: Vec<crate::player_command::DebriefingTextId>,
    /// Set when the `DisplaySherwoodReport` script native fired this tick.
    pub pending_sherwood_report: bool,
    /// Exact authoritative outcomes for Sherwood sale commands.  Presentation
    /// waits for these receipts instead of assuming a click succeeded.
    pub trade_receipts: Vec<crate::trading::TradeReceipt>,
    /// Set when the `DisplayConsole` script native (or cheat key) fired
    /// this tick.
    pub pending_show_console: bool,
    /// Entities the sim asked to render a one-frame full-alpha outline
    /// on this tick.  Currently only populated by the
    /// `AddPCToMissionTeam` native, marking the PC after it is added.
    /// Host merges into [`CursorFeedback::marked_pc_ids`] each frame.
    pub pending_mark_pc_ids: Vec<crate::element::EntityId>,
    /// Deferred patch-effect background decal inserts and
    /// removals (`RestoreBackground`).  Produced by
    /// `process_patch_effects`; drained host-side where
    /// renderer-owned sprite textures are available (see
    /// `robin_rs::blit_to_map`).
    pub bg_blits: Vec<super::PendingBgBlit>,
    /// Set when a silent `Win(false)` fired this tick (ambush/tactical
    /// silent win). Host flips the Sherwood start-mission /
    /// quit-mission widgets.
    pub pending_silent_win_widget_swap: bool,
    /// Set on the first-frame-after-mission-won mission-state banner.
    /// Host drains the flag, flips `quit_mission_enabled` to false,
    /// and shows the "you may leave the mission now" popup; choosing
    /// Yes then drives the normal quit-mission flow.
    pub pending_mission_state_notice: bool,
    /// `CenterOn` forces a rubber-band cancel (clears the multi-select
    /// / multi-unselect flags). The host clears the two flags on
    /// [`InputState`] in `apply_side_effects`.
    pub cancel_multi_selection: bool,
    /// Set when `SimpleMessage::ResetInput` was consumed from the
    /// messenger this tick. Zeroes the cached mouse/keyboard state
    /// and drops held-key edges after a modal closes. The host drains
    /// this by clearing the ThreadedInput pressed-key cache, resetting
    /// latch state, and re-syncing the cursor.
    pub pending_reset_input: bool,
    /// Swordfight-drag ignore-mouse-event bracket: when the selected PC
    /// was swordfighting at the start of `perform_hourglass` but is no
    /// longer swordfighting after the per-element / sequence-manager
    /// hourglass pass, and a drag is in flight, the engine calls
    /// `ignore_mouse_event(true, true, true)` so the in-flight drag
    /// doesn't leak into a left-click release the frame the swordfight
    /// ends.  Host drains this: if the flag is set and `is_dragging`
    /// is true, it flips `ignore_next_left_click`, `ignore_next_drag`,
    /// and `next_left_double_is_simple` on `InputState`.
    pub pending_swordfight_drag_ignore: bool,
    /// Sim observed `SimpleMessage::UiHasFocus` on the messenger this tick.
    /// Host display preparation clears `InputState.controls.has_focus` before
    /// later mouse dispatch. No separate host latch is needed by current code.
    /// TODO: port the original RHDISPLAY_INITZOOM focus gate when implementing
    /// that display path; preserve its per-frame message timing then.
    pub ui_has_focus: bool,
    /// New top-left of the deployed minimap when an accepted drag /
    /// resize / setup-time validation moved it this tick. The host
    /// drains this by writing the top-left into the active
    /// `PlayerProfile`'s `minimap_x` / `minimap_y` and persisting the
    /// profile.
    #[state_hash(skip)]
    #[bitcode(skip)]
    pub pending_minimap_position: Option<crate::coordinates::ScreenPoint>,
    /// Script/sequence-driven minimap show/hide requests produced this
    /// tick. The minimap itself is host-owned, so the game loop applies
    /// these to `HostDisplayState`.
    pub pending_minimap_display_maps: Vec<MinimapDisplayRequest>,
}

impl serde::Serialize for SideEffects {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PersistedSideEffects::capture(self).serialize(serializer)
    }
}
impl<'de> serde::Deserialize<'de> for SideEffects {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(PersistedSideEffects::deserialize(deserializer)?.into_runtime())
    }
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedSideEffects {
    code: crate::game_operation::GameCode,

    sounds: Vec<SoundCommand>,

    displayed_noises: Vec<crate::ai::Noise>,

    host_events: Vec<HostEvent>,

    overlay: Option<OverlayChange>,

    invalidate_background: bool,

    invalidate_trajectory_preview: bool,

    reset_input: bool,

    fade_to_black: Option<Option<FadeToBlack>>,

    set_draw_hidden: Option<bool>,

    skip_render: bool,

    pending_dialogues: Vec<i32>,

    pending_popup_texts: Vec<i32>,

    pending_debriefings: Vec<crate::player_command::DebriefingTextId>,

    pending_sherwood_report: bool,

    trade_receipts: Vec<crate::trading::TradeReceipt>,

    pending_show_console: bool,

    pending_mark_pc_ids: Vec<crate::element::EntityId>,

    bg_blits: Vec<super::PendingBgBlit>,

    pending_silent_win_widget_swap: bool,

    pending_mission_state_notice: bool,

    cancel_multi_selection: bool,

    pending_reset_input: bool,

    pending_swordfight_drag_ignore: bool,

    ui_has_focus: bool,

    pending_minimap_display_maps: Vec<MinimapDisplayRequest>,
}

impl PersistedSideEffects {
    pub(crate) fn capture(value: &SideEffects) -> Self {
        let SideEffects {
            code: _,
            sounds: _,
            displayed_noises: _,
            host_events: _,
            overlay: _,
            invalidate_background: _,
            invalidate_trajectory_preview: _,
            reset_input: _,
            fade_to_black: _,
            set_draw_hidden: _,
            skip_render: _,
            pending_dialogues: _,
            pending_popup_texts: _,
            pending_debriefings: _,
            pending_sherwood_report: _,
            trade_receipts: _,
            pending_show_console: _,
            pending_mark_pc_ids: _,
            bg_blits: _,
            pending_silent_win_widget_swap: _,
            pending_mission_state_notice: _,
            cancel_multi_selection: _,
            pending_reset_input: _,
            pending_swordfight_drag_ignore: _,
            ui_has_focus: _,
            pending_minimap_position: _,
            pending_minimap_display_maps: _,
        } = value;
        Self {
            code: value.code,
            sounds: value.sounds.clone(),
            displayed_noises: value.displayed_noises.clone(),
            host_events: value.host_events.clone(),
            overlay: value.overlay.clone(),
            invalidate_background: value.invalidate_background,
            invalidate_trajectory_preview: value.invalidate_trajectory_preview,
            reset_input: value.reset_input,
            fade_to_black: value.fade_to_black,
            set_draw_hidden: value.set_draw_hidden,
            skip_render: value.skip_render,
            pending_dialogues: value.pending_dialogues.clone(),
            pending_popup_texts: value.pending_popup_texts.clone(),
            pending_debriefings: value.pending_debriefings.clone(),
            pending_sherwood_report: value.pending_sherwood_report,
            trade_receipts: value.trade_receipts.clone(),
            pending_show_console: value.pending_show_console,
            pending_mark_pc_ids: value.pending_mark_pc_ids.clone(),
            bg_blits: value.bg_blits.clone(),
            pending_silent_win_widget_swap: value.pending_silent_win_widget_swap,
            pending_mission_state_notice: value.pending_mission_state_notice,
            cancel_multi_selection: value.cancel_multi_selection,
            pending_reset_input: value.pending_reset_input,
            pending_swordfight_drag_ignore: value.pending_swordfight_drag_ignore,
            ui_has_focus: value.ui_has_focus,
            pending_minimap_display_maps: value.pending_minimap_display_maps.clone(),
        }
    }

    pub(crate) fn into_runtime(self) -> SideEffects {
        SideEffects {
            code: self.code,
            sounds: self.sounds,
            displayed_noises: self.displayed_noises,
            host_events: self.host_events,
            overlay: self.overlay,
            invalidate_background: self.invalidate_background,
            invalidate_trajectory_preview: self.invalidate_trajectory_preview,
            reset_input: self.reset_input,
            fade_to_black: self.fade_to_black,
            set_draw_hidden: self.set_draw_hidden,
            skip_render: self.skip_render,
            pending_dialogues: self.pending_dialogues,
            pending_popup_texts: self.pending_popup_texts,
            pending_debriefings: self.pending_debriefings,
            pending_sherwood_report: self.pending_sherwood_report,
            trade_receipts: self.trade_receipts,
            pending_show_console: self.pending_show_console,
            pending_mark_pc_ids: self.pending_mark_pc_ids,
            bg_blits: self.bg_blits,
            pending_silent_win_widget_swap: self.pending_silent_win_widget_swap,
            pending_mission_state_notice: self.pending_mission_state_notice,
            cancel_multi_selection: self.cancel_multi_selection,
            pending_reset_input: self.pending_reset_input,
            pending_swordfight_drag_ignore: self.pending_swordfight_drag_ignore,
            ui_has_focus: self.ui_has_focus,
            pending_minimap_position: None,
            pending_minimap_display_maps: self.pending_minimap_display_maps,
        }
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
