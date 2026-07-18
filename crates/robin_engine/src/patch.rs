//! Map patch — interactive terrain areas (traps, hidden items, doors).
//!
//! A patch represents a toggleable area on the map that swaps between two
//! sets of masks, obstacles, sectors, and lines when triggered (e.g. by a
//! trap or player action).
//!
//! EngineInner side-effects (swapping masks, background bitmaps, pathfinder
//! obstacles, FX animations) are returned as [`PatchEffect`] values for
//! the caller to execute.

use serde::{Deserialize, Serialize};

use crate::coordinates::MapPoint;

// ---------------------------------------------------------------------------
// PatchIndex — nominal newtype
// ---------------------------------------------------------------------------

/// Index into `GameHost::patches`.  Wraps [`nonmax::NonMaxU32`] so
/// `Option<PatchIndex>` is 4 bytes via niche optimization.  `u32::MAX`
/// would be an absurd patch count, so forbidding it costs nothing.
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
pub struct PatchIndex(pub nonmax::NonMaxU32);

impl PatchIndex {
    #[inline]
    pub fn new(v: u32) -> Option<Self> {
        nonmax::NonMaxU32::new(v).map(Self)
    }
    #[inline]
    pub fn get(self) -> u32 {
        self.0.get()
    }
}
impl From<PatchIndex> for u32 {
    #[inline]
    fn from(i: PatchIndex) -> u32 {
        i.0.get()
    }
}
impl From<PatchIndex> for usize {
    #[inline]
    fn from(i: PatchIndex) -> usize {
        i.0.get() as usize
    }
}
impl std::fmt::Display for PatchIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().fmt(f)
    }
}

// ---------------------------------------------------------------------------
// PatchAnimation
// ---------------------------------------------------------------------------

/// Which animation phase a patch FX element should play.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum PatchAnimation {
    /// Idle loop shown before the patch is triggered.
    Initial,
    /// Played during the apply/unapply transition.
    Transition,
    /// Shown after the patch has been applied.
    Final,
}

// ---------------------------------------------------------------------------
// PatchState
// ---------------------------------------------------------------------------

/// High-level patch lifecycle state, derived from the boolean triple
/// (`active`, `applied`, `in_transition`).
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
)]
pub enum PatchState {
    /// The patch is inactive (definitive patch already used, or deactivated).
    Inactive,
    /// Active but not yet triggered.
    #[default]
    Unapplied,
    /// Transition animation is playing.
    InTransition,
    /// The patch has been fully applied.
    Applied,
}

// ---------------------------------------------------------------------------
// AnimationFlags
// ---------------------------------------------------------------------------

/// Which animation phases are available for this patch.
/// Loaded from the level file, not serialized in saves.
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
pub struct AnimationFlags {
    pub start_valid: bool,
    pub transition_valid: bool,
    pub end_valid: bool,
}

// ---------------------------------------------------------------------------
// OccupantId
// ---------------------------------------------------------------------------

/// Opaque handle to an actor occupying this patch's sector.
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
pub struct OccupantId(pub u32);

// ---------------------------------------------------------------------------
// PatchEffect
// ---------------------------------------------------------------------------

/// Side effects produced by patch state transitions.
///
/// The engine must execute these after the patch's state has been updated.
/// This decouples the state machine from the rendering, pathfinder, and
/// animation systems.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub enum PatchEffect {
    /// Swap masks, obstacles, sectors, and lines.
    SwapObjects { applied: bool, forced_reset: bool },
    /// Swap the background bitmap.
    SwapBackground { applied: bool },
    /// Toggle door rights for connected doors.
    SwapDoors,
    /// Start a specific animation on the FX element.
    /// `reverse`: if true, start at the last frame and play backwards
    /// (used for unapply transition).
    StartAnimation { anim: PatchAnimation, reverse: bool },
    /// Deactivate the FX animation element.
    DeactivateAnimation,
    /// Restore the background from the FX element's saved data.
    RestoreBackground,
}

// ---------------------------------------------------------------------------
// Patch
// ---------------------------------------------------------------------------

/// A map patch — an interactive terrain area.
///
/// Patch state and level-static references used by the script host and
/// patch transition logic.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct Patch {
    // -- Serialized (save game state) --
    /// Whether the patch is currently active (can be interacted with).
    pub active: bool,
    /// Whether the patch's "new" state is currently showing.
    pub applied: bool,
    /// Whether a transition animation is currently playing.
    pub in_transition: bool,
    /// Whether the patch is locked (cannot be triggered).
    pub locked: bool,
    /// Host-derived cursor/render cache: whether associated doors should be
    /// highlighted for the current local selection. This is deliberately not
    /// save, rollback, or state-hash data because each multiplayer peer owns
    /// its own cursor and selection presentation.
    #[serde(skip, default)]
    #[state_hash(skip)]
    pub display_doors: bool,
    /// Actors currently inside this patch's sector.
    pub occupants: Vec<OccupantId>,

    // -- Level data --
    /// If true, the patch can only be applied once and becomes inactive after.
    pub definitive: bool,
    /// The original `active` value from the level file; used by `force_reset`.
    pub initially_active: bool,
    /// Whether this patch uses FX animations.
    pub animated: bool,
    /// This patch is triggered when a connected door is opened.
    pub door_triggered: bool,
    /// This patch triggers connected doors when applied.
    pub triggers_door: bool,
    /// Whether the final frame should be baked into the background.
    pub integrate_in_background: bool,
    /// Which animation phases are valid for this patch.
    pub animation_flags: AnimationFlags,
    /// Whether this patch changes pathfinder obstacles.
    pub use_changing_obstacles: bool,
    /// Pathfinder layer index for obstacle changes.
    pub pathfinder_layer: u16,
    /// Pathfinder sector index for obstacle changes.
    pub pathfinder_sector: u16,
    /// Pathfinder changing-obstacle index.
    pub pathfinder_changing_obstacles: u32,
    /// Map layer this patch belongs to.
    pub layer: u16,
    /// Map sector this patch belongs to.
    pub sector: u16,
    /// Waypoint position for this patch.
    pub waypoint: MapPoint,
    /// Indices of doors controlled by this patch (into the GameHost
    /// doors array).  Mirrors the C++ `RHPatch::maDoors` list: populated
    /// only for `triggers_door` patches.  When `PatchEffect::SwapDoors`
    /// fires inside `apply_final`, `swap_rights_patch()` is called on
    /// each door in this list.  `door_triggered`-style links are stored
    /// on the *door* side as `Door::patch_index` instead.
    pub door_indices: Vec<u32>,

    // -- Swap data (old/new object indices for SwapObjects) --
    /// Indices into `EngineInner::sight_obstacles` for old-state obstacles.
    /// Toggled to `!applied` on SwapObjects.
    pub old_sight_obstacle_indices: Vec<crate::sight_obstacle::SightObstacleIndex>,
    /// Indices into `EngineInner::sight_obstacles` for new-state obstacles.
    /// Toggled to `applied` on SwapObjects.
    pub new_sight_obstacle_indices: Vec<crate::sight_obstacle::SightObstacleIndex>,
    /// Indices into `FastFindGrid::sectors` for old-state sectors.
    /// Toggled to `!applied` on SwapObjects.
    pub old_sector_indices: Vec<u32>,
    /// Indices into `FastFindGrid::sectors` for new-state sectors.
    /// Toggled to `applied` on SwapObjects.
    pub new_sector_indices: Vec<u32>,
    /// Indices into `FastFindGrid::lines` for old-state lines.
    /// Toggled to `!applied` on SwapObjects.
    pub old_line_indices: Vec<crate::fast_find_grid::LineIndex>,
    /// Indices into `FastFindGrid::lines` for new-state lines.
    /// Toggled to `applied` on SwapObjects.
    pub new_line_indices: Vec<crate::fast_find_grid::LineIndex>,
    /// Indices into `FastFindGrid::masks` for old-state sprite-occlusion
    /// masks.  Toggled to `!applied` on SwapObjects so the old building
    /// silhouette stops occluding actors once the patch has fired.
    pub old_mask_indices: Vec<crate::mask::MaskIndex>,
    /// Indices into `FastFindGrid::masks` for new-state sprite-occlusion
    /// masks.  Toggled to `applied` on SwapObjects so the new building
    /// silhouette starts occluding actors once the patch has fired.
    pub new_mask_indices: Vec<crate::mask::MaskIndex>,

    /// Grid-sector index of this patch's apply sector
    /// (`SECTOR_CROSS | SECTOR_PATCH | SECTOR_APPLY`).
    /// Populated when the patch loader registers the apply polygon in
    /// `FastFindGrid`.  Consulted by the per-tick `LINE_PATCH` crossing
    /// dispatch: when a PC crosses a `LINE_PATCH` boundary, its new
    /// position is tested against this sector's polygon to decide
    /// `Enter` vs `Leave`.  `None` when the patch carries no apply
    /// polygon (an empty proto polygon leaves the apply sector unset).
    pub apply_sector_index: Option<u32>,
}

impl Default for Patch {
    fn default() -> Self {
        Self {
            active: false,
            applied: false,
            definitive: false,
            in_transition: false,
            display_doors: false,
            animated: true,
            locked: false,
            occupants: Vec::new(),
            initially_active: false,
            door_triggered: false,
            triggers_door: false,
            integrate_in_background: false,
            animation_flags: AnimationFlags::default(),
            use_changing_obstacles: false,
            pathfinder_layer: 0,
            pathfinder_sector: 0,
            pathfinder_changing_obstacles: 0,
            layer: 0,
            sector: 0,
            waypoint: MapPoint::new(0.0, 0.0),
            door_indices: Vec::new(),
            old_sight_obstacle_indices: Vec::new(),
            new_sight_obstacle_indices: Vec::new(),
            old_sector_indices: Vec::new(),
            new_sector_indices: Vec::new(),
            old_line_indices: Vec::new(),
            new_line_indices: Vec::new(),
            old_mask_indices: Vec::new(),
            new_mask_indices: Vec::new(),
            apply_sector_index: None,
        }
    }
}

impl Patch {
    /// Create a new patch with default (unapplied, inactive) state.
    pub fn new() -> Self {
        Self::default()
    }

    // -- State queries -------------------------------------------------------

    /// Returns the high-level lifecycle state derived from the boolean flags.
    pub fn state(&self) -> PatchState {
        if !self.active {
            PatchState::Inactive
        } else if self.in_transition {
            PatchState::InTransition
        } else if self.applied {
            PatchState::Applied
        } else {
            PatchState::Unapplied
        }
    }

    pub fn is_applied(&self) -> bool {
        self.applied
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn is_in_transition(&self) -> bool {
        self.in_transition
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    // -- Lock / unlock -------------------------------------------------------

    pub fn lock(&mut self) {
        self.locked = true;
    }

    pub fn unlock(&mut self) {
        self.locked = false;
    }

    // -- Occupant tracking ---------------------------------------------------

    /// Record an actor entering this patch's sector.
    ///
    /// The if/else is "warn-or-insert"; the carried-actor recursion runs
    /// **regardless** of which branch fired and is handled at the call
    /// site (movement.rs `check_for_patch_line_crossing`), which has
    /// access to the entity's `element.carried` field.
    pub fn enter(&mut self, occupant: OccupantId) {
        if self.occupants.contains(&occupant) {
            tracing::warn!("actor {:?} entering patch sector twice", occupant);
        } else {
            // Insert at front so `any_occupant()` returns the most recent.
            self.occupants.insert(0, occupant);
        }
    }

    /// Record an actor leaving this patch's sector.
    ///
    /// The carried-actor recursion runs unconditionally after the
    /// find/remove and is handled at the call site in movement.rs.
    pub fn leave(&mut self, occupant: OccupantId) {
        if let Some(idx) = self.occupants.iter().position(|o| *o == occupant) {
            self.occupants.remove(idx);
        } else {
            tracing::warn!(
                "actor {:?} leaving patch sector it hadn't entered",
                occupant
            );
        }
    }

    /// Check if an actor is inside this patch's sector.
    pub fn is_inside(&self, occupant: OccupantId) -> bool {
        self.occupants.contains(&occupant)
    }

    /// Returns the first occupant, if any.
    pub fn any_occupant(&self) -> Option<OccupantId> {
        self.occupants.first().copied()
    }

    // -- State transitions ---------------------------------------------------

    /// Toggle the patch (apply if unapplied, unapply if applied).
    ///
    /// Returns the engine side-effects to execute.
    pub fn apply(&mut self) -> Vec<PatchEffect> {
        let mut effects = Vec::new();

        // If mid-transition, finalize the current transition first
        if self.animated && self.in_transition {
            effects.push(PatchEffect::DeactivateAnimation);
            self.in_transition = false;
            effects.extend(self.apply_final(false));
        }

        if self.applied {
            if !self.definitive {
                effects.push(PatchEffect::SwapBackground { applied: false });

                if self.animation_flags.transition_valid {
                    // Unapplying: play transition in reverse.
                    effects.push(PatchEffect::StartAnimation {
                        anim: PatchAnimation::Transition,
                        reverse: true,
                    });
                    self.in_transition = true;
                } else {
                    effects.push(PatchEffect::DeactivateAnimation);
                    effects.extend(self.apply_final(false));
                }
            }
        } else if self.animation_flags.transition_valid {
            // Applying: play transition forward.
            effects.push(PatchEffect::StartAnimation {
                anim: PatchAnimation::Transition,
                reverse: false,
            });
            self.in_transition = true;
        } else {
            effects.push(PatchEffect::DeactivateAnimation);
            effects.extend(self.apply_final(false));
        }

        effects
    }

    /// Finalize the patch application/unapplication.
    ///
    /// Called when a transition animation completes, or immediately if no
    /// transition animation exists.
    ///
    /// Does **not** clear `in_transition` — the caller (the animation
    /// system) is responsible for that flag.
    pub fn apply_final(&mut self, forced_reset: bool) -> Vec<PatchEffect> {
        let mut effects = Vec::new();

        if self.definitive {
            self.active = false;
        }

        effects.push(PatchEffect::SwapDoors);

        if self.applied {
            if !self.definitive {
                self.applied = false;

                if self.animation_flags.start_valid {
                    effects.push(PatchEffect::StartAnimation {
                        anim: PatchAnimation::Initial,
                        reverse: false,
                    });
                } else {
                    effects.push(PatchEffect::DeactivateAnimation);
                }

                effects.push(PatchEffect::SwapObjects {
                    applied: false,
                    forced_reset,
                });
            }
        } else {
            self.applied = true;

            effects.push(PatchEffect::SwapBackground { applied: true });
            effects.push(PatchEffect::SwapObjects {
                applied: true,
                forced_reset,
            });

            if self.animation_flags.end_valid {
                effects.push(PatchEffect::StartAnimation {
                    anim: PatchAnimation::Final,
                    reverse: false,
                });
            } else {
                effects.push(PatchEffect::DeactivateAnimation);
            }
        }

        effects
    }

    /// Force-reset the patch to its initial (unapplied) state.
    pub fn force_reset(&mut self) -> Vec<PatchEffect> {
        let mut effects = Vec::new();

        if self.applied {
            effects.push(PatchEffect::SwapBackground { applied: false });
            if !self.in_transition {
                effects.push(PatchEffect::SwapDoors);
            }
        }

        self.applied = false;
        self.active = self.initially_active;

        if self.animated {
            effects.push(PatchEffect::RestoreBackground);

            if self.animation_flags.start_valid {
                effects.push(PatchEffect::StartAnimation {
                    anim: PatchAnimation::Initial,
                    reverse: false,
                });
            } else {
                effects.push(PatchEffect::DeactivateAnimation);
            }

            self.in_transition = false;
        }

        effects.push(PatchEffect::SwapObjects {
            applied: false,
            forced_reset: true,
        });

        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_patch() -> Patch {
        Patch {
            active: true,
            initially_active: true,
            ..Patch::default()
        }
    }

    fn active_patch_with_animations() -> Patch {
        Patch {
            active: true,
            initially_active: true,
            animation_flags: AnimationFlags {
                start_valid: true,
                transition_valid: true,
                end_valid: true,
            },
            ..Patch::default()
        }
    }

    #[test]
    fn default_state() {
        let p = Patch::new();
        assert!(!p.is_applied());
        assert!(!p.is_active());
        assert!(!p.is_in_transition());
        assert!(!p.is_locked());
        assert_eq!(p.state(), PatchState::Inactive);
    }

    #[test]
    fn state_derivation() {
        let mut p = active_patch();
        assert_eq!(p.state(), PatchState::Unapplied);

        p.applied = true;
        assert_eq!(p.state(), PatchState::Applied);

        p.in_transition = true;
        assert_eq!(p.state(), PatchState::InTransition);

        p.active = false;
        assert_eq!(p.state(), PatchState::Inactive);
    }

    #[test]
    fn lock_unlock() {
        let mut p = Patch::new();
        assert!(!p.is_locked());
        p.lock();
        assert!(p.is_locked());
        p.unlock();
        assert!(!p.is_locked());
    }

    #[test]
    fn occupant_enter_leave() {
        let mut p = Patch::new();
        let a = OccupantId(1);
        let b = OccupantId(2);

        assert!(p.any_occupant().is_none());
        assert!(!p.is_inside(a));

        p.enter(a);
        assert!(p.is_inside(a));
        assert_eq!(p.any_occupant(), Some(a));

        p.enter(b);
        // Inserted at front
        assert_eq!(p.any_occupant(), Some(b));
        assert!(p.is_inside(a));

        p.leave(a);
        assert!(!p.is_inside(a));
        assert!(p.is_inside(b));

        p.leave(b);
        assert!(p.any_occupant().is_none());
    }

    #[test]
    fn double_enter_is_noop() {
        let mut p = Patch::new();
        let a = OccupantId(1);
        p.enter(a);
        p.enter(a); // warns but doesn't duplicate
        assert_eq!(p.occupants.len(), 1);
    }

    #[test]
    fn leave_absent_is_noop() {
        let mut p = Patch::new();
        p.leave(OccupantId(99)); // warns but doesn't panic
    }

    #[test]
    fn apply_no_transition_animation() {
        let mut p = active_patch();
        let effects = p.apply();
        assert!(p.is_applied());
        assert!(!p.is_in_transition());
        assert!(effects.contains(&PatchEffect::SwapDoors));
        assert!(effects.contains(&PatchEffect::SwapObjects {
            applied: true,
            forced_reset: false,
        }));
    }

    #[test]
    fn apply_with_transition_animation() {
        let mut p = active_patch_with_animations();
        let effects = p.apply();
        assert!(!p.is_applied());
        assert!(p.is_in_transition());
        assert!(effects.contains(&PatchEffect::StartAnimation {
            anim: PatchAnimation::Transition,
            reverse: false,
        }));
    }

    #[test]
    fn apply_final_completes_forward() {
        let mut p = active_patch_with_animations();
        p.in_transition = true;
        let effects = p.apply_final(false);
        assert!(p.is_applied());
        assert!(effects.contains(&PatchEffect::SwapDoors));
        assert!(effects.contains(&PatchEffect::StartAnimation {
            anim: PatchAnimation::Final,
            reverse: false,
        }));
    }

    #[test]
    fn apply_toggle_no_anim() {
        let mut p = active_patch();
        p.apply();
        assert!(p.is_applied());
        // Toggle back
        let effects = p.apply();
        assert!(!p.is_applied());
        assert!(effects.contains(&PatchEffect::SwapObjects {
            applied: false,
            forced_reset: false,
        }));
    }

    #[test]
    fn definitive_patch_cannot_unapply() {
        let mut p = active_patch();
        p.definitive = true;
        p.apply();
        assert!(p.is_applied());
        assert!(!p.is_active()); // definitive → deactivated after apply
        // Trying to toggle: applied && definitive → nothing happens
        let effects = p.apply();
        assert!(p.is_applied());
        assert!(effects.is_empty());
    }

    #[test]
    fn force_reset_from_applied() {
        let mut p = active_patch();
        p.apply();
        assert!(p.is_applied());
        let effects = p.force_reset();
        assert!(!p.is_applied());
        assert!(p.is_active()); // restored to initially_active
        assert!(!p.is_in_transition());
        assert!(effects.contains(&PatchEffect::SwapObjects {
            applied: false,
            forced_reset: true,
        }));
    }

    #[test]
    fn force_reset_during_transition_skips_door_swap() {
        let mut p = active_patch_with_animations();
        p.apply();
        assert!(p.is_in_transition());
        let effects = p.force_reset();
        assert!(!p.is_in_transition());
        assert!(!p.is_applied());
        // Doors are only swapped when applied && !in_transition.
        // Here applied was false (transition not finalized), so no SwapDoors at all.
        assert!(!effects.contains(&PatchEffect::SwapDoors));
    }

    #[test]
    fn apply_during_transition_finalizes_then_reverses() {
        let mut p = active_patch_with_animations();
        // Start forward transition
        p.apply();
        assert!(p.is_in_transition());
        assert!(!p.is_applied());

        // Apply again during transition → finalize forward, then start reverse
        let effects = p.apply();
        // After finalization: applied=true. Then reverse transition starts.
        assert!(p.is_in_transition());
        // The finalization produced SwapDoors + SwapBackground + SwapObjects + StartAnimation(Final)
        // Then reverse produced SwapBackground{false} + StartAnimation(Transition)
        assert!(effects.contains(&PatchEffect::SwapDoors));
        // Reverse transition since we're unapplying from applied state
        assert!(effects.contains(&PatchEffect::StartAnimation {
            anim: PatchAnimation::Transition,
            reverse: true,
        }));
    }

    #[test]
    fn serde_roundtrip_preserves_simulation_state_but_not_render_cache() {
        let mut p = active_patch_with_animations();
        p.applied = true;
        p.locked = true;
        p.display_doors = true;
        p.enter(OccupantId(42));
        p.layer = 5;
        p.sector = 10;

        let json = serde_json::to_string(&p).unwrap();
        let p2: Patch = serde_json::from_str(&json).unwrap();

        // Deterministic patch state survives, while the local cursor-derived
        // door highlight is rebuilt by the host after restore.
        assert_eq!(p2.active, p.active);
        assert_eq!(p2.applied, p.applied);
        assert_eq!(p2.in_transition, p.in_transition);
        assert_eq!(p2.locked, p.locked);
        assert!(!p2.display_doors);
        assert_eq!(p2.occupants, p.occupants);
        assert_eq!(p2.definitive, p.definitive);
        assert_eq!(p2.animated, p.animated);
        assert_eq!(p2.layer, p.layer);
        assert_eq!(p2.sector, p.sector);
    }

    #[test]
    fn display_doors_is_not_serialized_or_hashed() {
        let mut hidden = active_patch();
        let mut highlighted = hidden.clone();
        highlighted.display_doors = true;

        assert_eq!(
            robin_util::state_hash::compute(&hidden),
            robin_util::state_hash::compute(&highlighted),
        );

        let json = serde_json::to_value(&highlighted).expect("serialize patch");
        assert!(json.get("display_doors").is_none());

        // Historical JSON saves may still carry the old field. Serde ignores
        // it and the host reconstructs the cache from the local selection.
        let mut legacy = json;
        legacy["display_doors"] = serde_json::Value::Bool(true);
        hidden = serde_json::from_value(legacy).expect("load legacy patch save");
        assert!(!hidden.display_doors);
    }
}
