//! Main per-frame update tick (`perform_hourglass`).

mod actor_execution;
mod frame_systems;
mod mission;
mod paths;

#[path = "tick_action_change_step.rs"]
mod tick_action_change_step;
use tick_action_change_step::ActionChangeSlotCtx;

use super::movement::{CompletedPathWork, PathScheduleContext};
use super::*;
use crate::element::{Command, Entity, EntityId};
use crate::game_operation::GameCode;
use crate::messenger::{MessageType, SimpleMessage};
use crate::profiles::MissionType;

/// Strict opt-in gate for the Drop Execute-boundary diagnostic.
fn drop_owner_boundary_matches(frame: u32, owner: EntityId) -> bool {
    super::diagnostics::config().drop_boundary_matches(frame, owner)
}

#[cfg(test)]
thread_local! {
    static PROJECTILE_DERIVED_TAIL_TRACE: super::test_support::Probe<(EntityId, crate::element::ObjectType)> =
        const { super::test_support::Probe::new() };
}

pub(super) fn observe_projectile_derived_tail(
    id: EntityId,
    object_type: crate::element::ObjectType,
) {
    tracing::trace!(
        target: "robin_engine::engine::tick::projectile_tail",
        ?id,
        ?object_type,
        "projectile derived tail"
    );
    #[cfg(test)]
    PROJECTILE_DERIVED_TAIL_TRACE.with(|probe| probe.record((id, object_type)));
}

#[cfg(test)]
pub(super) fn capture_projectile_derived_tails<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<(EntityId, crate::element::ObjectType)>) {
    PROJECTILE_DERIVED_TAIL_TRACE.with(|probe| probe.capture(f))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NpcHourglassPhase {
    SoldierPrelude,
    Patrol,
    BaseHuman,
    Broadcasts,
    View,
    Detection,
    Ambush,
    Busy,
    Ladder,
    LockGate,
    SixteenthFrame,
    NormalTimer,
    MacroTimer,
    QueuedStimuli,
}

#[cfg(test)]
thread_local! {
    static NPC_HOURGLASS_PHASE_TRACE: super::test_support::Probe<NpcHourglassPhase> =
        const { super::test_support::Probe::new() };
}

fn observe_npc_hourglass_phase(phase: NpcHourglassPhase) {
    tracing::trace!(
        target: "robin_engine::engine::tick::npc_phases",
        ?phase,
        "npc hourglass phase"
    );
    #[cfg(test)]
    NPC_HOURGLASS_PHASE_TRACE.with(|probe| probe.record(phase));
}

#[cfg(test)]
pub(super) fn capture_npc_hourglass_phases<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<NpcHourglassPhase>) {
    NPC_HOURGLASS_PHASE_TRACE.with(|probe| probe.capture(f))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ActorAnimationBoundaryPhase {
    WaitReady(EntityId),
    GenericExecute(EntityId),
    CompletionEffects(EntityId),
    CombatInjuryThink(EntityId),
    ActionChange(EntityId),
}

#[cfg(test)]
thread_local! {
    static ACTOR_ANIMATION_BOUNDARY_TRACE: super::test_support::Probe<ActorAnimationBoundaryPhase> =
        const { super::test_support::Probe::new() };
}

pub(super) fn observe_actor_animation_boundary(phase: ActorAnimationBoundaryPhase) {
    tracing::trace!(
        target: "robin_engine::engine::tick::actor_animation_boundary",
        ?phase,
        "actor animation boundary"
    );
    #[cfg(test)]
    ACTOR_ANIMATION_BOUNDARY_TRACE.with(|probe| probe.record(phase));
}

#[cfg(test)]
pub(super) fn capture_actor_animation_boundary<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<ActorAnimationBoundaryPhase>) {
    ACTOR_ANIMATION_BOUNDARY_TRACE.with(|probe| probe.capture(f))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ActorOwnerEnvelopePhase {
    SoldierPrelude(EntityId),
    Patrol(EntityId),
    HumanPrelude(EntityId),
    BaseActor(EntityId),
    MovementExecute(EntityId),
    HumanNoise(EntityId),
    HumanTiredness(EntityId),
    PcTail(EntityId),
    NpcTail(EntityId),
}

#[cfg(test)]
thread_local! {
    static ACTOR_OWNER_ENVELOPE_TRACE: super::test_support::Probe<ActorOwnerEnvelopePhase> =
        const { super::test_support::Probe::new() };
}

fn observe_actor_owner_envelope(phase: ActorOwnerEnvelopePhase) {
    tracing::trace!(
        target: "robin_engine::engine::tick::actor_owner_envelope",
        ?phase,
        "actor owner envelope"
    );
    #[cfg(test)]
    ACTOR_OWNER_ENVELOPE_TRACE.with(|probe| probe.record(phase));
}

#[cfg(test)]
pub(super) fn capture_actor_owner_envelope<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<ActorOwnerEnvelopePhase>) {
    ACTOR_OWNER_ENVELOPE_TRACE.with(|probe| probe.capture(f))
}

/// Exact base-Actor Execute identity selected at entry to one legacy slot.
///
/// The coordinator carries the selected Original sequence/element/order
/// identity and revalidates it immediately before dispatch because an earlier
/// synchronous callback in the same actor slot may replace that order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(in crate::engine) struct MeleeOwnerSelection {
    pub(in crate::engine) seq_id: crate::sequence::SequenceId,
    pub(in crate::engine) elem_idx: usize,
    pub(in crate::engine) order_id: std::num::NonZeroU32,
}

pub(super) const MELEE_ORDERS: &[crate::order::OrderType] = &[
    crate::order::OrderType::StrikingStraightSword,
    crate::order::OrderType::StrikingStraightStrongSword,
    crate::order::OrderType::ExecutingSword,
    crate::order::OrderType::StrikingLeftSword,
    crate::order::OrderType::StrikingRightSword,
    crate::order::OrderType::StrikingSemiroundLeftSword,
    crate::order::OrderType::StrikingSemiroundRightSword,
    crate::order::OrderType::StrikingRoundLeftSword,
    crate::order::OrderType::StrikingRoundRightSword,
];

/// Actor category contributing an action-execution rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ExecuteOverride {
    Actor,
    Human,
    Pc,
    Npc,
    Soldier,
    Civilian,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(super) enum ExecuteOwnerFamily {
    GenericAnimation,
    Movement,
    Melee,
    Bow,
    Ability,
    Beggar,
    WaitingSword,
}

/// Whether a derived owner arm publishes the sprite's raw motion through the
/// specialized Execute latch.
///
/// Original-game actor execution advances the sword-waiting sprite but
/// deliberately returns an in-progress result after swordfight evaluation.
/// Its sprite's looping terminal edge must therefore remain private to the
/// arm instead of completing the actor's lazy Wait element.
fn specialized_execute_uses_sprite_motion(family: ExecuteOwnerFamily) -> bool {
    !matches!(
        family,
        ExecuteOwnerFamily::GenericAnimation | ExecuteOwnerFamily::WaitingSword
    )
}

/// Whether the entry-latched human action branch reaches the synchronous
/// `WAITING_SWORD` swordfight work.
///
/// Original keys this work to the selected order after the common execution-
/// freeze and validity exits; it is not conditional on a later sprite helper
/// returning a completion record.
fn waiting_sword_execute_reaches_evaluation(
    selected_order_type: Option<crate::order::OrderType>,
    validity_short_circuited: bool,
    execution_frozen: bool,
) -> bool {
    selected_order_type == Some(crate::order::OrderType::WaitingSword)
        && !validity_short_circuited
        && !execution_frozen
}

#[cfg(test)]
#[test]
fn waiting_sword_evaluation_follows_entry_latched_execute_arm() {
    use crate::order::OrderType;

    assert!(waiting_sword_execute_reaches_evaluation(
        Some(OrderType::WaitingSword),
        false,
        false,
    ));
    assert!(!waiting_sword_execute_reaches_evaluation(
        Some(OrderType::WaitingSword),
        true,
        false,
    ));
    assert!(!waiting_sword_execute_reaches_evaluation(
        Some(OrderType::WaitingSword),
        false,
        true,
    ));
    assert!(!waiting_sword_execute_reaches_evaluation(
        Some(OrderType::WaitingUpright),
        false,
        false,
    ));
}

#[cfg(test)]
#[test]
fn waiting_sword_does_not_publish_its_sprite_terminal_edge() {
    assert!(!specialized_execute_uses_sprite_motion(
        ExecuteOwnerFamily::WaitingSword
    ));
    assert!(specialized_execute_uses_sprite_motion(
        ExecuteOwnerFamily::Movement
    ));
}

macro_rules! actor_execute_arm_catalog {
    ($emit:ident) => {
        $emit! {
            (Actor, WaitingUpright, GenericAnimation),
            (Actor, WaitingUprightBored, GenericAnimation),
            (Actor, WaitingUprightBoredRandom, GenericAnimation),
            (Actor, TransitionWaitingUprightBoredWaitingUpright, GenericAnimation),
            (Actor, TransitionWaitingUprightWaitingUprightBored, GenericAnimation),
            (Actor, TransitionWalkingUprightWaitingUpright, Movement),
            (Actor, TransitionRunningUprightWaitingUpright, Movement),
            (Actor, TransitionWaitingUprightWalkingUpright, Movement),
            (Actor, TransitionWaitingUprightRunningUpright, Movement),
            (Actor, TransitionWalkingUprightRunningUpright, Movement),
            (Actor, TransitionRunningUprightWalkingUpright, Movement),
            (Actor, TransitionWaitingCrouchedWalkingCrouched, Movement),
            (Actor, TransitionWalkingCrouchedWaitingCrouched, Movement),
            (Actor, TransitionCrouchingDown, GenericAnimation),
            (Actor, TransitionCrouchingUp, GenericAnimation),
            (Actor, TransitionWalkingUprightWalkingCrouched, Movement),
            (Actor, TransitionWalkingCrouchedWalkingUpright, Movement),
            (Actor, TransitionRunningUprightWalkingCrouched, Movement),
            (Actor, TransitionWalkingCrouchedRunningUpright, Movement),
            (Actor, Turning, GenericAnimation),
            (Actor, Freezing, GenericAnimation),
            (Actor, ClimbingLadderUp, Movement),
            (Actor, ClimbingLadderUpAlerted, Movement),
            (Actor, ClimbingLadderDown, Movement),
            (Actor, ClimbingLadderDownAlerted, Movement),
            (Actor, ClimbingLadderDownFast, Movement),
            (Actor, ClimbingLadderUpFast, Movement),
            (Actor, TransitionClimbingLadderUpWaitingCrouched, Movement),
            (Actor, TransitionClimbingLadderUpWaitingUprightAlerted, Movement),
            (Actor, TransitionWaitingCrouchedClimbingLadderDown, Movement),
            (Actor, TransitionWaitingUprightClimbingLadderDownAlerted, Movement),
            (Actor, TransitionWaitingUprightClimbingLadderUp, Movement),
            (Actor, TransitionWaitingUprightClimbingLadderUpAlerted, Movement),
            (Actor, TransitionClimbingLadderDownWaitingUpright, Movement),
            (Actor, TransitionClimbingLadderDownWaitingUprightAlerted, Movement),
            (Actor, ClimbingWallUp, Movement),
            (Actor, ClimbingWallDown, Movement),
            (Actor, ClimbingWallDownFast, Movement),
            (Actor, ClimbingWallUpFast, Movement),
            (Actor, TransitionClimbingWallUpWaitingCrouched, Movement),
            (Actor, TransitionClimbingWallUpWaitingCrouchedCrenel, Movement),
            (Actor, TransitionWaitingCrouchedClimbingWallDown, Movement),
            (Actor, TransitionWaitingCrouchedClimbingWallDownCrenel, Movement),
            (Actor, TransitionWaitingUprightClimbingWallUp, Movement),
            (Actor, TransitionClimbingWallDownWaitingUpright, Movement),
            (Actor, WalkingUpright, Movement),
            (Actor, RunningUpright, Movement),
            (Actor, WalkingStairs, Movement),
            (Actor, RunningStairs, Movement),
            (Actor, PassingDoor, Movement),
            (Actor, WaitingFreeLift, GenericAnimation),
            (Actor, PlayCustom, GenericAnimation),
            (Actor, PlayCustomFreeze, GenericAnimation),
            (Actor, PlayCustomFrozen, GenericAnimation),
            (Actor, PlayCustomLooped, GenericAnimation),
            (Actor, RefreshingSeek, Movement),
            (Human, Select, GenericAnimation),
            (Human, TransitionEquipBow, Bow),
            (Human, TransitionEquipBowAnonymous, Bow),
            (Human, TransitionUnequipBow, Bow),
            (Human, TransitionUnequipBowAnonymous, Bow),
            (Human, AimingWithBow, GenericAnimation),
            (Human, AimingWithBowAnonymous, GenericAnimation),
            (Human, AimingWithBowUp, GenericAnimation),
            (Human, AimingWithBowUpAnonymous, GenericAnimation),
            (Human, TransitionLoadingBow, Bow),
            (Human, TransitionLoadingBowAnonymous, Bow),
            (Human, TransitionUnloadBow, Bow),
            (Human, TransitionUnloadBowAnonymous, Bow),
            (Human, TransitionLoweringBow, Bow),
            (Human, TransitionLoweringBowAnonymous, Bow),
            (Human, TransitionRaisingBow, Bow),
            (Human, TransitionRaisingBowAnonymous, Bow),
            (Human, ShootingWithBow, Bow),
            (Human, ShootingWithBowAnonymous, Bow),
            (Human, ShootingWithBowUp, Bow),
            (Human, ShootingWithBowUpAnonymous, Bow),
            (Human, TransitionRaisingSword, GenericAnimation),
            (Human, TransitionLoweringSword, GenericAnimation),
            (Human, WaitingSword, WaitingSword),
            (Human, WalkingWithSword, Movement),
            (Human, RunningWithSword, Movement),
            (Human, TransitionWaitingSwordParryingSword, GenericAnimation),
            (Human, TransitionWaitingSwordParryingSwordLow, GenericAnimation),
            (Human, TransitionParryingSwordWaitingSword, GenericAnimation),
            (Human, ParryingLowSword, GenericAnimation),
            (Human, ParryingSword, GenericAnimation),
            (Human, DyingSword, GenericAnimation),
            (Human, DyingBow, GenericAnimation),
            (Human, BeingDeadSword, GenericAnimation),
            (Human, BeingDeadBow, GenericAnimation),
            (Human, BeingDead, GenericAnimation),
            (Human, FallingBackSword, GenericAnimation),
            (Human, FallingBackBow, GenericAnimation),
            (Human, BeingUnconsciousSword, GenericAnimation),
            (Human, BeingUnconsciousBow, GenericAnimation),
            (Human, BeingDeadFallenBackSword, GenericAnimation),
            (Human, BeingDeadFallenBackBow, GenericAnimation),
            (Human, BeingDeadFallenBack, GenericAnimation),
            // Smalltalk strikes have bespoke human action-execution semantics. The
            // generic animation arm owns their back-facing/sword-state hit
            // test; the ordinary melee sweep intentionally does not.
            (Human, StrikingLeftSmalltalk, GenericAnimation),
            (Human, StrikingRightSmalltalk, GenericAnimation),
            (Human, StrikingLowRightSmalltalk, GenericAnimation),
            (Human, StrikingLowLeftSmalltalk, GenericAnimation),
            (Human, ParryingLeftSmalltalk, GenericAnimation),
            (Human, ParryingRightSmalltalk, GenericAnimation),
            (Human, ParryingLowRightSmalltalk, GenericAnimation),
            (Human, ParryingLowLeftSmalltalk, GenericAnimation),
            (Human, StrikingStraightSword, Melee),
            (Human, StrikingStraightStrongSword, Melee),
            (Human, ExecutingSword, Melee),
            (Human, StrikingLeftSword, Melee),
            (Human, StrikingRightSword, Melee),
            (Human, StrikingSemiroundRightSword, Melee),
            (Human, StrikingSemiroundLeftSword, Melee),
            (Human, StrikingRoundRightSword, Melee),
            (Human, StrikingRoundLeftSword, Melee),
            (Human, StrikingDownSword, Melee),
            (Human, DyingUpright, GenericAnimation),
            (Human, StandingUpSword, GenericAnimation),
            (Human, StandingUp, GenericAnimation),
            (Human, StandingUpBow, GenericAnimation),
            (Human, FallingLadderWall, GenericAnimation),
            (Human, FallingBackUpright, GenericAnimation),
            (Human, FallingBackCrouched, GenericAnimation),
            (Human, BeingUnconscious, GenericAnimation),
            (Human, BeingHitSword, GenericAnimation),
            (Human, BeingWeakSword, GenericAnimation),
            (Human, ExtractingArrowSword, GenericAnimation),
            (Human, ExtractingArrowUpright, GenericAnimation),
            (Human, ExtractingArrowCrouched, GenericAnimation),
            (Human, ExtractingArrowBow, GenericAnimation),
            (Human, DyingCrouched, GenericAnimation),
            (Human, BeingStunnedSword, GenericAnimation),
            (Human, WakingUp, GenericAnimation),
            (Human, Provoking, GenericAnimation),
            (Human, Hitting, Ability),
            (Human, FallingHitHarderUpright, GenericAnimation),
            (Human, FallingHitHarderWithBow, GenericAnimation),
            (Human, FallingHitHarderWithSword, GenericAnimation),
            (Human, FallingHitHarderCrouched, GenericAnimation),
            (Human, FallingHitUpright, GenericAnimation),
            (Human, FallingHitWithBow, GenericAnimation),
            (Human, FallingHitWithSword, GenericAnimation),
            (Human, FallingHitCrouched, GenericAnimation),
            (Human, FallingPushedUpright, GenericAnimation),
            (Human, FallingPushedWithBow, GenericAnimation),
            (Human, FallingPushedWithSword, GenericAnimation),
            (Human, FallingPushedCrouched, GenericAnimation),
            (Human, BeingCarriedLittleJohn, GenericAnimation),
            (Human, BeingCarriedPeasantC, GenericAnimation),
            (Human, RaisingShield, GenericAnimation),
            (Human, LoweringShield, GenericAnimation),
            (Human, ParryingShield, GenericAnimation),
            (Human, WaitingShield, GenericAnimation),
            (Human, Rolling, GenericAnimation),
            (Human, LyingStuckUnderNet, GenericAnimation),
            (Human, WriggleUnderNet, GenericAnimation),
            (Human, BeingTied, GenericAnimation),
            (Human, TakingNet, GenericAnimation),
            (Human, GettingWounded, GenericAnimation),
            (Human, PassingDoor, Movement),
            (Human, TransitionWaitingUprightSpecial, GenericAnimation),
            (Human, TransitionSpecialWaitingUpright, GenericAnimation),
            (Human, Special, GenericAnimation),
            (Pc, WalkingWithSword, Movement),
            (Pc, RunningWithSword, Movement),
            (Pc, Select, GenericAnimation),
            (Pc, WalkingCrouched, Movement),
            (Pc, WaitingCrouched, GenericAnimation),
            (Pc, WalkingCarryingOnShoulders, Movement),
            (Pc, ShootingWithBow, Bow),
            (Pc, ShootingWithBowUp, Bow),
            (Pc, JumpingUp, Movement),
            (Pc, JumpingDown, Movement),
            (Pc, JumpingLong, Movement),
            (Pc, JumpingLongSword, Movement),
            (Pc, TransitionWaitingOnShouldersJumpingUp, Movement),
            (Pc, TransitionWaitingOnShouldersJumpingLong, Movement),
            (Pc, TransitionWaitingUprightJumpingUp, Movement),
            (Pc, TransitionJumpingUpWaitingCrouched, Movement),
            (Pc, WaitingCape, GenericAnimation),
            (Pc, WaitingCapeAnonymousArcher, GenericAnimation),
            (Pc, TransitionWaitingCapeWaitingUpright, GenericAnimation),
            (Pc, WaitingHidden, GenericAnimation),
            (Pc, TransitionWaitingHiddenWaitingUpright, GenericAnimation),
            (Pc, TransitionWaitingCrouchedJumpingDown, Movement),
            (Pc, TransitionJumpingDownWaitingCrouched, Movement),
            (Pc, TransitionWaitingUprightJumpingLong, Movement),
            (Pc, TransitionWaitingSwordJumpingLongSword, Movement),
            (Pc, TransitionJumpingLongWaitingUpright, Movement),
            (Pc, TransitionJumpingLongSwordWaitingSword, Movement),
            (Pc, Taking, GenericAnimation),
            (Pc, TakingCrouched, GenericAnimation),
            (Pc, Eating, Ability),
            (Pc, Whistling, Ability),
            (Pc, Searching, GenericAnimation),
            (Pc, SearchingCrouched, GenericAnimation),
            (Pc, Healing, Ability),
            (Pc, TransitionWaitingUprightHelpingClimbing, GenericAnimation),
            (Pc, TransitionHelpingClimbingWaitingUpright, GenericAnimation),
            (Pc, WaitingHelpingClimbing, GenericAnimation),
            (Pc, WaitingCarryingOnShoulders, GenericAnimation),
            (Pc, WaitingOnShoulders, GenericAnimation),
            (Pc, ClimbingUpOnShoulders, Ability),
            (Pc, ClimbingDownFromShoulders, Ability),
            (Pc, TransitionHelpingClimbingDown, Movement),
            (Pc, TransitionWaitingUprightCarryingCorpse, Ability),
            (Pc, TransitionCarryingCorpseWaitingUpright, Ability),
            (Pc, WaitingWithCorpse, GenericAnimation),
            (Pc, WalkingWithCorpse, Movement),
            (Pc, FallingShoulders, GenericAnimation),
            (Pc, TransitionWaitingCarryingOnShouldersWaitingUpright, GenericAnimation),
            (Pc, DroppingAmmo, GenericAnimation),
            (Pc, DroppingAmmoCrouched, GenericAnimation),
            (Pc, ThrowingApple, Ability),
            (Pc, ThrowingStone, Ability),
            (Pc, ThrowingPurse, Ability),
            (Pc, ThrowingWaspNest, Ability),
            (Pc, ThrowingNet, Ability),
            (Pc, RaisingShield, GenericAnimation),
            (Pc, LoweringShield, GenericAnimation),
            (Pc, WalkingWithShield, Movement),
            (Pc, WaitingShield, GenericAnimation),
            (Pc, HidingBehindShield, GenericAnimation),
            (Pc, UsingLever, GenericAnimation),
            (Pc, DroppingAle, GenericAnimation),
            (Pc, DroppingAleCrouched, GenericAnimation),
            (Pc, UnlockingDoor, GenericAnimation),
            (Pc, UnlockingTrap, GenericAnimation),
            (Pc, HandlingTarget, GenericAnimation),
            (Pc, HittingTarget, GenericAnimation),
            (Pc, TakingTarget, GenericAnimation),
            (Pc, Paying, Ability),
            (Pc, Tying, Ability),
            (Pc, Strangling, Ability),
            (Pc, TransitionWaitingUprightSimulatingBeggar, Ability),
            (Pc, TransitionSimulatingBeggarWaitingUpright, Ability),
            (Pc, SimulatingBeggar, Beggar),
            (Pc, TransitionWaitingUprightListening, Ability),
            (Pc, TransitionListeningWaitingUpright, Ability),
            (Pc, Listening, Ability),
            (Pc, TransitionRaisingSword, GenericAnimation),
            (Pc, Provoking, GenericAnimation),
            (Pc, StrikingLeftSmalltalk, GenericAnimation),
            (Pc, StrikingRightSmalltalk, GenericAnimation),
            (Pc, StrikingLowRightSmalltalk, GenericAnimation),
            (Pc, StrikingLowLeftSmalltalk, GenericAnimation),
            (Pc, StrikingRoundLeftSword, Melee),
            (Pc, StrikingRoundRightSword, Melee),
            (Pc, ExecutingSword, Melee),
            (Pc, ExtractingArrowUpright, GenericAnimation),
            (Pc, ExtractingArrowBow, GenericAnimation),
            (Pc, ExtractingArrowSword, GenericAnimation),
            (Npc, Sitting, GenericAnimation),
            (Npc, TransitionSittingWaitingUpright, GenericAnimation),
            (Npc, TransitionWaitingUprightSitting, GenericAnimation),
            (Npc, BeggarShowingFace, GenericAnimation),
            (Npc, Pointing, GenericAnimation),
            (Npc, Searching, GenericAnimation),
            (Soldier, WaitingAlerted, GenericAnimation),
            (Soldier, WaitingUpright, GenericAnimation),
            (Soldier, TransitionWaitingUprightWaitingAlerted, GenericAnimation),
            (Soldier, LookingLeft, GenericAnimation),
            (Soldier, LookingLeftAlerted, GenericAnimation),
            (Soldier, LookingRight, GenericAnimation),
            (Soldier, LookingRightAlerted, GenericAnimation),
            (Soldier, TransitionWaitingAlertedWaitingUpright, GenericAnimation),
            (Soldier, TransitionWaitingAlertedWaitingUprightOfficer, GenericAnimation),
            (Soldier, TransitionWalkingUprightWaitingUpright, Movement),
            (Soldier, TransitionRunningUprightWaitingUpright, Movement),
            (Soldier, TransitionWaitingUprightWalkingUpright, Movement),
            (Soldier, TransitionWaitingUprightRunningUpright, Movement),
            (Soldier, TransitionWalkingUprightRunningUpright, Movement),
            (Soldier, TransitionRunningUprightWalkingUpright, Movement),
            (Soldier, WalkingUpright, Movement),
            (Soldier, WalkingStairs, Movement),
            (Soldier, RunningStairs, Movement),
            (Soldier, Turning, GenericAnimation),
            (Soldier, StandingUpSword, GenericAnimation),
            (Soldier, TransitionRaisingSword, GenericAnimation),
            (Soldier, TransitionCharging, Melee),
            (Soldier, GettingFreeFromWasp, GenericAnimation),
            (Soldier, Taking, GenericAnimation),
            (Soldier, TransitionWaitingSwordMenacing, GenericAnimation),
            (Soldier, Menacing, GenericAnimation),
            (Soldier, SleepingUpright, GenericAnimation),
            (Soldier, TransitionSleepingWaitingUpright, GenericAnimation),
            (Soldier, GatheringSoldiers, GenericAnimation),
            (Soldier, TransitionMenacingWaitingSword, GenericAnimation),
            (Soldier, LeaningOut, GenericAnimation),
            (Soldier, TransitionWaitingAlertedLeaningOut, GenericAnimation),
            (Soldier, TransitionLeaningOutWaitingAlerted, GenericAnimation),
            (Soldier, DrinkingAle, GenericAnimation),
            (Soldier, TransitionLoweringBowLeaningOut, Bow),
            (Soldier, TransitionRaisingBowLeaningOut, Bow),
            (Soldier, AimingWithBowLeaningOut, GenericAnimation),
            (Soldier, ShootingWithBowLeaningOut, Bow),
            (Soldier, RunningUpright, Movement),
            (Soldier, RiderCharging, Movement),
            (Soldier, Special, GenericAnimation),
            (Civilian, WaitingUpright, GenericAnimation),
            (Civilian, WaitingUprightBored, GenericAnimation),
            (Civilian, WaitingUprightBoredRandom, GenericAnimation),
            (Civilian, TransitionWaitingUprightBoredWaitingUpright, GenericAnimation),
            (Civilian, TransitionWaitingUprightWaitingUprightBored, GenericAnimation),
            (Civilian, ReceivingPurse, Ability),
            (Civilian, WaitingWithPurse, Ability),
            (Civilian, TransitionWaitingWithPurseWaitingUpright, Ability),
        }
    };
}

macro_rules! define_actor_execute_catalog {
    ($(($override:ident, $order:ident, $owner:ident),)*) => {
        #[cfg(test)]
        pub(super) const ACTOR_EXECUTE_CATALOG: &[(ExecuteOverride, crate::order::OrderType, ExecuteOwnerFamily)] = &[
            $((ExecuteOverride::$override, crate::order::OrderType::$order, ExecuteOwnerFamily::$owner),)*
        ];

        pub(super) fn classify_actor_execute_arm(
            override_kind: ExecuteOverride,
            order: crate::order::OrderType,
        ) -> Option<ExecuteOwnerFamily> {
            match (override_kind, order) {
                $((ExecuteOverride::$override, crate::order::OrderType::$order) => Some(ExecuteOwnerFamily::$owner),)*
                _ => None,
            }
        }
    };
}
actor_execute_arm_catalog!(define_actor_execute_catalog);

pub(super) fn classify_live_actor_execute_arm(
    entity_id: EntityId,
    order: crate::order::OrderType,
) -> Option<ExecuteOwnerFamily> {
    let chain: &[ExecuteOverride] = match entity_id {
        EntityId::Pc(_) => &[
            ExecuteOverride::Pc,
            ExecuteOverride::Human,
            ExecuteOverride::Actor,
        ],
        EntityId::Soldier(_) => &[
            ExecuteOverride::Soldier,
            ExecuteOverride::Npc,
            ExecuteOverride::Human,
            ExecuteOverride::Actor,
        ],
        EntityId::Civilian(_) => &[
            ExecuteOverride::Civilian,
            ExecuteOverride::Npc,
            ExecuteOverride::Human,
            ExecuteOverride::Actor,
        ],
        _ => return None,
    };
    chain
        .iter()
        .find_map(|override_kind| classify_actor_execute_arm(*override_kind, order))
}

/// Motion state returned by a specialized derived Execute arm.
///
/// Most specialized owners forward the sprite result. The PC beggar idle is
/// an explicit exception: player-character execution performs the sprite
/// action and side effects, then always returns an in-progress result.
fn specialized_execute_motion(
    sprite_motion: Option<crate::sprite::MotionState>,
    selected_beggar: bool,
    movement_entity_target_seek: bool,
) -> Option<crate::sprite::MotionState> {
    if selected_beggar {
        Some(crate::sprite::MotionState::InProgress)
    } else if movement_entity_target_seek
        && sprite_motion
            .is_some_and(|motion| !matches!(motion, crate::sprite::MotionState::Terminated))
    {
        // Actor seek handling consumes non-terminal sprite results while an
        // entity target remains live. The surrounding movement Execute arm
        // observes IN_PROGRESS even though sprite motion recorded a
        // raw START or DONE edge.
        Some(crate::sprite::MotionState::InProgress)
    } else {
        sprite_motion
    }
}

pub(super) trait IntoExplicitExecuteMotion {
    fn into_explicit_execute_motion(self) -> ExplicitExecuteMotion;
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ExplicitExecuteMotion {
    pub initial: Option<crate::sprite::MotionState>,
    pub post_completion_override: Option<crate::sprite::MotionState>,
}

impl IntoExplicitExecuteMotion for () {
    fn into_explicit_execute_motion(self) -> ExplicitExecuteMotion {
        ExplicitExecuteMotion::default()
    }
}

impl IntoExplicitExecuteMotion for Option<crate::sprite::MotionState> {
    fn into_explicit_execute_motion(self) -> ExplicitExecuteMotion {
        ExplicitExecuteMotion {
            initial: self,
            post_completion_override: None,
        }
    }
}

impl IntoExplicitExecuteMotion for ExplicitExecuteMotion {
    fn into_explicit_execute_motion(self) -> ExplicitExecuteMotion {
        self
    }
}

fn apply_post_completion_execute_override(
    projected: crate::sprite::MotionState,
    post_completion_override: Option<crate::sprite::MotionState>,
    selected_element_interrupted: bool,
    installed_successor_exists: bool,
) -> crate::sprite::MotionState {
    if !selected_element_interrupted || installed_successor_exists {
        projected
    } else {
        post_completion_override.unwrap_or(projected)
    }
}

fn project_post_completion_motion(
    current: crate::sprite::MotionState,
    selected_element_impossible: bool,
    installed_successor_exists: bool,
    selected_specialized_order_advanced: bool,
) -> crate::sprite::MotionState {
    use crate::sprite::MotionState;
    if selected_element_impossible {
        MotionState::Aborted
    } else if installed_successor_exists
        && (current == MotionState::Terminated || selected_specialized_order_advanced)
    {
        MotionState::InProgress
    } else if selected_specialized_order_advanced {
        MotionState::Terminated
    } else {
        current
    }
}

fn motion_latch_debug_config() -> Option<&'static super::diagnostics::ExactOwnerFrame> {
    super::diagnostics::config().motion_latch.as_ref()
}

fn specialized_order_advanced_after_execute(
    execute_motion: Option<crate::sprite::MotionState>,
    selected_order_rewritten_by_stop: bool,
    selected_element_retired: bool,
    selected_element_interrupted: bool,
    selected_entry_order_still_current: bool,
) -> bool {
    execute_motion.is_some_and(|motion| motion != crate::sprite::MotionState::Aborted)
        && !selected_order_rewritten_by_stop
        // The original actor update latches the execution result before
        // line-crossing checks. A synchronous line callback may interrupt and
        // replace the selected sequence, but the later motion-state switch
        // still reads that already-held nonterminal result; interruption is
        // not execution-owned order advancement.
        && !selected_element_interrupted
        && (selected_element_retired || !selected_entry_order_still_current)
}

/// Stopping movement rewrites its first order in
/// place and assigns a new identity. That identity change is not order advancement: the
/// motion result already returned by `Execute` remains authoritative.
fn is_start_stop_movement_rewrite(
    entry_order_id: std::num::NonZeroU32,
    entry_order: crate::order::OrderType,
    live_order_id: std::num::NonZeroU32,
    live_order: crate::order::OrderType,
    execute_motion: crate::sprite::MotionState,
) -> bool {
    use crate::order::OrderType;

    matches!(
        execute_motion,
        crate::sprite::MotionState::Start
            | crate::sprite::MotionState::InProgress
            | crate::sprite::MotionState::Done
    )
        // Movement stopping assigns a new ID to the existing order. Runtime order IDs
        // are monotonic, whereas a translated stop-transition successor was
        // allocated before path waypoints that may later be inserted ahead of
        // it. This separates an in-place reseed from order advancement exposing an
        // already queued transition after a fresh waypoint reaches its goal.
        && live_order_id > entry_order_id
        && matches!(
            (entry_order, live_order),
            (
                OrderType::WalkingUpright,
                OrderType::TransitionWalkingUprightWaitingUpright
            ) | (
                OrderType::RunningUpright,
                OrderType::TransitionRunningUprightWaitingUpright
            ) | (
                OrderType::WalkingCrouched,
                OrderType::TransitionWalkingCrouchedWaitingCrouched
            )
        )
}

#[cfg(test)]
pub(super) fn assert_execute_owner_handler_is_linked(family: ExecuteOwnerFamily) {
    match family {
        ExecuteOwnerFamily::GenericAnimation => {
            let _ = EngineInner::tick_actor_animation_for;
        }
        ExecuteOwnerFamily::Movement => {
            let _ = EngineInner::tick_entity_movement_owner;
        }
        ExecuteOwnerFamily::Melee => {
            let _ = EngineInner::tick_selected_melee_owner;
        }
        ExecuteOwnerFamily::Bow => {
            let _ = EngineInner::tick_bow_shot_for;
        }
        ExecuteOwnerFamily::Ability => {}
        ExecuteOwnerFamily::Beggar => {
            let _ = EngineInner::tick_beggar_bid_for;
        }
        ExecuteOwnerFamily::WaitingSword => {
            let _ = EngineInner::tick_waiting_sword_execute_for;
        }
    }
}
// ─── Per-tick timing instrumentation ─────────────────────────────────
//
// Records the wall-clock duration of every `perform_hourglass` call
// and emits a periodic summary so we can see where the rollback
// checker's 25-replays-per-frame cost actually goes. Lives in a
// thread-local so the live tick and the rollback-replay ticks each get
// their own histogram (rollback runs on the same thread but typically
// happens in bursts of 25, so they'll dominate any window they hit).
thread_local! {
    static HOURGLASS_STATS: std::cell::RefCell<HourglassStats> =
        std::cell::RefCell::new(HourglassStats::default());
    static HOURGLASS_PHASE_STATS: std::cell::RefCell<HourglassPhaseStats> =
        std::cell::RefCell::new(HourglassPhaseStats::default());
}

/// Number of `perform_hourglass` calls between log lines.
const HOURGLASS_LOG_INTERVAL: u32 = 100;

/// Coarse, ordered phases of [`EngineInner::perform_hourglass_inner`].
///
/// Keep these deliberately broader than individual systems: the phase trace is
/// an ordering contract for the tick spine, not a second scheduler.  In
/// particular, `Paths` names the fixed completion/start barrier and failed-path
/// deadlines; movement dispatch only queues the requests resolved there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HourglassPhase {
    DeferredEffectsStart,
    MissionAndMessages,
    NpcOrders,
    Paths,
    Entities,
    EntitySystems,
    Npcs,
    GameplaySystems,
    Sequences,
    DeferredEffectsEnd,
}

#[derive(Default)]
struct HourglassPhaseStats {
    count: u32,
    total_us: [u128; 10],
}

/// Opt-in detail inside the otherwise broad `EntitySystems` phase.
#[derive(Clone, Copy)]
pub(super) enum EntitySystemDetail {
    PrepareNpc = 0,
    StaticOwners = 1,
    OwnerPrelude = 2,
    OwnerExecute = 3,
    NpcTail = 4,
    CorpseUpdates = 5,
    FrameSounds = 6,
    BuildEntityViews = 7,
    BuildWorldView = 8,
    RefreshDetection = 9,
}

const ENTITY_SYSTEM_DETAIL_COUNT: usize = 10;

#[derive(Default)]
struct EntitySystemDetailStats {
    frames: u32,
    calls: [u64; ENTITY_SYSTEM_DETAIL_COUNT],
    total_us: [u128; ENTITY_SYSTEM_DETAIL_COUNT],
}

thread_local! {
    static ENTITY_SYSTEM_DETAIL_STATS: std::cell::RefCell<EntitySystemDetailStats> =
        std::cell::RefCell::new(EntitySystemDetailStats::default());
}

pub(super) struct EntitySystemDetailGuard {
    phase: EntitySystemDetail,
    start: Option<web_time::Instant>,
}

impl Drop for EntitySystemDetailGuard {
    fn drop(&mut self) {
        let Some(start) = self.start else { return };
        let elapsed_us = start.elapsed().as_micros();
        ENTITY_SYSTEM_DETAIL_STATS.with(|cell| {
            let mut stats = cell.borrow_mut();
            let index = self.phase as usize;
            stats.calls[index] += 1;
            stats.total_us[index] += elapsed_us;
        });
    }
}

pub(super) fn entity_system_detail_guard(phase: EntitySystemDetail) -> EntitySystemDetailGuard {
    EntitySystemDetailGuard {
        phase,
        start: tracing::enabled!(
            target: "robin_engine::engine::tick::entity_system_perf",
            tracing::Level::INFO
        )
        .then(web_time::Instant::now),
    }
}

fn finish_entity_system_detail_frame() {
    if !tracing::enabled!(
        target: "robin_engine::engine::tick::entity_system_perf",
        tracing::Level::INFO
    ) {
        return;
    }
    ENTITY_SYSTEM_DETAIL_STATS.with(|cell| {
        let mut stats = cell.borrow_mut();
        stats.frames += 1;
        if stats.frames < HOURGLASS_LOG_INTERVAL {
            return;
        }
        let frames = u128::from(stats.frames);
        let per_frame = |phase: EntitySystemDetail| stats.total_us[phase as usize] / frames;
        let calls = |phase: EntitySystemDetail| stats.calls[phase as usize];
        tracing::info!(
            target: "robin_engine::engine::tick::entity_system_perf",
            frames = stats.frames,
            prepare_npc_us = per_frame(EntitySystemDetail::PrepareNpc),
            static_owners_us = per_frame(EntitySystemDetail::StaticOwners),
            owner_prelude_us = per_frame(EntitySystemDetail::OwnerPrelude),
            owner_execute_us = per_frame(EntitySystemDetail::OwnerExecute),
            npc_tail_us = per_frame(EntitySystemDetail::NpcTail),
            corpse_us = per_frame(EntitySystemDetail::CorpseUpdates),
            frame_sounds_us = per_frame(EntitySystemDetail::FrameSounds),
            build_views_us = per_frame(EntitySystemDetail::BuildEntityViews),
            build_views_calls = calls(EntitySystemDetail::BuildEntityViews),
            world_view_us = per_frame(EntitySystemDetail::BuildWorldView),
            world_view_calls = calls(EntitySystemDetail::BuildWorldView),
            detection_us = per_frame(EntitySystemDetail::RefreshDetection),
            detection_calls = calls(EntitySystemDetail::RefreshDetection),
            "entity systems detail timing"
        );
        *stats = EntitySystemDetailStats::default();
    });
}

fn time_hourglass_phase<T>(phase: HourglassPhase, f: impl FnOnce() -> T) -> T {
    trace_hourglass_phase(phase);
    let timer = tracing::enabled!(
        target: "robin_engine::engine::tick::phase_perf",
        tracing::Level::INFO
    )
    .then(web_time::Instant::now);
    let result = f();
    if let Some(timer) = timer {
        HOURGLASS_PHASE_STATS.with(|cell| {
            let mut stats = cell.borrow_mut();
            stats.total_us[phase as usize] += timer.elapsed().as_micros();
            if phase == HourglassPhase::DeferredEffectsEnd {
                stats.count += 1;
                if stats.count >= HOURGLASS_LOG_INTERVAL {
                    tracing::info!(
                        target: "robin_engine::engine::tick::phase_perf",
                        count = stats.count,
                        deferred_start_us = stats.total_us[0] / stats.count as u128,
                        mission_us = stats.total_us[1] / stats.count as u128,
                        npc_orders_us = stats.total_us[2] / stats.count as u128,
                        paths_us = stats.total_us[3] / stats.count as u128,
                        entities_us = stats.total_us[4] / stats.count as u128,
                        entity_systems_us = stats.total_us[5] / stats.count as u128,
                        npcs_us = stats.total_us[6] / stats.count as u128,
                        gameplay_us = stats.total_us[7] / stats.count as u128,
                        sequences_us = stats.total_us[8] / stats.count as u128,
                        deferred_end_us = stats.total_us[9] / stats.count as u128,
                        "perform_hourglass phase timing"
                    );
                    *stats = HourglassPhaseStats::default();
                }
            }
        });
    }
    result
}

#[cfg(test)]
thread_local! {
    static CAPTURED_HOURGLASS_PHASES: super::test_support::Probe<HourglassPhase> =
        const { super::test_support::Probe::new() };
}

fn trace_hourglass_phase(phase: HourglassPhase) {
    tracing::trace!(
        target: "robin_engine::engine::tick::phases",
        ?phase,
        "perform_hourglass phase"
    );
    #[cfg(test)]
    CAPTURED_HOURGLASS_PHASES.with(|probe| probe.record(phase));
}

#[cfg(test)]
pub(super) fn capture_hourglass_phases<T>(f: impl FnOnce() -> T) -> (T, Vec<HourglassPhase>) {
    CAPTURED_HOURGLASS_PHASES.with(|probe| probe.capture(f))
}

#[test]
fn hourglass_observer_nested_captures_keep_their_own_order() {
    trace_hourglass_phase(HourglassPhase::MissionAndMessages);
    let (value, outer) = capture_hourglass_phases(|| {
        trace_hourglass_phase(HourglassPhase::DeferredEffectsStart);
        let (nested_value, nested) = capture_hourglass_phases(|| {
            trace_hourglass_phase(HourglassPhase::MissionAndMessages);
            17
        });
        assert_eq!(nested_value, 17);
        assert_eq!(nested, [HourglassPhase::MissionAndMessages]);
        trace_hourglass_phase(HourglassPhase::MissionAndMessages);
        23
    });
    assert_eq!(value, 23);
    assert_eq!(
        outer,
        [
            HourglassPhase::DeferredEffectsStart,
            HourglassPhase::MissionAndMessages
        ]
    );
    assert!(capture_hourglass_phases(|| ()).1.is_empty());
}

#[cfg(test)]
thread_local! {
    static CAPTURED_ORDERED_GAMEPLAY_ENTITIES: super::test_support::Probe<EntityId> =
        const { super::test_support::Probe::new() };
}

fn observe_ordered_gameplay_entity(entity_id: EntityId) {
    tracing::trace!(
        target: "robin_engine::engine::tick::ordered_gameplay",
        ?entity_id,
        "ordered gameplay slot"
    );
    #[cfg(test)]
    CAPTURED_ORDERED_GAMEPLAY_ENTITIES.with(|probe| probe.record(entity_id));
}

#[cfg(test)]
pub(super) fn capture_ordered_gameplay_entities<T>(f: impl FnOnce() -> T) -> (T, Vec<EntityId>) {
    CAPTURED_ORDERED_GAMEPLAY_ENTITIES.with(|probe| probe.capture(f))
}

/// Move exclamations whose decoded-duration deadline has arrived into
/// the callback queue consumed as the first mutation of the next
/// simulation-tick deferred-effects phase.
pub(super) fn drain_matured_exclamations(
    sound_sim: &mut crate::sound::SoundSimState,
    cur_frame: u32,
) {
    let mut still_playing = Vec::new();
    let mut finished = Vec::new();
    for p in sound_sim.playing_exclamations.drain(..) {
        if p.finish_frame <= cur_frame {
            finished.push((p.actor_id, p.exclamation_id));
        } else {
            still_playing.push(p);
        }
    }
    sound_sim.playing_exclamations = still_playing;
    sound_sim.finished_exclamations = finished;
}

#[derive(Default)]
struct HourglassStats {
    count: u32,
    total_us: u128,
    min_us: u128,
    max_us: u128,
}

impl HourglassStats {
    fn record(&mut self, us: u128) {
        if self.count == 0 {
            self.min_us = us;
            self.max_us = us;
        } else {
            self.min_us = self.min_us.min(us);
            self.max_us = self.max_us.max(us);
        }
        self.count += 1;
        self.total_us += us;
    }

    fn flush(&mut self) {
        if self.count == 0 {
            return;
        }
        let avg = self.total_us / self.count as u128;
        tracing::info!(
            target: "robin_engine::engine::tick::perf",
            count = self.count,
            avg_us = avg,
            min_us = self.min_us,
            max_us = self.max_us,
            "perform_hourglass timing"
        );
        *self = Self::default();
    }
}

/// RAII guard: timer.start() at construction, records on drop. Logs a
/// summary every `HOURGLASS_LOG_INTERVAL` ticks.
struct HourglassTimer {
    start: web_time::Instant,
}

impl HourglassTimer {
    fn start() -> Option<Self> {
        if !tracing::enabled!(target: "robin_engine::engine::tick::perf", tracing::Level::INFO) {
            return None;
        }
        Some(Self {
            start: web_time::Instant::now(),
        })
    }
}

impl Drop for HourglassTimer {
    fn drop(&mut self) {
        let us = self.start.elapsed().as_micros();
        HOURGLASS_STATS.with(|cell| {
            let mut s = cell.borrow_mut();
            s.record(us);
            if s.count >= HOURGLASS_LOG_INTERVAL {
                s.flush();
            }
        });
    }
}

impl EngineInner {
    pub(crate) fn perform_frame_hourglass(
        &mut self,
        assets: &LevelAssets,
        simulation_body_allowed: bool,
    ) -> super::SideEffects {
        let mut display = std::mem::take(&mut self.feedback.cutscene_camera.display);
        let effects =
            self.perform_hourglass_authoritative(&mut display, assets, simulation_body_allowed);
        self.feedback.cutscene_camera.display = display;
        effects
    }

    pub(crate) fn perform_frame_post_initialize(
        &mut self,
        assets: &LevelAssets,
    ) -> Option<super::SideEffects> {
        // Keep the existing placeholder/restoration boundary around script callbacks.
        let display = std::mem::take(&mut self.feedback.cutscene_camera.display);
        let effects = self.perform_post_initialize_authoritative(assets);
        self.feedback.cutscene_camera.display = display;
        effects
    }

    /// Expose the exact actor/sprite/sequence identities around the PC Drop
    /// Execute boundary without changing any authoritative state.
    fn debug_drop_owner_boundary(
        &self,
        phase: &'static str,
        owner: EntityId,
        selected_order: Option<(crate::sequence::SequenceId, usize, std::num::NonZeroU32)>,
    ) {
        let frame = self.control.frame_counter;
        if !drop_owner_boundary_matches(frame, owner) {
            return;
        }
        let entity = self
            .world
            .entities
            .get(owner)
            .unwrap_or_else(|| panic!("Drop boundary owner {owner:?} disappeared"));
        let actor = entity
            .actor_data()
            .unwrap_or_else(|| panic!("Drop boundary owner {owner:?} is not an actor"));
        let ability = crate::abilities::selected_ability(
            &self.world.entities,
            &self.orders.sequence_manager,
            owner,
        );
        let selected_state = selected_order.and_then(|(seq, elem, _)| {
            self.orders
                .sequence_manager
                .get_element(seq, elem)
                .map(|element| element.state)
        });
        eprintln!(
            "DROPBOUND frame={frame} phase={phase} owner={owner:?} execute_initialising={} ability={ability:?} selected={selected_order:?} selected_state={selected_state:?} installed={:?} actor_last_execute={:?} sprite_last_processed={} sprite_action={:?}",
            actor.execute_order_initialising,
            actor.installed_order,
            actor.last_execute_order_id,
            entity.element_data().sprite.last_processed_order_id,
            entity.element_data().sprite.last_action,
        );
    }
    // ─── Main update tick ────────────────────────────────────────

    /// Test-only adapter for the main per-frame logic update.
    ///
    /// Returns the game state code — normally `LevelInProgress`, but can
    /// return `LevelSucceeded`, `LevelFailed`, or `LevelInterrupted` to
    /// signal that the mission is over.
    ///
    /// Production callers must use [`super::rollback_safe::Engine::advance_frame`].
    /// Low-level engine tests use this adapter to preserve the legacy
    /// command/hourglass boundary while applying emitted host events to an
    /// explicit caller-owned input state.
    ///
    /// Called once per frame from the game loop, gated by:
    /// - console not displayed
    /// - no UI transition in progress
    /// - not paused
    /// - not in LEVEL_NEXT or LEVEL_LOAD state
    ///
    /// Supplies [`EngineInner::perform_hourglass_inner`] with an explicit
    /// simulation context and drains the deferred sound queue so all
    /// gameplay-affecting randomness is pulled from the engine-owned stream
    /// (deterministic across clients) and all audio is
    /// flushed *after* the sim is done (letting rollback replay the tick
    /// without duplicating playback).
    #[cfg(test)]
    pub(crate) fn perform_hourglass(
        &mut self,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        dev: &mut DevState,
    ) -> super::SideEffects {
        let mut camera = self.feedback.cutscene_camera.display.clone();
        let effects = self.perform_hourglass_authoritative(&mut camera, assets, true);
        self.feedback.cutscene_camera.display = camera;
        for event in effects.host_events.iter().cloned() {
            display.apply_host_event(input, event);
        }
        if dev.projectile_cheat_rain >= 0 {
            dev.projectile_cheat_rain = -1;
        }
        effects
    }

    /// Run an hourglass while optionally forcing the simulation-body gate
    /// closed for this tick.
    ///
    /// A closed gate still runs the mission script/message phase and advances
    /// the mission clock, exactly like the engine's persistent lock, but does
    /// not mutate that persistent lock state.
    fn perform_hourglass_authoritative(
        &mut self,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        simulation_body_allowed: bool,
    ) -> super::SideEffects {
        let _hourglass_timer = HourglassTimer::start();

        let sim = self.control.simulation_context();
        let sim = &sim;

        // The game records parity immediately after the simulation update, then its
        // render pass calls each element's Refresh. Reproduce the resulting
        // arrow and frame-sound mutations here, before the next engine frame.
        // A restored mission starts with no pending pass because its serialized
        // sprites already crossed the preceding Refresh boundary.
        self.apply_pending_presentation_refresh(sim);

        // Fade-to-black presents its ramp in a tight loop without
        // advancing simulation. Drain the corresponding presentation
        // count before lending the explicit simulation context or touching any simulation,
        // display-state, or sound timer. A frame-counter deadline cannot
        // represent this: advancing that clock would mature every deadline
        // that is supposed to remain frozen during the blocking native.
        if self.consume_fade_freeze_frame() {
            let mut fx = self.feedback.drain_side_effects();
            fx.code = GameCode::LevelInProgress;
            // Fast-forward render skipping must not strand the host fade.
            fx.skip_render = false;
            return fx;
        }

        let code = self.perform_hourglass_inner(sim, display, assets, simulation_body_allowed);
        self.refresh_achievement_progress(assets);
        self.advance_auto_quick_action_queues(sim, display, assets);
        self.refresh_fog_of_war(assets, false);
        self.control.arrow_refresh_pending = true;

        // Post-tick sim mutations that used to live in `game_session`
        // between the hourglass and the render pass. They have to run
        // inside `perform_hourglass` for rollback determinism: replay
        // only re-runs `perform_hourglass`, so anything advancing engine
        // state outside it would diverge from the live timeline.
        // Forbidden-expression timers age in the Original's per-frame PC
        // render refresh, which runs after the whole simulation frame.  Keep
        // the decrement here (not inside a mid-hourglass melee phase) so a
        // bark queued by any hourglass phase still ages this frame; otherwise
        // the 75-frame forbid window ends one frame late and a repeat bark
        // the Original accepted at exactly +75 frames is wrongly rejected.
        self.tick_refresh_hero_mouth();
        self.feedback
            .pending_side_effects
            .host_events
            .push(HostEvent::Minimap(MinimapHostEvent::Tick));
        self.feedback
            .pending_side_effects
            .host_events
            .push(HostEvent::MacroUi(MacroUiHostEvent::Tick {
                slots: self.macro_slot_lengths(),
                pc_ids: self.world.pc_ids.clone(),
            }));
        // Advance destination-marker animation and retire finished
        // marks.  Used to run during rendering, which broke rollback
        // determinism — the render path is now read-only.
        {
            let view_pos = self.feedback.cutscene_camera.view_position;
            let zoom = self.feedback.cutscene_camera.zoom_factor;
            let screen = Self::director_camera_view_size();
            let screen_w = screen.x as i32;
            let screen_h = screen.y as i32;
            let frame_counter = self.control.frame_counter;
            self.feedback.ground_mark.tick(
                view_pos.to_geo(),
                zoom,
                screen_w,
                screen_h,
                frame_counter,
            );
        }
        // Sound-source delay state machine. Original queues playback at zero
        // and re-rolls only when that playback finishes,
        // so keep a deterministic sim-side finish deadline rather than
        // consuming gameplay RNG immediately when playback starts.
        let num_sources = self.feedback.sound_sim.sources.num_sources();
        for i in 0..num_sources {
            let Some(src) = self.feedback.sound_sim.sources.get_mut(i) else {
                continue;
            };
            if !src.is_effectively_active()
                || src.source_kind != crate::sound_source::SoundSourceKind::Delayed
            {
                continue;
            }
            if src.timer > 0 {
                src.timer -= 1;
            }
            if src.timer == 0 {
                if self
                    .feedback
                    .sound_sim
                    .playing_sources
                    .iter()
                    .any(|playing| playing.source_index as usize == i)
                {
                    continue;
                }
                let duration = assets
                    .audio
                    .source_durations
                    .get(&src.id)
                    .copied()
                    .unwrap_or(0);
                self.feedback
                    .sound_sim
                    .playing_sources
                    .push(crate::sound::PlayingSource {
                        source_index: i as u32,
                        finish_frame: self.control.frame_counter + duration,
                    });
                self.feedback
                    .pending_side_effects
                    .sounds
                    .push(super::SoundCommand::PlayDelayedSource(i));
            }
        }

        // `perform_frame_hourglass` temporarily moves the authoritative
        // camera display state into this argument. Advance that exact value;
        // taking `cutscene_camera.display` again here would tick a fresh
        // default and then overwrite it when the outer value is restored.
        let skip_render = self.tick_display_state(sim, assets, display);

        // Original's portrait refresh mirrors these fields from canonical
        // profile/status/interface state. Event-driven open, burn, and
        // quick-icon fields are intentionally not derived here.
        let portrait_updates: Vec<_> = self
            .world
            .pc_ids
            .iter()
            .copied()
            .map(|pc_id| {
                let pc = self
                    .get_entity(pc_id)
                    .and_then(|entity| entity.pc_data())
                    .unwrap_or_else(|| panic!("PC list entry {pc_id:?} is not a PC"));
                let profile = assets
                    .profile_manager
                    .get_character(pc.profile_index)
                    .unwrap_or_else(|| {
                        panic!("PC {pc_id:?} has missing profile {}", pc.profile_index)
                    });
                let description = self
                    .pc_description_for_pc_data(pc)
                    .unwrap_or_else(|| panic!("PC {pc_id:?} has no campaign description"));
                (
                    pc_id,
                    profile
                        .actions
                        .map(|action| description.status.get_ammo(action)),
                    profile.actions[2] == crate::profiles::Action::NoAction,
                    !pc.interface_hidden,
                    f32::from(pc.life_points),
                    pc.trumpet_enabled,
                )
            })
            .collect();
        for (pc_id, quantities, two_buttons, displayed, life, trumpet) in portrait_updates {
            let pc = self
                .get_entity_mut(pc_id)
                .and_then(|entity| entity.pc_data_mut())
                .unwrap_or_else(|| panic!("PC list entry {pc_id:?} is not a PC"));
            pc.portrait.quantities = quantities;
            pc.portrait.two_buttons_mode = two_buttons;
            pc.portrait.displayed = displayed;
            pc.portrait.life_level = life;
            pc.portrait.trumpet_enabled = trumpet;
        }

        // Reset per-frame scroll dedupe after the camera display tick.
        // Host-local viewport scroll is host-side and never enters engine
        // state, so peer-2's held scroll doesn't gate the host's, and vice
        // versa.
        display.frame_scrolled = [false; 4];

        let mut fx = self.feedback.drain_side_effects();
        fx.code = code;
        // The trigger tick supplies the first FadeToBlack presentation.
        // Force that render even when the camera state machine requested a
        // fast-forward skip; the remaining presentations are forced by the
        // early-return path above.
        let starts_fade = matches!(fx.fade_to_black, Some(Some(_)));
        fx.skip_render = !starts_fade && skip_render != 0;
        fx
    }

    /// Apply the sprite mutations performed by the preceding original-game
    /// refresh, after its parity snapshot and before the next tick.
    pub(crate) fn apply_pending_presentation_refresh(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
    ) {
        if !std::mem::take(&mut self.control.arrow_refresh_pending) {
            return;
        }

        self.refresh_arrows_for_presentation(sim);
        {
            let _detail = entity_system_detail_guard(EntitySystemDetail::FrameSounds);
            self.dispatch_frame_sounds();
        }
    }

    /// Run the arrow portion of the game refresh immediately.
    ///
    /// Besides the ordinary post-snapshot refresh, Original can re-enter
    /// the refresh while constructing an in-game modal.
    /// Dialogue commands do that synchronously, so a newly-created arrow can
    /// publish its orientation before the same frame's parity snapshot.
    pub(crate) fn refresh_arrows_for_presentation(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
    ) {
        // Refresh walks the full display-sort result. Its FX-polyline merge
        // can interleave (and even reverse) two non-animation arrows that an
        // arrow-only depth sort would leave together. That exact order is
        // authoritative because every falling-arrow Refresh consumes one
        // global RNG draw.
        let arrows: Vec<_> = self
            .compute_display_order()
            .ids
            .into_iter()
            .filter(|&id| {
                matches!(
                    self.world.entities.get(id),
                    Some(Entity::Projectile(projectile))
                        if projectile.object.object_type == crate::element::ObjectType::Arrow
                )
            })
            .collect();

        for id in arrows {
            let Some(Entity::Projectile(projectile)) = self.world.entities.get_mut(id) else {
                panic!("arrow {id:?} vanished during deferred Refresh");
            };
            crate::bow_shot::refresh_arrow_after_previous_hourglass(sim, projectile);
        }
    }

    /// Run the one-shot mission-script `PostInitialize` stage.
    ///
    /// The original game loop calls this after the first refresh and sound
    /// update, not from inside the engine tick. The host therefore invokes this
    /// explicit stage after its first refresh/sound boundary.  Rollback
    /// replay invokes the same stage after replaying frame zero so the
    /// resulting pre-frame-one simulation state remains deterministic.
    fn perform_post_initialize_authoritative(
        &mut self,
        assets: &LevelAssets,
    ) -> Option<super::SideEffects> {
        if !self.control.sim_config.script_enabled
            || self.script_domains.mission_ui.game_post_initialized
        {
            return None;
        }

        // The game latch advances even without a mission callback. Preserve
        // the no-VM boundary: no refresh, RNG lease, or effects are consumed.
        if self.scripts.mission.is_none() {
            self.script_domains.mission_ui.game_post_initialized = true;
            // Completion is authoritative even with no callback effects:
            // the host records Some as the replay's post-initialize stage bit.
            return Some(super::SideEffects {
                code: GameCode::LevelInProgress,
                ..Default::default()
            });
        }

        // PostInitialize can call randomising natives, so keep it on the same
        // engine-owned deterministic stream while moving only the scheduling
        // boundary.
        let sim = self.control.simulation_context();
        let sim = &sim;

        // This explicit host stage is defined to run after the first native
        // Refresh. Cross the same pending presentation boundary before
        // PostInitialize can consume RNG or inspect sprite state.
        self.apply_pending_presentation_refresh(sim);

        self.run_post_initialize_if_needed(sim, assets);

        let mut fx = self.feedback.drain_side_effects();
        fx.code = GameCode::LevelInProgress;
        Some(fx)
    }

    #[cfg(test)]
    pub(crate) fn perform_post_initialize(
        &mut self,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
    ) -> Option<super::SideEffects> {
        let camera = self.feedback.cutscene_camera.display.clone();
        let effects = self.perform_post_initialize_authoritative(assets);
        self.feedback.cutscene_camera.display = camera;
        if let Some(effects) = &effects {
            let mut input = InputState::default();
            for event in effects.host_events.iter().cloned() {
                display.apply_host_event(&mut input, event);
            }
        }
        effects
    }

    /// Whether any PC is currently guarded.
    pub fn is_pc_guarded(&self) -> bool {
        for &pc_id in &self.world.pc_ids {
            if let Some(Entity::Pc(pc)) = self.get_entity(pc_id)
                && pc.pc.guard.is_some()
            {
                return true;
            }
        }
        false
    }

    fn perform_hourglass_inner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        simulation_body_allowed: bool,
    ) -> GameCode {
        let pc_guarded = time_hourglass_phase(HourglassPhase::DeferredEffectsStart, || {
            self.hourglass_phase_deferred_effects_start(sim, assets)
        });

        if let Some(code) = time_hourglass_phase(HourglassPhase::MissionAndMessages, || {
            self.hourglass_phase_mission_and_messages(
                sim,
                display,
                assets,
                pc_guarded,
                simulation_body_allowed,
            )
        }) {
            return code;
        }

        time_hourglass_phase(HourglassPhase::NpcOrders, || {
            self.hourglass_phase_npc_orders(sim, assets)
        });

        time_hourglass_phase(HourglassPhase::Paths, || {
            self.hourglass_phase_paths(sim, assets)
        });

        let was_swordfighting =
            time_hourglass_phase(HourglassPhase::Entities, || self.hourglass_phase_entities());

        time_hourglass_phase(HourglassPhase::EntitySystems, || {
            self.hourglass_phase_entity_systems(sim, assets)
        });

        time_hourglass_phase(HourglassPhase::Npcs, || self.hourglass_phase_npcs());

        time_hourglass_phase(HourglassPhase::GameplaySystems, || {
            self.hourglass_phase_gameplay_systems(sim, display, assets)
        });

        time_hourglass_phase(HourglassPhase::Sequences, || {
            self.hourglass_phase_sequences_authoritative(sim, assets)
        });

        time_hourglass_phase(HourglassPhase::DeferredEffectsEnd, || {
            self.hourglass_phase_deferred_effects_end(sim, assets, was_swordfighting)
        });

        GameCode::LevelInProgress
    }

    /// Prove that a live host speech completion came from the sealed timing
    /// catalog admitted before ranked engine construction. The concrete audio
    /// sample is presentation-only; the duration is the only selected value
    /// that enters simulation state. Explicit variants therefore bind one
    /// exact ordered catalog entry; random playback uses the group's longest
    /// English duration, independent of the local audio variant.
    fn validate_ranked_speech_resolution(
        assets: &LevelAssets,
        pending: &crate::sound::PendingExclamation,
        resolution: &crate::sound::ResolvedExclamation,
    ) -> Result<(), String> {
        let identifier = (pending.profile_id & 0xFFFF_0000) | u32::from(pending.exclamation_id);
        let group = assets
            .audio
            .speech_timing_catalog
            .groups
            .get(&identifier)
            .ok_or_else(|| {
                format!("ranked sound resolution {identifier:#010x} has no sealed timing group")
            })?;
        if group.variants.is_empty() {
            return Err(format!(
                "ranked sound timing group {identifier:#010x} has no authored variants"
            ));
        }

        let duration_matches = match pending.variant {
            -1 => {
                group
                    .variants
                    .iter()
                    .filter_map(|variant| variant.duration_frames)
                    .max()
                    == Some(resolution.duration_frames)
            }
            explicit if explicit >= 0 => {
                let variant_index = usize::try_from(explicit).map_err(|_| {
                    format!(
                        "ranked sound variant {explicit} for {identifier:#010x} is not representable"
                    )
                })?;
                let variant = group.variants.get(variant_index).ok_or_else(|| {
                    format!(
                        "ranked sound variant {variant_index} is outside the {} authored variants for {identifier:#010x}",
                        group.variants.len()
                    )
                })?;
                let expected = variant.duration_frames.ok_or_else(|| {
                    format!(
                        "ranked sound variant {variant_index} for {identifier:#010x} has no authoritative duration"
                    )
                })?;
                expected == resolution.duration_frames
            }
            invalid => {
                return Err(format!(
                    "ranked sound request {identifier:#010x} has invalid variant {invalid}"
                ));
            }
        };
        if !duration_matches {
            return Err(format!(
                "ranked sound resolution {identifier:#010x} supplied unauthoritative duration {}",
                resolution.duration_frames
            ));
        }
        Ok(())
    }

    /// Construct the path scheduler from exact leaf borrows of its two
    /// persistent owners. Cross-domain consequences deliberately remain in
    /// [`Self::hourglass_phase_paths`] after each scheduler operation returns.
    fn path_schedule_context(&mut self) -> PathScheduleContext<'_> {
        let frame_counter = self.control.frame_counter;
        let (entities, fast_grid, pathfinder) = self.world.path_schedule_parts();
        let (pending, failed, sequence_manager) = self.orders.path_schedule_parts();
        PathScheduleContext::new(
            frame_counter,
            entities,
            fast_grid,
            pathfinder,
            pending,
            failed,
            sequence_manager,
        )
    }

    fn trace_path_barrier(&self, stage: &str) {
        if !super::diagnostics::config().path_barrier {
            return;
        }
        let pending = self
            .orders
            .pending_path_requests
            .parity_state(&self.world.fast_grid);
        let brief: Vec<_> = pending
            .1
            .iter()
            .map(|entry| {
                (
                    entry.request.actor,
                    entry.sequence_id,
                    entry.element_index,
                    entry.in_flight,
                    entry.waypoints.as_ref().map(|w| w.len()),
                )
            })
            .collect();
        eprintln!(
            "[PATH_BARRIER frame={} stage={stage} ignore={} queue={brief:?}]",
            self.control.frame_counter, pending.0
        );
    }

    fn trace_path_barrier_completed(&self, stage: &str, completed: &Option<CompletedPathWork>) {
        if !super::diagnostics::config().path_barrier {
            return;
        }
        let brief = completed.as_ref().map(|work| match work {
            CompletedPathWork::Ready { request, waypoints } => (
                "ready",
                request.owner,
                request.seq_id,
                request.elem_idx,
                waypoints.len(),
            ),
            CompletedPathWork::Failed(request) => {
                ("failed", request.owner, request.seq_id, request.elem_idx, 0)
            }
        });
        eprintln!(
            "[PATH_BARRIER frame={} stage={stage} completed={brief:?}]",
            self.control.frame_counter
        );
    }

    fn apply_completed_path_work(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        completed: Option<CompletedPathWork>,
    ) {
        if let Some(owner) = completed.as_ref().map(|work| match work {
            CompletedPathWork::Ready { request, .. } | CompletedPathWork::Failed(request) => {
                request.owner
            }
        }) {
            assert!(
                self.world.entities.get(owner).is_some(),
                "completed path request for {owner:?} retains a live sequence element but its owner entity is missing"
            );
        }
        match completed {
            Some(CompletedPathWork::Ready { request, waypoints }) => {
                if let Some(element) = self
                    .orders
                    .sequence_manager
                    .get_element_mut(request.seq_id, request.elem_idx)
                {
                    element.command = crate::element::Command::MoveOk;
                }
                self.finish_move_path(sim, request, waypoints);
            }
            Some(CompletedPathWork::Failed(request)) => {
                tracing::warn!(
                    actor = ?request.owner,
                    seq_id = ?request.seq_id,
                    elem_idx = request.elem_idx,
                    src_x = request.source.x,
                    src_y = request.source.y,
                    dst_x = request.dest.x,
                    dst_y = request.dest.y,
                    layer = request.layer,
                    sector = request.sector,
                    "path scheduling barrier: pathfind FAILED",
                );
                if let Some(fallback) = self.tactical_path_failure_fallback(request.owner) {
                    if let Some(element) = self
                        .orders
                        .sequence_manager
                        .get_element_mut(request.seq_id, request.elem_idx)
                    {
                        element.command = crate::element::Command::MoveOk;
                    }
                    self.element_impossible(
                        sim,
                        assets,
                        &mut Vec::new(),
                        request.seq_id,
                        request.elem_idx,
                    );
                    if let Some(destination) = fallback {
                        tracing::info!(
                            actor = ?request.owner,
                            failed_x = request.dest.x,
                            failed_y = request.dest.y,
                            fallback_x = destination.x,
                            fallback_y = destination.y,
                            "allied formation slot unreachable; moving toward shared command center",
                        );
                        self.perform_group_move(
                            sim,
                            assets,
                            &[request.owner],
                            destination,
                            false,
                            false,
                            None,
                            None,
                            None,
                            &[],
                            &[],
                        );
                    }
                } else {
                    self.orders.failed_path_requests.push(
                        super::movement::FailedPathRequest::from_pending(
                            request,
                            self.control.frame_counter,
                        ),
                    );
                }
            }
            None => {}
        }
    }

    /// Execute a mobile element at its first masked-effect child's
    /// creation slot, then execute that one child. Later child slots animate
    /// only themselves and therefore cannot retrigger the master.
    fn tick_mobile_child_owner_boundary(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        child_id: EntityId,
    ) -> bool {
        let Some(mobile_index_u16) = self
            .world
            .entities
            .get(child_id)
            .and_then(crate::element::Entity::as_fx)
            .and_then(|fx| fx.fx.mobile_index)
        else {
            return false;
        };
        let mobile_index = usize::from(mobile_index_u16);
        let (first_child, child_offset) = {
            let mobile = self
                .world
                .mobile_elements
                .get(mobile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "mobile child {child_id} at its update slot references missing master index {mobile_index}"
                    )
                });
            let first_child = *mobile.sprite_ids.first().unwrap_or_else(|| {
                panic!("mobile {mobile_index} has no first masked child for its owner boundary")
            });
            let child_offset = mobile
                .sprite_ids
                .iter()
                .position(|&candidate| candidate == child_id)
                .unwrap_or_else(|| {
                    panic!(
                        "FXMasked child {child_id} claims mobile {mobile_index}, but the master does not own it"
                    )
                });
            (first_child, child_offset)
        };

        if child_id == first_child {
            let first_slot = child_id.index();
            let sprite_ids = self.world.mobile_elements[mobile_index].sprite_ids.clone();
            for (offset, &expected_child) in sprite_ids.iter().enumerate() {
                let slot = first_slot.checked_add(offset as u32).unwrap_or_else(|| {
                    panic!(
                        "mobile {mobile_index} child adjacency overflows after slot {first_slot}"
                    )
                });
                let actual_child = self.world.entities.id_at_legacy_slot(slot).unwrap_or_else(|| {
                    panic!(
                        "mobile {mobile_index} child {expected_child} is missing from required adjacent slot {slot}"
                    )
                });
                assert_eq!(
                    actual_child, expected_child,
                    "mobile {mobile_index} child {expected_child} expected at adjacent slot {slot}, found {actual_child}"
                );
                let actual_index = self
                    .world
                    .entities
                    .get(actual_child)
                    .and_then(crate::element::Entity::as_fx)
                    .unwrap_or_else(|| {
                        panic!(
                            "mobile {mobile_index} child {actual_child} at adjacent slot {slot} is missing or non-FX"
                        )
                    })
                    .fx
                    .mobile_index;
                assert_eq!(
                    actual_index,
                    Some(mobile_index_u16),
                    "mobile {mobile_index} child {actual_child} at adjacent slot {slot} has wrong master index {actual_index:?}"
                );
            }

            let path_index = self.world.mobile_elements[mobile_index].path_index;
            let path = assets
                .navigation
                .hiking_paths
                .get(usize::from(path_index))
                .unwrap_or_else(|| panic!("mobile {mobile_index} lost hiking path {path_index}"));
            if let Some(motion) = self.world.mobile_elements[mobile_index].begin_hourglass_motion()
            {
                let movement_animation_speed =
                    self.world.mobile_elements[mobile_index].animation_speed();
                // The original game translates every masked child before
                // line-crossing checks and before the goal/waypoint arm. Its
                // adaptive-speed branch also fixes this frame's child
                // modulation now; a reached waypoint speed macro applies to
                // the master immediately but not to child animation until the
                // next Update.
                for &sprite_id in &sprite_ids {
                    let fx = self
                        .world
                        .entities
                        .get_mut(sprite_id)
                        .and_then(crate::element::Entity::as_fx_mut)
                        .unwrap_or_else(|| {
                            panic!("mobile {mobile_index} child {sprite_id} became stale during master motion")
                        });
                    if motion.movement != crate::coordinates::MapVec::ZERO {
                        fx.element
                            .set_position_map(fx.element.position_map() + motion.movement);
                    }
                    fx.fx.animation_speed = movement_animation_speed;
                }

                // This deliberately precedes waypoint execution. Projection
                // fallback probes with the increment that produced this move,
                // not a direction selected by the newly reached waypoint.
                self.check_mobile_line_crossing(assets, mobile_index);
                self.world.mobile_elements[mobile_index]
                    .finish_hourglass_waypoint(sim, path, motion.reached_goal)
                    .unwrap_or_else(|error| {
                        panic!(
                            "mobile {mobile_index} waypoint update at child {child_id} failed: {error}"
                        )
                    });

                let mobile = &self.world.mobile_elements[mobile_index];
                let active = mobile.active;
                let layer = mobile.layer;
                let sector = mobile.sector;
                for sprite_id in sprite_ids {
                    let fx = self
                        .world
                        .entities
                        .get_mut(sprite_id)
                        .and_then(crate::element::Entity::as_fx_mut)
                        .unwrap_or_else(|| {
                            panic!("mobile {mobile_index} child {sprite_id} became stale during waypoint completion")
                    });
                    fx.element.active = active;
                    fx.element.set_layer(layer);
                    fx.element
                        .set_sector(crate::position_interface::SectorHandle::new(sector));
                }
            }
        } else {
            assert!(
                child_offset > 0,
                "mobile {mobile_index} first-child boundary bookkeeping failed for {child_id}"
            );
        }

        let stopped = self.world.mobile_elements[mobile_index].stopped;
        let frozen = self.actors_frozen();
        let fx = self
            .world
            .entities
            .get_mut(child_id)
            .and_then(crate::element::Entity::as_fx_mut)
            .unwrap_or_else(|| {
                panic!("mobile {mobile_index} child {child_id} vanished before masked FX update")
            });
        if fx.element.active && !stopped && !frozen {
            fx.element
                .sprite
                .increment_frame_modulated(fx.fx.animation_speed);
        }
        true
    }

    pub(super) fn tick_static_entity_hourglass_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        use crate::element::OriginalBonusConcreteClass;
        use crate::sprite::{FrameProgression, MotionState};

        let frozen = self.actors_frozen();
        let entity = self.world.entities.get(owner).unwrap_or_else(|| {
            panic!(
                "static update owner {owner:?} disappeared immediately after live legacy-slot resolution"
            )
        });
        match entity {
            Entity::Fx(fx) if fx.fx.mobile_index.is_some() => (),
            Entity::Fx(_) => {
                if !entity.is_active() || frozen {
                    return;
                }
                let patch_idx = entity.as_fx().and_then(|fx| fx.fx.patch_index);
                let (progression, in_transition) = if let Some(patch_idx) = patch_idx {
                    if self.scripts.mission.is_none() {
                        (FrameProgression::Default, false)
                    } else {
                        let patch = self
                            .script_domains
                            .interactables
                            .patches
                            .get(usize::from(patch_idx))
                            .unwrap_or_else(|| panic!("FX {owner:?} references missing patch {patch_idx} at its live update slot"));
                        (
                            if patch.applied && patch.in_transition {
                                FrameProgression::Reversed
                            } else {
                                FrameProgression::Default
                            },
                            patch.in_transition,
                        )
                    }
                } else {
                    (FrameProgression::Default, false)
                };
                let motion = self
                    .world
                    .entities
                    .get_mut(owner)
                    .unwrap_or_else(|| panic!("FX {owner:?} vanished before sprite update"))
                    .element_data_mut()
                    .sprite
                    .perform_virgin_increment(sim, progression);
                if matches!(motion, MotionState::Terminated) && in_transition {
                    self.finish_patch_transition_for(
                        sim,
                        assets,
                        patch_idx.expect("transitioning FX must retain its patch"),
                    );
                }
            }
            Entity::Target(target) => {
                let active = target.element.active;
                let progression = FrameProgression::from_ordinal(target.target.progression);
                if active && !frozen {
                    self.world
                        .entities
                        .get_mut(owner)
                        .unwrap()
                        .element_data_mut()
                        .sprite
                        .perform_virgin_increment(sim, progression);
                }
            }
            Entity::Scroll(scroll) => {
                if !scroll.element.active {
                    return;
                }
                self.dispatch_scroll_hourglass_for(sim, assets, owner);
                // Sprite handling samples the engine FreezeAll state after the
                // synchronous Scroll VM returns, not at update entry.
                if !self.actors_frozen()
                    && let Some(entity) = self.world.entities.get_mut(owner)
                {
                    let Entity::Scroll(scroll) = entity else {
                        panic!(
                            "scroll {owner:?} changed concrete type before entry-active sprite update"
                        )
                    };
                    // The original game tests activity only once on entry. A due VM
                    // callback may deactivate this surviving scroll, but its
                    // sprite still advances before this update returns.
                    scroll
                        .element
                        .sprite
                        .perform_virgin_increment(sim, FrameProgression::Default);
                }
            }
            Entity::Bonus(bonus) => match bonus.original_concrete_class() {
                OriginalBonusConcreteClass::Bonus => {
                    if !frozen {
                        self.world
                            .entities
                            .get_mut(owner)
                            .unwrap()
                            .element_data_mut()
                            .sprite
                            .perform_virgin_increment(sim, FrameProgression::Default);
                    }
                    self.refresh_bonus_discovered_for(assets, owner);
                }
                // The ale update returns false once inactive, but
                // The engine removes the element with its default
                // deactivation-only mode. The pointer stays in the element collection
                // because other elements may still reference it.
                OriginalBonusConcreteClass::Ale => {}
                OriginalBonusConcreteClass::Cape => {
                    if !frozen {
                        self.world
                            .entities
                            .get_mut(owner)
                            .unwrap()
                            .element_data_mut()
                            .sprite
                            .perform_virgin_increment(sim, FrameProgression::Default);
                    }
                }
                OriginalBonusConcreteClass::Unsupported => panic!(
                    "Entity::Bonus {owner:?} has unsupported original-game concrete-kind mapping for {:?}",
                    bonus.object.object_type
                ),
            },
            _ => {}
        }
    }

    /// Run the bounded base-actor update in live original-game element
    /// order: generic animation/Execute, synchronous combat-injury Think,
    /// completion/priority effects, then `ActionChange`.
    ///
    /// Element serialization sorts elements by creation order before writing a
    /// save, and the loaded compact array
    /// retains that order. Rust entity IDs keep the initialized mission's
    /// stable sparse slots, so their numeric order is not the loaded
    /// element order. Walk the authoritative original-game creation
    /// identities instead. The local vector is compacted after every callback
    /// and newly constructed elements are appended, preserving the Original
    /// loop's observable mutation behavior.
    ///
    /// Generic animation eligibility does not gate `ActionChange`; inactive,
    /// frozen, moving, active-shot, and active-melee actors still reach the
    /// callback boundary.
    #[cfg(test)]
    pub(super) fn tick_actor_animation_action_change_slots(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        self.tick_actor_animation_action_change_slots_with_hooks(
            sim,
            assets,
            |_, _| {},
            |_, _| {},
            |_, _, _, _, _, _, _| {},
            |_, _, _| {},
        );
    }

    #[cfg(test)]
    pub(super) fn tick_actor_animation_action_change_slots_with_after_slot(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        mut after_slot: impl FnMut(&mut Self, EntityId),
    ) {
        self.tick_actor_animation_action_change_slots_with_hooks(
            sim,
            assets,
            |_, _| {},
            |_, _| {},
            |_, _, _, _, _, _, _| {},
            |engine, owner, _| after_slot(engine, owner),
        );
    }

    pub(super) fn tick_actor_animation_action_change_slots_with_hooks<ExecuteMotion>(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        mut non_actor_slot: impl FnMut(&mut Self, EntityId),
        mut before_actor: impl FnMut(&mut Self, EntityId),
        mut execute_owner_arm: impl FnMut(
            &mut Self,
            EntityId,
            Option<super::movement::MovementOwnerSelection>,
            Option<MeleeOwnerSelection>,
            Option<(crate::sequence::SequenceId, usize, std::num::NonZeroU32)>,
            Option<(crate::sequence::SequenceId, usize, std::num::NonZeroU32)>,
            Option<std::num::NonZeroU32>,
        ) -> ExecuteMotion,
        mut after_slot: impl FnMut(&mut Self, EntityId, crate::order::OrderType),
    ) where
        ExecuteMotion: IntoExplicitExecuteMotion,
    {
        let mut original_slots = self
            .world
            .entities
            .occupied()
            .map(|(entity_id, _)| entity_id)
            .collect::<Vec<_>>();
        original_slots.sort_by_key(|&entity_id| self.world.original_creation_order(entity_id));
        let mut observed_creation_counter = self.world.next_original_creation_order;
        let mut slot = 0;
        while slot < original_slots.len() {
            let entity_id = original_slots[slot];
            if self.world.entities.get(entity_id).is_some() {
                observe_ordered_gameplay_entity(entity_id);
                let entity = self
                    .world
                    .entities
                    .get(entity_id)
                    .unwrap_or_else(|| {
                        panic!(
                            "actor animation coordinator lost entity {entity_id:?} resolved from Original element slot {slot}"
                        )
                    });
                let actor_enters_hourglass = entity.actor_data().is_some()
                    && !matches!(entity, Entity::Pc(pc) if pc.pc.fried_psykokwack);
                if actor_enters_hourglass {
                    self.tick_one_actor_animation_action_change_slot(
                        sim,
                        assets,
                        entity_id,
                        &mut before_actor,
                        &mut execute_owner_arm,
                        &mut after_slot,
                    );
                } else {
                    non_actor_slot(self, entity_id);
                }
            }

            // Original-game element removal compacts the element collection immediately, so
            // incrementing the loop index skips the element shifted into the
            // removed position. Retaining before incrementing reproduces that
            // behavior. Registration appends newly created elements; their
            // monotonically increasing creation identities let us discover
            // only the new tail without confusing stable Rust slots for
            // Original array positions.
            original_slots.retain(|&id| self.world.entities.get(id).is_some());
            if self.world.next_original_creation_order != observed_creation_counter {
                assert!(
                    self.world.next_original_creation_order > observed_creation_counter,
                    "original-game creation counter moved backwards during update"
                );
                original_slots.extend(
                    self.world
                        .original_creation_order_by_entity
                        .iter()
                        .filter_map(|(&id, &creation_order)| {
                            (creation_order >= observed_creation_counter
                                && self.world.entities.get(id).is_some())
                            .then_some(id)
                        }),
                );
                original_slots.sort_by_key(|&id| self.world.original_creation_order(id));
                observed_creation_counter = self.world.next_original_creation_order;
            }
            slot += 1;
        }

        // The original game's execution dispatch chain is closed here: generic sprite
        // arms use tick_actor_animation_for; selected movement, melee, bow,
        // ability, beggar, and WaitingSword work use their live owner arms;
        // the human/PC/NPC derived tail hook runs before the slot advances.
    }

    fn tick_one_actor_animation_action_change_slot<ExecuteMotion>(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        before_actor: &mut impl FnMut(&mut Self, EntityId),
        execute_owner_arm: &mut impl FnMut(
            &mut Self,
            EntityId,
            Option<super::movement::MovementOwnerSelection>,
            Option<MeleeOwnerSelection>,
            Option<(crate::sequence::SequenceId, usize, std::num::NonZeroU32)>,
            Option<(crate::sequence::SequenceId, usize, std::num::NonZeroU32)>,
            Option<std::num::NonZeroU32>,
        ) -> ExecuteMotion,
        after_slot: &mut impl FnMut(&mut Self, EntityId, crate::order::OrderType),
    ) where
        ExecuteMotion: IntoExplicitExecuteMotion,
    {
        let ctx = ActionChangeSlotCtx {
            sim,
            assets,
            entity_id,
        };

        // The actor update consumes one queued base
        // position update before it inspects the current
        // sequence/order.
        self.apply_delayed_actor_position(sim, assets, entity_id);
        self.debug_patrol_turn_lifecycle("actor_slot_before_prelude", entity_id);
        before_actor(self, entity_id);
        self.debug_patrol_turn_lifecycle("actor_slot_after_prelude", entity_id);
        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::BaseActor(entity_id));

        if self.action_change_frozen_without_order(ctx) {
            after_slot(self, entity_id, crate::order::OrderType::NonanimationEnd);
            return;
        }

        let entry = self.action_change_install_entry_order(ctx);
        let selections = self.action_change_owner_selections(ctx, entry);
        let explicit_execute = execute_owner_arm(
            self,
            entity_id,
            selections.movement_selection,
            selections.melee_selection,
            selections.bow_selection,
            selections.ability_selection,
            selections.beggar_selection,
        )
        .into_explicit_execute_motion();
        let motion =
            self.action_change_specialized_motion(ctx, entry, selections, explicit_execute);

        self.action_change_generic_execute(ctx, entry, selections, motion);
        self.action_change_latch_completion_motion(ctx, entry, motion);
        let installed_tail_order_type = self.action_change_dispatch(ctx);
        after_slot(self, entity_id, installed_tail_order_type);
        self.action_change_slot_tail(ctx, entry);
    }

    /// Fuse the supported Actor → Human → PC/NPC update phases into one
    /// live Original-element walk. The underlying actor coordinator owns the
    /// compact creation-ordered loop, including removals and callback-spawned
    /// tail elements; this hook closes the derived tail before it increments
    /// the slot.
    pub(crate) fn tick_actor_owner_envelopes(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        self.tick_actor_owner_envelopes_with_owner_hook(sim, assets, |_, _| {})
    }

    #[cfg(test)]
    pub(super) fn tick_actor_owner_envelopes_with_test_owner_hook(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner_hook: impl FnMut(&mut Self, EntityId),
    ) {
        self.tick_actor_owner_envelopes_with_owner_hook(sim, assets, owner_hook);
    }

    fn tick_actor_owner_envelopes_with_owner_hook(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        mut owner_hook: impl FnMut(&mut Self, EntityId),
    ) {
        {
            let _detail = entity_system_detail_guard(EntitySystemDetail::PrepareNpc);
            self.prepare_npc_owner_pass();
        }
        self.tick_actor_animation_action_change_slots_with_hooks(
            sim,
            assets,
            |engine, owner| {
                let _detail = entity_system_detail_guard(EntitySystemDetail::StaticOwners);
                use crate::element::OriginalHourglassClass as Class;

                // Original-derived nonactor nesting: the mobile master/child
                // boundary runs before the independent static owner, followed
                // by projectile/net dispatch.
                let class = engine
                    .get_entity(owner)
                    .unwrap_or_else(|| {
                        panic!(
                            "update owner {owner:?} disappeared immediately after live legacy-slot resolution"
                        )
                    })
                    .original_hourglass_class();
                match class {
                    Class::FxMasked => assert!(
                        engine.tick_mobile_child_owner_boundary(sim, assets, owner),
                        "mapped FXMasked owner {owner:?} lost its mobile boundary"
                    ),
                    Class::Fx
                    | Class::Target
                    | Class::Bonus
                    | Class::Ale
                    | Class::Cape
                    | Class::Scroll => {
                        engine.tick_static_entity_hourglass_for(sim, assets, owner)
                    }
                    Class::Arrow
                    | Class::Apple
                    | Class::Stone
                    | Class::Purse
                    | Class::Coin
                    | Class::Net
                    | Class::WaspNest
                    | Class::Wasp => {
                        engine.tick_projectile_or_net_hourglass(sim, assets, owner)
                    }
                    Class::ActorPc | Class::ActorSoldier | Class::ActorCivilian => {}
                }
            },
            |engine, owner| {
                let _detail = entity_system_detail_guard(EntitySystemDetail::OwnerPrelude);
                // The jump step lifecycle is the jump order's own work: the
                // step that starts here is the order this actor executes a few
                // lines later, and the landing posture it publishes is visible
                // to every later creation slot and to none of the earlier ones.
                engine.tick_active_jump_for(sim, assets, owner);
                if matches!(owner, EntityId::Soldier(_)) {
                    observe_actor_owner_envelope(ActorOwnerEnvelopePhase::SoldierPrelude(owner));
                    engine.tick_apple_smell_for(owner);
                    engine.tick_soldier_track_primary_target_for(owner);
                    engine.tick_attacking_reactiontime_enemy_near_for(sim, assets, owner);
                }
                if matches!(owner, EntityId::Soldier(_) | EntityId::Civilian(_))
                    && !engine.actors_frozen()
                {
                    observe_actor_owner_envelope(ActorOwnerEnvelopePhase::Patrol(owner));
                    engine.tick_patrol_coordination_for_npc(sim, assets, owner);
                }
                if engine
                    .world
                    .entities
                    .get(owner)
                    .is_some_and(|entity| entity.human_data().is_some())
                {
                    observe_actor_owner_envelope(ActorOwnerEnvelopePhase::HumanPrelude(owner));
                    engine.tick_concussion_healing_for(sim, owner, assets);
                    engine.process_shoot_list_for(sim, assets, owner);
                }
            },
            |engine, owner, movement, melee, bow, ability, selected_beggar| {
                let _detail = entity_system_detail_guard(EntitySystemDetail::OwnerExecute);
                let execution_frozen = engine
                    .get_entity(owner)
                    .and_then(Entity::actor_data)
                    .is_some_and(|actor| actor.execution_frozen);
                if execution_frozen {
                    return ExplicitExecuteMotion::default();
                }
                // Human's literal sword-movement arm rejects an unforced
                // movement with no opponents before opponent-facing or
                // seeking. In particular, a stale moved-target seek must
                // launch QuitSwordfight instead of refreshing itself first.
                if let Some(selection) = movement
                    && engine.abort_orphaned_sword_movement(sim, assets, owner, selection)
                {
                    return ExplicitExecuteMotion::default();
                }
                // Seeking's "wait for the post seek sequence to be
                // launched" arm runs ahead of every other seek step: Execute
                // returns TERMINATED before any motion, countdown ageing, or
                // seek refresh, and the actor update then advances the order.
                if let Some(selection) = movement
                    && super::refresh_seek::perform_seek_lost_actor_target(
                        engine, owner, selection,
                    )
                {
                    return ExplicitExecuteMotion {
                        initial: Some(crate::sprite::MotionState::Terminated),
                        post_completion_override: None,
                    };
                }
                // Seek refresh is part of this exact actor's seeking
                // Execute arm. Sampling here preserves creation-order
                // visibility of the moving target, and a replacement does
                // not itself execute until this owner returns next frame.
                if movement.is_some() {
                    if let Some(motion) =
                        engine.tick_refreshing_seek_for_owner(sim, assets, owner)
                    {
                        return ExplicitExecuteMotion {
                            initial: Some(motion),
                            post_completion_override: None,
                        };
                    }
                    // Opponent-facing / danger-facing run inside the execution
                    // arm *before* seeking, so their facing write and
                    // turning still happen on the frame seeking's
                    // moved-target seek-refresh branch preempts the motion.
                    if engine.selected_seek_refresh_decision(owner).is_some() {
                        engine.apply_pre_perform_seek_facing_prologue(owner);
                    }
                    if engine.tick_refresh_seek_for_owner(sim, assets, owner) {
                        return ExplicitExecuteMotion {
                            initial: Some(crate::sprite::MotionState::InProgress),
                            post_completion_override: None,
                        };
                    }
                }
                // Seeking's completion-time refresh branches return
                // in-progress motion explicitly,
                // so the actor update runs none of its DONE / TERMINATED /
                // ABORTED tail for that slot.
                let movement_motion =
                    engine.tick_entity_movement_owner(sim, assets, owner, movement);
                if movement_motion.initial.is_some()
                    || movement_motion.post_completion_override.is_some()
                {
                    return ExplicitExecuteMotion {
                        initial: movement_motion.initial,
                        post_completion_override: movement_motion.post_completion_override,
                    };
                }
                if let Some(selection) = melee {
                    engine.tick_selected_melee_owner(sim, assets, owner, selection);
                    if engine
                        .world
                        .entities
                        .get(owner)
                        .is_some_and(Entity::is_pc)
                    {
                        // The player override wraps human action execution. Therefore its
                        // START-edge remark follows Human's strike warning,
                        // but still belongs to this actor's live slot.
                        engine.tick_pc_combat_anim_speech_for_owner(sim, assets, owner);
                    }
                }
                if let Some((_, _, order_id)) = bow {
                    engine.tick_bow_shot_for(sim, assets, owner, order_id);
                }
                if ability.is_some() {
                    let listen = crate::abilities::selected_ability(
                        &engine.world.entities,
                        &engine.orders.sequence_manager,
                        owner,
                    )
                    .filter(|ability| ability.kind == crate::movement::AbilityKind::Listen);
                    let listen_counting = listen.is_some_and(|ability| {
                        ability.order_type == crate::order::OrderType::Listening
                    });
                    let listen_advanced = listen.is_some()
                        && engine.tick_enemy_ai_blip_detection_for_owner(sim, assets, owner);
                    // The original game's listening-animation update ignores
                    // the sprite's DONE/TERMINATED states and remains in
                    // progress until the wait timer reaches zero. The detection
                    // owner arm above is the complete Execute implementation
                    // while CountingDown; running generic tick_ability as
                    // well would let the short looping sprite terminate the
                    // order and enter the exit transition early.
                    if !listen_counting && !listen_advanced {
                        engine.tick_selected_ability(sim, assets, owner, engine.actors_frozen());
                    }
                }
                if let Some(order_id) = selected_beggar {
                    engine.tick_beggar_bid_for(sim, assets, owner, order_id);
                }
                ExplicitExecuteMotion::default()
            },
            |engine, owner, derived_tail_order_type| {
                let _detail = entity_system_detail_guard(EntitySystemDetail::NpcTail);
                let is_human = engine
                    .world
                    .entities
                    .get(owner)
                    .unwrap_or_else(|| {
                        panic!(
                            "actor owner {} disappeared before its specialized update tail",
                            owner.index()
                        )
                    })
                    .human_data()
                    .is_some();
                if !is_human {
                    return;
                }
                match owner {
                    EntityId::Pc(_) => {
                        engine.refresh_pc_produced_noise_for_with_order(
                            owner,
                            derived_tail_order_type,
                        );
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::HumanNoise(owner));
                        engine.tick_tiredness_for(owner, assets);
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::HumanTiredness(
                            owner,
                        ));
                        if engine
                            .world
                            .entities
                            .get(owner)
                            .is_some_and(|entity| entity.ai_controller().is_some())
                        {
                            engine.tick_npc_owner_pass(sim, assets, owner);
                        }
                        engine.tick_pc_auto_heal_for(sim, owner);
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::PcTail(owner));
                    }
                    EntityId::Soldier(_) | EntityId::Civilian(_) => {
                        engine.tick_tiredness_for(owner, assets);
                        // NPC humans have no produced-noise refresh, so
                        // their Human tail begins at tiredness.
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::HumanTiredness(
                            owner,
                        ));
                        engine.tick_npc_owner_pass(sim, assets, owner);
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::NpcTail(owner));
                    }
                    _ => panic!(
                        "human actor owner {} has unsupported entity kind",
                        owner.index()
                    ),
                }
                owner_hook(engine, owner);
            },
        );
    }

    /// Dispatch the exact original-game per-frame update chain for a live
    /// projectile/net creation slot.  Entity kind and `ObjectType` together
    /// are the Rust vtable: accepting any other pairing here would fabricate
    /// subtype behaviour that the loaded object never had.
    pub(super) fn tick_projectile_or_net_hourglass(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        id: EntityId,
    ) {
        let Some(entity) = self.get_entity(id) else {
            return;
        };
        // Validate the Rust kind/ObjectType vtable pairing before the base
        // inactive-removal rule. Otherwise an impossible inactive object
        // silently disappears while the same active object panics.
        match entity {
            Entity::Projectile(projectile)
                if !matches!(
                    projectile.object.object_type,
                    crate::element::ObjectType::Arrow
                        | crate::element::ObjectType::Apple
                        | crate::element::ObjectType::Stone
                        | crate::element::ObjectType::Purse
                        | crate::element::ObjectType::Coin
                        | crate::element::ObjectType::WaspNest
                        | crate::element::ObjectType::BonusWaspNest
                        | crate::element::ObjectType::Wasp
                ) =>
            {
                panic!(
                    "projectile entity {id:?} has unsupported ObjectType::{:?}; TODO(PA-013): map its Original concrete class",
                    projectile.object.object_type
                )
            }
            Entity::Net(net)
                if !matches!(
                    net.object.object_type,
                    crate::element::ObjectType::Net | crate::element::ObjectType::BonusNet
                ) =>
            {
                panic!(
                    "net entity {id:?} has unsupported ObjectType::{:?}; expected Net or BonusNet",
                    net.object.object_type
                )
            }
            _ => {}
        }
        let dispatch = match entity {
            Entity::Projectile(projectile) => Some((
                true,
                projectile.object.object_type,
                projectile.element.active,
            )),
            Entity::Net(net) => Some((false, net.object.object_type, net.element.active)),
            _ => None,
        };
        let Some((is_projectile, object_type, base_active)) = dispatch else {
            return;
        };
        let retain = if is_projectile {
            match object_type {
                crate::element::ObjectType::Arrow => {
                    if base_active {
                        let flying = self
                            .get_entity(id)
                            .and_then(|entity| match entity {
                                Entity::Projectile(projectile) => {
                                    Some(projectile.projectile.flying)
                                }
                                _ => None,
                            })
                            .expect("arrow owner changed concrete entity kind");
                        if flying {
                            self.tick_existing_projectile(sim, assets, id);
                        } else if let Some(Entity::Projectile(projectile)) =
                            self.world.entities.get_mut(id)
                        {
                            // The projectile update starts a move
                            // before testing the flying flag. Active stopped arrows
                            // therefore settle old=current on every owner tick
                            // until the later Refresh retires them.
                            projectile.element.sprite.position_iface.new_move();
                        }
                    }
                    base_active
                }
                crate::element::ObjectType::Apple | crate::element::ObjectType::Stone => {
                    if base_active {
                        self.tick_existing_projectile(sim, assets, id);
                    }
                    let frozen = self.actors_frozen();
                    if let Some(Entity::Projectile(projectile)) = self.get_entity_mut(id)
                        && !projectile.projectile.flying
                        && !frozen
                    {
                        observe_projectile_derived_tail(id, object_type);
                        let motion = projectile.element.sprite.perform_virgin_increment(
                            sim,
                            crate::sprite::FrameProgression::Default,
                        );
                        projectile.element.active =
                            motion != crate::sprite::MotionState::Terminated;
                    }
                    // Apple/Stone return the Projectile base result even
                    // though their grounded sprite tail may have changed
                    // active state afterward.
                    base_active
                }
                crate::element::ObjectType::Purse | crate::element::ObjectType::Coin => {
                    self.tick_purse_or_coin(sim, assets, id)
                }
                crate::element::ObjectType::WaspNest
                | crate::element::ObjectType::BonusWaspNest
                | crate::element::ObjectType::Wasp => {
                    self.tick_wasp_nest_or_wasp(sim, assets, id);
                    base_active
                }
                unsupported => panic!(
                    "projectile entity {id:?} has unsupported ObjectType::{unsupported:?}; TODO(PA-013): map its Original concrete class"
                ),
            }
        } else {
            match object_type {
                crate::element::ObjectType::Net | crate::element::ObjectType::BonusNet => {
                    self.tick_net(sim, assets, id);
                    true
                }
                unsupported => panic!(
                    "net entity {id:?} has unsupported ObjectType::{unsupported:?}; expected Net or BonusNet"
                ),
            }
        };
        if !retain && let Some(entity) = self.get_entity_mut(id) {
            // Element removal is called with its default
            // deactivation-only mode from the element's hourglass loop. The
            // projectile remains in the element array as an inactive
            // tombstone so outstanding references and creation order stay
            // valid; physical removal is reserved for teardown/load paths.
            entity.element_data_mut().active = false;
        }
    }

    /// Apply the two sequence-command motion modifiers owned by
    /// the actor update after one execution call.
    fn apply_actor_post_execute_wait_modifier(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        execute_result: &mut super::animation::ActorExecuteResult,
    ) {
        self.apply_actor_post_execute_wait_modifier_to_motion(
            sim,
            assets,
            owner,
            execute_result.entry_seq_id,
            execute_result.entry_elem_idx,
            &mut execute_result.motion,
        );
    }

    fn apply_actor_post_execute_wait_modifier_to_motion(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        entry_seq_id: crate::sequence::SequenceId,
        entry_elem_idx: usize,
        motion: &mut crate::sprite::MotionState,
    ) {
        let entry_command = self
            .orders
            .sequence_manager
            .get_element(entry_seq_id, entry_elem_idx)
            .map(|element| element.command);
        let live_element = self.world.entities.current_element_for_actor(owner);
        let live_command = live_element.and_then(|(seq_id, elem_idx)| {
            self.orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .map(|element| element.command)
        });

        // Execution is selected from the current sequence element before entering the
        // actor's specialized response. A WaitingSword callback may stop
        // that element before control returns to the actor update, but the
        // original game retains its reference while this update stack unwinds.
        // Rust's live-element scan then returns None, so fall back to the
        // Execute-entry identity. A genuinely instructed synchronous
        // replacement remains live and takes precedence. Completion itself
        // is still resolved against the then-live element by
        // finish_actor_execute_completion.
        let effective_command = live_command.or(entry_command);
        if effective_command == Some(crate::element::Command::WaitTimer) {
            let actor = self
                .world
                .entities
                .get_mut(owner)
                .unwrap_or_else(|| panic!("WAIT_TIMER post-Execute owner {owner:?} is missing"))
                .actor_data_mut()
                .unwrap_or_else(|| {
                    panic!("WAIT_TIMER post-Execute owner {owner:?} is not an actor")
                });
            if actor.wait_time == 0 {
                actor.seek_refresh_wait = 0;
                *motion = crate::sprite::MotionState::Terminated;
            } else {
                actor.wait_time -= 1;
                actor.seek_refresh_wait = actor.wait_time;
            }
            return;
        }

        if live_command == Some(crate::element::Command::WaitFreeLift)
            && let Some((seq_id, elem_idx)) = live_element
        {
            let authorized = self.authorize_and_reserve_lift_wait(owner, seq_id, elem_idx);
            if authorized {
                *motion = crate::sprite::MotionState::Terminated;
            }
        }
    }

    /// Resolve the retained base-Actor motion after derived Execute callbacks
    /// and wait modifiers. Original-game termination advances to the next order through the
    /// owner's live selected sequence element; ABORTED alone uses the sequence
    /// element snapshot captured before Execute.
    fn finish_actor_execute_completion(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        entry_order_id: Option<std::num::NonZeroU32>,
        execute_result: super::animation::ActorExecuteResult,
    ) {
        match execute_result.motion {
            crate::sprite::MotionState::Aborted => self.execute_seq_impossible(
                sim,
                assets,
                (execute_result.entry_seq_id, execute_result.entry_elem_idx),
            ),
            crate::sprite::MotionState::Terminated => {
                let Some((seq_id, elem_idx, order)) = self
                    .orders
                    .sequence_manager
                    .current_order_for_actor(&self.world.entities, owner)
                else {
                    return;
                };
                match order.completion.clone() {
                    crate::order::OrderCompletion::AdvanceElement => {
                        self.execute_seq_advance(sim, assets, (seq_id, elem_idx));
                    }
                    crate::order::OrderCompletion::UnlockDoor { door_id } => {
                        let _ = door_id;
                        self.execute_seq_advance(sim, assets, (seq_id, elem_idx));
                    }
                    crate::order::OrderCompletion::ResumeDoorPass => {
                        self.execute_resume_door_pass(sim, assets, owner);
                    }
                    crate::order::OrderCompletion::NextJumpStep => {
                        self.execute_next_jump_step(sim, assets, owner);
                    }
                    crate::order::OrderCompletion::WaspStruggleCycle { cycles_remaining } => {
                        if cycles_remaining <= 1 {
                            self.execute_seq_terminate(sim, assets, (seq_id, elem_idx));
                        } else {
                            self.execute_wasp_next_cycle(
                                sim,
                                assets,
                                (seq_id, elem_idx, cycles_remaining - 1),
                            );
                        }
                    }
                }
            }
            crate::sprite::MotionState::Done => {
                let order_id = entry_order_id.unwrap_or_else(|| {
                    panic!(
                        "actor {owner:?} returned Done without an entry-latched order for {:?}/{}",
                        execute_result.entry_seq_id, execute_result.entry_elem_idx
                    )
                });
                self.mark_entry_order_done(
                    owner,
                    execute_result.entry_seq_id,
                    execute_result.entry_elem_idx,
                    order_id,
                );
            }
            crate::sprite::MotionState::Start | crate::sprite::MotionState::InProgress => {}
            crate::sprite::MotionState::Error => panic!(
                "actor {owner:?} Execute returned MotionState::Error from entry {:?}/{}",
                execute_result.entry_seq_id, execute_result.entry_elem_idx
            ),
        }
    }

    fn mark_entry_order_done(
        &mut self,
        owner: EntityId,
        entry_seq_id: crate::sequence::SequenceId,
        entry_elem_idx: usize,
        order_id: std::num::NonZeroU32,
    ) {
        let Some(element) = self
            .orders
            .sequence_manager
            .get_element_mut(entry_seq_id, entry_elem_idx)
        else {
            // Execute may synchronously terminate and collect its own entry
            // element before returning. Original still writes through the
            // retained actor-order allocation, but no later priority decision can
            // observe that detached order.
            tracing::trace!(
                ?owner,
                ?entry_seq_id,
                entry_elem_idx,
                %order_id,
                "Done entry element was synchronously collected before actor-update write-back"
            );
            return;
        };
        let Some(order) = element
            .orders
            .iter_mut()
            .find(|order| order.order_id == order_id)
        else {
            // The same re-entrant teardown can retain the terminal element
            // shell while deleting its order list.
            tracing::trace!(
                ?owner,
                ?entry_seq_id,
                entry_elem_idx,
                %order_id,
                "Done entry order was synchronously removed before actor-update write-back"
            );
            return;
        };
        // The original actor update marks the order done immediately after
        // Execute returns. Later callbacks in this same owner slot and
        // The sequence-manager tick can therefore terminate a blocker
        // instead of postponing behind an animation which already reached its
        // action point.
        order.done = true;
    }

    /// Whether `owner` is a beggar civilian that refuses this command.
    ///
    /// Beggars accept only `RECEIVE_PURSE`, `BEGGAR_SHOW_FACE`, and
    /// `WAIT`.  Every other sequence command on a beggar is
    /// rejected — `sequence_manager.element_impossible` fires.
    pub(super) fn beggar_rejects_command(&self, owner: EntityId, cmd: Command) -> bool {
        let is_beggar = self.get_entity(owner).is_some_and(|e| {
            matches!(e, crate::element::Entity::Civilian(c)
                if c.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar)
        });
        is_beggar
            && !matches!(
                cmd,
                Command::ReceivePurse | Command::BeggarShowFace | Command::Wait
            )
    }

    pub(super) fn apply_helper_driven_shoulder_dismount(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        dismount: super::animation::ShoulderHelperDismount,
    ) {
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType;
        use crate::sprite::MotionState;

        let Some(carried) = self.get_entity_mut(dismount.carried_id) else {
            // The original game permits the carried-actor reference to become empty while the transition
            // runs and simply finishes the helper animation in that case.
            return;
        };
        let carried_sprite_direction = u16::try_from(carried.element_data().direction())
            .expect("PC shoulder rider has a negative direction");
        let sprite = &mut carried.element_data_mut().sprite;
        sprite.force_sprite_row(
            OrderType::ClimbingDownFromShoulders,
            carried_sprite_direction,
        );
        sprite.synchronize_anim(dismount.helper_frame, dismount.helper_frame_count);
        sprite.display_order_ref = Some(dismount.helper_id);
        sprite.behind_display_order_ref = false;

        if dismount.motion == MotionState::Done {
            carried.set_posture(Posture::Upright);
            carried
                .actor_data_mut()
                .expect("PC has actor data")
                .action_state = ActionState::Waiting;
        }
        if dismount.motion != MotionState::Terminated {
            return;
        }

        let helper_position = self
            .get_entity(dismount.helper_id)
            .expect("shoulder-dismount helper vanished before termination")
            .element_data()
            .position_map();
        let helper_current_point = self
            .get_entity(dismount.helper_id)
            .expect("shoulder-dismount helper vanished before landing search")
            .current_gameplay_point_map()
            .unwrap_or_else(|| {
                panic!(
                    "shoulder-dismount helper {:?} has no current action point",
                    dismount.helper_id
                )
            });
        let helper_layer = self
            .get_entity(dismount.helper_id)
            .expect("shoulder-dismount helper vanished before termination")
            .element_data()
            .layer();
        let landing_position = {
            let carried_box = self
                .get_entity(dismount.carried_id)
                .expect("shoulder rider vanished before landing search")
                .position_iface()
                .get_move_box()
                .to_owned();
            if carried_box.is_somewhere() {
                // Original translates the upright rider box from the
                // helper's live map-space animation hotspot,
                // while using the helper's map origin as the directional
                // reference for the three-argument authorization search.
                let mut box_at_helper = carried_box.translated(helper_current_point);
                if self.world.fast_grid.find_authorized_position_toward(
                    &mut box_at_helper,
                    helper_position,
                    helper_layer,
                ) {
                    box_at_helper.center()
                } else {
                    helper_position
                }
            } else {
                helper_position
            }
        };

        {
            let carried = self
                .get_entity_mut(dismount.carried_id)
                .expect("shoulder rider disappeared before landing");
            carried
                .element_data_mut()
                .set_position_map_delayed(landing_position);
            carried.set_posture(Posture::Upright);
            carried
                .actor_data_mut()
                .expect("shoulder rider must be actor")
                .action_state = ActionState::Waiting;
        }
        // Waiting can synchronously change the relationship. Release the
        // helper's then-current rider and use that rider's live carrier heading.
        self.actor_wait(sim, assets, dismount.carried_id);
        let carried_id = self
            .expect_entity(dismount.helper_id, "shoulder helper after rider wait")
            .pc_data()
            .and_then(|pc| pc.carried)
            .expect("shoulder helper lost its rider during wait");
        let carrier = self
            .expect_entity(carried_id, "shoulder rider after wait")
            .human_data()
            .expect("shoulder rider must be human")
            .carrier;
        if let Some(carrier) = carrier {
            let direction = self
                .expect_entity(carrier, "rider carrier before release")
                .element_data()
                .direction();
            let carried = self
                .get_entity_mut(carried_id)
                .expect("shoulder rider disappeared before release");
            carried.element_data_mut().set_direction_goal(direction);
            carried
                .human_data_mut()
                .expect("shoulder rider must be human")
                .carrier = None;
        }
        self.get_entity_mut(dismount.helper_id)
            .expect("shoulder helper disappeared before release")
            .pc_data_mut()
            .expect("shoulder helper must be PC")
            .carried = None;
    }
}

/// Insert randomised midpoint detours into a pathfinder-returned
/// waypoint list (drunken soldier post-process path).
///
/// Walks the waypoint list in passes (one pass per
/// `blood_alcohol / increment` increments) and for every segment
/// tries up to 3 random deviation vectors; the first reachable one
/// gets inserted as a new intermediate waypoint.  Running soldiers
/// use a lower increment + factor (they don't wobble as much per
/// step) than walking soldiers.
///
/// The RNG is drained deterministically from the explicit caller context, so
/// replays reproduce the same deviation sequence. Required behavior:
/// The original game's soldier path post-processing uses two draws for
/// each of up to three candidate deviations per segment.
#[inline]
fn drunken_deviation_direction(direction: i16) -> [f32; 2] {
    // Converting a sector direction with the aspect ratio compresses the
    // table direction's Y component back into isometric map space.
    crate::position_interface::sector_to_vector_iso(direction)
}

pub(super) fn apply_drunken_path_deviation(
    sim: &crate::sim_rng::SimulationContext,

    mut waypoints: Vec<crate::coordinates::MapPoint>,
    origin: crate::coordinates::MapPoint,
    blood_alcohol: u8,
    is_running: bool,
    layer: u16,
    move_box: &crate::coordinates::MoveBox,
    half_diagonal: crate::coordinates::MoveBoxHalfDiagonal,
    grid: &crate::fast_find_grid::FastFindGrid,
) -> Vec<crate::coordinates::MapPoint> {
    const DRUNKEN_DEVIATION_FACTOR: f32 = 0.03;

    // Max of (30, blood_alcohol) — the minimum ensures even mildly
    // tipsy soldiers still show some wobble.
    let clamped_ba = blood_alcohol.max(30) as f32;
    let (factor, increment) = if is_running {
        (0.003 * clamped_ba, 60u8)
    } else {
        (0.01 * clamped_ba, 30u8)
    };

    let mut iterator = 0u8;
    while iterator < blood_alcohol {
        let mut new_path: Vec<crate::coordinates::MapPoint> =
            Vec::with_capacity(waypoints.len() * 2);
        let mut prev = origin;
        for next in &waypoints {
            let straight = crate::coordinates::MapVec::new(next.x - prev.x, next.y - prev.y);
            let max_norm = straight.x.abs().max(straight.y.abs());
            // Midpoint of the current segment.
            let midpoint = crate::coordinates::MapPoint::new(
                prev.x + 0.5 * straight.x,
                prev.y + 0.5 * straight.y,
            );
            let mut inserted: Option<crate::coordinates::MapPoint> = None;
            for _try in 0..3 {
                // `rand() & 15` — pick a random 16-sector direction
                // and scale by another 0..15 random magnitude.
                let dir_sector =
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::DrunkenPathDeviation, 0..16)
                        as i16;
                let magnitude =
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::DrunkenPathDeviation, 0..16)
                        as f32;
                let [dx, dy] = drunken_deviation_direction(dir_sector);
                let scale = magnitude * max_norm * DRUNKEN_DEVIATION_FACTOR * factor;
                let candidate = crate::coordinates::MapPoint::new(
                    midpoint.x + dx * scale,
                    midpoint.y + dy * scale,
                );
                if grid.is_straight_movement_authorized(prev, candidate, layer, move_box)
                    && grid.is_reachable_thick(candidate, *next, layer, half_diagonal)
                {
                    inserted = Some(candidate);
                    break;
                }
            }
            if let Some(ip) = inserted {
                new_path.push(ip);
            }
            new_path.push(*next);
            prev = *next;
        }
        waypoints = new_path;
        iterator = iterator.saturating_add(increment);
    }

    waypoints
}

/// Original soldier post-processing runs after actor path post-processing has
/// already inserted startup/end transitions. Walk only the remaining upright
/// movement orders and insert deviated copies immediately before them, leaving
/// transition geometry untouched.
pub(super) fn apply_drunken_order_deviation(
    sim: &crate::sim_rng::SimulationContext,
    element: &mut crate::sequence::SequenceElement,
    origin: crate::coordinates::MapPoint,
    blood_alcohol: u8,
    is_running: bool,
    layer: u16,
    move_box: &crate::coordinates::MoveBox,
    half_diagonal: crate::coordinates::MoveBoxHalfDiagonal,
    grid: &crate::fast_find_grid::FastFindGrid,
    next_order_id: &mut u32,
) {
    const DRUNKEN_DEVIATION_FACTOR: f32 = 0.03;

    let clamped_ba = blood_alcohol.max(30) as f32;
    let (factor, increment) = if is_running {
        (0.003 * clamped_ba, 60usize)
    } else {
        (0.01 * clamped_ba, 30usize)
    };
    let passes = usize::from(blood_alcohol).div_ceil(increment);

    insert_drunken_orders_with(element, origin, passes, next_order_id, |first, second| {
        let straight = crate::coordinates::MapVec::new(second.x - first.x, second.y - first.y);
        let max_norm = straight.x.abs().max(straight.y.abs());
        let midpoint = crate::coordinates::MapPoint::new(
            first.x + 0.5 * straight.x,
            first.y + 0.5 * straight.y,
        );
        for _try in 0..3 {
            let dir_sector =
                crate::sim_rng::u32(sim, crate::sim_rng::RngSite::DrunkenPathDeviation, 0..16)
                    as i16;
            let magnitude =
                crate::sim_rng::u32(sim, crate::sim_rng::RngSite::DrunkenPathDeviation, 0..16)
                    as f32;
            let [dx, dy] = drunken_deviation_direction(dir_sector);
            let scale = magnitude * max_norm * DRUNKEN_DEVIATION_FACTOR * factor;
            let candidate =
                crate::coordinates::MapPoint::new(midpoint.x + dx * scale, midpoint.y + dy * scale);
            if grid.is_straight_movement_authorized(first, candidate, layer, move_box)
                && grid.is_reachable_thick(candidate, second, layer, half_diagonal)
            {
                return Some(candidate);
            }
        }
        None
    });
}

fn insert_drunken_orders_with(
    element: &mut crate::sequence::SequenceElement,
    origin: crate::coordinates::MapPoint,
    passes: usize,
    next_order_id: &mut u32,
    mut candidate_for_segment: impl FnMut(
        crate::coordinates::MapPoint,
        crate::coordinates::MapPoint,
    ) -> Option<crate::coordinates::MapPoint>,
) {
    for _ in 0..passes {
        let mut first = origin;
        let mut order_index = 0usize;
        while order_index < element.orders.len() {
            let order = &element.orders[order_index];
            if !matches!(
                order.order_type,
                crate::order::OrderType::WalkingUpright | crate::order::OrderType::RunningUpright
            ) {
                order_index += 1;
                continue;
            }

            let second = crate::coordinates::MapPoint::new(order.target_x, order.target_y);
            if let Some(candidate) = candidate_for_segment(first, second) {
                // The original game copies the complete order: all movement
                // metadata is copied, while the inserted order receives a
                // fresh identity and its midpoint destination.
                let mut inserted = order.clone();
                inserted.reseed_id(crate::order::alloc_order_id(next_order_id));
                inserted.target_x = candidate.x;
                inserted.target_y = candidate.y;
                element.insert_order(order_index, inserted);
                order_index += 1;
            }
            first = second;
            order_index += 1;
        }
    }
}

// ─── Titbit update query ─────────────────────────────────────────

/// Real implementation of [`crate::titbit::TitbitUpdateQuery`] that
/// queries live entity state.  Replaces the old `StubQuery` that kept
/// all titbits alive unconditionally.
struct EntityTitbitQuery<'a> {
    sim: &'a crate::sim_rng::SimulationContext,
    entities: &'a crate::entities::Entities,
    sequence_manager: &'a crate::sequence::SequenceManager,
    follow_element: Option<EntityId>,
}

impl crate::titbit::TitbitUpdateQuery for EntityTitbitQuery<'_> {
    /// True when the entity should keep its weak-stunned titbit.
    ///
    /// - Soldiers in `WonderingAppleSauceInTheVisor` always keep stars.
    /// - Otherwise, stars stay only while the current animation is
    ///   `BeingWeakSword` or `BeingStunnedSword`.
    fn is_weak_or_stunned(&self, element: crate::titbit::ElementHandle) -> bool {
        use crate::ai::Substate;
        use crate::order::OrderType;

        let Some(entity_id) = self.entities.id_at_legacy_slot(element.0) else {
            return false;
        };
        let Some(entity) = self.entities.get(entity_id) else {
            return false;
        };

        // Soldiers in apple-sauce substate keep stars unconditionally.
        if let Entity::Soldier(s) = entity
            && s.npc.ai_substate() == Substate::WonderingAppleSauceInTheVisor
        {
            return true;
        }

        // Otherwise, check if the current animation is weak/stunned sword.
        // Orders live on the owning `SequenceElement.orders` now —
        // look up via the actor's current in-progress element.
        matches!(
            self.sequence_manager
                .current_order_for_actor(self.entities, entity_id)
                .map(|(_, _, o)| o.order_type),
            Some(OrderType::BeingWeakSword | OrderType::BeingStunnedSword)
        )
    }

    fn is_unconscious_and_alive(&self, element: crate::titbit::ElementHandle) -> bool {
        let Some(entity_id) = self.entities.id_at_legacy_slot(element.0) else {
            return false;
        };
        let Some(entity) = self.entities.get(entity_id) else {
            return false;
        };
        match entity {
            Entity::Pc(pc) => pc.human.unconscious && pc.pc.life_points > 0,
            Entity::Soldier(s) => s.human.unconscious && s.npc.life_points > 0,
            Entity::Civilian(c) => c.human.unconscious && c.npc.life_points > 0,
            _ => false,
        }
    }

    fn is_follow_element(&self, element: crate::titbit::ElementHandle) -> bool {
        // The entity the camera is currently locked onto (via
        // `SelectFollowElement` / `LockCameraOn`).
        self.follow_element
            .is_some_and(|id| id.index() == element.0)
    }

    fn is_hidden_posture(&self, element: crate::titbit::ElementHandle) -> bool {
        use crate::element::Posture;
        let Some(entity_id) = self.entities.id_at_legacy_slot(element.0) else {
            return false;
        };
        let Some(entity) = self.entities.get(entity_id) else {
            return false;
        };
        matches!(
            entity.element_data().posture(),
            Posture::Spy | Posture::Cloaked | Posture::Tree | Posture::AnonymousArcher
        )
    }

    fn random_u32(&self) -> u32 {
        crate::sim_rng::u32(self.sim, crate::sim_rng::RngSite::TitbitUpdate, ..)
    }
}

#[cfg(test)]
mod tests;
