//! Preflighted Original v48 sequence-manager adoption.
//!
//! The decoder has already checked ID uniqueness and pointer membership. This
//! module performs the second boundary: map Original identities into the
//! initialized Rust mission, convert every represented field, retain exact
//! Original-only values in compatibility sidecars, and only then replace the
//! live manager atomically.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    num::NonZeroU32,
};

use thiserror::Error;

use crate::{
    element::{ActionState, Command, EntityId, Posture},
    engine::{EngineInner, LevelAssets},
    gate::DoorIndex,
    jump_line::JumpLineIndex,
    order::{Order, OrderType},
    position_interface::SectorHandle,
    sequence::{
        Field, FieldValue, LegacyV48OrderState, LegacyV48SequenceElementState, MoveFlags, Sequence,
        SequenceElement, SequenceElementData, SequenceElementRef, SequenceId,
        SequenceManagerV48State, SequencePriority, SequenceState,
    },
};

use super::{
    adopt::LegacyEntityFixups,
    gate_topology::derive_legacy_gate_order,
    payload_base::{LegacyLineRef, LegacyOrderRef, LegacySectorRef},
    payload_dispatch::{LegacyElementPayload, LegacyElementPayloadStream},
    payload_sequences::{
        LegacyGateRef, LegacyGenericField, LegacyGenericFieldKind, LegacyGenericFieldValue,
        LegacyInlineOrder, LegacyInlineSequence, LegacyInlineSequenceElement,
    },
    post_sequence_manager::LegacySequenceManagerState,
};

/// Initialized mission pointer spaces used by sequence payloads.
#[derive(Clone, Debug, Default)]
pub struct LegacySequenceTopology {
    /// Complete Original `marrayGates` order mapped to Rust door/gate IDs.
    pub gates: Vec<DoorIndex>,
    /// Exact Original `(layer, index-in-layer)` line identity.
    pub lines: BTreeMap<(u16, i16), JumpLineIndex>,
    /// The sole jump line on layers where identity remains unambiguous even
    /// when another retail edition inserted ordinary lines before it.
    pub unique_line_by_layer: BTreeMap<u16, JumpLineIndex>,
    /// Original sparse `marraySectors` slot to Rust's compact runtime sector.
    /// Constructor holes and non-position sector objects remain `None`.
    pub sectors: Vec<Option<SectorHandle>>,
    jump_pairs: Vec<LegacyJumpPair>,
    saved_actor_locations: BTreeMap<u32, LegacyActorLocation>,
}

#[derive(Clone, Copy, Debug)]
struct LegacyJumpPair {
    source: JumpLineIndex,
    destination: JumpLineIndex,
    source_layer: u16,
    destination_layer: u16,
    source_a: crate::coordinates::MapPoint,
    source_b: crate::coordinates::MapPoint,
}

#[derive(Clone, Copy, Debug)]
struct LegacyActorLocation {
    map: crate::coordinates::MapPoint,
}

impl LegacySequenceTopology {
    /// Reconstruct the mission-created pointer spaces used by sequences.
    ///
    /// Original gate order includes both stateful `RHDoor` objects and
    /// stateless `RHGateJump` objects. Rust represents both in its door table,
    /// but initializes reinforcement doors before attaching jump gates. Map
    /// the stable per-kind construction order instead of assuming the two
    /// mixed arrays have identical indices.
    pub fn derive(
        engine: &EngineInner,
        assets: &LevelAssets,
        payloads: &LegacyElementPayloadStream,
    ) -> Result<Self, LegacySequenceAdoptError> {
        let retained = assets.legacy_grid_topology.as_ref().ok_or_else(|| {
            LegacySequenceAdoptError::MissingTopology {
                field: "sequence.topology",
                identity: "retained Original grid topology".to_owned(),
            }
        })?;
        let gates =
            derive_legacy_gate_order(&retained.gates, &engine.script_domains.interactables.doors)
                .map_err(|error| LegacySequenceAdoptError::MissingTopology {
                field: "sequence.topology.gates",
                identity: error.to_string(),
            })?;

        if retained.jump_line_identities.len() != engine.world.fast_grid.level.jump_lines.len() {
            return Err(LegacySequenceAdoptError::MissingTopology {
                field: "sequence.topology.lines",
                identity: format!(
                    "retained {} Original jump-line identities for {} runtime jump lines",
                    retained.jump_line_identities.len(),
                    engine.world.fast_grid.level.jump_lines.len()
                ),
            });
        }
        let mut lines = BTreeMap::new();
        let mut unique_line_by_layer = BTreeMap::<u16, Option<JumpLineIndex>>::new();
        for (runtime_index, (&(layer, index_in_layer), line)) in retained
            .jump_line_identities
            .iter()
            .zip(&engine.world.fast_grid.level.jump_lines)
            .enumerate()
        {
            if line.layer != layer {
                return Err(LegacySequenceAdoptError::MissingTopology {
                    field: "sequence.topology.lines",
                    identity: format!(
                        "retained jump line {runtime_index} names layer {layer}, runtime uses {}",
                        line.layer
                    ),
                });
            }
            let runtime_index = u32::try_from(runtime_index).map_err(|_| {
                LegacySequenceAdoptError::MissingTopology {
                    field: "sequence.topology.lines",
                    identity: "runtime jump-line index exceeds u32".to_owned(),
                }
            })?;
            let handle = JumpLineIndex::new(runtime_index).ok_or_else(|| {
                LegacySequenceAdoptError::MissingTopology {
                    field: "sequence.topology.lines",
                    identity: "runtime jump-line index equals null sentinel".to_owned(),
                }
            })?;
            lines.insert((layer, index_in_layer), handle);
            unique_line_by_layer
                .entry(layer)
                .and_modify(|unique| *unique = None)
                .or_insert(Some(handle));
        }
        let unique_line_by_layer = unique_line_by_layer
            .into_iter()
            .filter_map(|(layer, handle)| handle.map(|handle| (layer, handle)))
            .collect();

        let jump_lines = &engine.world.fast_grid.level.jump_lines;
        let mut jump_pairs = Vec::new();
        for (source_index, source) in jump_lines.iter().enumerate() {
            let source_index_u32 = u32::try_from(source_index).map_err(|_| {
                LegacySequenceAdoptError::MissingTopology {
                    field: "sequence.topology.lines",
                    identity: format!("runtime jump-line index {source_index} exceeds u32"),
                }
            })?;
            let Some(destination_index) = source.associated_line_index else {
                continue;
            };
            let Some(destination) = jump_lines.get(destination_index as usize) else {
                return Err(LegacySequenceAdoptError::MissingTopology {
                    field: "sequence.topology.lines",
                    identity: format!(
                        "jump line {source_index} associates with absent runtime line {destination_index}"
                    ),
                });
            };
            if destination.associated_line_index != Some(source_index_u32) {
                return Err(LegacySequenceAdoptError::MissingTopology {
                    field: "sequence.topology.lines",
                    identity: format!(
                        "jump lines {source_index} and {destination_index} are not reciprocal"
                    ),
                });
            }
            let source_handle = JumpLineIndex::new(source_index_u32).ok_or_else(|| {
                LegacySequenceAdoptError::MissingTopology {
                    field: "sequence.topology.lines",
                    identity: "runtime jump-line index equals null sentinel".to_owned(),
                }
            })?;
            let destination_handle = JumpLineIndex::new(destination_index).ok_or_else(|| {
                LegacySequenceAdoptError::MissingTopology {
                    field: "sequence.topology.lines",
                    identity: "runtime associated jump-line index equals null sentinel".to_owned(),
                }
            })?;
            jump_pairs.push(LegacyJumpPair {
                source: source_handle,
                destination: destination_handle,
                source_layer: source.layer,
                destination_layer: destination.layer,
                source_a: source.point_a,
                source_b: source.point_b,
            });
        }

        let saved_actor_locations = payloads
            .records
            .iter()
            .filter_map(|record| {
                let position = match &record.payload {
                    LegacyElementPayload::ActorPc(pc) => &pc.human.actor.element.sprite.position,
                    LegacyElementPayload::ActorNpcSoldier(soldier) => {
                        &soldier.npc.human.actor.element.sprite.position
                    }
                    LegacyElementPayload::ActorNpcCivilian(civilian) => {
                        &civilian.npc.human.actor.element.sprite.position
                    }
                    _ => return None,
                };
                Some((
                    record.header.creation_order,
                    LegacyActorLocation {
                        map: crate::coordinates::MapPoint::new(position.map.x, position.map.y),
                    },
                ))
            })
            .collect();

        if retained.position_sector_numbers.len() != retained.sectors.len() {
            return Err(LegacySequenceAdoptError::MissingTopology {
                field: "sequence.topology.sectors",
                identity: format!(
                    "retained position-sector map has {} entries for {} sparse Original slots",
                    retained.position_sector_numbers.len(),
                    retained.sectors.len()
                ),
            });
        }
        let sectors = retained
            .position_sector_numbers
            .iter()
            .copied()
            .map(|number| {
                number
                    .map(|number| {
                        let raw = u16::try_from(number).map_err(|_| {
                            LegacySequenceAdoptError::MissingTopology {
                                field: "sequence.topology.sectors",
                                identity: format!(
                                    "runtime position-sector number {number} is negative"
                                ),
                            }
                        })?;
                        SectorHandle::new(raw).ok_or_else(|| {
                            LegacySequenceAdoptError::MissingTopology {
                                field: "sequence.topology.sectors",
                                identity:
                                    "runtime position-sector number equals null sentinel 0xffff"
                                        .to_owned(),
                            }
                        })
                    })
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            gates,
            lines,
            unique_line_by_layer,
            sectors,
            jump_pairs,
            saved_actor_locations,
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LegacySequenceAdoptError {
    #[error("saved sequence field {field} has value {value}; expected {expected}")]
    InvalidField {
        field: &'static str,
        value: String,
        expected: &'static str,
    },
    #[error("saved sequence reference {field} names absent ID {id}")]
    MissingIdentity { field: &'static str, id: u32 },
    #[error(
        "saved sequence topology reference {field} names {identity}, absent from the initialized mission"
    )]
    MissingTopology {
        field: &'static str,
        identity: String,
    },
    #[error("saved sequence {sequence_id} has duplicate generic field {field:?}")]
    DuplicateGenericField { sequence_id: u32, field: Field },
}

/// Fully converted manager state. Construction is read-only; applying it is
/// infallible and replaces the manager plus its order-ID counter together.
#[derive(Debug)]
pub(crate) struct LegacySequenceAdoptionPlan {
    manager: SequenceManagerV48State,
    next_order_id: u32,
}

impl LegacySequenceAdoptionPlan {
    pub(crate) fn apply(self, engine: &mut crate::engine::EngineInner) {
        engine
            .orders
            .sequence_manager
            .restore_v48_state(self.manager);
        engine.orders.next_order_id = self.next_order_id;
    }

    /// Resolve a tail-owned Original sequence-element pointer against the
    /// exact converted manager that will be installed. Tail preflight runs
    /// before this plan is consumed, so no live-manager fallback is needed.
    pub(crate) fn resolve_element(
        &self,
        field: &'static str,
        reference: super::payload_base::LegacySequenceElementRef,
    ) -> Result<Option<(SequenceElementRef, &SequenceElement)>, LegacySequenceAdoptError> {
        let Some(id) = reference.0 else {
            return Ok(None);
        };
        let (element_ref, element) = self
            .manager
            .sequences
            .iter()
            .find_map(|sequence| {
                sequence
                    .elements
                    .iter()
                    .enumerate()
                    .find(|(_, element)| element.id == id)
                    .map(|(element_index, element)| {
                        (SequenceElementRef::new(sequence.id, element_index), element)
                    })
            })
            .ok_or(LegacySequenceAdoptError::MissingIdentity { field, id })?;
        Ok(Some((element_ref, element)))
    }

    /// Resolve an Original order pointer in the manager-owned identity space.
    ///
    /// Rust reserves zero as the null `NonZeroU32` value, so sequence
    /// conversion maps every Original order ID bijectively to `id + 1`.
    pub(crate) fn resolve_order(
        &self,
        field: &'static str,
        reference: LegacyOrderRef,
    ) -> Result<Option<(SequenceElementRef, usize, &Order)>, LegacySequenceAdoptError> {
        let Some(original_id) = reference.0 else {
            return Ok(None);
        };
        let runtime_id = original_id
            .checked_add(1)
            .ok_or_else(|| invalid(field, original_id, "at most 0xfffffffe"))?;
        let resolved = self.manager.sequences.iter().find_map(|sequence| {
            sequence
                .elements
                .iter()
                .enumerate()
                .find_map(|(element_index, element)| {
                    element
                        .orders
                        .iter()
                        .enumerate()
                        .find(|(_, order)| order.order_id.get() == runtime_id)
                        .map(|(order_index, order)| {
                            (
                                SequenceElementRef::new(sequence.id, element_index),
                                order_index,
                                order,
                            )
                        })
                })
        });
        resolved
            .map(Some)
            .ok_or(LegacySequenceAdoptError::MissingIdentity {
                field,
                id: original_id,
            })
    }

    /// Actor selection reconstructed by the converted manager's canonical
    /// in-progress index.
    pub(crate) fn current_element_for_actor(&self, actor: EntityId) -> Option<SequenceElementRef> {
        let mut in_progress = self.manager.sequences.iter().flat_map(|sequence| {
            sequence
                .elements
                .iter()
                .enumerate()
                .filter(move |(_, element)| {
                    element.owner == Some(actor) && element.state == SequenceState::InProgress
                })
                .map(move |(element_index, element)| {
                    (
                        SequenceElementRef::new(sequence.id, element_index),
                        element.command,
                    )
                })
        });
        let first = in_progress.next()?;
        let Some(second) = in_progress.next() else {
            return Some(first.0);
        };
        std::iter::once(first)
            .chain(std::iter::once(second))
            .chain(in_progress)
            .find_map(|(reference, command)| (command != Command::Wait).then_some(reference))
            .or(Some(first.0))
    }
}

/// Convert every manager-owned sequence and deferred element without mutating
/// the initialized engine.
pub(crate) fn preflight_v48_sequence_manager(
    saved: &LegacySequenceManagerState,
    entities: &LegacyEntityFixups,
    topology: &LegacySequenceTopology,
) -> Result<LegacySequenceAdoptionPlan, LegacySequenceAdoptError> {
    let sequence_ids = saved
        .sequences
        .iter()
        .map(|saved| (saved.body.unique_id.0, SequenceId(saved.body.unique_id.0)))
        .collect::<BTreeMap<_, _>>();
    let mut element_refs = BTreeMap::new();
    for saved_sequence in &saved.sequences {
        let sequence_id = SequenceId(saved_sequence.body.unique_id.0);
        for (element_index, element) in saved_sequence.body.elements.iter().enumerate() {
            element_refs.insert(
                element.base().unique_id.0,
                SequenceElementRef::new(sequence_id, element_index),
            );
        }
    }

    let mut sequences = Vec::with_capacity(saved.sequences.len());
    for saved_sequence in &saved.sequences {
        sequences.push(convert_sequence(
            &saved_sequence.body,
            true,
            entities,
            topology,
            &sequence_ids,
            &element_refs,
        )?);
    }

    let mut elements_to_go = VecDeque::with_capacity(saved.deferred_elements.len());
    for deferred in &saved.deferred_elements {
        let id = deferred
            .element
            .0
            .expect("decoder rejects null manager deferred elements");
        let reference = resolve_element_ref("deferred_elements", Some(id), &element_refs)?
            .expect("non-null reference resolves to Some");
        elements_to_go.push_back((reference.sequence_id, reference.element_index));
    }

    Ok(LegacySequenceAdoptionPlan {
        manager: SequenceManagerV48State {
            sequences,
            elements_to_go,
            next_sequence_id: saved.static_ids.sequence_next_id,
            next_element_id: saved.static_ids.sequence_element_next_id,
        },
        // Original order ID zero is valid, while Rust uses NonZeroU32.
        // Shift the complete identity domain by one; this is bijective and
        // lets later pointer-adoption slices map `legacy_id -> legacy_id + 1`.
        next_order_id: saved
            .static_ids
            .order_next_id
            .checked_add(1)
            .ok_or_else(|| invalid("order_static.next_id", u32::MAX, "at most 0xfffffffe"))?,
    })
}

fn convert_sequence(
    saved: &LegacyInlineSequence,
    manager_owned: bool,
    entities: &LegacyEntityFixups,
    topology: &LegacySequenceTopology,
    sequence_ids: &BTreeMap<u32, SequenceId>,
    element_refs: &BTreeMap<u32, SequenceElementRef>,
) -> Result<Sequence, LegacySequenceAdoptError> {
    let cursor = usize::from(saved.sequence_element_cursor);
    if cursor > saved.elements.len() {
        return Err(invalid(
            "sequence_element_cursor",
            cursor,
            "an index at or before elements.len()",
        ));
    }

    let mut elements = Vec::with_capacity(saved.elements.len());
    for element in &saved.elements {
        elements.push(convert_element(
            saved.unique_id.0,
            element,
            manager_owned,
            entities,
            topology,
            sequence_ids,
            element_refs,
        )?);
    }
    for pair in elements.windows(2) {
        let previous = pair[0].command_level;
        let current = pair[1].command_level;
        if current != previous && previous.checked_add(1) != Some(current) {
            return Err(invalid(
                "command_level",
                current,
                "the previous command level or exactly its successor",
            ));
        }
    }

    Ok(Sequence::restore_v48_state(
        SequenceId(saved.unique_id.0),
        elements,
        cursor,
        saved.current_command_level,
        saved.running_elements,
        saved.elements_in_progress,
        saved.started,
    ))
}

/// Convert one owner-local sequence which is serialized inline rather than
/// owned by `RHSequenceManager`.
///
/// PC quick-action and quick-seek slots use `Serialize(file, false)`, so their
/// element-link fields are deliberately absent and must not be resolved
/// against the manager pointer space.
pub(crate) fn convert_owner_local_sequence(
    saved: &LegacyInlineSequence,
    entities: &LegacyEntityFixups,
    topology: &LegacySequenceTopology,
) -> Result<Sequence, LegacySequenceAdoptError> {
    convert_sequence(
        saved,
        false,
        entities,
        topology,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
}

#[allow(clippy::too_many_arguments)]
fn convert_element(
    sequence_id: u32,
    saved: &LegacyInlineSequenceElement,
    manager_owned: bool,
    entities: &LegacyEntityFixups,
    topology: &LegacySequenceTopology,
    sequence_ids: &BTreeMap<u32, SequenceId>,
    element_refs: &BTreeMap<u32, SequenceElementRef>,
) -> Result<SequenceElement, LegacySequenceAdoptError> {
    let base = saved.base();
    let command = Command::try_from(base.command)
        .map_err(|_| invalid("command", base.command, "a known RHcommand discriminant"))?;
    let owner = base
        .owner
        .0
        .map(|id| {
            entities
                .by_creation_order
                .get(&id)
                .or_else(|| entities.mobile_owner_by_creation_order.get(&id))
                .copied()
                .ok_or(LegacySequenceAdoptError::MissingIdentity { field: "owner", id })
        })
        .transpose()?;
    let state = sequence_state(base.state)?;
    let priority = sequence_priority(base.priority)?;
    // Original's RHSequenceElement constructors do not initialize either
    // transition result. Actor::Instruct stamps both from the actor's current
    // state before GenerateTransition or command translation can read them.
    // Only an actor-owned INPROGRESS element can expose the stored results to
    // execution. TODO and POSTPONED elements pass through Instruct (and are
    // stamped) before executing; terminal states never execute again.
    // Ownerless engine commands never consult actor-transition fields, and a
    // NULL actor command terminates before the stamp or any transition use.
    let transition_results_dormant =
        owner.is_none() || command == Command::Null || state != SequenceState::InProgress;
    let (posture_after_transition, raw_dormant_posture_after_transition) =
        convert_transition_result(
            "posture_after_transition",
            base.posture_after_transition,
            "RHposture 0..24",
            transition_results_dormant,
            Posture::Undefined,
            |raw| {
                u32::try_from(raw)
                    .ok()
                    .and_then(|raw| Posture::try_from(raw).ok())
            },
        )?;
    let (action_state_after_transition, raw_dormant_action_state_after_transition) =
        convert_transition_result(
            "action_state_after_transition",
            base.action_state_after_transition,
            "RHactionState 0..17",
            transition_results_dormant,
            ActionState::Waiting,
            |raw| {
                u32::try_from(raw)
                    .ok()
                    .and_then(|raw| ActionState::try_from(raw).ok())
            },
        )?;

    let (mut orders, order_state) = convert_orders(&base.orders, entities)?;
    preserve_translated_turn_direction(command, &mut orders);
    let mut generic_raw_unions = Vec::new();
    let mut raw_dormant_movement_action = None;
    let (data, linked_seek, damage_arrow, raw_sword_strike) = match saved {
        LegacyInlineSequenceElement::Simple(_) => (SequenceElementData::Simple, None, None, None),
        LegacyInlineSequenceElement::Interaction(interaction) => (
            SequenceElementData::Interaction {
                antagonist: resolve_entity("interaction.element", interaction.element, entities)?,
            },
            None,
            None,
            None,
        ),
        LegacyInlineSequenceElement::Damage(damage) => {
            let raw_strike = damage.sword_strike;
            let sword_strike = match raw_strike {
                0 => Some(crate::weapons::SwordStrike::A),
                1 => Some(crate::weapons::SwordStrike::B),
                2 => Some(crate::weapons::SwordStrike::C),
                3 => Some(crate::weapons::SwordStrike::D),
                4 => Some(crate::weapons::SwordStrike::E),
                5 => Some(crate::weapons::SwordStrike::F),
                6 => Some(crate::weapons::SwordStrike::G),
                7 => Some(crate::weapons::SwordStrike::H),
                8 => Some(crate::weapons::SwordStrike::I),
                9 => Some(crate::weapons::SwordStrike::Charge),
                10..=14 => None,
                value => {
                    return Err(invalid("damage.sword_strike", value, "RHSwordStrike 0..14"));
                }
            };
            let sword_profile_idx = damage.sword.map(|profile| profile.0);
            (
                SequenceElementData::Damage {
                    origin: resolve_entity("damage.origin", damage.origin, entities)?,
                    // Keep this in the legacy sidecar below as well: old
                    // adopted elements preserve the exact decoded payload,
                    // while live runtime elements use this typed field.
                    projectile: None,
                    damage: damage.damage,
                    concussion: damage.concussion,
                    sword_strike,
                    sword_profile_idx,
                    is_harder_hit: damage.harder_hit,
                },
                None,
                resolve_entity("damage.arrow", damage.arrow, entities)?,
                Some(raw_strike),
            )
        }
        LegacyInlineSequenceElement::Generic(generic) => {
            let mut properties = HashMap::with_capacity(generic.fields.len());
            let jump_pair = if command == Command::JumpCmd {
                let source = generic.fields.iter().find_map(|field| {
                    (field.kind == LegacyGenericFieldKind::JumpLineSource).then_some(&field.value)
                });
                let destination = generic.fields.iter().find_map(|field| {
                    (field.kind == LegacyGenericFieldKind::JumpLineDestination)
                        .then_some(&field.value)
                });
                match (source, destination) {
                    (
                        Some(LegacyGenericFieldValue::Line(source)),
                        Some(LegacyGenericFieldValue::Line(destination)),
                    ) => Some(resolve_jump_pair(
                        *source,
                        *destination,
                        base.owner,
                        topology,
                    )?),
                    (None, None) => None,
                    _ => {
                        return Err(LegacySequenceAdoptError::MissingTopology {
                            field: "generic.jump_lines",
                            identity: "JumpCmd requires line-valued source and destination fields"
                                .to_owned(),
                        });
                    }
                }
            } else {
                None
            };
            for field in &generic.fields {
                let converted_jump_line =
                    jump_pair.and_then(|(source, destination)| match field.kind {
                        LegacyGenericFieldKind::JumpLineSource => Some(source),
                        LegacyGenericFieldKind::JumpLineDestination => Some(destination),
                        _ => None,
                    });
                let (kind, value, raw) = if let Some(line) = converted_jump_line {
                    let kind = match field.kind {
                        LegacyGenericFieldKind::JumpLineSource => Field::JumplineSource,
                        LegacyGenericFieldKind::JumpLineDestination => Field::JumplineDestination,
                        _ => unreachable!("converted jump line is only produced for jump fields"),
                    };
                    (kind, FieldValue::LineId(line), None)
                } else {
                    convert_generic_field(field, entities, topology, sequence_id).inspect_err(
                        |_error| {
                            tracing::error!(
                                sequence_id,
                                ?command,
                                ?state,
                                element_id = base.unique_id.0,
                                field = ?field.kind,
                                value = ?field.value,
                                "failed to convert saved generic sequence field"
                            );
                        },
                    )?
                };
                if properties.insert(kind, value).is_some() {
                    return Err(LegacySequenceAdoptError::DuplicateGenericField {
                        sequence_id,
                        field: kind,
                    });
                }
                if let Some(raw) = raw {
                    generic_raw_unions.push((kind, raw));
                }
            }
            (
                SequenceElementData::Generic { properties },
                None,
                None,
                None,
            )
        }
        LegacyInlineSequenceElement::Movement(movement) => {
            let flags = MoveFlags::from_bits(movement.flags).ok_or_else(|| {
                invalid(
                    "movement.flags",
                    movement.flags,
                    "known RHmovementFlags bits",
                )
            })?;
            let (action, raw_dormant_action) =
                convert_movement_action(command, state, movement.action)?;
            raw_dormant_movement_action = raw_dormant_action;
            let post_seek_sequence = movement
                .post_seek_sequence
                .as_deref()
                .map(|sequence| {
                    convert_sequence(
                        sequence,
                        false,
                        entities,
                        topology,
                        sequence_ids,
                        element_refs,
                    )
                    .and_then(|sequence| {
                        sequence.try_into_post_seek().map_err(|_| {
                            invalid(
                                "movement.post_seek_sequence",
                                "nested continuation",
                                "at most one post-seek level",
                            )
                        })
                    })
                })
                .transpose()?;
            let linked_seek = if manager_owned {
                Some(resolve_element_ref(
                    "movement.linked_seek",
                    movement
                        .manager_linked_seek_fixup
                        .as_ref()
                        .expect("decoder requires manager movement fixup")
                        .0,
                    element_refs,
                )?)
            } else {
                None
            };
            (
                SequenceElementData::Movement {
                    destination: crate::coordinates::MapPoint::new(
                        movement.destination.x,
                        movement.destination.y,
                    ),
                    layer: movement.layer,
                    sector: resolve_sector(movement.sector, topology)?,
                    gate_id: resolve_gate("movement.gate", movement.gate, topology)?,
                    line_id: resolve_line("movement.line", movement.line, topology)?,
                    element: resolve_entity("movement.element", movement.element, entities)?,
                    flags,
                    tolerance: movement.tolerance,
                    direction: movement.direction,
                    action,
                    speed_factor: movement.speed_factor,
                    post_seek_sequence,
                },
                linked_seek,
                None,
                None,
            )
        }
    };

    let (next, postponed, mummy) = if manager_owned {
        let fixups = base
            .manager_fixups
            .as_ref()
            .expect("decoder requires manager fixups");
        (
            resolve_element_ref("element.next", fixups.next.0, element_refs)?,
            resolve_element_ref("element.postponed", fixups.postponed.0, element_refs)?,
            resolve_sequence_ref("element.mummy", fixups.mummy.0, sequence_ids)?,
        )
    } else {
        (None, None, None)
    };

    let mut element = SequenceElement::new(base.command_level, command, owner);
    element.id = base.unique_id.0;
    element.state = state;
    element.priority = priority;
    element.script_driven = base.script_driven;
    element.posture_after_transition = posture_after_transition;
    element.action_state_after_transition = action_state_after_transition;
    element.orders = orders.into();
    element.data = data;
    if let Some(postponed) = postponed {
        if postponed.sequence_id == SequenceId(sequence_id) {
            element.postponed_element_index = Some(postponed.element_index);
        } else {
            element.cross_postponed = Some((postponed.sequence_id, postponed.element_index));
        }
    }
    element.legacy_v48 = Some(LegacyV48SequenceElementState {
        deleted: base.deleted,
        script_driven: base.script_driven,
        raw_dormant_posture_after_transition,
        raw_dormant_action_state_after_transition,
        next,
        postponed,
        mummy,
        linked_seek,
        damage_arrow,
        raw_sword_strike,
        raw_dormant_movement_action,
        order_state,
        generic_raw_unions,
    });
    Ok(element)
}

/// Resolve the two pointers written together by Original's JumpCmd builder.
///
/// Original stores the approached line and then its reciprocal associated
/// line (`RHengine.cpp` JumpCmd construction). Some retail data editions add
/// ordinary boundaries before those jump lines, shifting the serialized
/// per-layer ordinal without changing the actual jump geometry. Exact
/// identities remain authoritative. For shifted identities, reciprocal layer
/// topology narrows the candidates and the saved owner's position identifies
/// the approached segment isomorphically.
fn resolve_jump_pair(
    source: LegacyLineRef,
    destination: LegacyLineRef,
    owner: super::payload_base::LegacyElementRef,
    topology: &LegacySequenceTopology,
) -> Result<(JumpLineIndex, JumpLineIndex), LegacySequenceAdoptError> {
    let (Some(source_layer), Some(source_index)) = (source.layer, source.index) else {
        return Err(invalid(
            "generic.jump_line_source",
            format!("{:?}", (source.layer, source.index)),
            "a non-null layer plus non-negative line index",
        ));
    };
    let (Some(destination_layer), Some(destination_index)) = (destination.layer, destination.index)
    else {
        return Err(invalid(
            "generic.jump_line_destination",
            format!("{:?}", (destination.layer, destination.index)),
            "a non-null layer plus non-negative line index",
        ));
    };
    if source_index < 0 || destination_index < 0 {
        return Err(invalid(
            "generic.jump_lines",
            format!("source index {source_index}, destination index {destination_index}"),
            "non-negative line indices",
        ));
    }

    let exact_source = topology.lines.get(&(source_layer, source_index)).copied();
    let exact_destination = topology
        .lines
        .get(&(destination_layer, destination_index))
        .copied();
    let mut candidates = topology
        .jump_pairs
        .iter()
        .copied()
        .filter(|pair| {
            pair.source_layer == source_layer
                && pair.destination_layer == destination_layer
                && exact_source.is_none_or(|line| pair.source == line)
                && exact_destination.is_none_or(|line| pair.destination == line)
        })
        .collect::<Vec<_>>();

    if candidates.len() == 1 {
        let pair = candidates[0];
        return Ok((pair.source, pair.destination));
    }
    if candidates.is_empty() {
        return Err(LegacySequenceAdoptError::MissingTopology {
            field: "generic.jump_lines",
            identity: format!(
                "reciprocal jump pair from layer {source_layer}, index {source_index} to layer {destination_layer}, index {destination_index}"
            ),
        });
    }

    let owner_id = owner.0.ok_or_else(|| LegacySequenceAdoptError::MissingTopology {
        field: "generic.jump_lines",
        identity: format!(
            "{} reciprocal layer {source_layer}->{destination_layer} pairs and no owner geometry",
            candidates.len()
        ),
    })?;
    let owner_location = topology.saved_actor_locations.get(&owner_id).ok_or_else(|| {
        LegacySequenceAdoptError::MissingTopology {
            field: "generic.jump_lines",
            identity: format!(
                "{} reciprocal layer {source_layer}->{destination_layer} pairs and no saved actor geometry for owner {owner_id}",
                candidates.len()
            ),
        }
    })?;
    candidates.sort_by(|left, right| {
        squared_distance_to_segment(owner_location.map, left.source_a, left.source_b).total_cmp(
            &squared_distance_to_segment(owner_location.map, right.source_a, right.source_b),
        )
    });
    let best_distance = squared_distance_to_segment(
        owner_location.map,
        candidates[0].source_a,
        candidates[0].source_b,
    );
    let second_distance = squared_distance_to_segment(
        owner_location.map,
        candidates[1].source_a,
        candidates[1].source_b,
    );
    if best_distance == second_distance {
        return Err(LegacySequenceAdoptError::MissingTopology {
            field: "generic.jump_lines",
            identity: format!(
                "ambiguous reciprocal layer {source_layer}->{destination_layer} pair for owner {owner_id} at ({}, {}): equal squared distance {best_distance}",
                owner_location.map.x, owner_location.map.y
            ),
        });
    }
    Ok((candidates[0].source, candidates[0].destination))
}

fn squared_distance_to_segment(
    point: crate::coordinates::MapPoint,
    start: crate::coordinates::MapPoint,
    end: crate::coordinates::MapPoint,
) -> f32 {
    let segment_x = end.x - start.x;
    let segment_y = end.y - start.y;
    let length_squared = segment_x * segment_x + segment_y * segment_y;
    if length_squared == 0.0 {
        return (point.x - start.x).powi(2) + (point.y - start.y).powi(2);
    }
    let projection = (((point.x - start.x) * segment_x + (point.y - start.y) * segment_y)
        / length_squared)
        .clamp(0.0, 1.0);
    let closest_x = start.x + projection * segment_x;
    let closest_y = start.y + projection * segment_y;
    (point.x - closest_x).powi(2) + (point.y - closest_y).powi(2)
}

/// Original `TURN`, `TURN_FAST`, and `TURN_ELEMENT` translation writes the
/// actor's direction goal once, then appends a `TURNING` order. The serialized
/// `RHOrder::bComputeDirection` remains at its default `true`, but Original
/// never interprets that flag as a request to recompute the turn goal from the
/// order's dormant `(0, 0)` destination on every engine frame.
///
/// Rust's AI turn sweep does use `Order::compute_direction` for positional
/// turns, so carrying the raw flag across would overwrite the authoritative
/// serialized goal as soon as a mid-turn save resumes. Runtime translation
/// already emits these command-owned orders with the flag cleared; normalize
/// restored orders to the same semantic representation.
fn preserve_translated_turn_direction(command: Command, orders: &mut [Order]) {
    if !matches!(
        command,
        Command::Turn | Command::TurnFast | Command::TurnElement
    ) {
        return;
    }
    for order in orders {
        if order.order_type == OrderType::Turning {
            order.compute_direction = false;
        }
    }
}

fn convert_transition_result<T>(
    field: &'static str,
    raw: i32,
    expected: &'static str,
    dormant: bool,
    dormant_runtime_value: T,
    convert: impl FnOnce(i32) -> Option<T>,
) -> Result<(T, Option<i32>), LegacySequenceAdoptError> {
    // These words are scratch output from the last time Instruct touched the
    // element, not authored state for a future instruction.  In particular,
    // a perfectly valid saved value can still be stale: Original
    // RHElementActor::Instruct overwrites both fields from the actor's live
    // posture/action state before it generates transitions.  Keep the raw
    // word for save fidelity, but leave the runtime sentinel in place so the
    // Rust Instruct boundary performs that same refresh.
    if dormant {
        return Ok((dormant_runtime_value, Some(raw)));
    }

    match convert(raw) {
        Some(value) => Ok((value, None)),
        None => Err(invalid(field, raw, expected)),
    }
}

fn convert_movement_action(
    command: Command,
    state: SequenceState,
    raw: i32,
) -> Result<(OrderType, Option<i32>), LegacySequenceAdoptError> {
    let converted = u32::try_from(raw)
        .ok()
        .and_then(|raw| OrderType::try_from(raw).ok());
    if let Some(action) = converted {
        return Ok((action, None));
    }

    // Four RHSequenceElementMovement paths cannot consume maction:
    //
    // * ASSERT_POSITION, WAIT_FREE_LIFT, and CHANGE_POSITION use constructors
    //   which never initialize it. Their translation branches inspect only
    //   position/sector/lift data.
    // * TELEPORT does initialize it, but ExecuteImmediately never reads it.
    //
    // Terminal elements are likewise never dispatched again: RHSequence::Go
    // only dispatches TODO/POSTPONED, while INPROGRESS is already executing.
    // Preserve the raw word for those proven-dormant cases and use Rust's
    // explicit unset action in the typed runtime slot. Commands whose movement
    // action drives transitions, path requests, seek refresh, or door orders
    // remain strict in every potentially executable state.
    let command_does_not_read_action = matches!(
        command,
        Command::AssertPosition
            | Command::WaitFreeLift
            | Command::Teleport
            | Command::ChangePosition
    );
    let terminal = matches!(
        state,
        SequenceState::Terminated
            | SequenceState::Done
            | SequenceState::Impossible
            | SequenceState::Interrupted
    );
    if command_does_not_read_action || terminal {
        return Ok((OrderType::Invalid, Some(raw)));
    }

    let expected = if raw < 0 {
        "a non-negative RHanimation"
    } else {
        "a known RHanimation"
    };
    Err(invalid("movement.action", raw, expected))
}

fn convert_orders(
    saved: &[LegacyInlineOrder],
    entities: &LegacyEntityFixups,
) -> Result<(Vec<Order>, Vec<LegacyV48OrderState>), LegacySequenceAdoptError> {
    let mut orders = Vec::with_capacity(saved.len());
    let mut retained = Vec::with_capacity(saved.len());
    for order in saved {
        let action =
            OrderType::try_from(u32::try_from(order.action).map_err(|_| {
                invalid("order.action", order.action, "a non-negative RHanimation")
            })?)
            .map_err(|_| invalid("order.action", order.action, "a known RHanimation"))?;
        let mapped_id = order
            .unique_id
            .0
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .ok_or_else(|| invalid("order.unique_id", order.unique_id.0, "at most 0xfffffffe"))?;
        let antagonist = resolve_entity("order.antagonist", order.antagonist, entities)?;
        let mut converted = Order::new(
            action,
            order.destination_2d.x,
            order.destination_2d.y,
            mapped_id,
        );
        converted.compute_direction = order.compute_direction;
        converted.tolerance = order.tolerance;
        converted.lock_ai = order.lock_ai;
        converted.apply_transition_at_this_point = order.apply_transition_at_this_point;
        converted.can_fly = order.can_fly;
        converted.transition = order.transition;
        converted.destination_3d = [
            order.destination_3d.x,
            order.destination_3d.y,
            order.destination_3d.z,
        ];
        converted.flight_vector = [order.flight_vector.x, order.flight_vector.y];
        converted.reverse = order.reverse;
        converted.target_actor = antagonist.map(EntityId::index);
        converted.antagonist = antagonist;
        orders.push(converted);
        retained.push(LegacyV48OrderState {
            legacy_id: order.unique_id.0,
        });
    }
    Ok((orders, retained))
}

fn convert_generic_field(
    saved: &LegacyGenericField,
    entities: &LegacyEntityFixups,
    topology: &LegacySequenceTopology,
    _sequence_id: u32,
) -> Result<(Field, FieldValue, Option<[u8; 12]>), LegacySequenceAdoptError> {
    use LegacyGenericFieldKind as K;
    let field = match saved.kind {
        K::Direction => Field::Direction,
        K::Event => Field::Event,
        K::Timer => Field::Timer,
        K::Message => Field::Message,
        K::MessageArgument => Field::MessageArgument,
        K::MessageExtendedArgument => Field::MessageExtendedArgument,
        K::BowTargetGuy => Field::BowTargetGuy,
        K::BowTargetPoint => Field::BowTargetPoint,
        K::CameraPoint => Field::CameraPoint,
        K::CameraZoomLevel => Field::CameraZoomLevel,
        K::CameraSpeed => Field::CameraSpeed,
        K::ActionId => Field::ActionId,
        K::ActionAvailable => Field::ActionAvailable,
        K::CharacterAvailable => Field::CharacterAvailable,
        K::ConcussionLevel => Field::ConcussionLevel,
        K::SpeakId => Field::SpeakId,
        K::SpeakFlags => Field::SpeakFlags,
        K::SpeakVariant => Field::SpeakVariant,
        K::DialogId => Field::DialogId,
        K::DialogSource => Field::DialogSource,
        K::PopupTextId => Field::PopupTextId,
        K::AnimationId => Field::AnimationId,
        K::MapDisplay => Field::MapDisplay,
        K::JumpLineSource => Field::JumplineSource,
        K::JumpLineDestination => Field::JumplineDestination,
        K::Amount => Field::Amount,
        K::ShieldDangerPoint => Field::ShieldDangerPoint,
        K::ShieldDangerPointLayer => Field::ShieldDangerPointLayer,
        K::ShieldProtected => Field::ShieldProtected,
        K::RollPoint => Field::RollPoint,
        K::PurseTarget => Field::PurseTarget,
        K::NetTarget => Field::NetTarget,
        K::WaspNestTarget => Field::WaspNestTarget,
        K::Opponent => Field::Opponent,
        K::SwordfightPrepared => Field::SwordfightPrepared,
        K::Gate => Field::Gate,
        K::Door => Field::Door,
        K::OldAnimation => Field::OldAnimation,
        K::NewAnimation => Field::NewAnimation,
        K::Freeze => Field::Freeze,
        K::Scroll => Field::Scroll,
        K::ScrollReader => Field::ScrollReader,
        K::ScrollOwner => Field::ScrollOwner,
    };
    let (value, raw) = match &saved.value {
        LegacyGenericFieldValue::Element(reference) => {
            let entity = resolve_entity("generic.element", *reference, entities)?;
            (
                entity
                    .map(FieldValue::Element)
                    .unwrap_or(FieldValue::OptionalElement(None)),
                None,
            )
        }
        LegacyGenericFieldValue::Line(reference) => {
            let line = resolve_line("generic.line", *reference, topology)?;
            (
                line.map(FieldValue::LineId)
                    .unwrap_or(FieldValue::OptionalLineId(None)),
                None,
            )
        }
        LegacyGenericFieldValue::Gate(reference) => {
            let gate = resolve_gate("generic.gate", *reference, topology)?;
            (
                gate.map(FieldValue::DoorId)
                    .unwrap_or(FieldValue::OptionalDoorId(None)),
                None,
            )
        }
        LegacyGenericFieldValue::RawUnion12(bytes) => {
            (FieldValue::Bool(bytes[0] != 0), Some(*bytes))
        }
        LegacyGenericFieldValue::Geo3(values) => {
            let raw = geo3_bytes(*values);
            let value = match saved.kind {
                K::CameraZoomLevel => FieldValue::Float(values[0]),
                K::CameraPoint => FieldValue::GeoPoint2D {
                    x: values[0],
                    y: values[1],
                },
                K::BowTargetPoint
                | K::ShieldDangerPoint
                | K::RollPoint
                | K::PurseTarget
                | K::NetTarget
                | K::WaspNestTarget => FieldValue::Point3D {
                    x: values[0],
                    y: values[1],
                    z: values[2],
                },
                K::AnimationId | K::OldAnimation | K::NewAnimation => {
                    let raw_animation = values[0].to_bits();
                    FieldValue::Animation(OrderType::try_from(raw_animation).map_err(|_| {
                        invalid(
                            "generic.animation",
                            raw_animation,
                            "a known RHanimation discriminant",
                        )
                    })?)
                }
                _ => FieldValue::Integer(values[0].to_bits()),
            };
            (value, Some(raw))
        }
    };
    Ok((field, value, raw))
}

fn geo3_bytes(values: [f32; 3]) -> [u8; 12] {
    let mut bytes = [0; 12];
    for (index, value) in values.into_iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_bits().to_le_bytes());
    }
    bytes
}

fn resolve_entity(
    field: &'static str,
    reference: super::payload_base::LegacyElementRef,
    entities: &LegacyEntityFixups,
) -> Result<Option<EntityId>, LegacySequenceAdoptError> {
    let Some(id) = reference.0 else {
        return Ok(None);
    };
    entities
        .by_creation_order
        .get(&id)
        .copied()
        .map(Some)
        .ok_or(LegacySequenceAdoptError::MissingIdentity { field, id })
}

fn resolve_element_ref(
    field: &'static str,
    id: Option<u32>,
    refs: &BTreeMap<u32, SequenceElementRef>,
) -> Result<Option<SequenceElementRef>, LegacySequenceAdoptError> {
    id.map(|id| {
        refs.get(&id)
            .copied()
            .ok_or(LegacySequenceAdoptError::MissingIdentity { field, id })
    })
    .transpose()
}

fn resolve_sequence_ref(
    field: &'static str,
    id: Option<u32>,
    refs: &BTreeMap<u32, SequenceId>,
) -> Result<Option<SequenceId>, LegacySequenceAdoptError> {
    id.map(|id| {
        refs.get(&id)
            .copied()
            .ok_or(LegacySequenceAdoptError::MissingIdentity { field, id })
    })
    .transpose()
}

fn resolve_gate(
    field: &'static str,
    reference: LegacyGateRef,
    topology: &LegacySequenceTopology,
) -> Result<Option<DoorIndex>, LegacySequenceAdoptError> {
    let Some(raw) = reference.0 else {
        return Ok(None);
    };
    let index =
        usize::try_from(raw).map_err(|_| invalid(field, raw, "a non-negative gate-array index"))?;
    topology.gates.get(index).copied().map(Some).ok_or_else(|| {
        LegacySequenceAdoptError::MissingTopology {
            field,
            identity: format!("gate at Original gate index {index}"),
        }
    })
}

fn resolve_line(
    field: &'static str,
    reference: LegacyLineRef,
    topology: &LegacySequenceTopology,
) -> Result<Option<JumpLineIndex>, LegacySequenceAdoptError> {
    match (reference.layer, reference.index) {
        (None, None) => Ok(None),
        (Some(layer), Some(index)) if index >= 0 => topology
            .lines
            .get(&(layer, index))
            // Retail data editions differ in some non-jump boundary counts.
            // A lone jump line on a layer still has a unique isomorphic
            // identity regardless of its raw combined-array ordinal.
            .or_else(|| topology.unique_line_by_layer.get(&layer))
            .copied()
            .map(Some)
            .ok_or_else(|| LegacySequenceAdoptError::MissingTopology {
                field,
                identity: format!("line layer {layer}, index {index}"),
            }),
        _ => Err(invalid(
            field,
            format!("{:?}", (reference.layer, reference.index)),
            "both null sentinels or a layer plus non-negative line index",
        )),
    }
}

fn resolve_sector(
    reference: LegacySectorRef,
    topology: &LegacySequenceTopology,
) -> Result<Option<SectorHandle>, LegacySequenceAdoptError> {
    let Some(index) = reference.0 else {
        return Ok(None);
    };
    let Some(sector) = topology.sectors.get(usize::from(index)) else {
        return Err(LegacySequenceAdoptError::MissingTopology {
            field: "movement.sector",
            identity: format!("sector index {index}"),
        });
    };
    (*sector)
        .map(Some)
        .ok_or_else(|| LegacySequenceAdoptError::MissingTopology {
            field: "movement.sector",
            identity: format!(
                "Original sector slot {index}, which has no Rust position-sector counterpart"
            ),
        })
}

fn sequence_state(raw: i32) -> Result<SequenceState, LegacySequenceAdoptError> {
    Ok(match raw {
        0 => SequenceState::Terminated,
        1 => SequenceState::Done,
        2 => SequenceState::InProgress,
        3 => SequenceState::Todo,
        4 => SequenceState::Postponed,
        5 => SequenceState::Impossible,
        6 => SequenceState::Interrupted,
        _ => return Err(invalid("state", raw, "RHsequenceState 0..6")),
    })
}

fn sequence_priority(raw: i32) -> Result<SequencePriority, LegacySequenceAdoptError> {
    Ok(match raw {
        0 => SequencePriority::NonInterruptable,
        1 => SequencePriority::PostponeEverythingButInjuries,
        2 => SequencePriority::Lethal,
        3 => SequencePriority::Ko,
        4 => SequencePriority::Ko2,
        5 => SequencePriority::Injury,
        6 => SequencePriority::Script,
        7 => SequencePriority::Preference,
        8 => SequencePriority::Normal,
        9 => SequencePriority::Wait,
        10 => SequencePriority::None,
        11 => SequencePriority::NotYetSet,
        _ => return Err(invalid("priority", raw, "RHpriority 0..11")),
    })
}

fn invalid(
    field: &'static str,
    value: impl std::fmt::Display,
    expected: &'static str,
) -> LegacySequenceAdoptError {
    LegacySequenceAdoptError::InvalidField {
        field,
        value: value.to_string(),
        expected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        element::EntityIdKind,
        engine::LegacyGridGateAsset,
        gate::GateType,
        legacy_save::{
            payload_base::{
                LegacyElementRef, LegacyPoint2, LegacyPoint3, LegacySequenceElementRef,
                LegacySequenceRef,
            },
            payload_sequences::{
                LegacyInlineOrderId, LegacyInlineSequenceElementId, LegacyInlineSequenceId,
                LegacySequenceElementBase, LegacySequenceElementFixups,
                LegacySequenceElementGeneric, LegacySequenceElementMovement,
            },
            post_sequence_manager::{
                LegacyDeferredSequenceElement, LegacyManagedSequence, LegacySequenceStaticIds,
            },
        },
    };

    fn entities() -> (LegacyEntityFixups, EntityId, EntityId) {
        let owner = EntityId::new(4, EntityIdKind::Pc);
        let target = EntityId::new(9, EntityIdKind::Soldier);
        let fixups = LegacyEntityFixups {
            by_creation_order: [(40, owner), (90, target)].into(),
            by_saved_slot: vec![Some(owner), Some(target)],
            creation_order_by_entity: [(owner, 40), (target, 90)].into(),
            mobile_by_creation_order: BTreeMap::new(),
            mobile_owner_by_creation_order: BTreeMap::new(),
        };
        (fixups, owner, target)
    }

    fn fixups(
        next: Option<u32>,
        postponed: Option<u32>,
        mummy: u32,
    ) -> LegacySequenceElementFixups {
        LegacySequenceElementFixups {
            next_offset: 0,
            next: LegacySequenceElementRef(next),
            postponed_offset: 0,
            postponed: LegacySequenceElementRef(postponed),
            mummy_offset: 0,
            mummy: LegacySequenceRef(Some(mummy)),
        }
    }

    fn order(id: u32) -> LegacyInlineOrder {
        LegacyInlineOrder {
            action: OrderType::WaitingUpright as i32,
            apply_transition_at_this_point: true,
            compute_direction: false,
            can_fly: true,
            lock_ai: true,
            reverse: true,
            transition: true,
            tolerance: 3.5,
            unique_id: LegacyInlineOrderId(id),
            destination_2d: LegacyPoint2 { x: 5.0, y: 6.0 },
            destination_3d: LegacyPoint3 {
                x: 7.0,
                y: 8.0,
                z: 9.0,
            },
            flight_vector: LegacyPoint2 { x: 1.5, y: 2.5 },
            antagonist: LegacyElementRef(Some(90)),
        }
    }

    fn base(
        id: u32,
        level: u16,
        state: i32,
        fixups: LegacySequenceElementFixups,
    ) -> LegacySequenceElementBase {
        LegacySequenceElementBase {
            command: Command::Move as i32,
            state,
            command_level: level,
            priority: SequencePriority::Normal as i32,
            unique_id: LegacyInlineSequenceElementId(id),
            posture_after_transition: Posture::Upright as i32,
            action_state_after_transition: ActionState::Waiting as i32,
            deleted: false,
            script_driven: true,
            owner: LegacyElementRef(Some(40)),
            orders: vec![order(0)],
            manager_fixups: Some(fixups),
        }
    }

    fn sequence(
        id: u32,
        elements: Vec<LegacyInlineSequenceElement>,
        cursor: u16,
        in_progress: u16,
    ) -> LegacyManagedSequence {
        LegacyManagedSequence {
            start_offset: 0,
            body: LegacyInlineSequence {
                started: true,
                current_command_level: 1,
                running_elements: 1,
                sequence_element_cursor: cursor,
                unique_id: LegacyInlineSequenceId(id),
                elements,
                elements_in_progress: in_progress,
            },
            end_offset: 0,
        }
    }

    fn fixture() -> LegacySequenceManagerState {
        let movement = LegacyInlineSequenceElement::Movement(LegacySequenceElementMovement {
            base: base(100, 1, 3, fixups(Some(101), Some(200), 10)),
            action: OrderType::WalkingUpright as i32,
            tolerance: 12.0,
            direction: 7,
            flags: (MoveFlags::SEEK | MoveFlags::USE_POINT).bits(),
            layer: 2,
            speed_factor: 1.25,
            destination: LegacyPoint2 { x: 30.0, y: 40.0 },
            element: LegacyElementRef(Some(90)),
            gate: LegacyGateRef(Some(0)),
            line: LegacyLineRef {
                layer: Some(2),
                index: Some(3),
            },
            sector: LegacySectorRef(Some(4)),
            post_seek_sequence: None,
            manager_linked_seek_fixup_offset: Some(0),
            manager_linked_seek_fixup: Some(LegacySequenceElementRef(Some(101))),
        });
        let generic = LegacyInlineSequenceElement::Generic(LegacySequenceElementGeneric {
            base: base(101, 2, 2, fixups(None, None, 10)),
            fields: vec![
                LegacyGenericField {
                    kind: LegacyGenericFieldKind::Opponent,
                    value: LegacyGenericFieldValue::Element(LegacyElementRef(Some(90))),
                },
                LegacyGenericField {
                    kind: LegacyGenericFieldKind::Freeze,
                    value: LegacyGenericFieldValue::RawUnion12([
                        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
                    ]),
                },
            ],
        });
        let other = LegacyInlineSequenceElement::Simple(base(200, 1, 3, fixups(None, None, 20)));
        LegacySequenceManagerState {
            start_offset: 0,
            static_ids: LegacySequenceStaticIds {
                order_next_id_offset: 0,
                order_next_id: 1,
                sequence_next_id_offset: 0,
                sequence_next_id: 21,
                sequence_element_next_id_offset: 0,
                sequence_element_next_id: 201,
            },
            sequences: vec![
                sequence(10, vec![movement, generic], 1, 1),
                sequence(20, vec![other], 1, 0),
            ],
            deferred_elements: vec![
                LegacyDeferredSequenceElement {
                    offset: 0,
                    element: LegacySequenceElementRef(Some(100)),
                },
                LegacyDeferredSequenceElement {
                    offset: 0,
                    element: LegacySequenceElementRef(Some(200)),
                },
            ],
            end_offset: 0,
        }
    }

    fn topology() -> LegacySequenceTopology {
        LegacySequenceTopology {
            gates: vec![DoorIndex::new(7).expect("valid door index")],
            lines: [((2, 3), JumpLineIndex::new(5).unwrap())].into(),
            unique_line_by_layer: [(2, JumpLineIndex::new(5).unwrap())].into(),
            sectors: (0..6).map(SectorHandle::new).collect(),
            ..LegacySequenceTopology::default()
        }
    }

    #[test]
    fn gate_mapping_preserves_per_kind_identity_across_mixed_order() {
        let runtime = [
            crate::gate::Door {
                gate_type: GateType::Door,
                ..Default::default()
            },
            crate::gate::Door {
                gate_type: GateType::Door,
                ..Default::default()
            },
            crate::gate::Door {
                gate_type: GateType::Door,
                ..Default::default()
            },
            crate::gate::Door {
                gate_type: GateType::Jump,
                ..Default::default()
            },
            crate::gate::Door {
                gate_type: GateType::Jump,
                ..Default::default()
            },
        ];
        let retained = [
            LegacyGridGateAsset::Door,
            LegacyGridGateAsset::Stateless,
            LegacyGridGateAsset::Door,
            LegacyGridGateAsset::Stateless,
            LegacyGridGateAsset::Door,
        ];
        let topology = LegacySequenceTopology {
            gates: derive_legacy_gate_order(&retained, &runtime).unwrap(),
            ..Default::default()
        };
        assert_eq!(
            resolve_gate("movement.gate", LegacyGateRef(Some(1)), &topology).unwrap(),
            Some(DoorIndex::new(3).expect("valid door index"))
        );
        assert_eq!(
            resolve_gate("movement.gate", LegacyGateRef(Some(4)), &topology).unwrap(),
            Some(DoorIndex::new(2).expect("valid door index"))
        );
    }

    #[test]
    fn restored_turn_keeps_direction_translated_before_the_save() {
        let mut orders = [
            Order::new(OrderType::Turning, 0.0, 0.0, NonZeroU32::new(1).unwrap()),
            Order::new(
                OrderType::WaitingUpright,
                0.0,
                0.0,
                NonZeroU32::new(2).unwrap(),
            ),
        ];
        assert!(orders.iter().all(|order| order.compute_direction));

        preserve_translated_turn_direction(Command::Turn, &mut orders);

        assert!(
            !orders[0].compute_direction,
            "the saved direction goal is authoritative for command-owned Turning orders"
        );
        assert!(
            orders[1].compute_direction,
            "unrelated restored orders preserve their serialized flag"
        );
    }

    #[test]
    fn preflights_and_atomically_restores_manager_identity_and_fifo_order() {
        let (entities, owner, target) = entities();
        let plan = preflight_v48_sequence_manager(&fixture(), &entities, &topology()).unwrap();
        let (resolved_ref, resolved) = plan
            .resolve_element("tail.timer", LegacySequenceElementRef(Some(101)))
            .unwrap()
            .unwrap();
        assert_eq!(resolved_ref, SequenceElementRef::new(SequenceId(10), 1));
        assert_eq!(resolved.id, 101);
        let (order_element, order_index, order) = plan
            .resolve_order("actor.order", LegacyOrderRef(Some(0)))
            .unwrap()
            .unwrap();
        assert_eq!(order_element, SequenceElementRef::new(SequenceId(10), 0));
        assert_eq!(order_index, 0);
        assert_eq!(order.order_id.get(), 1);
        assert_eq!(
            plan.current_element_for_actor(owner),
            Some(SequenceElementRef::new(SequenceId(10), 1))
        );
        assert!(
            plan.resolve_order("actor.order", LegacyOrderRef(Some(u32::MAX)))
                .is_err()
        );
        let mut engine = crate::engine::EngineInner::new();
        plan.apply(&mut engine);

        let manager = &engine.orders.sequence_manager;
        assert_eq!(manager.sequence_count(), 2);
        assert_eq!(
            manager.v48_elements_to_go(),
            vec![(SequenceId(10), 0), (SequenceId(20), 0)]
        );
        let movement = manager.get_element(SequenceId(10), 0).unwrap();
        assert_eq!(movement.owner, Some(owner));
        assert_eq!(movement.orders[0].order_id.get(), 1);
        assert_eq!(movement.orders[0].antagonist, Some(target));
        let movement_sector = match &movement.data {
            SequenceElementData::Movement { sector, .. } => *sector,
            other => panic!("fixture element is not movement data: {other:?}"),
        };
        assert_eq!(
            movement_sector,
            SectorHandle::new(4),
            "saved movement sectors use the sparse Original slot mapping"
        );
        let retained = movement.legacy_v48.as_ref().unwrap();
        assert_eq!(
            retained.next,
            Some(SequenceElementRef::new(SequenceId(10), 1))
        );
        assert_eq!(
            retained.postponed,
            Some(SequenceElementRef::new(SequenceId(20), 0))
        );
        assert_eq!(
            retained.linked_seek,
            Some(Some(SequenceElementRef::new(SequenceId(10), 1)))
        );
        assert!(retained.script_driven);
        assert_eq!(movement.orders[0].destination_3d, [7.0, 8.0, 9.0]);
        assert_eq!(
            manager
                .get_element(SequenceId(10), 1)
                .unwrap()
                .legacy_v48
                .as_ref()
                .unwrap()
                .generic_raw_unions,
            vec![(Field::Freeze, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12])]
        );

        assert_eq!(engine.orders.allocate_order_id().get(), 2);
        let mut fresh = Sequence::new();
        fresh.append_element(SequenceElement::new(1, Command::Wait, Some(owner)));
        let fresh_id = engine.orders.sequence_manager.launch_sequence(fresh);
        assert_eq!(fresh_id, SequenceId(21));
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(fresh_id, 0)
                .unwrap()
                .id,
            201
        );
    }

    #[test]
    fn movement_sector_maps_sparse_original_slot_to_compact_runtime_handle() {
        let topology = LegacySequenceTopology {
            sectors: vec![SectorHandle::new(42), None, SectorHandle::new(7)],
            ..Default::default()
        };

        assert_eq!(
            resolve_sector(LegacySectorRef(Some(2)), &topology).unwrap(),
            SectorHandle::new(7)
        );
        assert!(matches!(
            resolve_sector(LegacySectorRef(Some(1)), &topology),
            Err(LegacySequenceAdoptError::MissingTopology {
                field: "movement.sector",
                ..
            })
        ));
    }

    #[test]
    fn retains_invalid_transition_results_while_not_in_progress() {
        let (entities, _, _) = entities();
        let mut saved = fixture();
        let movement = match &mut saved.sequences[0].body.elements[0] {
            LegacyInlineSequenceElement::Movement(movement) => movement,
            other => panic!("fixture element is not movement: {other:?}"),
        };
        assert_eq!(movement.base.state, SequenceState::Todo as i32);
        movement.base.posture_after_transition = 252_736;
        movement.base.action_state_after_transition = -559_038_737;

        let plan = preflight_v48_sequence_manager(&saved, &entities, &topology())
            .expect("TODO is dormant until Instruct stamps it");
        let (_, element) = plan
            .resolve_element("test", LegacySequenceElementRef(Some(100)))
            .unwrap()
            .unwrap();

        assert_eq!(element.posture_after_transition, Posture::Undefined);
        assert_eq!(element.action_state_after_transition, ActionState::Waiting);
        let retained = element.legacy_v48.as_ref().unwrap();
        assert_eq!(retained.raw_dormant_posture_after_transition, Some(252_736));
        assert_eq!(
            retained.raw_dormant_action_state_after_transition,
            Some(-559_038_737)
        );
    }

    #[test]
    fn resets_valid_transition_results_while_waiting_for_instruct() {
        let (entities, _, _) = entities();
        let saved = fixture();

        let plan = preflight_v48_sequence_manager(&saved, &entities, &topology())
            .expect("TODO transition results are dormant until Instruct");
        let (_, element) = plan
            .resolve_element("test", LegacySequenceElementRef(Some(100)))
            .unwrap()
            .unwrap();

        assert_eq!(element.state, SequenceState::Todo);
        assert_eq!(element.posture_after_transition, Posture::Undefined);
        assert_eq!(element.action_state_after_transition, ActionState::Waiting);
        let retained = element.legacy_v48.as_ref().unwrap();
        assert_eq!(
            retained.raw_dormant_posture_after_transition,
            Some(Posture::Upright as i32)
        );
        assert_eq!(
            retained.raw_dormant_action_state_after_transition,
            Some(ActionState::Waiting as i32)
        );
    }

    #[test]
    fn retains_uninitialized_action_for_position_only_movement() {
        let (entities, _, _) = entities();
        let mut saved = fixture();
        let movement = match &mut saved.sequences[0].body.elements[0] {
            LegacyInlineSequenceElement::Movement(movement) => movement,
            other => panic!("fixture element is not movement: {other:?}"),
        };
        movement.base.command = Command::AssertPosition as i32;
        movement.action = -556_225_327;

        let plan = preflight_v48_sequence_manager(&saved, &entities, &topology())
            .expect("ASSERT_POSITION never reads its uninitialized maction storage");
        let (_, element) = plan
            .resolve_element("test", LegacySequenceElementRef(Some(100)))
            .unwrap()
            .unwrap();

        let SequenceElementData::Movement { action, .. } = &element.data else {
            panic!("fixture element is not restored as movement");
        };
        assert_eq!(*action, OrderType::Invalid);
        assert_eq!(
            element
                .legacy_v48
                .as_ref()
                .unwrap()
                .raw_dormant_movement_action,
            Some(-556_225_327)
        );
    }

    #[test]
    fn movement_action_validation_remains_strict_until_terminal() {
        let raw = 2_013_265_920;

        assert_eq!(
            convert_movement_action(Command::Move, SequenceState::Terminated, raw).unwrap(),
            (OrderType::Invalid, Some(raw)),
            "a terminal element is never dispatched again"
        );
        assert_eq!(
            convert_movement_action(Command::Teleport, SequenceState::Todo, raw).unwrap(),
            (OrderType::Invalid, Some(raw)),
            "TELEPORT execution never reads movement.action"
        );
        assert_eq!(
            convert_movement_action(Command::Move, SequenceState::Todo, raw).unwrap_err(),
            LegacySequenceAdoptError::InvalidField {
                field: "movement.action",
                value: raw.to_string(),
                expected: "a known RHanimation",
            },
            "a queued MOVE consumes movement.action when it is instructed"
        );
    }

    #[test]
    fn rejects_invalid_transition_result_after_instruction_is_live() {
        let (entities, _, _) = entities();
        let mut saved = fixture();
        let generic = match &mut saved.sequences[0].body.elements[1] {
            LegacyInlineSequenceElement::Generic(generic) => generic,
            other => panic!("fixture element is not generic: {other:?}"),
        };
        assert_eq!(generic.base.state, SequenceState::InProgress as i32);
        generic.base.posture_after_transition = 252_736;

        let error = preflight_v48_sequence_manager(&saved, &entities, &topology()).unwrap_err();

        assert_eq!(
            error,
            LegacySequenceAdoptError::InvalidField {
                field: "posture_after_transition",
                value: "252736".to_owned(),
                expected: "RHposture 0..24",
            }
        );
    }

    #[test]
    fn restores_nonmonotonic_ids_in_original_manager_order() {
        let (entities, _owner, _) = entities();
        let mut saved = fixture();
        saved.sequences.swap(0, 1);
        let mut engine = crate::engine::EngineInner::new();
        let plan = preflight_v48_sequence_manager(&saved, &entities, &topology()).unwrap();
        plan.apply(&mut engine);
        let restored_ids: Vec<_> = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .map(|sequence| sequence.id.0)
            .collect();
        assert_eq!(
            restored_ids,
            saved
                .sequences
                .iter()
                .map(|sequence| sequence.body.unique_id.0)
                .collect::<Vec<_>>()
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .get_sequence(SequenceId(20))
                .is_some()
        );
    }

    #[test]
    fn rejects_missing_initialized_line_identity_strictly() {
        let (entities, _, _) = entities();
        let error = preflight_v48_sequence_manager(
            &fixture(),
            &entities,
            &LegacySequenceTopology {
                // Keep the earlier sector lookup valid so this fixture
                // specifically exercises the missing gate identity.
                sectors: (0..6).map(SectorHandle::new).collect(),
                ..LegacySequenceTopology::default()
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            LegacySequenceAdoptError::MissingTopology {
                field: "movement.gate",
                ..
            }
        ));
    }

    fn jump_pair(source: u32, destination: u32, source_x: f32) -> LegacyJumpPair {
        LegacyJumpPair {
            source: JumpLineIndex::new(source).unwrap(),
            destination: JumpLineIndex::new(destination).unwrap(),
            source_layer: 2,
            destination_layer: 8,
            source_a: crate::coordinates::MapPoint::new(source_x, 0.0),
            source_b: crate::coordinates::MapPoint::new(source_x, 10.0),
        }
    }

    #[test]
    fn shifted_jump_ordinals_resolve_by_unique_owner_geometry() {
        let topology = LegacySequenceTopology {
            jump_pairs: vec![jump_pair(10, 11, 0.0), jump_pair(20, 21, 100.0)],
            saved_actor_locations: [(
                40,
                LegacyActorLocation {
                    map: crate::coordinates::MapPoint::new(3.0, 5.0),
                },
            )]
            .into(),
            ..LegacySequenceTopology::default()
        };

        assert_eq!(
            resolve_jump_pair(
                LegacyLineRef {
                    layer: Some(2),
                    index: Some(196),
                },
                LegacyLineRef {
                    layer: Some(8),
                    index: Some(241),
                },
                LegacyElementRef(Some(40)),
                &topology,
            )
            .unwrap(),
            (
                JumpLineIndex::new(10).unwrap(),
                JumpLineIndex::new(11).unwrap()
            )
        );
    }

    #[test]
    fn exact_jump_identity_remains_authoritative_over_geometry() {
        let expected_source = JumpLineIndex::new(10).unwrap();
        let expected_destination = JumpLineIndex::new(11).unwrap();
        let topology = LegacySequenceTopology {
            lines: [
                ((2, 196), expected_source),
                ((8, 241), expected_destination),
            ]
            .into(),
            jump_pairs: vec![jump_pair(10, 11, 0.0), jump_pair(20, 21, 100.0)],
            saved_actor_locations: [(
                40,
                LegacyActorLocation {
                    map: crate::coordinates::MapPoint::new(100.0, 5.0),
                },
            )]
            .into(),
            ..LegacySequenceTopology::default()
        };

        assert_eq!(
            resolve_jump_pair(
                LegacyLineRef {
                    layer: Some(2),
                    index: Some(196),
                },
                LegacyLineRef {
                    layer: Some(8),
                    index: Some(241),
                },
                LegacyElementRef(Some(40)),
                &topology,
            )
            .unwrap(),
            (expected_source, expected_destination)
        );
    }

    #[test]
    fn ambiguous_shifted_jump_geometry_is_rejected() {
        let topology = LegacySequenceTopology {
            jump_pairs: vec![jump_pair(10, 11, -10.0), jump_pair(20, 21, 10.0)],
            saved_actor_locations: [(
                40,
                LegacyActorLocation {
                    map: crate::coordinates::MapPoint::new(0.0, 5.0),
                },
            )]
            .into(),
            ..LegacySequenceTopology::default()
        };

        let error = resolve_jump_pair(
            LegacyLineRef {
                layer: Some(2),
                index: Some(196),
            },
            LegacyLineRef {
                layer: Some(8),
                index: Some(241),
            },
            LegacyElementRef(Some(40)),
            &topology,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            LegacySequenceAdoptError::MissingTopology {
                field: "generic.jump_lines",
                identity,
            } if identity.contains("ambiguous")
        ));
    }
}
