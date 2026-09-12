//! Non-actor object data and local behavior.
use super::*;

/// FX-level data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct FxData {
    pub restore_background: bool,
    pub force_display: bool,
    pub animation: Animation,
    /// Masking polyline for display order interleaving.  FX entities
    /// with a non-empty polyline participate in the animation merge
    /// pass of display sorting (characters can walk behind them).
    pub display_polyline: Vec<MapPoint>,
    /// If this FX entity is a patch's animation element, the patch
    /// index (into the canonical interactable patch table). Used by the animation tick
    /// to apply reversed playback and detect transition completion.
    pub patch_index: Option<crate::patch::PatchIndex>,
    /// Index of the owning mission mobile element for its masked child
    /// sprite. `None` for ordinary proto/patch FX.
    #[serde(default)]
    pub mobile_index: Option<u16>,
    /// Original-game masked-FX animation speed. Mobile carts update this
    /// to `1 / movement_speed`; ordinary FX retain `1.0`.
    #[serde(default = "default_fx_animation_speed")]
    pub animation_speed: f32,
    /// Rendering properties (`Blocky` vs `NeedShadow`) selected from
    /// the blit-type byte at level load.  `NeedShadow` selects the
    /// alpha-keyed drawing path (sprite gets the global shadow tint
    /// composited on); `Blocky` selects plain `Blit` with no shadow
    /// compositing.
    pub rendering_properties: RenderingProperties,
}

const fn default_fx_animation_speed() -> f32 {
    1.0
}

impl Default for FxData {
    fn default() -> Self {
        Self {
            restore_background: false,
            force_display: false,
            animation: Animation::default(),
            display_polyline: Vec::new(),
            patch_index: None,
            mobile_index: None,
            animation_speed: 1.0,
            rendering_properties: RenderingProperties::default(),
        }
    }
}

/// Target-level data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct TargetData {
    pub animation: Animation,
    /// Raw frame-progression ordinal from level data.
    pub progression: u32,
    pub linked_fx: Vec<EntityId>,
    /// Action filter flags determining which PC actions interact with this target.
    /// See `TargetFilter` bitflags.
    pub action_filter: TargetFilter,
    /// Action point — the position the PC walks to when interacting
    /// with this target.
    pub action_position: MapPoint,
    /// Action point sector.
    pub action_sector: u16,
    /// Action point layer.
    pub action_layer: u16,
    /// Optional Z height for elevated targets (negative = no Z).
    pub position_z: i16,
    /// Sprite filename (.rhs file).
    pub sprite_filename: String,
    /// Sprite profile name within the .rhs.
    pub sprite_profile_name: String,
    /// Masking polyline used for blit clipping.
    pub display_polyline: Vec<MapPoint>,
    /// Rendering properties (`Blocky` vs `NeedShadow`) selected from the
    /// blit type byte in the level data.
    pub rendering_properties: RenderingProperties,
    /// Script class name driving this target's per-instance script
    /// (`IElementTargetScript` implementation).  Loaded from the proto
    /// stream's target script-class field.  Each target carries its
    /// own VM, with named functions like `ActivatedByListenable`,
    /// `ActivatedByApple`, etc.  Empty string means "no script
    /// attached" (the dispatcher skips such targets).
    pub script_class: String,
}

impl Default for TargetData {
    fn default() -> Self {
        Self {
            // Initial animation is `WaitingUpright`.
            // `Animation::default()` would otherwise give ordinal 0
            // (`WaitingUprightBored`), which is a different pose.
            animation: Animation::WaitingUpright,
            progression: 0,
            linked_fx: Vec::new(),
            action_filter: TargetFilter::empty(),
            action_position: MapPoint::default(),
            action_sector: 0,
            action_layer: 0,
            position_z: -1,
            sprite_filename: String::new(),
            sprite_profile_name: String::new(),
            display_polyline: Vec::new(),
            rendering_properties: RenderingProperties::Blocky,
            script_class: String::new(),
        }
    }
}

/// Object-level data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ObjectData {
    pub associated_action: Action,
    pub terminate: bool,
    pub quantity: u16,
    pub object_type: ObjectType,
    pub animation: Animation,
    pub reference: Option<EntityId>,
    pub belongs_to_beggar: bool,
    pub taken: bool,
}

impl Default for ObjectData {
    fn default() -> Self {
        Self {
            associated_action: Action::default(),
            terminate: false,
            quantity: 1,
            object_type: ObjectType::default(),
            // Initial animation is `WaitingUpright`, not the enum-zero
            // `WaitingUprightBored`.
            animation: Animation::WaitingUpright,
            reference: None,
            belongs_to_beggar: false,
            taken: false,
        }
    }
}

/// A single waypoint on a precomputed ballistic trajectory.
///
/// The projectile moves linearly from its current position to
/// `position` over `time` frames before popping the next point from
/// the trajectory list.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct TrajectoryPoint {
    pub position: WorldPoint3D,
    /// Number of frames to reach this point from the previous position.
    pub time: u16,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct TrajectoryPointRuntime {
    pub bounce: bool,
    pub material: u32,
}

/// Projectile-level data.
#[derive(
    Default,
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ProjectileData {
    /// 3D launch point.  Read by the trajectory-arc debug overlay
    /// (`game_render::draw_trajectories`) to render from the launch
    /// origin rather than the current frame.
    pub start: WorldPoint3D,
    pub end: WorldPoint3D,
    /// X of the launch point.  Read by `EventGetArrow` (arrow hit) and
    /// `EventApple` (apple hit) so the AI stimulus anchors at the
    /// shooter's *original* position rather than the impact site.
    pub start_of_trajectory_x: f32,
    /// Y of the launch point.  Paired with `start_of_trajectory_x`.
    pub start_of_trajectory_y: f32,
    pub shooter: Option<EntityId>,
    pub frame_count: u16,
    pub flying: bool,
    /// One-shot latch for a ground-targeted stone. The first terminal impact
    /// broadcasts [`crate::ai::NoiseType::Distraction`] and clears this bit.
    #[serde(default)]
    pub noise_distraction: bool,
    /// Original-game water/hole landing latch.
    #[serde(default)]
    pub dive: bool,
    /// Bloodseeker-oil straight-flight latch.
    #[serde(default)]
    pub magic_bullet: bool,
    pub disappear: bool,
    /// Precomputed trajectory waypoints.  `tick_arrows` pops points
    /// from the front and interpolates position toward each one over
    /// `time` frames.
    pub trajectory: Vec<TrajectoryPoint>,
    /// Serialized metadata parallel to `trajectory`. Empty for a freshly
    /// generated trajectory until its collision metadata is materialized.
    #[serde(default)]
    pub trajectory_runtime: Vec<TrajectoryPointRuntime>,
    /// Constructor-only latch: generated trajectory runtime still needs its
    /// exact collision waypoint materialized before the first update.
    /// Cleared before publication by the purse/coin constructor boundary.
    #[serde(default)]
    pub terminal_material_pending: bool,
    /// Exact raw collision waypoint retained by trajectory calculation until the
    /// pre-publication update can determine its material.
    #[serde(default)]
    pub terminal_material_impact_index: Option<u16>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub trajectory_origin_sector: Option<u16>,
    /// Exact arena half of the original game's trajectory-start sector.
    /// The public number above remains for backward-compatible serialized
    /// state, but AI projectile-hit callbacks must copy the complete sector
    /// pointer identity into their stimulus position.
    #[serde(deserialize_with = "Option::deserialize")]
    pub trajectory_origin_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub trajectory_origin_layer: Option<crate::position_interface::Layer>,
    /// Per-frame position delta for the current trajectory segment.
    /// Recomputed each time a new waypoint is popped.
    pub velocity_increment: WorldVec3D,
    /// Original-game flight direction, sampled from the initial trajectory
    /// velocity. This is gameplay state used to orient hit actors; it is
    /// deliberately separate from the projectile element's sprite facing.
    #[serde(default)]
    pub flight_direction: u16,
    /// Optional explicit start point for the next collision segment.
    /// Used when a projectile is advanced once before entering the
    /// engine list, matching the original game's spawn-time advance.
    pub launch_segment_start: Option<WorldPoint3D>,
    /// Frames remaining in the current trajectory segment.
    /// When this reaches 0, the next `TrajectoryPoint` is popped.
    pub trajectory_frame_count: u16,
    /// Precomputed damage for this projectile.  Set at spawn time from the
    /// shooter's bow profile via `BowState::get_damage()`.  Applied on hit
    /// by `apply_arrow_hit`.
    pub damage: u16,
    /// True when the arrow has been deflected (by a shield or target) and
    /// is falling to the ground.  Falling arrows skip shield and victim
    /// collision checks.
    pub falling: bool,
    /// Last non-falling horizontal orientation computed by
    /// arrow refresh. The original game serializes this cache and reuses it
    /// when the trajectory has become empty.
    #[serde(default)]
    pub last_orientation_sector: u16,
    /// Last non-falling vertical orientation, in degrees in `[-60, 60]`.
    #[serde(default)]
    pub last_orientation_azimuth: i16,
    /// Sector (0..15) used by a falling arrow's visual rotation. Cycled by
    /// the deferred arrow-refresh boundary, not by the element tick.
    pub falling_direction: u16,
    /// Arrow-only serialized leaf state. `arrow_bow_profile` distinguishes a
    /// null bow (`None`) from a present default bow (`Some(None)`).
    #[serde(default)]
    pub arrow_bow_profile: Option<Option<u32>>,
    #[serde(default)]
    pub arrow_flat_shot: bool,
    #[serde(default)]
    pub arrow_play_impact: bool,
    /// Purse / coin back-pointers — populated for `ObjectType::Purse`
    /// and `ObjectType::Coin` projectiles, default for everything else.
    /// See [`PurseData`].
    pub purse: PurseData,
    /// Wasp-nest / wasp state — populated for
    /// `ObjectType::BonusWaspNest` / `ObjectType::WaspNest` parents and
    /// each `ObjectType::Wasp` child, default for everything else.
    /// See [`WaspData`].
    pub wasp: WaspData,
}

/// Net-specific data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct NetData {
    pub victims: Vec<EntityId>,
    /// Pre-landing countdown (frames). Set at spawn time to
    /// `total_trajectory_frames - 15`.  Decrements during flight;
    /// when it reaches 0 the net switches its sprite animation to
    /// `NetUnfolding` (or `NetUnfoldingCrumpled`).
    pub time_till_unfolding: u32,
    pub crumpled: bool,
    pub was_flying: bool,
    /// IDs of the (up to two) `RepulsivePoint`s registered on
    /// `AiGlobalState` while the net sits on the ground.  Cleared on
    /// despawn.
    pub repulsive_point_ids: Vec<i32>,
    /// True once the post-landing animation transition has fired
    /// (NetUnfolding → ObjectLying / NetMoving, or
    /// NetUnfoldingCrumpled → NetLyingCrumpled). Prevents repeating
    /// the transition each frame.
    pub landed_animation_resolved: bool,
}

/// Purse / coin back-pointers.
///
/// A single struct serves both roles since purses and coins are
/// sibling kinds of projectile: purses populate `child_coins` /
/// `number_of_coins` and leave `source_purse` empty; coins populate
/// `source_purse` and leave the other fields empty.  Lives on
/// [`ProjectileData`] so it travels with the existing
/// `Entity::Projectile(ElementProjectile)` payload — no extra entity
/// variant needed.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PurseData {
    /// On a coin: handle of the purse it was ejected from.  On a
    /// purse: always `None`.
    pub source_purse: Option<EntityId>,
    /// On a purse: list of child coins spawned by obstacle impact. On
    /// a coin: empty.
    pub child_coins: Vec<EntityId>,
    /// On a purse: number of coins still owed (decremented each burst).
    /// Initialised to `inventory::COINS_PER_PURSE` at spawn.  On a
    /// coin: always 0.
    pub number_of_coins: u16,
    /// On a purse: true once obstacle impact has occurred and child coins
    /// are in flight. Used by the update to know when to switch to
    /// the "drain children, then despawn" idle phase.
    pub burst: bool,
    /// On a coin: layer the coin should snap to on landing.  Stored
    /// at spawn so obstacle impact can re-key the coin onto its goal
    /// layer. On a purse: `None` because the field is not applicable.
    #[serde(deserialize_with = "Option::deserialize")]
    pub layer_goal: Option<crate::position_interface::Layer>,
    /// On a coin: sector the coin should snap to on landing (None
    /// when the scatter target wasn't resolved against a known
    /// sector).
    #[serde(deserialize_with = "Option::deserialize")]
    pub sector_goal: Option<crate::position_interface::SectorHandle>,
}

/// Wasp-nest / wasp shared fields.  Lives on [`ProjectileData`] so both
/// the wasp-nest parent (`ObjectType::BonusWaspNest`) and each spawned
/// wasp (`ObjectType::Wasp`) can share the storage without a separate
/// entity variant.
///
/// `flying_wasp_count` is decremented when a wasp dies and is checked
/// each tick to decide whether to keep emitting the buzz sound.
/// `source_nest` is the back-pointer so the wasp can notify its nest
/// on death.
///
/// Wasp AI (chase/sting soldiers) lives in `engine::wasp_nest`; when a
/// wasp dies it decrements the nest's counter through `source_nest`.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct WaspData {
    /// On a wasp nest: remaining wasps in flight.  Incremented to
    /// `NUMBER_OF_WASPS` on burst, decremented when each wasp dies.
    /// On a wasp: always 0.
    pub flying_wasp_count: u32,
    /// On a wasp nest: true once the nest has burst and the 20 wasps
    /// have been spawned, to prevent re-bursting each tick.  On a
    /// wasp: always false.
    pub burst: bool,
    /// On a wasp: handle of the parent nest (for decrementing
    /// `flying_wasp_count` on death).  On a wasp nest: always `None`.
    pub source_nest: Option<EntityId>,
    /// On a wasp: handle of the currently targeted soldier, or `None`.
    /// Cleared when the distance exceeds `VICTIM_FORGET_DISTANCE` or
    /// when the wasp dies.
    pub victim: Option<EntityId>,
    /// On a wasp: `true` once the wasp has closed to `STING_DISTANCE`
    /// and committed to stinging its victim; movement stops and the
    /// sting-timeout counter runs down.
    pub stinging: bool,
    /// On a wasp: frame counter until next direction change (or, while
    /// stinging, until the sting fires).
    pub timeout: u32,
    /// On a wasp: current per-frame movement vector (3D velocity).
    /// Current per-frame 3D vector in raw world axes.
    pub movement: WorldVec3D,
}

// ═══════════════════════════════════════════════════════════════════
//  Concrete entity structs
// ═══════════════════════════════════════════════════════════════════
