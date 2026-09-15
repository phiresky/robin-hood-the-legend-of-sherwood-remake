//! Phase methods of `EngineInner::tick_actor_animation_for`.
//!
//! This file is a child module of `engine::animation` (declared there with a
//! `#[path]` attribute) so the phases can use the animation-private helpers
//! without widening their visibility.
//!
//! The shell in `animation.rs` sequences these phases in the exact statement
//! order of the former monolithic body. Phases that could leave Execute early
//! report that through their return value and the shell returns immediately.
//! `ActorAnimationStepCtx` retains the selected order operands and borrows the
//! engine. Each operation releases its owner borrow before calling other owners.

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
            .current_order_for_actor(&self.world.entities, entity_id)
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

/// The selected order and the sequence-element properties Execute reads.
#[derive(Debug, Clone)]
pub(super) struct ActorAnimationOrderView {
    direction: u16,
    order_seq_elem: Option<(crate::sequence::SequenceId, usize)>,
    anim_type: OrderType,
    order_id: Option<std::num::NonZeroU32>,
    order_antagonist: Option<EntityId>,
    order_completion: Option<crate::order::OrderCompletion>,
    order_tolerance: f32,
    order_target: crate::coordinates::MapPoint,
    antagonist: Option<EntityId>,
    cur_command: Option<Command>,
    cur_command_level: Option<u16>,
    current_element_script_driven: bool,
    selected_order_is_custom_animation: bool,
    requested_custom_animation: Option<OrderType>,
    pointing_direction_goal: Option<i16>,
}

/// Dispatch facts and pre-sprite snapshots of the selected order.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct ActorAnimationPrep {
    is_turn: bool,
    effective_anim: OrderType,
    owner_is_pc: bool,
    jump_ground_motion_step: bool,
    jump_airborne_step: bool,
    order_is_initialising: bool,
    drinking_ale_antagonist_inactive: bool,
    special_speech_id: Option<u32>,
    weak_stunned_action_before_perform: Option<ActionState>,
}

/// Results of the per-arm `Turn()` step that precedes sprite playback.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct ActorAnimationSpriteTurn {
    wasp_still_turning: bool,
    pc_taking_still_turning: bool,
    pc_target_still_turning: bool,
    direction_before_turn: u16,
}

/// Borrowed engine view and accumulators for one actor's generic Execute.
///
/// No serde: this is a borrow bundle, not data.
pub(super) struct ActorAnimationStepCtx<'a> {
    pub(super) engine: &'a mut EngineInner,
    pub(super) sim: &'a crate::sim_rng::SimulationContext,
    pub(super) assets: &'a LevelAssets,
    pub(super) entity_id: EntityId,
    pub(super) frame_counter: u32,
    pub(super) reusable_cloaks_enabled: bool,
    pub(super) selected_generic_order: bool,
    pub(super) entry: ActorAnimationEntry,
    pub(super) operands: ActorAnimationOperands,
    pub(super) striking_down_sword_valid_after_perform: Option<bool>,
}

impl ActorAnimationStepCtx<'_> {
    /// Generic Execute dispatch for an actor entity.
    pub(super) fn run(mut self) -> Option<ActorExecuteResult> {
        let mut execute_result = None;
        'actor: {
            let ControlFlow::Continue(view) = self.selected_order_view() else {
                break 'actor;
            };
            // AI-driven animation (Pointing, RaisingShield, dying,
            // falling-hit, BORED idle cycle, …)?  Drive it via
            // perform_action; on completion, run side-effect
            // helpers and fire the bound `OrderCompletion`.
            if let Some((seq_id, elem_idx)) = view.order_seq_elem {
                execute_result = Some(self.execute_selected_order(seq_id, elem_idx, view));
                break 'actor;
            }

            // No current order on the actor.  Dispatch only ever
            // runs from a present front order — when there is
            // none, the next tick's `Wait()` lazy-init creates one
            // and dispatch resumes.  Nothing to play this tick;
            // let `tick_melee_strikes` drive an active sweep if
            // any, and the newly-launched wait element will take
            // over next hourglass.
            let _ = view.direction;
        }
        execute_result
    }

    /// Movement/bow admission re-check and the selected order snapshot.
    fn selected_order_view(&self) -> ControlFlow<(), ActorAnimationOrderView> {
        let entity: &Entity = self
            .engine
            .world
            .entities
            .get(self.entity_id)
            .expect("animation owner disappeared");
        let entity_id = self.entity_id;
        let selected_generic_order = self.selected_generic_order;
        let validated_antagonist = self.operands.validated_antagonist;
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
            return ControlFlow::Break(());
        }

        // Exact selected melee and bow arms are admitted by the live
        // owner coordinator/current-order check. Stale background
        // state must not suppress the actual selected generic arm.
        let direction = entity.element_data().direction() as u16;

        // Read the actor's current in-progress sequence element
        // and its front order.  All animation driving flows off
        // this — dispatch is on the current element's front
        // order. The selected identity survives synchronous owner callbacks.
        let order_snapshot = self
            .engine
            .orders
            .sequence_manager
            .current_order_for_actor(&self.engine.world.entities, entity_id);
        let (
            order_seq_elem,
            anim_type,
            order_id,
            order_antagonist,
            order_completion,
            order_tolerance,
            order_target,
        ) = if let Some((seq_id, elem_idx, order)) = order_snapshot {
            (
                Some((seq_id, elem_idx)),
                order.order_type,
                Some(order.order_id),
                order.antagonist,
                Some(order.completion.clone()),
                order.tolerance,
                crate::coordinates::MapPoint::new(order.target_x, order.target_y),
            )
        } else {
            (
                None,
                crate::order::OrderType::Invalid,
                None,
                None,
                None,
                0.0,
                crate::coordinates::MapPoint::ZERO,
            )
        };
        if let Some((seq_id, elem_idx)) = order_seq_elem
            && actor.active_shot.is_active()
            && actor.active_shot.sequence_id == Some(seq_id)
            && actor.active_shot.element_index == elem_idx
            && crate::bow_shot::is_active_bow_order(anim_type)
        {
            return ControlFlow::Break(());
        }
        let antagonist = validated_antagonist.or(order_antagonist);

        // Is the current element a one-shot action (not the
        // actor's `Command::Wait` idle element)?  One-shots
        // drive dead / unconscious actors through DYING_* /
        // FALLING_HIT_* terminates before the corpse settles.
        // The settled BEING_DEAD / BEING_UNCONSCIOUS wait
        // orders still need to execute every tick to keep the
        // sprite row on the corpse/KO hold row used by
        // body-point calculations (e.g. compute-stars-point).
        let cur_command = order_seq_elem.and_then(|(s, e)| {
            self.engine
                .orders
                .sequence_manager
                .get_element(s, e)
                .map(|el| el.command)
        });
        let cur_command_level = order_seq_elem.and_then(|(s, e)| {
            self.engine
                .orders
                .sequence_manager
                .get_element(s, e)
                .map(|el| el.command_level)
        });
        let current_element_script_driven = order_seq_elem.is_some_and(|(s, e)| {
            self.engine
                .orders
                .sequence_manager
                .get_element(s, e)
                .is_some_and(|element| element.script_driven)
        });
        // The sequence element keeps its PlayAnim* command while
        // Transition generation temporarily puts ordinary posture/action
        // transition orders at its front.  Original consults
        // AnimationId only in the custom-animation execution
        // arms; applying it to those transition orders replaces their
        // authored sprite row with the eventual custom animation.
        let selected_order_is_custom_animation = is_custom_animation_order(anim_type);
        let requested_custom_animation = selected_order_is_custom_animation
            .then_some(order_seq_elem)
            .flatten()
            .and_then(|(s, e)| {
                let element = self.engine.orders.sequence_manager.get_element(s, e)?;
                if !matches!(
                    element.command,
                    Command::PlayAnim
                        | Command::PlayAnimLoop
                        | Command::PlayAnimFreeze
                        | Command::PlayAnimFrozen
                ) {
                    return None;
                }
                match element.get_property(crate::sequence::Field::AnimationId) {
                    Some(crate::sequence::FieldValue::Animation(animation)) => Some(*animation),
                    Some(crate::sequence::FieldValue::Integer(value)) => {
                        OrderType::try_from(*value).ok()
                    }
                    _ => None,
                }
            });
        let pointing_direction_goal = if cur_command == Some(Command::Point) {
            let direction = order_seq_elem
                .and_then(|(s, e)| {
                    self.engine
                        .orders
                        .sequence_manager
                        .get_element(s, e)
                        .and_then(|element| element.get_property(crate::sequence::Field::Direction))
                })
                .and_then(|value| match value {
                    crate::sequence::FieldValue::Integer(direction) => Some(*direction as i16),
                    _ => None,
                })
                .expect("Point sequence is missing its integer Direction property");
            Some(direction)
        } else {
            None
        };
        ControlFlow::Continue(ActorAnimationOrderView {
            direction,
            order_seq_elem,
            anim_type,
            order_id,
            order_antagonist,
            order_completion,
            order_tolerance,
            order_target,
            antagonist,
            cur_command,
            cur_command_level,
            current_element_script_driven,
            selected_order_is_custom_animation,
            requested_custom_animation,
            pointing_direction_goal,
        })
    }

    /// Execute the selected order: prepare, run the arm, apply side effects,
    /// then decide the base-Actor return value.
    fn execute_selected_order(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        view: ActorAnimationOrderView,
    ) -> ActorExecuteResult {
        let owner = self.engine.expect_entity(self.entity_id, "animation owner");
        if owner.is_soldier() {
            match view.anim_type {
                OrderType::WaitingUpright if owner.enemy_ai().is_some() => {
                    self.engine
                        .execute_waiting_upright(self.sim, self.assets, self.entity_id);
                }
                OrderType::WaitingAlerted => {
                    self.engine
                        .execute_waiting_alerted(self.sim, self.assets, self.entity_id);
                }
                _ => {}
            }
        }
        let prep = self.prepare_selected_order(&view);
        if prep.weak_stunned_action_before_perform.is_some() {
            self.engine.add_weak_stunned_combat(
                self.sim,
                self.assets,
                self.entity_id,
                view.anim_type == OrderType::BeingWeakSword,
            );
        }
        const SPEECH_ID_HELBARDMAN: u32 = 0x4c484453;
        if let Some(speech_id) = prep
            .special_speech_id
            .filter(|id| *id != SPEECH_ID_HELBARDMAN)
        {
            self.execute_special_remark_at_sprite_point(speech_id);
        }
        let (motion, weak_sword_held) = self.selected_order_motion(&view, &prep);
        if let Some(speech_id) = prep
            .special_speech_id
            .filter(|id| *id == SPEECH_ID_HELBARDMAN)
        {
            self.execute_special_remark_at_sprite_point(speech_id);
        }
        let motion = self.apply_post_perform_effects(&view, &prep, motion, weak_sword_held);
        let anim_type = view.anim_type;
        self.apply_motion_side_effects(seq_id, elem_idx, view, &prep, motion);
        self.finish_selected_order(seq_id, elem_idx, anim_type, motion)
    }

    fn execute_special_remark_at_sprite_point(&mut self, speech_id: u32) {
        let sprite = self
            .engine
            .expect_entity(self.entity_id, "special action owner")
            .sprite();
        if special_remark_due_at_sprite_phase(speech_id, sprite.current_frame, sprite.frame_count) {
            self.engine
                .execute_special_remark(self.sim, self.assets, self.entity_id);
        }
    }

    /// Order-initialisation facing, posture and goal writes that precede the
    /// Execute arm.
    fn prepare_selected_order(&mut self, view: &ActorAnimationOrderView) -> ActorAnimationPrep {
        let &ActorAnimationOrderView {
            anim_type,
            order_id,
            antagonist,
            cur_command,
            pointing_direction_goal,
            ..
        } = view;
        let ActorAnimationOperands {
            drinking_ale_antagonist_active,
            door_pass_crenel_transition_dir,
            waiting_sword_direction_goal,
            extracting_arrow_sword_direction_goal,
            taking_direction_goal,
            pc_target_direction_goal,
            waiting_on_shoulders_direction,
            ..
        } = self.operands;
        let entity_id = self.entity_id;
        let assets = self.assets;
        let entity = self
            .engine
            .world
            .entities
            .get_mut(self.entity_id)
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
        let actor_in_jump = entity
            .actor_data()
            .is_some_and(|actor| actor.active_jump.is_some());
        let jump_ground_motion_step =
            super::jump::jump_step_uses_perform_motion(anim_type) && actor_in_jump;
        // Airborne segments fly the body themselves and ignore
        // what their animation reports, so they take their own
        // Execute path below.
        let jump_airborne_step = super::jump::jump_step_is_airborne(entity, anim_type);
        let order_is_initialising = actor.execute_order_initialising;
        if order_is_initialising
            && anim_type == OrderType::WaitingCarryingOnShoulders
            && let Some(carried_id) = entity.pc_data().and_then(|pc| pc.carried)
        {
            self.engine.actor_wait(self.sim, self.assets, carried_id);
        }
        let entity = self
            .engine
            .world
            .entities
            .get_mut(entity_id)
            .expect("animation owner disappeared");
        if anim_type == OrderType::TransitionHelpingClimbingDown
            && entity.pc_data().is_some_and(|pc| pc.carried.is_some())
        {
            // The transition sets the helper's
            // states before playing the lowering animation.
            entity.set_posture(crate::element::Posture::HelpingToClimb);
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
            entity.set_posture(crate::element::Posture::Flying);
        }
        let drinking_ale_antagonist_inactive = matches!(anim_type, OrderType::DrinkingAle)
            && antagonist.is_some()
            && drinking_ale_antagonist_active == Some(false);
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
        ActorAnimationPrep {
            is_turn,
            effective_anim,
            owner_is_pc,
            jump_ground_motion_step,
            jump_airborne_step,
            order_is_initialising,
            drinking_ale_antagonist_inactive,
            special_speech_id,
            weak_stunned_action_before_perform,
        }
    }

    /// Per-arm motion dispatch. Also reports whether the weak-sword hold
    /// replaced sprite playback.
    fn selected_order_motion(
        &mut self,
        view: &ActorAnimationOrderView,
        prep: &ActorAnimationPrep,
    ) -> (Option<MotionState>, bool) {
        let &ActorAnimationOrderView {
            anim_type,
            order_tolerance,
            ..
        } = view;
        let &ActorAnimationPrep {
            is_turn,
            order_is_initialising,
            drinking_ale_antagonist_inactive,
            ..
        } = prep;
        let entity_id = self.entity_id;
        let sim = self.sim;
        let assets = self.assets;
        let mut weak_sword_held = false;
        let motion = if is_turn {
            self.turn_order_motion(view, prep)
        } else if anim_type == OrderType::Select {
            // SELECT is a real non-animation order in the
            // translated door chain. It starts the Human/PC hulk
            // effect and terminates in this owner slot without
            // dispatching a sprite animation.
            if order_is_initialising {
                self.engine
                    .execute_select_hulk((entity_id, order_tolerance));
            }
            Some(MotionState::Terminated)
        } else if matches!(anim_type, OrderType::DrinkingAle)
            && order_is_initialising
            && drinking_ale_antagonist_inactive
        {
            Some(MotionState::Terminated)
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
                apply_under_net_initialization_side_effect(
                    sim,
                    self.engine
                        .world
                        .entities
                        .get_mut(self.entity_id)
                        .expect("animation owner disappeared"),
                    anim_type,
                );
            }
            let turn = self.pre_sprite_turn(view, prep);
            let wasp_still_turning = turn.wasp_still_turning;
            let sprite_motion =
                self.perform_selected_sprite(view, prep, turn, &mut weak_sword_held);
            // While still turning, the arm returns
            // InProgress regardless of what the
            // TURNING_ALERTED sprite reports — so the
            // WaspStruggleCycle completion can't fire early.
            if wasp_still_turning {
                Some(MotionState::InProgress)
            } else {
                if matches!(anim_type, OrderType::DrinkingAle)
                    && matches!(sprite_motion, Some(MotionState::Done))
                    && drinking_ale_antagonist_inactive
                {
                    Some(MotionState::Terminated)
                } else {
                    sprite_motion
                }
            }
        };
        (motion, weak_sword_held)
    }

    /// TURNING arm: `Turn()` drives completion, the sprite is visual only.
    fn turn_order_motion(
        &mut self,
        view: &ActorAnimationOrderView,
        prep: &ActorAnimationPrep,
    ) -> Option<MotionState> {
        let &ActorAnimationOrderView {
            anim_type,
            order_id,
            cur_command,
            ..
        } = view;
        let effective_anim = prep.effective_anim;
        let entity_id = self.entity_id;
        let sim = self.sim;
        let frame_counter = self.frame_counter;
        let globally_frozen = self.entry.globally_frozen;
        let entity = self
            .engine
            .world
            .entities
            .get_mut(self.entity_id)
            .expect("animation owner disappeared");
        let direction_before_turn = entity.element_data().direction() as u16;
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
        if !globally_frozen {
            let direction_after_turn = entity.element_data().direction() as u16;
            // Base actor execution turns before processing the action,
            // but the attentive Soldier override performs
            // TURNING_ALERTED first. The explicit row keeps
            // that Original ordering visible while the shared
            // turn step still updates the position interface
            // before this block returns.
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
        // TURNING is the exceptional Original arm whose
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

    /// Shield danger-point aiming and the per-arm `Turn()` before sprite
    /// playback.
    fn pre_sprite_turn(
        &mut self,
        view: &ActorAnimationOrderView,
        prep: &ActorAnimationPrep,
    ) -> ActorAnimationSpriteTurn {
        let &ActorAnimationOrderView {
            anim_type,
            antagonist,
            ..
        } = view;
        let &ActorAnimationPrep {
            owner_is_pc,
            order_is_initialising,
            ..
        } = prep;
        let entity_id = self.entity_id;
        let assets = self.assets;
        let frame_counter = self.frame_counter;
        let entity = self
            .engine
            .world
            .entities
            .get_mut(self.entity_id)
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
        let needs_turn = (matches!(
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
                crate::bow_shot::refresh_retained_shield_obstacle(entity, &assets.profile_manager);
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
        ActorAnimationSpriteTurn {
            wasp_still_turning,
            pc_taking_still_turning,
            pc_target_still_turning,
            direction_before_turn,
        }
    }

    /// Row selection, weak-sword hold and the selected sprite/jump motion.
    fn perform_selected_sprite(
        &mut self,
        view: &ActorAnimationOrderView,
        prep: &ActorAnimationPrep,
        turn: ActorAnimationSpriteTurn,
        weak_sword_held: &mut bool,
    ) -> Option<MotionState> {
        let &ActorAnimationOrderView {
            anim_type,
            order_id,
            order_antagonist,
            order_tolerance,
            order_target,
            cur_command,
            selected_order_is_custom_animation,
            requested_custom_animation,
            ..
        } = view;
        let &ActorAnimationPrep {
            effective_anim,
            owner_is_pc,
            jump_ground_motion_step,
            jump_airborne_step,
            ..
        } = prep;
        let ActorAnimationSpriteTurn {
            wasp_still_turning,
            pc_taking_still_turning,
            pc_target_still_turning,
            direction_before_turn,
        } = turn;
        let ActorAnimationEntry {
            globally_frozen,
            diagnostic_frame,
            diagnostic_creation_order,
            sprite_row_diagnostic,
            ..
        } = self.entry;
        let entity_id = self.entity_id;
        let sim = self.sim;
        let entity = self
            .engine
            .world
            .entities
            .get_mut(self.entity_id)
            .expect("animation owner disappeared");
        let row = actor_action_row(
            anim_type,
            effective_anim,
            direction_before_turn,
            entity.element_data().direction() as u16,
        );
        let held_weak_sword = hold_weak_sword_at_action_done(entity, anim_type);
        if held_weak_sword.is_some() {
            *weak_sword_held = true;
        }
        let sprite_motion = held_weak_sword.or_else(|| {
            if globally_frozen {
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
                // Original's exit art from its last frame back
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
                    entity, sim, order_id, played, row,
                ));
            }
            let elem = entity.element_data_mut();
            let sprite = &mut elem.sprite;
            let diagnostic_pre = sprite_row_diagnostic.then(|| sprite.sprite_row_diagnostic_pre());
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
        sprite_motion
    }

    /// Post-sprite revalidation, stand-up facing and tiredness work.
    fn apply_post_perform_effects(
        &mut self,
        view: &ActorAnimationOrderView,
        prep: &ActorAnimationPrep,
        motion: Option<MotionState>,
        weak_sword_held: bool,
    ) -> Option<MotionState> {
        let anim_type = view.anim_type;
        let striking_down_sword_valid_after_perform = self.striking_down_sword_valid_after_perform;
        let standing_up_sword_direction_goal = self.operands.standing_up_sword_direction_goal;
        let entity_id = self.entity_id;
        let frame_counter = self.frame_counter;
        let entity = self
            .engine
            .world
            .entities
            .get_mut(self.entity_id)
            .expect("animation owner disappeared");
        let motion = motion.map(|mut motion_state| {
            if anim_type == OrderType::StrikingDownSword
                && striking_down_sword_valid_after_perform == Some(false)
            {
                // This performs the
                // check after sprite advancement and returns before
                // START/DONE side effects when it fails.
                motion_state = MotionState::Terminated;
            }
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
        motion
    }

    /// START / DONE / TERMINATED side effects of this tick's motion.
    fn apply_motion_side_effects(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        view: ActorAnimationOrderView,
        prep: &ActorAnimationPrep,
        motion: Option<MotionState>,
    ) {
        let ActorAnimationOrderView {
            anim_type,
            order_completion,
            antagonist,
            cur_command,
            cur_command_level,
            current_element_script_driven,
            requested_custom_animation,
            ..
        } = view;
        let &ActorAnimationPrep {
            order_is_initialising,
            ..
        } = prep;
        let ActorAnimationOperands {
            principal_frames_from_now,
            striking_down_sword_direction_goal,
            taking_net_order_was_done,
            ..
        } = self.operands;
        let tiredness_probe = self.entry.tiredness_probe;
        let reusable_cloaks_enabled = self.reusable_cloaks_enabled;
        let entity_id = self.entity_id;
        let assets = self.assets;
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
            let phase = ActorMotionPhase {
                entity_id,
                anim_type,
                motion_state,
                antagonist,
            };
            phase.execute(
                self.engine,
                self.sim,
                assets,
                order_is_initialising,
                current_element_script_driven,
                taking_net_order_was_done,
                principal_frames_from_now,
                tiredness_probe,
                striking_down_sword_direction_goal,
                cur_command,
                reusable_cloaks_enabled,
            );
            if matches!(motion_state, MotionState::Done)
                && let Some(crate::order::OrderCompletion::UnlockDoor { door_id }) =
                    order_completion
            {
                self.engine.execute_unlock_door_done(door_id);
            }
            // Lift sequence-element priority to
            // NonInterruptable on initialisation for the
            // always-non-interruptable anim families.
            // This priority change is complete before the arm returns.
            if matches!(motion_state, MotionState::Start)
                && anim_forces_non_interruptable_on_start(anim_type)
            {
                self.engine
                    .execute_non_interruptable_lifts((seq_id, elem_idx));
            }
            if play_anim_freeze_completed(motion_state, cur_command, anim_type) {
                self.engine.execute_play_anim_frozen(
                    self.sim,
                    self.assets,
                    (
                        entity_id,
                        cur_command_level.unwrap_or(1),
                        requested_custom_animation.unwrap_or_else(|| {
                            panic!("PlayAnimFreeze for {entity_id:?} has no AnimationId property")
                        }),
                    ),
                );
            }
        }
    }

    /// Decide the base-Actor return value of this Execute.
    fn finish_selected_order(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        anim_type: OrderType,
        motion: Option<MotionState>,
    ) -> ActorExecuteResult {
        let entity_id = self.entity_id;
        let sim = self.sim;
        let entity = self
            .engine
            .world
            .entities
            .get_mut(self.entity_id)
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
            engine: self.engine,
            assets: self.assets,
        };
        finish_actor_execute_result(sim, anim_type, motion, &mut arm_ctx)
    }
}
