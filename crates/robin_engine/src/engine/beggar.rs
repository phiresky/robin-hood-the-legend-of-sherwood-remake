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
use crate::element::{Entity, EntityId, ObjectType, Posture};
use crate::inventory::COIN_VALUE;
use crate::position_interface::vector_to_sector_0_to_15_iso;

/// Minimum money a civilian must carry to consider donating to a beggar.
const MIN_MONEY_FOR_BEGGAR_GIFT: u32 = 200;

/// MaxNorm proximity window (map units) within which a civilian will
/// notice a beggar in front of them.
const BEGGAR_PROXIMITY: f32 = 70.0;

/// Radius (MaxNorm, map units) for the near-coins toggle sweep.
const NEAR_COINS_RADIUS: f32 = 100.0;

/// Civilian predicate: can this NPC toss a coin to `beggar_pc` right now?
///
/// Reads: NPC's `has_given_money_to_beggar`, `got_the_beggar_trick`,
/// `money`, `direction`, `position_map`, `sector`, `ai_state`, and the
/// human kind. Returns `false` (no donation) in every failure arm.
fn can_give_money_to_beggar(engine: &EngineInner, npc_id: EntityId, beggar_id: EntityId) -> bool {
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
    let (source_pos, layer, move_box, npc_pos_2d) = {
        let Some(npc) = engine.get_entity(npc_id) else {
            return;
        };
        let elem = npc.element_data();
        let (dx_dir, dy_dir) = crate::element::direction_vector_16(elem.direction());
        // Toss from 5 units in front of the belt so the coin leaves
        // the civilian's silhouette.
        let belt = npc
            .compute_belt_point()
            .unwrap_or(crate::coordinates::WorldPoint3D {
                x: elem.position_map().x,
                y: elem.position_map().y,
                z: 0.0,
            });
        let source = crate::coordinates::WorldPoint3D {
            x: belt.x + dx_dir * 5.0,
            y: belt.y + dy_dir * 5.0,
            z: belt.z,
        };
        let move_box = *npc.position_iface().get_move_box();
        (source, elem.layer(), move_box, elem.position_map())
    };

    let beggar_pos = match engine.get_entity(beggar_id) {
        Some(e) => {
            let elem = e.element_data();
            crate::coordinates::WorldPoint3D {
                x: elem.position_map().x,
                y: elem.position_map().y,
                z: e.compute_belt_point().map(|p| p.z).unwrap_or(0.0),
            }
        }
        None => return,
    };
    let beggar_pos_2d = crate::element::Point2D {
        x: beggar_pos.x,
        y: beggar_pos.y,
    };

    // When the space between civilian and beggar is clear, toss to the
    // midpoint so the coin lands in the PC's lap; otherwise drop it at
    // the civilian's own feet.
    let los_clear = engine.fast_grid.is_straight_movement_authorized(
        crate::geo2d::pt(npc_pos_2d.x, npc_pos_2d.y).into(),
        crate::geo2d::pt(beggar_pos_2d.x, beggar_pos_2d.y).into(),
        layer,
        &move_box,
    );
    let target_pos = if los_clear {
        crate::coordinates::WorldPoint3D {
            x: source_pos.x + 0.5 * (beggar_pos.x - source_pos.x),
            y: source_pos.y + 0.5 * (beggar_pos.y - source_pos.y),
            z: source_pos.z + 0.5 * (beggar_pos.z - source_pos.z),
        }
    } else {
        source_pos
    };

    // ── Spawn the coin with `belongs_to_beggar = true`. ──
    let mut coin = bow_shot::spawn_coin(
        None,
        source_pos,
        target_pos,
        layer,
        layer,
        None,
        bow_shot::APEX_BEGGAR_COIN,
        None,
    );
    // Set `belongs_to_beggar` before `add_entity` so the initial render
    // already reflects the flag.
    if let Entity::Projectile(p) = &mut coin {
        p.object.belongs_to_beggar = true;
    }
    let coin_id = engine.add_entity(coin);
    engine.attach_accessory_sprite(assets, coin_id);

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
        let npc_ids = self.npc_ids.clone();
        for npc_id in npc_ids {
            if can_give_money_to_beggar(self, npc_id, beggar_id) {
                give_money_to_beggar(self, assets, npc_id, beggar_id);
                return;
            }
        }
    }

    /// Per-frame driver for the beggar solicitation loop.
    ///
    /// For every PC currently in `SimulatingBeggar` posture (and thus
    /// standing in place with `action_state == Waiting`), invoke
    /// [`EngineInner::bid_for_money`].
    pub(crate) fn tick_beggar_bids(&mut self, assets: &crate::engine::LevelAssets) {
        let pc_ids = self.pc_ids.clone();
        for pc_id in pc_ids {
            let is_beggar = self
                .get_entity(pc_id)
                .map(|e| e.element_data().posture == Posture::SimulatingBeggar)
                .unwrap_or(false);
            if !is_beggar {
                continue;
            }
            self.bid_for_money(assets, pc_id);
        }
    }

    /// Toggle `belongs_to_beggar` on every coin on the ground within
    /// [`NEAR_COINS_RADIUS`] map units of `pc_id`.
    ///
    /// Called with `value=true` when the PC finishes the transition
    /// into the beggar disguise, and with `value=false` on the
    /// transition out — so residual coins from other sources briefly
    /// join the beggar's "do-not-touch" pool and revert afterwards.
    pub(crate) fn set_beggar_flags_of_near_coins_on_ground(
        &mut self,
        pc_id: EntityId,
        value: bool,
    ) {
        let Some(pc) = self.get_entity(pc_id) else {
            return;
        };
        let pc_pos = pc.element_data().position_map();

        for slot in self.entities.iter_mut() {
            let Some(entity) = slot.as_mut() else {
                continue;
            };
            let pos = entity.element_data().position_map();
            let dist = (pc_pos.x - pos.x).abs().max((pc_pos.y - pos.y).abs());
            if dist >= NEAR_COINS_RADIUS {
                continue;
            }
            if let Some(obj) = entity.object_data_mut()
                && obj.object_type == ObjectType::Coin
            {
                obj.belongs_to_beggar = value;
            }
        }
    }
}
