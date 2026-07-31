//! Post-SequenceManager adoption of common `RHElementActor` ownership.
//!
//! Original stores raw pointers to the selected sequence element, its
//! currently executing order, and the actor's idle wait element. Rust keeps
//! those objects in `SequenceManager` and derives selection from its exact
//! in-progress topology, so adoption validates that the converted manager
//! reconstructs the same pointers rather than adding a second source of
//! truth. The genuinely actor-owned post-seek sequence and script VM heap are
//! restored after that validation succeeds.

use thiserror::Error;

use crate::{
    element::{Command, Entity, EntityId},
    engine::{EngineInner, LevelAssets},
    movement::ActiveMelee,
    natives::ScriptHandleCodec,
    sequence::{Sequence, SequenceElementData, SequenceElementRef},
    weapons::SwordStrike,
};

use super::{
    adopt::{LegacyEntityFixups, LegacySaveAdoptError},
    adopt_object_leaves::{LegacyObjectLeafAdoptError, LegacyVmOwnerKind, preflight_vm},
    adopt_sequences::{
        LegacySequenceAdoptError, LegacySequenceAdoptionPlan, LegacySequenceTopology,
        convert_owner_local_sequence,
    },
    adopt_vm_arena::{LegacyVmArenaError, LegacyVmArenaOwner, LegacyVmArenaPlan},
    payload_base::LegacyActorPayload,
    payload_dispatch::{LegacyElementPayload, LegacyElementPayloadStream},
};

#[derive(Debug, Error)]
pub enum LegacyActorOwnershipAdoptError {
    #[error(transparent)]
    VmArena(#[from] LegacyVmArenaError),
    #[error(transparent)]
    Reference(#[from] LegacySaveAdoptError),
    #[error(transparent)]
    Sequence(#[from] LegacySequenceAdoptError),
    #[error(transparent)]
    Vm(#[from] LegacyObjectLeafAdoptError),
    #[error("saved actor creation order {creation_order} resolves to missing entity {entity_id}")]
    MissingEntity {
        creation_order: u32,
        entity_id: EntityId,
    },
    #[error("saved actor creation order {creation_order} resolves to non-actor entity {entity_id}")]
    ExpectedActor {
        creation_order: u32,
        entity_id: EntityId,
    },
    #[error(
        "saved actor creation order {creation_order} field {field} resolves to sequence element {reference:?} owned by {actual:?}, expected {expected}"
    )]
    WrongSequenceOwner {
        creation_order: u32,
        field: &'static str,
        reference: SequenceElementRef,
        actual: Option<EntityId>,
        expected: EntityId,
    },
    #[error(
        "saved actor creation order {creation_order} wait_sequence_element resolves to command {command:?}, expected Wait or Freeze"
    )]
    WrongWaitCommand {
        creation_order: u32,
        command: Command,
    },
    #[error(
        "saved actor creation order {creation_order} selected sequence element is {saved:?}, but the converted manager reconstructs {runtime:?}"
    )]
    SelectedElementMismatch {
        creation_order: u32,
        saved: Option<SequenceElementRef>,
        runtime: Option<SequenceElementRef>,
    },
    #[error(
        "saved actor creation order {creation_order} order pointer resolves to {order_element:?} order index {order_index}, but selected element is {selected:?}"
    )]
    OrderElementMismatch {
        creation_order: u32,
        order_element: SequenceElementRef,
        order_index: usize,
        selected: Option<SequenceElementRef>,
    },
    #[error(
        "saved actor creation order {creation_order} order pointer resolves to index {order_index}, but selected element's current order is index zero"
    )]
    OrderCursorMismatch {
        creation_order: u32,
        order_index: usize,
    },
    #[error(
        "saved actor creation order {creation_order} has selected element {selected:?} but a null order pointer while that element has a current order"
    )]
    MissingOrder {
        creation_order: u32,
        selected: SequenceElementRef,
    },
    #[error(
        "saved actor creation order {creation_order} has an order pointer but no selected sequence element"
    )]
    OrderWithoutElement { creation_order: u32 },
}

#[derive(Debug)]
pub(crate) struct LegacyActorOwnershipAdoptionPlan {
    records: Vec<PlannedActorOwnership>,
}

#[derive(Debug)]
struct PlannedActorOwnership {
    entity: EntityId,
    /// Retained for diagnostics: the manager is the canonical owner.
    selected_element: Option<SequenceElementRef>,
    /// Retained for diagnostics: the manager is the canonical owner.
    wait_element: Option<SequenceElementRef>,
    post_seek_sequence: Option<Box<Sequence>>,
    vm_heap: Option<Vec<u8>>,
}

impl LegacyActorOwnershipAdoptionPlan {
    /// Validate all actor pointers against the not-yet-installed converted
    /// SequenceManager, then convert actor-owned inline/script state.
    pub(crate) fn preflight(
        engine: &EngineInner,
        assets: &LevelAssets,
        payloads: &LegacyElementPayloadStream,
        entities: &LegacyEntityFixups,
        sequence_topology: &LegacySequenceTopology,
        sequences: &LegacySequenceAdoptionPlan,
        vm_arena: &LegacyVmArenaPlan,
    ) -> Result<Self, LegacyActorOwnershipAdoptError> {
        let mut records = Vec::new();
        for record in &payloads.records {
            let Some(saved) = actor_payload(&record.payload) else {
                continue;
            };
            let creation_order = record.header.creation_order;
            let entity = entities
                .by_creation_order
                .get(&creation_order)
                .copied()
                .ok_or(LegacySaveAdoptError::MissingCreationOrderReference { creation_order })?;
            let runtime = engine.world.entities.get(entity).ok_or(
                LegacyActorOwnershipAdoptError::MissingEntity {
                    creation_order,
                    entity_id: entity,
                },
            )?;
            if !runtime.is_actor() {
                return Err(LegacyActorOwnershipAdoptError::ExpectedActor {
                    creation_order,
                    entity_id: entity,
                });
            }

            let selected_element = resolve_owned_element(
                sequences,
                creation_order,
                entity,
                "sequence_element",
                saved.sequence_element,
            )?;
            let wait_element = resolve_owned_element(
                sequences,
                creation_order,
                entity,
                "wait_sequence_element",
                saved.wait_sequence_element,
            )?;
            if let Some(wait) = wait_element {
                let (_, element) = sequences
                    .resolve_element("wait_sequence_element", saved.wait_sequence_element)?
                    .expect("non-null preflighted wait element disappeared");
                if !matches!(element.command, Command::Wait | Command::Freeze) {
                    return Err(LegacyActorOwnershipAdoptError::WrongWaitCommand {
                        creation_order,
                        command: element.command,
                    });
                }
                debug_assert_eq!(
                    wait,
                    sequences
                        .resolve_element("wait_sequence_element", saved.wait_sequence_element)?
                        .expect("same immutable plan must resolve identically")
                        .0
                );
            }

            let reconstructed = sequences.current_element_for_actor(entity);
            if reconstructed != selected_element {
                return Err(LegacyActorOwnershipAdoptError::SelectedElementMismatch {
                    creation_order,
                    saved: selected_element,
                    runtime: reconstructed,
                });
            }

            let resolved_order = sequences.resolve_order("order", saved.order)?;
            match (selected_element, resolved_order) {
                (None, Some(_)) => {
                    return Err(LegacyActorOwnershipAdoptError::OrderWithoutElement {
                        creation_order,
                    });
                }
                (Some(selected), Some((order_element, order_index, _))) => {
                    if order_element != selected {
                        return Err(LegacyActorOwnershipAdoptError::OrderElementMismatch {
                            creation_order,
                            order_element,
                            order_index,
                            selected: Some(selected),
                        });
                    }
                    // Rust pops completed orders from the front just like
                    // RHSequenceElement::Proceed; the executing order must be
                    // the front of the restored queue.
                    if order_index != 0 {
                        return Err(LegacyActorOwnershipAdoptError::OrderCursorMismatch {
                            creation_order,
                            order_index,
                        });
                    }
                }
                (Some(selected), None) => {
                    let (_, element) = sequences
                        .resolve_element("sequence_element", saved.sequence_element)?
                        .expect("non-null preflighted selected element disappeared");
                    if element.current_order().is_some() {
                        return Err(LegacyActorOwnershipAdoptError::MissingOrder {
                            creation_order,
                            selected,
                        });
                    }
                }
                (None, None) => {}
            }

            let post_seek_sequence = saved
                .post_seek_sequence
                .as_ref()
                .map(|sequence| {
                    convert_owner_local_sequence(sequence, entities, sequence_topology)
                        .map(Box::new)
                })
                .transpose()?;
            let location_prefix = saved
                .script_members
                .as_ref()
                .map(|members| {
                    vm_arena.owner_prefix(LegacyVmArenaOwner::Element(creation_order), members)
                })
                .transpose()?
                .unwrap_or(0);
            let mut computed_locations = Vec::new();
            let vm_heap = preflight_vm(
                engine,
                assets,
                entities,
                entity,
                creation_order,
                LegacyVmOwnerKind::Actor,
                saved.script_members.as_ref(),
                location_prefix,
                &mut computed_locations,
            )?;
            records.push(PlannedActorOwnership {
                entity,
                selected_element,
                wait_element,
                post_seek_sequence,
                vm_heap,
            });
        }
        Ok(Self { records })
    }

    /// Apply after the exact SequenceManager plan used during preflight.
    pub(crate) fn apply(self, engine: &mut EngineInner) {
        for planned in self.records {
            debug_assert_eq!(
                engine
                    .orders
                    .sequence_manager
                    .current_element_for_actor(planned.entity)
                    .map(|(sequence_id, element_index)| {
                        SequenceElementRef::new(sequence_id, element_index)
                    }),
                planned.selected_element
            );
            if let Some(wait) = planned.wait_element {
                debug_assert!(
                    engine
                        .orders
                        .sequence_manager
                        .get_element(wait.sequence_id, wait.element_index)
                        .is_some()
                );
            }

            // `ActiveMelee` is a Rust execution cache; Original persists the
            // same information as the actor's selected strike element/order
            // plus the live sprite cursor. Rebuild it before the first resumed
            // Hourglass so a strike saved before its action-done frame still
            // delivers damage, while a strike saved after that frame cannot
            // deliver it twice.
            let restored_melee = planned.selected_element.and_then(|selected| {
                let element = engine
                    .orders
                    .sequence_manager
                    .get_element(selected.sequence_id, selected.element_index)?;
                let strike = SwordStrike::from_command(element.command)?;
                let target = match element.data {
                    SequenceElementData::Interaction {
                        antagonist: Some(target),
                    } => target,
                    _ => return None,
                };
                let order_id = element.orders.back()?.order_id;
                Some((selected, target, strike, order_id))
            });

            if let Some((selected, target, strike, order_id)) = restored_melee {
                let sprite = &engine
                    .world
                    .entities
                    .get(planned.entity)
                    .expect("preflighted melee actor disappeared")
                    .element_data()
                    .sprite;
                let sprite_is_driving = sprite.last_processed_order_id == order_id.get();
                let hit_applied =
                    sprite_is_driving && sprite.frames_from_now_till_action_done() <= 0;
                let mut melee = ActiveMelee::new(
                    target,
                    strike,
                    Some(selected.sequence_id),
                    selected.element_index,
                );
                melee.order_id = Some(order_id);
                melee.sprite_driving_hit = sprite_is_driving;
                melee.hit_applied = hit_applied;
                let actor = engine
                    .world
                    .entities
                    .get_mut(planned.entity)
                    .and_then(Entity::actor_data_mut)
                    .expect("preflighted melee actor ownership entity changed kind");
                actor.active_melee = melee;
            }
            engine
                .world
                .entities
                .get_mut(planned.entity)
                .and_then(Entity::actor_data_mut)
                .expect("preflighted actor ownership entity changed kind")
                .post_seek_sequence = planned.post_seek_sequence;
            if let Some(heap) = planned.vm_heap {
                engine
                    .scripts
                    .mission
                    .as_mut()
                    .expect("preflighted actor VM mission disappeared")
                    .replace_actor_vm_heap(ScriptHandleCodec::actor_handle(planned.entity), heap);
            }
        }
    }
}

fn resolve_owned_element(
    sequences: &LegacySequenceAdoptionPlan,
    creation_order: u32,
    owner: EntityId,
    field: &'static str,
    reference: super::payload_base::LegacySequenceElementRef,
) -> Result<Option<SequenceElementRef>, LegacyActorOwnershipAdoptError> {
    let Some((reference, element)) = sequences.resolve_element(field, reference)? else {
        return Ok(None);
    };
    if element.owner != Some(owner) {
        return Err(LegacyActorOwnershipAdoptError::WrongSequenceOwner {
            creation_order,
            field,
            reference,
            actual: element.owner,
            expected: owner,
        });
    }
    Ok(Some(reference))
}

fn actor_payload(payload: &LegacyElementPayload) -> Option<&LegacyActorPayload> {
    match payload {
        LegacyElementPayload::ActorPc(pc) => Some(&pc.human.actor),
        LegacyElementPayload::ActorNpcSoldier(soldier) => Some(&soldier.npc.human.actor),
        LegacyElementPayload::ActorNpcCivilian(civilian) => Some(&civilian.npc.human.actor),
        LegacyElementPayload::ObjectItem(_)
        | LegacyElementPayload::Bonus(_)
        | LegacyElementPayload::Scroll(_)
        | LegacyElementPayload::Target(_)
        | LegacyElementPayload::Fx(_)
        | LegacyElementPayload::FxMasked(_) => None,
    }
}
