//! Order system — movement/action commands given to characters.

use std::num::NonZeroU32;

// ---------------------------------------------------------------------------
// OrderType
// ---------------------------------------------------------------------------

/// Every possible action/animation a character can be ordered to perform.
///
/// Values are `#[repr(u32)]` with sequential discriminants starting at 0.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum OrderType {
    // -- Upright / idle --
    WaitingUprightBored = 0,
    WaitingUprightBoredRandom,
    TransitionWaitingUprightBoredWaitingUpright,
    WaitingUpright,
    TransitionWaitingUprightWaitingUprightBored,
    TransitionWaitingUprightWalkingUpright,
    WalkingUpright,
    WalkingStairs,
    TransitionWalkingUprightWaitingUpright,
    TransitionWalkingUprightRunningUpright,
    RunningUpright, // 10
    TransitionRunningUprightWalkingUpright,
    TransitionRunningUprightWaitingUpright,
    TransitionCrouchingDown,
    WaitingCrouched,
    TransitionWaitingCrouchedWalkingCrouched,
    WalkingCrouched,
    TransitionWalkingCrouchedWaitingCrouched,
    TransitionCrouchingUp,
    TransitionWaitingUprightClimbingWallUp,
    ClimbingWallUp, // 20
    ClimbingWallDown,
    TransitionClimbingWallUpWaitingCrouched,
    TransitionClimbingWallDownWaitingUpright,
    TransitionWaitingCrouchedClimbingWallDown,
    TransitionWaitingUprightJumpingUp,
    JumpingUp,
    TransitionJumpingUpWaitingCrouched,
    TransitionWaitingCrouchedJumpingDown,
    JumpingDown,
    TransitionJumpingDownWaitingCrouched, // 30
    TransitionWaitingUprightJumpingLong,
    JumpingLong,
    TransitionJumpingLongWaitingUpright,
    TransitionWaitingUprightClimbingLadderUp,
    ClimbingLadderUp,
    TransitionClimbingLadderUpWaitingCrouched,
    TransitionWaitingCrouchedClimbingLadderDown,
    ClimbingLadderDown,
    TransitionClimbingLadderDownWaitingUpright,
    ExtractingArrowUpright, // 40
    DyingUpright,
    ExtractingArrowCrouched,
    DyingCrouched,
    FallingBackUpright,
    BeingDeadFallenBack,
    FallingBackCrouched,
    BeingDead,
    BeingUnconscious,
    StandingUp,
    Turning, // 50
    TransitionWaitingUprightRunningUpright,
    TransitionRaisingSword,
    TransitionLoweringSword,
    WaitingSword,
    WalkingSword,
    WalkingBackwardsSword,
    StrafingRightSword,
    StrafingLeftSword,
    StrikingRightSmalltalk,
    StrikingLeftSmalltalk, // 60
    ParryingRightSmalltalk,
    ParryingLeftSmalltalk,
    StrikingLowRightSmalltalk,
    StrikingLowLeftSmalltalk,
    ParryingLowRightSmalltalk,
    ParryingLowLeftSmalltalk,
    StrikingStraightSword,
    StrikingStraightStrongSword,
    StrikingRightSword,
    StrikingLeftSword, // 70
    StrikingRoundRightSword,
    StrikingRoundLeftSword,
    StrikingSemiroundRightSword,
    StrikingSemiroundLeftSword,
    ExecutingSword,
    TransitionWaitingSwordParryingSword,
    ParryingSword,
    TransitionParryingSwordWaitingSword,
    ParryingLowSword,

    Reserved80, // 80

    TransitionWalkingUprightWalkingCrouched,
    TransitionWalkingCrouchedWalkingUpright,
    TransitionRunningUprightWalkingCrouched,
    TransitionWalkingCrouchedRunningUpright,

    // -- Bow animations --
    TransitionEquipBow, // 85
    TransitionUnequipBow,
    TransitionLoadingBow,
    TransitionUnloadBow,
    AimingWithBow,
    TransitionRaisingBow, // 90
    TransitionLoweringBow,
    AimingWithBowUp,
    ShootingWithBow,
    ShootingWithBowUp,

    Reserved95, // 95

    // -- Sword provocation --
    Provoking,

    // -- Jumping with sword --
    TransitionWaitingSwordJumpingLongSword,
    TransitionJumpingLongSwordWaitingSword,
    JumpingLongSword,

    StrikingDownSword, // 100

    // -- Injuries with sword --
    BeingWeakSword,
    BeingHitSword,
    BeingStunnedSword,
    ExtractingArrowSword,
    DyingSword,
    BeingDeadSword,
    FallingBackSword,
    BeingUnconsciousSword,
    BeingDeadFallenBackSword,
    StandingUpSword, // 110

    // -- Injuries with bow --
    ExtractingArrowBow,
    DyingBow,
    BeingDeadBow,
    FallingBackBow,
    BeingUnconsciousBow, // 115
    BeingDeadFallenBackBow,
    StandingUpBow,

    // -- Misc --
    BeingLiftedLittleJohn,
    BeingCarriedLittleJohn,
    BeingLiftedPeasantC, // 120
    BeingCarriedPeasantC,

    Searching,
    Hitting,
    ThrowingPurse,
    Paying, // 125
    Taking,

    Rolling,

    // -- Help to climb --
    ClimbingUpOnShoulders,
    ClimbingDownFromShoulders,
    WaitingOnShoulders, // 130
    TransitionWaitingOnShouldersJumpingUp,
    TransitionWaitingOnShouldersJumpingLong,
    WriggleUnderNet,
    WaitingCape,
    TransitionWaitingCapeWaitingUpright, // 135
    WaitingHidden,
    TransitionWaitingHiddenWaitingUpright,
    HandlingTarget,
    HittingTarget,
    WaitingAlerted, // 140
    TransitionWaitingUprightWaitingAlerted,
    TransitionWaitingAlertedWaitingUpright,
    WalkingAlerted,
    TransitionWaitingAlertedWalkingAlerted,
    TransitionWaitingAlertedRunningAlerted, // 145
    TransitionWalkingAlertedWaitingAlerted,
    TransitionRunningAlertedWalkingAlerted,
    TransitionRunningAlertedWaitingAlerted,
    TransitionWalkingAlertedRunningAlerted,

    TurningAlerted, // 150
    WalkingStairsAlerted,
    Menacing,
    TransitionWaitingSwordMenacing,
    TransitionMenacingWaitingSword,

    Placeholder155, // 155

    TransitionCharging,

    Placeholder157,
    TransitionWaitingUprightSitting,
    Sitting,
    TransitionSittingWaitingUpright, // 160
    Placeholder161,
    SleepingUpright,
    TransitionSleepingWaitingUpright,
    BeingStrangled,
    DrinkingAle, // 165

    GettingFreeFromWasp,

    Placeholder167,

    // -- Net --
    ThrowingNet,
    TakingNet,

    // -- Shield --
    RaisingShield, // 170
    WaitingShield,
    ParryingShield,
    LoweringShield,
    WalkingShield,
    WalkingBackwardsShield, // 175
    StrafingRightShield,
    StrafingLeftShield,

    // -- Drop --
    BeingDroppedLittleJohn,
    BeingDroppedPeasantC,

    // -- Help to climb --
    TransitionWaitingUprightHelpingClimbing, // 180
    TransitionHelpingClimbingWaitingUpright,
    WaitingHelpingClimbing,
    TransitionHelpingClimbingUp,

    // -- Lever --
    UsingLever,

    // -- Corpse transportation --
    TransitionWaitingUprightCarryingCorpse, // 185
    WaitingWithCorpse,
    WalkingWithCorpse,
    TransitionCarryingCorpseWaitingUpright,

    WakingUp,

    // -- Lying bonus items --
    BonusOne, // 190
    BonusTwo,
    BonusThree,
    BonusFour,
    BonusFive,

    // -- Objects --
    ObjectLying,
    ObjectFlying,
    ObjectBursting,

    Placeholder198,

    // -- Help to climb (cont.) --
    WaitingCarryingOnShoulders,
    TransitionHelpingClimbingDown, // 200
    WalkingCarryingOnShoulders,

    LookingRight,
    LookingLeft,
    LookingRightAlerted,
    LookingLeftAlerted,
    Pointing,
    TransitionWaitingAlertedLeaningOut,
    TransitionLeaningOutWaitingAlerted,
    LeaningOut,

    Target210, // 210
    Target211,
    Target212,
    Target213,
    Target214,

    FallingInHole, // 215

    GatheringSoldiers,

    Healing,
    ThrowingApple,

    BeingTied,
    ThrowingStone, // 220

    // -- Net (cont.) --
    NetUnfolding,
    NetMoving,
    NetBeingTaken,
    NetUnfoldingCrumpled,
    NetLyingCrumpled, // 225
    NetBeingTakenCrumpled,

    // -- Ladders alerted --
    TransitionWaitingUprightClimbingLadderUpAlerted,
    ClimbingLadderUpAlerted,
    TransitionClimbingLadderUpWaitingUprightAlerted,
    TransitionWaitingUprightClimbingLadderDownAlerted, // 230
    ClimbingLadderDownAlerted,
    TransitionClimbingLadderDownWaitingUprightAlerted,

    TransitionWaitingAlertedWaitingUprightOfficer,

    TransitionLoweringBowLeaningOut,
    TransitionRaisingBowLeaningOut, // 235
    AimingWithBowLeaningOut,
    ShootingWithBowLeaningOut,

    Eating,
    UnlockingDoor,
    Tying, // 240
    DroppingAle,
    SimulatingBeggar,
    ThrowingWaspNest,
    Strangling,
    RaisingToStrangle, // 245
    TransitionWaitingUprightSimulatingBeggar,
    TransitionSimulatingBeggarWaitingUpright,
    TakingCrouched,
    TransitionWaitingCarryingOnShouldersWaitingUpright,
    Weeping, // 250
    TransitionWaitingUprightListening,
    Listening,
    TransitionListeningWaitingUpright,
    Whistling,
    TransitionClimbingWallUpWaitingCrouchedCrenel,
    TransitionWaitingCrouchedClimbingWallDownCrenel,

    Placeholder257,
    Placeholder258,
    Placeholder259,
    Placeholder260,
    Placeholder261,
    Placeholder262,
    Placeholder263,

    // -- Beggar --
    ReceivingPurse,
    WaitingWithPurse, // 265
    TransitionWaitingWithPurseWaitingUpright,
    BeggarShowingFace,

    TransitionWaitingUprightSpecial,
    TransitionSpecialWaitingUpright,
    Special, // 270
    UnlockingTrap,

    TransitionEquipBowAnonymous,
    TransitionUnequipBowAnonymous,
    TransitionLoadingBowAnonymous,
    TransitionUnloadBowAnonymous,
    AimingWithBowAnonymous,
    TransitionRaisingBowAnonymous,
    TransitionLoweringBowAnonymous,
    AimingWithBowUpAnonymous,
    ShootingWithBowAnonymous, // 280
    ShootingWithBowUpAnonymous,
    SearchingCrouched,

    /// End of physical animations marker.
    NonanimationEnd,

    // -- Animations driven by scripts --
    PlayCustom,
    PlayCustomLooped,
    PlayCustomFreeze,
    PlayCustomFrozen,

    // -- Provisional animations --
    ProvPlouf,
    ProvExplode,

    // -- Rider charges --
    RiderCharging,

    // -- Non-animation actions (logical order commands) --
    /// Invalid / unset action.
    #[default]
    Invalid,

    Freezing,
    WaitingFreeLift,
    RunningStairs,
    PassingDoor,
    ClimbingLadderUpFast,
    ClimbingLadderDownFast,
    ClimbingWallUpFast,
    ClimbingWallDownFast,
    FallingLadderWall,
    FallingShoulders,
    CrossRoad,

    WalkingWithSword,
    RunningWithSword,
    WalkingWithShield,

    Select,

    FallingHitUpright,
    FallingHitWithBow,
    FallingHitWithSword,
    FallingHitCrouched,
    FallingHitHarderUpright,
    FallingHitHarderWithBow,
    FallingHitHarderWithSword,
    FallingHitHarderCrouched,

    FallingPushedUpright,
    FallingPushedWithBow,
    FallingPushedWithSword,
    FallingPushedCrouched,

    HidingBehindShield,
    TransitionWaitingSwordParryingSwordLow,

    DroppingAmmo,
    DroppingAmmoCrouched,
    DroppingAleCrouched,
    LyingStuckUnderNet,

    GettingWounded,

    WaitingCapeAnonymousArcher,
    TakingTarget,

    RefreshingSeek,
}

impl OrderType {
    /// Alias used by the patch system.
    pub const PATCH_INITIAL: Self = Self::TransitionRunningAlertedWaitingAlerted;
    /// Alias used by the patch system.
    pub const PATCH_TRANSITION: Self = Self::TransitionWalkingAlertedRunningAlerted;
    /// Alias used by the patch system.
    pub const PATCH_FINAL: Self = Self::TurningAlerted;
}

// ---------------------------------------------------------------------------
// OrderCompletion — side-channel data that the animation-driver needs when
// an order finishes.  Lives on `Order` so each order carries its own
// completion hook, letting per-arm side effects fire as the sprite
// reaches DONE / TERMINATED.
// ---------------------------------------------------------------------------

/// What should happen when an order finishes playing.
///
/// The default is [`OrderCompletion::AdvanceElement`]: pop this order off
/// the owning element via `do_next_order` and advance to the next (or
/// terminate the element when empty).  The non-default variants carry
/// extra data that must ride on the order itself — lockpick door id,
/// wasp-struggle cycle counter, etc.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum OrderCompletion {
    /// Default: `do_next_order` pops this order and advances /
    /// terminates the owning element when the animation terminates.
    #[default]
    AdvanceElement,

    /// Flip `door.locked_pc = false`, then terminate the lockpick
    /// sequence element.  Used by `Command::UnlockDoor` to free the
    /// door on animation end.
    UnlockDoor { door_id: crate::gate::DoorIndex },

    /// Resume `advance_door_pass` on the actor — the door-pass chain
    /// continues with the next step after a transition animation (crouch
    /// down, climb ladder, etc.) plays out.
    ResumeDoorPass,

    /// Advance to the next step of an `ActiveJump`.
    NextJumpStep,

    /// Soldier wasp-sting struggle cycles.  On each animation
    /// terminate, either re-push another `GETTING_FREE_FROM_WASP` order
    /// with `cycles_remaining - 1`, or terminate the element when the
    /// last cycle just finished.
    WaspStruggleCycle { cycles_remaining: u16 },
}

// ---------------------------------------------------------------------------
// Order
// ---------------------------------------------------------------------------

/// A single movement/action command given to a character.
///
/// Fields that are not yet needed are omitted; add them as more of the
/// sequence system is ported.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct Order {
    pub order_type: OrderType,
    /// Target position (2D map coordinates).
    pub target_x: f32,
    pub target_y: f32,
    /// The actor this order targets, if any (entity ID).
    pub target_actor: Option<u32>,
    /// Whether to compute facing direction from movement.
    pub compute_direction: bool,
    /// Tolerance for reaching the destination.
    pub tolerance: f32,
    /// Lock the AI while this order is being executed.
    pub lock_ai: bool,
    /// Play the animation in reverse.
    pub reverse: bool,
    /// Whether this order has been completed.
    pub done: bool,
    /// Movement flags propagated from AI GotoFlags.
    /// Carries `MoveFlags` bits (e.g. `RIDER_CHARGE`) through from the AI
    /// decision to the engine's pathfinding / movement dispatch.
    pub move_flags: u16,

    /// Per-order tag used by the sprite pipeline so animation bookings
    /// don't silently restart mid-play.
    ///
    /// Required at construction: `Order::new` takes the id as a
    /// parameter so the only way to build an `Order` is with a stamped
    /// id.  AI callers who don't have engine access construct
    /// [`AiOrderIntent`] instead; the drain site in
    /// `process_pending_ai_orders` stamps the id there.
    pub order_id: NonZeroU32,

    /// Optional target entity — the animation's antagonist.  `None` for
    /// animations that do not need a target (Turning, Pointing,
    /// RaisingShield, etc.).  Used by soldier Execute handlers that
    /// touch the antagonist on DONE / TERMINATED (DRINKING_ALE hides
    /// the bottle, TAKING picks up the coin/purse, etc.).
    pub antagonist: Option<crate::element::EntityId>,

    /// Side-channel work to perform when this order's animation
    /// completes (flip a door flag, resume a door-pass chain, advance a
    /// jump step, …).  Default `AdvanceElement` routes through
    /// `do_next_order`.
    pub completion: OrderCompletion,
}

impl Order {
    pub fn new(order_type: OrderType, x: f32, y: f32, order_id: NonZeroU32) -> Self {
        Self {
            order_type,
            target_x: x,
            target_y: y,
            target_actor: None,
            compute_direction: true,
            tolerance: 0.0,
            lock_ai: false,
            reverse: false,
            done: false,
            move_flags: 0,
            order_id,
            antagonist: None,
            completion: OrderCompletion::AdvanceElement,
        }
    }

    pub fn with_target_actor(mut self, actor_id: u32) -> Self {
        self.target_actor = Some(actor_id);
        self
    }

    /// Attach a target entity (the order's antagonist).  Required for
    /// interaction animations (DrinkAle → ale bottle, Take → purse,
    /// GettingFreeFromWasp → wasp, etc.).
    pub fn with_antagonist(mut self, antagonist: crate::element::EntityId) -> Self {
        self.antagonist = Some(antagonist);
        self
    }

    /// Override the default `AdvanceElement` completion with a custom
    /// side-effect hook (UnlockDoor, ResumeDoorPass, NextJumpStep,
    /// WaspStruggleCycle).
    pub fn with_completion(mut self, completion: OrderCompletion) -> Self {
        self.completion = completion;
        self
    }

    /// Reroll `order_id` so the sprite pipeline treats the mutated
    /// order as a fresh booking.  Used by the BORED ↔ RANDOM idle cycle
    /// and any other in-place order mutation that wants to re-trigger
    /// the animation start hook.
    pub fn reseed_id(&mut self, new_id: NonZeroU32) {
        self.order_id = new_id;
    }

    /// Test-only constructor that stamps a placeholder id.  Production
    /// code must route through [`alloc_order_id`] so rollback
    /// snapshots reproduce the same id sequence; test code building
    /// fixture sequences in isolation does not.
    #[cfg(test)]
    pub fn test_new(order_type: OrderType, x: f32, y: f32) -> Self {
        Self::new(order_type, x, y, NonZeroU32::new(1).unwrap())
    }
}

/// Bump `counter` by one and return the allocated id as a
/// [`NonZeroU32`], skipping zero on wrap-around.  All engine-side order
/// id allocation flows through here (directly via
/// `EngineInner::alloc_order_id`, or via the `&mut u32` counter the
/// animation/jump/bow/movement paths thread through helper fns).  The
/// counter itself stays a plain `u32` so it fits in rollback snapshots
/// and round-trips through serde without conversion.
#[inline]
pub fn alloc_order_id(counter: &mut u32) -> NonZeroU32 {
    // Skip zero so the returned id can be a NonZeroU32.  In practice
    // `counter` starts at 1 and wraps billions of allocations later,
    // but we still skip 0 on wrap for safety.
    if *counter == 0 {
        *counter = 1;
    }
    let v = *counter;
    *counter = counter.wrapping_add(1);
    NonZeroU32::new(v).expect("alloc_order_id: zero escaped the skip")
}

// ---------------------------------------------------------------------------
// AiOrderIntent — AI-side pending order without an id yet.
// ---------------------------------------------------------------------------

/// An order produced by an AI controller that hasn't been stamped with
/// an `order_id` yet.  AI code (`AiBase::go_to`, `raise_shield`,
/// `point_to`, etc.) has no `EngineInner` reference and can't allocate
/// an id, so it pushes `AiOrderIntent`s onto `AiBase.pending_orders`.
/// The engine drains these after each `think()` call in
/// `process_pending_ai_orders`, allocates ids via
/// `EngineInner::alloc_order_id`, and calls [`AiOrderIntent::stamp`]
/// to produce real [`Order`] values.
///
/// Same shape as [`Order`] minus `order_id` and `completion` (AI
/// orders always use the default `AdvanceElement` completion).
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct AiOrderIntent {
    pub order_type: OrderType,
    pub target_x: f32,
    pub target_y: f32,
    pub target_actor: Option<u32>,
    pub compute_direction: bool,
    pub tolerance: f32,
    pub lock_ai: bool,
    pub reverse: bool,
    pub done: bool,
    pub move_flags: u16,
    /// Movement speed multiplier copied onto generated movement sequence elements.
    pub speed_factor: f32,
    pub no_halt: bool,
    pub antagonist: Option<crate::element::EntityId>,
    /// When set, the engine drain runs
    /// `FastFindGrid::find_authorized_position` against the actor's
    /// `MoveBox + (target_x, target_y)` and rewrites `target_x/y` to
    /// the snapped centre on success; on failure it sets
    /// `AiController::couldnt_reachpoint` and skips the launch.
    pub find_accessible: bool,
    /// Pre-flight `FastFindGrid::is_straight_movement_authorized`
    /// between the actor and the destination; on rejection the engine
    /// drain sets `couldnt_reachpoint` and skips the launch.  Only
    /// meaningful when paired with a straight (`!compute_direction`)
    /// move.
    pub ask_obstacle: bool,
}

impl AiOrderIntent {
    pub fn new(order_type: OrderType, x: f32, y: f32) -> Self {
        Self {
            order_type,
            target_x: x,
            target_y: y,
            target_actor: None,
            compute_direction: true,
            tolerance: 0.0,
            lock_ai: false,
            reverse: false,
            done: false,
            move_flags: 0,
            speed_factor: 1.0,
            no_halt: false,
            antagonist: None,
            find_accessible: false,
            ask_obstacle: false,
        }
    }

    pub fn face_toward(x: f32, y: f32) -> Self {
        Self::new(OrderType::Turning, x, y)
    }

    /// Stamp an allocated `order_id` onto this intent to produce a
    /// real [`Order`].  Called by the AI drain site in
    /// `process_pending_ai_orders`.
    pub fn stamp(self, order_id: NonZeroU32) -> Order {
        Order {
            order_type: self.order_type,
            target_x: self.target_x,
            target_y: self.target_y,
            target_actor: self.target_actor,
            compute_direction: self.compute_direction,
            tolerance: self.tolerance,
            lock_ai: self.lock_ai,
            reverse: self.reverse,
            done: self.done,
            move_flags: self.move_flags,
            order_id,
            antagonist: self.antagonist,
            completion: OrderCompletion::AdvanceElement,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_type_default_is_invalid() {
        assert_eq!(OrderType::default(), OrderType::Invalid);
    }

    #[test]
    fn order_type_discriminants_match_expected() {
        // Spot-check key discriminants against their expected values.
        assert_eq!(OrderType::WaitingUprightBored as u32, 0);
        assert_eq!(OrderType::RunningUpright as u32, 10);
        assert_eq!(OrderType::ClimbingWallUp as u32, 20);
        assert_eq!(OrderType::TransitionJumpingDownWaitingCrouched as u32, 30);
        assert_eq!(OrderType::ExtractingArrowUpright as u32, 40);
        assert_eq!(OrderType::Turning as u32, 50);
        assert_eq!(OrderType::StrikingLeftSmalltalk as u32, 60);
        assert_eq!(OrderType::StrikingLeftSword as u32, 70);
        assert_eq!(OrderType::Reserved80 as u32, 80);
        assert_eq!(OrderType::TransitionEquipBow as u32, 85);
        assert_eq!(OrderType::TransitionRaisingBow as u32, 90);
        assert_eq!(OrderType::Reserved95 as u32, 95);
        assert_eq!(OrderType::StrikingDownSword as u32, 100);
        assert_eq!(OrderType::StandingUpSword as u32, 110);
        assert_eq!(OrderType::BeingUnconsciousBow as u32, 115);
        assert_eq!(OrderType::BeingLiftedPeasantC as u32, 120);
        assert_eq!(OrderType::Paying as u32, 125);
        assert_eq!(OrderType::WaitingOnShoulders as u32, 130);
        assert_eq!(OrderType::TransitionWaitingCapeWaitingUpright as u32, 135);
        assert_eq!(OrderType::WaitingAlerted as u32, 140);
        assert_eq!(
            OrderType::TransitionWaitingAlertedRunningAlerted as u32,
            145
        );
        assert_eq!(OrderType::TurningAlerted as u32, 150);
        assert_eq!(OrderType::Placeholder155 as u32, 155);
        assert_eq!(OrderType::TransitionSittingWaitingUpright as u32, 160);
        assert_eq!(OrderType::DrinkingAle as u32, 165);
        assert_eq!(OrderType::RaisingShield as u32, 170);
        assert_eq!(OrderType::WalkingBackwardsShield as u32, 175);
        assert_eq!(
            OrderType::TransitionWaitingUprightHelpingClimbing as u32,
            180
        );
        assert_eq!(
            OrderType::TransitionWaitingUprightCarryingCorpse as u32,
            185
        );
        assert_eq!(OrderType::BonusOne as u32, 190);
        assert_eq!(OrderType::TransitionHelpingClimbingDown as u32, 200);
        assert_eq!(OrderType::Target210 as u32, 210);
        assert_eq!(OrderType::FallingInHole as u32, 215);
        assert_eq!(OrderType::ThrowingStone as u32, 220);
        assert_eq!(OrderType::NetLyingCrumpled as u32, 225);
        assert_eq!(
            OrderType::TransitionWaitingUprightClimbingLadderDownAlerted as u32,
            230
        );
        assert_eq!(OrderType::TransitionRaisingBowLeaningOut as u32, 235);
        assert_eq!(OrderType::Tying as u32, 240);
        assert_eq!(OrderType::RaisingToStrangle as u32, 245);
        assert_eq!(OrderType::Weeping as u32, 250);
        assert_eq!(OrderType::WaitingWithPurse as u32, 265);
        assert_eq!(OrderType::Special as u32, 270);
        assert_eq!(OrderType::ShootingWithBowAnonymous as u32, 280);
    }

    #[test]
    fn order_type_patch_aliases() {
        assert_eq!(
            OrderType::PATCH_INITIAL,
            OrderType::TransitionRunningAlertedWaitingAlerted
        );
        assert_eq!(
            OrderType::PATCH_TRANSITION,
            OrderType::TransitionWalkingAlertedRunningAlerted
        );
        assert_eq!(OrderType::PATCH_FINAL, OrderType::TurningAlerted);
    }

    #[test]
    fn order_builder() {
        let order = Order::test_new(OrderType::WalkingUpright, 10.0, 20.0).with_target_actor(42);

        assert_eq!(order.order_type, OrderType::WalkingUpright);
        assert_eq!(order.target_x, 10.0);
        assert_eq!(order.target_y, 20.0);
        assert_eq!(order.target_actor, Some(42));
    }

    #[test]
    fn order_defaults() {
        let order = Order::test_new(OrderType::Invalid, 0.0, 0.0);
        assert_eq!(order.target_actor, None);
    }

    #[test]
    fn serde_roundtrip_order() {
        let order = Order::test_new(OrderType::ShootingWithBow, 100.0, 200.0).with_target_actor(7);

        let json = serde_json::to_string(&order).unwrap();
        let back: Order = serde_json::from_str(&json).unwrap();

        assert_eq!(back.order_type, OrderType::ShootingWithBow);
        assert_eq!(back.target_x, 100.0);
        assert_eq!(back.target_y, 200.0);
        assert_eq!(back.target_actor, Some(7));
    }
}
