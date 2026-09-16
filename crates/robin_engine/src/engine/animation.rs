//! Animation ticking.

use super::*;
use crate::element::{ActionState, Command, Entity, EyeStatus, Posture};
use crate::sprite::{FrameProgression, MotionState};

const WEAKNESS_DISMISH: u16 = 5;

/// Whether the opt-in direction-latch diagnostic admits this actor/frame.
///
/// The master switch deliberately requires both an owner and either an exact
/// frame or a bounded frame range. This keeps an accidentally enabled release
/// runner from printing every actor's `Turn()` calls.
fn turn_provenance_matches(frame: u32, owner: EntityId) -> bool {
    super::diagnostics::config().turn_provenance_matches(frame, owner)
}

/// Emit one direction-latch boundary for the opt-in Turn provenance probe.
///
/// This deliberately shares the existing strict owner/frame gate: the PC181
/// investigation needs to distinguish a genuine `Turn` from direct
/// direction writers and from owner-envelope work, without adding any
/// reads of sprite state when the diagnostic is disabled.
pub(super) fn direction_provenance_snapshot(
    position: &crate::position_interface::PositionInterface,
    owner: EntityId,
    frame: u32,
    call_class: &'static str,
) {
    if !turn_provenance_matches(frame, owner) {
        return;
    }

    let state = position.parity_turn_provenance_state();
    let map = position.map_position();
    let old_map = position.old_map_position();
    let increment = position
        .is_increment_map_computed()
        .then(|| position.get_increment_map());
    eprintln!(
        "DIRPROV frame={frame} owner={owner:?} class={call_class} deviated={} count={} dir={} goal={} map_x={:08x} map_y={:08x} old_x={:08x} old_y={:08x} increment={increment:?}",
        state.0,
        state.1,
        state.2,
        state.3,
        map.x.to_bits(),
        map.y.to_bits(),
        old_map.x.to_bits(),
        old_map.y.to_bits(),
    );
}

/// Execute one actor-owned Turn call and, when explicitly selected, expose
/// the serialized anti-vibration latch on both sides of that exact call.
fn turn_with_provenance(
    entity: &mut Entity,
    owner: EntityId,
    frame: u32,
    call_class: &'static str,
    fast: bool,
) -> bool {
    if !turn_provenance_matches(frame, owner) {
        return if fast {
            entity.position_iface_mut().turn_fast()
        } else {
            entity.position_iface_mut().turn()
        };
    }

    let before = entity.position_iface().parity_turn_provenance_state();
    let result = if fast {
        entity.position_iface_mut().turn_fast()
    } else {
        entity.position_iface_mut().turn()
    };
    let after = entity.position_iface().parity_turn_provenance_state();
    eprintln!(
        "TURNPROV frame={frame} owner={owner:?} class={call_class} fast={fast} result={result} before_deviated={} before_count={} before_dir={} before_goal={} after_deviated={} after_count={} after_dir={} after_goal={}",
        before.0, before.1, before.2, before.3, after.0, after.1, after.2, after.3,
    );
    result
}

/// Return the "alerted" variant of an animation order type when a
/// soldier is attentive.  Each soldier-specific animation handler
/// substitutes the alerted variant at the top of its "attentive"
/// branch before delegating to action/motion playback.  Returns
/// `None` for animation types that have no alerted variant.
pub(super) fn alerted_variant(anim: OrderType) -> Option<OrderType> {
    use OrderType as OT;
    match anim {
        OT::Turning => Some(OT::TurningAlerted),
        OT::WaitingUpright => Some(OT::WaitingAlerted),
        OT::WalkingUpright => Some(OT::WalkingAlerted),
        OT::WalkingStairs => Some(OT::WalkingStairsAlerted),
        OT::LookingLeft => Some(OT::LookingLeftAlerted),
        OT::LookingRight => Some(OT::LookingRightAlerted),
        OT::TransitionWalkingUprightWaitingUpright => {
            Some(OT::TransitionWalkingAlertedWaitingAlerted)
        }
        OT::TransitionRunningUprightWaitingUpright => {
            Some(OT::TransitionRunningAlertedWaitingAlerted)
        }
        OT::TransitionWaitingUprightWalkingUpright => {
            Some(OT::TransitionWaitingAlertedWalkingAlerted)
        }
        OT::TransitionWaitingUprightRunningUpright => {
            Some(OT::TransitionWaitingAlertedRunningAlerted)
        }
        OT::TransitionWalkingUprightRunningUpright => {
            Some(OT::TransitionWalkingAlertedRunningAlerted)
        }
        OT::TransitionRunningUprightWalkingUpright => {
            Some(OT::TransitionRunningAlertedWalkingAlerted)
        }
        _ => None,
    }
}

/// Resolve the concrete sprite animation used by the movement portion of
/// soldier execution while leaving the authored order intact.
///
/// The Original's guards are deliberately branch-specific: upright movement
/// transitions and ordinary walking only test current attentiveness, while
/// stairs/turning exclude the complete sword-action family. The two sword
/// checks in the ordinary-walking branch are assertions, not release-build
/// control flow.
pub(super) fn soldier_movement_animation(
    anim: OrderType,
    attentive: bool,
    action_state: ActionState,
) -> OrderType {
    use OrderType as OT;

    if !attentive {
        return anim;
    }

    let all_sword_states = matches!(
        action_state,
        ActionState::WaitingSword
            | ActionState::MovingSword
            | ActionState::MovingFastSword
            | ActionState::ParryingSword
            | ActionState::ParryingSwordLow
    );

    match anim {
        OT::TransitionWalkingUprightWaitingUpright
        | OT::TransitionRunningUprightWaitingUpright
        | OT::TransitionWaitingUprightWalkingUpright
        | OT::TransitionWaitingUprightRunningUpright
        | OT::TransitionWalkingUprightRunningUpright
        | OT::TransitionRunningUprightWalkingUpright => alerted_variant(anim).unwrap_or(anim),
        OT::WalkingUpright => OT::WalkingAlerted,
        OT::WalkingStairs | OT::Turning if !all_sword_states => {
            alerted_variant(anim).unwrap_or(anim)
        }
        _ => anim,
    }
}

fn raising_sword_direction(owner: &Entity, opponent: &Entity) -> i16 {
    let (dx, dy) = if matches!(owner, Entity::Soldier(_)) {
        let from = owner.element_data().position_map();
        let to = opponent.element_data().position_map();
        (to.x - from.x, to.y - from.y)
    } else {
        let from = owner.element_data().position();
        let to = opponent.element_data().position();
        (to.x - from.x, to.y - from.y)
    };
    crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy)
}

fn striking_down_sword_direction(owner: &Entity, antagonist: &Entity) -> i16 {
    let from = owner.ground_position();
    let to = antagonist.ground_position();
    crate::position_interface::vector_to_sector_0_to_15_iso(to.x - from.x, to.y - from.y)
}

fn taking_initial_direction(
    owner_is_pc: bool,
    order_is_initialising: bool,
    anim_type: OrderType,
    from: crate::coordinates::MapPoint,
    to: crate::coordinates::MapPoint,
) -> Option<i16> {
    if !order_is_initialising {
        return None;
    }
    match (owner_is_pc, anim_type) {
        (true, OrderType::Taking | OrderType::TakingCrouched) => Some(
            crate::position_interface::vector_to_sector_0_to_15(to.x - from.x, to.y - from.y),
        ),
        (false, OrderType::Taking) => Some(
            crate::position_interface::vector_to_sector_0_to_15_iso(to.x - from.x, to.y - from.y),
        ),
        _ => None,
    }
}

/// Beggar animation arms whose PC execution turns the actor
/// before advancing the sprite action.
fn pc_beggar_execute_calls_turn(anim: OrderType) -> bool {
    matches!(
        anim,
        OrderType::TransitionWaitingUprightSimulatingBeggar
            | OrderType::TransitionSimulatingBeggarWaitingUpright
            | OrderType::SimulatingBeggar
    )
}

/// Extra per-class conditions guarding a turning animation arm.
///
/// A few arms exist in more than one actor subclass and the overrides disagree
/// about whether they rotate:
///
/// * `TransitionRaisingSword` — the human (and, through its fall-through, the
///   PC) arm turns unconditionally, while the soldier override turns only when
///   the order carries an antagonist.
/// * `ExtractingArrowSword` — turns only while actually swordfighting.
///
/// Everything else in the turning set rotates unconditionally in every
/// subclass that defines it.
fn turn_arm_condition_holds(entity: &Entity, anim: OrderType, has_antagonist: bool) -> bool {
    let is_swordfighting = entity
        .human_data()
        .is_some_and(|human| !human.opponents.is_empty());
    match anim {
        OrderType::TransitionRaisingSword => !entity.is_soldier() || has_antagonist,
        OrderType::ExtractingArrowSword => is_swordfighting,
        _ => true,
    }
}

/// Select the direction row stamped by an actor action that also turns.
///
/// Two arms call the sprite *before* they rotate, so their visible row belongs
/// to the direction at Execute entry even though the position interface has
/// advanced by the end of the same simulation frame: the attentive-soldier
/// `Turning` arm, which plays `TurningAlerted` ahead of its rotation step, and
/// the human `StandingUpSword` arm, which re-aims and turns only after its
/// sprite action.
fn actor_action_row(
    anim_type: OrderType,
    effective_anim: OrderType,
    direction_before_turn: u16,
    direction_after_turn: u16,
) -> u16 {
    if (anim_type == OrderType::Turning && effective_anim == OrderType::TurningAlerted)
        || anim_type == OrderType::StandingUpSword
    {
        direction_before_turn
    } else {
        direction_after_turn
    }
}

fn is_custom_animation_order(order_type: OrderType) -> bool {
    matches!(
        order_type,
        OrderType::PlayCustom
            | OrderType::PlayCustomLooped
            | OrderType::PlayCustomFreeze
            | OrderType::PlayCustomFrozen
    )
}

fn play_anim_freeze_completed(
    motion_state: MotionState,
    command: Option<Command>,
    selected_order: OrderType,
) -> bool {
    // The original game launches frozen animation playback only from the
    // custom frozen-animation execution arm. The sequence keeps its
    // PlayAnimFreeze command while prerequisite posture/action transitions
    // run, and those transitions can terminate too.
    motion_state == MotionState::Terminated
        && command == Some(Command::PlayAnimFreeze)
        && selected_order == OrderType::PlayCustomFreeze
}

/// Complete the human `STANDING_UP_SWORD` arm after sprite playback.
///
/// Original refreshes the goal from the live principal opponent only while
/// swordfighting, but calls `Turn()` unconditionally. Soldiers override this
/// arm and only replay the sprite, so they must not enter this helper.
fn apply_standing_up_sword_post_perform_facing(
    entity: &mut Entity,
    principal_direction: Option<i16>,
    owner: EntityId,
    frame: u32,
) {
    if entity.is_soldier() {
        return;
    }
    if let Some(direction) = principal_direction {
        entity.element_data_mut().set_direction_goal(direction);
    }
    let _ = turn_with_provenance(
        entity,
        owner,
        frame,
        "standing_up_sword_post_perform",
        false,
    );
}

#[cfg(test)]
#[path = "animation/tests.rs"]
mod tests;

// Phase methods of `tick_actor_animation_for`. The file lives next to
// `animation.rs` (not in `animation/`, which holds only tests), but is a child
// module so it can use the animation-private helpers without widening their
// visibility.
#[path = "animation_step.rs"]
mod animation_step;

/// Whether a soldier is "attentive" for sprite-row purposes.  Reads
/// the soldier's attentive flag, which the completion handler for
/// `TransitionWaitingUprightWaitingAlerted` flips on.  Sword-pose
/// action states suppress the alerted variant to match the explicit
/// gating in the per-case soldier handlers.
///
/// Historic note: this used to also fall back to a
/// `ai.current_state == Attacking` proxy as a workaround for the
/// pre-port bug where the alerted transition never fired and the
/// port "snapped straight into AttackingReactiontime" with the flag
/// stuck at `false`.  The transition now fires correctly via
/// `set_soldier_attentive_mode` → sequence-phase attention dispatch, so
/// the proxy is obsolete — and harmful, because it made
/// sprite-substitution happen one frame *before* the transition
/// animation started (the AI state flips synchronously inside
/// `set_state(Attacking, …)`, a frame before the attention context
/// queues the animation).  That one-frame lead produced a visible
/// pop: `WaitingUprightBored` → `TurningAlerted` (1 frame of
/// "sword-drawn" pose) → `TransitionWaitingUprightBoredWaitingUpright`
/// → the real lean-forward transition.
pub(super) fn soldier_is_attentive(entity: &Entity) -> bool {
    if !matches!(entity, Entity::Soldier(_)) {
        return false;
    }
    let attentive_flag = entity.enemy_ai().map(|e| e.attentive).unwrap_or(false);
    let action_state = entity
        .actor_data()
        .map(|a| a.action_state)
        .unwrap_or(ActionState::Waiting);
    let not_sword = !matches!(
        action_state,
        ActionState::WaitingSword
            | ActionState::MovingSword
            | ActionState::MovingFastSword
            | ActionState::ParryingSword
            | ActionState::ParryingSwordLow
    );
    attentive_flag && not_sword
}

#[derive(Debug, Clone, Copy)]
pub(super) struct TakingNetTick {
    pub taker: EntityId,
    pub net: EntityId,
    pub action_done: bool,
    pub order_was_done: bool,
}

fn forwards_pc_bow_action_on_start(
    entity: &Entity,
    anim_type: OrderType,
    motion: MotionState,
    script_driven: bool,
) -> bool {
    entity.is_pc()
        && !script_driven
        && motion == MotionState::Start
        && matches!(
            anim_type,
            OrderType::TransitionEquipBow | OrderType::TransitionEquipBowAnonymous
        )
}

/// Apply the soldier-specific side effects triggered on each motion-state
/// transition (Start, Done, Terminated) of an `active_ai_anim`
/// animation.  Covers the 42 animation cases: action-state/posture
/// transitions, attentive-flag toggling, view-status updates for
/// LOOKING_* anims, sleep/leaning-out/bow-lean transitions,
/// DRINKING_ALE / TAKING / SPECIAL / GETTING_FREE_FROM_WASP
/// antagonist-dependent effects.
///
/// Executes owner and antagonist changes before the selected animation arm returns.
fn apply_soldier_execute_side_effects(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
    entity_id: EntityId,
) {
    let entity = engine
        .world
        .entities
        .get_mut(entity_id)
        .expect("animation owner disappeared");
    use crate::order::OrderType as OT;
    use crate::sprite::MotionState as MS;

    if !matches!(entity, Entity::Soldier(_)) {
        return;
    }

    let set_states = |e: &mut Entity, posture: Posture, action: ActionState| {
        e.set_posture(posture);
        if let Some(a) = e.actor_data_mut() {
            a.action_state = action;
        }
    };

    match (anim_type, motion) {
        // TRANSITION_WAITING_UPRIGHT_WAITING_ALERTED: attentive = true
        (OT::TransitionWaitingUprightWaitingAlerted, MS::Done | MS::Terminated) => {
            if let Some(e) = entity.enemy_ai_mut() {
                e.attentive = true;
            }
        }
        // TRANSITION_WAITING_ALERTED_WAITING_UPRIGHT (+officer variant):
        // attentive = false
        (
            OT::TransitionWaitingAlertedWaitingUpright
            | OT::TransitionWaitingAlertedWaitingUprightOfficer,
            MS::Done | MS::Terminated,
        ) => {
            if let Some(e) = entity.enemy_ai_mut() {
                e.attentive = false;
            }
        }

        // Movement-transition end states.
        // NB the WAITING_UPRIGHT → WALKING_UPRIGHT transition sets the
        // *end* state to Waiting — the actual walk is launched by the
        // next order in the sequence.
        (
            OT::TransitionWalkingUprightWaitingUpright
            | OT::TransitionRunningUprightWaitingUpright
            | OT::TransitionWaitingUprightWalkingUpright
            | OT::TransitionWalkingAlertedWaitingAlerted
            | OT::TransitionRunningAlertedWaitingAlerted
            | OT::TransitionWaitingAlertedWalkingAlerted,
            MS::Done | MS::Terminated,
        ) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // Walking / running animation Start: flip `action_state` to
        // Moving / MovingFast when the walk order becomes the actor's
        // current order.  Without this, a soldier coming out of a
        // WAITING_UPRIGHT_WALKING_UPRIGHT startup transition ends in
        // `Waiting` (per the handler above), the walk order becomes
        // current, but `tick_entity_movement`'s `is_moving()` gate
        // never flips → actor walks-in-place forever.
        // Walking/running Start handlers moved to
        // `apply_npc_execute_side_effects` so civilians get them too
        // (the NPC dispatch covers both soldier and civilian
        // walking/running).
        (
            OT::TransitionWaitingUprightRunningUpright
            | OT::TransitionWalkingUprightRunningUpright
            | OT::TransitionWaitingAlertedRunningAlerted
            | OT::TransitionWalkingAlertedRunningAlerted,
            MS::Done | MS::Terminated,
        ) => {
            set_states(entity, Posture::Upright, ActionState::MovingFast);
        }
        (
            OT::TransitionRunningUprightWalkingUpright | OT::TransitionRunningAlertedWalkingAlerted,
            MS::Done | MS::Terminated,
        ) => {
            set_states(entity, Posture::Upright, ActionState::Moving);
        }

        // BEING_UNCONSCIOUS_* START is handled for every human by
        // `apply_active_animation_start_state_side_effect`.

        // TRANSITION_RAISING_SWORD → WaitingSword on DONE
        (OT::TransitionRaisingSword, MS::Done) => {
            set_states(entity, Posture::Upright, ActionState::WaitingSword);
        }

        // TRANSITION_CHARGING → WaitingSword on DONE (damage resolved
        // separately in melee module).
        (OT::TransitionCharging, MS::Done) => {
            set_states(entity, Posture::Upright, ActionState::WaitingSword);
        }

        // TAKING: TERMINATED → Waiting
        (OT::Taking, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // TRANSITION_WAITING_SWORD_MENACING: TERMINATED → Menacing
        (OT::TransitionWaitingSwordMenacing, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Menacing);
        }

        // SLEEPING_UPRIGHT: every tick → (Upright, Sleeping)
        (OT::SleepingUpright, _) => {
            set_states(entity, Posture::Upright, ActionState::Sleeping);
        }

        // TRANSITION_SLEEPING_WAITING_UPRIGHT: TERMINATED → Waiting
        (OT::TransitionSleepingWaitingUpright, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // TRANSITION_MENACING_WAITING_SWORD: TERMINATED → WaitingSword
        (OT::TransitionMenacingWaitingSword, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::WaitingSword);
        }

        // LEANING_OUT: START → (LeaningOut, Waiting)
        (OT::LeaningOut, MS::Start) => {
            set_states(entity, Posture::LeaningOut, ActionState::Waiting);
        }

        // TRANSITION_WAITING_ALERTED_LEANING_OUT: DONE → (LeaningOut, Waiting)
        (OT::TransitionWaitingAlertedLeaningOut, MS::Done) => {
            set_states(entity, Posture::LeaningOut, ActionState::Waiting);
        }

        // TRANSITION_LEANING_OUT_WAITING_ALERTED: DONE → (Upright, Waiting)
        (OT::TransitionLeaningOutWaitingAlerted, MS::Done) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // TRANSITION_LOWERING_BOW_LEANING_OUT: DONE/TERMINATED →
        // (LeaningOut, AimingWithBowDown).
        (OT::TransitionLoweringBowLeaningOut, MS::Done | MS::Terminated) => {
            set_states(entity, Posture::LeaningOut, ActionState::AimingWithBowDown);
        }

        // TRANSITION_RAISING_BOW_LEANING_OUT: DONE/TERMINATED →
        // (Upright, AimingWithBow).
        (OT::TransitionRaisingBowLeaningOut, MS::Done | MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::AimingWithBow);
        }

        // SHOOTING_WITH_BOW_LEANING_OUT: DONE → (LeaningOut, AimingWithBow)
        // The actual arrow release side effect is handled in the
        // `bow_shot` module.
        (OT::ShootingWithBowLeaningOut, MS::Done) => {
            set_states(entity, Posture::LeaningOut, ActionState::AimingWithBow);
        }

        // LOOKING_LEFT / LOOKING_LEFT_ALERTED: START → LookToTheLeft,
        // DONE → LookForward.
        (OT::LookingLeft | OT::LookingLeftAlerted, MS::Start) => {
            if let Some(npc) = entity.npc_data_mut() {
                crate::ai_vision::set_view_status(npc, EyeStatus::LookToTheLeft);
            }
        }
        (OT::LookingLeft | OT::LookingLeftAlerted, MS::Done) => {
            if let Some(npc) = entity.npc_data_mut() {
                crate::ai_vision::set_view_status(npc, EyeStatus::LookForward);
            }
        }
        (OT::LookingRight | OT::LookingRightAlerted, MS::Start) => {
            if let Some(npc) = entity.npc_data_mut() {
                crate::ai_vision::set_view_status(npc, EyeStatus::LookToTheRight);
            }
        }
        (OT::LookingRight | OT::LookingRightAlerted, MS::Done) => {
            if let Some(npc) = entity.npc_data_mut() {
                crate::ai_vision::set_view_status(npc, EyeStatus::LookForward);
            }
        }

        // DRINKING_ALE:
        //   START: set states to (Upright, Waiting).
        //   DONE:  deactivate the antagonist (hide the bottle).
        //   TERMINATED: blood_alcohol += profile.beer.
        (OT::DrinkingAle, MS::Start) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }
        (OT::DrinkingAle, MS::Done) => {
            if let Some(a) = antagonist {
                engine.execute_deactivate_entities(a);
            }
        }
        (OT::DrinkingAle, MS::Terminated) => {
            engine.execute_drink_done(assets, (entity_id, antagonist));
        }

        // TAKING DONE: pick up the antagonist (Purse or Coin) and add
        // its value to the soldier's money.  TERMINATED switches back
        // to (Upright, Waiting) — already handled above (the match arm
        // for `(OT::Taking, MS::Terminated)` fires before this).
        (OT::Taking, MS::Done) => {
            if let Some(a) = antagonist {
                engine.execute_pickups(sim, assets, (entity_id, a));
            }
        }

        // GETTING_FREE_FROM_WASP: on initialisation sets a random
        // rotation offset and says REMARK_WASP_STING; while still
        // turning, plays TURNING_ALERTED instead of the configured
        // animation.  The random-rotation setup is booking-site
        // business (must happen before `active_ai_anim` is set so the
        // target direction is correct); here we just fire the remark
        // once the animation actually starts.
        (OT::GettingFreeFromWasp, MS::Start) => {
            engine.execute_wasp_sting_remark(sim, assets, entity_id);
        }
        // NB: `wasp_victim = false` is not handled here.  The reset
        // lives in `EngineInner::send_condolation_card`
        // (engine/soldier_helpers.rs), fired by the general
        // sequence-terminated queue when the `ReceiveWaspSting`
        // element finishes.

        // MENACING, WAITING_ALERTED, GATHERING_SOLDIERS,
        // AIMING_WITH_BOW_LEANING_OUT: these cases just play the
        // action and return — no state side effects other than what
        // perform_action already does.
        //
        // TRANSITION_CHARGING DONE damage is applied in `melee.rs`
        // where the charging is booked; we only handle the
        // (Upright, WaitingSword) state change here.
        _ => {}
    }
}

fn special_remark_due_at_sprite_phase(
    speech_id: u32,
    current_frame: u16,
    frame_count: u16,
) -> bool {
    const SPEECH_ID_HELBARDMAN: u32 = 0x4c484453;
    const EAT_FRAMES_HELBARDMAN: u16 = 40;
    frame_count == 0
        && if speech_id == SPEECH_ID_HELBARDMAN {
            current_frame == EAT_FRAMES_HELBARDMAN
        } else {
            current_frame == 0
        }
}

/// Walk/run animation Start → flip `action_state` to the matching
/// moving variant.  Fires at the start of each walking order's
/// sprite playback for all variants (running, sword, alerted,
/// crouched).
///
/// Applies to **all actor kinds** (PC, soldier, civilian).  Without
/// this, an actor coming out of a
/// `WAITING_UPRIGHT_WALKING_UPRIGHT` startup transition ends in
/// `Waiting` (per the transition-end handler above), the walk order
/// becomes current, but nothing flips `action_state` →
/// `tick_entity_movement`'s `is_moving()` gate never trips and the
/// actor walks-in-place forever.
pub(super) fn apply_actor_walk_start_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    use crate::order::OrderType as OT;
    use crate::sprite::MotionState as MS;

    if !matches!(motion, MS::Start) {
        return;
    }
    if entity.actor_data().is_none() {
        return;
    }

    let set_states = |e: &mut Entity, posture: Posture, action: ActionState| {
        e.set_posture(posture);
        if let Some(a) = e.actor_data_mut() {
            a.action_state = action;
        }
    };

    match anim_type {
        OT::WalkingUpright | OT::WalkingAlerted => {
            set_states(entity, Posture::Upright, ActionState::Moving);
        }
        // The crouched walk keeps the actor crouched; only the action
        // state moves on. Sharing the upright arm made every crouched
        // walk order stand the actor up on its first frame.
        OT::WalkingCrouched => {
            set_states(entity, Posture::Crouched, ActionState::Moving);
        }
        OT::RunningUpright => {
            set_states(entity, Posture::Upright, ActionState::MovingFast);
        }
        OT::WalkingWithSword => {
            set_states(entity, Posture::Upright, ActionState::MovingSword);
        }
        OT::RunningWithSword => {
            set_states(entity, Posture::Upright, ActionState::MovingFastSword);
        }
        _ => {}
    }
}

/// Apply the per-anim-type side effects from the NPC parent handler
/// — the cases that fall through from the soldier switch into the
/// shared NPC dispatch, plus the civilian-side handlers that reuse
/// them.  Covers SITTING / POINTING / SEARCHING /
/// TRANSITION_SITTING_WAITING_UPRIGHT / TRANSITION_WAITING_UPRIGHT_SITTING /
/// BEGGAR_SHOWING_FACE.
///
/// Applied to any NPC (soldier or civilian) with an
/// `active_ai_anim` — the actual animation is played by the sprite
/// system; this function just runs the post-motion-state dispatch
/// (state transitions, pickpocket money transfer).  Called after
/// `apply_soldier_execute_side_effects` so soldier-specific
/// overrides still take priority.
pub(super) fn apply_npc_execute_side_effects(
    engine: &mut EngineInner,
    assets: &LevelAssets,
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
    entity_id: EntityId,
) {
    let entity = engine
        .world
        .entities
        .get_mut(entity_id)
        .expect("animation owner disappeared");
    use crate::order::OrderType as OT;
    use crate::sprite::MotionState as MS;

    // Only NPCs (soldiers, civilians) get the NPC dispatch treatment.
    if !matches!(entity, Entity::Soldier(_) | Entity::Civilian(_)) {
        return;
    }

    let set_states = |e: &mut Entity, posture: Posture, action: ActionState| {
        e.set_posture(posture);
        if let Some(a) = e.actor_data_mut() {
            a.action_state = action;
        }
    };

    match (anim_type, motion) {
        // SITTING: START → (Sitting, Waiting).
        (OT::Sitting, MS::Start) => {
            set_states(entity, Posture::Sitting, ActionState::Waiting);
        }

        // TRANSITION_SITTING_WAITING_UPRIGHT: TERMINATED → (Upright, Waiting).
        (OT::TransitionSittingWaitingUpright, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // TRANSITION_WAITING_UPRIGHT_SITTING: TERMINATED → (Sitting, Waiting).
        (OT::TransitionWaitingUprightSitting, MS::Terminated) => {
            set_states(entity, Posture::Sitting, ActionState::Waiting);
        }

        // TRANSITION_WAITING_UPRIGHT_SPECIAL (ENTER_LEISURE):
        // DONE/TERMINATED → (Leisure, Waiting).  Both motion states
        // flip posture so the actor stays in Leisure once the
        // transition animation finishes (DONE) or is interrupted
        // (TERMINATED).
        (OT::TransitionWaitingUprightSpecial, MS::Done | MS::Terminated) => {
            set_states(entity, Posture::Leisure, ActionState::Waiting);
        }

        // TRANSITION_SPECIAL_WAITING_UPRIGHT (leave-leisure):
        // DONE/TERMINATED → (Upright, Waiting).
        (OT::TransitionSpecialWaitingUpright, MS::Done | MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // BEGGAR_SHOWING_FACE: TERMINATED → (Upright, Waiting).
        // The sprite-animation selection above falls back to Rolling when
        // the beggar lacks the showing-face animation; dispatch remains
        // keyed on this original order type, matching the Original.
        (OT::BeggarShowingFace, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // POINTING: TERMINATED → (Upright, Waiting).
        // The booking site already sets the direction field; we only
        // restore the idle state when the point gesture finishes.
        (OT::Pointing, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // Base actor idle-state changes are applied by
        // `apply_active_animation_start_state_side_effect`. The NPC override
        // only adds the officer eye-status behavior here.
        (OT::WaitingUprightBoredRandom, MS::Start) => {
            // Officer-only: LookToTheRight on START, LookForward on DONE.
            if entity
                .enemy_ai()
                .map(|e| {
                    e.profile(&assets.profile_manager).rank == crate::profiles::ProfileRank::Officer
                })
                .unwrap_or(false)
                && let Some(npc) = entity.npc_data_mut()
            {
                crate::ai_vision::set_view_status(npc, EyeStatus::LookToTheRight);
            }
        }
        (OT::WaitingUprightBoredRandom, MS::Done) => {
            if entity
                .enemy_ai()
                .map(|e| {
                    e.profile(&assets.profile_manager).rank == crate::profiles::ProfileRank::Officer
                })
                .unwrap_or(false)
                && let Some(npc) = entity.npc_data_mut()
            {
                crate::ai_vision::set_view_status(npc, EyeStatus::LookForward);
            }
        }

        // SEARCHING: DONE → (Upright, Waiting) + NPC-on-NPC pickpocket
        // money transfer (thief gains the victim's money and the
        // victim is zeroed out).  The state change fires on DONE
        // (before the switch advances) rather than TERMINATED.
        (OT::Searching, MS::Done) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
            if let Some(victim) = antagonist {
                engine.execute_pickpockets((entity_id, victim));
            }
        }

        _ => {}
    }
}

/// PC target interactions are two-stage: the PC first plays the
/// visible action animation, then the target receives the activation
/// command on motion Done.
pub(super) fn apply_pc_target_interaction_side_effect(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
    entity_id: EntityId,
) {
    let entity = engine
        .world
        .entities
        .get_mut(entity_id)
        .expect("animation owner disappeared");
    if !matches!(entity, Entity::Pc(_)) || !matches!(motion, MotionState::Done) {
        return;
    }
    let Some(target) = antagonist else {
        return;
    };
    let activation = match anim_type {
        OrderType::HittingTarget => Command::ActivateSword,
        OrderType::HandlingTarget | OrderType::TakingTarget => Command::ActivateHandle,
        OrderType::UsingLever => Command::ActivateLever,
        OrderType::Searching => Command::ActivateSearch,
        _ => return,
    };
    engine.execute_pc_target_activations(sim, assets, (entity_id, target, activation));
}

/// Stage the exact post-sprite `TakingNet` tail. The original does not remove
/// the net at DONE: it changes the net's own animation, pulls it toward the
/// taker's live action point for eight ticks, then removes it on the following
/// owner slot.
fn apply_taking_net_side_effect(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
    entity_id: EntityId,
    order_was_done: bool,
) {
    if anim_type == OrderType::TakingNet
        && (motion == MotionState::Done || order_was_done)
        && let Some(net) = antagonist
    {
        engine.execute_taking_net_ticks(
            sim,
            assets,
            TakingNetTick {
                taker: entity_id,
                net,
                action_done: motion == MotionState::Done,
                order_was_done,
            },
        );
    }
}

fn apply_waking_up_done_side_effect(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
    entity_id: EntityId,
) {
    if matches!(anim_type, OrderType::WakingUp)
        && matches!(motion, MotionState::Done)
        && let Some(target) = antagonist
    {
        engine.execute_waking_up_done(sim, assets, (entity_id, target));
    }
}

/// Capture the action-state view that Original exposes to the synchronous
/// `EVENT_ADVERSARY_WEAK` callbacks at the start of weak/stunned sword work.
/// The callback precedes action processing, whose initial edge can change the
/// actor to `WaitingSword` later in the same Execute call.
fn weak_stunned_start_action_before_perform(
    entity: &Entity,
    anim_type: OrderType,
    order_is_initialising: bool,
) -> Option<ActionState> {
    (order_is_initialising
        && matches!(
            anim_type,
            OrderType::BeingWeakSword | OrderType::BeingStunnedSword
        ))
    .then(|| {
        entity
            .actor_data()
            .unwrap_or_else(|| {
                panic!("weak/stunned initialization owner is not an actor: {anim_type:?}")
            })
            .action_state
    })
}

/// Universal active-animation START state changes shared by PCs and
/// NPCs. Mirrors the original game's motion-start state-change side
/// effects for active animation arms whose completion logic is handled
/// elsewhere.
fn apply_active_animation_start_state_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    // Civilian execution overrides this entire family, coerces
    // the sprite animation to WAITING_UPRIGHT_BORED, and returns directly.
    // None of the base actor's posture/action-state switch arms run.
    if entity.is_civilian()
        && matches!(
            anim_type,
            OrderType::WaitingUpright
                | OrderType::WaitingUprightBored
                | OrderType::WaitingUprightBoredRandom
                | OrderType::TransitionWaitingUprightBoredWaitingUpright
                | OrderType::TransitionWaitingUprightWaitingUprightBored
        )
    {
        return;
    }

    match (anim_type, motion) {
        // These cases are implemented by human-actor execution, not
        // the soldier override, so PCs must receive the same START state.
        // Keeping them in the soldier-only side-effect dispatcher left a PC
        // in MovingSword for the first BEING_HIT_SWORD frame.
        (
            OrderType::BeingHitSword
            | OrderType::ExtractingArrowSword
            | OrderType::BeingStunnedSword,
            MotionState::Start,
        ) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::WaitingSword;
            }
            return;
        }
        // The knock-out hold animations are owned by the human Execute
        // switch, so every human — PC, soldier, civilian — settles into
        // the lying pose on the first frame.  Leaving them soldier-only
        // let a knocked-out PC keep whatever action state it carried
        // into the blow (typically MovingSword or Bored).
        (OrderType::BeingUnconsciousSword, MotionState::Start) if entity.is_human() => {
            entity.set_posture(Posture::Lying);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::WaitingSword;
            }
            return;
        }
        (OrderType::BeingUnconsciousBow, MotionState::Start) if entity.is_human() => {
            entity.set_posture(Posture::Lying);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::AimingWithBow;
            }
            return;
        }
        (OrderType::BeingUnconscious, MotionState::Start) if entity.is_human() => {
            entity.set_posture(Posture::Lying);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        // Corpse-carry idle hold: only a PC ever executes it, and its
        // first frame settles the carrier back into Waiting.  Without
        // it a carrier that stops walking keeps the Moving state the
        // walk order stamped, and every later carry frame — including
        // the drop transition — inherits it.
        (OrderType::WaitingWithCorpse, MotionState::Start) if entity.is_pc() => {
            entity.set_posture(Posture::CarryingCorpse);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        // End of the lift animation: the carrier owns the body and is
        // standing still with it.
        (OrderType::TransitionWaitingUprightCarryingCorpse, MotionState::Done)
            if entity.is_pc() =>
        {
            entity.set_posture(Posture::CarryingCorpse);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (OrderType::Taking | OrderType::TakingCrouched, MotionState::Terminated)
            if entity.is_pc() =>
        {
            entity.set_posture(if anim_type == OrderType::Taking {
                Posture::Upright
            } else {
                Posture::Crouched
            });
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        // The crouched idle loop is executed only by the PC, and settles
        // the actor into Waiting on its first frame. Without this a PC
        // that stops crouch-walking keeps a stale Moving action state
        // into the following posture transition.
        (OrderType::WaitingCrouched, MotionState::Start) if entity.is_pc() => {
            entity.set_posture(Posture::Crouched);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        // Human actors own this START transition for players and other
        // non-soldier humans.  Soldiers override the arm and settle into
        // WaitingSword on DONE instead.
        (OrderType::TransitionRaisingSword, MotionState::Start)
            if !matches!(entity, Entity::Soldier(_)) =>
        {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::WaitingSword;
            }
            return;
        }
        (OrderType::TransitionLoweringSword, MotionState::Start) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (OrderType::WaitingUpright, MotionState::Start) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (
            OrderType::WaitingUprightBored | OrderType::WaitingUprightBoredRandom,
            MotionState::Start,
        ) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Bored;
            }
            return;
        }
        (
            OrderType::TransitionWaitingUprightBoredWaitingUpright,
            MotionState::Done | MotionState::Terminated,
        ) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (
            OrderType::TransitionWaitingUprightWaitingUprightBored,
            MotionState::Done | MotionState::Terminated,
        ) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Bored;
            }
            return;
        }
        (
            OrderType::TransitionWalkingUprightWaitingUpright
            | OrderType::TransitionRunningUprightWaitingUpright,
            MotionState::Done | MotionState::Terminated,
        ) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (
            OrderType::TransitionWalkingCrouchedWaitingCrouched,
            MotionState::Done | MotionState::Terminated,
        ) => {
            entity.set_posture(Posture::Crouched);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (OrderType::TransitionWaitingUprightHelpingClimbing, MotionState::Done) => {
            entity.set_posture(Posture::HelpingToClimb);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            if let Some(pc) = entity.pc_data_mut() {
                pc.current_action = crate::profiles::Action::HelpToClimb;
            }
            return;
        }
        (OrderType::TransitionHelpingClimbingWaitingUpright, MotionState::Done) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            if let Some(pc) = entity.pc_data_mut() {
                pc.current_action = crate::profiles::Action::NoAction;
            }
            return;
        }
        (OrderType::TransitionWaitingUprightSimulatingBeggar, MotionState::Done) => {
            entity.set_posture(Posture::SimulatingBeggar);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            if let Some(pc) = entity.pc_data_mut() {
                pc.current_action = crate::profiles::Action::Beggar;
            }
            return;
        }
        (OrderType::TransitionSimulatingBeggarWaitingUpright, MotionState::Done) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            // Player action execution does not clear the action directly here. An
            // unselected PC stores NoAction, while a selected PC forwards
            // MSG_UNSELECT_ACTION(BEGGAR); the messenger can reject that
            // message when another action has already replaced Beggar.
            // Keep the action handoff in `execute_beggar_wait_handoffs`, where
            // both selection and the live messenger action are available.
            return;
        }
        (OrderType::TransitionCrouchingUp, MotionState::Done | MotionState::Terminated) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (OrderType::TransitionCrouchingDown, MotionState::Done | MotionState::Terminated) => {
            entity.set_posture(Posture::Crouched);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (
            OrderType::TransitionEquipBow | OrderType::TransitionEquipBowAnonymous,
            MotionState::Start,
        ) => {
            if entity.element_data().posture() != Posture::AnonymousArcher {
                entity.set_posture(Posture::Upright);
            }
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::AimingWithBow;
            }
            return;
        }
        (
            OrderType::TransitionUnequipBow | OrderType::TransitionUnequipBowAnonymous,
            MotionState::Start,
        ) => {
            if entity.element_data().posture() != Posture::AnonymousArcher {
                entity.set_posture(Posture::Upright);
            }
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (
            OrderType::TransitionUnloadBow | OrderType::TransitionUnloadBowAnonymous,
            MotionState::Start,
        ) => {
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (
            OrderType::TransitionLoweringBow | OrderType::TransitionLoweringBowAnonymous,
            MotionState::Done | MotionState::Terminated,
        ) => {
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::AimingWithBow;
            }
            return;
        }
        (
            OrderType::TransitionRaisingBow | OrderType::TransitionRaisingBowAnonymous,
            MotionState::Done | MotionState::Terminated,
        ) => {
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::AimingWithBowUp;
            }
            return;
        }
        _ => {}
    }

    if !matches!(motion, MotionState::Start) {
        return;
    }
    let Some(action_state) = (match anim_type {
        OrderType::Provoking => Some(ActionState::WaitingSword),
        OrderType::WaitingShield => Some(ActionState::HoldingShield),
        OrderType::TakingNet => Some(ActionState::Waiting),
        _ => None,
    }) else {
        return;
    };

    entity.set_posture(Posture::Upright);
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = action_state;
    }
}

fn rejected_dead_idle_posture_callback_required(
    entity: &Entity,
    anim_type: OrderType,
    motion: MotionState,
) -> bool {
    entity.is_human()
        && motion == MotionState::Start
        && matches!(
            anim_type,
            OrderType::WaitingUpright
                | OrderType::WaitingUprightBored
                | OrderType::WaitingUprightBoredRandom
        )
        && matches!(
            entity.element_data().posture(),
            Posture::Dead | Posture::DeadBack
        )
}

/// PC `Taking` / `TakingCrouched` Done handler — fires when a PC finishes the generic
/// pickup animation for a scroll / bonus / landed projectile.
///
/// Dispatches per `ObjectType` (amulet, purse, coin, relics, scroll,
/// and the default ammo-bonus fallthrough).  The soldier counterpart
/// is handled by `apply_soldier_execute_side_effects`.
///
/// Object removal and pickup callbacks finish before this action returns.
fn apply_pc_taking_side_effect(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
    entity_id: EntityId,
) {
    let entity = engine
        .world
        .entities
        .get_mut(entity_id)
        .expect("animation owner disappeared");
    if matches!(entity, Entity::Pc(_))
        && matches!(anim_type, OrderType::Taking | OrderType::TakingCrouched)
        && matches!(motion, MotionState::Done)
        && let Some(a) = antagonist
    {
        engine.execute_pickups(sim, assets, (entity_id, a));
    }
}

/// Translate an order's `OrderType` (which can be a non-animation
/// dispatch token) into the sprite animation that should actually be
/// played.  Centralises the per-arm "non-animation X → animation Y"
/// mappings, plus the missing-shield-variant fallback cases.
///
/// The order's own `action` field (i.e. `anim_type` at the call site)
/// stays unchanged so side-effect handlers keep matching on the
/// original token — only the value handed to `sprite.perform_action`
/// is substituted.  The `action` field is the dispatch key, while
/// `perform_action` receives an explicit animation argument.
///
/// Source mappings:
///  - `FALLING_HIT_*` / `FALLING_HIT_HARDER_*` / `FALLING_PUSHED_*`
///    → `FALLING_BACK_*` (the falling-hit / falling-pushed dispatch
///    delegates to the `FALLING_BACK_*` sprite animation).
///  - `LOWERING_SHIELD` / `PARRYING_SHIELD` fall back to
///    `TRANSITION_LOWERING_SWORD` / `PARRYING_SWORD` when the sprite
///    lacks the shield variant.
///  - NPC `BEGGAR_SHOWING_FACE` falls back to `ROLLING` when the sprite
///    lacks the showing-face animation.
///  - PC target interactions fall back to compatible authored attacks when
///    an NPC sprite promoted to a playable character lacks the PC-only target
///    rows. The logical order stays unchanged so target activation still fires.
///  - `TRANSITION_WAITING_SWORD_PARRYING_SWORD_LOW`
///    → `TRANSITION_WAITING_SWORD_PARRYING_SWORD` — the `_LOW`
///    non-animation token re-uses the regular transition sprite anim.
///  - Soldier/NPC `TAKING` → `SEARCHING` (the order remains
///    `TAKING` so pickup side effects still dispatch there).
///  - `TAKING_NET` → `TAKING` when the actor profile has no dedicated row.
///  - `WAITING_CAPE_ANONYMOUS_ARCHER` → `WAITING_CAPE`.
///  - `FALLING_LADDER_WALL` → `FALLING_BACK_UPRIGHT`.  The ladder/wall
///    fall is a pure dispatch token with no sprite row of its own; only
///    the flight bookkeeping keys off the original action.
///
/// Identity for everything else (most order types play their own
/// sprite anim).
pub(crate) fn sprite_anim_for_order(
    sprite: &crate::sprite::Sprite,
    effective_anim: OrderType,
    owner_is_pc: bool,
) -> OrderType {
    use OrderType as OT;
    match effective_anim {
        OT::Taking if !owner_is_pc => OT::Searching,
        OT::TakingNet if !sprite.has_animation(OT::TakingNet) => OT::Taking,
        OT::HittingTarget if owner_is_pc && !sprite.has_animation(OT::HittingTarget) => {
            // TODO(mod-pc-animation-aliases): make these aliases authorable per
            // character profile when the mod format grows animation overrides.
            if sprite.has_animation(OT::StrikingDownSword) {
                OT::StrikingDownSword
            } else {
                OT::Hitting
            }
        }
        OT::HandlingTarget if owner_is_pc && !sprite.has_animation(OT::HandlingTarget) => {
            OT::Hitting
        }
        OT::LoweringShield if !sprite.has_animation(OT::LoweringShield) => {
            OT::TransitionLoweringSword
        }
        OT::ParryingShield if !sprite.has_animation(OT::ParryingShield) => OT::ParryingSword,
        OT::BeggarShowingFace if !sprite.has_animation(OT::BeggarShowingFace) => OT::Rolling,
        OT::FallingHitUpright
        | OT::FallingHitHarderUpright
        | OT::FallingPushedUpright
        | OT::FallingLadderWall => OT::FallingBackUpright,
        OT::FallingHitWithBow | OT::FallingHitHarderWithBow | OT::FallingPushedWithBow => {
            OT::FallingBackBow
        }
        OT::FallingHitWithSword | OT::FallingHitHarderWithSword | OT::FallingPushedWithSword => {
            OT::FallingBackSword
        }
        OT::FallingHitCrouched | OT::FallingHitHarderCrouched | OT::FallingPushedCrouched => {
            OT::FallingBackCrouched
        }
        OT::TransitionWaitingSwordParryingSwordLow => OT::TransitionWaitingSwordParryingSword,
        OT::WaitingCapeAnonymousArcher => OT::WaitingCape,
        other => other,
    }
}

fn default_actor_sprite_playback(
    sprite: &crate::sprite::Sprite,
    anim_type: OrderType,
    effective_anim: OrderType,
    owner_is_pc: bool,
) -> (OrderType, FrameProgression) {
    if matches!(anim_type, OrderType::LyingStuckUnderNet) {
        // LYING_STUCK_UNDER_NET is a logical hold, not an authored
        // animation. Original displays the frozen first frame of the
        // wriggle animation until the 1/31 struggle gate fires.
        (
            OrderType::WriggleUnderNet,
            FrameProgression::FrozenFirstFrame,
        )
    } else {
        (
            sprite_anim_for_order(sprite, effective_anim, owner_is_pc),
            FrameProgression::Default,
        )
    }
}

/// Anims whose initialisation lifts the parent sequence element to
/// `NonInterruptable`:
/// - `FALLING_LADDER_WALL`
/// - `ROLLING`
/// - ordinary (non-harder) `FALLING_HIT_*`
/// - `FALLING_PUSHED_*`
/// - `FALLING_SHOULDERS` (the priority is set when shoulder damage
///   is translated; we unify it into the start-side-effect set since
///   the runtime assertion would pass either way.)
///
/// These are the anims that unconditionally lift the parent element
/// to non-interruptable so a fresh damage can't preempt the in-flight
/// visual. Hit-induced falling deliberately does this only in its
/// non-harder arm; harder hits merely perform the action and retain the
/// damage element's ordinary `Injury` priority. Other fall families
/// (`FALLING_BACK_*`, `DYING_*`, `BEING_DEAD_*`) inherit their parent
/// element's priority instead, so they're not in this list.
fn anim_forces_non_interruptable_on_start(anim_type: OrderType) -> bool {
    matches!(
        anim_type,
        OrderType::FallingLadderWall
            | OrderType::Rolling
            | OrderType::FallingHitUpright
            | OrderType::FallingHitWithBow
            | OrderType::FallingHitWithSword
            | OrderType::FallingHitCrouched
            | OrderType::FallingShoulders
            | OrderType::FallingPushedUpright
            | OrderType::FallingPushedWithBow
            | OrderType::FallingPushedWithSword
            | OrderType::FallingPushedCrouched
    )
}

/// `DYING_SWORD` / `DYING_BOW` / `DYING_UPRIGHT` / `DYING_CROUCHED`
/// on motion Start: set posture to Dead (if already dead) or Lying,
/// then set action_state per family.
fn apply_dying_start_side_effect(entity: &mut Entity, anim_type: OrderType, motion: MotionState) {
    if !matches!(motion, MotionState::Start) {
        return;
    }
    let action = match anim_type {
        OrderType::DyingSword => Some(ActionState::WaitingSword),
        OrderType::DyingBow => Some(ActionState::AimingWithBow),
        OrderType::DyingUpright | OrderType::DyingCrouched => Some(ActionState::Waiting),
        _ => None,
    };
    let action = match action {
        Some(a) => a,
        None => return,
    };
    let posture = if entity.is_dead() {
        Posture::Dead
    } else {
        Posture::Lying
    };
    entity.set_posture(posture);
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = action;
    }
}

/// `EXTRACTING_ARROW_UPRIGHT` / `EXTRACTING_ARROW_CROUCHED` /
/// `EXTRACTING_ARROW_BOW` on motion Start restore the same posture and
/// action state as the shared human action branch in the original game.
fn apply_arrow_extraction_start_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    if !matches!(motion, MotionState::Start) {
        return;
    }
    let (posture, action) = match anim_type {
        OrderType::ExtractingArrowUpright => (Posture::Upright, ActionState::Waiting),
        OrderType::ExtractingArrowCrouched => (Posture::Crouched, ActionState::Waiting),
        OrderType::ExtractingArrowBow => (Posture::Upright, ActionState::AimingWithBow),
        _ => return,
    };
    entity.set_posture(posture);
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = action;
    }
}

/// `STANDING_UP*` on motion start follows the original game's shared human
/// Execute branch: normal stand-up enters waiting, sword stand-up
/// enters sword waiting, and bow stand-up only restores upright
/// posture while preserving the bow action state.
fn apply_standing_up_start_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    if !matches!(motion, MotionState::Start) {
        return;
    }
    match anim_type {
        OrderType::StandingUp => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
        }
        OrderType::StandingUpSword => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::WaitingSword;
            }
        }
        OrderType::StandingUpBow => {
            entity.set_posture(Posture::Upright);
        }
        _ => {}
    }
}

/// `BEING_CARRIED_LITTLE_JOHN` / `BEING_CARRIED_PEASANT_C` on motion
/// Start enter the carried idle state in the shared human Execute
/// branch.
fn apply_carried_start_side_effect(entity: &mut Entity, anim_type: OrderType, motion: MotionState) {
    if !matches!(
        (anim_type, motion),
        (
            OrderType::BeingCarriedLittleJohn | OrderType::BeingCarriedPeasantC,
            MotionState::Start
        )
    ) {
        return;
    }
    if entity.actor_data().is_none() {
        return;
    }
    entity.set_posture(Posture::Carried);
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = ActionState::Waiting;
    }
}

fn endurance_for_smalltalk_recovery(
    entity: &Entity,
    profile_manager: &crate::profiles::ProfileManager,
) -> Option<u16> {
    match entity {
        Entity::Pc(pc) => profile_manager
            .get_character(pc.pc.profile_index)
            .map(|profile| profile.endurance),
        Entity::Soldier(soldier) => profile_manager
            .get_soldier(soldier.soldier.soldier_profile_index)
            .map(|profile| profile.endurance),
        _ => None,
    }
}

/// Smalltalk strike/parry Execute branches set sword-waiting state at
/// animation start and recover tiredness by one tenth of endurance at
/// Terminated.
fn apply_smalltalk_start_and_recovery_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
    profile_manager: &crate::profiles::ProfileManager,
    probe: Option<(u32, u32)>,
) {
    let is_smalltalk = matches!(
        anim_type,
        OrderType::StrikingLeftSmalltalk
            | OrderType::StrikingRightSmalltalk
            | OrderType::StrikingLowLeftSmalltalk
            | OrderType::StrikingLowRightSmalltalk
            | OrderType::ParryingLeftSmalltalk
            | OrderType::ParryingRightSmalltalk
            | OrderType::ParryingLowLeftSmalltalk
            | OrderType::ParryingLowRightSmalltalk
    );
    if !is_smalltalk {
        return;
    }
    match motion {
        MotionState::Start => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::WaitingSword;
            }
        }
        MotionState::Terminated => {
            let Some(endurance) = endurance_for_smalltalk_recovery(entity, profile_manager) else {
                tracing::warn!(
                    ?anim_type,
                    "smalltalk animation terminated but actor profile endurance is unavailable"
                );
                return;
            };
            if let Some(human) = entity.human_data_mut() {
                let before = human.tiredness;
                human.tiredness = human.tiredness.saturating_sub(endurance / 10);
                if let Some((frame, creation_order)) = probe {
                    eprintln!(
                        "RUST_TIREDNESS frame={frame} co={creation_order} \
                         site=smalltalk_terminated before={before} after={} \
                         endurance={endurance} animation={anim_type:?}",
                        human.tiredness
                    );
                }
            }
        }
        _ => {}
    }
}

/// STRIKING_DOWN_SWORD refreshes its antagonist-facing goal and sets
/// sword-waiting state at Start, then launches GET_KILLED_AT_BOTTOM on the
/// victim at the action-done tag.
fn apply_striking_down_sword_side_effect(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
    antagonist_direction: Option<i16>,
    entity_id: EntityId,
) {
    let entity = engine
        .world
        .entities
        .get_mut(entity_id)
        .expect("animation owner disappeared");
    if anim_type != OrderType::StrikingDownSword {
        return;
    }
    match motion {
        MotionState::Start => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::WaitingSword;
            }
            let direction = antagonist_direction.unwrap_or_else(|| {
                panic!(
                    "actor {entity_id:?} StrikingDownSword started without an antagonist direction"
                )
            });
            entity.position_iface_mut().set_direction(
                crate::position_interface::Direction::from_raw(i32::from(direction)),
            );
        }
        MotionState::Done => {
            let Some(target) = antagonist else {
                tracing::warn!(
                    ?entity_id,
                    "StrikingDownSword reached action-done without an antagonist"
                );
                return;
            };
            engine.execute_killed_at_bottom(sim, assets, (target, entity_id));
        }
        _ => {}
    }
}

/// `BEING_DEAD_*` on motion Start: set states to (Dead, ...).
/// The dispatch returns InProgress always (never Done/Terminated), so
/// the active_ai_anim teardown never fires for these anims and the
/// corpse loops the idle sprite forever.
fn apply_being_dead_start_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    if !matches!(motion, MotionState::Start) {
        return;
    }
    let action = match anim_type {
        OrderType::BeingDeadSword => Some(ActionState::WaitingSword),
        OrderType::BeingDeadBow => Some(ActionState::AimingWithBow),
        OrderType::BeingDead => Some(ActionState::Waiting),
        OrderType::BeingDeadFallenBackSword => Some(ActionState::WaitingSword),
        OrderType::BeingDeadFallenBackBow => Some(ActionState::AimingWithBow),
        OrderType::BeingDeadFallenBack => Some(ActionState::Waiting),
        _ => None,
    };
    let action = match action {
        Some(a) => a,
        None => return,
    };
    let posture = match anim_type {
        OrderType::BeingDeadFallenBackSword
        | OrderType::BeingDeadFallenBackBow
        | OrderType::BeingDeadFallenBack => Posture::DeadBack,
        _ => Posture::Dead,
    };
    entity.set_posture(posture);
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = action;
    }
}

/// Pick the landing `(posture, optional action_state)` for a fall
/// animation that just hit its terminal motion event.  Timing differs
/// by family — see the `fall_motion_for_completion` helper for which
/// `MotionState` triggers the set.
///
/// | Anim | Trigger | Posture | ActionState |
/// |------|---------|---------|-------------|
/// | `FALLING_HIT_*` | TERMINATED | DeadBack/Lying | by weapon/posture variant |
/// | `FALLING_PUSHED_*` | TERMINATED | DeadBack/Lying | by weapon/posture variant |
/// | `FALLING_SHOULDERS` | TERMINATED | DeadBack/Lying | (unchanged) |
/// | `FALLING_BACK_UPRIGHT`/`CROUCHED` | START | DeadBack/Lying | Waiting |
/// | `FALLING_BACK_SWORD` | DONE\|TERMINATED | DeadBack/Lying | WaitingSword |
/// | `FALLING_BACK_BOW` | DONE\|TERMINATED | DeadBack/Lying | AimingWithBow |
/// | `ROLLING` | TERMINATED | Dead/Lying | (unchanged) |
///
/// `FALLING_LADDER_WALL` is absent: its landing states come from the
/// ladder-fall arrival in the flight tick, not from a sprite event.
fn fall_landing_states(
    anim_type: OrderType,
    is_dead: bool,
) -> Option<(Posture, Option<ActionState>)> {
    let lying_or_dead_back = if is_dead {
        Posture::DeadBack
    } else {
        Posture::Lying
    };
    match anim_type {
        OrderType::FallingHitUpright
        | OrderType::FallingHitHarderUpright
        | OrderType::FallingHitCrouched
        | OrderType::FallingHitHarderCrouched
        | OrderType::FallingPushedUpright
        | OrderType::FallingPushedCrouched => {
            Some((lying_or_dead_back, Some(ActionState::Waiting)))
        }
        OrderType::FallingHitWithBow
        | OrderType::FallingHitHarderWithBow
        | OrderType::FallingPushedWithBow => {
            Some((lying_or_dead_back, Some(ActionState::AimingWithBow)))
        }
        OrderType::FallingHitWithSword
        | OrderType::FallingHitHarderWithSword
        | OrderType::FallingPushedWithSword => {
            Some((lying_or_dead_back, Some(ActionState::WaitingSword)))
        }
        OrderType::FallingShoulders => Some((lying_or_dead_back, None)),
        OrderType::FallingBackUpright | OrderType::FallingBackCrouched => {
            Some((lying_or_dead_back, Some(ActionState::Waiting)))
        }
        OrderType::FallingBackSword => Some((lying_or_dead_back, Some(ActionState::WaitingSword))),
        OrderType::FallingBackBow => Some((lying_or_dead_back, Some(ActionState::AimingWithBow))),
        OrderType::Rolling => Some((
            if is_dead {
                Posture::Dead
            } else {
                Posture::Lying
            },
            None,
        )),
        // FALLING_LADDER_WALL sets no landing state from its sprite
        // event: the ladder-fall arrival (tick countdown reaching zero
        // in the flight tick) applies the lying/dead-back posture, and
        // a sprite that runs out before the countdown leaves the
        // flying posture in place.
        _ => None,
    }
}

/// Which `MotionState` triggers `fall_landing_states` for `anim_type`?
/// State is set at varying points per family:
/// - `FALLING_BACK_UPRIGHT`/`CROUCHED`: Start.
/// - `FALLING_BACK_SWORD`/`BOW`: Done or Terminated.
/// - `FALLING_HIT_HARDER_*`: Done or Terminated.
/// - non-hard `FALLING_HIT_*` / `FALLING_SHOULDERS` / `ROLLING` /
///   `FALLING_LADDER_WALL`: Terminated.
fn fall_state_trigger_matches(anim_type: OrderType, motion: MotionState) -> bool {
    match anim_type {
        OrderType::FallingBackUpright | OrderType::FallingBackCrouched => {
            matches!(motion, MotionState::Start)
        }
        OrderType::FallingBackSword | OrderType::FallingBackBow => {
            matches!(motion, MotionState::Done | MotionState::Terminated)
        }
        OrderType::FallingHitHarderUpright
        | OrderType::FallingHitHarderWithBow
        | OrderType::FallingHitHarderWithSword
        | OrderType::FallingHitHarderCrouched => {
            matches!(motion, MotionState::Done | MotionState::Terminated)
        }
        // Non-hard FallingHit sets only on TERMINATED, not DONE.
        // Rolling, FallingLadderWall, and FallingShoulders do the same.
        _ => matches!(motion, MotionState::Terminated),
    }
}

/// Orders dispatched through sprite flight rather than plain
/// action or motion processing.
///
/// The four harder-hit variants deliberately do not belong here: Original's
/// Hit-induced landing uses action processing. Ladder/wall falling
/// also uses action processing plus its own wait-time movement loop.
fn uses_perform_flight(anim_type: OrderType) -> bool {
    matches!(
        anim_type,
        OrderType::FallingHitUpright
            | OrderType::FallingHitWithBow
            | OrderType::FallingHitWithSword
            | OrderType::FallingHitCrouched
            | OrderType::FallingPushedUpright
            | OrderType::FallingPushedWithBow
            | OrderType::FallingPushedWithSword
            | OrderType::FallingPushedCrouched
    )
}

/// Non-hard `FALLING_HIT_*` / `FALLING_PUSHED_*` on motion Start.
///
/// Falling-hit behavior enters `(Flying, Moving)`; the original game
/// Pushed falling enters `(Flying, WaitingSword)` before the
/// wrapper later restores the variant-specific action on termination.
/// The ladder/wall fall enters `(Flying, Moving)` on the same event, so it
/// rides along with the falling-hit group.
/// The harder-hit falling branch uses action processing, not flight,
/// and deliberately keeps the current posture/action until landing.
/// Other fall families set state on later motion events — handled by
/// `apply_falling_completion_side_effect`.
fn apply_falling_start_side_effect(entity: &mut Entity, anim_type: OrderType, motion: MotionState) {
    if !matches!(motion, MotionState::Start) {
        return;
    }
    let action_state = if matches!(
        anim_type,
        OrderType::FallingHitUpright
            | OrderType::FallingHitWithBow
            | OrderType::FallingHitWithSword
            | OrderType::FallingHitCrouched
            | OrderType::FallingLadderWall
    ) {
        Some(ActionState::Moving)
    } else if matches!(
        anim_type,
        OrderType::FallingPushedUpright
            | OrderType::FallingPushedWithBow
            | OrderType::FallingPushedWithSword
            | OrderType::FallingPushedCrouched
    ) {
        Some(ActionState::WaitingSword)
    } else {
        None
    };
    let Some(action_state) = action_state else {
        return;
    };
    if uses_perform_flight(anim_type) {
        // Sprite flight disables anti-collision on START. Besides
        // suppressing collision checks this synchronously clears the stale
        // deviation latch used by turn-vibration suppression.
        entity.position_iface_mut().set_anti_collision_on(false);
    }
    entity.set_posture(Posture::Flying);
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = action_state;
    }
}

/// Falling-hit / shoulder-fall / falling-back / ladder-wall completion
/// for the active_ai_anim path.  Rolling is intentionally NOT covered
/// here — that's Phase 5 and still flows through the `combat_anim`
/// block.
fn apply_falling_completion_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    if anim_type == OrderType::Rolling {
        return;
    }
    if !fall_state_trigger_matches(anim_type, motion) {
        return;
    }
    if motion == MotionState::Terminated && uses_perform_flight(anim_type) {
        // Sprite flight restores anti-collision before the wrapper
        // applies its landing posture/action state.
        entity.position_iface_mut().set_anti_collision_on(true);
    }
    let is_dead = entity.is_dead();
    if let Some((posture, action_state)) = fall_landing_states(anim_type, is_dead) {
        entity.set_posture(posture);
        let hard_hit_done = motion == MotionState::Done
            && matches!(
                anim_type,
                OrderType::FallingHitHarderUpright
                    | OrderType::FallingHitHarderWithBow
                    | OrderType::FallingHitHarderWithSword
                    | OrderType::FallingHitHarderCrouched
            );
        // Hit-induced landing completes on the done state, but the outer
        // FALLING_HIT_HARDER_* wrapper restores its action family only
        // when the sprite later reports TERMINATED.
        let action_state = if uses_perform_flight(anim_type) {
            match anim_type {
                OrderType::FallingPushedUpright
                | OrderType::FallingPushedWithBow
                | OrderType::FallingPushedWithSword
                | OrderType::FallingPushedCrouched => Some(ActionState::WaitingSword),
                _ => None,
            }
        } else {
            action_state
        };
        if !hard_hit_done
            && let Some(action) = action_state
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.action_state = action;
        }
    }
}

/// Restore the enclosing action family after landing callbacks return.
fn finish_flight_action_state(entity: &mut Entity, anim_type: OrderType, motion: MotionState) {
    if motion == MotionState::Terminated && uses_perform_flight(anim_type) {
        let (_, action) = fall_landing_states(anim_type, entity.is_dead())
            .expect("flight order has no landing state");
        entity
            .actor_data_mut()
            .expect("flight owner is not an actor")
            .action_state = action.expect("flight order has no final action");
    }
}

/// Soldier combat-injury anims (`BEING_HIT_SWORD`,
/// `EXTRACTING_ARROW_SWORD`, `BEING_WEAK_SWORD`, `BEING_STUNNED_SWORD`,
/// `STANDING_UP_SWORD`)
/// dispatch `EventAfterCombatInjury` to the AI when they terminate so
/// the soldier can resume the fight before order advancement.
fn apply_combat_injury_side_effect(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    anim_type: OrderType,
    motion: MotionState,
    entity_id: EntityId,
) {
    if matches!(motion, MotionState::Terminated)
        && matches!(
            anim_type,
            OrderType::BeingHitSword
                | OrderType::ExtractingArrowSword
                | OrderType::BeingWeakSword
                | OrderType::BeingStunnedSword
                | OrderType::StandingUpSword
        )
        && engine
            .expect_entity(entity_id, "combat injury owner")
            .is_soldier()
    {
        engine.dispatch_combat_injury_think_for_actor_hourglass(sim, entity_id, assets);
        super::tick::observe_actor_animation_boundary(
            super::tick::ActorAnimationBoundaryPhase::CombatInjuryThink(entity_id),
        );
    }
}

/// `BEING_WEAK_SWORD` reduces tiredness after a sprite tick. When that tick
/// first reaches the action-done frame, preserve the sprite's `Done` result:
/// The original game checks action completion before advancing the action, so the hold begins
/// only on the following actor tick in [`hold_weak_sword_at_action_done`].
fn apply_weak_sword_tiredness_after_perform(entity: &mut Entity, anim_type: OrderType) {
    if anim_type != OrderType::BeingWeakSword {
        return;
    }
    let Some(human) = entity.human_data_mut() else {
        return;
    };

    human.tiredness = human.tiredness.saturating_sub(WEAKNESS_DISMISH);
}

fn sprite_is_at_action_done(sprite: &crate::sprite::Sprite) -> bool {
    sprite.current_frame == sprite.action_done_frame
        && sprite.frame_count == sprite.action_done_counter
}

fn hold_weak_sword_at_action_done(
    entity: &mut Entity,
    anim_type: OrderType,
) -> Option<MotionState> {
    if anim_type != OrderType::BeingWeakSword {
        return None;
    }
    if !sprite_is_at_action_done(&entity.element_data().sprite) {
        return None;
    }
    let human = entity.human_data_mut()?;
    human.tiredness = human.tiredness.saturating_sub(WEAKNESS_DISMISH);
    if human.tiredness == 0 {
        None
    } else {
        Some(MotionState::InProgress)
    }
}

/// Shield transition completion: set the post-animation `(posture, action_state)`
/// when a shield raise / lower / parry one-shot finishes.  Each
/// arm sets states fully so posture is also coerced to UPRIGHT.
///
/// - `RAISING_SHIELD`  on `MOTION_DONE`       → `(Upright, HoldingShield)`
/// - `LOWERING_SHIELD` on `MOTION_DONE`       → `(Upright, Waiting)`
/// - `PARRYING_SHIELD` on `MOTION_DONE`       → `(Upright, ParryingShield)`
/// - `PARRYING_SHIELD` on `MOTION_TERMINATED` → `(Upright, HoldingShield)`
fn apply_shield_transition_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    let new_state = match (anim_type, motion) {
        (OrderType::RaisingShield, MotionState::Done | MotionState::Terminated) => {
            Some(ActionState::HoldingShield)
        }
        (OrderType::LoweringShield, MotionState::Done | MotionState::Terminated) => {
            Some(ActionState::Waiting)
        }
        // DONE → ParryingShield: the parry animation running to completion
        // is what puts the actor into the parrying state.
        (OrderType::ParryingShield, MotionState::Done) => Some(ActionState::ParryingShield),
        (OrderType::ParryingShield, MotionState::Terminated) => Some(ActionState::HoldingShield),
        _ => None,
    };
    if let Some(state) = new_state {
        entity.set_posture(Posture::Upright);
        if let Some(actor) = entity.actor_data_mut() {
            actor.action_state = state;
        }
    }
}

/// PC cape/tree disguise exit completion mirrors the original game's action
/// arms: on DONE the actor becomes Upright/Waiting and the Hidden
/// titbit is removed before the transition returns.
fn apply_pc_disguise_exit_side_effect(
    engine: &mut EngineInner,
    anim_type: OrderType,
    motion: MotionState,
    command: Option<Command>,
    reusable_cloaks_enabled: bool,
    entity_id: EntityId,
) {
    let entity = engine
        .world
        .entities
        .get_mut(entity_id)
        .expect("animation owner disappeared");
    if !entity.is_pc() || !matches!(motion, MotionState::Done) {
        return;
    }
    if !matches!(
        anim_type,
        OrderType::TransitionWaitingCapeWaitingUpright
            | OrderType::TransitionWaitingHiddenWaitingUpright
    ) {
        return;
    }
    let entering_reusable_cloak = anim_type == OrderType::TransitionWaitingCapeWaitingUpright
        && command == Some(Command::EnterCloak)
        && reusable_cloaks_enabled;
    entity.set_posture(if entering_reusable_cloak {
        Posture::Cloaked
    } else {
        Posture::Upright
    });
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = ActionState::Waiting;
    }
    if !entering_reusable_cloak {
        engine.execute_hidden_titbit_removals(entity_id);
    }
}

/// Sword parry state transitions mirror the original game:
/// waiting-to-parry transition seeds normal parry on termination, low
/// parry enters its low state on start and computes its hold counter
/// relative to the opponent's current action-done timing, and
/// parry-to-waiting / low-parry completion return to WaitingSword.
fn apply_sword_parry_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
    principal_frames_from_now: Option<i16>,
) {
    let new_state = match (anim_type, motion) {
        (OrderType::TransitionWaitingSwordParryingSword, MotionState::Terminated) => {
            if let Some(human) = entity.human_data_mut() {
                human.parry_counter = crate::engine::melee::TIME_TO_STAY_IN_PARRY_MODE;
            }
            Some(ActionState::ParryingSword)
        }
        (OrderType::TransitionWaitingSwordParryingSwordLow, MotionState::Start) => {
            Some(ActionState::ParryingSwordLow)
        }
        (OrderType::TransitionWaitingSwordParryingSwordLow, MotionState::Terminated) => {
            let counter = if entity
                .human_data()
                .map(|h| !h.opponents.is_empty())
                .unwrap_or(false)
            {
                let own_start = entity
                    .element_data()
                    .sprite
                    .frames_from_start_till_action_done(OrderType::ParryingLowSword)
                    as i16;
                principal_frames_from_now.unwrap_or(1) - own_start
            } else {
                crate::engine::melee::TIME_TO_STAY_IN_PARRY_MODE as i16
            };
            if let Some(human) = entity.human_data_mut() {
                human.parry_counter = counter.max(1) as u16;
            }
            None
        }
        (OrderType::ParryingSword, MotionState::Start) => Some(ActionState::ParryingSword),
        (OrderType::ParryingLowSword, MotionState::Start) => Some(ActionState::ParryingSwordLow),
        (
            OrderType::TransitionParryingSwordWaitingSword | OrderType::ParryingLowSword,
            MotionState::Terminated,
        ) => Some(ActionState::WaitingSword),
        _ => None,
    };
    if let Some(state) = new_state {
        entity.set_posture(Posture::Upright);
        if let Some(actor) = entity.actor_data_mut() {
            actor.action_state = state;
        }
    }
}

fn apply_under_net_initialization_side_effect(
    sim: &crate::sim_rng::SimulationContext,
    entity: &mut Entity,
    anim_type: OrderType,
) {
    use crate::ai::EmoticonType;
    use crate::order::OrderType as OT;

    if !matches!(anim_type, OT::LyingStuckUnderNet | OT::WriggleUnderNet) {
        return;
    }

    let is_unconscious = entity.is_unconscious();
    let is_tied = entity.element_data().posture() == Posture::Tied;
    if !entity.is_dead() && !is_unconscious && !is_tied {
        entity.set_posture(Posture::StuckUnderNet);
    }
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = ActionState::Waiting;
    }

    if matches!(anim_type, OT::WriggleUnderNet) {
        match crate::sim_rng::u32(sim, crate::sim_rng::RngSite::WriggleDirection, ..3) {
            0 => {
                let direction = (entity.element_data().direction() + 1) & 15;
                entity.element_data_mut().set_direction_instantly(direction);
            }
            1 => {
                let direction = (entity.element_data().direction() + 15) & 15;
                entity.element_data_mut().set_direction_instantly(direction);
            }
            _ => {}
        }

        if matches!(entity, Entity::Soldier(_)) {
            if let Some(ai) = entity.ai_controller_mut() {
                ai.set_emoticon(EmoticonType::Thunderstorm);
            } else {
                // TODO(net-parity): level-loaded soldiers should always have an AI controller.
                tracing::warn!(
                    "WriggleUnderNet soldier has no AI controller for thunderstorm emoticon"
                );
            }
        }
    }
}

fn apply_under_net_termination_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    if !matches!(
        (anim_type, motion),
        (OrderType::WriggleUnderNet, MotionState::Terminated)
    ) {
        return;
    }

    if matches!(entity, Entity::Soldier(_)) {
        if let Some(ai) = entity.ai_controller_mut() {
            ai.clear_emoticon();
        } else {
            // TODO(net-parity): level-loaded soldiers should always have an AI controller.
            tracing::warn!("WriggleUnderNet soldier has no AI controller to clear emoticon");
        }
    }
}

fn safe_frames_from_now_till_action_done(sprite: &crate::sprite::Sprite) -> Option<i16> {
    let script = sprite.scripts.get(sprite.current_row as usize)?;
    let max_frame = sprite.current_frame.max(sprite.action_done_frame) as usize;
    if max_frame >= script.delays.len() {
        return None;
    }
    Some(sprite.frames_from_now_till_action_done())
}

/// Per-arm dispatch result.  Each arm of the per-anim switch returns
/// a motion state that the actor-update dispatcher then acts on.
///
/// Semantics:
/// - `Forward(TERMINATED)` → advance the sequence element.
/// - `Forward(ABORTED)`    → set the sequence element to Impossible.
/// - forwarding completion → no-op (the completion flag was never
///   actually read in the original engine).
/// - `Forward(START/IN_PROGRESS)` → no-op.
/// - `Consumed`            → arm short-circuited the update
///   dispatch entirely (loop/idle/corpse anims + BORED in-place
///   mutation).
#[derive(Debug, Clone, Copy)]
enum ExecuteOutcome {
    /// Arm consumed the event.  Equivalent to "return InProgress" —
    /// the arm has handled the motion state internally, and the
    /// The update must not advance or terminate the element.
    Consumed,
    /// Forward the motion state to actor-update dispatch.
    /// Only TERMINATED (advance) and ABORTED (impossible) have
    /// observable effects; DONE / START / InProgress are no-ops.
    Forward(MotionState),
}

/// Animation arms that always return `InProgress` from their per-anim
/// dispatch — loop/idle/corpse/immobilization arms.  They cycle
/// forever (or until external interruption) and must never advance
/// the owning sequence element even when the sprite reaches
/// TERMINATED.  This matches every always-IN_PROGRESS arm across the
/// five dispatch switches (base actor + Human + NPC + PC + Soldier
/// subclass overrides).
///
/// Arms *not* in this list take the default "forward motion" path —
/// TERMINATED advances, everything else no-ops.  A few arms with
/// conditional IN_PROGRESS branches (e.g. `GETTING_FREE_FROM_WASP`
/// which returns IN_PROGRESS only while still turning, otherwise
/// returns the sprite's motion state) are *not* in this list because
/// their `OrderCompletion` side-channel (e.g. `WaspStruggleCycle`)
/// needs the TERMINATED advance to fire.
fn arm_is_always_consumed(anim_type: OrderType) -> bool {
    use OrderType as OT;
    matches!(
        anim_type,
        // Idle loops whose derived Execute arms explicitly return
        // InProgress. Plain WaitingUpright is intentionally absent:
        // base actor execution forwards the action's raw motion result so a
        // Wait transition chain can advance when that animation terminates.
        // The PC cape/hidden idle loops are absent for the same reason: their
        // arms play a cyclic action and return its raw result, so the
        // first tick of a freshly adopted order must still report Start.
        OT::WaitingCrouched
            | OT::WaitingAlerted
            | OT::WaitingSword
            | OT::WaitingShield
            | OT::WaitingOnShoulders
            | OT::WaitingHelpingClimbing
            | OT::WaitingCarryingOnShoulders
            | OT::WaitingWithCorpse
            | OT::WaitingWithPurse
            // Aim loops
            | OT::AimingWithBow
            | OT::AimingWithBowUp
            | OT::AimingWithBowLeaningOut
            | OT::AimingWithBowAnonymous
            | OT::AimingWithBowUpAnonymous
            // Activity loops
            | OT::Sitting
            | OT::Listening
            | OT::Menacing
            | OT::SleepingUpright
            | OT::LeaningOut
            | OT::SimulatingBeggar
            // Parry holds are timer-controlled in the soldier execute
            // arm. Their sprite may terminate earlier, but the original game keeps
            // returning IN_PROGRESS until the parry counter expires.
            | OT::ParryingSword
            | OT::ParryingLowSword
            // Corpse / KO loops (freeze-when-terminated progression)
            | OT::BeingDead
            | OT::BeingDeadFallenBack
            | OT::BeingDeadSword
            | OT::BeingDeadBow
            | OT::BeingDeadFallenBackSword
            | OT::BeingDeadFallenBackBow
            // Immobilization / tied
            | OT::WriggleUnderNet
            | OT::BeingTied
            // Non-animation holds
            | OT::Freezing
            | OT::PlayCustomFrozen
            | OT::RefreshingSeek
            // HIDING_BEHIND_SHIELD (PC): always IN_PROGRESS except for
            // a sequence-validity early-out.  The validity check
            // isn't modelled in the dispatcher; mark Consumed — if
            // the shield-holder becomes invalid, the sequence system
            // terminates the element via its own cascade.
            | OT::HidingBehindShield
    )
}

/// Per-arm completion dispatch.  Each arm decides whether to short
/// circuit (Consumed) or forward its motion state to actor-update
/// semantics (Forward).
///
/// Arms covered by explicit in-place mutation:
/// - **`WAITING_UPRIGHT_BORED` / `WAITING_UPRIGHT_BORED_RANDOM`**:
///   Terminated rerolls the animation type and assigns a new ID (BORED ↔ RANDOM
///   with 1/10 bias).
/// - **`LYING_STUCK_UNDER_NET`**: Terminated rolls a 1/31 chance to
///   mutate to `WRIGGLE_UNDER_NET` with a new ID.
///
/// Everything else: Consumed if in [`arm_is_always_consumed`],
/// otherwise Forward(motion) — the actor-update dispatcher in the
/// teardown block decides per motion state.
/// Context passed to [`dispatch_arm_completion`].  Bundles the
/// entity / sequence / RNG handles the dispatcher needs so the
/// function signature stays small even as more arms are implemented.
struct ArmCtx<'a> {
    entity_id: EntityId,
    is_npc: bool,
    is_unconscious: bool,
    seq_id: crate::sequence::SequenceId,
    elem_idx: usize,
    engine: &'a mut EngineInner,
    assets: &'a LevelAssets,
}

fn dispatch_arm_completion(
    sim: &crate::sim_rng::SimulationContext,

    anim_type: OrderType,
    motion: MotionState,
    ctx: &mut ArmCtx<'_>,
) -> ExecuteOutcome {
    use crate::order::OrderType as OT;
    use crate::sprite::MotionState as MS;

    // Human action execution always returns IN_PROGRESS from WAITING_SWORD after
    // advancing the sprite and evaluating smalltalk/swordfight state.  A
    // looping sprite may still report Done; that edge is not visible to the
    // base actor-update completion machinery.
    if anim_type == OT::WaitingSword {
        return ExecuteOutcome::Consumed;
    }

    // BORED ↔ RANDOM idle cycle — always Consumed. Terminated rerolls
    // the animation type and assign a new ID for every owning command except
    // `WaitTimer`, exactly matching the base Actor Execute guard.
    if matches!(
        anim_type,
        OT::WaitingUprightBored | OT::WaitingUprightBoredRandom
    ) {
        // Civilian execution returns the raw coerced
        // action result for this family, bypassing all base-actor
        // bored-loop mutation and return-value handling.
        if matches!(ctx.entity_id, EntityId::Civilian(_)) {
            return ExecuteOutcome::Forward(motion);
        }
        if anim_type == OT::WaitingUprightBored && motion == MS::Start {
            // Base Actor preserves the first-entry edge for the ordinary
            // bored loop after applying its Bored action state. Every other
            // result, and every BoredRandom result, is consumed as
            // InProgress.
            return ExecuteOutcome::Forward(MS::Start);
        }
        if matches!(motion, MS::Terminated) {
            let is_wait_timer = ctx
                .engine
                .orders
                .sequence_manager
                .get_element(ctx.seq_id, ctx.elem_idx)
                .map(|el| matches!(el.command, crate::element::Command::WaitTimer))
                .unwrap_or_else(|| {
                    panic!(
                        "bored animation owner {:?} lost sequence element {:?}/{}",
                        ctx.entity_id, ctx.seq_id, ctx.elem_idx
                    )
                });
            if !is_wait_timer {
                let next_type = match anim_type {
                    OT::WaitingUprightBored => {
                        tracing::debug!(
                            target: "parity_rng_owner",
                            owner = ?ctx.entity_id,
                            site = ?crate::sim_rng::RngSite::BoredAnimationChoice,
                            "authoritative RNG draw owner"
                        );
                        (crate::sim_rng::u32(
                            sim,
                            crate::sim_rng::RngSite::BoredAnimationChoice,
                            ..10,
                        ) == 0)
                            .then_some(OT::WaitingUprightBoredRandom)
                    }
                    OT::WaitingUprightBoredRandom => Some(OT::WaitingUprightBored),
                    _ => unreachable!(),
                };
                if let Some(next_type) = next_type
                    && let Some(elem) = ctx
                        .engine
                        .orders
                        .sequence_manager
                        .get_element_mut(ctx.seq_id, ctx.elem_idx)
                    && let Some(front) = elem.orders.front_mut()
                {
                    front.order_type = next_type;
                    front.order_id =
                        crate::order::alloc_order_id(&mut ctx.engine.orders.next_order_id);
                }
            }
        }
        return ExecuteOutcome::Consumed;
    }

    // LYING_STUCK_UNDER_NET: every tick, 1/31 chance to mutate to
    // WRIGGLE_UNDER_NET with a new ID. The sprite plays `WRIGGLE_UNDER_NET`
    // frozen on the first frame regardless; the cycle flip is what
    // makes the actor occasionally play the struggle animation.  Roll
    // on every motion state (the roll runs before the motion-state
    // switch), then always return InProgress.
    if matches!(anim_type, OT::LyingStuckUnderNet) {
        if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::NetWriggleGate, ..31) == 0 {
            if let Some(elem) = ctx
                .engine
                .orders
                .sequence_manager
                .get_element_mut(ctx.seq_id, ctx.elem_idx)
                && let Some(front) = elem.orders.front_mut()
            {
                front.order_type = OT::WriggleUnderNet;
                front.order_id = crate::order::alloc_order_id(&mut ctx.engine.orders.next_order_id);
            }
            // Cry for help: NPCs (soldier or civilian) say
            // REMARK_UNDER_NET / CIV_REMARK_UNDER_NET and emit a
            // HEEELP noise at their position.  The remark variant is
            // picked at post-tick time based on entity subclass.
            if ctx.is_npc {
                ctx.engine
                    .execute_cry_for_help_under_net(sim, ctx.assets, ctx.entity_id);
            }
        }
        return ExecuteOutcome::Consumed;
    }

    // WRIGGLE_UNDER_NET: on sprite termination the current order
    // mutates back to the frozen lying-under-net hold with a fresh id,
    // while the sequence still sees InProgress.
    if matches!(anim_type, OT::WriggleUnderNet) {
        if matches!(motion, MS::Terminated)
            && let Some(elem) = ctx
                .engine
                .orders
                .sequence_manager
                .get_element_mut(ctx.seq_id, ctx.elem_idx)
            && let Some(front) = elem.orders.front_mut()
        {
            front.order_type = OT::LyingStuckUnderNet;
            front.order_id = crate::order::alloc_order_id(&mut ctx.engine.orders.next_order_id);
        }
        return ExecuteOutcome::Consumed;
    }

    if matches!(
        anim_type,
        OT::BeingUnconscious | OT::BeingUnconsciousSword | OT::BeingUnconsciousBow
    ) {
        return if ctx.is_unconscious {
            ExecuteOutcome::Consumed
        } else {
            ExecuteOutcome::Forward(MS::Terminated)
        };
    }

    // Loop/idle/corpse arms that always return InProgress.
    if arm_is_always_consumed(anim_type) {
        return ExecuteOutcome::Consumed;
    }

    // Default: forward to actor-update dispatch. TERMINATED advances;
    // ABORTED sets sequence IMPOSSIBLE; DONE / START / InProgress are
    // no-ops.
    ExecuteOutcome::Forward(motion)
}

/// Resolve the arm's return after its callbacks. The actor update applies
/// wait modifiers and crossings before completing the then-live sequence.
fn finish_actor_execute_result(
    sim: &crate::sim_rng::SimulationContext,
    anim_type: OrderType,
    motion: Option<MotionState>,
    arm_ctx: &mut ArmCtx<'_>,
) -> MotionState {
    let entity_id = arm_ctx.entity_id;
    let seq_id = arm_ctx.seq_id;
    let elem_idx = arm_ctx.elem_idx;
    let outcome = motion.map(|m| dispatch_arm_completion(sim, anim_type, m, arm_ctx));

    let effective_motion = match outcome.unwrap_or_else(|| {
        panic!(
            "actor {entity_id:?} {anim_type:?} produced no Execute motion at {seq_id:?}/{elem_idx}"
        )
    }) {
        ExecuteOutcome::Forward(motion) => motion,
        ExecuteOutcome::Consumed => MotionState::InProgress,
    };
    effective_motion
}

impl EngineInner {
    pub(super) fn finish_patch_transition_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::types::LevelAssets,
        patch_idx: crate::patch::PatchIndex,
    ) {
        {
            let patch = self
                .script_domains
                .interactables
                .patches
                .get_mut(usize::from(patch_idx))
                .unwrap_or_else(|| {
                    panic!("completed patch animation references missing patch {patch_idx}")
                });
            patch.in_transition = false;
        }
        self.apply_patch_final(sim, assets, patch_idx, false);

        for door in self.script_domains.interactables.doors.iter_mut() {
            if door.patch_index == Some(patch_idx) {
                door.gate_state.finish_transition();
                tracing::debug!(
                    %patch_idx,
                    new_state = ?door.gate_state,
                    "gate_state advanced on patch transition complete"
                );
            }
        }

        tracing::debug!(
            %patch_idx,
            "Patch transition animation completed → apply final state"
        );
    }

    /// Run the existing generic actor animation/`Execute` dispatch for one
    /// live legacy creation slot.
    ///
    /// Eligibility remains deliberately narrower than the actor update:
    /// movement, melee, and bow actors keep their existing owners. Inactive
    /// actors still execute: the original game visits every
    /// element without an activity gate, and actor idles keep advancing
    /// while the actor is hidden from the active world.
    /// Per-actor execution-frozen actors remain skipped unless an installed
    /// WAIT_TIMER/WAIT_FREE_LIFT needs the post-execution check. A
    /// global FrozenAll does not skip Execute: it suppresses the selected
    /// sprite call while preserving pre/post-sprite arm work. The caller must
    /// still run `ActionChange` for skipped actors.
    pub(super) fn tick_actor_animation_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::types::LevelAssets,
        entity_id: EntityId,
    ) -> Option<MotionState> {
        let entry = match self.actor_animation_entry(entity_id) {
            std::ops::ControlFlow::Continue(entry) => entry,
            std::ops::ControlFlow::Break(frozen_wait) => {
                return frozen_wait;
            }
        };

        // The original game dispatches the selected actor order through ordinary
        // Execute regardless of the actor's stale action-state enum. Only an
        // order stored in the movement sequence element belongs to the separate
        // movement driver. This distinction covers movement-exit animations,
        // injuries, waits, and interactions without a command allowlist.
        let selected_generic_order = self.actor_animation_selects_generic_order(entity_id);
        let Some(operands) = self.actor_animation_operands(entity_id, selected_generic_order)
        else {
            return None;
        };

        self.initialize_actor_animation_placement(assets, entity_id);

        self.execute_actor_animation(
            sim,
            assets,
            entity_id,
            selected_generic_order,
            entry,
            operands,
        )
    }

    /// Initialize live takeoff and death placement before generic sprite dispatch.
    fn initialize_actor_animation_placement(&mut self, assets: &LevelAssets, entity_id: EntityId) {
        let ladder = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execute_order_initialising)
            .then(|| {
                self.orders
                    .sequence_manager
                    .current_order_for_actor(&self.world.entities, entity_id)
                    .filter(|(_, _, order)| order.order_type == OrderType::FallingLadderWall)
                    .map(|(seq_id, elem_idx, order)| (seq_id, elem_idx, order.destination_3d))
            })
            .flatten();
        if let Some((seq_id, elem_idx, destination)) = ladder {
            self.execute_non_interruptable_lifts((seq_id, elem_idx));
            self.initialize_ladder_fall(entity_id, destination);
        }

        // Hit-damage translation only appends a FALLING_HIT_* order. Original
        // Hit-induced falling samples live geometry and prepares takeoff
        // during initialization, so actors whose creation slot has already
        // passed retain their old facing and no flight state until next frame.
        let initial_hit_flight = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execute_order_initialising)
            .then(|| {
                self.orders
                    .sequence_manager
                    .current_order_for_actor(&self.world.entities, entity_id)
                    .map(|(seq_id, elem_idx, order)| {
                        (seq_id, elem_idx, order.order_type, order.antagonist)
                    })
            })
            .flatten()
            .filter(|(_, _, anim, _)| {
                matches!(
                    anim,
                    OrderType::FallingHitUpright
                        | OrderType::FallingHitWithBow
                        | OrderType::FallingHitWithSword
                        | OrderType::FallingHitCrouched
                )
            });
        if let Some((seq_id, elem_idx, anim, antagonist)) = initial_hit_flight {
            self.execute_non_interruptable_lifts((seq_id, elem_idx));
            self.initialize_hit_flight(assets, entity_id, antagonist, anim);
        }

        // Push-damage translation likewise only authors the falling order.
        // Pushed falling initializes takeoff from live strike
        // geometry immediately before its first flight update.
        let initial_push_flight = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execute_order_initialising)
            .then(|| {
                self.orders
                    .sequence_manager
                    .current_order_for_actor(&self.world.entities, entity_id)
                    .map(|(sequence_id, element_index, order)| {
                        (sequence_id, element_index, order.order_type)
                    })
            })
            .flatten()
            .filter(|(_, _, anim)| {
                matches!(
                    anim,
                    OrderType::FallingPushedUpright
                        | OrderType::FallingPushedWithBow
                        | OrderType::FallingPushedWithSword
                        | OrderType::FallingPushedCrouched
                )
            });
        if let Some((sequence_id, element_index, anim)) = initial_push_flight {
            self.execute_non_interruptable_lifts((sequence_id, element_index));
            self.initialize_push_flight(assets, entity_id, (sequence_id, element_index), anim);
        }

        // The human actor's eight dying/falling branches run
        // death-place selection during initialization immediately before
        // action processing. Do this before borrowing the actor for the generic
        // sprite dispatch so same-stack animation side effects and callbacks
        // observe the relocated position in Original order.
        let find_place_to_die_on_initialisation = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execute_order_initialising)
            && self
                .orders
                .sequence_manager
                .current_order_for_actor(&self.world.entities, entity_id)
                .is_some_and(|(_, _, order)| {
                    matches!(
                        order.order_type,
                        OrderType::DyingSword
                            | OrderType::DyingBow
                            | OrderType::FallingBackSword
                            | OrderType::FallingBackBow
                            | OrderType::DyingUpright
                            | OrderType::FallingBackUpright
                            | OrderType::FallingBackCrouched
                            | OrderType::DyingCrouched
                    )
                });
        if find_place_to_die_on_initialisation {
            self.find_place_to_die(entity_id);
        }
    }

    /// Dispatch animation sound triggers at the deferred presentation boundary.
    ///
    /// Every animated element type runs a current-sound-id check from its
    /// refresh in the original game. That happens after the authoritative
    /// post-simulation-tick snapshot, so Rust invokes this immediately
    /// before the next hourglass through the pending presentation refresh.
    /// When the sprite's current frame has a
    /// non-zero sound ID, an FX sound is queued at the entity's
    /// position (with material for actors and projectiles, without
    /// for scenic FX/objects).
    ///
    /// This must not run inside the entity tick: doing so updates
    /// `Sprite::last_sound_id` one parity boundary too early.
    pub(super) fn dispatch_frame_sounds(&mut self) {
        use crate::element::GameMaterial;
        use crate::sound_cache::Material;

        // Collect triggers during iteration so the sound manager mutation
        // isn't interleaved with mutable entity iteration.
        let mut triggers: Vec<(u32, crate::coordinates::MapPoint, Option<Material>)> = Vec::new();

        for (_, entity) in self.world.entities.occupied_mut() {
            if !entity.is_active() {
                continue;
            }

            // Actors (PC/NPC) and projectiles pass material; other
            // elements (FX/objects/ale/bonus/scroll/net) pass None.
            let wants_material = entity.actor_data().is_some() || entity.is_projectile();

            let elem = entity.element_data_mut();
            let sprite = &mut elem.sprite;

            let sound_id = sprite.current_sound_id();
            if sound_id == 0 {
                continue;
            }

            let material = if wants_material {
                // GameMaterial::LightShadow (10) has no sound-material
                // counterpart (Material enum only covers Ground..=Hole),
                // so treat it as material-less.
                match elem.material() {
                    GameMaterial::Ground => Some(Material::Ground),
                    GameMaterial::Wood => Some(Material::Wood),
                    GameMaterial::Stone => Some(Material::Stone),
                    GameMaterial::Grass => Some(Material::Grass),
                    GameMaterial::Leaves => Some(Material::Leaves),
                    GameMaterial::Water => Some(Material::Water),
                    GameMaterial::Bush => Some(Material::Bush),
                    GameMaterial::Ice => Some(Material::Ice),
                    GameMaterial::Hole => Some(Material::Hole),
                    GameMaterial::NumberOfMaterials | GameMaterial::LightShadow => None,
                }
            } else {
                None
            };

            triggers.push((sound_id as u32, elem.position_map(), material));
        }

        for (fx_id, position, material) in triggers {
            self.feedback
                .pending_side_effects
                .sounds
                .push(super::SoundCommand::Fx {
                    fx_id,
                    position,
                    material,
                });
        }
    }
}

#[cfg(test)]
mod soldier_take_drink_parity_tests {
    use super::*;
    use crate::order::Order;
    use crate::sequence::SequenceElement;

    fn three_frame_animation(action: OrderType) -> crate::sprite::Sprite {
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[action as usize] = 0;
        let script = crate::sprite_script::SpriteScript {
            action_id: action as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2, 3],
            delays: vec![0, 0, 1],
            distances: vec![0; 3],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0; 3],
        };
        crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        )
    }

    #[test]
    fn drinking_only_adds_alcohol_after_successful_sprite_tail() {
        for removed_at_tick in [Some(0), Some(1), None] {
            let sim = crate::sim_rng::test_context();
            let mut engine = EngineInner::new();
            let mut soldier = crate::engine::test_support::actors::make_test_ai_soldier(
                crate::element::Camp::Lacklandists,
            );
            soldier.element_data_mut().sprite = three_frame_animation(OrderType::DrinkingAle);
            let owner = engine.add_test_entity(soldier);
            let bottle = engine.add_test_entity(Entity::Bonus(crate::element::ElementBonus {
                element: {
                    let mut element = crate::element::ElementData::default();
                    element.kind = crate::element::ElementKind::ObjectOther;
                    element.active = true;
                    element
                },
                object: crate::element::ObjectData {
                    object_type: crate::element::ObjectType::Ale,
                    ..Default::default()
                },
            }));
            let mut assets = engine.test_runtime_assets();
            let profile = engine
                .get_entity(owner)
                .unwrap()
                .soldier_data()
                .unwrap()
                .soldier_profile_index;
            std::sync::Arc::make_mut(&mut assets.profile_manager).soldiers[profile.0 as usize]
                .beer = 40;
            let mut element =
                SequenceElement::new_interaction(1, Command::Wait, Some(owner), Some(bottle));
            element.orders.push_back(
                Order::test_new(OrderType::DrinkingAle, 0.0, 0.0).with_antagonist(bottle),
            );
            let sequence = engine.orders.sequence_manager.insert_element(element);
            engine
                .orders
                .sequence_manager
                .start_sequence_level(sequence);
            engine.select_sequence_element(owner, Some((sequence, 0)));

            let mut terminated = false;
            let mut saw_done = false;
            for tick in 0..8 {
                if removed_at_tick == Some(tick) {
                    engine
                        .get_entity_mut(bottle)
                        .unwrap()
                        .element_data_mut()
                        .active = false;
                }
                let result = engine
                    .tick_actor_animation_for(&sim, &assets, owner)
                    .expect("selected drinking order must execute");
                saw_done |= result == MotionState::Done;
                let alcohol = engine
                    .get_entity(owner)
                    .unwrap()
                    .npc_data()
                    .unwrap()
                    .ai_brain
                    .base()
                    .unwrap()
                    .blood_alcohol;
                if result == MotionState::Terminated {
                    assert_eq!(alcohol, if removed_at_tick.is_none() { 40 } else { 0 });
                    terminated = true;
                    break;
                }
                assert_eq!(alcohol, 0, "taking the bottle is not the drinking tail");
                engine
                    .get_entity_mut(owner)
                    .unwrap()
                    .actor_data_mut()
                    .unwrap()
                    .execute_order_initialising = false;
            }
            assert!(terminated);
            assert_eq!(saw_done, removed_at_tick.is_none());
            assert!(!engine.get_entity(bottle).unwrap().is_active());
        }
    }

    #[test]
    fn attentive_turning_plays_entry_direction_and_returns_turn_completion() {
        let sim = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        let mut soldier = crate::engine::test_support::actors::make_test_ai_soldier(
            crate::element::Camp::Lacklandists,
        );
        soldier.enemy_ai_mut().unwrap().attentive = true;
        soldier.element_data_mut().sprite = three_frame_animation(OrderType::TurningAlerted);
        soldier.element_data_mut().set_direction_instantly(3);
        soldier.element_data_mut().set_direction_goal(5);
        let owner = engine.add_test_entity(soldier);
        let assets = engine.test_runtime_assets();
        let mut element = SequenceElement::new(1, Command::Turn, Some(owner));
        element
            .orders
            .push_back(Order::test_new(OrderType::Turning, 0.0, 0.0));
        let sequence = engine.orders.sequence_manager.insert_element(element);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);
        engine.select_sequence_element(owner, Some((sequence, 0)));

        for (entry_direction, direction, expected) in [
            (3, 4, MotionState::InProgress),
            (4, 5, MotionState::InProgress),
            (5, 5, MotionState::Terminated),
        ] {
            let result = engine
                .tick_actor_animation_for(&sim, &assets, owner)
                .unwrap();
            let actor = engine.get_entity(owner).unwrap();
            assert_eq!(actor.sprite().current_row, entry_direction);
            assert_eq!(actor.element_data().direction(), direction);
            assert_eq!(result, expected);
            assert_eq!(actor.sprite().last_motion_state, Some(expected));
        }
    }

    #[test]
    fn play_anim_property_only_applies_to_the_custom_wrapper_order() {
        for wrapper in [
            OrderType::PlayCustom,
            OrderType::PlayCustomLooped,
            OrderType::PlayCustomFreeze,
            OrderType::PlayCustomFrozen,
        ] {
            assert!(is_custom_animation_order(wrapper));
        }
        assert!(
            !is_custom_animation_order(OrderType::TransitionParryingSwordWaitingSword),
            "a PlayAnim sequence can retain its command while a prerequisite transition is selected"
        );
        assert!(!is_custom_animation_order(OrderType::WaitingUpright));
    }

    #[test]
    fn play_anim_freeze_followup_waits_for_the_custom_wrapper_to_terminate() {
        assert!(!play_anim_freeze_completed(
            MotionState::Terminated,
            Some(Command::PlayAnimFreeze),
            OrderType::TransitionParryingSwordWaitingSword,
        ));
        assert!(play_anim_freeze_completed(
            MotionState::Terminated,
            Some(Command::PlayAnimFreeze),
            OrderType::PlayCustomFreeze,
        ));
        assert!(!play_anim_freeze_completed(
            MotionState::InProgress,
            Some(Command::PlayAnimFreeze),
            OrderType::PlayCustomFreeze,
        ));
    }

    #[test]
    fn attentive_movement_uses_the_original_alerted_animation_family() {
        use OrderType as OT;

        let substitutions = [
            (OT::WalkingUpright, OT::WalkingAlerted),
            (
                OT::TransitionWalkingUprightWaitingUpright,
                OT::TransitionWalkingAlertedWaitingAlerted,
            ),
            (
                OT::TransitionRunningUprightWaitingUpright,
                OT::TransitionRunningAlertedWaitingAlerted,
            ),
            (
                OT::TransitionWaitingUprightWalkingUpright,
                OT::TransitionWaitingAlertedWalkingAlerted,
            ),
            (
                OT::TransitionWaitingUprightRunningUpright,
                OT::TransitionWaitingAlertedRunningAlerted,
            ),
            (
                OT::TransitionWalkingUprightRunningUpright,
                OT::TransitionWalkingAlertedRunningAlerted,
            ),
            (
                OT::TransitionRunningUprightWalkingUpright,
                OT::TransitionRunningAlertedWalkingAlerted,
            ),
            (OT::WalkingStairs, OT::WalkingStairsAlerted),
            (OT::Turning, OT::TurningAlerted),
        ];

        for (authored, effective) in substitutions {
            assert_eq!(
                soldier_movement_animation(authored, true, ActionState::Waiting),
                effective
            );
        }
        assert_eq!(
            soldier_movement_animation(OT::WalkingUpright, true, ActionState::MovingSword),
            OT::WalkingAlerted,
            "the sword-state invariant must not change attentive-branch behavior"
        );
        assert_eq!(
            soldier_movement_animation(OT::WalkingStairs, true, ActionState::MovingFastSword),
            OT::WalkingStairs
        );
        assert_eq!(
            soldier_movement_animation(
                OT::TransitionWaitingUprightWalkingUpright,
                true,
                ActionState::MovingSword,
            ),
            OT::TransitionWaitingAlertedWalkingAlerted,
            "upright transition substitutions only test mbAttentive"
        );
        assert_eq!(
            soldier_movement_animation(OT::WalkingUpright, false, ActionState::Waiting),
            OT::WalkingUpright
        );
    }

    #[test]
    fn npc_taking_plays_searching_sprite_row_without_changing_pc_taking() {
        let sprite = crate::sprite::Sprite::default();

        assert_eq!(
            sprite_anim_for_order(&sprite, OrderType::Taking, false),
            OrderType::Searching
        );
        assert_eq!(
            sprite_anim_for_order(&sprite, OrderType::Taking, true),
            OrderType::Taking
        );
    }

    #[test]
    fn promoted_npc_pc_uses_authored_fallbacks_for_target_interactions() {
        use crate::sprite_script::UNMAPPED;

        let mut sprite = crate::sprite::Sprite::default();
        let mut conversion = vec![UNMAPPED; OrderType::HittingTarget as usize + 1];
        conversion[OrderType::Hitting as usize] = 0;
        conversion[OrderType::StrikingDownSword as usize] = 16;
        sprite.conversion = std::sync::Arc::new(conversion);

        assert_eq!(
            sprite_anim_for_order(&sprite, OrderType::HittingTarget, true),
            OrderType::StrikingDownSword
        );
        assert_eq!(
            sprite_anim_for_order(&sprite, OrderType::HandlingTarget, true),
            OrderType::Hitting
        );
    }

    #[test]
    fn lying_under_net_hold_freezes_the_first_wriggle_frame() {
        let sprite = crate::sprite::Sprite::default();

        assert_eq!(
            default_actor_sprite_playback(
                &sprite,
                OrderType::LyingStuckUnderNet,
                OrderType::LyingStuckUnderNet,
                false,
            ),
            (
                OrderType::WriggleUnderNet,
                FrameProgression::FrozenFirstFrame,
            )
        );
    }

    #[test]
    fn taking_initial_facing_uses_the_owner_specific_original_projection() {
        let from = crate::coordinates::MapPoint::new(863.8749, 702.40265);
        let to = crate::coordinates::MapPoint::new(846.728, 693.8904);

        assert_eq!(
            taking_initial_direction(false, true, OrderType::Taking, from, to),
            Some(14),
            "soldier Taking uses aspect-corrected direction-sector selection on first execution"
        );
        assert_eq!(
            taking_initial_direction(true, true, OrderType::Taking, from, to),
            Some(13),
            "PC Taking retains its non-isometric direction-sector selection"
        );
        assert_eq!(
            taking_initial_direction(true, true, OrderType::TakingCrouched, from, to),
            Some(13)
        );
        assert_eq!(
            taking_initial_direction(false, true, OrderType::DrinkingAle, from, to),
            None,
            "soldier DrinkAle only turns toward its existing goal"
        );
        assert_eq!(
            taking_initial_direction(false, false, OrderType::Taking, from, to),
            None,
            "Taking refreshes its goal only during order initialization"
        );
    }

    #[test]
    fn beggar_show_face_falls_back_to_rolling_only_when_the_animation_is_missing() {
        use crate::sprite_script::UNMAPPED;

        let missing = crate::sprite::Sprite::default();
        assert_eq!(
            sprite_anim_for_order(&missing, OrderType::BeggarShowingFace, false),
            OrderType::Rolling
        );

        let mut available = crate::sprite::Sprite::default();
        let mut conversion = vec![UNMAPPED; OrderType::BeggarShowingFace as usize + 1];
        conversion[OrderType::BeggarShowingFace as usize] = 0;
        available.conversion = std::sync::Arc::new(conversion);
        assert_eq!(
            sprite_anim_for_order(&available, OrderType::BeggarShowingFace, false),
            OrderType::BeggarShowingFace
        );
    }
}

#[cfg(test)]
mod shoulder_idle_initialization_tests {
    use super::*;
    use crate::element::{ActorPc, ElementData, ElementKind, Entity, HumanData, PcData, Posture};
    use crate::order::Order;
    use crate::sequence::{SequenceElement, SequencePriority};
    use crate::sprite_script::SpriteScript;

    #[test]
    fn waiting_carrying_on_shoulders_initialization_idles_carried_once() {
        let sim = crate::sim_rng::test_context();
        let assets = crate::engine::types::LevelAssets::new();
        let mut engine = EngineInner::new();

        let mut helper = Entity::Pc(ActorPc {
            element: {
                let mut initial_element =
                    ElementData::from_initial_posture(Posture::CarryingOnShoulders);
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[OrderType::WaitingCarryingOnShoulders as usize] = 0;
        let script = SpriteScript {
            action_id: OrderType::WaitingCarryingOnShoulders as u16,
            action_done: 0,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        helper.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script]),
            std::sync::Arc::new(conversion),
        );
        let helper_id = engine.add_test_entity(helper);
        let climber_id = engine.add_test_entity(Entity::Pc(ActorPc {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::OnShoulders);
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        }));
        engine
            .get_entity_mut(helper_id)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .carried = Some(climber_id);

        let mut wait = SequenceElement::new(1, Command::Wait, Some(helper_id));
        wait.priority = SequencePriority::Wait;
        wait.orders.push_back(Order::test_new(
            OrderType::WaitingCarryingOnShoulders,
            0.0,
            0.0,
        ));
        let helper_wait = engine.orders.sequence_manager.insert_element(wait);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(helper_wait);
        engine.select_sequence_element(helper_id, Some((helper_wait, 0)));
        engine.element_in_progress(
            &crate::sim_rng::test_context(),
            &assets,
            &mut Vec::new(),
            helper_wait,
            0,
        );

        engine.tick_actor_animation_for(&sim, &assets, helper_id);

        let (_, _, order) = engine
            .orders
            .sequence_manager
            .current_order_for_actor(&engine.world.entities, climber_id)
            .expect("helper idle initialization must wake the carried PC");
        assert_eq!(order.order_type, OrderType::WaitingOnShoulders);
        let climber = engine.get_entity(climber_id).unwrap().actor_data().unwrap();
        assert_eq!(
            engine
                .actor_installed_order(climber_id)
                .map(|order| order.order_type),
            Some(OrderType::WaitingOnShoulders)
        );
        assert_eq!(climber.continuation.motion_state, MotionState::InProgress);

        engine
            .get_entity_mut(helper_id)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .execute_order_initialising = false;
        let carried_order = engine
            .orders
            .sequence_manager
            .current_order_for_actor(&engine.world.entities, climber_id)
            .expect("carried wait must remain selected")
            .2
            .order_id;
        engine.tick_actor_animation_for(&sim, &assets, helper_id);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_order_for_actor(&engine.world.entities, climber_id)
                .expect("carried wait must remain selected")
                .2
                .order_id,
            carried_order,
            "the carried Wait is an initialization-only side effect"
        );
    }

    #[test]
    fn waiting_on_shoulders_selects_row_from_live_carrier_direction() {
        let sim = crate::sim_rng::test_context();
        let assets = crate::engine::types::LevelAssets::new();
        let mut engine = EngineInner::new();

        let mut helper = ActorPc {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        helper.element.set_direction_instantly(4);
        let helper_id = engine.add_test_entity(Entity::Pc(helper));

        let mut climber = ActorPc {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::OnShoulders);
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        climber.human.carrier = Some(helper_id);
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[OrderType::WaitingOnShoulders as usize] = 100;
        let script = SpriteScript {
            action_id: OrderType::WaitingOnShoulders as u16,
            action_done: 0,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let scripts = vec![script; 116];
        climber.element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(scripts),
            std::sync::Arc::new(conversion),
        );
        let climber_id = engine.add_test_entity(Entity::Pc(climber));

        let mut wait = SequenceElement::new(1, Command::Wait, Some(climber_id));
        wait.orders
            .push_back(Order::test_new(OrderType::WaitingOnShoulders, 0.0, 0.0));
        let wait_id = engine.orders.sequence_manager.insert_element(wait);
        engine.orders.sequence_manager.start_sequence_level(wait_id);
        engine.select_sequence_element(climber_id, Some((wait_id, 0)));
        engine.element_in_progress(
            &crate::sim_rng::test_context(),
            &assets,
            &mut Vec::new(),
            wait_id,
            0,
        );

        engine.tick_actor_animation_for(&sim, &assets, climber_id);

        let climber = engine.get_entity(climber_id).unwrap().element_data();
        assert_eq!(climber.direction(), 12);
        assert_eq!(climber.sprite.current_row, 112);
    }
}
