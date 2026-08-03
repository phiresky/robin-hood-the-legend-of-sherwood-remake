//! Sector identifiers, type flags, and the few specialised sector data
//! structs that survive on the production hot path.
//!
//! Most "sector" data at runtime lives on [`crate::fast_find_grid::GridSector`];
//! this module hosts only the typed helpers that the rest of the engine
//! references directly:
//!
//! * [`SectorType`] / [`SectorNumber`] — the type-flag bitset and id newtype
//!   threaded through every grid query.
//! * [`LiftType`] — lift sub-type + animation-translation logic + actor
//!   authorisation.
//! * [`BuildingIdx`] / [`ArcheryPointIdx`] — newtypes for the parallel
//!   canonical per-building / per-archery-point tables.
//! * [`OccupantKind`] — building-occupant classifier returned by AI helpers.
//! * [`ScriptSectorData`] — per-script-zone runtime state stored in
//!   `EngineInner::script_zone_data`.
//! * [`ShadowData`] — per-shadow-sector centroid + radius, keyed by sector
//!   index in `LevelGrid::shadow_data`.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::coordinates::MapPoint;
use crate::gate::ActorAuthInfo;
use crate::sector_production;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BuildingIdx — nominal newtype
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Index into the engine's building table.  Wraps [`nonmax::NonMaxU16`]
/// so `Option<BuildingIdx>` is 2 bytes via niche optimization.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct BuildingIdx(pub nonmax::NonMaxU16);

impl BuildingIdx {
    #[inline]
    pub fn new(v: u16) -> Option<Self> {
        nonmax::NonMaxU16::new(v).map(Self)
    }
    #[inline]
    pub fn get(self) -> u16 {
        self.0.get()
    }
}
impl From<BuildingIdx> for u16 {
    #[inline]
    fn from(i: BuildingIdx) -> u16 {
        i.0.get()
    }
}
impl From<BuildingIdx> for u32 {
    #[inline]
    fn from(i: BuildingIdx) -> u32 {
        u32::from(i.0.get())
    }
}
impl From<BuildingIdx> for usize {
    #[inline]
    fn from(i: BuildingIdx) -> usize {
        usize::from(i.0.get())
    }
}
impl std::fmt::Display for BuildingIdx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().fmt(f)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SectorNumber — nominal newtype
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Canonical sector identifier.
///
/// Stored as signed i16 on sectors and reinterpreted as u16 on door
/// records; both refer to the same id space.  Negative values are used
/// as "invalid" sentinels (`sector == -1` ⇒ "not found"), so we keep the
/// i16 representation and surface it with `From<SectorNumber> for u16` via
/// `as u16` (sign-preserving reinterpretation at door-comparison sites).
///
/// Distinct from [`crate::fast_find_grid::SectorIndex`] which is the
/// slot index into `FastFindGrid::level::sectors`.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct SectorNumber(pub i16);

impl SectorNumber {
    /// Wrap an i16 as a sector number.
    #[inline]
    pub const fn new(v: i16) -> Self {
        Self(v)
    }
    /// Raw signed value.
    #[inline]
    pub const fn get(self) -> i16 {
        self.0
    }
    /// True when the value is a valid (non-negative) id.  Negative
    /// sector numbers (especially `-1`) are used as "invalid" sentinels.
    #[inline]
    pub const fn is_valid(self) -> bool {
        self.0 >= 0
    }
}
impl From<i16> for SectorNumber {
    #[inline]
    fn from(v: i16) -> Self {
        Self(v)
    }
}
impl From<SectorNumber> for i16 {
    #[inline]
    fn from(n: SectorNumber) -> i16 {
        n.0
    }
}
impl From<SectorNumber> for u16 {
    #[inline]
    fn from(n: SectorNumber) -> u16 {
        // Sign-preserving reinterpretation at door-comparison sites
        // (`d.sector_in == sector_number as u16`).
        n.0 as u16
    }
}
impl PartialEq<i16> for SectorNumber {
    #[inline]
    fn eq(&self, other: &i16) -> bool {
        self.0 == *other
    }
}
impl PartialEq<SectorNumber> for i16 {
    #[inline]
    fn eq(&self, other: &SectorNumber) -> bool {
        *self == other.0
    }
}
impl PartialEq<u16> for SectorNumber {
    #[inline]
    fn eq(&self, other: &u16) -> bool {
        // Compare as bit patterns (door-site convention).
        (self.0 as u16) == *other
    }
}
impl PartialEq<SectorNumber> for u16 {
    #[inline]
    fn eq(&self, other: &SectorNumber) -> bool {
        *self == (other.0 as u16)
    }
}
// Handy for tests / i32 literals.  Compares by widening to i32 (the
// SectorNumber fits losslessly).
impl PartialEq<i32> for SectorNumber {
    #[inline]
    fn eq(&self, other: &i32) -> bool {
        i32::from(self.0) == *other
    }
}
impl PartialEq<SectorNumber> for i32 {
    #[inline]
    fn eq(&self, other: &SectorNumber) -> bool {
        *self == i32::from(other.0)
    }
}
// Increment by i16 literal — useful for the level-loading pass that
// walks motion areas and doors sequentially.
impl std::ops::AddAssign<i16> for SectorNumber {
    #[inline]
    fn add_assign(&mut self, rhs: i16) {
        self.0 += rhs;
    }
}
impl std::ops::Add<i16> for SectorNumber {
    type Output = SectorNumber;
    #[inline]
    fn add(self, rhs: i16) -> Self {
        Self(self.0 + rhs)
    }
}
impl std::fmt::Display for SectorNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SectorType bitflags
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct SectorType: u32 {
        const AREA       = 1;
        const MOTION     = 2;
        const PATCH      = 4;
        const SCRIPT     = 8;
        const SOUND      = 16;
        const PLANE      = 32;
        const MOUSE      = 64;
        const CROSS      = 128;
        const APPLY      = 256;
        const LIFT       = 512;
        const ASSOCIATED = 1024;
        const SURROUND   = 2048;
        const DOOR       = 4096;
        const BUILDING   = 8192;
        const SHADOW     = 16384;
        const JUMP       = 32768;
        const RAILROAD   = 65536;
        const APEX       = 131072;
    }
}

impl SectorType {
    pub fn is_area(self) -> bool {
        self.contains(Self::AREA)
    }
    /// An obstacle is any sector without the AREA flag.
    pub fn is_obstacle(self) -> bool {
        !self.contains(Self::AREA)
    }
    pub fn is_motion(self) -> bool {
        self.contains(Self::MOTION)
    }

    pub fn is_patch(self) -> bool {
        self.contains(Self::PATCH)
    }

    pub fn is_lift(self) -> bool {
        self.contains(Self::LIFT)
    }
    pub fn is_building(self) -> bool {
        self.contains(Self::BUILDING)
    }

    pub fn is_door(self) -> bool {
        self.contains(Self::DOOR)
    }
    pub fn is_shadow(self) -> bool {
        self.contains(Self::SHADOW)
    }

    pub fn is_jump(self) -> bool {
        self.contains(Self::JUMP)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// LiftType
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Lift sub-type.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum LiftType {
    #[default]
    Normal,
    Stairs,
    Ladder,
    Wall,
}

impl LiftType {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Normal,
            1 => Self::Stairs,
            2 => Self::Ladder,
            3 => Self::Wall,
            _ => panic!("unknown lift type: {v}"),
        }
    }

    /// Wall and ladder lifts restrict who can traverse them.
    pub fn is_wall_or_ladder(self) -> bool {
        matches!(self, Self::Wall | Self::Ladder)
    }

    /// Whether an actor is authorised to traverse a lift of this type.
    ///
    /// - **Wall**: only PCs with the climb action.
    /// - **Ladder**: humans who are not civilians.
    /// - **Stairs**: all humans + objects (no animals ship in the game).
    /// - **Normal**: everyone.
    pub fn is_actor_authorized(self, actor: &ActorAuthInfo) -> bool {
        match self {
            LiftType::Wall => actor.kind.is_pc() && actor.has_climb,
            LiftType::Ladder => actor.kind.is_human() && !actor.kind.is_civilian(),
            LiftType::Stairs => true,
            LiftType::Normal => true,
        }
    }

    /// Lift translation for an actor in upright posture moving inside this
    /// lift sector.  Upright posture has upwards == downwards, so the
    /// upwards mapping is used unconditionally.
    pub fn translate_upright_action(
        self,
        action: crate::order::OrderType,
    ) -> crate::order::OrderType {
        // Upright posture has upwards == downwards; pick the upwards mapping.
        self.translate_climb_action(action, false)
    }

    /// Lift translation for an actor on a ladder or wall (`Posture::OnLadder`
    /// / `Posture::OnWall`).  When `going_down` is true the actor is moving
    /// in the `low - high` direction and gets the downwards animation;
    /// otherwise the upwards animation.  Determined by dot-producting the
    /// ladder vector with the movement vector to pick downwards vs upwards.
    ///
    /// Stairs / Normal lift types reach this only via the upright path
    /// (their up/down mappings are identical).
    pub fn translate_climb_action(
        self,
        action: crate::order::OrderType,
        going_down: bool,
    ) -> crate::order::OrderType {
        use crate::order::OrderType as A;
        match self {
            LiftType::Normal => {
                // Normal: up/down both return the same.
                if action == A::RunningUpright {
                    A::RunningStairs
                } else {
                    A::WalkingUpright
                }
            }
            LiftType::Stairs => {
                if action == A::RunningUpright {
                    action
                } else {
                    A::WalkingStairs
                }
            }
            LiftType::Ladder => {
                if going_down {
                    match action {
                        A::RunningUpright => A::ClimbingLadderDownFast,
                        A::WalkingAlerted => A::ClimbingLadderDownAlerted,
                        _ => A::ClimbingLadderDown,
                    }
                } else {
                    match action {
                        A::RunningUpright => A::ClimbingLadderUpFast,
                        A::WalkingAlerted => A::ClimbingLadderUpAlerted,
                        _ => A::ClimbingLadderUp,
                    }
                }
            }
            LiftType::Wall => {
                if going_down {
                    if action == A::RunningUpright {
                        A::ClimbingWallDownFast
                    } else {
                        A::ClimbingWallDown
                    }
                } else if action == A::RunningUpright {
                    A::ClimbingWallUpFast
                } else {
                    A::ClimbingWallUp
                }
            }
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// OccupantKind
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Classification of NPC occupants in a building sector.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum OccupantKind {
    Villains,
    Civilians,
    Empty,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ArcheryPointIdx
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Index of a waypoint within an archery sector's `points` Vec.
///
/// Plain `u16` newtype with `Default = 0` (the first waypoint is a
/// valid index).  Sentinel fields like `index_first_shooting_point`
/// use `Option<ArcheryPointIdx>` with `None` instead of `u16::MAX`.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct ArcheryPointIdx(pub u16);

impl From<ArcheryPointIdx> for u16 {
    #[inline]
    fn from(i: ArcheryPointIdx) -> u16 {
        i.0
    }
}
impl From<ArcheryPointIdx> for u32 {
    #[inline]
    fn from(i: ArcheryPointIdx) -> u32 {
        u32::from(i.0)
    }
}
impl From<ArcheryPointIdx> for usize {
    #[inline]
    fn from(i: ArcheryPointIdx) -> usize {
        usize::from(i.0)
    }
}
impl From<u16> for ArcheryPointIdx {
    #[inline]
    fn from(v: u16) -> Self {
        Self(v)
    }
}
impl std::fmt::Display for ArcheryPointIdx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ScriptSectorData
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Script-triggered zone sector with enter/leave events.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ScriptSectorData {
    /// Sector index within the fast-find grid.  `None` until the
    /// owning sector is registered with the grid.
    pub sector_index: Option<crate::fast_find_grid::SectorIndex>,

    /// Whether this sector has a script class bound to it.
    pub script_associated: bool,

    /// Script class name (if `script_associated`).
    pub script_class_name: Option<String>,

    /// Production type for production sectors (UNKNOWN if not a production sector).
    pub production_sector_type: sector_production::Type,

    /// Maximum throwing apex height (only valid when sector_type contains APEX).
    pub max_throwing_apex_height: f32,

    /// `true` once `DefineFlatTrajectoryZone` has converted this script
    /// sector into an apex sector.  The conversion drops the SCRIPT|CROSS
    /// bits and rewrites the type to APEX, so the engine then stops
    /// scanning the sector for occupant transitions.  Our overlay can only
    /// OR bits, so we track the conversion explicitly and skip the zone
    /// in `tick_zone_occupants`.
    pub transformed_to_apex: bool,

    // ── Serialized (save-game state) ──
    /// Entities currently inside this script zone.
    pub occupant_indices: Vec<crate::entity_id::EntityId>,
}

impl Default for ScriptSectorData {
    fn default() -> Self {
        Self {
            sector_index: None,
            script_associated: false,
            script_class_name: None,
            production_sector_type: sector_production::Type::Unknown,
            max_throwing_apex_height: 0.0,
            transformed_to_apex: false,
            occupant_indices: Vec::new(),
        }
    }
}

impl ScriptSectorData {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_inside(&self, element_index: crate::entity_id::EntityId) -> bool {
        self.occupant_indices.contains(&element_index)
    }

    /// Populate an occupant during the engine's bulk zone scan.
    ///
    /// Original `RHSectorScript::AddOccupant` appends without firing a
    /// script callback; runtime boundary crossings use [`Self::enter`]
    /// instead and insert at the front.
    pub fn add_occupant(&mut self, element_index: crate::entity_id::EntityId) {
        self.occupant_indices.push(element_index);
    }

    pub fn enter(&mut self, element_index: crate::entity_id::EntityId) {
        if self.occupant_indices.contains(&element_index) {
            tracing::warn!(
                "actor {element_index} entering script sector {:?} twice",
                self.sector_index
            );
            return;
        }
        // Insert at the front.
        self.occupant_indices.insert(0, element_index);
    }

    pub fn leave(&mut self, element_index: crate::entity_id::EntityId) {
        if let Some(pos) = self
            .occupant_indices
            .iter()
            .position(|&i| i == element_index)
        {
            self.occupant_indices.remove(pos);
        } else {
            tracing::warn!(
                "actor {element_index} leaving script sector {:?} it hadn't entered",
                self.sector_index
            );
        }
    }

    pub fn remove_all_occupants(&mut self) {
        self.occupant_indices.clear();
    }

    pub fn num_occupants(&self) -> usize {
        self.occupant_indices.len()
    }

    pub fn get_occupant(&self, index: usize) -> crate::entity_id::EntityId {
        self.occupant_indices[index]
    }

    /// Transform this script sector into an apex sector.
    /// The sector_type on the parent `Sector` must also be updated to `APEX`.
    pub fn transform_into_apex(&mut self, apex_height: f32) {
        assert!(
            !self.script_associated,
            "cannot convert a script-associated sector to apex"
        );
        self.max_throwing_apex_height = apex_height;
        // Sector is no longer a script zone — see field doc.
        self.transformed_to_apex = true;
        // Drop any cached occupants; the occupant list isn't maintained
        // once the sector becomes APEX.
        self.occupant_indices.clear();
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ShadowData
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Shadow sector with computed barycentre and average radius.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ShadowData {
    /// 2D barycentre (centroid) of the shadow polygon.
    pub barycentre_2d: MapPoint,

    /// 3D barycentre — includes Z from the height plane at the centroid.
    pub barycentre_3d_x: f32,
    pub barycentre_3d_y: f32,
    pub barycentre_3d_z: f32,

    /// Average distance from centroid to vertices.
    pub radius: f32,
}

impl Default for ShadowData {
    fn default() -> Self {
        Self {
            barycentre_2d: MapPoint::ZERO,
            barycentre_3d_x: 0.0,
            barycentre_3d_y: 0.0,
            barycentre_3d_z: 0.0,
            radius: 0.0,
        }
    }
}

impl ShadowData {
    /// Compute the 2D barycentre and average radius from the parent sector's
    /// polygon points.
    ///
    /// Note: the 3D barycentre requires height plane lookup and must be
    /// computed separately (see `initialize_3d`).
    pub fn initialize_2d(&mut self, points: &[MapPoint]) {
        assert!(!points.is_empty(), "shadow sector has no points");
        let n = points.len() as f32;

        let mut cx = 0.0_f32;
        let mut cy = 0.0_f32;
        for p in points {
            cx += p.x;
            cy += p.y;
        }
        let inv_n = 1.0 / n;
        cx *= inv_n;
        cy *= inv_n;
        self.barycentre_2d = MapPoint::new(cx, cy);

        let mut total_dist = 0.0_f32;
        for p in points {
            let dx = p.x - cx;
            let dy = p.y - cy;
            total_dist += (dx * dx + dy * dy).sqrt();
        }
        self.radius = total_dist * inv_n;
    }

    /// Compute the 3D barycentre given the top plane of the plane sector
    /// that contains the shadow's 2D barycentre.  `top_plane_points` comes
    /// from the `SightObstacle` of the enclosing plane sector; the caller
    /// is responsible for looking it up via the fast-find grid.  Pass
    /// `None` when no plane sector encloses the barycentre — we fall back
    /// to a flat `z=0`.
    ///
    /// The 2D and 3D passes are split so the engine can drive the plane
    /// lookup.
    pub fn initialize_3d(&mut self, top_plane_points: Option<&[[f32; 3]; 3]>) {
        let bx = self.barycentre_2d.x;
        let by = self.barycentre_2d.y;
        match top_plane_points {
            None => {
                self.barycentre_3d_x = bx;
                self.barycentre_3d_y = by;
                self.barycentre_3d_z = 0.0;
            }
            Some(points) => {
                // Use the shared plane-coefficient derivation so the shadow
                // centroid sees bit-identical `az`/`bz`/`dz` to every other
                // consumer of a top plane. Reducing the ratios algebraically
                // here instead moved the centroid Z by a few ULP, which is
                // enough to change a visibility ray's endpoint.
                let coeffs = crate::position_interface::PlaneZCoeffs::from_plane_points(points);
                let z = 0.1 + coeffs.compute_z(bx, by);
                self.barycentre_3d_z = z;
                // Iso projection: screen-space Y is world Y + Z.
                self.barycentre_3d_y = by + z;
                self.barycentre_3d_x = bx;
            }
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ElementKind;

    #[test]
    fn sector_type_bitflags() {
        let t = SectorType::MOTION | SectorType::AREA;
        assert!(t.is_motion());
        assert!(t.is_area());
        assert!(!t.is_obstacle());
        assert!(!t.is_lift());

        let t2 = t | SectorType::LIFT;
        assert!(t2.is_lift());
        assert!(t2.is_motion());
        assert!(t2.is_area());
    }

    #[test]
    fn lift_type_from_u8() {
        assert_eq!(LiftType::from_u8(0), LiftType::Normal);
        assert_eq!(LiftType::from_u8(1), LiftType::Stairs);
        assert_eq!(LiftType::from_u8(2), LiftType::Ladder);
        assert_eq!(LiftType::from_u8(3), LiftType::Wall);

        assert!(LiftType::Wall.is_wall_or_ladder());
        assert!(LiftType::Ladder.is_wall_or_ladder());
        assert!(!LiftType::Stairs.is_wall_or_ladder());
    }

    #[test]
    #[should_panic(expected = "unknown lift type")]
    fn lift_type_from_u8_invalid() {
        LiftType::from_u8(99);
    }

    #[test]
    fn lift_type_translate_upright_action_stairs() {
        use crate::order::OrderType as A;
        // Stairs: walking → walking_stairs, running stays running.
        assert_eq!(
            LiftType::Stairs.translate_upright_action(A::WalkingUpright),
            A::WalkingStairs
        );
        assert_eq!(
            LiftType::Stairs.translate_upright_action(A::RunningUpright),
            A::RunningUpright
        );
    }

    #[test]
    fn lift_type_translate_upright_action_ladder_and_wall() {
        use crate::order::OrderType as A;
        assert_eq!(
            LiftType::Ladder.translate_upright_action(A::WalkingUpright),
            A::ClimbingLadderUp
        );
        assert_eq!(
            LiftType::Ladder.translate_upright_action(A::RunningUpright),
            A::ClimbingLadderUpFast
        );
        assert_eq!(
            LiftType::Ladder.translate_upright_action(A::WalkingAlerted),
            A::ClimbingLadderUpAlerted
        );
        assert_eq!(
            LiftType::Wall.translate_upright_action(A::WalkingUpright),
            A::ClimbingWallUp
        );
        assert_eq!(
            LiftType::Wall.translate_upright_action(A::RunningUpright),
            A::ClimbingWallUpFast
        );
    }

    #[test]
    fn lift_type_translate_upright_action_normal() {
        use crate::order::OrderType as A;
        // Normal lift: walking stays walking, running becomes running_stairs.
        assert_eq!(
            LiftType::Normal.translate_upright_action(A::WalkingUpright),
            A::WalkingUpright
        );
        assert_eq!(
            LiftType::Normal.translate_upright_action(A::RunningUpright),
            A::RunningStairs
        );
    }

    #[test]
    fn lift_type_translate_climb_action_ladder_directional() {
        use crate::order::OrderType as A;
        // Going down a ladder: walking → ClimbingLadderDown,
        // running → ClimbingLadderDownFast, alerted → ClimbingLadderDownAlerted.
        assert_eq!(
            LiftType::Ladder.translate_climb_action(A::WalkingUpright, true),
            A::ClimbingLadderDown
        );
        assert_eq!(
            LiftType::Ladder.translate_climb_action(A::RunningUpright, true),
            A::ClimbingLadderDownFast
        );
        assert_eq!(
            LiftType::Ladder.translate_climb_action(A::WalkingAlerted, true),
            A::ClimbingLadderDownAlerted
        );
        // Going up: same as upright translation.
        assert_eq!(
            LiftType::Ladder.translate_climb_action(A::WalkingUpright, false),
            A::ClimbingLadderUp
        );
        assert_eq!(
            LiftType::Ladder.translate_climb_action(A::RunningUpright, false),
            A::ClimbingLadderUpFast
        );
    }

    #[test]
    fn lift_type_translate_climb_action_wall_directional() {
        use crate::order::OrderType as A;
        assert_eq!(
            LiftType::Wall.translate_climb_action(A::WalkingUpright, true),
            A::ClimbingWallDown
        );
        assert_eq!(
            LiftType::Wall.translate_climb_action(A::RunningUpright, true),
            A::ClimbingWallDownFast
        );
        assert_eq!(
            LiftType::Wall.translate_climb_action(A::WalkingUpright, false),
            A::ClimbingWallUp
        );
        assert_eq!(
            LiftType::Wall.translate_climb_action(A::RunningUpright, false),
            A::ClimbingWallUpFast
        );
    }

    #[test]
    fn lift_type_translate_climb_action_stairs_normal_symmetric() {
        use crate::order::OrderType as A;
        // Stairs / Normal: up == down.  Direction is ignored.
        for going_down in [false, true] {
            assert_eq!(
                LiftType::Stairs.translate_climb_action(A::WalkingUpright, going_down),
                A::WalkingStairs
            );
            assert_eq!(
                LiftType::Stairs.translate_climb_action(A::RunningUpright, going_down),
                A::RunningUpright
            );
            assert_eq!(
                LiftType::Normal.translate_climb_action(A::WalkingUpright, going_down),
                A::WalkingUpright
            );
            assert_eq!(
                LiftType::Normal.translate_climb_action(A::RunningUpright, going_down),
                A::RunningStairs
            );
        }
    }

    #[test]
    fn lift_type_upright_matches_climb_upwards() {
        // `translate_upright_action` must be identical to
        // `translate_climb_action(_, false)` for every type / action,
        // since upwards == downwards for upright posture.
        use crate::order::OrderType as A;
        for lt in [
            LiftType::Normal,
            LiftType::Stairs,
            LiftType::Ladder,
            LiftType::Wall,
        ] {
            for action in [
                A::WalkingUpright,
                A::RunningUpright,
                A::WalkingAlerted,
                A::WalkingCrouched,
            ] {
                assert_eq!(
                    lt.translate_upright_action(action),
                    lt.translate_climb_action(action, false),
                    "upright vs climb-upwards mismatch for {lt:?} {action:?}",
                );
            }
        }
    }

    fn lift_actor(kind: ElementKind, has_climb: bool) -> ActorAuthInfo {
        ActorAuthInfo {
            kind,
            pc_auth_bit: 0,
            has_lockpick: false,
            has_climb,
            has_jump: false,
            is_rider: false,
            posture: crate::element::Posture::Upright,
        }
    }

    #[test]
    fn lift_wall_only_pc_with_climb() {
        let pc_climb = lift_actor(ElementKind::ActorPc, true);
        let pc_no_climb = lift_actor(ElementKind::ActorPc, false);
        let soldier = lift_actor(ElementKind::ActorSoldier, false);

        assert!(LiftType::Wall.is_actor_authorized(&pc_climb));
        assert!(!LiftType::Wall.is_actor_authorized(&pc_no_climb));
        assert!(!LiftType::Wall.is_actor_authorized(&soldier));
    }

    #[test]
    fn lift_ladder_humans_except_civilians() {
        let pc = lift_actor(ElementKind::ActorPc, false);
        let soldier = lift_actor(ElementKind::ActorSoldier, false);
        let civilian = lift_actor(ElementKind::ActorCivilian, false);

        assert!(LiftType::Ladder.is_actor_authorized(&pc));
        assert!(LiftType::Ladder.is_actor_authorized(&soldier));
        assert!(!LiftType::Ladder.is_actor_authorized(&civilian));
    }

    #[test]
    fn lift_stairs_allows_humans() {
        let pc = lift_actor(ElementKind::ActorPc, false);
        let civilian = lift_actor(ElementKind::ActorCivilian, false);

        assert!(LiftType::Stairs.is_actor_authorized(&pc));
        assert!(LiftType::Stairs.is_actor_authorized(&civilian));
    }

    #[test]
    fn lift_normal_allows_everyone() {
        let pc = lift_actor(ElementKind::ActorPc, false);
        assert!(LiftType::Normal.is_actor_authorized(&pc));
    }

    #[test]
    fn script_sector_enter_leave() {
        use crate::entity_id::EntityId;
        let mut script = ScriptSectorData::new();
        script.enter(EntityId::Pc(crate::entity_id::PcId(10)));
        script.enter(EntityId::Pc(crate::entity_id::PcId(20)));

        assert!(script.is_inside(EntityId::Pc(crate::entity_id::PcId(10))));
        assert!(script.is_inside(EntityId::Pc(crate::entity_id::PcId(20))));
        assert!(!script.is_inside(EntityId::Pc(crate::entity_id::PcId(30))));
        assert_eq!(script.num_occupants(), 2);

        // Insert at front
        assert_eq!(
            script.get_occupant(0),
            EntityId::Pc(crate::entity_id::PcId(20))
        );
        assert_eq!(
            script.get_occupant(1),
            EntityId::Pc(crate::entity_id::PcId(10))
        );

        script.leave(EntityId::Pc(crate::entity_id::PcId(10)));
        assert!(!script.is_inside(EntityId::Pc(crate::entity_id::PcId(10))));
        assert_eq!(script.num_occupants(), 1);
    }

    #[test]
    fn script_sector_bulk_occupants_append_in_scan_order() {
        use crate::entity_id::{EntityId, PcId};

        let first = EntityId::Pc(PcId(10));
        let second = EntityId::Pc(PcId(20));
        let mut script = ScriptSectorData::new();
        script.add_occupant(first);
        script.add_occupant(second);

        assert_eq!(script.occupant_indices, vec![first, second]);
    }

    #[test]
    fn shadow_initialize_2d() {
        let mut shadow = ShadowData::default();
        let points = vec![
            MapPoint::new(0.0, 0.0),
            MapPoint::new(10.0, 0.0),
            MapPoint::new(10.0, 10.0),
            MapPoint::new(0.0, 10.0),
        ];
        shadow.initialize_2d(&points);

        assert!((shadow.barycentre_2d.x - 5.0).abs() < 1e-6);
        assert!((shadow.barycentre_2d.y - 5.0).abs() < 1e-6);
        assert!(shadow.radius > 0.0);
    }
}
