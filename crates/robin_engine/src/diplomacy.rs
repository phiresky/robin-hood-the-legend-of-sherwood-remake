//! Deterministic mission diplomacy and entity reconciliation.

pub use robin_engine_types::diplomacy::*;
use std::collections::BTreeMap;

/// Rebuild relationship-derived entity caches after an authoritative matrix
/// edit. Kept outside `EngineInner` so both frame commands and synchronous Lua
/// natives execute the identical transition.
pub(crate) fn reconcile_entities(
    entities: &mut crate::entities::Entities,
    diplomacy: &DiplomacyState,
) {
    use crate::element::{Detectable, DetectableType, Entity, EntityId};

    let humans = entities
        .actors()
        .filter_map(|(id, entity)| {
            entity.is_human().then_some((
                EntityId::from(id),
                entity.camp(),
                entity.is_pc(),
                entity.is_soldier(),
            ))
        })
        .collect::<Vec<_>>();
    let actors_by_id = humans
        .iter()
        .map(|(id, camp, is_pc, is_soldier)| (*id, (*camp, *is_pc, *is_soldier)))
        .collect::<BTreeMap<_, _>>();
    let actors_by_handle = humans
        .iter()
        .map(|(id, camp, is_pc, _)| (id.index(), (*camp, *is_pc)))
        .collect::<BTreeMap<_, _>>();

    for (id, own_camp, own_is_pc, _) in &humans {
        let entity = entities
            .get_mut(*id)
            .unwrap_or_else(|| panic!("diplomacy reconciliation actor {id:?} disappeared"));
        if let Some(human) = entity.human_data_mut() {
            let retained = human
                .opponents
                .iter_with_jump_lines()
                .filter(|(opponent, _)| {
                    actors_by_id.get(opponent).is_some_and(|(camp, is_pc, _)| {
                        diplomacy.actors_may_fight(*own_camp, *own_is_pc, *camp, *is_pc)
                    })
                })
                .collect::<Vec<_>>();
            human.opponents = crate::element::SwordfightOpponents::from_pairs(retained);
        }
    }

    let npc_ids = entities.npc_ids().collect::<Vec<_>>();
    for npc_id in npc_ids {
        let (npc_camp, npc_is_soldier) = {
            let npc = entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("diplomacy NPC {npc_id:?} disappeared"));
            (npc.camp(), npc.is_soldier())
        };
        let npc = entities
            .get_mut(npc_id)
            .and_then(Entity::ai_actor_data_mut)
            .unwrap_or_else(|| panic!("diplomacy NPC {npc_id:?} has no AI actor data"));
        let enemies = &mut npc.detectable_lists[DetectableType::Enemy as usize];
        enemies.retain(|detectable| {
            detectable.element.is_some_and(|id| {
                actors_by_id.get(&id).is_some_and(
                    |(target_camp, target_is_pc, target_is_soldier)| {
                        crate::ai_detectable_filter::should_add_enemy_detectable_with(
                            diplomacy,
                            npc_camp,
                            npc_is_soldier,
                            *target_is_pc,
                            *target_is_soldier,
                            *target_camp,
                        )
                    },
                )
            })
        });
        for (target_id, target_camp, target_is_pc, target_is_soldier) in &humans {
            if *target_id == npc_id
                || !crate::ai_detectable_filter::should_add_enemy_detectable_with(
                    diplomacy,
                    npc_camp,
                    npc_is_soldier,
                    *target_is_pc,
                    *target_is_soldier,
                    *target_camp,
                )
                || enemies
                    .iter()
                    .any(|detectable| detectable.element == Some(*target_id))
            {
                continue;
            }
            enemies.push(Detectable {
                element: Some(*target_id),
                detectable_type: DetectableType::Enemy,
                seen_last_frame: false,
                heard_last_frame: false,
                seen_now: false,
                shadow_seen_now: false,
                shadow_seen_last_frame: false,
                last_visibility: 0.0,
            });
        }
        for detectable_type in [DetectableType::Friend, DetectableType::MissedFriend] {
            npc.detectable_lists[detectable_type as usize].retain(|detectable| {
                detectable.element.is_some_and(|id| {
                    actors_by_id
                        .get(&id)
                        .is_some_and(|(camp, _, _)| diplomacy.is_allied(npc_camp, *camp))
                })
            });
        }
        if let Some(enemy) = npc.ai_brain.enemy_mut() {
            enemy.base.list_us.retain(|handle| {
                actors_by_handle
                    .get(handle)
                    .is_some_and(|(camp, _)| diplomacy.is_allied(npc_camp, *camp))
            });
            enemy.list_them.retain(|handle| {
                actors_by_handle.get(handle).is_some_and(|(camp, is_pc)| {
                    diplomacy.actors_may_fight(npc_camp, false, *camp, *is_pc)
                })
            });
            if enemy.base.primary_target.is_some_and(|target| {
                !actors_by_handle
                    .get(&target.get())
                    .is_some_and(|(camp, is_pc)| {
                        diplomacy.actors_may_fight(npc_camp, false, *camp, *is_pc)
                    })
            }) {
                enemy.base.primary_target = None;
                enemy.base.outbox.actor.set_unfocus();
            }
        }
    }
}
