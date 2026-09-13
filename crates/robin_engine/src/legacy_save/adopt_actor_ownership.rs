//! Post-sequence-manager adoption of common actor-element ownership.
//!
//! Saves retain identities for the selected sequence element, its
//! currently executing order, and the actor's idle wait element. Rust keeps
//! those objects in `SequenceManager` and derives selection from its exact
//! in-progress topology, so adoption validates that the converted manager
//! reconstructs the same relationships rather than adding a second source of
//! truth. The genuinely actor-owned post-seek sequence and script VM heap are
//! restored after that validation succeeds.

use crate::{
    element::{Command, Entity, EntityId, InstalledActorOrder},
    engine::EngineInner,
    natives::ScriptHandleCodec,
    sequence::{PostSeekSequence, SequenceElementRef},
};

use super::{
    adopt::missing_creation_order,
    adopt_common::{AdoptCtx, AdoptErrorKind, AdoptSite, LegacyAdoptError},
    adopt_object_leaves::{LegacyVmOwner, LegacyVmOwnerKind, preflight_vm},
    adopt_sequences::{LegacySequenceAdoptionPlan, convert_owner_local_sequence},
    adopt_vm_arena::LegacyVmArenaPlan,
    payload_base::LegacyActorPayload,
    payload_dispatch::{LegacyElementPayload, LegacyElementPayloadStream},
};

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
    installed_order: Option<InstalledActorOrder>,
    post_seek_sequence: Option<PostSeekSequence>,
    vm_heap: Option<Vec<u8>>,
}

impl LegacyActorOwnershipAdoptionPlan {
    /// Validate all actor pointers against the not-yet-installed converted
    /// SequenceManager, then convert actor-owned inline/script state.
    pub(crate) fn preflight(
        ctx: &AdoptCtx<'_>,
        payloads: &LegacyElementPayloadStream,
        sequences: &LegacySequenceAdoptionPlan,
        vm_arena: &LegacyVmArenaPlan,
    ) -> Result<Self, LegacyAdoptError> {
        let AdoptCtx {
            engine,
            entities,
            sequence_topology,
            ..
        } = *ctx;
        let mut records = Vec::new();
        for record in &payloads.records {
            let Some(saved) = actor_payload(&record.payload) else {
                continue;
            };
            let creation_order = record.header.creation_order;
            let site = AdoptSite::element("saved actor", creation_order);
            let entity = entities
                .by_creation_order
                .get(&creation_order)
                .copied()
                .ok_or_else(|| missing_creation_order(creation_order))?;
            let runtime =
                engine.world.entities.get(entity).ok_or_else(|| {
                    site.error(AdoptErrorKind::MissingEntity { entity_id: entity })
                })?;
            if !runtime.is_actor() {
                return Err(site.error(AdoptErrorKind::WrongEntityKind {
                    entity_id: entity,
                    expected: "actor",
                }));
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
                    return Err(site.field_error(
                        "wait_sequence_element",
                        AdoptErrorKind::WrongWaitCommand {
                            command: element.command,
                        },
                    ));
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
                return Err(site.error(AdoptErrorKind::SelectedElementMismatch {
                    saved: selected_element,
                    runtime: reconstructed,
                }));
            }

            let resolved_order = sequences.resolve_order("order", saved.order)?;
            let installed_order = resolved_order.map(|(_, _, order)| InstalledActorOrder {
                order_id: order.order_id,
                order_type: order.order_type,
            });
            match (selected_element, resolved_order) {
                (None, Some(_)) => {
                    return Err(site.error(AdoptErrorKind::OrderWithoutElement));
                }
                (Some(selected), Some((order_element, order_index, _))) => {
                    if order_element != selected {
                        return Err(site.error(AdoptErrorKind::OrderElementMismatch {
                            order_element,
                            order_index,
                            selected: Some(selected),
                        }));
                    }
                    // Rust pops completed orders from the front just like
                    // sequence progression; the executing order must be
                    // the front of the restored queue.
                    if order_index != 0 {
                        return Err(site.error(AdoptErrorKind::OrderCursorMismatch { order_index }));
                    }
                }
                (Some(selected), None) => {
                    let (_, element) = sequences
                        .resolve_element("sequence_element", saved.sequence_element)?
                        .expect("non-null preflighted selected element disappeared");
                    if element.current_order().is_some() {
                        return Err(site.error(AdoptErrorKind::MissingOrder { selected }));
                    }
                }
                (None, None) => {}
            }

            let post_seek_sequence = saved
                .post_seek_sequence
                .as_ref()
                .map(|sequence| {
                    convert_owner_local_sequence(sequence, entities, sequence_topology).and_then(
                        |sequence| {
                            sequence.try_into_post_seek().map_err(|_| {
                                AdoptSite::new("saved sequence").invalid(
                                    "actor.post_seek_sequence",
                                    "nested continuation",
                                    "at most one post-seek level",
                                )
                            })
                        },
                    )
                })
                .transpose()?;
            let location_prefix =
                vm_arena.element_prefix(creation_order, saved.script_members.as_ref())?;
            let mut computed_locations = Vec::new();
            let vm_heap = preflight_vm(
                ctx,
                LegacyVmOwner {
                    entity,
                    creation_order,
                    kind: LegacyVmOwnerKind::Actor,
                },
                saved.script_members.as_ref(),
                location_prefix,
                &mut computed_locations,
            )?;
            records.push(PlannedActorOwnership {
                entity,
                selected_element,
                wait_element,
                installed_order,
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

            let actor = engine
                .world
                .entities
                .get_mut(planned.entity)
                .and_then(Entity::actor_data_mut)
                .expect("preflighted actor ownership entity changed kind");
            actor.installed_order = planned.installed_order;
            actor.post_seek_sequence = planned.post_seek_sequence;
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
) -> Result<Option<SequenceElementRef>, LegacyAdoptError> {
    let Some((reference, element)) = sequences.resolve_element(field, reference)? else {
        return Ok(None);
    };
    if element.owner != Some(owner) {
        return Err(
            AdoptSite::element("saved actor", creation_order).field_error(
                field,
                AdoptErrorKind::WrongSequenceOwner {
                    reference,
                    actual: element.owner,
                    expected: owner,
                },
            ),
        );
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
