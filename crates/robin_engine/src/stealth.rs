//! Stealth mechanics — hiding, disguises, crouching, detection modifiers.
//!
//! All stealth transition logic is consolidated here so the engine
//! tick can call `stealth::validate_*` before dispatching, and
//! `stealth::execute_*` to get the animation + posture result.

use crate::element::{ActionState, Command, Posture};
use crate::element_kinds::{CRAWLING_OFFSETS_X, CRAWLING_OFFSETS_Y};
use crate::order::OrderType;

// ─── Constants ─────────────────────────────────────────────────────

/// Minimum IQ for an NPC to detect a beggar disguise during a seek.
pub const CHECK_BEGGAR_MIN_IQ: i32 = 30;

/// Squared near-detection radius.  Within this distance NPCs auto-detect
/// enemies regardless of facing.
pub const SQR_NEAR_DETECTION_RADIUS: f32 = 225.0; // 15²

// ─── Transition result ─────────────────────────────────────────────

/// The outcome of a validated stealth transition: which animation to
/// play and what posture/action-state the character ends up in.
#[derive(Debug, Clone, Copy)]
pub struct StealthTransition {
    /// Transition animation to queue.
    pub animation: OrderType,
    /// Posture after the transition animation finishes.
    pub result_posture: Posture,
    /// Action state after the transition.
    pub result_action_state: ActionState,
    /// Whether to call Stop() before executing (beggar anti-run fix).
    pub stop_first: bool,
}

// ─── Crouch transitions ────────────────────────────────────────────

/// Validate whether a character can crouch down.
///
/// Requires posture Upright and one of Waiting / Moving / MovingFast.
/// Cannot crouch during swordfight.
pub fn can_crouch_down(
    posture: Posture,
    action_state: ActionState,
    is_swordfighting: bool,
) -> bool {
    if is_swordfighting {
        return false;
    }
    if posture != Posture::Upright {
        return false;
    }
    matches!(
        action_state,
        ActionState::Waiting | ActionState::Moving | ActionState::MovingFast
    )
}

/// Execute crouch-down: returns the transition info.
pub fn crouch_down() -> StealthTransition {
    StealthTransition {
        animation: OrderType::TransitionCrouchingDown,
        result_posture: Posture::Crouched,
        result_action_state: ActionState::Waiting,
        stop_first: false,
    }
}

/// Validate whether a character can stand up from crouching.
///
/// Requires posture Crouched and one of Waiting / Moving / MovingFast.
pub fn can_crouch_up(posture: Posture, action_state: ActionState, is_swordfighting: bool) -> bool {
    if is_swordfighting {
        return false;
    }
    if posture != Posture::Crouched {
        return false;
    }
    matches!(
        action_state,
        ActionState::Waiting | ActionState::Moving | ActionState::MovingFast
    )
}

/// Execute stand-up from crouch: returns the transition info.
pub fn crouch_up() -> StealthTransition {
    StealthTransition {
        animation: OrderType::TransitionCrouchingUp,
        result_posture: Posture::Upright,
        result_action_state: ActionState::Waiting,
        stop_first: false,
    }
}

// ─── Beggar disguise transitions ───────────────────────────────────

/// Validate whether a character can enter beggar disguise.
///
/// Requires posture Upright and action state Waiting.
pub fn can_enter_beggar(posture: Posture, action_state: ActionState) -> bool {
    posture == Posture::Upright && action_state == ActionState::Waiting
}

/// Execute enter-beggar: returns the transition info.
///
/// Note: `stop_first = true` — the actor must be stopped before the
/// transition to avoid a beggar-and-run bug.
pub fn enter_beggar() -> StealthTransition {
    StealthTransition {
        animation: OrderType::TransitionWaitingUprightSimulatingBeggar,
        result_posture: Posture::SimulatingBeggar,
        result_action_state: ActionState::Waiting,
        stop_first: true,
    }
}

/// Validate whether a character can leave beggar disguise.
///
/// Requires posture SimulatingBeggar and action state Waiting.
pub fn can_leave_beggar(posture: Posture, action_state: ActionState) -> bool {
    posture == Posture::SimulatingBeggar && action_state == ActionState::Waiting
}

/// Execute leave-beggar: returns the transition info.
pub fn leave_beggar() -> StealthTransition {
    StealthTransition {
        animation: OrderType::TransitionSimulatingBeggarWaitingUpright,
        result_posture: Posture::Upright,
        result_action_state: ActionState::Waiting,
        stop_first: false,
    }
}

// ─── Helping-to-climb transitions ──────────────────────────────────

/// Validate whether a character can enter the helping-to-climb stance
/// (kneel down to let a partner climb on their shoulders).
///
/// Requires posture Upright and action state Waiting.
pub fn can_enter_helping_climb(posture: Posture, action_state: ActionState) -> bool {
    posture == Posture::Upright && action_state == ActionState::Waiting
}

/// Execute enter-helping-climb: returns the transition info.
pub fn enter_helping_climb() -> StealthTransition {
    StealthTransition {
        animation: OrderType::TransitionWaitingUprightHelpingClimbing,
        result_posture: Posture::HelpingToClimb,
        result_action_state: ActionState::Waiting,
        stop_first: false,
    }
}

/// Validate whether a character can leave the helping-to-climb stance.
///
/// Requires action state Waiting and posture HelpingToClimb or
/// CarryingOnShoulders — the leave-helping-climb dispatch switches
/// animation based on whether anyone is still on the carrier's
/// shoulders.
pub fn can_leave_helping_climb(posture: Posture, action_state: ActionState) -> bool {
    matches!(
        posture,
        Posture::HelpingToClimb | Posture::CarryingOnShoulders
    ) && action_state == ActionState::Waiting
}

/// Execute leave-helping-climb: returns the transition info.
///
/// The carrier either has a partner still on their shoulders (two
/// orders: climb-down + helping→upright) or they don't (single order:
/// helping→upright).  We collapse to the single-order variant — the
/// carried-partner state machine is driven separately through the
/// carrier/carried entity fields (see the `ClimbUpOnShoulders` dispatch
/// in tick.rs).
pub fn leave_helping_climb() -> StealthTransition {
    StealthTransition {
        animation: OrderType::TransitionHelpingClimbingWaitingUpright,
        result_posture: Posture::Upright,
        result_action_state: ActionState::Waiting,
        stop_first: false,
    }
}

// ─── Spy (cape) disguise transitions ──────────────────────────────
//
// Entry is not a runtime command. The Spy posture is set by
// `InitializeAction` when the level data spawns a PC with the
// waiting-cape initial animation. Only the leave transition is
// reachable at runtime, via `MakePostureTransition`.

/// Validate whether a character can leave spy (cape) disguise.
///
/// Unconditional transition when posture is Spy or AnonymousArcher
/// and action state is Waiting or Moving.
pub fn can_leave_spy(posture: Posture, action_state: ActionState) -> bool {
    (posture == Posture::Spy || posture == Posture::AnonymousArcher)
        && matches!(action_state, ActionState::Waiting | ActionState::Moving)
}

/// Transition animation for leaving spy (cape) posture.
pub fn leave_spy() -> StealthTransition {
    StealthTransition {
        animation: OrderType::TransitionWaitingCapeWaitingUpright,
        result_posture: Posture::Upright,
        result_action_state: ActionState::Waiting,
        stop_first: false,
    }
}

// ─── Tree (hidden in bush) disguise transitions ──────────────────
//
// Entry is not a runtime command. Tree posture is set by
// `InitializeAction` when the level data spawns a PC with the
// waiting-hidden initial animation.

/// Validate whether a character can leave tree (hidden) disguise.
///
/// Unconditional transition when posture is Tree and action state is
/// Waiting or Moving.
pub fn can_leave_tree(posture: Posture, action_state: ActionState) -> bool {
    posture == Posture::Tree && matches!(action_state, ActionState::Waiting | ActionState::Moving)
}

/// Transition animation for leaving tree (hidden in bush) posture.
pub fn leave_tree() -> StealthTransition {
    StealthTransition {
        animation: OrderType::TransitionWaitingHiddenWaitingUpright,
        result_posture: Posture::Upright,
        result_action_state: ActionState::Waiting,
        stop_first: false,
    }
}

/// Soldier-specific MakePostureTransition override: a LeaningOut
/// soldier that receives a command requiring Upright posture must play
/// `TransitionLeaningOutWaitingAlerted` before the command runs.
pub fn leave_leaning_out() -> StealthTransition {
    StealthTransition {
        animation: OrderType::TransitionLeaningOutWaitingAlerted,
        result_posture: Posture::Upright,
        result_action_state: ActionState::Waiting,
        stop_first: false,
    }
}

/// NPC sitting → upright transition.
pub fn leave_sitting() -> StealthTransition {
    StealthTransition {
        animation: OrderType::TransitionSittingWaitingUpright,
        result_posture: Posture::Upright,
        result_action_state: ActionState::Waiting,
        stop_first: false,
    }
}

/// NPC leisure → upright transition.  Queued when a command requires
/// Upright and does not allow Leisure.
pub fn leave_leisure() -> StealthTransition {
    StealthTransition {
        animation: OrderType::TransitionSpecialWaitingUpright,
        result_posture: Posture::Upright,
        result_action_state: ActionState::Waiting,
        stop_first: false,
    }
}

/// Get the leave-disguise transition for a hidden posture.
/// Returns `None` if the posture is not a disguise that needs leaving.
///
/// Also covers the soldier-only `Posture::LeaningOut` → `Upright`
/// transition and the NPC `Sitting` / `Leisure` → `Upright`
/// transitions.
pub fn leave_disguise(posture: Posture) -> Option<StealthTransition> {
    match posture {
        Posture::Spy | Posture::AnonymousArcher => Some(leave_spy()),
        Posture::Tree => Some(leave_tree()),
        Posture::SimulatingBeggar => Some(leave_beggar()),
        Posture::LeaningOut => Some(leave_leaning_out()),
        Posture::Sitting => Some(leave_sitting()),
        Posture::Leisure => Some(leave_leisure()),
        _ => None,
    }
}

// ─── Stealth command validation ────────────────────────────────────

/// Check whether a stealth command can be executed given the current
/// entity state.  Returns `true` if the command is valid.
pub fn can_execute_stealth_command(
    command: Command,
    posture: Posture,
    action_state: ActionState,
    is_swordfighting: bool,
) -> bool {
    match command {
        Command::CrouchDown => can_crouch_down(posture, action_state, is_swordfighting),
        Command::CrouchUp => can_crouch_up(posture, action_state, is_swordfighting),
        Command::EnterBeggar => can_enter_beggar(posture, action_state),
        Command::LeaveBeggar => can_leave_beggar(posture, action_state),
        Command::EnterHelpingClimb => can_enter_helping_climb(posture, action_state),
        Command::LeaveHelpingClimb => can_leave_helping_climb(posture, action_state),
        Command::LeaveSpy => can_leave_spy(posture, action_state),
        Command::LeaveTree => can_leave_tree(posture, action_state),
        _ => false,
    }
}

/// Get the transition for a stealth command, if the command is a
/// stealth command.  Returns `None` for non-stealth commands.
pub fn stealth_transition(command: Command) -> Option<StealthTransition> {
    match command {
        Command::CrouchDown => Some(crouch_down()),
        Command::CrouchUp => Some(crouch_up()),
        Command::EnterBeggar => Some(enter_beggar()),
        Command::LeaveBeggar => Some(leave_beggar()),
        Command::EnterHelpingClimb => Some(enter_helping_climb()),
        Command::LeaveHelpingClimb => Some(leave_helping_climb()),
        Command::LeaveSpy => Some(leave_spy()),
        Command::LeaveTree => Some(leave_tree()),
        _ => None,
    }
}

// ─── Auto-leave helpers ────────────────────────────────────────────

/// True when the command pairs `MUST_BE_UPRIGHT` with
/// `CAN_BE_LEANING_OUT` — the actor should keep its lean-out pose
/// rather than unsticking before the command.
///
/// Bow-shooting commands set the flag pair.  Soldier's own `LEAN_OUT`
/// also sets it but is not in `command_requires_upright`, so it can't
/// hit the auto-leave path.
pub fn command_allows_leaning_out(command: Command) -> bool {
    matches!(command, Command::ShootBow | Command::ShootBowOnce)
}

/// True when the command pairs `MUST_BE_UPRIGHT` with
/// `CAN_BE_ANONYMOUS_ARCHER` — the actor should keep the
/// anonymous-archer (cape-hood) disguise rather than unsticking before
/// the command.
///
/// All bow-handling commands (equip, unequip, raise, lower, shoot) set
/// the flag pair.
pub fn command_allows_anonymous_archer(command: Command) -> bool {
    matches!(
        command,
        Command::EquipBow
            | Command::EquipBowUp
            | Command::EquipBowDown
            | Command::UnequipBow
            | Command::LowerBow
            | Command::RaiseBow
            | Command::ShootBow
            | Command::ShootBowOnce
    )
}

/// True when the given command requires the character to be in Upright
/// posture.  Used by the engine tick to auto-leave disguises before
/// executing the command.
pub fn command_requires_upright(command: Command) -> bool {
    matches!(
        command,
        Command::Move
            | Command::Seek
            | Command::PrepareSwordfight
            | Command::EnterSwordfight
            | Command::EquipBow
            | Command::EquipBowUp
            | Command::EquipBowDown
            | Command::ShootBow
            | Command::HitCmd
            | Command::ThrowApple
            | Command::ThrowStone
            | Command::ThrowPurse
            | Command::ThrowWaspNest
            | Command::ThrowNet
            | Command::StrangleCmd
            | Command::TakeCorpse
            | Command::Take
            | Command::Pay
            | Command::TieCmd
            | Command::WhistleCmd
            | Command::UseLever
            | Command::HandleTarget
            | Command::HitTarget
            | Command::TakeTarget
            | Command::EatCmd
            | Command::RaiseShield
            | Command::HideBehindShield
            | Command::EnterHelpingClimb
            | Command::ClimbUpOnShoulders
            | Command::EnterBeggar
            | Command::EnterListen
            | Command::JumpCmd
            | Command::PassDoor
            // NPC-specific commands all require Upright, so e.g. a
            // sitting NPC who receives `Point` first stands up.  The
            // leisure exemption is encoded by the leave-leisure
            // transition not being installed when the actor is already
            // in Leisure.
            | Command::Point
            | Command::SitDown
            | Command::BeggarShowFace
            | Command::EnterLeisure
    )
}

/// Eye-point Z offset for a given posture.
///
/// Used by the VIEWER (NPC) side of the near-auto-visible distance check
/// and by PC blip/seen-by-pc geometry. Riders get a +60 instead of +45
/// for the upright-group.  Lying/dead postures use a +5 base; the
/// direction-dependent crawling XY shift lives in [`eye_point_xy`].
pub fn eye_z_for_posture(posture: Posture, is_rider: bool) -> f32 {
    match posture {
        // Upright-group (riders get +60).
        Posture::Upright
        | Posture::Spy
        | Posture::Leisure
        | Posture::Siesta
        | Posture::CarryingCorpse
        | Posture::HelpingToClimb
        | Posture::CarryingOnShoulders
        | Posture::AnonymousArcher
        | Posture::OnLadder
        | Posture::OnWall
        | Posture::Flying => {
            if is_rider {
                60.0
            } else {
                45.0
            }
        }
        // LeaningOut always gets +45 in the original, plus an XY bend
        // shift handled by `leaning_out_xy_offset`.
        Posture::LeaningOut => 45.0,
        // Carried on shoulders → +85.
        Posture::OnShoulders => 85.0,
        // Low postures → +25.
        Posture::Crouched | Posture::Sitting | Posture::SimulatingBeggar | Posture::Tree => 25.0,
        // Lying/dead variants → +5 (plus crawling XY offsets via
        // `eye_point_xy`).
        Posture::Lying
        | Posture::Dead
        | Posture::DeadBack
        | Posture::StuckUnderNet
        | Posture::Tied => 5.0,
        Posture::Carried | Posture::Undefined | Posture::Unused => {
            panic!("eye_z_for_posture called for posture without an eye point: {posture:?}")
        }
    }
}

/// Detection-point Z offset for a given posture.
///
/// Used by the TARGET side of the near-auto-visible check. Differs from
/// `eye_z_for_posture` in two places: lying variants use +2 rather than
/// +5 with crawling offsets, and `Carried` is enumerated with +25.
/// LeaningOut also adds a direction-vector × 40 XY shift which callers
/// can fetch via `leaning_out_xy_offset`.
pub fn detection_z_for_posture(posture: Posture, is_rider: bool) -> f32 {
    match posture {
        // Upright-group (riders get +60).
        Posture::Upright
        | Posture::Spy
        | Posture::Leisure
        | Posture::Siesta
        | Posture::CarryingCorpse
        | Posture::HelpingToClimb
        | Posture::CarryingOnShoulders
        | Posture::AnonymousArcher
        | Posture::OnLadder
        | Posture::OnWall
        | Posture::Flying => {
            if is_rider {
                60.0
            } else {
                45.0
            }
        }
        // LeaningOut → +45 Z (plus XY bend vector).
        Posture::LeaningOut => 45.0,
        // On shoulders → +85.
        Posture::OnShoulders => 85.0,
        // Low postures and carried → +25.
        Posture::Crouched
        | Posture::Sitting
        | Posture::SimulatingBeggar
        | Posture::Tree
        | Posture::Carried => 25.0,
        // Lying/dead/tied variants → +2.
        Posture::Lying
        | Posture::Dead
        | Posture::DeadBack
        | Posture::StuckUnderNet
        | Posture::Tied => 2.0,
        Posture::Undefined | Posture::Unused => {
            panic!("detection_z_for_posture called for undefined posture: {posture:?}")
        }
    }
}

/// LeaningOut XY bend vector: the detection / eyes point projects
/// `direction_vector * 40` forward so a guard leaning out a window is
/// "seen" at the window-projection position rather than the anchor
/// position. Callers pass in the actor's 2D facing direction vector.
pub fn leaning_out_xy_offset(direction_x: f32, direction_y: f32) -> (f32, f32) {
    (direction_x * 40.0, direction_y * 40.0)
}

/// Apply the LeaningOut detection-point XY shift to a ground-plane position,
/// for use as the target of a `VisibilityQuery` cone / distance test.
///
/// When posture is `LeaningOut`, the detection point projects
/// `direction × 40` forward so the cone test sees a window-leaning
/// soldier at their projected position rather than the anchor sprite
/// position.  Other postures return the input unchanged.
///
/// `direction` is the actor's 16-sector facing.
pub fn detection_point_xy(
    ground: crate::geo2d::Point2D,
    posture: Posture,
    direction: i16,
) -> crate::geo2d::Point2D {
    if posture != Posture::LeaningOut {
        return ground;
    }
    let (dx, dy) = crate::element_kinds::direction_vector_16(direction);
    let (sx, sy) = leaning_out_xy_offset(dx, dy);
    crate::geo2d::pt(ground.x + sx, ground.y + sy)
}

/// Apply the posture-dependent eye-point XY shift to a ground-plane
/// position. This is the XY half of C++ `ComputeEyesPoint`.
pub fn eye_point_xy(
    ground: crate::geo2d::Point2D,
    posture: Posture,
    direction: i16,
    emergency_lying: bool,
) -> crate::geo2d::Point2D {
    match posture {
        Posture::LeaningOut => detection_point_xy(ground, posture, direction),
        Posture::Lying
        | Posture::Dead
        | Posture::DeadBack
        | Posture::StuckUnderNet
        | Posture::Tied => {
            let dir = direction.rem_euclid(16) as usize;
            let scale = if emergency_lying { 0.5 } else { 1.0 };
            crate::geo2d::pt(
                ground.x + scale * CRAWLING_OFFSETS_X[dir],
                ground.y + scale * CRAWLING_OFFSETS_Y[dir],
            )
        }
        _ => ground,
    }
}

/// Select the appropriate damage animation variant based on posture.
///
/// Returns `true` when the crouched variants of the hit/dying
/// animations should be used, `false` for the upright variants.
pub fn use_crouched_damage_anims(posture: Posture) -> bool {
    matches!(
        posture,
        Posture::Crouched | Posture::SimulatingBeggar | Posture::Tree
    )
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crouch_down_requires_upright() {
        assert!(can_crouch_down(
            Posture::Upright,
            ActionState::Waiting,
            false
        ));
        assert!(can_crouch_down(
            Posture::Upright,
            ActionState::Moving,
            false
        ));
        assert!(can_crouch_down(
            Posture::Upright,
            ActionState::MovingFast,
            false
        ));
        assert!(!can_crouch_down(
            Posture::Crouched,
            ActionState::Waiting,
            false
        ));
        assert!(!can_crouch_down(
            Posture::Lying,
            ActionState::Waiting,
            false
        ));
        assert!(!can_crouch_down(Posture::Spy, ActionState::Waiting, false));
    }

    #[test]
    fn crouch_down_blocked_during_swordfight() {
        assert!(!can_crouch_down(
            Posture::Upright,
            ActionState::Waiting,
            true
        ));
    }

    #[test]
    fn crouch_down_blocked_during_bow() {
        assert!(!can_crouch_down(
            Posture::Upright,
            ActionState::AimingWithBow,
            false
        ));
    }

    #[test]
    fn crouch_up_requires_crouched() {
        assert!(can_crouch_up(
            Posture::Crouched,
            ActionState::Waiting,
            false
        ));
        assert!(can_crouch_up(Posture::Crouched, ActionState::Moving, false));
        assert!(!can_crouch_up(
            Posture::Upright,
            ActionState::Waiting,
            false
        ));
        assert!(!can_crouch_up(
            Posture::SimulatingBeggar,
            ActionState::Waiting,
            false
        ));
    }

    #[test]
    fn enter_beggar_requires_upright_waiting() {
        assert!(can_enter_beggar(Posture::Upright, ActionState::Waiting));
        assert!(!can_enter_beggar(Posture::Upright, ActionState::Moving));
        assert!(!can_enter_beggar(Posture::Crouched, ActionState::Waiting));
    }

    #[test]
    fn leave_beggar_requires_beggar_waiting() {
        assert!(can_leave_beggar(
            Posture::SimulatingBeggar,
            ActionState::Waiting
        ));
        assert!(!can_leave_beggar(Posture::Upright, ActionState::Waiting));
        assert!(!can_leave_beggar(
            Posture::SimulatingBeggar,
            ActionState::Moving
        ));
    }

    #[test]
    fn crouch_down_transition_animation() {
        let t = crouch_down();
        assert_eq!(t.animation, OrderType::TransitionCrouchingDown);
        assert_eq!(t.result_posture, Posture::Crouched);
        assert!(!t.stop_first);
    }

    #[test]
    fn enter_beggar_stops_first() {
        let t = enter_beggar();
        assert!(t.stop_first);
        assert_eq!(
            t.animation,
            OrderType::TransitionWaitingUprightSimulatingBeggar
        );
        assert_eq!(t.result_posture, Posture::SimulatingBeggar);
    }

    #[test]
    fn leave_spy_requires_spy_posture() {
        assert!(can_leave_spy(Posture::Spy, ActionState::Waiting));
        assert!(can_leave_spy(
            Posture::AnonymousArcher,
            ActionState::Waiting
        ));
        assert!(!can_leave_spy(Posture::Upright, ActionState::Waiting));
        assert!(!can_leave_spy(Posture::Tree, ActionState::Waiting));
    }

    #[test]
    fn leave_tree_requires_tree_posture() {
        assert!(can_leave_tree(Posture::Tree, ActionState::Waiting));
        assert!(!can_leave_tree(Posture::Upright, ActionState::Waiting));
        assert!(!can_leave_tree(Posture::Spy, ActionState::Waiting));
    }

    #[test]
    fn leave_disguise_dispatch() {
        assert!(leave_disguise(Posture::Spy).is_some());
        assert!(leave_disguise(Posture::AnonymousArcher).is_some());
        assert!(leave_disguise(Posture::Tree).is_some());
        assert!(leave_disguise(Posture::SimulatingBeggar).is_some());
        assert!(leave_disguise(Posture::Upright).is_none());
        assert!(leave_disguise(Posture::Crouched).is_none());
    }

    #[test]
    fn spy_leave_animation() {
        let t = leave_spy();
        assert_eq!(t.animation, OrderType::TransitionWaitingCapeWaitingUpright);
        assert_eq!(t.result_posture, Posture::Upright);
    }

    #[test]
    fn tree_leave_animation() {
        let t = leave_tree();
        assert_eq!(
            t.animation,
            OrderType::TransitionWaitingHiddenWaitingUpright
        );
        assert_eq!(t.result_posture, Posture::Upright);
    }

    #[test]
    fn near_detection_radius_is_15_squared() {
        assert_eq!(SQR_NEAR_DETECTION_RADIUS, 15.0 * 15.0);
    }

    #[test]
    fn move_requires_upright() {
        assert!(command_requires_upright(Command::Move));
        assert!(command_requires_upright(Command::ShootBow));
        assert!(command_requires_upright(Command::StrangleCmd));
        assert!(command_requires_upright(Command::PassDoor));
    }

    #[test]
    fn stealth_commands_dont_require_upright() {
        assert!(!command_requires_upright(Command::CrouchDown));
        assert!(!command_requires_upright(Command::CrouchUp));
        assert!(!command_requires_upright(Command::LeaveBeggar));
        assert!(!command_requires_upright(Command::Null));
    }

    #[test]
    fn eye_z_upright_vs_crouched() {
        // Upright-group eye height = +45 (foot), +60 for riders.
        assert_eq!(eye_z_for_posture(Posture::Upright, false), 45.0);
        assert_eq!(eye_z_for_posture(Posture::Upright, true), 60.0);
        assert_eq!(eye_z_for_posture(Posture::Crouched, false), 25.0);
        assert_eq!(eye_z_for_posture(Posture::Tree, false), 25.0);
        assert_eq!(eye_z_for_posture(Posture::SimulatingBeggar, false), 25.0);
        assert_eq!(eye_z_for_posture(Posture::OnShoulders, false), 85.0);
        // Lying uses +5 (eye) vs +2 (detection).
        assert_eq!(eye_z_for_posture(Posture::Lying, false), 5.0);
        assert_eq!(detection_z_for_posture(Posture::Lying, false), 2.0);
    }

    #[test]
    fn detection_point_xy_leaning_out_shifts_forward() {
        use crate::geo2d::pt;
        // Sector 4 (= +X, east).  direction_vector_16 returns
        // (sin(τ·4/16), -cos(τ·4/16)) = (1, 0).
        let ground = pt(100.0, 200.0);
        let shifted = detection_point_xy(ground, Posture::LeaningOut, 4);
        assert!((shifted.x - 140.0).abs() < 1e-3);
        assert!((shifted.y - 200.0).abs() < 1e-3);
        // Non-LeaningOut postures pass through.
        let same = detection_point_xy(ground, Posture::Upright, 4);
        assert_eq!(same.x, ground.x);
        assert_eq!(same.y, ground.y);
    }

    #[test]
    fn eye_point_xy_applies_crawling_offsets() {
        use crate::geo2d::pt;
        let ground = pt(100.0, 200.0);
        let shifted = eye_point_xy(ground, Posture::Lying, 4, false);
        assert!((shifted.x - 116.0).abs() < 1e-3);
        assert!((shifted.y - 200.0).abs() < 1e-3);

        let emergency = eye_point_xy(ground, Posture::Lying, 4, true);
        assert!((emergency.x - 108.0).abs() < 1e-3);
        assert!((emergency.y - 200.0).abs() < 1e-3);

        let same = eye_point_xy(ground, Posture::Upright, 4, false);
        assert_eq!(same.x, ground.x);
        assert_eq!(same.y, ground.y);
    }

    #[test]
    fn crouched_damage_anim_variants() {
        assert!(use_crouched_damage_anims(Posture::Crouched));
        assert!(use_crouched_damage_anims(Posture::SimulatingBeggar));
        assert!(use_crouched_damage_anims(Posture::Tree));
        assert!(!use_crouched_damage_anims(Posture::Upright));
        assert!(!use_crouched_damage_anims(Posture::Spy));
    }
}
