//! EngineInner-related type definitions.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

// BTreeMap (not HashMap) so iteration order is deterministic — per-actor
// script state is part of the rollback simulation snapshot, and any
// iteration during `Hourglass` / native callbacks must produce the same
// order on every client.
use std::collections::BTreeMap;

use crate::coordinates::{MapPoint, MapSize, MapVec, ScreenPoint};
use crate::natives::{GameHost, NativeContext, ScriptState};
use crate::script_manager::{ScriptInstance, ScriptManager};

use super::{
    DEFAULT_SCROLLING_ACCELERATION, DEFAULT_SCROLLING_LIMIT, DEFAULT_SCROLLING_START,
    PANNEL_HEIGHT, SCROLLING_TABLE_SIZE, ZOOM_LEVEL_COUNT,
};

// ─── Deterministic simulation RNG ─────────────────────────────────────────

/// Engine-owned capability for deterministic gameplay randomness.
///
/// This always owns the one `fastrand::Rng`. Gameplay receives an explicit
/// [`crate::sim_rng::SimulationContext`] handle tied to this allocation;
/// cloning an engine snapshot deep-copies the current stream state rather than
/// sharing it with the live engine.
///
/// Original provenance: `original-code/launcher.cpp:763-765` seeds the one
/// process-wide C RNG, and gameplay consumers call that shared `rand()`
/// stream. Rust keeps ownership explicit so replay/save snapshots can carry
/// the exact corresponding state.
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct SimulationRng {
    #[serde(with = "simulation_rng_serde")]
    state: Arc<Mutex<fastrand::Rng>>,
}

impl Clone for SimulationRng {
    fn clone(&self) -> Self {
        Self::with_seed(self.seed())
    }
}

impl SimulationRng {
    #[allow(clippy::disallowed_methods)]
    pub(crate) fn with_seed(seed: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(fastrand::Rng::with_seed(seed))),
        }
    }

    pub(crate) fn context(
        &self,
        config: crate::engine::SimConfig,
    ) -> crate::sim_rng::SimulationContext {
        crate::sim_rng::SimulationContext::new(Arc::clone(&self.state), config)
    }

    pub(crate) fn seed(&self) -> u64 {
        self.state
            .lock()
            .expect("simulation RNG mutex poisoned")
            .get_seed()
    }

    #[allow(clippy::disallowed_methods)]
    pub(crate) fn reseed(&mut self, seed: u64) {
        *self.state.lock().expect("simulation RNG mutex poisoned") = fastrand::Rng::with_seed(seed);
    }
}

impl robin_util::state_hash::StateHash for SimulationRng {
    fn state_hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        robin_util::state_hash::StateHash::state_hash(
            &*self.state.lock().expect("simulation RNG mutex poisoned"),
            hasher,
        );
    }
}

mod simulation_rng_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::{Arc, Mutex};

    pub(super) fn serialize<S: Serializer>(
        state: &Arc<Mutex<fastrand::Rng>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        state
            .lock()
            .map_err(|_| serde::ser::Error::custom("simulation RNG mutex poisoned"))?
            .get_seed()
            .serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<Mutex<fastrand::Rng>>, D::Error> {
        let seed = u64::deserialize(deserializer)?;
        #[allow(clippy::disallowed_methods)]
        Ok(Arc::new(Mutex::new(fastrand::Rng::with_seed(seed))))
    }
}

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
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
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
)]
pub enum Ambiance {
    #[default]
    Day,
    Fog,
    Night,
    Attack,
    Custom1,
    Custom2,
    Custom3,
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

// ─── Background transform ────────────────────────────────────────────

/// All state related to background scrolling and zoom transitions.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct BackgroundTransform {
    // Scrolling state
    pub scroll_to_left: bool,
    pub scroll_to_up: bool,
    pub current_x_scrolling_level: u16,
    pub current_y_scrolling_level: u16,

    // Zoom state
    pub zoom_to_up: bool,
    pub zoom_to_down: bool,
    pub required_zoom_up: bool,
    pub required_zoom_down: bool,
    pub zoom_count: u16,
    pub number_of_zoom_steps: u16,

    /// Pre-computed scrolling speed tables (32 entries each).
    pub x_scrolling_values: [f32; SCROLLING_TABLE_SIZE],
    pub y_scrolling_values: [f32; SCROLLING_TABLE_SIZE],

    /// Current zoom level index (0 = half, 1 = normal, 2 = double).
    pub current_zoom_level: u16,
    /// The three zoom factors.
    pub zoom_values: [f32; ZOOM_LEVEL_COUNT],

    /// Center of the current zoom operation.
    pub center_zoom: MapVec,
    /// Clipped zoom offset.
    pub clipped_zoom: MapVec,
    /// Current scrolling vector for this frame.
    pub scrolling_vector: MapVec,

    /// Source zoom factor at the start of the active zoom transition.
    /// Valid only while `zoom_to_up` or `zoom_to_down` is set.
    pub zoom_from: f32,
    /// Target zoom factor for the active zoom transition.
    pub zoom_to: f32,
    /// Source view position at the start of the active zoom transition.
    pub view_from: MapPoint,
    /// Target view position for the active zoom transition.
    pub view_to: MapPoint,
}

impl Default for BackgroundTransform {
    fn default() -> Self {
        let mut bg = Self {
            scroll_to_left: false,
            scroll_to_up: false,
            current_x_scrolling_level: 0,
            current_y_scrolling_level: 0,
            zoom_to_up: false,
            zoom_to_down: false,
            required_zoom_up: false,
            required_zoom_down: false,
            zoom_count: 0,
            number_of_zoom_steps: 0,
            x_scrolling_values: [0.0; SCROLLING_TABLE_SIZE],
            y_scrolling_values: [0.0; SCROLLING_TABLE_SIZE],
            current_zoom_level: 1, // Start at 1x zoom
            zoom_values: [0.5, 1.0, 2.0],
            center_zoom: MapVec::ZERO,
            clipped_zoom: MapVec::ZERO,
            scrolling_vector: MapVec::ZERO,
            zoom_from: 1.0,
            zoom_to: 1.0,
            view_from: MapPoint::ZERO,
            view_to: MapPoint::ZERO,
        };
        bg.generate_scrolling_table();
        bg
    }
}

impl BackgroundTransform {
    /// Pre-compute the scrolling speed ramp.
    fn generate_scrolling_table(&mut self) {
        self.x_scrolling_values[0] = 0.0;
        self.y_scrolling_values[0] = 0.0;

        let mut value = DEFAULT_SCROLLING_START;
        for i in 1..SCROLLING_TABLE_SIZE {
            // Round up to even if odd
            if !(value as u16).is_multiple_of(2) {
                value += 1.0;
            }
            let floored = value.floor();
            self.x_scrolling_values[i] = floored;
            self.y_scrolling_values[i] = floored;

            if value < DEFAULT_SCROLLING_LIMIT {
                value *= DEFAULT_SCROLLING_ACCELERATION;
            }
        }
    }
}

// ─── Camera state ────────────────────────────────────────────────────

const DIRECTOR_CAMERA_VIEW_WIDTH: f32 = 1024.0;
const DIRECTOR_CAMERA_VIEW_HEIGHT: f32 = 768.0;

fn default_zoom_factor() -> f32 {
    1.0
}

/// Script/director camera state.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct CameraState {
    /// Top-left corner of the view in map coordinates.
    pub view_position: MapPoint,
    /// Previous frame's view position. Display interpolation scratch:
    /// kept on the legacy camera object for now, but excluded from
    /// deterministic snapshots and rollback hashes.
    #[serde(skip)]
    pub old_view_position: MapPoint,
    /// Target position for camera slide animations.
    pub camera_slide: MapPoint,
    /// Desired camera slide destination.
    pub camera_wanted: MapPoint,
    /// Speed of fixed camera movements (0 = not active).
    pub fixed_camera_speed: u16,

    /// Current zoom factor (0.5, 1.0, or 2.0).
    pub zoom_factor: f32,
    /// Previous frame's zoom factor. Display interpolation scratch;
    /// excluded from deterministic snapshots.
    #[serde(skip, default = "default_zoom_factor")]
    pub old_zoom_factor: f32,
    /// Target zoom factor for smooth zoom transitions.
    pub desired_zoom_factor: f32,
    /// Whether zoom initialization is done for current transition.
    /// Display-state machine scratch; excluded from sim snapshots.
    #[serde(skip)]
    pub zoom_init_done: bool,
    /// Whether the current zoom was triggered programmatically.
    /// Transient zoom-message context; excluded from sim snapshots.
    #[serde(skip)]
    pub mechanized_zoom: bool,

    /// Level size in map units.
    pub level_size: MapSize,

    // Elastic/follow-camera display interpolation buffer. It still
    // lives beside the shared script camera until the legacy director
    // pipeline is split from local presentation, but it is not part of
    // deterministic save/replay state.
    #[serde(skip)]
    pub displacement: MapVec,
    #[serde(skip)]
    pub displacement_counter: u16,

    /// Snapshot of the followed element's screen-space position when
    /// locker mode engaged (or was last retargeted).  The director work
    /// loop tries to keep the target at this exact screen point every
    /// frame.  Populated by `select_follow_element`. Not strictly
    /// serialization state, but while it lives on `EngineInner` it
    /// participates in serde/hash.
    pub position_saved: ScreenPoint,

    /// Currently-executing camera sequence element (zoom / scroll-to /
    /// lock-on). The dispatcher for `Command::CameraGoto`,
    /// `Command::ZoomLevel`, and `Command::LockCameraOn` stores the
    /// element here, and `perform_director_work` marks it terminated
    /// when the zoom / slide completes.
    pub sequence_element: Option<crate::sequence::SequenceElementRef>,

    /// Display-op/zoom-transition state for the shared script camera.
    ///
    /// This is engine-owned because it advances `view_position`,
    /// `zoom_factor`, and camera sequence completion. Host-local viewport
    /// scroll/zoom has its own state in `robin_rs::Host`.
    pub display: super::CameraDisplayState,

    /// Screen-space mouse position captured when a non-mechanized zoom
    /// request fires (host sets this before `EngineStateRequest::
    /// Zooming{Up,Down}`). At `DisplayOpCode::InitZoom`, display_state
    /// consumes it to bias `view_to` so the pixel under the mouse stays
    /// anchored during the zoom: `mouse_vector = (screen_center -
    /// mouse_screen) / zoom` when the UI is not focused and the zoom
    /// is not mechanized. `None` = treat as mechanized zoom (no mouse
    /// recentering). Host-input transient state; excluded from sim
    /// snapshots.
    #[serde(skip)]
    pub pending_zoom_mouse_screen: Option<ScreenPoint>,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            view_position: MapPoint::ZERO,
            old_view_position: MapPoint::ZERO,
            camera_slide: MapPoint::new(-1.0, -1.0), // -1 = inactive
            camera_wanted: MapPoint::ZERO,
            fixed_camera_speed: 0,
            zoom_factor: 1.0,
            old_zoom_factor: 1.0,
            desired_zoom_factor: 1.0,
            zoom_init_done: false,
            mechanized_zoom: false,
            level_size: MapSize::ZERO,
            displacement: MapVec::ZERO,
            displacement_counter: 0,
            position_saved: ScreenPoint::ZERO,
            sequence_element: None,
            display: super::CameraDisplayState::default(),
            pending_zoom_mouse_screen: None,
        }
    }
}

impl CameraState {
    /// Whether the camera slide is currently active.
    pub fn is_sliding(&self) -> bool {
        self.camera_slide.x >= 0.0
    }

    /// Deactivate the camera slide.
    pub(crate) fn stop_slide(&mut self) {
        self.camera_slide = MapPoint::new(-1.0, -1.0);
    }

    /// Clamp the view position so the camera stays within the level bounds.
    /// On double-axis over-clip (level smaller than the zoomed-out viewport
    /// on that axis), reset `zoom_factor` to 1.0 and return the origin.
    pub(crate) fn clip_view(&mut self) -> bool {
        let mut clipped_h = false;
        let mut clipped_v = false;

        if self.view_position.x < 0.0 {
            self.view_position.x = 0.0;
            clipped_h = true;
        }
        if self.view_position.y < 0.0 {
            self.view_position.y = 0.0;
            clipped_v = true;
        }

        let view_w = DIRECTOR_CAMERA_VIEW_WIDTH / self.zoom_factor;
        let view_h = (DIRECTOR_CAMERA_VIEW_HEIGHT - PANNEL_HEIGHT) / self.zoom_factor;

        let right_edge = self.view_position.x + view_w;
        if right_edge > self.level_size.x {
            if clipped_h {
                // Level narrower than viewport at current zoom: fall back
                // to 1× zoom and park at the origin.
                self.zoom_factor = 1.0;
                self.view_position = MapPoint::ZERO;
                return true;
            } else {
                self.view_position.x = self.level_size.x - view_w;
            }
            clipped_h = true;
        }

        let bottom_edge = self.view_position.y + view_h;
        if bottom_edge > self.level_size.y {
            if clipped_v {
                // Level shorter than viewport at current zoom.
                self.zoom_factor = 1.0;
                self.view_position = MapPoint::ZERO;
                return true;
            } else {
                self.view_position.y = self.level_size.y - view_h;
            }
            clipped_v = true;
        }

        clipped_h || clipped_v
    }
}

// ─── Host-emitted ramp consumed by the FADE_TO_BLACK opcode ─────────
// (Host struct itself moved to robin_rs::host. FadeToBlack stays here
// because `SideEffects` carries it.)

/// Two-phase pixel ramp scheduled by the `FADE_TO_BLACK` script opcode.
#[derive(
    Default,
    Clone,
    Copy,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct FadeToBlack {
    /// Total frames per phase (fade-out + fade-in each last `speed` frames).
    pub speed: u32,
    /// Frames left until the whole effect ends (counts down from `2*speed`).
    pub frames_remaining: u32,
}

impl FadeToBlack {
    /// Alpha (0..=255) of the black overlay for the current frame.
    ///
    /// Per-pixel ramp: fade-out iterates `pass = speed..1` with
    /// `scale = pass / speed` (first frame `scale = 1.0`, alpha `0`;
    /// last frame `scale = 1/speed`, alpha `(speed-1)*255/speed`), and
    /// fade-in is the symmetric reverse.
    pub fn current_alpha(self) -> u8 {
        if self.speed == 0 || self.frames_remaining == 0 {
            return 0;
        }
        // Phase 1 (fade-out): frames_remaining ∈ (speed..=2*speed], alpha rises.
        // Phase 2 (fade-in):  frames_remaining ∈ (0..=speed],      alpha falls.
        let num = if self.frames_remaining > self.speed {
            // pass = frames_remaining - speed; alpha = (1 - pass/speed) * 255
            //   = (speed - pass) * 255 / speed = (2*speed - frames_remaining) * 255 / speed.
            2 * self.speed - self.frames_remaining
        } else {
            // pass = frames_remaining; scale = (speed - (pass-1))/speed;
            //   alpha = (1 - scale) * 255 = (pass - 1) * 255 / speed.
            self.frames_remaining - 1
        };
        ((num * 255) / self.speed).min(255) as u8
    }

    /// Consume one frame after the live framebuffer has been presented.
    ///
    /// Returns whether another fade frame remains. Keeping this separate
    /// from drawing prevents throwaway screenshot and thumbnail renders
    /// from shortening the transition.
    pub fn advance_presented_frame(&mut self) -> bool {
        self.frames_remaining = self.frames_remaining.saturating_sub(1);
        self.frames_remaining > 0
    }
}

// ─── Level assets (immutable after load) ────────────────────────────

/// Host-side callback for per-pixel sprite opacity.
///
/// Wired at level-load time into [`LevelAssets::pixel_opacity`]: the host
/// owns the `FrameHolder` with the packed sprite banks and implements
/// this trait. The engine uses it to close the per-pixel sprite pick
/// path (transparent-color and night-shadow rejection) without
/// depending on `robin_assets`.
pub trait PixelOpacityLookup: Send + Sync {
    /// Return `true` if the pixel at local `(x, y)` within the sprite
    /// frame identified by `bank_id` is opaque.
    ///
    /// `night_shadow_color` is the ambient night-shadow RGB565 value
    /// (`Weather::night_color`); pixels matching it are treated as
    /// transparent unless `blue_pixels_are_in` is `true` (the engine
    /// passes the entity's `is_blipped` flag so blipped entities
    /// remain clickable in their shadow area).
    fn is_pixel_opaque(
        &self,
        bank_id: u32,
        x: u16,
        y: u16,
        night_shadow_color: u16,
        blue_pixels_are_in: bool,
    ) -> bool;
}

/// Immutable level assets loaded once per mission.
///
/// These are read-only after the level-load sequence completes. They
/// never change during gameplay and are identical across every client
/// in a multiplayer session. Not serialized — the host re-attaches
/// them after deserialization from the loaded level files.
///
/// `sprite_scriptor` is a rendering asset.
/// `hiking_paths` and `profile_manager` are shared via `Arc` so cloning
/// EngineInner for rollback snapshots is a cheap reference-count bump.
///
/// Note: the former `frame_holder: Arc<robin_assets::FrameHolder>` field
/// was removed in the engine carve-out (Decision 1) so the engine crate
/// does not depend on `robin_assets`. Frame-holder-dependent operations
/// (sprite-variant dictionary setup, `signature()` / `is_pixel_opaque`)
/// now live on the host side in `robin_rs`, and the per-pixel pick
/// path reaches the packed sprite data through [`PixelOpacityLookup`].
#[derive(Clone, Default)]
pub struct LevelAssets {
    /// Sprite script loader/cache. Loads `.rhs` animation profiles.
    /// Arc-wrapped — immutable after load, cheap to clone for rollback.
    pub sprite_scriptor: std::sync::Arc<crate::sprite_script::SpriteScriptor>,
    /// Static fast-find grid geometry built at level load. Runtime
    /// active/overlay bits live on `EngineInner::fast_grid`; snapshots
    /// reattach this Arc after decode.
    pub level_grid: std::sync::Arc<crate::fast_find_grid::LevelGrid>,
    /// Static pathfinder graph built at level load. Runtime pathfinder
    /// snapshots carry only the per-area state table; after decode the
    /// engine clones this baseline graph and reapplies those states.
    pub pathfinder_graph: std::sync::Arc<crate::pathfinder::PathGraph>,
    /// Hiking/patrol paths loaded from the mission file (PWAY/RAIL chunks).
    pub hiking_paths: std::sync::Arc<Vec<crate::level_data::RawHikingPath>>,
    /// Weapon / character profiles loaded from the CPF file.
    /// Shared via `Arc` with `Campaign`.
    pub profile_manager: std::sync::Arc<crate::profiles::ProfileManager>,
    /// "Bank changed" token used by the sprite-script cache to decide
    /// whether a per-profile cache entry needs reloading. The host writes
    /// this to its frame-holder signature after the sprite bank is
    /// loaded — engine code reads it during sprite-script lookups.
    pub bank_signature: u32,
    /// Immutable mission bytecode and script-indexed authored data.
    pub scripts: LevelScriptAssets,
    /// Immutable entity identities and construction-time script attachments.
    pub entities: LevelEntityAssets,
    // TODO(level-assets): migrate rendering, navigation, environment, and
    // audio fields into equivalent domain groups in focused follow-up slices.
    /// Host-provided per-pixel sprite hit-test callback. `None` before
    /// the host wires it up; engine code that wants per-pixel sprite
    /// pick behaviour falls back to bbox-only when missing.
    pub pixel_opacity: Option<std::sync::Arc<dyn PixelOpacityLookup>>,
    /// Localized peasant firstname pool (menu text IDs 100-121). Used
    /// to build civilian display names by picking a random
    /// firstname/surname for non-VIP peasants. Populated once at
    /// level-load when the text resource is attached.
    pub peasant_firstnames: Vec<String>,
    /// Localized peasant surname pool (menu text IDs 122-143).
    pub peasant_surnames: Vec<String>,
    /// Preloaded accessory-sprite prototypes, one per projectile
    /// `ObjectType` (arrow, stone, apple, net, wasp-nest, purse, coin,
    /// ale, cape). Loaded once at mission init via
    /// `EngineInner::preload_accessory_sprite_prototypes`; runtime spawn
    /// paths clone from here to hydrate `ElementData::sprite`.
    pub accessory_sprite_prototypes:
        std::collections::HashMap<crate::element::ObjectType, crate::sprite::Sprite>,
    /// Exclamation sample-length lookup table populated by the host
    /// at level load. Engine code consults it when an NPC starts
    /// speaking to schedule the deterministic MYTALK finish frame.
    /// `Arc` so cloning `LevelAssets` is a refcount bump.
    pub exclamation_durations: ExclamationDurations,
    /// Sample-length lookup for sound sources (sample id → sim frames).
    /// Populated by the host at level load from the decoded WAV lengths
    /// in `SoundCache::source_cache` after initializing the required
    /// source sample IDs.  The engine reads it when
    /// activating a `Single` / `Volatile` source to schedule the
    /// deterministic finish frame — so rollback replay reproduces the
    /// exact `sources.active` / `delete` transitions without depending
    /// on the audio backend's wall-clock playback-completion callback.
    pub source_durations: super::SourceDurations,
    /// Required sound-source sample IDs collected during proto-level
    /// loading. The host consumes this after `Engine::new` to populate
    /// `SoundCache::initialize_sound_source_cache`, immediately after
    /// the source manager is loaded.
    pub sound_source_required_ids: std::collections::BTreeSet<u32>,
    /// Water/hole zones for projectile-splash detection. Rebuilt from
    /// the proto material chunk at level load. Used by the water/hole
    /// determination path.
    pub water_zones: crate::water_zones::WaterZones,
    /// Full SECTOR_SOUND registry (material + polygon for every material
    /// sector) plus the map's default material. Used by the no-obstacle
    /// branch of `Engine::set_obstacle_and_material` to resolve footstep
    /// material from the actor's position. Rebuilt from
    /// `ProtoData::material_sectors` + `ProtoMisc::default_material` at
    /// level load.
    pub material_sectors: crate::material_sectors::MaterialSectors,
    /// Static sight obstacles loaded from the level (3D occluders).
    /// Wrapped in `Arc` so cloning `LevelAssets` is a refcount bump
    /// rather than a 600+ KB deep copy. Mutated only at level load
    /// time via `Arc::make_mut`. The runtime per-obstacle active flag
    /// (toggled by `PatchEffect::SwapObjects`) lives separately on
    /// `EngineInner::static_sight_obstacle_active` — that vec
    /// participates in rollback hashing; this immutable geometry does
    /// not.
    pub static_sight_obstacles: std::sync::Arc<Vec<crate::sight_obstacle::SightObstacle>>,
}

/// Script-facing immutable level data, grouped separately from rendering and
/// navigation assets. It is populated only while constructing a mission and is
/// borrowed read-only after [`Engine`](super::Engine) creation.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct LevelScriptAssets {
    /// Pre-decoded bytecode in host load order, keyed by mission base name.
    pub mission_programs: std::sync::Arc<
        std::collections::BTreeMap<String, std::sync::Arc<crate::script_manager::ScriptProgram>>,
    >,
    /// Exact mission script identity selected during construction.
    pub mission_name: Option<String>,
    /// Spellforge Lua name tables. Vanilla missions leave these empty.
    pub names: std::sync::Arc<crate::natives::ScriptNameBindings>,
    /// Number of authored script locations.
    pub location_count: usize,
    /// Number of point locations at the front of the location arrays.
    pub point_count: usize,
    /// Positions of points, lines, then sectors in authored order.
    pub location_positions: std::sync::Arc<Vec<(f32, f32)>>,
    /// Layers parallel to `location_positions`.
    pub location_layers: std::sync::Arc<Vec<u16>>,
    /// Motion-sector numbers parallel to `location_positions`.
    pub location_sectors: std::sync::Arc<Vec<u16>>,
    /// Number of buildings exposed to the mission script.
    pub building_count: usize,
    /// Number of hiking paths exposed to the mission script.
    pub hiking_path_count: usize,
    /// Fast-grid indices for authored script zones, in authored sector order.
    pub zone_grid_indices: std::sync::Arc<Vec<u32>>,
}

/// Immutable entity bindings created while loading a mission.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct LevelEntityAssets {
    /// Number of authored mobile elements required by compatible snapshots.
    pub mobile_element_count: usize,
    /// Patch index to optional FX actor handle, in proto-then-mission order.
    pub patch_animation_entities: std::sync::Arc<Vec<Option<i32>>>,
    /// Scroll entity IDs in authored creation order.
    pub scroll_entity_ids: Vec<super::EntityId>,
    /// Soldier load-order index to typed entity ID.
    pub soldier_entity_ids: Vec<super::EntityId>,
    /// Soldier load-order index to subordinate soldier load-order IDs.
    pub soldier_subordinate_ids: Vec<Vec<u16>>,
}

/// Sample duration in sim frames (40 ms each), keyed by
/// `(group, profile_id, exclamation_id)`. Lives on `EngineInner` (so it
/// rides along in rollback snapshots cheaply via `Arc`); the host
/// populates it at level load by walking the sound cache. EngineInner
/// reads this when an NPC speaks to schedule the deterministic
/// MYTALK finish — instead of waiting for the audio backend's
/// wall-clock playback completion, which doesn't replay during
/// rollback. As in the original sound hourglass, a missing sample has
/// length zero and completes at the next scheduling boundary.
pub type ExclamationDurations =
    std::sync::Arc<std::collections::BTreeMap<(crate::sound::ExclamationGroup, u32, u16), u32>>;

impl LevelAssets {
    /// Mutable access to sprite_scriptor during initialization.
    pub fn sprite_scriptor_mut(&mut self) -> &mut crate::sprite_script::SpriteScriptor {
        std::sync::Arc::make_mut(&mut self.sprite_scriptor)
    }

    pub fn new() -> Self {
        Self {
            sprite_scriptor: std::sync::Arc::new(crate::sprite_script::SpriteScriptor::new()),
            level_grid: std::sync::Arc::new(crate::fast_find_grid::LevelGrid::default()),
            pathfinder_graph: std::sync::Arc::new(crate::pathfinder::PathGraph::default()),
            hiking_paths: std::sync::Arc::new(Vec::new()),
            profile_manager: std::sync::Arc::new(crate::profiles::ProfileManager::new()),
            bank_signature: 0,
            scripts: LevelScriptAssets::default(),
            entities: LevelEntityAssets::default(),
            pixel_opacity: None,
            peasant_firstnames: Vec::new(),
            peasant_surnames: Vec::new(),
            accessory_sprite_prototypes: std::collections::HashMap::new(),
            exclamation_durations: std::sync::Arc::new(std::collections::BTreeMap::new()),
            source_durations: std::sync::Arc::new(std::collections::BTreeMap::new()),
            sound_source_required_ids: std::collections::BTreeSet::new(),
            water_zones: crate::water_zones::WaterZones::new(),
            material_sectors: crate::material_sectors::MaterialSectors::new(),
            static_sight_obstacles: std::sync::Arc::new(Vec::new()),
        }
    }

    /// Pick a deterministic firstname+surname for a civilian using
    /// `seed` as the index. Returns `None` if the name pool hasn't
    /// been populated.
    pub fn random_peasant_name(&self, seed: usize) -> Option<String> {
        if self.peasant_firstnames.is_empty() || self.peasant_surnames.is_empty() {
            return None;
        }
        let f = &self.peasant_firstnames[seed % self.peasant_firstnames.len()];
        let l = &self.peasant_surnames[(seed / self.peasant_firstnames.len().max(1) + seed * 7)
            % self.peasant_surnames.len()];
        Some(format!("{f} {l}"))
    }
}

/// Per-simulation-stream scratch rebuilt from canonical engine state.
///
/// This is deliberately outside [`LevelAssets`] and outside serialized
/// [`super::EngineInner`] state. AI code uses these borrow-breaking
/// snapshots while dispatching a tick, but they are derived data and
/// must not be shared between live simulation and rollback replay.
#[derive(Clone)]
pub struct SimScratch {
    pub ai_entity_views: crate::ai_entity_view::SharedAiEntityViews,
    pub ai_sight_obstacles: crate::sight_obstacle::SharedSightObstacles,
}

impl Default for SimScratch {
    fn default() -> Self {
        Self {
            ai_entity_views: std::sync::Arc::new(crate::ai_entity_view::AiEntityViewMap::new()),
            ai_sight_obstacles: crate::sight_obstacle::SharedSightObstacles::default(),
        }
    }
}

// ─── Level-load staging data ────────────────────────────────────────

/// Raw data stashed during `initialize_from_mission` and consumed later
/// by `load_background_map` / `initialize_motion_from_level_data`.
///
/// These fields are transient: populated during the level load sequence,
/// fully drained before the first tick runs, and empty for the rest of
/// the mission. They are not simulation state and are never serialized.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LevelLoadStaging {
    /// Proto geometry that must wait until map dimensions are known.
    pub motion: MotionStageInput,
    /// Attachments produced while building geometry and consumed after the
    /// canonical authored door table exists.
    pub attachments: DeferredLevelAttachments,
}

/// Typed input to the motion/grid construction stage.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
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
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DeferredLevelAttachments {
    /// Jump gates produced by proto geometry in exact jump-pair order.
    pub jump_gates: Vec<JumpGateAttachment>,
}

/// Deferred jump-gate attachment produced by the motion stage.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct JumpGateAttachment {
    pub point_out: crate::coordinates::MapPoint,
    pub point_in: crate::coordinates::MapPoint,
    pub layer_out: u16,
    pub layer_in: u16,
    pub sector_out: crate::sector::SectorNumber,
    pub sector_in: crate::sector::SectorNumber,
    pub jump_line_out: u32,
    pub jump_line_in: u32,
    pub jump_line_in_helper_needed: bool,
    pub jump_line_out_helper_needed: bool,
    pub penalty: f32,
}

// ─── Mission script ─────────────────────────────────────────────────

/// Runtime-only stack of active script callback receivers.
///
/// The Original brackets its process-global script receiver around every
/// dispatch. Keeping the equivalent values in a structural stack makes nested
/// inheritance explicit and prevents callback state from leaking into saves.
#[derive(Clone, Debug, Default)]
struct ScriptCallStack {
    frames: Vec<crate::natives::ScriptCallFrame>,
}

impl ScriptCallStack {
    fn current(&self) -> crate::natives::ScriptCallFrame {
        self.frames.last().copied().unwrap_or_default()
    }

    fn push(&mut self, frame: crate::natives::ScriptCallFrame) {
        self.frames.push(frame);
    }

    fn pop(&mut self) -> crate::natives::ScriptCallFrame {
        self.frames
            .pop()
            .expect("script call-frame stack underflow")
    }

    fn len(&self) -> usize {
        self.frames.len()
    }

    fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

/// Wraps the script VM for a single mission level.
///
/// Holds the `ScriptManager` (loaded `.scb` bytecode), the global
/// `ScriptInstance` (bound to `StartUp`), and per-actor script instances
/// that persist across event callbacks (`Initialize`, `ActionChange`,
/// `HandleEvent`, `FilterAIEvent`, `ProcessMessage`).
///
/// One global engine-script instance plus one per-actor instance, each
/// with its own persistent heap.
#[derive(Clone, robin_state_hash_derive::StateHash)]
pub struct MissionScript {
    /// Mission base filename used to reattach immutable bytecode from
    /// [`LevelAssets`] after snapshot deserialization.
    pub script_name: String,
    pub manager: ScriptManager,
    /// Persistent state belonging to the script subsystem. This is separate
    /// from `game_host`, which is only the transitional adapter for canonical
    /// engine state that has not yet moved to borrowed native capabilities.
    pub state: ScriptState,
    /// Immutable level-native capabilities. Snapshot decode intentionally
    /// leaves this detached; the engine's snapshot adoption/restore boundary
    /// restores it from [`LevelAssets`] before the VM can resume.
    #[state_hash(skip)]
    pub(crate) bindings: crate::natives::AttachedScriptBindings,
    /// Concrete script-native state. VMs borrow this through their
    /// transient trait-object host field only while a script call is
    /// executing, so snapshots keep the real state instead of losing it
    /// behind `Vm::host`'s serde skip.
    pub game_host: GameHost,
    /// Active callback receivers. This is runtime control state, excluded from
    /// serialization and hashing; snapshots are rejected while it is nonempty.
    #[state_hash(skip)]
    call_stack: ScriptCallStack,
    pub instance: ScriptInstance,
    /// Per-actor script instances, keyed by actor script handle.
    ///
    /// Each actor with a `script_class` gets a persistent `ScriptInstance`
    /// whose heap survives across calls. The host (`GameHost`) is NOT stored
    /// on these — it lives on the global `instance` and is transferred
    /// in/out for each per-actor call.
    pub actor_instances: BTreeMap<i32, ScriptInstance>,
    /// Per-zone script instances, keyed by zone index (0-based index into
    /// `EngineInner::script_zone_grid_indices`).
    ///
    /// Zones with a `script_class` get a persistent `ScriptInstance` for
    /// `Initialize`, `EnterZone(actor)`, and `ExitZone(actor)` callbacks.
    pub zone_instances: BTreeMap<usize, ScriptInstance>,
    /// Per-target script instances, keyed by target actor script handle.
    ///
    /// FX targets with a non-empty `script_class` get a persistent
    /// `ScriptInstance` whose heap survives across calls.  Each target
    /// is its own VM, with named functions like `ActivatedByListenable`,
    /// `ActivatedByApple`, `ActivatedByArrow`, etc.  Calls go through
    /// [`MissionScript::call_target_function`].
    pub target_instances: BTreeMap<i32, ScriptInstance>,
    /// Per-scroll script instances, keyed by scroll actor script handle.
    ///
    /// Scrolls with a non-empty `script_class` bind their class during
    /// scroll mission-stream init and then run their script's
    /// `Initialize()` in `initialize_all_scrolls`. `IsTaken(pc)` is
    /// dispatched later when a PC picks up the scroll.
    pub scroll_instances: BTreeMap<i32, ScriptInstance>,
    /// Per-waypoint script instances, keyed by `(hiking_path_index,
    /// waypoint_index)`.
    ///
    /// Waypoints whose command is `WaypointCommand::Script(class)` bind
    /// the class at level load, run their `Initialize()` once, and then
    /// receive `ReachPoint(actor)` every time an NPC arrives at that
    /// waypoint (dispatched from `execute_waypoint_script`). Each
    /// waypoint is its own VM instance so the heap persists across
    /// traversals.
    pub waypoint_instances: BTreeMap<(crate::ai::PathId, u8), ScriptInstance>,

    /// Has the script's `PostInitialize` entry point run yet?  The host's
    /// first post-refresh stage flips this after rendering and sound.
    /// Serialized so rollback replay reproduces the same frame boundary
    /// without a host-owned companion bool.
    pub post_initialized: bool,
}

#[derive(Serialize)]
struct MissionScriptSnapshotRef<'a> {
    script_name: &'a str,
    manager: &'a ScriptManager,
    state: &'a ScriptState,
    game_host: &'a GameHost,
    instance: &'a ScriptInstance,
    actor_instances: &'a BTreeMap<i32, ScriptInstance>,
    zone_instances: &'a BTreeMap<usize, ScriptInstance>,
    target_instances: &'a BTreeMap<i32, ScriptInstance>,
    scroll_instances: &'a BTreeMap<i32, ScriptInstance>,
    #[serde(with = "serde_json_any_key::any_key_map_sized")]
    waypoint_instances: BTreeMap<(crate::ai::PathId, u8), ScriptInstance>,
    post_initialized: bool,
}

impl Serialize for MissionScript {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if !self.call_stack.is_empty() {
            return Err(serde::ser::Error::custom(
                "cannot snapshot MissionScript during an active script callback",
            ));
        }
        MissionScriptSnapshotRef {
            script_name: &self.script_name,
            manager: &self.manager,
            state: &self.state,
            game_host: &self.game_host,
            instance: &self.instance,
            actor_instances: &self.actor_instances,
            zone_instances: &self.zone_instances,
            target_instances: &self.target_instances,
            scroll_instances: &self.scroll_instances,
            waypoint_instances: self.waypoint_instances.clone(),
            post_initialized: self.post_initialized,
        }
        .serialize(serializer)
    }
}

#[derive(Deserialize)]
struct MissionScriptSnapshot {
    script_name: String,
    manager: ScriptManager,
    state: ScriptState,
    game_host: GameHost,
    instance: ScriptInstance,
    actor_instances: BTreeMap<i32, ScriptInstance>,
    zone_instances: BTreeMap<usize, ScriptInstance>,
    target_instances: BTreeMap<i32, ScriptInstance>,
    scroll_instances: BTreeMap<i32, ScriptInstance>,
    #[serde(with = "serde_json_any_key::any_key_map")]
    waypoint_instances: BTreeMap<(crate::ai::PathId, u8), ScriptInstance>,
    post_initialized: bool,
}

impl<'de> Deserialize<'de> for MissionScript {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let snapshot = MissionScriptSnapshot::deserialize(deserializer)?;
        Ok(Self {
            script_name: snapshot.script_name,
            manager: snapshot.manager,
            state: snapshot.state,
            bindings: crate::natives::AttachedScriptBindings::default(),
            game_host: snapshot.game_host,
            call_stack: ScriptCallStack::default(),
            instance: snapshot.instance,
            actor_instances: snapshot.actor_instances,
            zone_instances: snapshot.zone_instances,
            target_instances: snapshot.target_instances,
            scroll_instances: snapshot.scroll_instances,
            waypoint_instances: snapshot.waypoint_instances,
            post_initialized: snapshot.post_initialized,
        })
    }
}

/// Which instance map a bound script class belongs to.
///
/// Used by [`MissionScript::bind_actor`] / [`MissionScript::bind_target`]
/// / [`MissionScript::bind_scroll`] to reuse the same host-transfer and
/// Initialize-dispatch plumbing across all three entity flavours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptBindKind {
    Actor,
    Target,
    Scroll,
}

impl std::fmt::Debug for MissionScript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MissionScript")
            .field("class_count", &self.manager.class_count())
            .field("actor_instances", &self.actor_instances.len())
            .field("zone_instances", &self.zone_instances.len())
            .finish()
    }
}

fn nested_actor_function_default(fn_name: &str) -> i32 {
    match fn_name {
        "FilterAIEvent" => 1,
        _ => 0,
    }
}

impl MissionScript {
    /// Build a [`MissionScript`] from an already-parsed `.scb` payload.
    pub fn from_scb(scb: crate::scb::ScbFile) -> Result<Self, String> {
        Self::from_manager(String::new(), ScriptManager::new(scb))
    }

    /// Build a [`MissionScript`] from host-owned immutable bytecode.
    pub fn from_program(
        script_name: String,
        program: std::sync::Arc<crate::script_manager::ScriptProgram>,
    ) -> Result<Self, String> {
        Self::from_manager(script_name, ScriptManager::from_program(program))
    }

    fn from_manager(script_name: String, manager: ScriptManager) -> Result<Self, String> {
        let instance = manager
            .create_instance("StartUp")
            .map_err(|e| format!("No StartUp class in mission script: {e}"))?;

        Ok(Self {
            script_name,
            manager,
            state: ScriptState::default(),
            bindings: crate::natives::AttachedScriptBindings::default(),
            game_host: GameHost::new(),
            call_stack: ScriptCallStack::default(),
            instance,
            actor_instances: BTreeMap::new(),
            zone_instances: BTreeMap::new(),
            target_instances: BTreeMap::new(),
            scroll_instances: BTreeMap::new(),
            waypoint_instances: BTreeMap::new(),
            post_initialized: false,
        })
    }

    pub(crate) fn attach_program(
        &mut self,
        program: std::sync::Arc<crate::script_manager::ScriptProgram>,
    ) {
        self.manager.attach_program(program);
    }

    pub(crate) fn attach_bindings(&mut self, bindings: crate::natives::AttachedScriptBindings) {
        self.bindings = bindings;
    }

    fn current_call_frame(&self) -> crate::natives::ScriptCallFrame {
        self.call_stack.current()
    }

    /// Push one callback frame, run `f`, and restore the previous frame on
    /// success, error returns, and panic unwinds.
    pub(crate) fn with_call_frame<R>(
        &mut self,
        frame: crate::natives::ScriptCallFrame,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let previous_depth = self.call_stack.len();
        self.call_stack.push(frame);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self)));
        let popped = self.call_stack.pop();
        assert_eq!(popped, frame, "script call-frame stack order changed");
        assert_eq!(
            self.call_stack.len(),
            previous_depth,
            "script call-frame stack depth was not restored"
        );
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    #[cfg(test)]
    pub(crate) fn active_call_frame_count(&self) -> usize {
        self.call_stack.len()
    }

    pub(crate) fn assert_no_active_call_frames(&self) {
        assert!(
            self.call_stack.is_empty(),
            "mission script crossed a session boundary with active call frames"
        );
    }

    pub(crate) fn with_game_host_attached<R>(
        game_host: &mut GameHost,
        state: &mut ScriptState,
        script_domains: &mut super::ScriptDomains,
        bindings: &crate::natives::AttachedScriptBindings,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
        call_frame: crate::natives::ScriptCallFrame,
        inst: &mut ScriptInstance,
        f: impl FnOnce(&mut ScriptInstance, &mut NativeContext<'_, '_>) -> R,
    ) -> R {
        let mut context = NativeContext::with_call_frame(
            game_host,
            state,
            script_domains,
            bindings,
            capabilities,
            call_frame,
        );
        f(inst, &mut context)
    }

    /// Bind a script class to an entity handle, creating a persistent
    /// `ScriptInstance` and running its `Initialize()` function.
    ///
    /// The resulting instance is inserted into `actor_instances` keyed by
    /// `handle`, so later per-actor event dispatches
    /// ([`MissionScript::call_actor_function`]) find it.
    ///
    /// Returns `true` when the class was found and the instance was
    /// stored. Missing classes log at debug level and return `false`.
    pub(crate) fn bind_actor(
        &mut self,
        handle: i32,
        class_name: &str,
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> bool {
        self.bind_and_init(
            handle,
            class_name,
            ScriptBindKind::Actor,
            script_domains,
            capabilities,
        )
    }

    /// Target analogue of [`bind_actor`]. Stores the created instance in
    /// `target_instances`.
    pub(crate) fn bind_target(
        &mut self,
        handle: i32,
        class_name: &str,
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> bool {
        self.bind_and_init(
            handle,
            class_name,
            ScriptBindKind::Target,
            script_domains,
            capabilities,
        )
    }

    /// Scroll analogue of [`bind_actor`]. Stores the created instance in
    /// `scroll_instances` and runs `Initialize()` on the bound class.
    pub(crate) fn bind_scroll(
        &mut self,
        handle: i32,
        class_name: &str,
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> bool {
        self.bind_and_init(
            handle,
            class_name,
            ScriptBindKind::Scroll,
            script_domains,
            capabilities,
        )
    }

    /// Shared implementation for [`bind_actor`], [`bind_target`], and
    /// [`bind_scroll`]: look up the class, create an instance, transfer
    /// the game host in, call `Initialize()` if present, transfer the
    /// host back, and insert the instance into the appropriate map.
    fn bind_and_init(
        &mut self,
        handle: i32,
        class_name: &str,
        kind: ScriptBindKind,
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> bool {
        let class_idx = match self.manager.find_class(class_name) {
            Some(idx) => idx,
            None => {
                // The original engine treats a missing script class as
                // a fatal error on the shipping build; log at `error!`
                // so corrupt datadirs surface loudly while still
                // allowing the headless / replay harness to continue
                // past a missing script class.
                tracing::error!(
                    "{:?} script class '{class_name}' not found in SCB — skipping (handle {handle})",
                    kind
                );
                return false;
            }
        };
        let mut inst = self.manager.create_instance_idx(class_idx);

        let mut frame = self.current_call_frame().with_script_this(handle);
        if kind == ScriptBindKind::Scroll {
            frame = frame.with_current_scroll(handle);
        }
        self.with_call_frame(frame, |script| {
            Self::with_game_host_attached(
                &mut script.game_host,
                &mut script.state,
                script_domains,
                &script.bindings,
                capabilities,
                frame,
                &mut inst,
                |inst, host| {
                    if inst.has_function(&script.manager, "Initialize") {
                        match inst.call_function_limited_with_host(
                            &mut script.manager,
                            "Initialize",
                            10_000,
                            host,
                        ) {
                            Ok(ret) => {
                                tracing::debug!(
                                    "{:?} Init {class_name} (handle {handle}) → {ret}",
                                    kind
                                )
                            }
                            Err(crate::script_manager::ScriptError::Vm(
                                crate::interp::StopReason::StepLimit,
                            )) => tracing::debug!(
                                "{:?} Init {class_name} (handle {handle}) hit step limit",
                                kind
                            ),
                            Err(e) => {
                                tracing::warn!(
                                    "{:?} Init {class_name} (handle {handle}) failed: {e}",
                                    kind
                                )
                            }
                        }
                    }
                },
            );
        });

        match kind {
            ScriptBindKind::Actor => {
                self.actor_instances.insert(handle, inst);
            }
            ScriptBindKind::Target => {
                self.target_instances.insert(handle, inst);
            }
            ScriptBindKind::Scroll => {
                self.scroll_instances.insert(handle, inst);
            }
        }
        true
    }

    /// True if `handle` has a bound actor script that defines `fn_name`.
    ///
    /// Distinguishes the two Ok(0) returns from [`call_actor_function`]:
    /// "no instance or no override" (this returns false) vs "script ran
    /// and returned 0" (this returns true).  Used by `filter_stimulus`
    /// to avoid treating a missing `FilterAIEvent` override as a block
    /// — the base-class `FilterAIEvent` returns 1 (allow).
    pub fn actor_has_function(&self, handle: i32, fn_name: &str) -> bool {
        self.actor_instances
            .get(&handle)
            .map(|inst| inst.has_function(&self.manager, fn_name))
            .unwrap_or(false)
    }

    /// Call a named function on a per-actor script instance.
    ///
    /// Runs the actor instance with the shared `GameHost`, pushes a frame
    /// binding `ThisActor` to the actor, and restores the caller frame
    /// afterwards.
    ///
    /// If the running script invokes a native that requests a nested
    /// script call (currently only `PrototypeFilterEvent`), the VM
    /// yields with [`StopReason::PendingNestedCall`].  We dispatch the
    /// requested call recursively, patch
    /// the result into `vm.native_return_value`, and resume — so the
    /// outer caller sees a single transparent `Ok(i32)`.
    ///
    /// Returns `Ok(return_value)` or `Ok(0)` if the actor has no script
    /// instance or the function doesn't exist on the class.
    pub fn call_actor_function(
        &mut self,
        handle: i32,
        fn_name: &str,
        params: &[i32],
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> Result<i32, String> {
        self.call_actor_function_with_script_this(
            handle,
            fn_name,
            params,
            crate::interp::NestedCallScriptThis::TargetActor,
            script_domains,
            capabilities,
        )
    }

    fn call_actor_function_with_script_this(
        &mut self,
        handle: i32,
        fn_name: &str,
        params: &[i32],
        script_this: crate::interp::NestedCallScriptThis,
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> Result<i32, String> {
        let actor_inst = match self.actor_instances.get_mut(&handle) {
            Some(inst) => inst,
            None => return Ok(0),
        };

        if !actor_inst.has_function(&self.manager, fn_name) {
            return Ok(0);
        }

        // Ordinary actor calls bind to their target. PrototypeFilterEvent
        // explicitly inherits its caller frame, matching RHScript.cpp:6508-6535.
        let mut frame = self.current_call_frame();
        if script_this == crate::interp::NestedCallScriptThis::TargetActor {
            frame = frame.with_script_this(handle);
        }

        self.with_call_frame(frame, |script| {
            script.call_actor_function_in_current_frame(
                handle,
                fn_name,
                params,
                script_domains,
                capabilities,
            )
        })
    }

    fn call_actor_function_in_current_frame(
        &mut self,
        handle: i32,
        fn_name: &str,
        params: &[i32],
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> Result<i32, String> {
        let actor_inst = self
            .actor_instances
            .get_mut(&handle)
            .expect("validated actor instance vanished before dispatch");

        // Push parameters.
        for &p in params {
            actor_inst.push_param(p);
        }

        // Set up frames + IP without yet running.
        if let Err(e) = actor_inst.begin_call(&self.manager, fn_name) {
            return Err(format!(
                "Actor script {fn_name} (handle {handle}) failed: {e}"
            ));
        }

        // Drive the VM, dispatching any nested-script-call yields by
        // recursing into this same function.  The `resolve_nested_call`
        // helper takes ownership of the borrow on `actor_instances` so
        // we can call back into `&mut self`.
        self.run_actor_with_nested_resume(handle, fn_name, script_domains, capabilities)
    }

    /// Drive `actor_instances[handle]`'s VM until it returns or hits a
    /// non-recoverable stop.  On [`StopReason::PendingNestedCall`],
    /// dispatches the queued call (recursively through
    /// [`MissionScript::call_actor_function`]) and resumes. The surrounding
    /// call-frame scope owns receiver restoration.
    fn run_actor_with_nested_resume(
        &mut self,
        handle: i32,
        outer_fn_name: &str,
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> Result<i32, String> {
        loop {
            let stop = {
                let actor_inst = self
                    .actor_instances
                    .get_mut(&handle)
                    .expect("actor instance vanished mid-run");
                let mut context = NativeContext::with_call_frame(
                    &mut self.game_host,
                    &mut self.state,
                    script_domains,
                    &self.bindings,
                    capabilities,
                    self.call_stack.current(),
                );
                actor_inst.resume_run_with_host(
                    &mut self.manager,
                    10_000_000,
                    outer_fn_name,
                    &mut context,
                )
            };
            match stop {
                crate::interp::StopReason::ReturnedValue(v) => return Ok(v),
                crate::interp::StopReason::Returned => return Ok(0),
                crate::interp::StopReason::PendingNestedCall(call) => {
                    self.dispatch_nested_call_from_actor(
                        handle,
                        outer_fn_name,
                        call,
                        script_domains,
                        capabilities,
                    );
                    // Loop and resume the outer VM.
                }
                crate::interp::StopReason::StepLimit => {
                    tracing::debug!(
                        "Actor script {outer_fn_name} (handle {handle}) hit step limit"
                    );
                    return Ok(0);
                }
                other => {
                    return Err(format!(
                        "Actor script {outer_fn_name} (handle {handle}) failed: VM stopped abnormally: {other:?}"
                    ));
                }
            }
        }
    }

    /// Resolve a `PendingNestedCall` queued by the actor at
    /// `outer_handle`, recurses through `call_actor_function`, then
    /// writes the resolved result into `native_return_value` so the
    /// next opcode picks it up.
    fn dispatch_nested_call_from_actor(
        &mut self,
        outer_handle: i32,
        outer_fn_name: &str,
        pc: crate::interp::PendingNestedCall,
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) {
        // The outer callback occupies the first stack slot, so the current
        // length is exactly the prospective nested-call depth.
        let depth = u8::try_from(self.call_stack.len()).unwrap_or(u8::MAX);

        let result = if depth > crate::natives::MAX_NESTED_CALL_DEPTH {
            // Cycle / runaway recursion — return the base-class default
            // so the caller proceeds without blowing the stack.
            // `FilterAIEvent`'s base default is 1 (allow); other
            // future nested functions get 0 ("don't" / safe).
            tracing::warn!(
                target: "filter_ai_event_divergence",
                outer_handle,
                outer_fn = outer_fn_name,
                target_handle = pc.actor_handle,
                target_fn = %pc.fn_name,
                depth,
                "nested script call depth limit ({}) exceeded; returning base-class default",
                crate::natives::MAX_NESTED_CALL_DEPTH,
            );
            nested_actor_function_default(&pc.fn_name)
        } else if !self.actor_has_function(pc.actor_handle, &pc.fn_name) {
            // ActorScript's inherited FilterAIEvent body returns one. A
            // missing Rust instance/override represents that inherited body,
            // not a script-authored zero result.
            nested_actor_function_default(&pc.fn_name)
        } else {
            match self.call_actor_function_with_script_this(
                pc.actor_handle,
                &pc.fn_name,
                &pc.params,
                pc.script_this,
                script_domains,
                capabilities,
            ) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        target: "filter_ai_event_divergence",
                        outer_handle,
                        outer_fn = outer_fn_name,
                        target_handle = pc.actor_handle,
                        target_fn = %pc.fn_name,
                        error = %e,
                        "nested script call failed; returning base-class default (1 for FilterAIEvent, 0 otherwise)",
                    );
                    nested_actor_function_default(&pc.fn_name)
                }
            }
        };

        // Write the resolved return value into `native_return_value`
        // so the outer VM's next instruction (typically
        // `Aff1NativeGetReturn` for the original
        // `PrototypeFilterEvent` call site) reads the real result
        // instead of the placeholder `0`.
        let outer_inst = self
            .actor_instances
            .get_mut(&outer_handle)
            .expect("outer actor instance vanished during nested call");
        outer_inst.vm.native_return_value = result;
    }

    /// Call a named function on a per-zone script instance.
    ///
    /// Same host-transfer pattern as `call_actor_function`, but for zones.
    /// Returns `Ok(0)` if the zone has no script instance or the function
    /// doesn't exist.
    pub fn call_zone_function(
        &mut self,
        zone_idx: usize,
        fn_name: &str,
        params: &[i32],
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> Result<i32, String> {
        if !self
            .zone_instances
            .get(&zone_idx)
            .is_some_and(|inst| inst.has_function(&self.manager, fn_name))
        {
            return Ok(0);
        }
        let frame = self.current_call_frame();
        let result = self.with_call_frame(frame, |script| {
            let zone_inst = script
                .zone_instances
                .get_mut(&zone_idx)
                .expect("validated zone instance vanished before dispatch");
            Self::with_game_host_attached(
                &mut script.game_host,
                &mut script.state,
                script_domains,
                &script.bindings,
                capabilities,
                frame,
                zone_inst,
                |zone_inst, host| {
                    for &p in params {
                        zone_inst.push_param(p);
                    }
                    zone_inst.call_function_with_host(&mut script.manager, fn_name, host)
                },
            )
        });

        match result {
            Ok(v) => Ok(v),
            Err(crate::script_manager::ScriptError::Vm(crate::interp::StopReason::StepLimit)) => {
                tracing::debug!("Zone script {fn_name} (zone {zone_idx}) hit step limit");
                Ok(0)
            }
            Err(e) => Err(format!(
                "Zone script {fn_name} (zone {zone_idx}) failed: {e}"
            )),
        }
    }

    /// Call a named function on a per-target script instance.
    ///
    /// Same host-transfer pattern as `call_actor_function`, applied to
    /// FX targets with a non-empty `script_class`. Each target has its
    /// own VM, with `ActivatedBy*(pc)` style named dispatches.
    /// Returns `Ok(0)` when the target has no script instance or the
    /// function doesn't exist on its class.
    pub fn call_target_function(
        &mut self,
        target_handle: i32,
        fn_name: &str,
        params: &[i32],
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> Result<i32, String> {
        if !self
            .target_instances
            .get(&target_handle)
            .is_some_and(|inst| inst.has_function(&self.manager, fn_name))
        {
            return Ok(0);
        }
        let frame = self.current_call_frame().with_script_this(target_handle);
        let result = self.with_call_frame(frame, |script| {
            let target_inst = script
                .target_instances
                .get_mut(&target_handle)
                .expect("validated target instance vanished before dispatch");
            Self::with_game_host_attached(
                &mut script.game_host,
                &mut script.state,
                script_domains,
                &script.bindings,
                capabilities,
                frame,
                target_inst,
                |target_inst, host| {
                    for &p in params {
                        target_inst.push_param(p);
                    }
                    target_inst.call_function_with_host(&mut script.manager, fn_name, host)
                },
            )
        });
        match result {
            Ok(v) => Ok(v),
            Err(crate::script_manager::ScriptError::Vm(crate::interp::StopReason::StepLimit)) => {
                tracing::debug!("Target script {fn_name} (handle {target_handle}) hit step limit");
                Ok(0)
            }
            Err(e) => Err(format!(
                "Target script {fn_name} (handle {target_handle}) failed: {e}"
            )),
        }
    }

    /// Call a named function on a per-scroll script instance.
    ///
    /// Pushes an executing-scroll frame around every scroll-script dispatch.
    /// `ThisActor` and `ThisScroll` read that frame for every VM resume, and
    /// the previous frame is restored structurally afterwards.
    ///
    /// Returns `Ok(0)` when the scroll has no bound script or the
    /// function doesn't exist on its class.
    pub fn call_scroll_function(
        &mut self,
        scroll_handle: i32,
        fn_name: &str,
        params: &[i32],
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> Result<i32, String> {
        if !self
            .scroll_instances
            .get(&scroll_handle)
            .is_some_and(|inst| inst.has_function(&self.manager, fn_name))
        {
            return Ok(0);
        }
        let frame = self
            .current_call_frame()
            .with_script_this(scroll_handle)
            .with_current_scroll(scroll_handle);
        let result = self.with_call_frame(frame, |script| {
            let scroll_inst = script
                .scroll_instances
                .get_mut(&scroll_handle)
                .expect("validated scroll instance vanished before dispatch");
            Self::with_game_host_attached(
                &mut script.game_host,
                &mut script.state,
                script_domains,
                &script.bindings,
                capabilities,
                frame,
                scroll_inst,
                |scroll_inst, host| {
                    for &p in params {
                        scroll_inst.push_param(p);
                    }
                    scroll_inst.call_function_with_host(&mut script.manager, fn_name, host)
                },
            )
        });
        match result {
            Ok(v) => Ok(v),
            Err(crate::script_manager::ScriptError::Vm(crate::interp::StopReason::StepLimit)) => {
                tracing::debug!("Scroll script {fn_name} (handle {scroll_handle}) hit step limit");
                Ok(0)
            }
            Err(e) => Err(format!(
                "Scroll script {fn_name} (handle {scroll_handle}) failed: {e}"
            )),
        }
    }

    /// Bind a waypoint-script class to a given `(path_idx, wp_idx)`
    /// pair, creating a persistent `ScriptInstance` and running
    /// `Initialize()`. Each waypoint binds its class during path-load.
    ///
    /// Returns `true` when the class was found and the instance stored;
    /// missing classes log a warning and return `false`.
    pub(crate) fn bind_waypoint(
        &mut self,
        path_idx: crate::ai::PathId,
        wp_idx: u8,
        class_name: &str,
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> bool {
        let class_idx = match self.manager.find_class(class_name) {
            Some(idx) => idx,
            None => {
                tracing::warn!(
                    "Waypoint script class '{class_name}' \
                     (path {path_idx}, wp {wp_idx}) not found in SCB"
                );
                return false;
            }
        };
        let mut inst = self.manager.create_instance_idx(class_idx);

        // Waypoints aren't entities so no `script_this` is installed;
        // Initialize doesn't push an actor either (only ReachPoint does).
        let frame = self.current_call_frame();
        self.with_call_frame(frame, |script| {
            Self::with_game_host_attached(
                &mut script.game_host,
                &mut script.state,
                script_domains,
                &script.bindings,
                capabilities,
                frame,
                &mut inst,
                |inst, host| {
                if inst.has_function(&script.manager, "Initialize") {
                    match inst.call_function_limited_with_host(
                        &mut script.manager,
                        "Initialize",
                        10_000,
                        host,
                    ) {
                        Ok(ret) => tracing::debug!(
                            "Waypoint Init {class_name} (path {path_idx}, wp {wp_idx}) → {ret}"
                        ),
                        Err(crate::script_manager::ScriptError::Vm(
                            crate::interp::StopReason::StepLimit,
                        )) => tracing::debug!(
                            "Waypoint Init {class_name} (path {path_idx}, wp {wp_idx}) hit step limit"
                        ),
                        Err(e) => {
                            tracing::warn!(
                                "Waypoint Init {class_name} (path {path_idx}, wp {wp_idx}) failed: {e}"
                            );
                            // A bound waypoint class failing its own
                            // `Initialize` is a logic bug.
                            debug_assert!(
                                false,
                                "Waypoint Init {class_name} (path {path_idx}, wp {wp_idx}) failed: {e}"
                            );
                        }
                    }
                }
                },
            );
        });
        self.waypoint_instances.insert((path_idx, wp_idx), inst);
        true
    }

    /// Call a named function on a per-waypoint script instance.
    ///
    /// Same host-transfer pattern as [`call_actor_function`], keyed by
    /// `(path_idx, wp_idx)`. Waypoints have no entity handle so
    /// `script_this` is left at 0 — `ReachPoint(actor)` dispatches
    /// without a script-this assignment, and the actor is pushed as a
    /// parameter instead.
    ///
    /// Returns `Ok(0)` when no instance is bound for this waypoint or
    /// the function is absent.
    pub fn call_waypoint_function(
        &mut self,
        path_idx: crate::ai::PathId,
        wp_idx: u8,
        fn_name: &str,
        params: &[i32],
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> Result<i32, String> {
        let key = (path_idx, wp_idx);
        if !self
            .waypoint_instances
            .get(&key)
            .is_some_and(|inst| inst.has_function(&self.manager, fn_name))
        {
            return Ok(0);
        }
        let frame = self.current_call_frame();
        let result = self.with_call_frame(frame, |script| {
            let wp_inst = script
                .waypoint_instances
                .get_mut(&key)
                .expect("validated waypoint instance vanished before dispatch");
            Self::with_game_host_attached(
                &mut script.game_host,
                &mut script.state,
                script_domains,
                &script.bindings,
                capabilities,
                frame,
                wp_inst,
                |wp_inst, host| {
                    for &p in params {
                        wp_inst.push_param(p);
                    }
                    wp_inst.call_function_with_host(&mut script.manager, fn_name, host)
                },
            )
        });
        match result {
            Ok(v) => Ok(v),
            Err(crate::script_manager::ScriptError::Vm(crate::interp::StopReason::StepLimit)) => {
                tracing::debug!(
                    "Waypoint script {fn_name} (path {path_idx}, wp {wp_idx}) hit step limit"
                );
                Ok(0)
            }
            Err(e) => Err(format!(
                "Waypoint script {fn_name} (path {path_idx}, wp {wp_idx}) failed: {e}"
            )),
        }
    }

    /// Call the script's `Hourglass` function (once per game-second).
    pub(crate) fn hourglass(
        &mut self,
        game_seconds: u32,
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> Result<i32, String> {
        let frame = crate::natives::ScriptCallFrame::default();
        self.with_call_frame(frame, |script| {
            Self::with_game_host_attached(
                &mut script.game_host,
                &mut script.state,
                script_domains,
                &script.bindings,
                capabilities,
                frame,
                &mut script.instance,
                |instance, host| {
                    instance.push_param(game_seconds as i32);
                    instance
                        .call_function_with_host(&mut script.manager, "Hourglass", host)
                        .map_err(|e| format!("Script Hourglass failed: {e}"))
                },
            )
        })
    }

    /// Call the script's `CheckVictoryCondition` function.
    ///
    /// Returns: 0 = in progress, 1 = victory, 2 = defeat.
    pub(crate) fn check_victory_condition(
        &mut self,
        game_seconds: u32,
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> Result<i32, String> {
        let frame = crate::natives::ScriptCallFrame::default();
        self.with_call_frame(frame, |script| {
            Self::with_game_host_attached(
                &mut script.game_host,
                &mut script.state,
                script_domains,
                &script.bindings,
                capabilities,
                frame,
                &mut script.instance,
                |instance, host| {
                    instance.push_param(game_seconds as i32);
                    instance
                        .call_function_with_host(&mut script.manager, "CheckVictoryCondition", host)
                        .map_err(|e| format!("Script CheckVictoryCondition failed: {e}"))
                },
            )
        })
    }

    /// Call the script's `Finalize` function.
    ///
    /// `abandoned` is true if the player quit, false if won/lost normally.
    pub(crate) fn finalize(
        &mut self,
        abandoned: bool,
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> Result<(), String> {
        let frame = crate::natives::ScriptCallFrame::default();
        self.with_call_frame(frame, |script| {
            Self::with_game_host_attached(
                &mut script.game_host,
                &mut script.state,
                script_domains,
                &script.bindings,
                capabilities,
                frame,
                &mut script.instance,
                |instance, host| {
                    instance.push_param(if abandoned { 1 } else { 0 });
                    instance
                        .call_function_with_host(&mut script.manager, "Finalize", host)
                        .map(|_| ())
                        .map_err(|e| format!("Script Finalize failed: {e}"))
                },
            )
        })
    }

    /// Call the script's `PostInitialize` function, if it exists.
    pub(crate) fn post_initialize(
        &mut self,
        script_domains: &mut super::ScriptDomains,
        capabilities: &crate::natives::NativeSessionCapabilities<'_>,
    ) -> Result<(), String> {
        if self.instance.has_function(&self.manager, "PostInitialize") {
            let frame = crate::natives::ScriptCallFrame::default();
            self.with_call_frame(frame, |script| {
                Self::with_game_host_attached(
                    &mut script.game_host,
                    &mut script.state,
                    script_domains,
                    &script.bindings,
                    capabilities,
                    frame,
                    &mut script.instance,
                    |instance, host| {
                        instance
                            .call_function_with_host(&mut script.manager, "PostInitialize", host)
                            .map(|_| ())
                            .map_err(|e| format!("Script PostInitialize failed: {e}"))
                    },
                )
            })?;
        }
        Ok(())
    }

    /// Get a mutable reference to the underlying [`GameHost`].
    ///
    /// Exposed to the host crate so custom-mission Lua scripts can
    /// reach engine state through `MissionLuaState::with_host`
    /// (see `robin_rs::lua_session`). Modifying the host through
    /// this handle outside of a script event is fine for queued commands (the
    /// engine drains them next tick). Canonical entities, AI, and grid state
    /// are deliberately unavailable through this adapter.
    pub fn game_host_mut(&mut self) -> Option<&mut GameHost> {
        Some(&mut self.game_host)
    }

    /// Get an immutable reference to the underlying [`GameHost`].
    pub fn game_host(&self) -> Option<&GameHost> {
        Some(&self.game_host)
    }
}

// ─── Mission state ───────────────────────────────────────────────────

/// Tracks win/lose/interrupted conditions for the current mission.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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
}

// ─── Input state (transient per-frame) ───────────────────────────────

/// Per-frame mouse/input state tracked by the engine.
/// All fields are reset on serialization or transient.
#[derive(Debug, Clone, Default)]
pub struct InputState {
    /// The engine currently has OS focus.
    pub has_focus: bool,

    // Multi-selection rubber-band
    pub multi_selection_active: bool,
    pub multi_unselection_active: bool,
    pub draw_multi_selection: bool,
    pub multi_selection_pt1: MapPoint,
    pub multi_selection_pt2: MapPoint,

    /// Left mouse button is currently held down.
    pub left_mouse_down: bool,
    /// Screen position where left mouse was pressed (for drag detection).
    pub left_mouse_start_screen: ScreenPoint,

    /// Whether the player is currently dragging with the left mouse.
    ///
    /// Distinct from `left_mouse_down`: this is set on left-mouse-down
    /// and cleared on left-mouse-up — same as `left_mouse_down` in the
    /// normal flow — but it is *also* cleared when the portrait re-arms
    /// an action on a left-double-click. Used as the gate for extending
    /// the swordfight mouse-way polyline.
    pub is_dragging: bool,

    /// Right mouse button is currently held down.
    pub right_mouse_down: bool,

    /// Alt modifier is currently held.  Persisted on `InputState`
    /// rather than read ad-hoc from the physical key state each
    /// frame.  Updated from the key-state snapshot at the top of the
    /// event loop; consumed by mouse-way append gating, view-cone
    /// overlay, and any other subsystem that doesn't otherwise have
    /// the platform modifier state.
    pub is_alt: bool,

    /// Set when left MouseDown has clicks >= 2 (platform double-click).
    /// Consumed on the corresponding MouseUp to dispatch a double-click.
    pub left_double_click_pending: bool,

    // Mouse event suppression
    pub ignore_next_drag: bool,
    pub ignore_next_left_click: bool,
    pub next_left_double_is_simple: bool,

    // Currently hovered map point/layer/sector.
    pub selected_map_point: MapPoint,
    pub selected_layer: u16,
    /// Index into `FastFindGrid::sectors` for the sector under the mouse.
    /// Set each frame in `update_mouse`. Used for door/jump alpha overlays.
    pub selected_sector_idx: Option<crate::fast_find_grid::SectorIndex>,
    /// Index into `GameHost::patches` for the patch whose overlay sector
    /// the mouse is hovering, if any.  Persisted on InputState so the
    /// cursor / render hooks don't re-scan `self.script_domains.interactables.patches` each
    /// frame.
    pub selected_patch_idx: Option<u32>,
    /// Index into `GameHost::doors` for a door whose click polygon is
    /// under the mouse. Some building doors are wider than their grid
    /// door sector, so hover/click handling must not depend only on
    /// `selected_sector_idx`.
    pub hovered_door_idx: Option<u32>,
    /// True when the hovered sector is a motion-area / door / jump
    /// sector or a patch overlay sits over the mouse — i.e. a move
    /// command dispatched here would have somewhere to land. Updated
    /// alongside `selected_sector_idx` each frame.
    pub valid_position_for_move: bool,

    /// Entity currently under the mouse cursor.  Reset each frame.
    pub focused_entity_id: Option<crate::element::EntityId>,

    /// Last element successfully clicked.  Written by `left_click_no_action`
    /// / `perform_swordfight` whenever a click resolves against a
    /// focusable target; read by `left_double_click_no_action` to
    /// re-dispatch the previous click's target on a double-click, and
    /// by `is_focusable_click_and_drag` to cache drag targets for bow
    /// shots and click-and-drag actions.
    ///
    /// Zeroed at load/save resets.
    pub element_old_click: Option<crate::element::EntityId>,

    /// Current drag target for click-and-drag actions (bow, apple,
    /// stone, strangle, heal, hit, lever).  Written by
    /// `is_focusable_click_and_drag` when the drag target changes;
    /// cleared when the drag ends or the cursor leaves every
    /// focusable.  Used to keep cursor previews and action handlers
    /// stable across individual mouse-move samples during a drag.
    pub target_drag: Option<crate::element::EntityId>,

    /// Entity whose double status bar should be shown this frame.
    pub double_status_bar_entity_id: Option<crate::element::EntityId>,

    /// PCs that should render at full-alpha outline this frame in
    /// response to a requirements-bar action hover. Iterates the PC
    /// list and marks each PC whose profile has the action. Populated
    /// by the host-side requirements-bar hit test before the outline
    /// pass; cleared at the start of each frame.
    pub marked_pc_ids: Vec<crate::element::EntityId>,

    /// Mouse cursor shadow intensity (0 = fully transparent, 50 = normal).
    /// Default = 40.  Set by bow/projectile branches.
    pub mouse_opacity: u16,

    /// Mouse cursor shadow color (16-bit packed, 0 = no shadow tint).
    /// Set by bow branch for no-target / civilian / VIP coloring.
    pub mouse_shadow_color: u16,

    /// Whether to advance cursor animation this frame.
    /// Set false for door cursors and some other cases where cursor
    /// animation should freeze.
    /// Default is `true` (via Default impl reset each frame).
    pub increment_cursor_animation: bool,

    /// Whether to render door hover UI this frame.
    /// Set true at start of `choose_mouse_pointer_for_no_action`,
    /// cleared when an entity is focused.
    pub display_door: bool,

    /// Set after a right-click cancels an action on the portrait.
    /// When armed, the next action-button click drops ammo instead of
    /// arming the action. Cleared when any action is successfully armed.
    pub portrait_drop_ammo_armed: bool,

    /// Portrait action countdown.
    /// Starts at 5 when an action is dispatched via portrait; decrements
    /// each frame. If a double-click lands within the window, the action
    /// is accelerated via MakeFast.
    pub portrait_action_countdown: u16,
    /// The PC whose action was just dispatched (for MakeFast targeting).
    pub portrait_action_pc: Option<crate::element::EntityId>,

    /// Debug "draw hidden" toggle, flipped by the masked-display
    /// switch message. When on, titbits attached to entities the
    /// player can't currently see (inside buildings, blipped) are
    /// still rendered so the debug view can inspect AI state.
    pub draw_hidden: bool,
}

impl InputState {
    /// Start a drag-box multi-selection at the given map-space point.
    pub fn start_multi_selection(&mut self, map_pt: MapPoint) {
        self.multi_selection_active = true;
        self.draw_multi_selection = false;
        self.multi_selection_pt1 = map_pt;
        self.multi_selection_pt2 = map_pt;
    }

    /// Update the drag-box endpoint during a multi-selection drag.
    pub fn update_multi_selection(&mut self, map_pt: MapPoint) {
        self.multi_selection_pt2 = map_pt;
    }

    /// Cancel an in-progress multi-selection.
    pub fn cancel_multi_selection(&mut self) {
        self.multi_selection_active = false;
        self.draw_multi_selection = false;
    }

    /// Start a drag-box multi-UNselection at the given map-space point.
    pub fn start_multi_unselection(&mut self, map_pt: MapPoint) {
        self.multi_unselection_active = true;
        self.draw_multi_selection = false;
        self.multi_selection_pt1 = map_pt;
        self.multi_selection_pt2 = map_pt;
    }

    /// Cancel an in-progress multi-unselection.
    pub fn cancel_multi_unselection(&mut self) {
        self.multi_unselection_active = false;
        self.draw_multi_selection = false;
    }

    /// Sets the three suppression flags the host reads at the next
    /// mouse event:
    ///
    /// - `click` → suppresses the next LMB-up.
    /// - `drag` → suppresses the next LMB drag motion.
    /// - `next_left_double_is_simple` → demotes the next platform double-
    ///   click to a single click; the event loop consumes this at
    ///   MouseDown to clear `left_double_click_pending`.
    ///
    /// The double-click demotion is done directly against
    /// `left_double_click_pending` in `handle_mouse_input`.
    pub fn ignore_mouse_event(
        &mut self,
        click: bool,
        drag: bool,
        next_left_double_is_simple: bool,
    ) {
        if click {
            self.ignore_next_left_click = true;
        }
        if drag {
            self.ignore_next_drag = true;
        }
        self.next_left_double_is_simple = next_left_double_is_simple;
    }

    /// Clears the matching suppression flags.  Used by
    /// `perform_mouse_left_click` after it consumes the ignore-click,
    /// and by `perform_mouse_right_click` at the end of its body to
    /// drop any pending ignore state.
    pub fn accept_mouse_event(&mut self, click: bool, drag: bool) {
        if click {
            self.ignore_next_left_click = false;
        }
        if drag {
            self.ignore_next_drag = false;
        }
    }
}

// ─── Weather ─────────────────────────────────────────────────────────

/// Weather and environmental state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ShieldState {
    pub is_protected: bool,
    /// The PC whose defensive arc is being honoured. `None` means no
    /// PC is protecting.
    pub protected_pc: Option<crate::element::EntityId>,
}

// ─── Element index ───────────────────────────────────────────────────

/// Opaque handle into the element arrays.
/// Will be replaced with proper entity handles when the element system is ported.
pub type ElementIndex = u32;

// ─── The EngineInner ──────────────────────────────────────────────────────

/// An anonymous countdown timer tracked by the engine.
///
/// One entry per sequence element with a timer countdown property that
/// decrements each frame.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct TimerEntry {
    /// Frames remaining. Decremented each frame; entry removed when it hits 0.
    pub remaining: u32,
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
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
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
    /// Update the sound-system's listen-point (camera tracking).
    SetListenPoint {
        position: crate::coordinates::MapPoint,
        zoom: f32,
    },
}

// ─── Side effects ────────────────────────────────────────────────────

/// Changes the PC-info hover overlay applied post-tick by the host.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum OverlayChange {
    Show { pc_id: crate::element::EntityId },
    Hide,
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
    Debug, Default, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct SideEffects {
    /// The game-state code returned by the tick (in-progress / succeeded / failed / interrupted).
    pub code: crate::game_operation::GameCode,
    /// Exclamations and other sim-originated sound triggers.
    pub sounds: Vec<SoundCommand>,
    /// Broadcast noises emitted this tick for the developer noise
    /// overlay. Host/game-session code drains these into `DevState`.
    pub displayed_noises: Vec<crate::ai::Noise>,
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
    /// Set when the `DisplayConsole` script native (or cheat key) fired
    /// this tick.
    pub pending_show_console: bool,
    /// Entities the sim asked to render a one-frame full-alpha outline
    /// on this tick.  Currently only populated by the
    /// `AddPCToMissionTeam` native, marking the PC after it is added.
    /// Host merges into [`InputState::marked_pc_ids`] each frame.
    pub pending_mark_pc_ids: Vec<crate::element::EntityId>,
    /// Deferred patch-effect background decal inserts (`BlitToMap`) and
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
    /// Sim observed `SimpleMessage::UiHasFocus` on the messenger this
    /// tick. A latch set by the UI-has-focus message and cleared every
    /// frame as part of the messenger's per-frame sweep. Host drains
    /// this by setting `host.ui_focus = true`; the host clears the
    /// latch back to false at end of `update_mouse` each frame.
    pub ui_has_focus: bool,
    /// New top-left of the deployed minimap when an accepted drag /
    /// resize / setup-time validation moved it this tick. The host
    /// drains this by writing the top-left into the active
    /// `PlayerProfile`'s `minimap_x` / `minimap_y` and persisting the
    /// profile.
    pub pending_minimap_position: Option<crate::coordinates::ScreenPoint>,
    /// Script/sequence-driven minimap show/hide requests produced this
    /// tick. The minimap itself is host-owned, so the game loop applies
    /// these to `HostDisplayState`.
    pub pending_minimap_display_maps: Vec<(bool, bool)>,
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

    #[error(transparent)]
    MissionLevelBuild(#[from] MissionLevelBuildError),
}

/// Validation failures raised by the staged mission-level builder.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
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
        "{lift_type} lift sector {sector_number} is missing a {endpoint} authored door endpoint"
    )]
    MissingLiftEndpoint {
        lift_type: String,
        sector_number: i16,
        endpoint: String,
    },
}
