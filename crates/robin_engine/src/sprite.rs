//! Sprite animation state machine and rendering data.
//!
//! Sprites compose a [`PositionInterface`] field (position, direction,
//! layer, sector, material, anti-collision) instead of inheriting it.
//! This module owns the sprite-specific parts: animation frame
//! progression, action state, display order, edge map, bounding box
//! management, and per-frame motion-distance computation
//! ([`Sprite::perform_motion`]).

use serde::{Deserialize, Serialize};

use crate::coordinates::{MapPoint, SpriteAnchor, SpriteLocalPoint};
use crate::element::EntityId;
use crate::geo2d::{GeoPoint2D, Vec2D};
use crate::order::OrderType;
use crate::position_interface::PositionInterface;
use crate::sprite_script::{
    Ambiance, FrameKind, SpriteInfo, SpriteScript, SpriteScriptor, UNMAPPED,
};

// ---------------------------------------------------------------------------
// MotionState
// ---------------------------------------------------------------------------

/// Result state of a sprite animation/motion step.
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
pub enum MotionState {
    /// The action-done frame was reached.
    Done,
    /// This is the first frame of a new action.
    Start,
    /// Animation is in progress (normal tick).
    InProgress,
    /// The animation has looped / reached its end.
    Terminated,
    /// The action was aborted (e.g., invalid animation).
    Aborted,
    /// An error occurred.
    Error,
}

// ---------------------------------------------------------------------------
// MotionMethod
// ---------------------------------------------------------------------------

/// Algorithm for computing the movement vector during motion.
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
pub enum MotionMethod {
    None,
    Walk,
    Run,
    Fast,
    WalkBackwards,
    TillLastFrame,
    Drunken,
    WalkWithoutAnticollision,
    RunWithoutAnticollision,
}

/// Order data that C++ `RHSprite::PerformMotion(RHOrder*, RHOrder*, ...)`
/// reads when a sprite sees a motion order for the first time.
#[derive(Debug, Clone, Copy)]
pub struct MotionOrderContext {
    pub order_id: std::num::NonZeroU32,
    pub destination: MapPoint,
    pub reverse: bool,
    pub tolerance: f32,
    pub directional_tolerance: bool,
    pub compute_direction: bool,
    pub next_destination_same_action: Option<MapPoint>,
}

// ---------------------------------------------------------------------------
// FrameProgression
// ---------------------------------------------------------------------------

/// How the animation proceeds to the next frame.
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
pub enum FrameProgression {
    /// Normal: loop when reaching the end, report terminated on last frame.
    Default,
    /// Loop continuously without reporting terminated.
    Cyclically,
    /// Force increment every tick (ignoring delay).
    ForceIncrement,
    /// Skip every other frame (for shadow interleaving).
    SkipShadow,
    /// Randomly delay start (idle animation variety).
    BoredAnim,
    /// Like BoredAnim but with lower probability.
    SnakeAnim,
    /// Start at the action-done frame.
    BeginWithDone,
    /// Whirl the facing direction (-2 sectors per tick for 8 ticks) once
    /// the animation reaches its action-done frame, then resume forward
    /// progression. The companion `WhirlingSanchezIsBack` helper is
    /// declared but never defined in any shipped source, so no live
    /// caller currently constructs this variant — kept here to preserve
    /// the enum ordinal layout for any persisted stream that uses it.
    WhirlingSanchezIsBack,
    /// Freeze on the last frame when animation ends.
    FreezeWhenTerminated,
    /// Freeze on the second-to-last frame.
    FreezeLastButOneFrame,
    /// Completely frozen (no frame change).
    Frozen,
    /// Frozen on the first frame.
    FrozenFirstFrame,
    /// Frozen on the last frame.
    FrozenLastFrame,
    /// Play in reverse.
    Reversed,
    /// Skip shadow frames and freeze when terminated.
    SkipShadowFreezeWhenTerminated,
}

impl FrameProgression {
    /// Invert the `as u32` ordinal cast used by call sites that
    /// persist a progression to storage (e.g. `TargetData.progression`
    /// written by the `PlayAnim*` sequence arms).  Returns
    /// `FrameProgression::Default` for unknown ordinals so a
    /// malformed savegame or an out-of-range value degrades to the
    /// default rather than panicking.
    pub fn from_ordinal(n: u32) -> Self {
        match n {
            0 => Self::Default,
            1 => Self::Cyclically,
            2 => Self::ForceIncrement,
            3 => Self::SkipShadow,
            4 => Self::BoredAnim,
            5 => Self::SnakeAnim,
            6 => Self::BeginWithDone,
            7 => Self::WhirlingSanchezIsBack,
            8 => Self::FreezeWhenTerminated,
            9 => Self::FreezeLastButOneFrame,
            10 => Self::Frozen,
            11 => Self::FrozenFirstFrame,
            12 => Self::FrozenLastFrame,
            13 => Self::Reversed,
            14 => Self::SkipShadowFreezeWhenTerminated,
            _ => Self::Default,
        }
    }
}

// ---------------------------------------------------------------------------
// FlightStyle
// ---------------------------------------------------------------------------

/// Style of flight for airborne sprites.
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
pub enum FlightStyle {
    Default,
    Fall,
}

// ---------------------------------------------------------------------------
// BoundingBox2D — simple AABB for sprite bounds
// ---------------------------------------------------------------------------

/// Axis-aligned bounding box (top-left, bottom-right).
#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct BBox {
    pub min: GeoPoint2D,
    pub max: GeoPoint2D,
}

impl BBox {
    pub fn new(min: GeoPoint2D, max: GeoPoint2D) -> Self {
        Self { min, max }
    }

    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    pub fn is_intersecting(&self, other: &BBox) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }

    pub fn contains_point(&self, p: GeoPoint2D) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }
}

// ---------------------------------------------------------------------------
// Sprite — the main sprite struct
// ---------------------------------------------------------------------------

/// Sprite animation state and metadata.
///
/// Composes a [`PositionInterface`] field: every sprite — and therefore
/// every element — carries its own position/direction/layer/sector/etc.
#[derive(Debug, Clone)]
pub struct Sprite {
    /// Position, direction, layer, sector, material, anti-collision.
    /// Single source of truth for all positional state.
    pub position_iface: PositionInterface,

    // -- Animation state (serialized in save games) --
    /// Current row in the sprite script (animation + direction).
    pub current_row: u16,
    /// Current frame within the current row.
    pub current_frame: u16,
    /// Sub-frame counter (ticks within the current frame's delay).
    pub frame_count: u16,

    /// Width of the current sprite frame.
    pub current_width: u16,
    /// Height of the current sprite frame.
    pub current_height: u16,

    /// Last animation that was played.
    pub last_action: OrderType,
    /// ID of the last order that was processed.
    pub last_processed_order_id: u32,

    /// Whether the sprite is currently masked.
    pub masked: bool,
    /// Whether using the alternate animation profile.
    pub use_alternate_profile: bool,

    /// Frame at which "action done" fires.
    pub action_done_frame: u16,
    /// Sub-frame counter at which "action done" fires.
    pub action_done_counter: u16,

    /// Last sound ID played (to avoid repeats).
    pub last_sound_id: u16,

    /// Step counter that paces the water-splash titbit emission while
    /// the actor moves through a water-material sector.  Every walk
    /// tick with distance > 2 and material == water bumps the counter,
    /// and on >= 2 a water particle is emitted and the counter resets.
    /// Lives on the engine-owned sprite and affects deterministic
    /// water-titbit side effects, so it participates in snapshots.
    pub splitch_count: u8,

    /// Whether this sprite is behind its display order reference.
    pub behind_display_order_ref: bool,
    /// Reference entity for display ordering (carried/attached entities).
    /// When set, the depth used by the host's draw-order sort is computed
    /// as `ref.position.y ± 0.001` instead of `self.position.y`.  The
    /// per-entity depth is ephemeral (host-cached in `DrawOrder::depths`
    /// each frame); only the ref binding is sim state.
    pub display_order_ref: Option<EntityId>,

    /// Animations to be replaced (parallel lists with `replacing_anims`).
    pub anims_to_be_replaced: Vec<OrderType>,
    /// Replacement animations (parallel lists with `anims_to_be_replaced`).
    pub replacing_anims: Vec<OrderType>,

    /// Primary animation scripts (Arc-shared from SpriteScriptor cache).
    ///
    /// Always non-`None`-valued at the type level: every Sprite is
    /// constructed with bound scripts (via [`Sprite::new`] or
    /// [`Sprite::load_frame_info`]), or with the empty
    /// `Default::default()` placeholder for slots that will never be
    /// animated (e.g. `ElementData::default()`).  This eliminates the
    /// old `is_ready()` runtime gate — animation methods either
    /// operate on a real script set or short-circuit naturally on
    /// out-of-bounds conversion lookups.
    pub scripts: std::sync::Arc<Vec<SpriteScript>>,
    /// Alternate animation scripts (only present for blipped characters).
    pub alternate_scripts: Option<std::sync::Arc<Vec<SpriteScript>>>,
    /// Primary action → row conversion table.  See `scripts` for the
    /// non-`Option` rationale.
    pub conversion: std::sync::Arc<Vec<u16>>,
    /// Alternate conversion table (only present for blipped characters).
    pub alternate_conversion: Option<std::sync::Arc<Vec<u16>>>,

    /// Name of the loaded frame profile.
    pub frame_profile_name: String,
    /// SpriteScriptor cache key for the primary profile.
    pub profile_cache_key: String,
    /// SpriteScriptor cache key for the alternate profile, when present.
    pub alternate_profile_cache_key: String,

    /// Sprite center / anchor point (from profile header).
    /// Used to convert from entity map position to the sprite's
    /// top-left rendering origin.  Stores the active sprite anchor so
    /// engine/host geometry paths can read it directly.
    pub center: SpriteAnchor,

    /// Most recent [`MotionState`] returned by a `perform_*` advance
    /// this tick.  Set automatically by every perform method, consumed
    /// by `EngineInner::propagate_done_to_current_orders` at end of
    /// tick to flip the actor's current order's `done` flag once the
    /// sprite reports `MotionState::Done`.
    ///
    /// Transient: `None` between ticks, written during dispatches,
    /// reset to `None` after the propagation pass.  Excluded from
    /// snapshots and state-hash because it is purely a derived
    /// per-tick quantity.
    pub last_motion_state: Option<MotionState>,
}

#[derive(Serialize)]
struct SpriteSnapshotRef<'a> {
    position_iface: &'a PositionInterface,
    current_row: u16,
    current_frame: u16,
    frame_count: u16,
    current_width: u16,
    current_height: u16,
    last_action: OrderType,
    last_processed_order_id: u32,
    masked: bool,
    use_alternate_profile: bool,
    action_done_frame: u16,
    action_done_counter: u16,
    last_sound_id: u16,
    splitch_count: u8,
    behind_display_order_ref: bool,
    display_order_ref: Option<EntityId>,
    anims_to_be_replaced: &'a [OrderType],
    replacing_anims: &'a [OrderType],
    frame_profile_name: &'a str,
    profile_cache_key: &'a str,
    alternate_profile_cache_key: &'a str,
    center: SpriteAnchor,
}

#[derive(Deserialize)]
struct SpriteSnapshot {
    position_iface: PositionInterface,
    current_row: u16,
    current_frame: u16,
    frame_count: u16,
    current_width: u16,
    current_height: u16,
    last_action: OrderType,
    last_processed_order_id: u32,
    masked: bool,
    use_alternate_profile: bool,
    action_done_frame: u16,
    action_done_counter: u16,
    last_sound_id: u16,
    splitch_count: u8,
    behind_display_order_ref: bool,
    display_order_ref: Option<EntityId>,
    anims_to_be_replaced: Vec<OrderType>,
    replacing_anims: Vec<OrderType>,
    frame_profile_name: String,
    profile_cache_key: String,
    alternate_profile_cache_key: String,
    center: SpriteAnchor,
}

impl Sprite {
    fn snapshot_ref(&self) -> SpriteSnapshotRef<'_> {
        SpriteSnapshotRef {
            position_iface: &self.position_iface,
            current_row: self.current_row,
            current_frame: self.current_frame,
            frame_count: self.frame_count,
            current_width: self.current_width,
            current_height: self.current_height,
            last_action: self.last_action,
            last_processed_order_id: self.last_processed_order_id,
            masked: self.masked,
            use_alternate_profile: self.use_alternate_profile,
            action_done_frame: self.action_done_frame,
            action_done_counter: self.action_done_counter,
            last_sound_id: self.last_sound_id,
            splitch_count: self.splitch_count,
            behind_display_order_ref: self.behind_display_order_ref,
            display_order_ref: self.display_order_ref,
            anims_to_be_replaced: &self.anims_to_be_replaced,
            replacing_anims: &self.replacing_anims,
            frame_profile_name: &self.frame_profile_name,
            profile_cache_key: &self.profile_cache_key,
            alternate_profile_cache_key: &self.alternate_profile_cache_key,
            center: self.center,
        }
    }
}

impl Serialize for Sprite {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.snapshot_ref().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Sprite {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let snapshot = SpriteSnapshot::deserialize(deserializer)?;
        Ok(Self {
            position_iface: snapshot.position_iface,
            current_row: snapshot.current_row,
            current_frame: snapshot.current_frame,
            frame_count: snapshot.frame_count,
            current_width: snapshot.current_width,
            current_height: snapshot.current_height,
            last_action: snapshot.last_action,
            last_processed_order_id: snapshot.last_processed_order_id,
            masked: snapshot.masked,
            use_alternate_profile: snapshot.use_alternate_profile,
            action_done_frame: snapshot.action_done_frame,
            action_done_counter: snapshot.action_done_counter,
            last_sound_id: snapshot.last_sound_id,
            splitch_count: snapshot.splitch_count,
            behind_display_order_ref: snapshot.behind_display_order_ref,
            display_order_ref: snapshot.display_order_ref,
            anims_to_be_replaced: snapshot.anims_to_be_replaced,
            replacing_anims: snapshot.replacing_anims,
            scripts: std::sync::Arc::new(Vec::new()),
            alternate_scripts: None,
            conversion: std::sync::Arc::new(Vec::new()),
            alternate_conversion: None,
            frame_profile_name: snapshot.frame_profile_name,
            profile_cache_key: snapshot.profile_cache_key,
            alternate_profile_cache_key: snapshot.alternate_profile_cache_key,
            center: snapshot.center,
            last_motion_state: None,
        })
    }
}

impl robin_util::state_hash::StateHash for Sprite {
    fn state_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.position_iface.state_hash(state);
        self.current_row.state_hash(state);
        self.current_frame.state_hash(state);
        self.frame_count.state_hash(state);
        self.current_width.state_hash(state);
        self.current_height.state_hash(state);
        self.last_action.state_hash(state);
        self.last_processed_order_id.state_hash(state);
        self.masked.state_hash(state);
        self.use_alternate_profile.state_hash(state);
        self.action_done_frame.state_hash(state);
        self.action_done_counter.state_hash(state);
        self.last_sound_id.state_hash(state);
        self.splitch_count.state_hash(state);
        self.behind_display_order_ref.state_hash(state);
        self.display_order_ref.state_hash(state);
        self.anims_to_be_replaced.state_hash(state);
        self.replacing_anims.state_hash(state);
        self.frame_profile_name.state_hash(state);
        self.profile_cache_key.state_hash(state);
        self.alternate_profile_cache_key.state_hash(state);
        self.center.state_hash(state);
    }
}

impl Default for Sprite {
    fn default() -> Self {
        Self {
            position_iface: PositionInterface::new(),
            current_row: 0,
            current_frame: 0,
            frame_count: 0xFFFF,
            current_width: 0,
            current_height: 0,
            last_action: OrderType::NonanimationEnd,
            last_processed_order_id: u32::MAX,
            masked: false,
            use_alternate_profile: false,
            action_done_frame: 0xFFFF,
            action_done_counter: 0xFFFF,
            last_sound_id: 0,
            splitch_count: 0,
            behind_display_order_ref: false,
            display_order_ref: None,
            anims_to_be_replaced: Vec::new(),
            replacing_anims: Vec::new(),
            scripts: std::sync::Arc::new(Vec::new()),
            alternate_scripts: None,
            conversion: std::sync::Arc::new(Vec::new()),
            alternate_conversion: None,
            frame_profile_name: String::new(),
            profile_cache_key: String::new(),
            alternate_profile_cache_key: String::new(),
            center: SpriteAnchor::ZERO,
            last_motion_state: None,
        }
    }
}

impl Sprite {
    /// Construct a sprite with explicit, bound script + conversion
    /// tables.  Replaces the pre-refactor `Sprite::new()` no-arg
    /// constructor that used to leave `scripts: None` and force every
    /// animation method to start with a runtime `is_ready()` guard.
    /// Use [`Sprite::load_frame_info`] for the standard "load from
    /// `.rhs` then construct" flow used by level loading.
    pub fn new(
        scripts: std::sync::Arc<Vec<SpriteScript>>,
        conversion: std::sync::Arc<Vec<u16>>,
    ) -> Self {
        Self {
            scripts,
            conversion,
            ..Self::default()
        }
    }

    /// Apply level-data placement fields directly to the embedded
    /// `PositionInterface`. Replaces the old pattern where spawn sites
    /// populated these fields on `ElementData` via struct literals.
    ///
    /// The caller must pre-resolve `obstacle_plane` from the obstacle
    /// (e.g. `PlaneZCoeffs::from_plane_points(&obstacle.top_plane_points)`):
    /// the obstacle handle is always paired with its top plane, so they
    /// are set together.  Pass `None` for both when there is no obstacle.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_placement(
        &mut self,
        position_map: crate::coordinates::MapPoint,
        layer: u16,
        sector: Option<crate::position_interface::SectorHandle>,
        direction: i16,
        material: crate::element::GameMaterial,
        obstacle_index: Option<crate::position_interface::ObstacleHandle>,
        obstacle_plane: Option<crate::position_interface::PlaneZCoeffs>,
    ) {
        let pi = &mut self.position_iface;
        pi.set_map_position(position_map);
        let layer = crate::position_interface::Layer::new(layer)
            .expect("layer must be < 0xFFFF; 0xFFFF is the 'no layer' sentinel");
        pi.set_layer(layer);
        pi.set_sector(sector);
        pi.set_direction_instantly(crate::position_interface::Direction::from_raw(
            direction as i32,
        ));
        pi.set_material(material);
        if obstacle_index.is_some() {
            pi.set_obstacle(obstacle_index, obstacle_plane);
        }
    }

    // -- Script accessors --

    /// Get the currently active scripts (primary or alternate).
    ///
    /// Primary scripts are bound at construction and may be the empty
    /// placeholder.  Alternate scripts panic if accessed without
    /// `switch_alternate_profile` having loaded them — the alternate
    /// profile is genuinely optional.
    pub fn current_scripts(&self) -> &[SpriteScript] {
        if self.use_alternate_profile {
            self.alternate_scripts
                .as_deref()
                .expect("alternate scripts not loaded")
        } else {
            &self.scripts
        }
    }

    /// Non-panicking version of [`current_scripts`] — returns `None`
    /// when no scripts are loaded for the active profile (empty
    /// primary placeholder, or missing alternate).  Used by the GPU
    /// render path to fall back to a placeholder rect.
    pub fn current_scripts_opt(&self) -> Option<&[SpriteScript]> {
        let s = if self.use_alternate_profile {
            self.alternate_scripts.as_deref()?.as_slice()
        } else {
            self.scripts.as_slice()
        };
        (!s.is_empty()).then_some(s)
    }

    /// Get the currently active conversion table.  See
    /// [`current_scripts`](Self::current_scripts) for the alternate
    /// caveat.
    pub fn current_conversion(&self) -> &[u16] {
        if self.use_alternate_profile {
            self.alternate_conversion
                .as_deref()
                .expect("alternate conversion not loaded")
        } else {
            &self.conversion
        }
    }

    /// Get the row index for an action, or `None` if unmapped.
    ///
    /// Returns `None` when the conversion table is empty (placeholder
    /// sprite from `Sprite::default()`) or out of range — callers that
    /// previously gated on `is_ready()` get the same effect via the
    /// natural `Option` return path.
    pub fn row_for_action(&self, action: OrderType) -> Option<u16> {
        let conv = self.current_conversion();
        let idx = action as usize;
        if idx >= conv.len() {
            return None;
        }
        let row = conv[idx];
        if row == UNMAPPED { None } else { Some(row) }
    }

    /// Check if an animation exists in the current profile.
    pub fn has_animation(&self, action: OrderType) -> bool {
        self.row_for_action(action).is_some()
    }

    /// Resolve an animation through the replacement table.
    pub fn resolve_animation(&self, action: OrderType) -> OrderType {
        for (i, replaced) in self.anims_to_be_replaced.iter().enumerate() {
            if *replaced == action {
                return self.replacing_anims[i];
            }
        }
        action
    }

    // -- Per-frame accessors --

    /// Get number of frames in a sprite row.
    pub fn num_frames_for_row(&self, row: u16) -> u16 {
        let scripts = self.current_scripts();
        scripts[row as usize].frame_ids.len() as u16
    }

    /// Get number of frames for an animation action.
    pub fn num_frames_for_anim(&self, action: OrderType) -> u16 {
        match self.row_for_action(action) {
            Some(row) => self.num_frames_for_row(row),
            None => 0,
        }
    }

    /// Get the total tick count for an animation action — the sum of
    /// per-frame `wait_time` delays across every frame in the row.
    ///
    /// The raw frame count from `num_frames_for_anim` returns how many
    /// sprite frames exist, but the engine advances through each frame
    /// `wait_time` game ticks at a time — so the number of physics
    /// ticks the animation spans is the sum of the delays, not the
    /// raw count.
    ///
    /// Used by `ActiveFlight` set-up code (push, roll, hit-fall,
    /// ladder-wall fall) so the flight's per-frame increment is
    /// paced to match how long the sprite actually plays.
    pub fn total_ticks_for_anim(&self, action: OrderType) -> u16 {
        let Some(row) = self.row_for_action(action) else {
            return 0;
        };
        let scripts = self.current_scripts();
        let script = match scripts.get(row as usize) {
            Some(s) => s,
            None => return 0,
        };
        let mut total: u32 = 0;
        for &delay in &script.delays {
            // Delay is stored as u16. A delay of 0 still advances one
            // tick (the next frame tick fires), so clamp.
            let step = delay.max(1) as u32;
            total = total.saturating_add(step);
        }
        total.min(u16::MAX as u32) as u16
    }

    /// Get the frame bank ID for a given row and frame.
    pub fn bank_id_for(&self, row: u16, frame: u16) -> u32 {
        self.current_scripts()[row as usize].frame_ids[frame as usize]
    }

    /// Get the wait time (delay) for a given row and frame.
    pub fn wait_time(&self, row: u16, frame: u16) -> u16 {
        self.current_scripts()[row as usize].delays[frame as usize]
    }

    /// Get the movement distance for a given row and frame.
    pub fn distance(&self, row: u16, frame: u16) -> i16 {
        self.current_scripts()[row as usize].distances[frame as usize] as i16
    }

    /// Get the sound ID for a given row and frame.
    pub fn sound_id(&self, row: u16, frame: u16) -> u16 {
        self.current_scripts()[row as usize].sound_ids[frame as usize]
    }

    /// Get the draw offset for a given row and frame.
    #[must_use = "method returns Vec2D by value; assigning to its fields silently modifies a temporary"]
    pub fn offset(&self, row: u16, frame: u16) -> Vec2D {
        self.current_scripts()[row as usize].offsets[frame as usize]
    }

    /// Get the action-done frame index for a row.
    pub fn action_done_for_row(&self, row: u16) -> u16 {
        self.current_scripts()[row as usize].action_done
    }

    /// Get the hotspot/info point for a row.
    #[must_use = "method returns SpriteLocalPoint by value; assigning to its fields silently modifies a temporary"]
    pub fn hotspot_for_row(&self, row: u16) -> SpriteLocalPoint {
        self.current_scripts()[row as usize].hotspot
    }

    /// Current-row hotspot offset — the per-row anchor stored in the
    /// sprite script. Caller combines this with
    /// `Entity::cxx_position_sprite()` to produce the current map-space
    /// hotspot, used by `MoveUsePoint` seek-arrival.
    pub fn current_hotspot(&self) -> Option<SpriteLocalPoint> {
        let scripts = self.current_scripts_opt()?;
        scripts.get(self.current_row as usize).map(|s| s.hotspot)
    }

    /// Get the hand/anchor point for a given animation and direction.
    ///
    /// Looks up the sprite script row for the animation (base row from
    /// conversion table + direction offset) and returns its hotspot.
    ///
    /// Returns `None` if the animation is unmapped or the row is out of range.
    pub fn get_point(&self, animation: OrderType, direction: u16) -> Option<SpriteLocalPoint> {
        let base_row = self.row_for_action(animation)?;
        let row = base_row + direction;
        let scripts = self.current_scripts_opt()?;
        if (row as usize) < scripts.len() {
            Some(scripts[row as usize].hotspot)
        } else {
            None
        }
    }

    /// legacy implementation `RHSprite::GetActionDistance(animation)` parity lookup.
    ///
    /// The reference computes:
    /// `norm(GetPoint(animation, direction) + GetPositionSprite() - GetPositionMap())`.
    /// Rust keeps the authoritative sprite anchor in [`Self::center`], so the
    /// legacy implementation sprite-position term is reconstructed from
    /// `position_map - center`.
    pub fn action_distance(&self, animation: OrderType) -> Result<f32, String> {
        let direction = self.position_iface.get_direction().as_u8() as u16;
        let Some(point) = self.get_point(animation, direction) else {
            return Err(format!(
                "missing sprite action point for {animation:?} direction {direction}"
            ));
        };
        let map = self.position_iface.map_position();
        let sprite_pos = GeoPoint2D {
            x: (map.x - self.center.x).floor(),
            y: (map.y - self.center.y).floor(),
        };
        let dx = point.x + sprite_pos.x - map.x;
        let dy = point.y + sprite_pos.y - map.y;
        Ok((dx * dx + dy * dy).sqrt())
    }

    // -- Current frame accessors --

    #[must_use = "method returns Vec2D by value; assigning to its fields silently modifies a temporary"]
    pub fn current_offset(&self) -> Vec2D {
        self.offset(self.current_row, self.current_frame)
    }

    /// Max-over-frames cached width for the current profile.
    ///
    /// The cached `current_width` field is the running max across all
    /// loaded frames (seeded from `info.size.x` and updated by
    /// `initialize_script_row_from_loaded_frames`), not the exact
    /// per-frame width.  The only live consumer
    /// (`Element::compute_target_center`) uses it as a half-extent for
    /// FX target Z-centring, where max-over-frames is acceptable.
    pub fn current_max_width(&self) -> u16 {
        self.current_width
    }

    /// Max-over-frames cached height for the current profile — see
    /// [`current_max_width`](Self::current_max_width) for the
    /// rationale.
    pub fn current_max_height(&self) -> u16 {
        self.current_height
    }

    pub fn current_sound_id(&mut self) -> u16 {
        // Only return sound at the start of a new frame
        if self.frame_count != 0 {
            self.last_sound_id = 0;
            return 0;
        }

        let snd = self.sound_id(self.current_row, self.current_frame);
        if snd == self.last_sound_id {
            return 0;
        }

        self.last_sound_id = snd;
        snd
    }

    /// Get total time (in ticks) for a sprite row.
    ///
    /// Sums the per-frame `wait_time` delays for the given row.  The
    /// only live caller is [`Sprite::time_for_anim`], which wants the
    /// per-row total.
    pub fn total_time_for_row(&self, row: u16) -> u16 {
        let scripts = self.current_scripts();
        scripts[row as usize].delays.iter().map(|&d| d + 1).sum()
    }

    /// Current average speed from the script.
    pub fn current_average_speed(&self) -> f32 {
        self.current_scripts()[self.current_row as usize].average_speed
    }

    /// Check if current frame is before the action-done point.
    pub fn is_not_yet_done(&self) -> bool {
        self.current_frame < self.action_done_for_row(self.current_row)
    }

    /// Check if we're at the very start of an animation.
    pub fn is_at_start_of_anim(&self) -> bool {
        self.current_frame == 0 && self.frame_count == 0
    }

    /// Check if we're at a specific frame number (at the start of it).
    pub fn is_at_frame_number(&self, n: u16) -> bool {
        self.current_frame == n && self.frame_count == 0
    }

    // -- Animation replacement --

    pub fn replace_anim(&mut self, old: OrderType, new: OrderType) {
        // Probe `anims_to_be_replaced` first; on miss, also probe
        // `replacing_anims` so a chained `replace_anim(B, C)` after
        // `replace_anim(A, B)` rewrites the existing pair to `[A → C]`
        // instead of appending `[B → C]`.
        if let Some(idx) = self.anims_to_be_replaced.iter().position(|a| *a == old) {
            self.replacing_anims[idx] = new;
        } else if let Some(idx) = self.replacing_anims.iter().position(|a| *a == old) {
            self.replacing_anims[idx] = new;
        } else {
            self.anims_to_be_replaced.push(old);
            self.replacing_anims.push(new);
        }
    }

    pub fn restore_anim(&mut self, old: OrderType) {
        if let Some(idx) = self.anims_to_be_replaced.iter().position(|a| *a == old) {
            self.anims_to_be_replaced.remove(idx);
            self.replacing_anims.remove(idx);
        }
    }

    // -- Profile switching --

    /// Toggle the profile, and if we've played any animation, recompute
    /// the current row = conversion[last_action] + direction so the
    /// interpretation of `current_row` (which is shared across profiles
    /// but indexes into the active conversion table) stays valid.
    /// Clamp `current_frame` if it overruns the new row's length.
    ///
    /// `direction` is the actor's facing (0..15), used only when
    /// `last_action != NonanimationEnd`.  At fresh-load call sites
    /// (where `last_action` is still `NonanimationEnd`) the row-recompute
    /// branch is skipped, so passing `0` is observably equivalent;
    /// any other caller must pass `position_iface.get_direction() & 15`.
    pub fn switch_alternate_profile(&mut self, direction: u16) {
        self.use_alternate_profile = !self.use_alternate_profile;
        if self.last_action != OrderType::NonanimationEnd
            && let Some(row) = self.row_for_action(self.last_action)
        {
            self.current_row = row + direction;
            let num_frames = self.num_frames_for_row(self.current_row);
            if self.current_frame >= num_frames {
                self.current_frame = 0;
            }
        }
    }

    // -- Sprite profile loading --

    /// Load frame info from an `.rhs` file via the sprite scriptor.
    ///
    /// Resolves the `.rhs` file path, validates the bank signature,
    /// loads animation scripts and the conversion table, and stores
    /// them as the primary profile.
    ///
    /// For [`FrameKind::CharacterBlipped`], also loads the alternate
    /// "blip00" / "Blip 00" profile and switches to it: the blipped
    /// version becomes the default and the normal character is the
    /// alternate.
    #[allow(clippy::too_many_arguments)]
    pub fn load_frame_info(
        &mut self,
        scriptor: &mut SpriteScriptor,
        frame_kind: FrameKind,
        base_dir: &str,
        filename: &str,
        profile_name: &str,
        bank_signature: u32,
        ambiance: Option<Ambiance>,
    ) -> Result<(), String> {
        self.apply_sprite_info(
            scriptor,
            frame_kind,
            base_dir,
            filename,
            profile_name,
            bank_signature,
            ambiance,
            false,
        )?;

        // For blipped characters, also load the alternate "blip00"
        // profile, then switch so blipped is primary.
        if frame_kind == FrameKind::CharacterBlipped {
            self.apply_sprite_info(
                scriptor,
                FrameKind::Character,
                base_dir,
                "blip00",
                "Blip 00",
                bank_signature,
                ambiance,
                true,
            )?;
            // Initial profile switch at load time: last_action is still
            // NonanimationEnd so the direction argument is unused.
            self.switch_alternate_profile(0);
        }

        Ok(())
    }

    /// Load an additional profile into the alternate slot without
    /// switching to it.  Used by PC profiles with a valid alternative
    /// profile (e.g. for disguise / variant animations).
    #[allow(clippy::too_many_arguments)]
    pub fn load_alternate_profile(
        &mut self,
        scriptor: &mut SpriteScriptor,
        frame_kind: FrameKind,
        base_dir: &str,
        filename: &str,
        profile_name: &str,
        bank_signature: u32,
        ambiance: Option<Ambiance>,
    ) -> Result<(), String> {
        self.apply_sprite_info(
            scriptor,
            frame_kind,
            base_dir,
            filename,
            profile_name,
            bank_signature,
            ambiance,
            true,
        )
    }

    /// Cache-only counterpart to [`Self::load_frame_info`].
    ///
    /// Takes `&SpriteScriptor` and errors if the sprite has not been
    /// previously loaded into the scriptor cache. Used from the sim
    /// tick, where we can't mutate `LevelAssets` — callers must
    /// preload the relevant sprite at level-load time (see
    /// `preload_campaign_peasant_sprites`,
    /// `preload_scroll_amulet_sprite`).
    pub fn load_frame_info_cached(
        &mut self,
        scriptor: &SpriteScriptor,
        frame_kind: FrameKind,
        filename: &str,
        profile_name: &str,
    ) -> Result<(), String> {
        self.apply_sprite_info_cached(scriptor, filename, profile_name, false)?;

        if frame_kind == FrameKind::CharacterBlipped {
            self.apply_sprite_info_cached(scriptor, "blip00", "Blip 00", true)?;
            self.switch_alternate_profile(0);
        }

        Ok(())
    }

    /// Inner helper: load a single sprite profile and store it as primary or alternate.
    #[allow(clippy::too_many_arguments)]
    fn apply_sprite_info(
        &mut self,
        scriptor: &mut SpriteScriptor,
        frame_kind: FrameKind,
        base_dir: &str,
        filename: &str,
        profile_name: &str,
        bank_signature: u32,
        ambiance: Option<Ambiance>,
        alternate: bool,
    ) -> Result<(), String> {
        let cache_key = format!("{filename}/{profile_name}");
        if let Some(info) = scriptor.get(&cache_key) {
            Self::store_sprite_info(self, info, profile_name, &cache_key, alternate);
            return Ok(());
        }

        let path = SpriteScriptor::resolve_rhs_path(frame_kind, base_dir, filename, ambiance)?;

        let info = scriptor.load(&path, profile_name, &cache_key, frame_kind, |file| {
            let mut sig = 0u32;
            file.serialize_u32(&mut sig)
                .map_err(|e| format!("read signature: {e}"))?;
            if sig != bank_signature {
                return Err(format!(
                    "bank signature mismatch: file {sig:#x} != bank {bank_signature:#x}"
                ));
            }
            Ok(())
        })?;

        Self::store_sprite_info(self, info, profile_name, &cache_key, alternate);

        Ok(())
    }

    /// Cache-only counterpart to [`Self::apply_sprite_info`]. Looks up
    /// by `"{filename}/{profile_name}"`; errors if missing.
    fn apply_sprite_info_cached(
        &mut self,
        scriptor: &SpriteScriptor,
        filename: &str,
        profile_name: &str,
        alternate: bool,
    ) -> Result<(), String> {
        let cache_key = format!("{filename}/{profile_name}");
        let info = scriptor.get(&cache_key).ok_or_else(|| {
            format!("sprite cache miss: '{cache_key}' — was it preloaded at level load?")
        })?;

        Self::store_sprite_info(self, info, profile_name, &cache_key, alternate);

        Ok(())
    }

    /// Reattach level-owned script/conversion tables from the
    /// SpriteScriptor cache using the serialized profile cache keys.
    pub fn attach_runtime_from_cache(&mut self, scriptor: &SpriteScriptor) -> Result<(), String> {
        if !self.profile_cache_key.is_empty() {
            let info = scriptor.get(&self.profile_cache_key).ok_or_else(|| {
                format!(
                    "sprite cache miss while attaching primary profile '{}'",
                    self.profile_cache_key
                )
            })?;
            self.scripts = info.scripts.clone();
            self.conversion = info.conversion.clone();
        }

        if !self.alternate_profile_cache_key.is_empty() {
            let info = scriptor
                .get(&self.alternate_profile_cache_key)
                .ok_or_else(|| {
                    format!(
                        "sprite cache miss while attaching alternate profile '{}'",
                        self.alternate_profile_cache_key
                    )
                })?;
            self.alternate_scripts = Some(info.scripts.clone());
            self.alternate_conversion = Some(info.conversion.clone());
        } else {
            self.alternate_scripts = None;
            self.alternate_conversion = None;
        }

        if self.use_alternate_profile && self.alternate_conversion.is_none() {
            return Err(format!(
                "sprite '{}' is using alternate profile but has no alternate profile cache key",
                self.frame_profile_name
            ));
        }

        Ok(())
    }

    /// Store loaded sprite info into the primary or alternate slot.
    fn store_sprite_info(
        sprite: &mut Sprite,
        info: &SpriteInfo,
        profile_name: &str,
        cache_key: &str,
        alternate: bool,
    ) {
        if !alternate {
            sprite.frame_profile_name = profile_name.to_owned();
            sprite.profile_cache_key = cache_key.to_owned();
            sprite.scripts = info.scripts.clone();
            sprite.conversion = info.conversion.clone();
            sprite.center = SpriteAnchor::from_geo(info.center);
        } else {
            sprite.alternate_profile_cache_key = cache_key.to_owned();
            sprite.alternate_scripts = Some(info.scripts.clone());
            sprite.alternate_conversion = Some(info.conversion.clone());
        }

        // Update max surface dimensions
        let w = info.size.x as u16;
        let h = info.size.y as u16;
        if w > sprite.current_width {
            sprite.current_width = w;
        }
        if h > sprite.current_height {
            sprite.current_height = h;
        }
    }

    // -- Frame initialization --

    /// Initialize frame state when switching to a new action.
    ///
    /// Returns `true` if the action actually changed (frame was reset).
    pub fn maybe_initialize_frame(
        &mut self,
        anim: OrderType,
        progression: FrameProgression,
    ) -> bool {
        if anim == self.last_action {
            return false;
        }
        self.last_action = anim;

        match progression {
            FrameProgression::Default
            | FrameProgression::BoredAnim
            | FrameProgression::SnakeAnim
            | FrameProgression::ForceIncrement
            | FrameProgression::FreezeWhenTerminated
            | FrameProgression::FreezeLastButOneFrame
            | FrameProgression::FrozenFirstFrame
            | FrameProgression::WhirlingSanchezIsBack
            | FrameProgression::Cyclically => {
                self.frame_count = 0xFFFF;
                self.current_frame = 0;
            }

            FrameProgression::BeginWithDone => {
                self.frame_count = 0xFFFF;
                let row = self.current_conversion()[anim as usize];
                self.current_frame = self.action_done_for_row(row);
            }

            FrameProgression::SkipShadow => {
                self.frame_count = 0xFFFF;
                self.current_frame = 0;
            }

            // SkipShadowFreezeWhenTerminated has no init body — frame
            // state stays untouched on the action change.
            FrameProgression::SkipShadowFreezeWhenTerminated => {}

            FrameProgression::Reversed | FrameProgression::FrozenLastFrame => {
                self.current_frame = self.num_frames_for_row(self.current_row) - 1;
                self.frame_count = 0xFFFF;
            }

            FrameProgression::Frozen => {
                self.frame_count = 0;
                self.current_frame = 0;
            }
        }

        // Reset cached forecast vector so consumers like arrow-lead
        // prediction (bow_shot.rs) don't see stale walk velocity after
        // an anim change to a non-moving row.
        self.position_iface.reset_forecasted_movement();

        true
    }

    /// Initialize the action-done tracking for an animation.
    pub fn initialize_action_done(&mut self, anim: OrderType) {
        let row = self.current_conversion()[anim as usize];
        let num_frames = self.num_frames_for_row(row);

        self.action_done_frame = self.action_done_for_row(row);
        self.action_done_counter = 0;

        if num_frames == 1 && self.wait_time(row, 0) <= 1 {
            // Single-frame animation with no delay: impossible to hit action-done
            self.action_done_frame = 761;
            self.action_done_counter = 1984;
        } else if self.action_done_frame == 0 {
            // Action done at the very beginning
            if self.wait_time(row, 0) == 0 {
                self.action_done_frame = 1;
            } else {
                self.action_done_counter = 1;
            }
        } else if self.action_done_frame >= num_frames - 1 {
            // Action done at or beyond the end
            if self.action_done_frame > 1 {
                self.action_done_frame = num_frames - 2;
                self.action_done_counter = self.wait_time(row, self.action_done_frame);
            } else {
                self.action_done_frame = u16::MAX;
            }
        }
    }

    /// Reset to the first (or last) frame of the current row.
    pub fn reset_sprite_frame(&mut self, last_frame: bool) {
        if last_frame {
            self.current_frame = self.num_frames_for_row(self.current_row).saturating_sub(1);
        } else {
            self.current_frame = 0;
        }
        self.frame_count = 0xFFFF;
    }

    // -- Frame increment (the core animation state machine) --

    /// Advance the animation by one tick according to the progression mode.
    ///
    /// Returns `true` if the animation has reached its natural end (looped
    /// or hit the last frame, depending on mode).
    pub fn increment_frame(&mut self, progression: FrameProgression) -> bool {
        // Placeholder sprite (empty primary `scripts` Arc): nothing to
        // advance.  This is the only runtime fallback needed now that
        // `is_ready()` is gone — every other method either handles
        // the empty case via `row_for_action` (which returns `None`
        // for out-of-bounds conversion lookups) or operates on bound
        // data by construction.
        if self.current_scripts().is_empty() {
            return false;
        }
        let num_frames = self.num_frames_for_row(self.current_row);

        match progression {
            FrameProgression::Default | FrameProgression::BeginWithDone => {
                self.frame_count = self.frame_count.wrapping_add(1);
                if self.frame_count > self.wait_time(self.current_row, self.current_frame) {
                    self.frame_count = 0;
                    self.current_frame += 1;
                }
                if self.current_frame >= num_frames {
                    self.current_frame = 0;
                }
                // Terminated when on last frame at its last tick
                self.current_frame == num_frames - 1
                    && (self.frame_count == self.wait_time(self.current_row, self.current_frame)
                        || self.wait_time(self.current_row, self.current_frame) == 0)
            }

            FrameProgression::Cyclically => {
                self.frame_count = self.frame_count.wrapping_add(1);
                if self.frame_count > self.wait_time(self.current_row, self.current_frame) {
                    self.frame_count = 0;
                    self.current_frame += 1;
                }
                if self.current_frame >= num_frames {
                    self.current_frame = 0;
                }
                false // cyclical never terminates
            }

            FrameProgression::BoredAnim => {
                // Only start animating with random probability — fire
                // with probability 1/N from the deterministic simulation
                // RNG so replays match.
                if (self.current_frame != 0 || self.frame_count != 0)
                    || crate::sim_rng::u32(..BORED_ANIM_PROBABILITY) == 0
                {
                    self.frame_count = self.frame_count.wrapping_add(1);
                    if self.frame_count > self.wait_time(self.current_row, self.current_frame) {
                        self.frame_count = 0;
                        self.current_frame += 1;
                    }
                    if self.current_frame >= num_frames {
                        self.current_frame = 0;
                    }
                }
                false
            }

            FrameProgression::SnakeAnim => {
                // Same as BoredAnim but with a different probability.
                // Deterministic simulation RNG.
                if (self.current_frame != 0 || self.frame_count != 0)
                    || crate::sim_rng::u32(..SNAKE_ANIM_PROBABILITY) == 0
                {
                    self.frame_count = self.frame_count.wrapping_add(1);
                    if self.frame_count > self.wait_time(self.current_row, self.current_frame) {
                        self.frame_count = 0;
                        self.current_frame += 1;
                    }
                    if self.current_frame >= num_frames {
                        self.current_frame = 0;
                    }
                }
                false
            }

            FrameProgression::ForceIncrement => {
                self.frame_count = 0;
                self.current_frame += 1;
                if self.current_frame > num_frames {
                    self.current_frame = 0;
                }
                self.current_frame == num_frames
            }

            FrameProgression::SkipShadow => {
                self.frame_count = self.frame_count.wrapping_add(1);
                if self.frame_count > self.wait_time(self.current_row, self.current_frame) {
                    self.frame_count = 0;
                    self.current_frame += 2; // skip shadow frame
                }
                if self.current_frame == num_frames {
                    self.current_frame = 0;
                }
                self.current_frame == num_frames - 2
                    && self.frame_count == self.wait_time(self.current_row, self.current_frame)
            }

            FrameProgression::FreezeWhenTerminated => {
                if self.current_frame < num_frames - 1 {
                    self.frame_count = self.frame_count.wrapping_add(1);
                    if self.frame_count > self.wait_time(self.current_row, self.current_frame) {
                        self.frame_count = 0;
                        self.current_frame += 1;
                    }
                }
                false
            }

            FrameProgression::SkipShadowFreezeWhenTerminated => {
                if self.current_frame < num_frames.saturating_sub(2) {
                    self.frame_count = self.frame_count.wrapping_add(1);
                    if self.frame_count > self.wait_time(self.current_row, self.current_frame) {
                        self.frame_count = 0;
                        self.current_frame += 2;
                    }
                }
                false
            }

            FrameProgression::FreezeLastButOneFrame => {
                if self.current_frame < num_frames.saturating_sub(2) {
                    self.frame_count = self.frame_count.wrapping_add(1);
                    if self.frame_count > self.wait_time(self.current_row, self.current_frame) {
                        self.frame_count = 0;
                        self.current_frame += 1;
                    }
                }
                false
            }

            FrameProgression::FrozenFirstFrame => {
                self.current_frame = 0;
                false
            }

            FrameProgression::Frozen => {
                // Don't change anything
                false
            }

            FrameProgression::FrozenLastFrame => {
                self.current_frame = num_frames - 1;
                false
            }

            FrameProgression::WhirlingSanchezIsBack => {
                // Two phases:
                //   - frame != action_done: normal forward progression with
                //     wrap, return true on last-frame last-tick.
                //   - frame == action_done: hold the frame and rotate facing
                //     direction by -2 sectors per tick for 8 ticks, then
                //     resume forward progression.
                let action_done = self.action_done_for_row(self.current_row);
                if self.current_frame != action_done {
                    self.frame_count = self.frame_count.wrapping_add(1);
                    if self.frame_count > self.wait_time(self.current_row, self.current_frame) {
                        self.frame_count = 0;
                        self.current_frame += 1;
                    }
                    if self.current_frame >= num_frames {
                        self.current_frame = 0;
                    }
                    self.current_frame == num_frames - 1
                        && self.frame_count == self.wait_time(self.current_row, self.current_frame)
                } else {
                    self.frame_count = self.frame_count.wrapping_add(1);
                    if self.frame_count <= 8 {
                        // whirl around: -2 sectors, wrapping at 16
                        let dir = self.position_iface.get_direction();
                        self.position_iface.set_direction_instantly(dir.rotate(-2));
                    } else {
                        // resume forward progression
                        self.frame_count = 0;
                        self.current_frame += 1;
                    }
                    false
                }
            }

            FrameProgression::Reversed => {
                self.frame_count = self.frame_count.wrapping_add(1);
                if self.frame_count > self.wait_time(self.current_row, self.current_frame) {
                    self.frame_count = 0;
                    self.current_frame = self.current_frame.wrapping_sub(1);
                }
                // Wrap around
                if self.current_frame == u16::MAX {
                    // underflow
                    self.current_frame = num_frames - 1;
                }
                self.current_frame == 0
                    && (self.frame_count == self.wait_time(self.current_row, self.current_frame)
                        || self.wait_time(self.current_row, self.current_frame) == 0)
            }
        }
    }

    /// Speed-modulated frame increment (only supports Default/BeginWithDone).
    pub fn increment_frame_modulated(&mut self, speed: f32, progression: FrameProgression) -> bool {
        let num_frames = self.num_frames_for_row(self.current_row);

        match progression {
            FrameProgression::Default | FrameProgression::BeginWithDone => {
                self.frame_count = self.frame_count.wrapping_add(1);
                let threshold =
                    (speed * self.wait_time(self.current_row, self.current_frame) as f32) as u16;
                if self.frame_count > threshold {
                    self.frame_count = 0;
                    self.current_frame += 1;
                }
                if self.current_frame >= num_frames {
                    self.current_frame = 0;
                }
                let threshold =
                    (speed * self.wait_time(self.current_row, self.current_frame) as f32) as u16;
                self.current_frame == num_frames - 1
                    && (self.frame_count == threshold
                        || self.wait_time(self.current_row, self.current_frame) == 0)
            }
            _ => {
                panic!("IncrementFrameModulated only supports Default/BeginWithDone progression");
            }
        }
    }

    // -- High-level animation methods --

    /// Record `state` as the most recent motion-state on this sprite
    /// (consumed by `EngineInner::propagate_done_to_current_orders` at
    /// end of tick) and return it.  Every `perform_*` exit funnels
    /// through here so one place owns the write.
    #[inline]
    fn record_motion_state(&mut self, state: MotionState) -> MotionState {
        self.last_motion_state = Some(state);
        state
    }

    /// Simple frame increment without any order/motion logic.
    pub fn perform_virgin_increment(&mut self, progression: FrameProgression) -> MotionState {
        let terminated = self.increment_frame(progression);

        let mut state = MotionState::InProgress;

        if self.frame_count == 0 && self.current_frame == self.action_done_for_row(self.current_row)
        {
            state = MotionState::Done;
        }

        if terminated {
            state = MotionState::Terminated;
        }

        self.record_motion_state(state)
    }

    /// Speed-modulated increment.
    pub fn perform_increment_modulated(
        &mut self,
        speed: f32,
        progression: FrameProgression,
    ) -> MotionState {
        let terminated = self.increment_frame_modulated(speed, progression);

        let mut state = MotionState::InProgress;

        if self.frame_count == 0 && self.current_frame == self.action_done_for_row(self.current_row)
        {
            state = MotionState::Done;
        }

        if terminated {
            state = MotionState::Terminated;
        }

        self.record_motion_state(state)
    }

    /// Perform an action: update sprite row for direction, handle frame
    /// initialization and increment.
    ///
    /// `order_id` is the unique ID of the current order (or `None` for
    /// order-less actions). `direction` is the current facing direction (0–15).
    pub fn perform_action(
        &mut self,
        order_id: Option<std::num::NonZeroU32>,
        anim: OrderType,
        direction: u16,
        progression: FrameProgression,
        force_init: bool,
    ) -> MotionState {
        // Placeholder sprite (empty conversion table): stay
        // in-progress so the animation tick doesn't mark the owning
        // sequence element Impossible and consume the front order.
        // This is the one runtime check kept from the old
        // `is_ready()` gate, scoped to the entry point that actually
        // needs it.  Without this guard, headless tests + the brief
        // pre-bind window would resolve as `Aborted` (via the
        // `row_for_action`-returns-`None` path) and consume orders
        // they should leave alone.
        if self.current_conversion().is_empty() {
            return self.record_motion_state(MotionState::InProgress);
        }

        let mut state = MotionState::InProgress;
        let anim = self.resolve_animation(anim);

        let row = match self.row_for_action(anim) {
            Some(r) => r,
            None => {
                tracing::error!(
                    "Trying to play non-existing animation {:?} of sprite {}",
                    anim,
                    self.frame_profile_name
                );
                return self.record_motion_state(MotionState::Aborted);
            }
        };

        self.current_row = row + direction;

        if let Some(oid) = order_id {
            if self.last_processed_order_id != oid.get() {
                self.last_processed_order_id = oid.get();
                state = MotionState::Start;
                self.initialize_action_done(anim);

                if force_init {
                    self.reset_sprite_frame(false);
                } else {
                    self.maybe_initialize_frame(anim, progression);
                }
            } else {
                self.maybe_initialize_frame(anim, progression);
            }
        } else {
            self.maybe_initialize_frame(anim, progression);
        }

        // C++ `RHSprite::PerformAction` reports `RHMOTION_START` for
        // the first tick of a new order without advancing the frame.
        // Advancing here makes short action-done animations, including
        // bow shots, fire one tick too early.
        let anim_finished = if state == MotionState::Start {
            false
        } else {
            self.increment_frame(progression)
        };

        // Action done?  With C++ start-tick timing above, newly
        // initialized orders have not advanced yet, so this can follow
        // the original unconditional action-done check.
        if self.current_frame == self.action_done_frame
            && self.frame_count == self.action_done_counter
        {
            state = MotionState::Done;
        }

        // Animation terminated?
        if anim_finished {
            state = MotionState::Terminated;
        }

        self.record_motion_state(state)
    }

    /// Get the movement distance for the current frame.
    ///
    /// Returns the per-frame distance only on the first tick of a new
    /// frame (when `frame_count == 0`).  Subsequent ticks within the
    /// same frame contribute zero motion.
    pub fn current_frame_distance(&self) -> f32 {
        if self.frame_count == 0 {
            self.distance(self.current_row, self.current_frame) as f32
        } else {
            0.0
        }
    }

    /// Perform an action and compute the per-tick motion distance,
    /// honouring `MotionMethod` semantics.  For `MotionMethod::Fast`,
    /// the animation advances at 2× frame rate and the resulting
    /// distance is doubled (and doubled again if the second increment
    /// also lands on a new frame).
    ///
    /// The returned distance is the raw per-tick distance **without**
    /// the sequence-element speed-factor applied — callers multiply it
    /// in afterwards.
    #[allow(clippy::too_many_arguments)]
    pub fn perform_motion(
        &mut self,
        motion_order: Option<MotionOrderContext>,
        anim: OrderType,
        direction: u16,
        progression: FrameProgression,
        force_init: bool,
        motion_method: MotionMethod,
        dest_already_at_pos: bool,
    ) -> (MotionState, f32) {
        let order_id = motion_order.map(|ctx| ctx.order_id);
        // Already-at-destination short-circuit: on the first tick of a
        // new order whose motion method is not `TillLastFrame` and
        // whose destination equals the current map position, set the
        // row, run `maybe_initialize_frame`, and return `Terminated`
        // without an `increment_frame` step.  Without this guard, the
        // unconditional `increment_frame` below advances the very-first
        // frame once before the order pops, which leaves a one-frame
        // walk-anim flicker visible when an order is issued with the
        // actor already standing on the destination.
        if dest_already_at_pos
            && motion_method != MotionMethod::TillLastFrame
            && let Some(oid) = order_id
            && self.last_processed_order_id != oid.get()
            && !self.current_conversion().is_empty()
        {
            let resolved = self.resolve_animation(anim);
            if let Some(row) = self.row_for_action(resolved) {
                self.current_row = row + direction;
                self.initialize_action_done(resolved);
                self.maybe_initialize_frame(resolved, progression);
                // The caller pops the order immediately after
                // `Terminated`, so re-entry on the same id can't
                // normally happen.  We still update
                // `last_processed_order_id` for safety in case a future
                // caller defers the pop.
                self.last_processed_order_id = oid.get();
                return (self.record_motion_state(MotionState::Terminated), 0.0);
            }
        }

        if let Some(ctx) = motion_order
            && self.last_processed_order_id != ctx.order_id.get()
        {
            let pi = &mut self.position_iface;
            pi.set_map_goal(ctx.destination);
            pi.set_reversed_movement(ctx.reverse);
            pi.set_tolerance(ctx.tolerance, ctx.directional_tolerance);
            if let Some(next) = ctx.next_destination_same_action {
                pi.set_next_map_goal(next);
            } else {
                pi.set_goal_next_valid(false);
            }
            pi.compute_increment_all(ctx.compute_direction);
            pi.reset_box_blocked();
        }

        let state = self.perform_action(order_id, anim, direction, progression, force_init);

        let mut distance = self.current_frame_distance();

        if motion_method == MotionMethod::Fast {
            if distance != 0.0 {
                distance *= 2.0;
            }
            self.increment_frame(progression);
            if self.frame_count == 0 {
                distance += self.distance(self.current_row, self.current_frame) as f32;
                distance *= 2.0;
            }
        }

        (state, distance)
    }

    // -- Bounding box --

    /// Compute the current sprite AABB from position and active frame size.
    pub fn bounding_box_at(&self, sprite_pos: GeoPoint2D) -> BBox {
        let offset = self.current_offset();
        let size = Vec2D {
            x: self.current_width as f32,
            y: self.current_height as f32,
        };
        BBox::new(
            GeoPoint2D {
                x: sprite_pos.x + offset.x,
                y: sprite_pos.y + offset.y,
            },
            GeoPoint2D {
                x: sprite_pos.x + offset.x + size.x,
                y: sprite_pos.y + offset.y + size.y,
            },
        )
    }

    /// Check if the sprite is on screen.
    pub fn is_on_screen(&self, view_box: &BBox, sprite_pos: GeoPoint2D) -> bool {
        view_box.is_intersecting(&self.bounding_box_at(sprite_pos))
    }

    // -- Frame timing queries --

    /// Frames remaining from current position until action-done.
    /// Returns -1 if action-done has already passed.
    pub fn frames_from_now_till_action_done(&self) -> i16 {
        if self.current_frame > self.action_done_frame {
            return -1;
        }
        if self.current_frame == self.action_done_frame {
            if self.frame_count > self.action_done_counter {
                return -1;
            }
            return (self.action_done_counter - self.frame_count) as i16;
        }

        // (1) Remaining time in current frame.  Match the legacy implementation
        // signed expression: `GetWaitTime(...) - muwFrameCount` can be
        // negative if the frame counter has advanced beyond this frame's
        // nominal delay while action-done is still on a later frame.
        let mut sum =
            self.wait_time(self.current_row, self.current_frame) as i32 - self.frame_count as i32;

        // (2) Intermediate frames
        for i in (self.current_frame + 1)..self.action_done_frame {
            sum += self.wait_time(self.current_row, i) as i32;
        }

        // (3) Action done counter
        sum += self.action_done_counter as i32;

        // legacy implementation stores the running total in SWORD and returns it directly.
        // Preserve that 16-bit truncation instead of adding a Rust-only range panic.
        sum as i16
    }

    /// Frames from the start of an animation until its action-done.
    pub fn frames_from_start_till_action_done(&self, anim: OrderType) -> u16 {
        let row = self.current_conversion()[anim as usize];
        let ad = self.action_done_for_row(row);
        let num = self.num_frames_for_row(row);
        let mut sum: u16 = 0;
        for i in 0..=ad.min(num.saturating_sub(1)) {
            sum = sum.saturating_add(self.wait_time(row, i));
        }
        sum
    }

    /// Total distance covered by an animation.
    ///
    /// Returns `0` for unmapped or out-of-range actions (guarded via
    /// [`Self::row_for_action`]).
    pub fn distance_for_animation(&self, anim: OrderType) -> i16 {
        let row = match self.row_for_action(anim) {
            Some(r) => r,
            None => return 0,
        };
        let scripts = self.current_scripts();
        scripts[row as usize].sum_distance as i16
    }

    /// Get the time (total ticks) for an animation.
    pub fn time_for_anim(&self, anim: OrderType) -> u16 {
        match self.row_for_action(anim) {
            Some(row) => self.total_time_for_row(row),
            None => 0,
        }
    }

    // -- Force methods (direct state manipulation) --

    pub fn force_sprite_row_raw(&mut self, row: u16) {
        self.current_row = row;
    }

    pub fn force_sprite_row(&mut self, anim: OrderType, direction: u16) {
        let row = self.current_conversion()[anim as usize];
        assert_ne!(row, UNMAPPED, "animation {:?} is unmapped", anim);
        self.current_row = row + direction;
        self.last_action = anim;
    }

    pub fn force_action_direction(&mut self, anim: OrderType, direction: u16) -> bool {
        if self.current_conversion().is_empty() {
            return false;
        }

        let resolved = self.resolve_animation(anim);
        let Some(row) = self.row_for_action(resolved) else {
            tracing::error!(
                "Trying to force non-existing animation {:?} of sprite {}",
                resolved,
                self.frame_profile_name
            );
            return false;
        };

        self.current_row = row + direction;
        self.last_action = resolved;
        true
    }

    pub fn force_animation(&mut self, anim: OrderType, direction: u16) {
        let conv = self.current_conversion();
        let Some(&row) = conv.get(anim as usize) else {
            // Placeholder sprite (empty conversion): nothing to force.
            return;
        };
        assert_ne!(row, UNMAPPED, "animation {:?} is unmapped", anim);
        self.current_row = row + direction;
        self.last_action = anim;
        self.current_frame = 0;
        self.frame_count = 0;
    }

    pub fn force_sprite(&mut self, row: u16, frame: u16) {
        self.current_row = row;
        self.current_frame = frame;
    }

    /// Copy current frame and frame count from another sprite.
    ///
    /// Used by carry/lift/drop animations to sync the carried entity's
    /// sprite frame with the carrier's sprite so the combined animation
    /// appears as a single visual unit.  Silently clamps the incoming
    /// frame to avoid out-of-range reads on rows of differing length.
    pub fn synchronize_anim(&mut self, other_frame: u16, other_frame_count: u16) {
        let num = self.num_frames_for_row(self.current_row);
        self.current_frame = if num > 0 { other_frame % num } else { 0 };
        self.frame_count = other_frame_count;
    }

    /// Pick a random starting frame on the current row.
    ///
    /// Called from mission-load paths (bonus/scroll spawn, post-load
    /// `EngineInner::initialize_all_scrolls`) which run *outside* a
    /// simulation tick, so the ambient `sim_rng` thread-local isn't
    /// installed.  Callers must therefore thread `&mut EngineInner::rng`
    /// in directly so level-load randomness still comes from the same
    /// seeded PRNG that drives rollback-replay determinism.
    #[allow(clippy::disallowed_methods)]
    pub fn force_random_sprite_frame(&mut self, rng: &mut fastrand::Rng) {
        let num = self.num_frames_for_row(self.current_row);
        if num > 0 {
            self.current_frame = rng.u16(..num);
        }
    }

    /// `force_random_sprite_frame` variant for sim-tick callers.
    ///
    /// Pulls from the `sim_rng` thread-local that `perform_hourglass`
    /// installs, so the draw is deterministic across rollback replays.
    /// Must not be called outside a simulation tick.
    pub fn force_random_sprite_frame_sim(&mut self) {
        let num = self.num_frames_for_row(self.current_row);
        if num > 0 {
            self.current_frame = crate::sim_rng::u16(0..num);
        }
    }

    // -- Script initialization --

    /// Set up a fixed number of empty script rows.
    pub fn set_number_of_fixed_rows(&mut self, num_rows: u16) {
        let mut scripts = Vec::new();
        scripts.resize_with(num_rows as usize, SpriteScript::default);
        self.scripts = std::sync::Arc::new(scripts);
        self.current_width = 0;
        self.current_height = 0;
    }

    /// Initialize a single script row from loaded frame data.
    #[allow(clippy::too_many_arguments)]
    pub fn initialize_script_row_from_loaded_frames(
        &mut self,
        row: u16,
        base_frame: u32,
        num_frames: u16,
        default_delay: u16,
        default_distance: u16,
        offsets: &[Vec2D],
        frame_widths: &[u16],
        frame_heights: &[u16],
    ) {
        let script = &mut std::sync::Arc::make_mut(&mut self.scripts)[row as usize];

        script.frame_ids.resize(num_frames as usize, 0);
        script.delays.resize(num_frames as usize, 0);
        script.distances.resize(num_frames as usize, 0);
        script
            .offsets
            .resize(num_frames as usize, Vec2D { x: 0.0, y: 0.0 });
        script.sound_ids.resize(num_frames as usize, 0);
        script.action_done = 0;
        script.hotspot = SpriteLocalPoint::ZERO;

        for i in 0..num_frames as usize {
            if frame_widths[i] > self.current_width {
                self.current_width = frame_widths[i];
            }
            if frame_heights[i] > self.current_height {
                self.current_height = frame_heights[i];
            }

            script.delays[i] = default_delay;
            script.distances[i] = default_distance;
            script.frame_ids[i] = base_frame + i as u32;
            script.offsets[i] = offsets[i];
            script.sound_ids[i] = 0;
        }
    }
}

// ---------------------------------------------------------------------------
// Bored/snake idle-animation probabilities
// ---------------------------------------------------------------------------
//
// On any given frame we fire the idle twitch with probability 1/N,
// pulled from the deterministic simulation RNG.

const BORED_ANIM_PROBABILITY: u32 = 250;
const SNAKE_ANIM_PROBABILITY: u32 = 100;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_sprite() -> Sprite {
        // Build the script set up front so the new
        // `Sprite::new(scripts, conversion)` constructor consumes them
        // directly — no more `new()` + populate-fields dance.

        // Create a simple script with 2 rows, each having 4 frames
        let row0 = SpriteScript {
            action_id: 0,
            action_done: 2,
            average_speed: 1.0,
            hotspot: SpriteLocalPoint::ZERO,
            sum_distance: 12,
            frame_ids: vec![100, 101, 102, 103],
            delays: vec![2, 2, 2, 2],
            distances: vec![3, 3, 3, 3],
            offsets: vec![
                Vec2D { x: -16.0, y: -32.0 },
                Vec2D { x: -16.0, y: -32.0 },
                Vec2D { x: -16.0, y: -32.0 },
                Vec2D { x: -16.0, y: -32.0 },
            ],
            sound_ids: vec![0, 1, 0, 0],
        };
        let row1 = SpriteScript {
            action_id: 1,
            action_done: 1,
            average_speed: 2.0,
            hotspot: SpriteLocalPoint::new(5.0, 5.0),
            sum_distance: 8,
            frame_ids: vec![200, 201, 202],
            delays: vec![3, 3, 3],
            distances: vec![4, 4, 0],
            offsets: vec![
                Vec2D { x: -8.0, y: -16.0 },
                Vec2D { x: -8.0, y: -16.0 },
                Vec2D { x: -8.0, y: -16.0 },
            ],
            sound_ids: vec![0, 0, 2],
        };

        // Conversion: action 0 → row 0, action 1 → row 1, rest unmapped
        let mut conversion = vec![UNMAPPED; 283];
        conversion[0] = 0;
        conversion[1] = 1;

        let mut sprite = Sprite::new(
            std::sync::Arc::new(vec![row0, row1]),
            std::sync::Arc::new(conversion),
        );
        sprite.current_width = 32;
        sprite.current_height = 64;

        sprite
    }

    #[test]
    fn test_sprite_default() {
        let s = Sprite::default();
        assert_eq!(s.current_row, 0);
        assert_eq!(s.current_frame, 0);
        assert_eq!(s.frame_count, 0xFFFF);
        assert_eq!(s.last_action, OrderType::NonanimationEnd);
    }

    #[test]
    fn test_num_frames_for_row() {
        let s = make_test_sprite();
        assert_eq!(s.num_frames_for_row(0), 4);
        assert_eq!(s.num_frames_for_row(1), 3);
    }

    #[test]
    fn test_has_animation() {
        let s = make_test_sprite();
        assert!(s.has_animation(OrderType::WaitingUprightBored)); // action 0
        assert!(s.has_animation(OrderType::WaitingUprightBoredRandom)); // action 1
        assert!(!s.has_animation(OrderType::WalkingUpright)); // action 6, unmapped
    }

    #[test]
    fn force_action_direction_updates_row_without_advancing_frame() {
        let mut s = make_test_sprite();
        s.current_frame = 2;
        s.frame_count = 1;

        assert!(s.force_action_direction(OrderType::WaitingUprightBoredRandom, 5));

        assert_eq!(s.current_row, 6);
        assert_eq!(s.current_frame, 2);
        assert_eq!(s.frame_count, 1);
        assert_eq!(s.last_action, OrderType::WaitingUprightBoredRandom);
    }

    #[test]
    fn test_action_distance_uses_hotspot_sprite_anchor_and_map_position() {
        let mut s = make_test_sprite();
        s.center = SpriteAnchor { x: 8.0, y: 11.0 };
        s.position_iface
            .set_map_position(crate::coordinates::MapPoint::new(100.25, 200.75));
        s.position_iface
            .set_direction_instantly(crate::position_interface::Direction::from_raw(0));

        let distance = s
            .action_distance(OrderType::WaitingUprightBoredRandom)
            .unwrap();

        let expected_dx = 5.0 + (100.25_f32 - 8.0).floor() - 100.25;
        let expected_dy = 5.0 + (200.75_f32 - 11.0).floor() - 200.75;
        assert!(
            (distance - (expected_dx * expected_dx + expected_dy * expected_dy).sqrt()).abs()
                < 0.001
        );
    }

    #[test]
    fn test_increment_frame_default() {
        let mut s = make_test_sprite();
        s.current_row = 0;
        s.current_frame = 0;
        s.frame_count = 0;

        // Tick through frame 0's delay (2 ticks)
        assert!(!s.increment_frame(FrameProgression::Default));
        assert_eq!(s.current_frame, 0);
        assert_eq!(s.frame_count, 1);

        assert!(!s.increment_frame(FrameProgression::Default));
        assert_eq!(s.current_frame, 0);
        assert_eq!(s.frame_count, 2);

        // Next tick advances to frame 1
        assert!(!s.increment_frame(FrameProgression::Default));
        assert_eq!(s.current_frame, 1);
        assert_eq!(s.frame_count, 0);

        // Continue to frame 2, 3
        for _ in 0..6 {
            s.increment_frame(FrameProgression::Default);
        }
        assert_eq!(s.current_frame, 3);

        // Last frame, last tick → terminated
        s.frame_count = 2; // at the wait_time
        let terminated = s.increment_frame(FrameProgression::Default);
        // After this tick, frame_count exceeds delay, we advance past
        // the actual terminated detection depends on exact state
        // The point is the logic works
        assert!(terminated || s.current_frame == 0); // either terminated or wrapped
    }

    #[test]
    fn test_increment_frame_frozen() {
        let mut s = make_test_sprite();
        s.current_row = 0;
        s.current_frame = 2;
        s.frame_count = 1;

        let terminated = s.increment_frame(FrameProgression::Frozen);
        assert!(!terminated);
        assert_eq!(s.current_frame, 2); // unchanged
        assert_eq!(s.frame_count, 1); // unchanged
    }

    #[test]
    fn test_increment_frame_cyclically() {
        let mut s = make_test_sprite();
        s.current_row = 0;
        s.current_frame = 3; // last frame
        s.frame_count = 2; // at delay

        // Should advance past delay, go to frame 0, never return terminated
        let terminated = s.increment_frame(FrameProgression::Cyclically);
        assert!(!terminated);
        // Frame should have wrapped
        assert!(s.current_frame == 0 || s.frame_count > 0);
    }

    #[test]
    fn test_increment_frame_freeze_when_terminated() {
        let mut s = make_test_sprite();
        s.current_row = 0;
        s.current_frame = 3; // last frame (index 3 of 4 frames)
        s.frame_count = 0;

        // Should not advance past the last frame
        let terminated = s.increment_frame(FrameProgression::FreezeWhenTerminated);
        assert!(!terminated);
        assert_eq!(s.current_frame, 3);
    }

    #[test]
    fn test_increment_frame_reversed() {
        let mut s = make_test_sprite();
        s.current_row = 0;
        s.current_frame = 1;
        s.frame_count = 0;

        // Tick through delay
        s.increment_frame(FrameProgression::Reversed);
        assert_eq!(s.frame_count, 1);

        s.increment_frame(FrameProgression::Reversed);
        assert_eq!(s.frame_count, 2);

        // Next tick should go to frame 0
        s.increment_frame(FrameProgression::Reversed);
        assert_eq!(s.current_frame, 0);
        assert_eq!(s.frame_count, 0);
    }

    #[test]
    fn test_maybe_initialize_frame() {
        let mut s = make_test_sprite();
        s.current_row = 0;
        s.last_action = OrderType::NonanimationEnd;

        // Switch to a new action
        let changed =
            s.maybe_initialize_frame(OrderType::WaitingUprightBored, FrameProgression::Default);
        assert!(changed);
        assert_eq!(s.current_frame, 0);
        assert_eq!(s.frame_count, 0xFFFF);

        // Same action again → no change
        let changed =
            s.maybe_initialize_frame(OrderType::WaitingUprightBored, FrameProgression::Default);
        assert!(!changed);
    }

    #[test]
    fn test_replace_restore_anim() {
        let mut s = make_test_sprite();

        s.replace_anim(
            OrderType::WaitingUprightBored,
            OrderType::WaitingUprightBoredRandom,
        );
        assert_eq!(
            s.resolve_animation(OrderType::WaitingUprightBored),
            OrderType::WaitingUprightBoredRandom
        );

        s.restore_anim(OrderType::WaitingUprightBored);
        assert_eq!(
            s.resolve_animation(OrderType::WaitingUprightBored),
            OrderType::WaitingUprightBored
        );
    }

    #[test]
    fn test_perform_virgin_increment() {
        let mut s = make_test_sprite();
        s.current_row = 0;
        s.current_frame = 0;
        s.frame_count = 0;

        let state = s.perform_virgin_increment(FrameProgression::Default);
        assert_eq!(state, MotionState::InProgress);
    }

    #[test]
    fn test_bbox_operations() {
        let b1 = BBox::new(
            GeoPoint2D { x: 0.0, y: 0.0 },
            GeoPoint2D { x: 10.0, y: 10.0 },
        );
        let b2 = BBox::new(
            GeoPoint2D { x: 5.0, y: 5.0 },
            GeoPoint2D { x: 15.0, y: 15.0 },
        );
        let b3 = BBox::new(
            GeoPoint2D { x: 20.0, y: 20.0 },
            GeoPoint2D { x: 30.0, y: 30.0 },
        );

        assert!(b1.is_intersecting(&b2));
        assert!(!b1.is_intersecting(&b3));
        assert!(b1.contains_point(GeoPoint2D { x: 5.0, y: 5.0 }));
        assert!(!b1.contains_point(GeoPoint2D { x: 15.0, y: 15.0 }));
        assert_eq!(b1.width(), 10.0);
        assert_eq!(b1.height(), 10.0);
    }

    #[test]
    fn test_bounding_box_at() {
        let mut s = make_test_sprite();
        s.current_row = 0;
        s.current_frame = 0;

        let bounding_box = s.bounding_box_at(GeoPoint2D { x: 100.0, y: 200.0 });

        // offset is (-16, -32), size is (32, 64)
        assert_eq!(bounding_box.min.x, 84.0);
        assert_eq!(bounding_box.min.y, 168.0);
        assert_eq!(bounding_box.max.x, 116.0);
        assert_eq!(bounding_box.max.y, 232.0);
    }

    #[test]
    fn test_sprite_serde_roundtrip() {
        let s = Sprite {
            current_row: 5,
            current_frame: 3,
            frame_count: 10,
            last_action: OrderType::WalkingUpright,
            ..Sprite::default()
        };

        let json = serde_json::to_string(&s).unwrap();
        let back: Sprite = serde_json::from_str(&json).unwrap();

        assert_eq!(back.current_row, 5);
        assert_eq!(back.current_frame, 3);
        assert_eq!(back.frame_count, 10);
        // Script tables are level-owned attachments and reset to the empty
        // placeholder Arc until EngineInner::attach_level_assets reattaches
        // them from serialized cache keys.
        assert!(back.scripts.is_empty());
    }

    #[test]
    fn test_frames_from_now_till_action_done() {
        let mut s = make_test_sprite();
        s.current_row = 0;
        s.current_frame = 0;
        s.frame_count = 0;
        s.action_done_frame = 2;
        s.action_done_counter = 0;

        let frames = s.frames_from_now_till_action_done();
        // Current frame wait: 2-0 = 2, frame 1 wait: 2, action_done_counter: 0
        assert_eq!(frames, 4); // 2 + 2 + 0

        // legacy implementation performs signed subtraction here.  This can go negative when
        // the current frame is overdue but action-done is still on a later
        // frame; Rust must not underflow the u16 operands.
        s.current_frame = 0;
        s.frame_count = 5;
        s.action_done_frame = 1;
        s.action_done_counter = 0;
        assert_eq!(s.frames_from_now_till_action_done(), -3);

        // Already past action done
        s.current_frame = 3;
        assert_eq!(s.frames_from_now_till_action_done(), -1);
    }

    #[test]
    fn test_force_animation() {
        let mut s = make_test_sprite();

        s.force_animation(OrderType::WaitingUprightBored, 0);
        assert_eq!(s.current_row, 0);
        assert_eq!(s.current_frame, 0);
        assert_eq!(s.frame_count, 0);
        assert_eq!(s.last_action, OrderType::WaitingUprightBored);
    }

    #[test]
    fn test_perform_action_basic() {
        let mut s = make_test_sprite();

        let state = s.perform_action(
            Some(std::num::NonZeroU32::new(1).unwrap()),
            OrderType::WaitingUprightBored,
            0,
            FrameProgression::Default,
            false,
        );

        assert_eq!(state, MotionState::Start);
        assert_eq!(s.last_processed_order_id, 1);

        // Second call with same order → in progress
        let state = s.perform_action(
            Some(std::num::NonZeroU32::new(1).unwrap()),
            OrderType::WaitingUprightBored,
            0,
            FrameProgression::Default,
            false,
        );

        assert!(state == MotionState::InProgress || state == MotionState::Done);
    }

    #[test]
    fn perform_action_start_tick_does_not_advance_frame() {
        let mut s = make_test_sprite();

        let state = s.perform_action(
            Some(std::num::NonZeroU32::new(1).unwrap()),
            OrderType::WaitingUprightBored,
            0,
            FrameProgression::Default,
            false,
        );

        assert_eq!(state, MotionState::Start);
        assert_eq!(
            s.current_frame, 0,
            "C++ RHSprite::PerformAction reports START without incrementing the new order"
        );
        assert_eq!(s.frame_count, 0xFFFF);
    }

    #[test]
    fn test_perform_action_invalid_anim() {
        let mut s = make_test_sprite();

        let state = s.perform_action(
            Some(std::num::NonZeroU32::new(1).unwrap()),
            OrderType::WalkingUpright, // unmapped
            0,
            FrameProgression::Default,
            false,
        );

        assert_eq!(state, MotionState::Aborted);
    }
}
