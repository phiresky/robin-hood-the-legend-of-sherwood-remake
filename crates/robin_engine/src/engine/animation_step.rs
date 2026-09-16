//! Direct execution of selected animation orders. Owner borrows end before
//! synchronous callbacks; the actor update handles the returned completion.

use super::*;
use std::ops::ControlFlow;

/// Freeze and diagnostic facts sampled at Execute entry.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(super) struct ActorAnimationEntry {
    pub(super) globally_frozen: bool,
    pub(super) diagnostic_frame: u32,
    pub(super) diagnostic_creation_order: Option<u32>,
    pub(super) sprite_row_diagnostic: bool,
    pub(super) tiredness_probe: Option<(u32, u32)>,
}

/// Non-sprite operands retained at the selected Execute entry.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(super) struct ActorAnimationOperands {
    pub(super) principal_frames_from_now: Option<i16>,
    pub(super) door_pass_crenel_transition_dir: Option<i16>,
    pub(super) validated_antagonist: Option<EntityId>,
    pub(super) waiting_sword_direction_goal: Option<i16>,
    pub(super) standing_up_sword_direction_goal: Option<i16>,
    pub(super) extracting_arrow_sword_direction_goal: Option<i16>,
    pub(super) striking_down_sword_direction_goal: Option<i16>,
    pub(super) taking_direction_goal: Option<i16>,
    pub(super) pc_target_direction_goal: Option<i16>,
    pub(super) waiting_on_shoulders_direction: Option<i16>,
    pub(super) taking_net_order_was_done: bool,
}

/// Combat-facing and interaction-facing goals derived from live opponents.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct ActorAnimationFacingGoals {
    waiting_sword_direction: Option<i16>,
    standing_up_sword_direction: Option<i16>,
    extracting_arrow_sword_direction: Option<i16>,
    taking_direction: Option<i16>,
    target_direction: Option<i16>,
    waiting_on_shoulders_direction: Option<i16>,
}

impl EngineInner {
    /// Stamp the order-initialisation edge and short-circuit a per-actor
    /// execution freeze.
    pub(super) fn actor_animation_entry(
        &mut self,
        entity_id: EntityId,
    ) -> ControlFlow<Option<MotionState>, ActorAnimationEntry> {
        let globally_frozen = self.actors_frozen();
        let diagnostic_frame = self.control.frame_counter;
        let diagnostic_creation_order =
            crate::sprite::sprite_row_diagnostic_creation_order(diagnostic_frame, || {
                self.world.original_creation_order(entity_id)
            });
        let sprite_row_diagnostic = diagnostic_creation_order.is_some();
        let tiredness_probe = crate::combat::tiredness_debug_enabled()
            .then(|| self.world.original_creation_order(entity_id))
            .filter(|co| crate::combat::tiredness_debug_matches(*co))
            .map(|co| (diagnostic_frame, co));

        // Production enters through the owner coordinator, which stamps this
        // before choosing a specialized arm. Keep this helper self-contained
        // for focused callers while preserving the same actor-level identity.
        if let Some((_, _, order)) = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, entity_id)
        {
            let actor = self
                .world
                .entities
                .get_mut(entity_id)
                .and_then(Entity::actor_data_mut)
                .unwrap_or_else(|| panic!("generic Execute owner {entity_id:?} lost actor data"));
            if actor.last_execute_order_id != Some(order.order_id) {
                actor.last_execute_order_id = Some(order.order_id);
                actor.execute_order_initialising = true;
            }
        }

        // Actor execution returns IN_PROGRESS immediately for a
        // per-actor execution freeze. The actor update still applies its
        // WAIT_TIMER / WAIT_FREE_LIFT modifier to that return value, so retain
        // the selected identity for those two commands without entering any
        // Execute arm or touching the sprite.
        if let Some(actor) = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            && actor.execution_frozen
        {
            let frozen_wait = self.orders.sequence_manager.current_order_for_actor(&self.world.entities, entity_id)
                .and_then(|(seq_id, elem_idx, _)| {
                    let element = self
                        .orders
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .unwrap_or_else(|| {
                            panic!(
                                "execution-frozen actor {entity_id:?} selected missing element {seq_id:?}/{elem_idx}"
                            )
                        });
                    matches!(element.command, Command::WaitTimer | Command::WaitFreeLift)
                        .then_some(MotionState::InProgress)
                });
            return ControlFlow::Break(frozen_wait);
        }
        ControlFlow::Continue(ActorAnimationEntry {
            globally_frozen,
            diagnostic_frame,
            diagnostic_creation_order,
            sprite_row_diagnostic,
            tiredness_probe,
        })
    }

    /// Whether the selected order belongs to the generic Execute switch
    /// rather than the movement driver.
    pub(super) fn actor_animation_selects_generic_order(&self, entity_id: EntityId) -> bool {
        let selected_generic_order = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, entity_id)
            .and_then(|(seq_id, elem_idx, order)| {
                self.orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .map(|element| {
                        !element.data.is_movement()
                            || order.order_type == OrderType::Select
                            || matches!(element.command, Command::WaitTimer | Command::WaitFreeLift)
                            // A movement element's translated order chain also
                            // holds pure animation steps — the door-pass Turning
                            // and crouch transitions, and the FREEZING token a
                            // pathfinding move carries. Those belong to the
                            // generic Execute switch even though the element
                            // itself is a movement element, so the actor's
                            // stale moving action-state must not suppress them.
                            || super::tick::classify_live_actor_execute_arm(
                                entity_id,
                                order.order_type,
                            ) == Some(super::tick::ExecuteOwnerFamily::GenericAnimation)
                    })
            })
            .unwrap_or(false);
        selected_generic_order
    }

    /// Admission gates plus the antagonist, door-pass and facing operands.
    /// `None` means Execute does not run this tick.
    pub(super) fn actor_animation_operands(
        &self,
        entity_id: EntityId,
        selected_generic_order: bool,
    ) -> Option<ActorAnimationOperands> {
        let entity = self.world.entities.get(entity_id).unwrap_or_else(|| {
            panic!(
                "actor animation creation slot {} lost entity {entity_id:?}",
                entity_id.index()
            )
        });
        let actor = entity.actor_data().unwrap_or_else(|| {
            panic!(
                "actor animation creation slot {} resolved non-actor {entity_id:?}",
                entity_id.index()
            )
        });
        if (actor.action_state.is_moving()
            || matches!(
                actor.action_state,
                crate::element::ActionState::MovingSword
                    | crate::element::ActionState::MovingFastSword
                    | crate::element::ActionState::MovingShield
            ))
            && !selected_generic_order
        {
            return None;
        }

        let Some((seq_id, elem_idx, order)) = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, entity_id)
        else {
            return None;
        };
        let exact_selected_bow = self
            .selected_bow_order(entity_id)
            .is_some_and(|(sequence, index, _)| (sequence, index) == (seq_id, elem_idx));
        if exact_selected_bow {
            return None;
        }
        let anim_type = order.order_type;
        let principal_frames = if anim_type == OrderType::TransitionWaitingSwordParryingSwordLow {
            entity
                    .human_data()
                    .and_then(|human| human.opponents.first().copied())
                    .and_then(|opponent| {
                        let opponent_entity =
                            self.world.entities.get(opponent).unwrap_or_else(|| {
                                panic!(
                                    "actor {entity_id:?} low-parry opponent {opponent:?} is missing at legacy slot {}",
                                    entity_id.index()
                                )
                            });
                        safe_frames_from_now_till_action_done(
                            &opponent_entity.element_data().sprite,
                        )
                    })
        } else {
            None
        };

        let anim_uses_antagonist = matches!(
            anim_type,
            OrderType::DrinkingAle
                | OrderType::Taking
                | OrderType::TakingCrouched
                | OrderType::Searching
                | OrderType::TakingNet
                | OrderType::WakingUp
                | OrderType::HittingTarget
                | OrderType::HandlingTarget
                | OrderType::TakingTarget
                | OrderType::UsingLever
                | OrderType::StrikingDownSword
        );
        let anim_requires_antagonist =
            anim_uses_antagonist && (anim_type != OrderType::Searching || entity.is_pc());
        let validated_antagonist = if anim_requires_antagonist {
            order.antagonist.or_else(|| {
                panic!(
                    "actor {entity_id:?} {anim_type:?} requires antagonist at legacy slot {}",
                    entity_id.index()
                )
            })
        } else if anim_uses_antagonist {
            order.antagonist
        } else {
            None
        };
        if let Some(antagonist) = validated_antagonist {
            self.expect_entity(antagonist, "animation antagonist");
        }

        let striking_down_sword_direction = if anim_type == OrderType::StrikingDownSword {
            let antagonist_id =
                validated_antagonist.expect("StrikingDownSword requires a validated antagonist");
            let antagonist = self.world.entities.get(antagonist_id).unwrap_or_else(|| {
                        panic!(
                            "actor {entity_id:?} StrikingDownSword antagonist {antagonist_id:?} is missing at legacy slot {}",
                            entity_id.index()
                        )
                    });
            Some(striking_down_sword_direction(entity, antagonist))
        } else {
            None
        };

        let door_direction = if matches!(
            anim_type,
            OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
        ) && actor.execute_order_initialising
        {
            let door_index = entity.position_iface().get_door().unwrap_or_else(|| {
                    panic!(
                        "actor {entity_id:?} {anim_type:?} lacks required active door pass at legacy slot {}",
                        entity_id.index()
                    )
                });
            let reverse_direction =
                anim_type == OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel;
            let door = self
                    .script_domains
                    .interactables
                    .doors
                    .get(usize::from(door_index))
                    .unwrap_or_else(|| {
                        panic!(
                            "actor {entity_id:?} crenel transition references missing door {} at legacy slot {}",
                            door_index,
                            entity_id.index()
                        )
                    });
            let sector_number = crate::sector::SectorNumber::new(i16::from(door.sector_in));
            let sector_index = self
                    .world
                    .fast_grid
                    .level
                    .sector_number_map
                    .get(&sector_number)
                    .copied()
                    .unwrap_or_else(|| {
                        panic!(
                            "actor {entity_id:?} crenel door {} references missing sector {sector_number:?} at legacy slot {}",
                            door_index,
                            entity_id.index()
                        )
                    });
            let sector = self
                    .world
                    .fast_grid
                    .level
                    .sectors
                    .get(sector_index)
                    .unwrap_or_else(|| {
                        panic!(
                            "actor {entity_id:?} crenel door {} resolved invalid sector index {sector_index} at legacy slot {}",
                            door_index,
                            entity_id.index()
                        )
                    });
            if sector.lift_type != Some(crate::sector::LiftType::Wall) {
                panic!(
                    "actor {entity_id:?} crenel door {} requires wall-lift sector {sector_number:?}, found {:?}",
                    door_index, sector.lift_type
                );
            }
            Some(if reverse_direction {
                (sector.lift_direction + 8) & 15
            } else {
                sector.lift_direction
            })
        } else {
            None
        };

        let ActorAnimationFacingGoals {
            waiting_sword_direction,
            standing_up_sword_direction,
            extracting_arrow_sword_direction,
            taking_direction,
            target_direction,
            waiting_on_shoulders_direction,
        } = self.actor_animation_facing_goals(
            entity_id,
            entity,
            actor,
            order,
            anim_type,
            validated_antagonist,
        );

        Some(ActorAnimationOperands {
            principal_frames_from_now: principal_frames,
            door_pass_crenel_transition_dir: door_direction,
            validated_antagonist,
            waiting_sword_direction_goal: waiting_sword_direction,
            standing_up_sword_direction_goal: standing_up_sword_direction,
            extracting_arrow_sword_direction_goal: extracting_arrow_sword_direction,
            striking_down_sword_direction_goal: striking_down_sword_direction,
            taking_direction_goal: taking_direction,
            pc_target_direction_goal: target_direction,
            waiting_on_shoulders_direction,
            taking_net_order_was_done: order.done,
        })
    }

    fn actor_animation_facing_goals(
        &self,
        entity_id: EntityId,
        entity: &Entity,
        actor: &crate::element::ActorData,
        order: &crate::order::Order,
        anim_type: OrderType,
        validated_antagonist: Option<EntityId>,
    ) -> ActorAnimationFacingGoals {
        // Human-actor execution refreshes combat-facing goals
        // before turning and action processing. Sword waiting, normal parry,
        // and smalltalk parries face the live principal opponent;
        // smalltalk strikes face their order antagonist. Normal low parry
        // does not share either arm. A stale reference is an invariant
        // failure here: the Original dereferences it directly.
        let is_swordfighting = entity
            .human_data()
            .is_some_and(|human| !human.opponents.is_empty());
        let facing_opponent = if matches!(
            anim_type,
            OrderType::WaitingSword
                | OrderType::ParryingSword
                | OrderType::ParryingLeftSmalltalk
                | OrderType::ParryingRightSmalltalk
                | OrderType::ParryingLowLeftSmalltalk
                | OrderType::ParryingLowRightSmalltalk
        ) {
            entity
                .human_data()
                .and_then(|human| human.opponents.first().copied())
        } else if is_swordfighting
            && matches!(
                anim_type,
                OrderType::StrikingLeftSmalltalk
                    | OrderType::StrikingRightSmalltalk
                    | OrderType::StrikingLowLeftSmalltalk
                    | OrderType::StrikingLowRightSmalltalk
            )
        {
            order.antagonist
        } else if anim_type == OrderType::TransitionRaisingSword && actor.execute_order_initialising
        {
            order.antagonist
        } else {
            None
        };
        let waiting_sword_direction = facing_opponent.map(|opponent_id| {
                let opponent = self.world.entities.get(opponent_id).unwrap_or_else(|| {
                    panic!(
                        "actor {entity_id:?} {anim_type:?} opponent {opponent_id:?} is missing at legacy slot {}",
                        entity_id.index()
                    )
                });
                if anim_type == OrderType::TransitionRaisingSword {
                    raising_sword_direction(entity, opponent)
                } else {
                    let from = entity.element_data().position();
                    let to = opponent.element_data().position();
                    crate::position_interface::vector_to_sector_0_to_15_iso(
                        to.x - from.x,
                        to.y - from.y,
                    )
                }
            });

        // Unlike the other combat-facing arms above, Original's human
        // STANDING_UP_SWORD arm plays the sprite first and only then
        // refreshes the goal and turns. Compute the live principal here,
        // but defer both mutations until after action processing. The soldier
        // override has no corresponding facing or Turn step.
        let standing_up_sword_direction = if anim_type == OrderType::StandingUpSword
            && !entity.is_soldier()
            && is_swordfighting
        {
            let opponent_id = entity
                .human_data()
                .and_then(|human| human.opponents.first().copied())
                .expect("swordfighting stand-up actor has no principal opponent");
            let opponent = self.world.entities.get(opponent_id).unwrap_or_else(|| {
                    panic!(
                        "actor {entity_id:?} standing-up opponent {opponent_id:?} is missing at legacy slot {}",
                        entity_id.index()
                    )
                });
            let from = entity.element_data().position();
            let to = opponent.element_data().position();
            Some(crate::position_interface::vector_to_sector_0_to_15_iso(
                to.x - from.x,
                to.y - from.y,
            ))
        } else {
            None
        };

        // Sword-arrow extraction only
        // refreshes the goal on the order's initialization tick, then
        // calls Turn() on every tick while the actor remains in a
        // swordfight.  The arrow impact may have just snapped the body
        // toward the projectile, so retaining that direction as the goal
        // makes the extraction animation face away from the live duel.
        let extracting_arrow_sword_direction = if actor.execute_order_initialising
            && anim_type == OrderType::ExtractingArrowSword
            && is_swordfighting
        {
            let opponent_id = entity
                .human_data()
                .and_then(|human| human.opponents.first().copied())
                .expect("swordfighting arrow-damage actor has no principal opponent");
            let opponent = self.world.entities.get(opponent_id).unwrap_or_else(|| {
                    panic!(
                        "actor {entity_id:?} extracting-arrow opponent {opponent_id:?} is missing at legacy slot {}",
                        entity_id.index()
                    )
                });
            let from = entity.element_data().position();
            let to = opponent.element_data().position();
            Some(crate::position_interface::vector_to_sector_0_to_15_iso(
                to.x - from.x,
                to.y - from.y,
            ))
        } else {
            None
        };

        // The original game initializes live facing on the first execution tick,
        // after translation has deliberately preserved the previous goal.
        // PC Taking/TakingCrouched uses the ordinary 2-D sector, while the
        // soldier Taking override passes ASPECT_RATIO. DrinkAle merely
        // calls Turn() and must not synthesize a new direction.
        let taking_direction = validated_antagonist.and_then(|antagonist_id| {
                let antagonist = self.world.entities.get(antagonist_id).unwrap_or_else(|| {
                    panic!(
                        "actor {entity_id:?} {anim_type:?} antagonist {antagonist_id:?} is missing at legacy slot {}",
                        entity_id.index()
                    )
                });
                taking_initial_direction(
                    entity.is_pc(),
                    actor.execute_order_initialising,
                    anim_type,
                    entity.element_data().position_map(),
                    antagonist.element_data().position_map(),
                )
            });
        // HIT_TARGET / HANDLE_TARGET use the target sprite row's live
        // action hotspot, not the target's interaction map position. The
        // original game intentionally selects the direction sector without the
        // isometric aspect argument for this screen-space vector.
        let target_direction = if entity.is_pc()
            && actor.execute_order_initialising
            && matches!(
                anim_type,
                OrderType::HittingTarget | OrderType::HandlingTarget
            ) {
            validated_antagonist.map(|antagonist_id| {
                let antagonist = self.world.entities.get(antagonist_id).unwrap_or_else(|| {
                    panic!(
                        "actor {entity_id:?} {anim_type:?} antagonist \
                                 {antagonist_id:?} is missing at legacy slot {}",
                        entity_id.index()
                    )
                });
                let from = entity.element_data().position_map();
                let to = antagonist.current_gameplay_point_map().unwrap_or_else(|| {
                    panic!(
                        "actor {entity_id:?} {anim_type:?} target {antagonist_id:?} \
                             has no current sprite hotspot"
                    )
                });
                crate::position_interface::vector_to_sector_0_to_15(to.x - from.x, to.y - from.y)
            })
        } else {
            None
        };
        let waiting_on_shoulders_direction =
                (anim_type == OrderType::WaitingOnShoulders).then(|| {
                    let carrier_id = entity
                        .human_data()
                        .and_then(|human| human.carrier)
                        .unwrap_or_else(|| {
                            panic!(
                                "WaitingOnShoulders owner {entity_id:?} has no carrier at legacy slot {}",
                                entity_id.index()
                            )
                        });
                    let carrier = self.world.entities.get(carrier_id).unwrap_or_else(|| {
                        panic!(
                            "WaitingOnShoulders owner {entity_id:?} references missing carrier {carrier_id:?}"
                        )
                    });
                    (carrier.element_data().direction() + 8) & 15
                });

        ActorAnimationFacingGoals {
            waiting_sword_direction,
            standing_up_sword_direction,
            extracting_arrow_sword_direction,
            taking_direction,
            target_direction,
            waiting_on_shoulders_direction,
        }
    }
}

impl EngineInner {
    /// Execute the selected animation with owner borrows ending at each callback.
    /// Order identity and explicit entry operands survive synchronous callbacks.
    pub(super) fn execute_actor_animation(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        selected_generic_order: bool,
        entry: ActorAnimationEntry,
        operands: ActorAnimationOperands,
    ) -> Option<MotionState> {
        let frame_counter = self.control.frame_counter;
        let reusable_cloaks_enabled = self.control.sim_config.reusable_cloaks;

        let entity: &Entity = self
            .world
            .entities
            .get(entity_id)
            .expect("animation owner disappeared");
        let validated_antagonist = operands.validated_antagonist;
        let actor = entity
            .actor_data()
            .expect("actor animation step requires actor data");
        // Moving actors are animated in tick_entity_movement(sim, ),
        // which computes per-frame combat directional anims
        // (WalkingSword / StrafingRightSword / …) for the
        // sword/shield variants rather than using the Move
        // element's order.action (which is the logical
        // `WalkingWithSword`, unmapped in PC sprite profiles).
        // Keep this gate aligned with tick_entity_movement's
        // own is_moving / sword / shield guard
        // (movement.rs:2210).
        if (actor.action_state.is_moving()
            || matches!(
                actor.action_state,
                crate::element::ActionState::MovingSword
                    | crate::element::ActionState::MovingFastSword
                    | crate::element::ActionState::MovingShield
            ))
            && !selected_generic_order
        {
            return None;
        }

        // Exact selected melee and bow arms are admitted by the live
        // owner coordinator/current-order check. Stale background
        // state must not suppress the actual selected generic arm.

        // Read the actor's current in-progress sequence element
        // and its front order.  All animation driving flows off
        // this — dispatch is on the current element's front
        // order. The selected identity survives synchronous owner callbacks.
        let (seq_id, elem_idx, order) = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, entity_id)?;
        let anim_type = order.order_type;
        let order_id = Some(order.order_id);
        let order_antagonist = order.antagonist;
        let unlock_door = match order.completion {
            crate::order::OrderCompletion::UnlockDoor { door_id } => Some(door_id),
            _ => None,
        };
        let order_tolerance = order.tolerance;
        let order_target = crate::coordinates::MapPoint::new(order.target_x, order.target_y);
        if self
            .selected_bow_order(entity_id)
            .is_some_and(|(sequence, index, _)| (sequence, index) == (seq_id, elem_idx))
        {
            return None;
        }
        let antagonist = validated_antagonist.or(order_antagonist);
        let element = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("selected animation element disappeared");
        let cur_command = Some(element.command);
        let cur_command_level = element.command_level;
        let current_element_script_driven = element.script_driven;
        let selected_order_is_custom_animation = is_custom_animation_order(anim_type);
        let requested_custom_animation = if selected_order_is_custom_animation
            && matches!(
                element.command,
                Command::PlayAnim
                    | Command::PlayAnimLoop
                    | Command::PlayAnimFreeze
                    | Command::PlayAnimFrozen
            ) {
            match element.get_property(crate::sequence::Field::AnimationId) {
                Some(crate::sequence::FieldValue::Animation(animation)) => Some(*animation),
                Some(crate::sequence::FieldValue::Integer(value)) => {
                    OrderType::try_from(*value).ok()
                }
                _ => None,
            }
        } else {
            None
        };
        let pointing_direction_goal = if element.command == Command::Point {
            match element.get_property(crate::sequence::Field::Direction) {
                Some(crate::sequence::FieldValue::Integer(direction)) => Some(*direction as i16),
                _ => panic!("Point sequence is missing its integer Direction property"),
            }
        } else {
            None
        };
        self.prepare_jump_order(sim, assets, entity_id);
        let owner = self.expect_entity(entity_id, "animation owner");
        if owner.is_soldier() {
            match anim_type {
                OrderType::WaitingUpright if owner.enemy_ai().is_some() => {
                    self.execute_waiting_upright(sim, assets, entity_id);
                }
                OrderType::WaitingAlerted => {
                    self.execute_waiting_alerted(sim, assets, entity_id);
                }
                _ => {}
            }
        }

        let ActorAnimationOperands {
            door_pass_crenel_transition_dir,
            waiting_sword_direction_goal,
            extracting_arrow_sword_direction_goal,
            taking_direction_goal,
            pc_target_direction_goal,
            waiting_on_shoulders_direction,
            ..
        } = operands;
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("animation owner disappeared");
        let actor = entity
            .actor_data()
            .expect("actor animation step requires actor data");
        tracing::trace!(
            entity = entity_id.index(),
            ?anim_type,
            order_id,
            ?cur_command,
            "animation: driving order"
        );
        // The soldier dispatch swaps many animations for
        // an "alerted" variant when attentive is set.  For
        // TURNING specifically the completion flag is
        // driven by `turn_fast()` / `turn()` rather than
        // the sprite's `action_done_frame` — the sprite is
        // just played for the visual, while the body
        // rotates progressively toward the direction goal.
        let is_turn = matches!(anim_type, OrderType::Turning);
        let effective_anim = if soldier_is_attentive(entity) {
            alerted_variant(anim_type).unwrap_or(anim_type)
        } else {
            anim_type
        };
        // The civilian dispatch coerces the entire
        // `WAITING_UPRIGHT` family to
        // `WAITING_UPRIGHT_BORED` for civilians, so a
        // civilian never plays the plain upright wait or
        // its get-up / random variants — they stay in the
        // bored idle loop.  This sits after the
        // soldier-attentive remap so it can't be
        // re-overridden.
        let effective_anim = if entity.is_civilian()
            && matches!(
                effective_anim,
                OrderType::WaitingUpright
                    | OrderType::WaitingUprightBored
                    | OrderType::WaitingUprightBoredRandom
                    | OrderType::TransitionWaitingUprightBoredWaitingUpright
                    | OrderType::TransitionWaitingUprightWaitingUprightBored,
            ) {
            OrderType::WaitingUprightBored
        } else {
            effective_anim
        };
        // crouched ale-dropping is only a
        // dispatch token. The PC override plays the authored
        // TAKING_CROUCHED sprite while retaining the drop order
        // for command/state and DONE-side-effect handling.
        let effective_anim = if effective_anim == OrderType::DroppingAleCrouched {
            OrderType::TakingCrouched
        } else {
            effective_anim
        };
        let owner_is_pc = entity.is_pc();
        // Jump steps whose execution arm processes motion rather
        // than action processing: they approach an authored map point
        // while their transition animation plays.
        let actor_in_jump = cur_command == Some(Command::JumpCmd);
        let jump_ground_motion_step =
            super::jump::jump_step_uses_perform_motion(anim_type) && actor_in_jump;
        // Airborne segments fly the body themselves and ignore
        // what their animation reports, so they take their own
        // Execute path below.
        let jump_airborne_step = actor_in_jump && super::jump::jump_order_is_airborne(anim_type);
        let order_is_initialising = actor.execute_order_initialising;
        if owner_is_pc && anim_type == OrderType::WaitingWithCorpse && order_is_initialising {
            let carried = entity
                .pc_data()
                .and_then(|pc| pc.carried)
                .expect("waiting corpse carrier has no body");
            self.actor_freeze_execution(sim, assets, carried);
        }
        let entity = self.expect_entity(entity_id, "animation owner");
        if order_is_initialising
            && anim_type == OrderType::WaitingCarryingOnShoulders
            && let Some(carried_id) = entity.pc_data().and_then(|pc| pc.carried)
        {
            self.actor_wait(sim, assets, carried_id);
        }
        if anim_type == OrderType::TransitionHelpingClimbingDown {
            let carried = self
                .expect_entity(entity_id, "shoulder dismount helper")
                .pc_data()
                .and_then(|pc| pc.carried);
            if let Some(carried) = carried {
                if order_is_initialising {
                    self.actor_freeze_execution(sim, assets, carried);
                    self.install_actor_order(carried, None);
                }
                let direction = (self
                    .expect_entity(entity_id, "shoulder helper")
                    .element_data()
                    .direction()
                    + 8)
                    & 15;
                self.world
                    .entities
                    .get_mut(carried)
                    .expect("shoulder rider disappeared")
                    .element_data_mut()
                    .set_direction_goal(direction);
            }
        }
        if anim_type == OrderType::WaitingCarryingOnShoulders {
            let carrier = self.expect_entity(entity_id, "waiting shoulder carrier");
            if let Some(carried) = carrier.pc_data().and_then(|pc| pc.carried) {
                let direction = (carrier.element_data().direction() + 8) & 15;
                self.world
                    .entities
                    .get_mut(carried)
                    .expect("waiting shoulder rider disappeared")
                    .element_data_mut()
                    .set_direction_instantly(direction);
            }
        }
        let mut entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("animation owner disappeared");
        if anim_type == OrderType::TransitionHelpingClimbingDown
            && entity.pc_data().is_some_and(|pc| pc.carried.is_some())
        {
            // The transition sets the helper's
            // states before playing the lowering animation.
            self.set_entity_posture(entity_id, crate::element::Posture::HelpingToClimb);
            entity = self.expect_entity_mut(entity_id, "animation owner");
            entity
                .actor_data_mut()
                .expect("PC has actor data")
                .action_state = crate::element::ActionState::Waiting;
        }
        if let Some(direction) = waiting_sword_direction_goal {
            entity.element_data_mut().set_direction_goal(direction);
        }
        if let Some(direction) = extracting_arrow_sword_direction_goal {
            entity.element_data_mut().set_direction_goal(direction);
        }
        if let Some(direction) = taking_direction_goal {
            entity.element_data_mut().set_direction_goal(direction);
        }
        if let Some(direction) = pc_target_direction_goal {
            entity.element_data_mut().set_direction_goal(direction);
        }
        if let Some(direction) = waiting_on_shoulders_direction {
            // Player-character waiting on shoulders
            // snaps to the carrier's reversed live direction
            // before action processing selects the directional row.
            entity.element_data_mut().set_direction_instantly(direction);
        }
        if order_is_initialising
            && anim_type == OrderType::Pointing
            && let Some(direction) = pointing_direction_goal
        {
            // The original game's point translation only books the order.
            // Its first Execute tick sets the progressive goal,
            // then Turn() advances one sector before sprite
            // playback.
            entity.element_data_mut().set_direction_goal(direction);
        }
        if order_is_initialising
            && matches!(
                anim_type,
                OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                    | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
            )
            && let Some(direction) = door_pass_crenel_transition_dir
        {
            entity.element_data_mut().set_direction_instantly(direction);
            self.set_entity_posture(entity_id, crate::element::Posture::Flying);
            entity = self.expect_entity_mut(entity_id, "animation owner");
        }
        let special_speech_id = if anim_type == OrderType::Special && entity.is_soldier() {
            Some(
                entity
                    .soldier_data()
                    .and_then(|soldier| {
                        assets
                            .profile_manager
                            .get_soldier(soldier.soldier_profile_index)
                    })
                    .map(|profile| profile.exclamation_id)
                    .unwrap_or_else(|| {
                        panic!("Special animation owner {entity_id:?} has no soldier profile")
                    }),
            )
        } else {
            None
        };
        let weak_stunned_action_before_perform =
            weak_stunned_start_action_before_perform(entity, anim_type, order_is_initialising);

        if weak_stunned_action_before_perform.is_some() {
            self.add_weak_stunned_combat(
                sim,
                assets,
                entity_id,
                anim_type == OrderType::BeingWeakSword,
            );
        }
        const SPEECH_ID_HELBARDMAN: u32 = 0x4c484453;
        if let Some(speech_id) = special_speech_id.filter(|id| *id != SPEECH_ID_HELBARDMAN) {
            self.execute_special_remark_at_sprite_point(sim, assets, entity_id, speech_id);
        }

        let mut weak_sword_held = false;
        let motion = if is_turn {
            {
                let globally_frozen = entry.globally_frozen;
                let entity = self
                    .world
                    .entities
                    .get_mut(entity_id)
                    .expect("animation owner disappeared");
                let direction_before_turn = entity.element_data().direction() as u16;
                let sprite_before_turn = effective_anim == OrderType::TurningAlerted;
                if !globally_frozen && sprite_before_turn {
                    entity.element_data_mut().sprite.perform_action(
                        sim,
                        order_id,
                        effective_anim,
                        direction_before_turn,
                        FrameProgression::Default,
                        false,
                    );
                }
                let still_turning = turn_with_provenance(
                    entity,
                    entity_id,
                    frame_counter,
                    if cur_command == Some(Command::TurnFast) {
                        "turn_order_fast"
                    } else {
                        "turn_order"
                    },
                    cur_command == Some(Command::TurnFast),
                );
                if !globally_frozen && !sprite_before_turn {
                    let direction_after_turn = entity.element_data().direction() as u16;
                    let row = actor_action_row(
                        anim_type,
                        effective_anim,
                        direction_before_turn,
                        direction_after_turn,
                    );
                    let sprite = &mut entity.element_data_mut().sprite;
                    let _ = sprite.perform_action(
                        sim,
                        order_id,
                        effective_anim,
                        row,
                        FrameProgression::Default,
                        false,
                    );
                }
                let authoritative_motion = if still_turning {
                    MotionState::InProgress
                } else {
                    MotionState::Terminated
                };
                // Action processing records its raw visual edge on the
                // sprite for the end-of-frame Done propagation pass.
                // TURNING is the exceptional arm whose
                // return value ignores that edge and is controlled
                // entirely by Turn(): leaving a visual Done here
                // marks the order complete even though the body just
                // rotated and must execute Turn once more next frame.
                entity.element_data_mut().sprite.last_motion_state = Some(authoritative_motion);
                // PI is the single source of truth for direction —
                // no sync needed now that `ElementData.direction`
                // is gone.
                // Swallow `sprite_motion` — its Done/Terminated
                // is driven by `action_done_frame` in sprite
                // data, but the control flow here ties
                // completion to `turn_fast()` instead.
                Some(authoritative_motion)
            }
        } else if anim_type == OrderType::Select {
            // SELECT is a real non-animation order in the
            // translated door chain. It starts the Human/PC hulk
            // effect and terminates in this owner slot without
            // dispatching a sprite animation.
            if order_is_initialising {
                self.execute_select_hulk((entity_id, order_tolerance));
            }
            Some(MotionState::Terminated)
        } else if matches!(anim_type, OrderType::DrinkingAle)
            && order_is_initialising
            && !self
                .expect_entity(
                    antagonist.expect("drinking actor has no bottle"),
                    "drinking bottle",
                )
                .is_active()
        {
            return Some(MotionState::Terminated);
        } else {
            // Human under-net initialization runs before
            // action processing in the original game. Wriggling may rotate the
            // actor, and the new direction selects this tick's row.
            if order_is_initialising
                && matches!(
                    anim_type,
                    OrderType::LyingStuckUnderNet | OrderType::WriggleUnderNet
                )
            {
                apply_under_net_initialization_side_effect(sim, self, entity_id, anim_type);
            }

            let entity = self
                .world
                .entities
                .get_mut(entity_id)
                .expect("animation owner disappeared");
            // Many per-anim handlers call `Turn()` each
            // tick so the body keeps rotating toward the
            // direction goal *while* the action animation
            // plays.  Turning is decided strictly per animation
            // arm, and neighbouring arms of the same family often
            // disagree: `ParryingSword` turns but `ParryingLowSword`
            // does not, `StandingUpSword` turns but `StandingUpBow`
            // does not, and the helping-to-climb entry/exit
            // transitions do not turn while the
            // `WaitingHelpingClimbing` idle between them does.
            // Step the rotation here, then sync `element.direction`
            // to match before the sprite picks the row to play.
            let needs_turn =
                (matches!(
                    anim_type,
                    OrderType::TransitionRaisingSword
                                | OrderType::TransitionLoweringSword
                                | OrderType::WaitingSword
                                | OrderType::WaitingShield
                                | OrderType::ParryingSword
                                | OrderType::StrikingLowLeftSmalltalk
                                | OrderType::StrikingLowRightSmalltalk
                                | OrderType::StrikingLeftSmalltalk
                                | OrderType::StrikingRightSmalltalk
                                | OrderType::ParryingLeftSmalltalk
                                | OrderType::ParryingRightSmalltalk
                                | OrderType::ParryingLowLeftSmalltalk
                                | OrderType::ParryingLowRightSmalltalk
                                | OrderType::StrikingDownSword
                                | OrderType::ExtractingArrowSword
                                | OrderType::FallingLadderWall
                                | OrderType::RaisingShield
                                | OrderType::Rolling
                                | OrderType::Taking
                                | OrderType::TakingCrouched
                                | OrderType::TakingTarget
                                | OrderType::DroppingAmmo
                                | OrderType::DroppingAmmoCrouched
                                | OrderType::DroppingAle
                                | OrderType::DroppingAleCrouched
                                | OrderType::UsingLever
                                | OrderType::DrinkingAle
                                | OrderType::TakingNet
                                | OrderType::HittingTarget
                                | OrderType::HandlingTarget
                                | OrderType::UnlockingDoor
                                | OrderType::UnlockingTrap
                                | OrderType::SearchingCrouched
                                | OrderType::WaitingHelpingClimbing
                                | OrderType::WaitingCarryingOnShoulders
                                | OrderType::TransitionWaitingCarryingOnShouldersWaitingUpright
                                | OrderType::FallingShoulders
                                | OrderType::TransitionCrouchingUp
                                | OrderType::TransitionCrouchingDown
                                | OrderType::TransitionWaitingUprightClimbingWallUp
                                | OrderType::TransitionClimbingWallUpWaitingCrouched
                                | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                                | OrderType::TransitionWaitingCrouchedClimbingWallDown
                                | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                                | OrderType::TransitionClimbingWallDownWaitingUpright
                                | OrderType::TransitionWaitingUprightClimbingLadderUp
                                | OrderType::TransitionClimbingLadderUpWaitingCrouched
                                | OrderType::TransitionWaitingCrouchedClimbingLadderDown
                                | OrderType::TransitionClimbingLadderDownWaitingUpright
                                | OrderType::ClimbingWallUp
                                | OrderType::ClimbingWallDown
                                | OrderType::ClimbingWallUpFast
                                | OrderType::ClimbingWallDownFast
                                | OrderType::ClimbingLadderUp
                                | OrderType::ClimbingLadderDown
                                | OrderType::ClimbingLadderUpFast
                                | OrderType::ClimbingLadderDownFast
                                // GETTING_FREE_FROM_WASP calls `Turn()` each
                                // tick; the "still turning" branch substitutes
                                // `TURNING_ALERTED` for the sprite — handled
                                // below via `wasp_still_turning`.
                                | OrderType::GettingFreeFromWasp
                                // NPC-only arms that call Turn() per-tick.
                                // POINTING installs its authored direction
                                // goal on the first Execute tick above.
                                // SEARCHING has no direction_goal writer yet,
                                // but the parity slot is required.
                                | OrderType::Pointing
                                | OrderType::Searching
                ) && turn_arm_condition_holds(entity, anim_type, antagonist.is_some()))
                    || (entity.is_pc() && pc_beggar_execute_calls_turn(anim_type));
            // Capture `Turn()`'s return for the GETTING_FREE_FROM_WASP
            // still-turning substitution: while still
            // turning, play TURNING_ALERTED and return
            // InProgress; otherwise delegate to the
            // configured animation.
            let mut wasp_still_turning = false;
            let mut pc_taking_still_turning = false;
            let mut pc_target_still_turning = false;
            let direction_before_turn = entity.element_data().direction() as u16;
            if order_is_initialising
                && anim_type == OrderType::RaisingShield
                && let Some(danger) = entity
                    .actor_data()
                    .and_then(|actor| actor.shield_face_point)
            {
                // Human-actor execution initializes
                // RAISING_SHIELD by applying the generic
                // SHIELD_DANGER_POINT as the direction goal,
                // immediately before its first Turn().  Do this
                // here rather than at command dispatch: an
                // exit-action transition can leave the order
                // queued for several frames.
                let position = entity.element_data().position();
                let dx = danger.x - position.x;
                let dy = danger.y - position.y;
                if dx != 0.0 || dy != 0.0 {
                    let direction = crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy);
                    entity.position_iface_mut().set_direction(
                        crate::position_interface::Direction::from_raw(direction as i32),
                    );
                }
            }
            if owner_is_pc
                && anim_type == OrderType::WaitingShield
                && let Some(danger) = entity
                    .actor_data()
                    .and_then(|actor| actor.shield_face_point)
                && (danger.x != 0.0 || danger.y != 0.0)
            {
                // The PC override of WAITING_SHIELD re-aims at the
                // shield danger point on *every* tick, not just at
                // initialization, so a danger point that moves
                // while the shield is up keeps dragging the facing
                // around. The soldier override has no such step.
                let position = entity.element_data().position();
                let dx = danger.x - position.x;
                let dy = danger.y - position.y;
                let direction = crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy);
                entity.position_iface_mut().set_direction(
                    crate::position_interface::Direction::from_raw(direction as i32),
                );
            }
            if needs_turn {
                if owner_is_pc && anim_type == OrderType::RaisingShield {
                    // The PC override turns and then delegates to
                    // the human arm, which turns again: a PC
                    // raising its shield rotates two steps per
                    // tick, unlike every other shield holder.
                    let _ = turn_with_provenance(
                        entity,
                        entity_id,
                        frame_counter,
                        "raising_shield_pc_override",
                        false,
                    );
                }
                let still_turning = turn_with_provenance(
                    entity,
                    entity_id,
                    frame_counter,
                    if anim_type == OrderType::RaisingShield {
                        "raising_shield_human"
                    } else if anim_type == OrderType::WaitingShield {
                        "waiting_shield_human"
                    } else {
                        "animation_turn_arm"
                    },
                    false,
                );
                if anim_type == OrderType::WaitingShield && still_turning {
                    // Shield waiting updates the shield iff turning
                    // actually changed the facing direction.
                    crate::bow_shot::refresh_retained_shield_obstacle(
                        entity,
                        &assets.profile_manager,
                    );
                }
                if matches!(anim_type, OrderType::GettingFreeFromWasp) {
                    wasp_still_turning = still_turning;
                }
                if owner_is_pc
                    && matches!(
                        anim_type,
                        OrderType::Taking
                            | OrderType::TakingCrouched
                            | OrderType::TakingTarget
                            | OrderType::DroppingAmmo
                            | OrderType::DroppingAmmoCrouched
                    )
                {
                    pc_taking_still_turning = still_turning;
                }
                if owner_is_pc
                    && matches!(
                        anim_type,
                        OrderType::HittingTarget | OrderType::HandlingTarget
                    )
                {
                    pc_target_still_turning = still_turning;
                }
            }

            let ActorAnimationEntry {
                globally_frozen,
                diagnostic_frame,
                diagnostic_creation_order,
                sprite_row_diagnostic,
                ..
            } = entry;
            let entity = self
                .world
                .entities
                .get_mut(entity_id)
                .expect("animation owner disappeared");
            let row = actor_action_row(
                anim_type,
                effective_anim,
                direction_before_turn,
                entity.element_data().direction() as u16,
            );
            let held_weak_sword = hold_weak_sword_at_action_done(entity, anim_type);
            if held_weak_sword.is_some() {
                weak_sword_held = true;
            }
            let sprite_motion = held_weak_sword.or_else(|| {
                if globally_frozen && !jump_airborne_step {
                    // Global freezing leaves actor execution live
                    // but sprite action returns
                    // IN_PROGRESS without selecting, stamping, or
                    // advancing the sprite. Actor initialization
                    // was nevertheless consumed at update
                    // entry, independently of this sprite call.
                    return Some(MotionState::InProgress);
                }
                if effective_anim == OrderType::Freezing {
                    // The original game's actor execution handles
                    // the freezing state by returning an
                    // in-progress result without calling any
                    // sprite method. In particular it must not
                    // stamp WaitingUpright: a pathfinding
                    // MoveWaiting can be inserted between two
                    // instances of the same transition, and
                    // Frame initialization then resumes the
                    // transition's existing frame phase.
                    return Some(MotionState::InProgress);
                }
                let elem = entity.element_data_mut();
                let sprite = &mut elem.sprite;
                // GETTING_FREE_FROM_WASP still-turning: the
                // arm substitutes `TURNING_ALERTED` while
                // `Turn()` is still rotating the body
                // toward the random offset, then switches
                // to the configured animation once rotation
                // is done.  Otherwise delegate to
                // `sprite_anim_for_order` which runs the
                // per-arm non-animation → animation
                // substitutions (FALLING_HIT_* →
                // FALLING_BACK_*, missing-shield-variant
                // fallbacks, etc.).  The order's `anim_type`
                // stays unchanged so side-effect handlers
                // keep matching on the original token.
                let (played, progression) = if wasp_still_turning {
                    (OrderType::TurningAlerted, FrameProgression::Default)
                } else if pc_taking_still_turning {
                    (
                        sprite_anim_for_order(sprite, effective_anim, owner_is_pc),
                        FrameProgression::FrozenFirstFrame,
                    )
                } else if pc_target_still_turning {
                    (
                        sprite_anim_for_order(sprite, effective_anim, owner_is_pc),
                        FrameProgression::FrozenFirstFrame,
                    )
                } else if owner_is_pc
                    && anim_type == OrderType::TransitionWaitingCapeWaitingUpright
                    && cur_command == Some(Command::EnterCloak)
                {
                    // The shipped game has no separate cape-entry
                    // strip. Reusable cloaks deliberately play the
                    // the exit art from its last frame back
                    // to its first, then settle on WaitingCape.
                    (
                        sprite_anim_for_order(sprite, effective_anim, owner_is_pc),
                        FrameProgression::Reversed,
                    )
                } else if owner_is_pc
                    && matches!(
                        anim_type,
                        OrderType::WaitingCape
                            | OrderType::WaitingCapeAnonymousArcher
                            | OrderType::WaitingHidden
                    )
                {
                    // These PC idle arms explicitly use
                    // cyclic progression in the original game.
                    // WaitingCapeAnonymousArcher is a
                    // non-animation control token which plays the
                    // ordinary WaitingCape sprite, but retains the
                    // same non-terminating progression.
                    (
                        sprite_anim_for_order(sprite, effective_anim, owner_is_pc),
                        FrameProgression::Cyclically,
                    )
                } else if let Some(animation) = requested_custom_animation {
                    let progression = match cur_command {
                        Some(Command::PlayAnimLoop) => FrameProgression::Cyclically,
                        Some(Command::PlayAnimFrozen) => FrameProgression::FrozenLastFrame,
                        _ => FrameProgression::Default,
                    };
                    (animation, progression)
                } else if selected_order_is_custom_animation
                    && matches!(cur_command, Some(Command::PlayAnimLoop))
                {
                    (
                        sprite_anim_for_order(sprite, effective_anim, owner_is_pc),
                        FrameProgression::Cyclically,
                    )
                } else if selected_order_is_custom_animation
                    && matches!(cur_command, Some(Command::PlayAnimFrozen))
                {
                    (
                        sprite_anim_for_order(sprite, effective_anim, owner_is_pc),
                        FrameProgression::FrozenLastFrame,
                    )
                } else {
                    default_actor_sprite_playback(sprite, anim_type, effective_anim, owner_is_pc)
                };
                if jump_ground_motion_step {
                    // This take-off / landing transition runs its
                    // sprite through the shared motion path, which
                    // seeds the goal and its increment itself.
                    let order_id = order_id.unwrap_or_else(|| {
                        panic!("jump transition {anim_type:?} for {entity_id:?} has no order id")
                    });
                    let motion_order = crate::sprite::MotionOrderContext {
                        order_id,
                        destination: order_target,
                        reverse: false,
                        tolerance: order_tolerance,
                        directional_tolerance: false,
                        compute_direction: false,
                        next_destination_same_action: None,
                        target_element: order_antagonist,
                    };
                    return Some(super::jump::perform_jump_ground_motion(
                        entity,
                        sim,
                        motion_order,
                        played,
                        row,
                    ));
                }
                if jump_airborne_step {
                    return Some(super::jump::perform_jump_airborne_motion(
                        entity,
                        sim,
                        order_id,
                        played,
                        row,
                        globally_frozen,
                    ));
                }
                let elem = entity.element_data_mut();
                let sprite = &mut elem.sprite;
                let diagnostic_pre =
                    sprite_row_diagnostic.then(|| sprite.sprite_row_diagnostic_pre());
                // The original game's downward-sword-strike arm uniquely
                // forces initialization. On the first
                // order tick action processing resets the sprite frame
                // but deliberately skips frame initialization, so
                // last_action and the movement forecast remain
                // live until the following tick.
                let raw_motion = sprite.perform_action(
                    sim,
                    order_id,
                    played,
                    row,
                    progression,
                    anim_type == OrderType::StrikingDownSword,
                );
                if let Some(pre) = diagnostic_pre {
                    sprite.emit_sprite_row_diagnostic(
                        "perform_action",
                        diagnostic_frame,
                        diagnostic_creation_order.expect("enabled diagnostic has owner"),
                        entity_id.index(),
                        anim_type,
                        played,
                        row,
                        progression,
                        pre,
                        raw_motion,
                    );
                }
                Some(raw_motion)
            });
            // While still turning, the arm returns
            // InProgress regardless of what the
            // TURNING_ALERTED sprite reports — so the
            // WaspStruggleCycle completion can't fire early.
            if wasp_still_turning {
                Some(MotionState::InProgress)
            } else {
                if matches!(anim_type, OrderType::DrinkingAle)
                    && matches!(sprite_motion, Some(MotionState::Done))
                    && !self
                        .expect_entity(
                            antagonist.expect("drinking actor has no bottle"),
                            "drinking bottle",
                        )
                        .is_active()
                {
                    return Some(MotionState::Terminated);
                } else {
                    sprite_motion
                }
            }
        };
        let motion = motion.map(|state| {
            if uses_perform_flight(anim_type) {
                self.perform_combat_flight_position(entity_id, state)
            } else if anim_type == OrderType::FallingLadderWall {
                self.execute_ladder_fall_position(sim, assets, entity_id, state)
            } else {
                state
            }
        });
        if let Some(speech_id) = special_speech_id.filter(|id| *id == SPEECH_ID_HELBARDMAN) {
            self.execute_special_remark_at_sprite_point(sim, assets, entity_id, speech_id);
        }

        if anim_type == OrderType::StrikingDownSword {
            let element = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .expect("sword-down entry element disappeared");
            let target = match element.data {
                crate::sequence::SequenceElementData::Interaction { antagonist } => {
                    antagonist.expect("sword-down interaction has no target")
                }
                _ => panic!("sword-down element is not an interaction"),
            };
            let actor = self.expect_entity(entity_id, "sword-down owner");
            let victim = self.expect_entity(target, "sword-down victim");
            if !super::sequence_validity::striking_down_sword_valid_without_position(
                actor,
                victim,
                self.is_entity_vip(assets, victim),
            ) {
                return Some(MotionState::Terminated);
            }
        }
        let standing_up_sword_direction_goal = operands.standing_up_sword_direction_goal;
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("animation owner disappeared");
        let motion = motion.map(|motion_state| {
            if anim_type == OrderType::StandingUpSword {
                // Human-actor execution performs the
                // stand-up sprite first, then re-aims at the live
                // principal opponent (when swordfighting), then
                // turns. Its soldier override performs none of
                // this post-sprite facing work.
                apply_standing_up_sword_post_perform_facing(
                    entity,
                    standing_up_sword_direction_goal,
                    entity_id,
                    frame_counter,
                );
            }
            if !weak_sword_held {
                apply_weak_sword_tiredness_after_perform(entity, anim_type);
            }

            motion_state
        });
        if motion.is_some()
            && self
                .expect_entity(entity_id, "carry animation owner")
                .is_pc()
        {
            match anim_type {
                OrderType::WaitingWithCorpse
                | OrderType::TransitionWaitingUprightCarryingCorpse
                | OrderType::TransitionCarryingCorpseWaitingUpright => {
                    crate::abilities::sync_corpse_animation_for_carrier(
                        &mut self.world.entities,
                        &assets.profile_manager,
                        entity_id,
                        anim_type,
                    );
                }
                OrderType::WaitingOnShoulders => {
                    let carrier = self
                        .expect_entity(entity_id, "waiting rider")
                        .human_data()
                        .and_then(|human| human.carrier)
                        .expect("waiting rider has no carrier");
                    let depth = self
                        .expect_entity(carrier, "waiting rider carrier")
                        .sprite()
                        .display_depth;
                    let sprite = &mut self
                        .world
                        .entities
                        .get_mut(entity_id)
                        .expect("waiting rider disappeared")
                        .element_data_mut()
                        .sprite;
                    sprite.compute_display_depth_relative_to(depth, false);
                }
                _ => {}
            }
        }

        let ActorAnimationOperands {
            principal_frames_from_now,
            striking_down_sword_direction_goal,
            taking_net_order_was_done,
            ..
        } = operands;
        let tiredness_probe = entry.tiredness_probe;
        // Apply soldier-side per-anim-type side effects
        // (posture/action-state transitions, attentive-flag
        // toggling, view-status updates, bottle-hide /
        // coin-pickup / remarks).  Runs every tick so
        // START / DONE / TERMINATED get a chance to fire.
        // Uses `anim_type` (the original order type) — not
        // `effective_anim` — because the dispatch switch is
        // keyed on the order's animation field, not the
        // substituted one.
        //
        // `apply_npc_execute_side_effects` handles the
        // cases inherited from the NPC parent class
        // (SITTING / POINTING / SEARCHING pickpocket /
        // TRANSITION_SITTING / BEGGAR_SHOWING_FACE) — it
        // applies to both soldier and civilian NPCs.
        if let Some(motion_state) = motion {
            self.apply_jump_order_state(sim, assets, entity_id, motion_state);
            if anim_type == OrderType::TransitionHelpingClimbingDown {
                self.execute_helper_shoulder_dismount(sim, assets, entity_id, motion_state);
            }
            let entity = self.expect_entity(entity_id, "animation owner");
            let owner_is_pc = entity.is_pc();
            if owner_is_pc
                && anim_type == OrderType::TransitionCarryingCorpseWaitingUpright
                && motion_state == MotionState::Terminated
            {
                self.execute_corpse_drop_done(sim, assets, entity_id);
            }
            apply_soldier_execute_side_effects(
                self,
                sim,
                assets,
                anim_type,
                motion_state,
                antagonist,
                entity_id,
            );
            apply_npc_execute_side_effects(
                self,
                assets,
                anim_type,
                motion_state,
                antagonist,
                entity_id,
            );
            apply_actor_walk_start_side_effect(self, entity_id, anim_type, motion_state);
            let entity = self
                .world
                .entities
                .get_mut(entity_id)
                .expect("animation owner disappeared");
            super::jump::apply_jump_down_takeoff_drop(entity, anim_type, motion_state);
            apply_active_animation_start_state_side_effect(
                self,
                entity_id,
                anim_type,
                motion_state,
            );
            let equip_bow = forwards_pc_bow_action_on_start(
                self.expect_entity(entity_id, "animation owner"),
                anim_type,
                motion_state,
                current_element_script_driven,
            );
            if equip_bow {
                self.execute_pc_bow_equip_action(sim, assets, entity_id);
            }
            if owner_is_pc
                && motion_state == MotionState::Start
                && matches!(
                    anim_type,
                    OrderType::TransitionUnequipBow | OrderType::TransitionUnequipBowAnonymous
                )
            {
                self.execute_pc_bow_unequip_action(
                    sim,
                    assets,
                    (entity_id, current_element_script_driven),
                );
            }
            if owner_is_pc && motion_state == MotionState::Done {
                if matches!(
                    anim_type,
                    OrderType::DroppingAle | OrderType::DroppingAleCrouched
                ) {
                    self.execute_drop_ale_done(assets, entity_id);
                }
                match anim_type {
                    OrderType::TransitionWaitingUprightSimulatingBeggar
                    | OrderType::TransitionSimulatingBeggarWaitingUpright => {
                        let entering =
                            anim_type == OrderType::TransitionWaitingUprightSimulatingBeggar;
                        self.execute_beggar_wait_handoffs(sim, assets, (entity_id, entering));
                        self.execute_beggar_coin_flags(assets, (entity_id, entering));
                    }
                    OrderType::TransitionWaitingUprightHelpingClimbing => {
                        self.execute_pc_helping_climb_action(sim, assets, entity_id);
                    }
                    _ => {}
                }
            }
            apply_taking_net_side_effect(
                self,
                sim,
                assets,
                anim_type,
                motion_state,
                antagonist,
                entity_id,
                taking_net_order_was_done,
            );
            apply_waking_up_done_side_effect(
                self,
                sim,
                assets,
                anim_type,
                motion_state,
                antagonist,
                entity_id,
            );
            apply_pc_taking_side_effect(
                self,
                sim,
                assets,
                anim_type,
                motion_state,
                antagonist,
                entity_id,
            );
            apply_pc_target_interaction_side_effect(
                self,
                sim,
                assets,
                anim_type,
                motion_state,
                antagonist,
                entity_id,
            );
            apply_sword_parry_side_effect(
                self,
                entity_id,
                anim_type,
                motion_state,
                principal_frames_from_now,
            );
            apply_under_net_termination_side_effect(
                self.expect_entity_mut(entity_id, "animation owner"),
                anim_type,
                motion_state,
            );
            apply_smalltalk_start_and_recovery_side_effect(
                self,
                entity_id,
                anim_type,
                motion_state,
                &assets.profile_manager,
                tiredness_probe,
            );
            apply_striking_down_sword_side_effect(
                self,
                sim,
                assets,
                anim_type,
                motion_state,
                antagonist,
                striking_down_sword_direction_goal,
                entity_id,
            );
            apply_arrow_extraction_start_side_effect(self, entity_id, anim_type, motion_state);
            apply_shield_transition_side_effect(self, entity_id, anim_type, motion_state);
            if anim_type == OrderType::RaisingShield && motion_state == MotionState::Done {
                crate::bow_shot::refresh_retained_shield_obstacle(
                    self.expect_entity_mut(entity_id, "animation owner"),
                    &assets.profile_manager,
                );
            }
            apply_pc_disguise_exit_side_effect(
                self,
                anim_type,
                motion_state,
                cur_command,
                reusable_cloaks_enabled,
                entity_id,
            );
            apply_standing_up_start_side_effect(self, entity_id, anim_type, motion_state);
            apply_carried_start_side_effect(self, entity_id, anim_type, motion_state);
            apply_falling_start_side_effect(self, entity_id, anim_type, motion_state);
            apply_falling_completion_side_effect(self, entity_id, anim_type, motion_state);
            apply_dying_start_side_effect(self, entity_id, anim_type, motion_state);
            apply_being_dead_start_side_effect(self, entity_id, anim_type, motion_state);
            if uses_perform_flight(anim_type) {
                self.finish_combat_flight(sim, assets, entity_id, motion_state);
                finish_flight_action_state(
                    self.world
                        .entities
                        .get_mut(entity_id)
                        .expect("flight owner disappeared"),
                    anim_type,
                    motion_state,
                );
            }
            apply_combat_injury_side_effect(self, sim, assets, anim_type, motion_state, entity_id);
            if motion_state == MotionState::Done {
                let strike = match anim_type {
                    OrderType::StrikingLeftSmalltalk | OrderType::StrikingLowLeftSmalltalk => {
                        Some(crate::weapons::SwordStrike::SmalltalkLeft)
                    }
                    OrderType::StrikingRightSmalltalk | OrderType::StrikingLowRightSmalltalk => {
                        Some(crate::weapons::SwordStrike::SmalltalkRight)
                    }
                    _ => None,
                };
                if let Some(strike) = strike {
                    self.execute_smalltalk_strikes(
                        sim,
                        assets,
                        (
                            entity_id,
                            antagonist.expect("smalltalk strike order must retain its antagonist"),
                            strike,
                        ),
                    );
                }
            }
            if matches!(motion_state, MotionState::Done)
                && let Some(door_id) = unlock_door
            {
                self.execute_unlock_door_done(door_id);
            }
            // Lift sequence-element priority to
            // NonInterruptable on initialisation for the
            // always-non-interruptable anim families.
            // This priority change is complete before the arm returns.
            if matches!(motion_state, MotionState::Start)
                && anim_forces_non_interruptable_on_start(anim_type)
            {
                self.execute_non_interruptable_lifts((seq_id, elem_idx));
            }
            if play_anim_freeze_completed(motion_state, cur_command, anim_type) {
                self.execute_play_anim_frozen(
                    sim,
                    assets,
                    (
                        entity_id,
                        cur_command_level,
                        requested_custom_animation.unwrap_or_else(|| {
                            panic!("PlayAnimFreeze for {entity_id:?} has no AnimationId property")
                        }),
                    ),
                );
            }
        }
        if anim_type == OrderType::WaitingWithCorpse && motion == Some(MotionState::Start) {
            let carried = self
                .expect_entity(entity_id, "waiting corpse carrier")
                .pc_data()
                .and_then(|pc| pc.carried)
                .expect("waiting corpse carrier has no body");
            self.set_entity_posture(carried, crate::element::Posture::Carried);
            let body = self
                .world
                .entities
                .get_mut(carried)
                .expect("carried body disappeared");
            body.actor_data_mut()
                .expect("carried body must be actor")
                .action_state = crate::element::ActionState::Waiting;
        }

        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("animation owner disappeared");
        // Derived callbacks have completed. Retain the resulting motion for
        // the actor update's wait modifiers and crossing checks; termination
        // follows the live selection while abortion targets the entry element.
        let is_npc = matches!(entity, Entity::Soldier(_) | Entity::Civilian(_));
        let is_unconscious = entity.is_unconscious();
        let mut arm_ctx = ArmCtx {
            entity_id,
            is_npc,
            is_unconscious,
            seq_id,
            elem_idx,
            engine: self,
            assets,
        };
        Some(finish_actor_execute_result(
            sim,
            anim_type,
            motion,
            &mut arm_ctx,
        ))
    }

    fn execute_special_remark_at_sprite_point(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        speech_id: u32,
    ) {
        let sprite = self
            .expect_entity(entity_id, "special action owner")
            .sprite();
        if special_remark_due_at_sprite_phase(speech_id, sprite.current_frame, sprite.frame_count) {
            self.execute_special_remark(sim, assets, entity_id);
        }
    }
}
