//! Beggar-money-drop flow.
//!
//! A PC can adopt the [`Posture::SimulatingBeggar`] disguise. While in
//! that posture, the PC's idle tick iterates every NPC looking for a
//! civilian who can donate; the civilian tosses one coin on the ground
//! flagged `belongs_to_beggar=true` (so soldier AIs ignore it as
//! clutter) that the PC can then pick up. Each civilian may only give
//! once; a PC that already "got the beggar trick" from a given
//! civilian is excluded from that civilian's future donations.
//!
//! The transition into/out of `SimulatingBeggar` also toggles the
//! `belongs_to_beggar` flag on every coin already on the ground within
//! 100 map units of the PC — so residual coins from other sources
//! (thrown purses, etc.) become pickable while the disguise is active
//! and revert to their previous state when the PC stands up.

use super::EngineInner;
use crate::bow_shot;
use crate::coordinates::MapPoint;
use crate::element::{Entity, EntityId, ObjectType};
use crate::inventory::COIN_VALUE;
use crate::position_interface::vector_to_sector_0_to_15_iso;

/// Minimum money a civilian must carry to consider donating to a beggar.
const MIN_MONEY_FOR_BEGGAR_GIFT: u32 = 200;

/// MaxNorm proximity window (map units) within which a civilian will
/// notice a beggar in front of them.
const BEGGAR_PROXIMITY: f32 = 70.0;

/// Radius (MaxNorm, map units) for the near-coins toggle sweep.
const NEAR_COINS_RADIUS: f32 = 100.0;

/// Original `CHECK_BEGGAR_MIN_IQ`.
const CHECK_BEGGAR_MIN_IQ: u16 = 30;

fn beggar_coin_source(
    belt: crate::coordinates::WorldPoint3D,
    direction: i16,
) -> crate::coordinates::WorldPoint3D {
    let (dx, dy) = crate::element::direction_vector_16(direction);
    crate::coordinates::WorldPoint3D {
        x: belt.x + dx * 5.0,
        y: belt.y + dy * crate::position_interface::ASPECT_RATIO * 5.0,
        z: belt.z,
    }
}

fn beggar_coin_target(
    source: crate::coordinates::WorldPoint3D,
    beggar_position: crate::coordinates::WorldPoint3D,
    line_of_sight_clear: bool,
) -> crate::coordinates::WorldPoint3D {
    if line_of_sight_clear {
        crate::coordinates::WorldPoint3D {
            x: source.x + 0.5 * (beggar_position.x - source.x),
            y: source.y + 0.5 * (beggar_position.y - source.y),
            z: source.z + 0.5 * (beggar_position.z - source.z),
        }
    } else {
        source
    }
}

/// Toggle nearby ground coins for the stealth-command transition without
/// borrowing the rest of [`EngineInner`].
pub(super) fn set_flags_of_near_coins_on_ground(
    entities: &mut crate::entities::Entities,
    pc_id: EntityId,
    value: bool,
) {
    let Some(pc) = entities.get(pc_id) else {
        return;
    };
    let pc_pos = pc.element_data().position_map();

    for (_, entity) in entities.objects_mut() {
        let pos = entity.element_data().position_map();
        let dist = (pc_pos.x - pos.x).abs().max((pc_pos.y - pos.y).abs());
        if dist >= NEAR_COINS_RADIUS {
            continue;
        }
        if let Some(object) = entity.object_data_mut()
            && object.object_type == ObjectType::Coin
        {
            object.belongs_to_beggar = value;
        }
    }
}

/// Register a newly disguised PC with every intelligent Lacklandist soldier
/// currently searching an area.
///
/// This is the exact scope of Original
/// `RHElementActorNPC::AddBeggarForAllIntelligentSeekingSoldiers`: the camp
/// registry is scanned in order, difficulty-modified `GetIQ()` is compared
/// against `CHECK_BEGGAR_MIN_IQ`, and `_ANY_SEEK_AREA_SUBSTATE_` is the only
/// admitted state family. `AddDetectable` appends to the Beggar list and
/// requires the pair not to be present already.
pub(super) fn add_beggar_for_all_intelligent_seeking_soldiers(
    entities: &mut crate::entities::Entities,
    diplomacy: &crate::diplomacy::DiplomacyState,
    beggar_id: EntityId,
    difficulty: crate::player_profile::DifficultyLevel,
) {
    use crate::element::{Detectable, DetectableType};
    let eligible: Vec<_> = entities
        .soldiers()
        .filter_map(|(soldier_id, soldier)| {
            let ai = soldier.npc.ai_brain.enemy()?;
            let iq = difficulty.rules().enemy_iq(ai.soldier_profile_iq, 100);
            (diplomacy.is_hostile_to_player(soldier.soldier.cached_camp)
                && iq >= CHECK_BEGGAR_MIN_IQ
                && ai.base.current_substate.is_seek_area())
            .then_some(soldier_id)
        })
        .collect();
    let beggar_idx = DetectableType::Beggar as usize;
    for soldier_id in eligible {
        let soldier = entities
            .get_mut(soldier_id)
            .and_then(Entity::npc_data_mut)
            .expect("eligible beggar observer disappeared from the soldier registry");
        let list = soldier
            .detectable_lists
            .get_mut(beggar_idx)
            .expect("NPC detectable lists must include the Beggar bucket");
        if !list.iter().any(|detectable| {
            detectable.element == Some(beggar_id)
                && detectable.detectable_type == DetectableType::Beggar
        }) {
            list.push(Detectable {
                element: Some(beggar_id),
                detectable_type: DetectableType::Beggar,
                ..Detectable::default()
            });
        }
    }
}

/// Civilian predicate: can this NPC toss a coin to `beggar_pc` right now?
///
/// Reads: NPC's `has_given_money_to_beggar`, `got_the_beggar_trick`,
/// `money`, `direction`, `position_map`, `sector`, `ai_state`, and the
/// human kind. Returns `false` (no donation) in every failure arm.
pub(super) fn can_give_money_to_beggar(
    engine: &EngineInner,
    npc_id: EntityId,
    beggar_id: EntityId,
) -> bool {
    let Some(npc) = engine.get_entity(npc_id) else {
        return false;
    };
    let Some(beggar) = engine.get_entity(beggar_id) else {
        return false;
    };

    let Some(npc_data) = npc.npc_data() else {
        return false;
    };

    // Single-shot flag — each civilian gives at most one coin for the
    // lifetime of the level.
    if npc_data.has_given_money_to_beggar {
        return false;
    }

    // Only civilians in their default behaviour state look around to
    // donate. Alerted, fleeing, etc. NPCs ignore beggars.
    if npc_data.ai_state() != crate::ai::AiState::Default {
        return false;
    }

    // Civilians only — soldiers and camp-neutral hostiles never donate.
    if !npc.element_data().kind.is_civilian() {
        return false;
    }

    // Rich-civilian threshold.
    if npc_data.money < MIN_MONEY_FOR_BEGGAR_GIFT {
        return false;
    }

    // The civilian is only generous while passing by; a stopped civilian
    // doesn't donate.
    let is_moving = npc
        .actor_data()
        .map(|a| a.action_state.is_moving())
        .unwrap_or(false);
    if !is_moving {
        return false;
    }

    // If the beggar is a PC and the civilian's AI has been told not to
    // fall for the beggar trick (script hook), skip.
    let ai_got_trick = npc
        .ai_controller()
        .map(|ai| ai.got_the_beggar_trick)
        .unwrap_or(false);
    if beggar.element_data().kind.is_pc() && ai_got_trick {
        return false;
    }

    // Same-sector proximity prefilter.
    if npc.element_data().sector() != beggar.element_data().sector() {
        return false;
    }

    // The "facing check" vector is
    //   v_beggar_me = npc.pos - beggar.pos + 20 * npc.dir_vec
    // which extends the geometry forward by 20 units along the
    // civilian's heading. The MaxNorm of this vector must be ≤ 70 and
    // its 16-sector direction must lie within ±1 of the beggar's
    // facing (sectors 15 / 0 / 1 after XORing directions) for the
    // beggar to count as "in front of and looking at" the civilian.
    let npc_pos = npc.element_data().position_map();
    let beggar_pos = beggar.element_data().position_map();
    let (dx_dir, dy_dir) = crate::element::direction_vector_16(npc.element_data().direction());
    let vx = npc_pos.x - beggar_pos.x + 20.0 * dx_dir;
    let vy = npc_pos.y - beggar_pos.y + 20.0 * dy_dir;
    let max_norm = vx.abs().max(vy.abs());
    if max_norm > BEGGAR_PROXIMITY {
        return false;
    }

    let beggar_me_sector = vector_to_sector_0_to_15_iso(vx, vy);
    let delta = (beggar_me_sector - beggar.element_data().direction()).rem_euclid(16);
    matches!(delta, 15 | 0 | 1)
}

/// Civilian action: drop one coin in front of `npc_id` aimed at
/// `beggar_id`.
///
/// Decrements the civilian's `money` by [`COIN_VALUE`], spawns a coin
/// projectile with `belongs_to_beggar=true`, and sets
/// `has_given_money_to_beggar` so the civilian is retired from the
/// donor pool. The landing point is halfway between civilian and
/// beggar when the straight-line path is clear, else directly at the
/// civilian's feet.
fn give_money_to_beggar(
    engine: &mut EngineInner,
    assets: &crate::engine::LevelAssets,
    npc_id: EntityId,
    beggar_id: EntityId,
) {
    // ── Gather source / target geometry under immutable borrows. ──
    let (source_pos, layer, source_sector, move_box, npc_pos_2d) = {
        let Some(npc) = engine.get_entity(npc_id) else {
            return;
        };
        let elem = npc.element_data();
        // Toss from 5 units in front of the belt so the coin leaves
        // the civilian's silhouette.
        let belt = npc
            .compute_belt_point()
            .unwrap_or(crate::coordinates::WorldPoint3D {
                x: elem.position_map().x,
                y: elem.position_map().y,
                z: 0.0,
            });
        let source = beggar_coin_source(belt, elem.direction());
        let move_box = *npc.position_iface().get_move_box();
        (
            source,
            elem.layer(),
            elem.sector(),
            move_box,
            elem.position_map(),
        )
    };

    let beggar_pos = match engine.get_entity(beggar_id) {
        Some(e) => e.element_data().position(),
        None => return,
    };
    let beggar_pos_2d = beggar_pos.to_map();

    // When the space between civilian and beggar is clear, toss to the
    // midpoint so the coin lands in the PC's lap; otherwise drop it at
    // the civilian's own feet.
    let los_clear = engine.world.fast_grid.is_straight_movement_authorized(
        MapPoint::new(npc_pos_2d.x, npc_pos_2d.y),
        beggar_pos_2d,
        layer,
        &move_box,
    );
    let target_pos = beggar_coin_target(source_pos, beggar_pos, los_clear);

    // ── Spawn the coin with `belongs_to_beggar = true`. ──
    let coin = {
        let obstacle_check = bow_shot::TrajectoryObstacleCheck {
            fast_find_grid: &engine.world.fast_grid,
            sight_obstacles: engine.sight_obstacles(assets),
            water_zones: Some(&assets.water_zones),
        };
        bow_shot::spawn_coin(
            None,
            source_pos,
            target_pos,
            crate::position_interface::Layer::new(layer),
            Some(crate::position_interface::Layer::ZERO),
            None,
            bow_shot::APEX_BEGGAR_COIN,
            Some(&obstacle_check),
        )
    };
    let coin_id = engine.with_simulation_context(|engine, sim| {
        engine.publish_primed_coin(
            sim,
            assets,
            coin,
            npc_pos_2d,
            source_sector,
            crate::position_interface::Layer::new(layer),
        )
    });
    // Original SetBelongsToBeggar follows AddElement.
    let Some(Entity::Projectile(coin)) = engine.world.entities.get_mut(coin_id) else {
        unreachable!("published beggar coin changed entity kind")
    };
    coin.object.belongs_to_beggar = true;

    // ── Debit the civilian and retire them from the donor pool. ──
    if let Some(npc) = engine.get_entity_mut(npc_id)
        && let Some(npc_data) = npc.npc_data_mut()
    {
        // Guard against underflow; civilians below one coin's value
        // just bank the "given" flag without debiting negative.
        if npc_data.money >= COIN_VALUE {
            npc_data.money -= COIN_VALUE;
        }
        npc_data.has_given_money_to_beggar = true;
    }

    tracing::debug!(
        ?npc_id,
        ?beggar_id,
        ?coin_id,
        los_clear,
        "beggar: civilian tossed a coin"
    );
}

impl EngineInner {
    /// PC side: solicit a donation from the first civilian in range.
    ///
    /// Iterates every NPC and fires
    /// [`give_money_to_beggar`] against the first one whose predicate
    /// ([`can_give_money_to_beggar`]) passes. Called each tick while
    /// `beggar_id` wears the `SimulatingBeggar` disguise.
    fn bid_for_money(&mut self, assets: &crate::engine::LevelAssets, beggar_id: EntityId) {
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for npc_id in npc_ids {
            if can_give_money_to_beggar(self, npc_id, beggar_id) {
                give_money_to_beggar(self, assets, npc_id, beggar_id);
                return;
            }
        }
    }

    /// Per-frame driver for the beggar solicitation loop.
    ///
    /// Run the `RHANIMATION_SIMULATING_BEGGAR` bid belonging to one selected
    /// PC Execute arm. Donors are searched in live NPC creation order.
    pub(crate) fn tick_beggar_bid_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        pc_id: EntityId,
        order_id: std::num::NonZeroU32,
    ) {
        let Some(entity) = self.get_entity(pc_id) else {
            panic!("beggar owner {pc_id:?} disappeared in its Actor Hourglass slot");
        };
        assert!(matches!(entity, Entity::Pc(_)), "beggar owner must be a PC");
        let sprite_frozen = self.actors_frozen();
        let pc = self
            .world
            .entities
            .get_mut(pc_id)
            .expect("validated beggar owner disappeared");
        // TurnFast precedes PerformAction in Original and is not a sprite
        // increment, so FrozenAll still permits the turn and following Bid.
        pc.position_iface_mut().turn();
        let direction = u16::try_from(pc.element_data().direction())
            .expect("beggar direction must be in the canonical 0..=15 range");
        let motion = if sprite_frozen {
            crate::sprite::MotionState::InProgress
        } else {
            pc.element_data_mut().sprite.perform_action(
                sim,
                Some(order_id),
                crate::order::OrderType::SimulatingBeggar,
                direction,
                crate::sprite::FrameProgression::Default,
                false,
            )
        };
        if motion == crate::sprite::MotionState::Start {
            let pc = self.world.entities.get_mut(pc_id).unwrap();
            pc.set_posture(crate::element::Posture::SimulatingBeggar);
            pc.actor_data_mut().unwrap().action_state = crate::element::ActionState::Waiting;
        }
        self.bid_for_money(assets, pc_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coin_geometry_uses_iso_forward_and_stored_world_target() {
        let belt = crate::coordinates::WorldPoint3D::new(10.0, 20.0, 30.0);
        let direction = 2;
        let (dx, dy) = crate::element::direction_vector_16(direction);
        let source = beggar_coin_source(belt, direction);
        assert_eq!(source.x, belt.x + dx * 5.0);
        assert_eq!(
            source.y,
            belt.y + dy * crate::position_interface::ASPECT_RATIO * 5.0
        );
        assert_eq!(source.z, belt.z);

        // World-y deliberately differs from map-y (`y - z`): target the
        // stored Position exactly, not a reconstructed map/belt point.
        let beggar = crate::coordinates::WorldPoint3D::new(50.0, 80.0, 17.0);
        let target = beggar_coin_target(source, beggar, true);
        assert!((target.x - (source.x + beggar.x) * 0.5).abs() < 1.0e-5);
        assert!((target.y - (source.y + beggar.y) * 0.5).abs() < 1.0e-5);
        assert!((target.z - (source.z + beggar.z) * 0.5).abs() < 1.0e-5);
        assert_eq!(beggar_coin_target(source, beggar, false), source);
    }
}
