//! Per-frame entity-view snapshots indexed by handle.
//!
//! The AI tick runs without a mutable borrow on the entity store, so
//! handlers can't reach through a live entity pointer to read
//! position / animation / flags.  To bridge the gap the engine builds
//! an [`AiEntityViewMap`] at the start of the AI tick — a
//! handle-keyed snapshot of every entity's minimal public AI-facing
//! state — and threads it through [`crate::ai::AiContext`] so any
//! `think()` call can look up `handle → view` in O(1).
//!
//! This replaces the old handle-resolution stubs littered
//! across `ai.rs`, `ai_enemy.rs`, and `ai_friendly.rs` that used to
//! return `Position::default()` or silently no-op.
//!
//! # What goes in a view
//!
//! The view carries the **union** of fields the AI probes on any
//! entity (antagonist, body, object), flattened so consumers don't
//! care whether the entity is a PC, a soldier, a civilian, or a prop.
//! If a field doesn't apply (e.g. `ai_state` on a bonus pickup), it
//! takes its `Default` value.

use std::collections::HashMap;
use std::sync::Arc;

use crate::ai::{AiState, Position, PreparedForecastDestination, Substate};
use crate::coordinates::MapPoint;
use crate::element::{Camp, Entity, Posture};
use crate::order::OrderType;

fn patrol_path_view_fields(
    base: &crate::ai::AiController,
) -> (u8, u8, bool, Option<crate::ai::PathId>) {
    if let Some(path) = base.patrol_path.as_ref() {
        (
            path.current_waypoint_index,
            path.last_waypoint_index,
            path.forward,
            Some(path.hiking_path_index),
        )
    } else {
        // `RHPath::Init(alert_path_id)` immediately leaves a usable path
        // object behind. Rust temporarily detaches that object when a state
        // transition has no access to level assets, but other NPCs must still
        // observe its authored ID and iterator status synchronously.
        let path = &base.detached_patrol_path_status;
        (
            path.current_waypoint_index,
            path.last_waypoint_index,
            path.forward,
            path.hiking_path_index,
        )
    }
}

/// Snapshot of a single entity's AI-facing state at the top of the
/// current tick.
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct AiEntityView {
    /// Original `RHElement::GetCreationOrder()`. Runtime entity slots are not
    /// an interchangeable identity for creation-ordered AI behavior.
    pub original_creation_order: u32,
    /// Map-space x/y plus sector/level.
    pub position: Position,
    /// Raw element map position before AI `Position()` snaps door transit to
    /// a gate endpoint or substitutes a carrier for an OnShoulders PC.
    /// Direct actor geometry such as `ComputeDetectionPoint` uses this.
    pub detection_position: MapPoint,
    /// The same raw element position in stored 3D world coordinates.
    /// `ComputeDetectionPoint` and `ComputeEyesPoint` build on this point, so
    /// opaque-reachability endpoints must come from here rather than from
    /// `detection_position` plus the elevation.
    pub detection_position_world: crate::coordinates::WorldPoint3D,
    /// 0–15 facing sector.
    pub direction: u16,
    /// Standing / crouching / etc.
    pub posture: Posture,
    /// Defaults to [`Camp::default()`] (Neutral) for non-human entities.
    pub camp: Camp,

    pub is_pc: bool,
    pub is_robin: bool,
    pub is_vip: bool,
    /// Only set for civilians whose profile type is
    /// [`crate::profiles::CivilianType::Beggar`].
    pub is_beggar: bool,
    /// Only set for civilians whose profile type is
    /// [`crate::profiles::CivilianType::Child`].
    pub is_child: bool,
    /// Entity kind tag — the variant of [`crate::element::Entity`]
    /// this view was built from.  Used by `is_soldier` / `is_civilian`
    /// quick checks and by object-vs-human dispatch in
    /// `return_to_duty` fallback paths.
    pub kind: EntityKind,

    /// `true` for soldiers whose AI has the tower-guard flag set.
    /// Used by [`crate::ai_enemy::EnemyAi`]'s `TowerGuardCallAlert` to
    /// skip broadcasting CALL_TOWER_GUARD_ALERT back to other tower
    /// guards.
    pub is_tower_guard: bool,
    /// True when this human is engaged in melee (has at least one
    /// opponent).  Note: this is based on the opponent list, not the
    /// current action state — during enter/approach transitions an
    /// engaged actor can still be Moving/MovingFast.
    pub is_swordfighting: bool,
    /// Alive, conscious, not in the middle of a stagger / dying
    /// animation.
    pub is_able_to_fight: bool,
    /// Raw `ElementData::active`. Normal `IsDetecting(human)` uses this
    /// for its outside-building target gate; it must not be approximated
    /// by `is_able_to_fight` because unconscious actors remain active.
    pub active: bool,
    /// Human's unconscious flag.  Distinct from
    /// [`Self::is_able_to_fight`] which also requires
    /// `active && life_points > 0`; this is the raw KO flag used by
    /// the alerting-event view handler for sleeping-enemy substates.
    pub is_unconscious: bool,
    /// Raw actor action state used by normal visibility sharpness.
    pub action_state: crate::element::ActionState,
    /// Raw map-motion latch. A later creation-order actor may already be at
    /// its waypoint while its queued `EVENT_REACHPOINT` has not yet run.
    /// Cross-NPC synchronization needs to distinguish that arrived boundary
    /// from an actor still travelling toward the same current waypoint.
    pub is_moving_map: bool,
    /// True while the actor is executing a door pass. The original only
    /// applies this gate when viewer and target share a building sector.
    pub passing_door: bool,
    /// Projection obstacle the actor is standing on. Normal visibility
    /// passes it to `ComputeViewRadius` for the obstacle top-plane slice.
    pub obstacle_idx: Option<crate::position_interface::ObstacleHandle>,
    /// Inside a building sector.
    pub in_building: bool,
    /// Building sector handle (`None` if `in_building` is false).
    pub building_sector: Option<crate::position_interface::SectorHandle>,

    /// `script_locked` flag on the entity's AI controller.  Used by
    /// `AlertSoldier` to skip script-locked soldier candidates so a
    /// `WaitInformatively`-scripted guard isn't dragged off-script by
    /// an unrelated civilian alert.  `false` for entities without an
    /// AI brain.
    pub script_locked: bool,
    /// RNG-free alternatives for `ForecastDestinationForIA`. Building-exit
    /// selection is deliberately deferred until the exact AI routine where
    /// Original calls that method.
    pub forecasted_destination: PreparedForecastDestination,
    /// Defaults to [`AiState::default()`] for entities that have no
    /// AI brain.
    pub ai_state: AiState,
    /// Defaults to [`Substate::default()`].
    pub ai_substate: Substate,

    /// What animation the entity is currently playing — the live sequence
    /// order's action, or the sprite-driven animation while no order is
    /// selected.  Used by e.g. the weeping-animation check in
    /// `AiFriendly::random_speech` and the shield bearer's arrow-launched
    /// re-raise gate.  Not `ActorData::old_action`, which only feeds the
    /// next `ActionChange` callback and stays `Invalid` through most
    /// visible animations.
    pub current_animation: OrderType,

    /// World-Z coordinate of the entity's ground point (feet).  Used
    /// by `EnemyIsBelowMe` to pick the bow-down posture when an
    /// archer spots a target on a lower elevation.
    pub elevation: f32,

    /// Set for the [`EntityKind::Bonus`] kind,
    /// [`crate::element_kinds::ObjectType::None`] otherwise.  Used by
    /// `EventSeesObjectStandardProcedure` to branch between purse /
    /// coin / ale reactions.
    pub object_type: crate::element_kinds::ObjectType,

    /// `true` when the human's life points have reached zero.
    /// Distinct from [`Self::is_able_to_fight`] (which is `false` for
    /// KO'd-but-alive humans too).  Used by
    /// `EventSeesBodyStandardProcedure` to push NPC corpses onto the
    /// `missed_in_action` list.  `false` for non-humans.
    pub is_dead: bool,

    /// `true` when this human is being carried on another entity's
    /// shoulders (`HumanData::carrier` is `Some`).  Used by
    /// `EventViewStandardProcedure`'s Royalist-camp guards to skip
    /// chasing already-carried prisoners.
    pub is_carried: bool,
    /// `true` for soldier NPCs whose enemy AI brain is flagged as an
    /// archer unit.  `false` for non-soldiers and for soldiers without
    /// an AI brain.  Used by `EventViewStandardProcedure`'s Royalist
    /// guard to skip non-archer enemies on high walls.
    pub is_archer: bool,

    /// `true` for soldier NPCs mounted on a horse.  Drives the +60
    /// (vs +45) eye/detection Z offset in `stealth::eye_z_for_posture`
    /// / `detection_z_for_posture`, which the
    /// `EnemyAi::is_detecting_360_degrees` distance check needs to
    /// include the Z² term.  `false` for non-soldiers.
    pub is_rider: bool,

    /// True when `stuck_under_nets_counter > 0`.  Used by
    /// `SeekingNet`/`SeekingTakingNet` handlers to decide between
    /// "resurrection / keep waiting" and the SEARCH+TAKE sequence.
    pub stuck_under_net: bool,

    /// Nets currently covering this human.  Populated only when
    /// [`Self::stuck_under_net`] is true (empty list otherwise).
    /// Computed by iterating every active net and collecting those
    /// whose victim list contains this human.  Pre-computed here so
    /// `RunToFreeNetVictim` doesn't need a second borrow on the
    /// entity store.
    pub covering_nets: Vec<NetCoverInfo>,

    /// Only meaningful for PCs; resolved via the per-PC campaign
    /// status (`PcStatus::in_coma`).  Used by the approach-sleeping-
    /// enemy decision tree to route the "menace PC in coma" branch.
    pub in_coma: bool,

    /// Handle of the NPC currently menacing a PC in coma, or `None`
    /// if nobody has the guard role yet.  Stored as an AI handle
    /// (entity slot index) for consistency with the other views.
    pub guard: Option<u32>,

    /// True when the NPC owns a patrol path.  Mapped through
    /// `AiController::has_patrol_path`, **not** the pathfinder-
    /// waypoint `ActorData::has_path`.  Used by `MissedCharlyAlert`
    /// and `SearchCharly` to pick between patrol-radius and
    /// fix-radius fallbacks.
    pub has_patrol_path: bool,

    /// The NPC's current assigned post (guard post, waypoint-0 fallback).
    /// Read from `AiController::initial_position`, which is the Rust mirror
    /// of mutable Original `RHElementActorNPC::GetInitialPosition()`.
    pub initial_position: Position,

    /// Current arrow count.  Exposed so archers can check reserves
    /// without touching the engine-side inventory.  `0` for entities
    /// without a bow.
    pub number_of_arrows: u16,

    /// Profile rank (Officer / Soldier / Knight).
    /// `ProfileRank::None` for non-soldiers.  Used by cross-NPC
    /// dispatch (e.g. `EventSeesCharlyStandardProcedure`) that needs
    /// to branch on the *target*'s rank without walking camp_soldiers.
    pub rank: crate::profiles::ProfileRank,
    /// True once the soldier has reported back to an officer after a
    /// charly-seeking run.  Sourced from `EnemyAi::reported_to_officer`.
    pub reported_to_officer: bool,
    /// True when this soldier has already been reserved/looted after a
    /// money brawl.  Sourced from `AiBase::looted_after_money_fight` so
    /// money-fight looters can skip candidates already claimed by another
    /// scanner.
    pub looted_after_money_fight: bool,
    /// Current NPC money (`NpcData::money`).  Used by post-search looting
    /// speech to distinguish "found gold" from "found nothing".
    pub current_money: u32,
    /// Macro-in-progress flag on the target's AI brain — sourced from
    /// `AiController::macro_in_progress`.  Used by
    /// `EventSeesCharlyStandardProcedure` to detect whether the
    /// friend-to-synchronize-with is still executing a macro.
    pub macro_in_progress: bool,
    /// Current waypoint index on the target's patrol path.  `0` when
    /// the target has no patrol path.
    pub path_current_waypoint_index: u8,
    /// Last waypoint index on the target's patrol path.
    pub path_last_waypoint_index: u8,
    /// `true` when the patrol path is being walked in its forward
    /// direction, `false` for the reversed (ping-pong) leg.  Sourced
    /// from `PatrolPath::forward`.  `true` when the target has no
    /// patrol path (default for an uninitialised path).  Used by
    /// `InitializeFriendCheck`'s sync-matrix to compare the partner's
    /// traversal direction against ours.
    pub path_forward_movement: bool,
    /// The hiking-path index this NPC is patrolling.  `None` when the
    /// target has no patrol path.  Used by `SearchCharly` to walk the
    /// checkpoint charly's waypoint list without needing a second
    /// borrow on its AI brain.
    pub patrol_hiking_path_index: Option<crate::ai::PathId>,

    /// The pickup object (ale bottle, coin, …) this NPC has committed
    /// to in its current Wondering substate.  `0` when not set.  Used
    /// by `IsBeerStillAvailable` to detect friends racing for the
    /// same bottle.
    pub interesting_object: u32,

    /// AI brain's reconnaissance report classification (Nothing /
    /// MissedCharly / Body / Enemy).  Used by `GetReportFromCivilian`
    /// to read the civilian's report without touching the brain
    /// mid-think.
    pub report_type: crate::ai::ReportType,
    pub report_seek_position: Position,
    /// List of body handles the actor has logged in their report.
    /// Used by `ConsiderReport` for body-list merging across actors.
    pub report_seen_bodies: Vec<crate::ai::HumanHandle>,
    /// Handle of the missing-friend (charly) the actor is tracking,
    /// or `0` when none is set.
    pub report_charly: crate::ai::NpcHandle,
}

/// Per-net info carried on [`AiEntityView::covering_nets`] for humans
/// stuck under nets.  Carries the fields `RunToFreeNetVictim` reads
/// off each net in the cover list: position (for Chebyshev max-norm
/// distance ranking and the straight-movement authorisation check),
/// and radius (for the `radius + 15` goal distance).
#[derive(
    Debug,
    Clone,
    Copy,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct NetCoverInfo {
    /// Entity slot index of the covering net.  Stored into
    /// `AiBase::interesting_object` when the NPC commits to this net.
    pub handle: u32,
    /// Net's map-space position.
    pub position: Position,
    /// Net's radius in map units: 40 when the net is deployed
    /// normally, 10 when crumpled.
    pub radius: f32,
}

/// Entity kind tag carried on [`AiEntityView`].
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum EntityKind {
    /// No view / unknown / non-actor prop.  Matches the
    /// `Default::default()` value of the struct.
    #[default]
    Other,
    /// Player character ([`Entity::Pc`]).
    Pc,
    /// Soldier NPC ([`Entity::Soldier`]).
    Soldier,
    /// Civilian NPC ([`Entity::Civilian`]).
    Civilian,
    /// Pickup-style object ([`Entity::Bonus`]) — money, ale bottles,
    /// scroll items.  These are the targets of `WonderingTakingMoney`
    /// and related "go pick up an interesting object" states.
    Bonus,
    /// Script/runtime scroll object ([`Entity::Scroll`]).
    Scroll,
    /// Projectile-derived object ([`Entity::Projectile`]); thrown coins use
    /// this Original hierarchy branch while remaining valid money objects.
    Projectile,
    /// Net projectile object ([`Entity::Net`]).
    Net,
}

impl AiEntityView {
    /// Reconstruct the typed entity ID for the raw legacy-table `handle`
    /// used as this view's map key.
    ///
    /// AI stores C++ `RHElement*` values as their `marrayElements` slot.  A
    /// slot alone does not imply PC, soldier, or civilian: use the kind that
    /// was captured from the live entity when this view was built rather
    /// than inventing a typed ID at an effect boundary.
    pub fn entity_id(&self, handle: u32) -> Option<crate::element::EntityId> {
        use crate::ai_entity_view::EntityKind;
        use crate::entity_id::{
            BonusId, CivilianId, NetId, PcId, ProjectileId, ScrollId, SoldierId,
        };

        match self.kind {
            EntityKind::Pc => Some(crate::element::EntityId::Pc(PcId(handle))),
            EntityKind::Soldier => Some(crate::element::EntityId::Soldier(SoldierId(handle))),
            EntityKind::Civilian => Some(crate::element::EntityId::Civilian(CivilianId(handle))),
            EntityKind::Bonus => Some(crate::element::EntityId::Bonus(BonusId(handle))),
            EntityKind::Scroll => Some(crate::element::EntityId::Scroll(ScrollId(handle))),
            EntityKind::Projectile => {
                Some(crate::element::EntityId::Projectile(ProjectileId(handle)))
            }
            EntityKind::Net => Some(crate::element::EntityId::Net(NetId(handle))),
            EntityKind::Other => None,
        }
    }

    /// True if this view refers to a soldier NPC.
    pub fn is_soldier(&self) -> bool {
        self.kind == EntityKind::Soldier
    }

    /// True if this view refers to a civilian NPC.
    pub fn is_civilian(&self) -> bool {
        self.kind == EntityKind::Civilian
    }

    /// Port of `RHElementActorHuman::IsOutOfOrder`
    /// (`RHelementactorhuman.cpp:13271`):
    ///
    /// ```text
    /// return IsDead() || IsUnconscious() || IsStuckUnderNet()
    ///     || posture == RHPOSTURE_TIED || posture == RHPOSTURE_CARRIED
    ///     || ( IsPC() && IsInComa() );
    /// ```
    ///
    /// This is *not* `!is_able_to_fight`: `IsAbleToFight` is a virtual with a
    /// combat-readiness meaning (civilians are never able to fight, soldiers
    /// lose it while sleeping / menacing / fleeing / staggering) whereas
    /// `IsOutOfOrder` asks only whether the human is a "body" — down, netted,
    /// tied, carried, or comatose. Body queues such as
    /// `mlistOtherBodiesToExamine` are pruned with this predicate, so
    /// substituting the combat-readiness flag keeps recovered civilians in the
    /// queue forever and drops netted-but-alert soldiers out of it.
    pub fn is_out_of_order(&self) -> bool {
        self.is_dead
            || self.is_unconscious
            || self.stuck_under_net
            || matches!(
                self.posture,
                crate::element::Posture::Tied | crate::element::Posture::Carried
            )
            || (self.kind == EntityKind::Pc && self.in_coma)
    }
}

/// Handle → [`AiEntityView`] map.  Populated once per AI tick by
/// [`build_entity_views`] and shared into each [`crate::ai::AiContext`]
/// via an [`Arc`] so building a new `AiContext` is O(1).
pub type AiEntityViewMap = HashMap<u32, AiEntityView>;

/// Per-tick AI-readable world snapshot. Building authorization lives beside
/// the entity map because both must be captured from the same canonical engine
/// boundary before immutable AI handlers run.
#[derive(Debug, Clone, Default)]
pub struct AiEntityViews {
    pub entities: AiEntityViewMap,
    pub building_authorizations: HashMap<crate::sector::SectorNumber, bool>,
}

impl std::ops::Deref for AiEntityViews {
    type Target = AiEntityViewMap;

    fn deref(&self) -> &Self::Target {
        &self.entities
    }
}

impl std::ops::DerefMut for AiEntityViews {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entities
    }
}

impl AiEntityViews {
    pub fn building_is_authorized(&self, sector: crate::sector::SectorNumber) -> bool {
        *self
            .building_authorizations
            .get(&sector)
            .unwrap_or_else(|| panic!("AI gate route references missing building sector {sector}"))
    }
}

/// `Arc`-wrapped [`AiEntityViews`]. Cloning is a single atomic increment so
/// every `think()` dispatch can embed its own reference.
pub type SharedAiEntityViews = Arc<AiEntityViews>;

/// Test/helper constructor for snapshots that do not exercise building-gate
/// routing. Production snapshots are built by `EngineInner` and always carry
/// the exact building authorization map.
pub fn shared_entity_views(entities: AiEntityViewMap) -> SharedAiEntityViews {
    Arc::new(AiEntityViews {
        entities,
        building_authorizations: HashMap::new(),
    })
}

/// Build an [`AiEntityView`] from a generic [`Entity`] reference.
///
/// `in_building` / `building_sector` must be resolved by the caller
/// via `EngineInner::entity_building_sector(elem.sector)` — the building
/// lookup lives on [`crate::engine::EngineInner`] and can't be reached
/// from this module without threading the engine through.
pub fn entity_view_from_entity(
    entity: &Entity,
    original_creation_order: u32,
    in_building: bool,
    building_sector: Option<crate::position_interface::SectorHandle>,
    campaign: Option<&crate::campaign::Campaign>,
    current_animation: OrderType,
) -> AiEntityView {
    let elem = entity.element_data();
    let actor = entity.actor_data();
    let position = Position {
        x: elem.position_map().x,
        y: elem.position_map().y,
        sector: elem.sector(),
        level: elem.layer(),
    };
    let direction = elem.direction() as u16;
    let posture = elem.posture;
    // Swordfighting is based on the opponent list, not the current
    // action state. During enter/approach transitions an engaged actor
    // can still be Moving/MovingFast.
    let is_swordfighting = entity
        .human_data()
        .map(|h| !h.opponents.is_empty())
        .unwrap_or(false);
    let is_tower_guard = match entity {
        Entity::Soldier(s) => s
            .npc
            .ai_brain
            .enemy()
            .map(|e| e.tower_guard)
            .unwrap_or(false),
        _ => false,
    };

    let (kind, camp, is_pc, is_robin, is_vip, is_beggar, is_child) = match entity {
        Entity::Pc(pc) => (
            EntityKind::Pc,
            Camp::Royalists,
            true,
            pc.pc.robin,
            // Conservatively flag only Robin as VIP, matching the
            // `antagonist_info_from_entity` path in `engine::ai`.
            pc.pc.robin,
            false,
            false,
        ),
        Entity::Soldier(s) => {
            let enemy_vip = s.npc.ai_brain.enemy().map(|e| e.is_vip).unwrap_or(false);
            (
                EntityKind::Soldier,
                s.soldier.cached_camp,
                false,
                false,
                enemy_vip,
                false,
                false,
            )
        }
        Entity::Civilian(c) => {
            use crate::profiles::CivilianType;
            let ctype = c.civilian.cached_civilian_type;
            (
                EntityKind::Civilian,
                c.civilian.cached_camp,
                false,
                false,
                ctype == CivilianType::Vip,
                ctype == CivilianType::Beggar,
                ctype == CivilianType::Child,
            )
        }
        Entity::Bonus(_) => (
            EntityKind::Bonus,
            Camp::default(),
            false,
            false,
            false,
            false,
            false,
        ),
        Entity::Scroll(_) => (
            EntityKind::Scroll,
            Camp::default(),
            false,
            false,
            false,
            false,
            false,
        ),
        Entity::Projectile(_) => (
            EntityKind::Projectile,
            Camp::default(),
            false,
            false,
            false,
            false,
            false,
        ),
        Entity::Net(_) => (
            EntityKind::Net,
            Camp::default(),
            false,
            false,
            false,
            false,
            false,
        ),
        _ => (
            EntityKind::Other,
            Camp::default(),
            false,
            false,
            false,
            false,
            false,
        ),
    };

    // `IsAbleToFight` is virtual in the original hierarchy. The base
    // human implementation is always false, so civilians (which do not
    // override it) can never fight. Soldiers layer tied/carried checks
    // and a state-machine switch on top of their body state. PCs reject
    // the two disguised postures in addition to the common live/active
    // gates. Non-human entities are false.
    let is_able_to_fight = match entity {
        Entity::Soldier(s) => {
            let human_ok = !s.human.unconscious
                && s.element.active
                && s.npc.life_points > 0
                && s.element.posture != Posture::Tied
                && s.human.carrier.is_none();
            if !human_ok {
                false
            } else {
                match s.npc.ai_state() {
                    AiState::Sleeping | AiState::Menacing | AiState::Fleeing => false,
                    AiState::Default | AiState::Wondering | AiState::Seeking => true,
                    AiState::Attacking => !matches!(
                        s.npc.ai_substate(),
                        Substate::AttackingGotHit
                            | Substate::AttackingGotHitStandingUp
                            | Substate::AttackingHitting,
                    ),
                }
            }
        }
        Entity::Civilian(_) => false,
        Entity::Pc(pc) => {
            !pc.human.unconscious
                && pc.element.active
                && pc.pc.life_points > 0
                && !matches!(pc.element.posture, Posture::Tree | Posture::Spy)
        }
        _ => false,
    };

    let is_unconscious = match entity {
        Entity::Soldier(s) => s.human.unconscious,
        Entity::Civilian(c) => c.human.unconscious,
        Entity::Pc(pc) => pc.human.unconscious,
        _ => false,
    };
    let active = elem.active;
    let action_state = actor.map(|a| a.action_state).unwrap_or_default();
    let passing_door = actor.is_some_and(|a| a.active_door_pass.is_some());
    let obstacle_idx = elem.obstacle_index();

    let (ai_state, ai_substate) = match entity {
        Entity::Soldier(s) => (s.npc.ai_state(), s.npc.ai_substate()),
        Entity::Civilian(c) => (c.npc.ai_state(), c.npc.ai_substate()),
        // Non-NPC entities (PCs, bonus pickups) have no AI brain —
        // use `AiState::Default` as the null sentinel.
        _ => (AiState::Default, Substate::StartSleepingSubstates),
    };

    // Read off the AI controller's `script_locked` flag (set by
    // `ScriptLockAI`, cleared by `ScriptUnlockAI`).  Non-NPC entities
    // have no AI brain, return false.
    let script_locked = match entity {
        Entity::Soldier(s) => s
            .npc
            .ai_brain
            .base()
            .map(|b| b.ai_is_script_locked())
            .unwrap_or(false),
        Entity::Civilian(c) => c
            .npc
            .ai_brain
            .base()
            .map(|b| b.ai_is_script_locked())
            .unwrap_or(false),
        _ => false,
    };

    let elevation = elem.position().z;

    let stuck_under_net = match entity {
        Entity::Soldier(s) => s.human.stuck_under_nets_counter > 0,
        Entity::Civilian(c) => c.human.stuck_under_nets_counter > 0,
        Entity::Pc(pc) => pc.human.stuck_under_nets_counter > 0,
        _ => false,
    };

    // Only meaningful for Bonus entities; human/None defaults make
    // sense for everything else.
    let object_type = entity
        .object_data()
        .map(|o| o.object_type)
        .unwrap_or(crate::element_kinds::ObjectType::None);

    // `life_points <= 0` on humans (no concept of "dead" for
    // non-humans).
    let is_dead = match entity {
        Entity::Soldier(s) => s.npc.life_points <= 0,
        Entity::Civilian(c) => c.npc.life_points <= 0,
        Entity::Pc(pc) => pc.pc.life_points <= 0,
        _ => false,
    };

    let is_carried = match entity {
        Entity::Soldier(s) => s.human.carrier.is_some(),
        Entity::Civilian(c) => c.human.carrier.is_some(),
        Entity::Pc(pc) => pc.human.carrier.is_some(),
        _ => false,
    };

    // Read off the enemy AI brain's `is_archer_unit` flag.  Defaults
    // to false for soldiers without a brain or for non-soldier kinds.
    let is_archer = match entity {
        Entity::Soldier(s) => s
            .npc
            .ai_brain
            .enemy()
            .map(|e| e.is_archer_unit)
            .unwrap_or(false),
        _ => false,
    };

    // Only mounted soldiers; `false` for everyone else.  Mirrors the
    // in-Entity logic at `element.rs:2926` / `compute_eyes_point`.
    let is_rider = matches!(entity, Entity::Soldier(s) if s.soldier.rider);

    // `in_coma` + `guard` on PCs — look up through the exact campaign
    // description because `PcStatus` isn't embedded in `PcData`.
    // `list_index` is only the actor/UI list byte; it is not the identity of
    // Original's `mpDescription` and can point at an unrelated campaign PC.
    let (in_coma, guard) = match entity {
        Entity::Pc(pc) => {
            let coma = campaign.map_or(false, |c| {
                let description_index = pc.pc.campaign_description_index.unwrap_or_else(|| {
                    panic!("live PC is missing its required campaign-description identity")
                });
                c.characters
                    .get(description_index as usize)
                    .unwrap_or_else(|| {
                        panic!(
                            "live PC campaign-description index {description_index} is outside the campaign character table"
                        )
                    })
                    .status
                    .in_coma
            });
            let guard_handle = pc.pc.guard.map(|eid| eid.index());
            (coma, guard_handle)
        }
        _ => (false, None),
    };

    // True when a patrol path is registered on the AI controller
    // (see `AiController::has_patrol_path`).
    let has_patrol_path = match entity {
        Entity::Soldier(s) => s
            .npc
            .ai_brain
            .base()
            .map(|b| b.has_patrol_path)
            .unwrap_or(false),
        Entity::Civilian(c) => c
            .npc
            .ai_brain
            .base()
            .map(|b| b.has_patrol_path)
            .unwrap_or(false),
        _ => false,
    };

    // Original `GetInitialPosition()` is mutated by AssignNewPost and the
    // no-path AssignNewPatrolPath branches. Rust's controller owns that live
    // value; the NPC fields retain their level-load/save snapshot and can be
    // stale after a script changes the post.
    let initial_position = match entity {
        Entity::Soldier(s) => authoritative_initial_position(
            s.npc.ai_brain.base(),
            Position {
                x: s.npc.initial_position_x,
                y: s.npc.initial_position_y,
                sector: s.npc.initial_position_sector,
                level: s.npc.initial_position_level,
            },
        ),
        Entity::Civilian(c) => authoritative_initial_position(
            c.npc.ai_brain.base(),
            Position {
                x: c.npc.initial_position_x,
                y: c.npc.initial_position_y,
                sector: c.npc.initial_position_sector,
                level: c.npc.initial_position_level,
            },
        ),
        _ => position,
    };

    // Arrow count on soldier NPCs; other kinds always return 0.
    let number_of_arrows = match entity {
        Entity::Soldier(s) => s.npc.number_of_arrows,
        _ => 0,
    };

    // Soldier rank + report/looting flags — sourced from the enemy/base
    // AI brain when present, defaulting to None / false for
    // non-soldiers (civilians, PCs, props).
    let (rank, reported_to_officer, looted_after_money_fight) = match entity {
        Entity::Soldier(s) => s
            .npc
            .ai_brain
            .enemy()
            .map(|e| {
                (
                    e.soldier_profile_rank,
                    e.reported_to_officer,
                    e.base.looted_after_money_fight,
                )
            })
            .unwrap_or((crate::profiles::ProfileRank::None, false, false)),
        _ => (crate::profiles::ProfileRank::None, false, false),
    };

    let current_money = match entity {
        Entity::Soldier(s) => s.npc.money,
        Entity::Civilian(c) => c.npc.money,
        _ => 0,
    };

    // Read `interesting_object` off `AiController::base` for NPCs.
    let interesting_object = match entity {
        Entity::Soldier(s) => s
            .npc
            .ai_brain
            .base()
            .map(|b| b.interesting_object)
            .unwrap_or(0),
        Entity::Civilian(c) => c
            .npc
            .ai_brain
            .base()
            .map(|b| b.interesting_object)
            .unwrap_or(0),
        _ => 0,
    };

    // Read `my_reconnaissance_report` off `AiBase` for any NPC
    // (soldier or civilian).  Civilians need this exposed so an
    // officer's `GetReportFromCivilian` can merge bodies/charly
    // without a second borrow on the civilian's AI brain mid-think.
    let (report_type, report_seek_position, report_seen_bodies, report_charly) = match entity {
        Entity::Soldier(s) => s
            .npc
            .ai_brain
            .base()
            .map(|b| {
                (
                    b.my_reconnaissance_report.report_type,
                    b.my_reconnaissance_report.seek_position,
                    b.my_reconnaissance_report.seen_bodies.clone(),
                    b.my_reconnaissance_report.charly,
                )
            })
            .unwrap_or((
                crate::ai::ReportType::Nothing,
                Position::default(),
                Vec::new(),
                0,
            )),
        Entity::Civilian(c) => c
            .npc
            .ai_brain
            .base()
            .map(|b| {
                (
                    b.my_reconnaissance_report.report_type,
                    b.my_reconnaissance_report.seek_position,
                    b.my_reconnaissance_report.seen_bodies.clone(),
                    b.my_reconnaissance_report.charly,
                )
            })
            .unwrap_or((
                crate::ai::ReportType::Nothing,
                Position::default(),
                Vec::new(),
                0,
            )),
        _ => (
            crate::ai::ReportType::Nothing,
            Position::default(),
            Vec::new(),
            0,
        ),
    };

    // `macro_in_progress` + patrol-path waypoint indices — read off
    // `AiController::base` for NPCs.
    let (
        macro_in_progress,
        path_current_waypoint_index,
        path_last_waypoint_index,
        path_forward_movement,
        patrol_hiking_path_index,
    ) = match entity {
        Entity::Soldier(s) => s
            .npc
            .ai_brain
            .base()
            .map(|b| {
                let (cur, last, fwd, idx) = patrol_path_view_fields(b);
                (b.macro_in_progress, cur, last, fwd, idx)
            })
            .unwrap_or((false, 0, 0, true, None)),
        Entity::Civilian(c) => c
            .npc
            .ai_brain
            .base()
            .map(|b| {
                let (cur, last, fwd, idx) = patrol_path_view_fields(b);
                (b.macro_in_progress, cur, last, fwd, idx)
            })
            .unwrap_or((false, 0, 0, true, None)),
        _ => (false, 0, 0, true, None),
    };

    AiEntityView {
        original_creation_order,
        position,
        detection_position: elem.position_map(),
        detection_position_world: elem.position(),
        direction,
        posture,
        camp,
        is_pc,
        is_robin,
        is_vip,
        is_beggar,
        is_child,
        kind,
        is_tower_guard,
        is_swordfighting,
        is_able_to_fight,
        active,
        is_unconscious,
        action_state,
        is_moving_map: entity.position_iface().is_moving_map(),
        passing_door,
        obstacle_idx,
        in_building,
        building_sector,
        ai_state,
        ai_substate,
        script_locked,
        // The engine view-builder overwrites this with all authored
        // door/lift/building alternatives for human actors.
        forecasted_destination: PreparedForecastDestination::fixed(position, direction),
        current_animation,
        elevation,
        object_type,
        is_dead,
        is_carried,
        is_archer,
        is_rider,
        stuck_under_net,
        // Net coverage is evaluated by the view-map builder
        // (`engine::ai::build_entity_views`) which has access to the
        // full entity store.  Leave empty here — the builder
        // overwrites this list for stuck victims.
        covering_nets: Vec::new(),
        in_coma,
        guard,
        has_patrol_path,
        initial_position,
        number_of_arrows,
        rank,
        reported_to_officer,
        looted_after_money_fight,
        current_money,
        macro_in_progress,
        path_current_waypoint_index,
        path_last_waypoint_index,
        path_forward_movement,
        patrol_hiking_path_index,
        interesting_object,
        report_type,
        report_seek_position,
        report_seen_bodies,
        report_charly,
    }
}

fn authoritative_initial_position(
    controller: Option<&crate::ai::AiController>,
    serialized_npc_fallback: Position,
) -> Position {
    controller
        .map(|controller| controller.initial_position)
        .unwrap_or(serialized_npc_fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detached_alert_path_remains_visible_to_other_ai() {
        let mut base = crate::ai::AiController::new(17);
        base.has_patrol_path = true;
        base.detached_patrol_path_status = crate::ai::DetachedPatrolPathStatus {
            hiking_path_index: crate::ai::PathId::new(33),
            current_waypoint_index: 4,
            last_waypoint_index: 3,
            forward: false,
            history: Vec::new(),
        };

        assert_eq!(
            patrol_path_view_fields(&base),
            (4, 3, false, crate::ai::PathId::new(33))
        );
    }

    #[test]
    fn entity_view_uses_mutable_ai_post_over_stale_npc_snapshot() {
        let stale = Position {
            x: 1163.0,
            y: 97.0,
            ..Position::default()
        };
        let mut controller = crate::ai::AiController::new(17);
        controller.initial_position = Position {
            x: 956.9903,
            y: 404.00244,
            ..Position::default()
        };

        assert_eq!(
            authoritative_initial_position(Some(&controller), stale),
            controller.initial_position
        );
        assert_eq!(authoritative_initial_position(None, stale), stale);
    }
}
