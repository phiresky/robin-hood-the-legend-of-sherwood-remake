//! Phase methods of `EngineInner::tick_actor_animation_for`.
//!
//! This file is a child module of `engine::animation` (declared there with a
//! `#[path]` attribute) so the phases can use the animation-private helpers
//! without widening their visibility.
//!
//! The shell in `animation.rs` sequences these phases in the exact statement
//! order of the former monolithic body. Phases that could leave Execute early
//! report that through their return value and the shell returns immediately.
//! `ActorAnimationStepCtx` owns the single mutable entity borrow for the
//! generic dispatch (re-looking the entity up mutably would bump its slot
//! generation) next to the disjoint engine borrows the phases need.

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

/// Non-sprite operands snapshotted from the entity table before the
/// exclusive actor borrow is taken.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(super) struct ActorAnimationOperands {
    pub(super) principal_frames_from_now: Option<i16>,
    pub(super) drinking_ale_antagonist_active: Option<bool>,
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
    ) -> ControlFlow<Option<ActorExecuteResult>, ActorAnimationEntry> {
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
            .current_order_for_actor(entity_id)
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
            let frozen_wait = self
                .orders
                .sequence_manager
                .current_order_for_actor(entity_id)
                .and_then(|(seq_id, elem_idx, order)| {
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
                        .then_some(ActorExecuteResult {
                            order_type: order.order_type,
                            entry_seq_id: seq_id,
                            entry_elem_idx: elem_idx,
                            motion: MotionState::InProgress,
                        })
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
            .current_order_for_actor(entity_id)
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
            .current_order_for_actor(entity_id)
        else {
            return None;
        };
        let exact_selected_bow = actor.active_shot.is_active()
            && actor.active_shot.sequence_id == Some(seq_id)
            && actor.active_shot.element_index == elem_idx
            && crate::bow_shot::is_active_bow_order(order.order_type);
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
        let antagonist_active = validated_antagonist.and_then(|antagonist| {
                let antagonist_entity =
                    self.world.entities.get(antagonist).unwrap_or_else(|| {
                        panic!(
                            "actor {entity_id:?} required {anim_type:?} antagonist {antagonist:?} is missing at legacy slot {}",
                            entity_id.index()
                        )
                    });
                if anim_type == OrderType::DrinkingAle {
                    Some(antagonist_entity.is_active())
                } else {
                    None
                }
            });

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
            let dp = actor.active_door_pass.as_ref().unwrap_or_else(|| {
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
                    .get(usize::from(dp.door_index))
                    .unwrap_or_else(|| {
                        panic!(
                            "actor {entity_id:?} crenel transition references missing door {} at legacy slot {}",
                            dp.door_index,
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
                            dp.door_index,
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
                            dp.door_index,
                            entity_id.index()
                        )
                    });
            if sector.lift_type != Some(crate::sector::LiftType::Wall) {
                panic!(
                    "actor {entity_id:?} crenel door {} requires wall-lift sector {sector_number:?}, found {:?}",
                    dp.door_index, sector.lift_type
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
            drinking_ale_antagonist_active: antagonist_active,
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

    /// Snapshot whether the selected STRIKING_DOWN_SWORD element stays valid
    /// after sprite advancement.
    pub(super) fn striking_down_sword_valid_after_perform(
        &self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) -> Option<bool> {
        let striking_down_sword_valid_after_perform = self
            .orders
            .sequence_manager
            .current_order_for_actor(entity_id)
            .and_then(|(seq_id, elem_idx, order)| {
                (order.order_type == OrderType::StrikingDownSword).then_some((seq_id, elem_idx))
            })
            .map(|(seq_id, elem_idx)| {
                let element = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .expect("selected StrikingDownSword element disappeared before Execute");
                let antagonist = match element.data {
                    crate::sequence::SequenceElementData::Interaction { antagonist } => {
                        antagonist.expect("selected StrikingDownSword element lost its antagonist")
                    }
                    _ => panic!("selected StrikingDownSword element is not an interaction"),
                };
                let actor = self.world.entities.get(entity_id).unwrap_or_else(|| {
                    panic!("StrikingDownSword owner {entity_id:?} disappeared before Execute")
                });
                let victim = self.world.entities.get(antagonist).unwrap_or_else(|| {
                    panic!("StrikingDownSword victim {antagonist:?} disappeared before Execute")
                });
                super::sequence_validity::striking_down_sword_valid_without_position(
                    actor,
                    victim,
                    self.is_entity_vip(assets, victim),
                )
            });
        striking_down_sword_valid_after_perform
    }
}
