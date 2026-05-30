//! Purse / coin lifecycle on the engine side.
//!
//! Drives the purse and coin per-frame behaviour:
//!
//! 1. **Purse impact**: when a thrown purse lands, the purse's trajectory
//!    is exhausted by [`bow_shot`].  This module detects that, calls
//!    [`burst_purse`] to scatter `NUMBER_OF_COINS_IN_PURSE` coin
//!    projectiles around the impact point, plays the purse-burst SFX,
//!    and emits a PLING noise.
//!
//! 2. **Coin landing**: when a coin's trajectory finishes, the coin is
//!    registered as a `DetectableType::Object` with every NPC so the
//!    soldier-distraction AI (`EventSeesObject`) fires.  The coin
//!    transitions into its `ObjectBursting` animation and waits to be
//!    picked up.
//!
//! 3. **Coin pickup**: PCs auto-pick coins in proximity (extension of
//!    [`super::EngineInner::tick_bonus_auto_pickup`]).  When the picked-up
//!    coin has a `source_purse`, the pickup routes through
//!    [`EngineInner::take_purse`] which deactivates *every* still-active
//!    sibling coin in one go and credits the cumulative ransom value.
//!
//! 4. **Purse Hourglass**: the bursted purse element stays alive
//!    forever in `ObjectBursting` so the empty-pouch sprite remains as
//!    decoration; the per-tick drain only prunes dead child handles
//!    off the purse's `child_coins` list so `take_purse` doesn't iterate
//!    them.

use super::EngineInner;
use crate::bow_shot::{self, COIN_SCATTER_MIN, NUMBER_OF_COINS_IN_PURSE};
use crate::coordinates::MapPoint;
use crate::coordinates::WorldPoint3D;
use crate::element::{Animation, DetectableType, Entity, EntityId, ObjectType};
use crate::fast_find_grid::SectorHit;

/// Purse-impact FX id.
const FX_PURSE_IMPACT: u32 = 506;

/// Per-material coin-shower FX ids.
fn coin_fx_for_material(material: crate::element::GameMaterial) -> u32 {
    use crate::element::GameMaterial as M;
    match material {
        M::Ground => 481,
        M::Wood => 500,
        M::Stone => 493,
        M::Ice => 487,
        // Leaves / Bush / Hole / Grass / default → 474
        _ => 474,
    }
}

impl EngineInner {
    /// Per-frame tick for purses and coins.  Drives:
    ///
    /// * **Purse trajectory advancement** until impact, then burst.
    /// * **Coin trajectory advancement** until landing, then detectable
    ///   broadcast.
    /// * **Purse Hourglass** post-burst — prunes dead/taken child handles
    ///   off the purse's `child_coins` list (the empty pouch stays alive
    ///   forever as decoration).
    pub(super) fn tick_purses_and_coins(&mut self, assets: &crate::engine::LevelAssets) {
        if self.freeze_all {
            return;
        }

        // ── Phase 1: trajectory advancement + impact detection ──────
        //
        // Pop trajectory waypoints and interpolate per frame until the
        // trajectory list is empty.  We replicate the minimum needed for
        // purses/coins inline here — the bow_shot::tick_arrows path
        // already filters us out by `object_type != Arrow`, so no double
        // motion update.

        struct Impact {
            id: EntityId,
            kind: ImpactKind,
        }
        enum ImpactKind {
            PurseLanded { pos: WorldPoint3D, layer: u16 },
            CoinLanded { pos: WorldPoint3D, layer: u16 },
        }
        let mut impacts: Vec<Impact> = Vec::new();

        for (id, entity) in crate::engine::occupied_entity_slots_mut(&mut self.entities) {
            if !entity.element_data().active {
                continue;
            }
            let Entity::Projectile(proj) = entity else {
                continue;
            };
            let object_type = proj.object.object_type;
            if !matches!(object_type, ObjectType::Purse | ObjectType::Coin) {
                continue;
            }
            if !proj.projectile.flying {
                continue;
            }

            // Advance trajectory by one frame via the shared helper that
            // also drives arrow ticks.  Returns true when the trajectory
            // ran out — the projectile has landed.
            let exhausted = proj.advance_trajectory_one_frame();
            if exhausted {
                let pos = proj.element.position();
                let layer = proj.element.layer();
                impacts.push(Impact {
                    id,
                    kind: match object_type {
                        ObjectType::Purse => ImpactKind::PurseLanded { pos, layer },
                        ObjectType::Coin => ImpactKind::CoinLanded { pos, layer },
                        _ => unreachable!(),
                    },
                });
            }
        }

        // ── Phase 2: handle impacts ────────────────────────────────
        //
        // The mutable-borrow on `self.entities` is released; we can now
        // call back into `&mut self` for noise broadcasts, detectable
        // dispatch, and child-coin spawning.
        for Impact { id, kind } in impacts {
            let resolution = self.apply_projectile_landing_resolution(assets, id);
            let landed_layer = resolution
                .filter(|r| !r.blocked_by_motion_obstacle)
                .map(|r| r.layer);
            match kind {
                ImpactKind::PurseLanded { pos, layer } => {
                    self.burst_purse(assets, id, pos, landed_layer.unwrap_or(layer))
                }
                ImpactKind::CoinLanded { pos, layer } => {
                    self.coin_landed(id, pos, landed_layer.unwrap_or(layer))
                }
            }
        }

        // ── Phase 3: Purse Hourglass — prune dead children ──────────
        //
        // The bursted purse element stays alive forever in the bursting
        // animation row with freeze-when-terminated — the empty pouch
        // sprite stays on the ground as visible loot decoration until
        // the level unloads.  We *don't* deactivate the purse here; we
        // only prune child handles for dead/taken coins so the
        // click-to-take-all path can iterate the live ones.  The only
        // despawn paths are `take_purse` (clicking the purse) and level
        // unload.
        let purses_to_check: Vec<EntityId> = self
            .entities_iter_with_id()
            .filter_map(|(id, entity)| match entity {
                Entity::Projectile(p)
                    if p.element.active
                        && p.object.object_type == ObjectType::Purse
                        && p.projectile.purse.burst
                        && !p.projectile.purse.child_coins.is_empty() =>
                {
                    Some(id)
                }
                _ => None,
            })
            .collect();

        for purse_id in purses_to_check {
            let children: Vec<EntityId> = match self.get_entity(purse_id) {
                Some(Entity::Projectile(p)) => p.projectile.purse.child_coins.clone(),
                _ => continue,
            };
            let alive: Vec<EntityId> = children
                .into_iter()
                .filter(|cid| {
                    self.get_entity(*cid)
                        .map(|e| {
                            e.element_data().active
                                && !matches!(
                                    e,
                                    Entity::Projectile(p) if p.object.taken
                                )
                        })
                        .unwrap_or(false)
                })
                .collect();
            if let Some(Entity::Projectile(purse)) = self
                .entities
                .get_mut(purse_id.index() as usize)
                .and_then(|s| s.as_mut())
            {
                purse.projectile.purse.child_coins = alive;
            }
        }
    }

    /// Burst a landed purse into [`NUMBER_OF_COINS_IN_PURSE`] child
    /// coins scattered around the impact point.
    fn burst_purse(
        &mut self,
        assets: &crate::engine::LevelAssets,
        purse_id: EntityId,
        impact_pos: WorldPoint3D,
        layer: u16,
    ) {
        // ── Resolve the shooter's MoveBox ──────────────────────────
        //
        // Used both for impact-position correction and the per-coin
        // accessibility loop.  Without a shooter (e.g. a purse loaded
        // from a save with no live thrower), fall back to a 1-unit box
        // so the grid checks still terminate.
        let shooter_id = self.get_entity(purse_id).and_then(|e| match e {
            Entity::Projectile(p) => p.projectile.shooter,
            _ => None,
        });
        let shooter_move_box = shooter_id
            .and_then(|sid| self.get_entity(sid))
            .map(|e| e.position_iface())
            .map(|pi| *pi.get_move_box())
            .unwrap_or_else(|| crate::geo2d::BBox2D::from_coords(-1.0, -1.0, 1.0, 1.0));

        // ── Position correction ────────────────────────────────────
        //
        // Push the impact box to a walkable spot, then use the
        // corrected centre as the coin-spawn source.  Falls back to the
        // raw impact when the level is invalid or no walkable spot is
        // found within search radius.
        let mut corrected_2d = MapPoint {
            x: impact_pos.x,
            y: impact_pos.y,
        };
        let mut box_at_pos =
            shooter_move_box.translated(crate::geo2d::pt(impact_pos.x, impact_pos.y));
        if self
            .fast_grid
            .find_authorized_position(&mut box_at_pos, layer)
        {
            let centre = box_at_pos.center();
            corrected_2d = MapPoint {
                x: centre.x,
                y: centre.y,
            };
        }
        let source_pos = WorldPoint3D {
            x: corrected_2d.x,
            y: corrected_2d.y,
            z: impact_pos.z,
        };

        // PLING noise so nearby NPCs hear the impact.
        self.broadcast_noise(
            crate::ai::NoiseType::Pling,
            crate::coordinates::MapPoint::new(source_pos.x, source_pos.y),
            layer,
            crate::parameters_ai::NOISE_VOLUME_PLING as u16,
            source_pos.z.max(0.0) as u16,
            Some(purse_id),
        );

        // Resolve the landing material via the obstacle-aware
        // `material_at_with_obstacle`, called from the purse / coin
        // trajectory builder at every landing point.  Branches:
        //   * no obstacle: scan SECTOR_SOUND polygons, fall back to the
        //     default material.
        //   * obstacle present: iterate the obstacle's material
        //     sub-sectors, falling back to the obstacle's own material.
        //     Honours heterogeneous surfaces — e.g. a stone inlay on a
        //     wooden platform.
        // Result is written onto the element so `coin_fx_for_material`
        // (and later readers) see the correct material instead of the
        // default `Ground`.
        let impact_map = crate::coordinates::MapPoint::new(source_pos.x, source_pos.y);
        let landing_obstacle_idx = self.get_entity(purse_id).and_then(|e| match e {
            Entity::Projectile(p) => p.element.obstacle_index(),
            _ => None,
        });
        let obstacle_list = self.sight_obstacles(assets);
        let landing_obstacle = landing_obstacle_idx.and_then(|h| obstacle_list.get(usize::from(h)));
        let material = assets
            .material_sectors
            .material_at_with_obstacle(landing_obstacle, impact_map);
        if let Some(Entity::Projectile(p)) = self
            .entities
            .get_mut(purse_id.index() as usize)
            .and_then(|s| s.as_mut())
        {
            p.element.set_material(material);
        }

        // PlayCoinFx — material-keyed coin-shower SFX.  Use the raw
        // `impact_pos` (uncorrected 2D position captured before
        // `find_authorized_position` shifts it) instead of the
        // corrected `source_pos`.
        self.pending_side_effects
            .sounds
            .push(super::SoundCommand::Fx {
                fx_id: coin_fx_for_material(material),
                position: MapPoint::new(impact_pos.x, impact_pos.y),
                material: None,
            });

        // Bonus impact FX (the purse hitting the ground itself).
        self.pending_side_effects
            .sounds
            .push(super::SoundCommand::Fx {
                fx_id: FX_PURSE_IMPACT,
                position: MapPoint::new(source_pos.x, source_pos.y),
                material: None,
            });

        // ── Spawn child coins ──────────────────────────────────────
        //
        // For each coin, try up to `COIN_SCATTER_ATTEMPTS` random
        // scatter positions, accept the first one reachable from the
        // corrected source via `is_straight_movement_authorized`; if
        // none reachable, fall back to the source position itself.
        let mut spawned_children: Vec<EntityId> =
            Vec::with_capacity(NUMBER_OF_COINS_IN_PURSE as usize);
        for _ in 0..NUMBER_OF_COINS_IN_PURSE {
            let mut goal_2d = MapPoint {
                x: source_pos.x,
                y: source_pos.y,
            };
            for _ in 0..bow_shot::COIN_SCATTER_ATTEMPTS {
                // Random direction in one of 16 sectors, magnitude in
                // [COIN_SCATTER_MIN, COIN_SCATTER_MIN+31].
                let sector = (self.rng.u32(..) & 15) as i16;
                let magnitude = COIN_SCATTER_MIN + (self.rng.u32(..) & 31) as f32;
                let (ux, uy) = crate::element::direction_vector_16(sector);
                // Y is compressed by ASPECT_RATIO to match isometric ground.
                let scatter_x = ux * magnitude;
                let scatter_y = uy * magnitude * crate::position_interface::ASPECT_RATIO;
                let candidate = MapPoint {
                    x: source_pos.x + scatter_x,
                    y: source_pos.y + scatter_y,
                };
                if self.fast_grid.is_straight_movement_authorized(
                    MapPoint::new(corrected_2d.x, corrected_2d.y),
                    candidate,
                    layer,
                    &shooter_move_box,
                ) {
                    goal_2d = candidate;
                    break;
                }
            }
            // Compute the goal Z via the projection-area top plane at
            // `(goal_2d.x, goal_2d.y)` on the purse's sector.  A
            // scattered coin landing on a ramp / stairs / neighbouring
            // projection area needs its top-plane Z to feed
            // `compute_initial_throw_velocity`; reusing `source_pos.z`
            // would skew the arc when source + goal sit on
            // different-slope projection areas.
            let purse_sector = self
                .get_entity(purse_id)
                .map(|e| e.position_iface().get_sector())
                .unwrap_or(None);
            let target_pos: WorldPoint3D = self
                .position_to_point_3d(assets, purse_sector, layer, goal_2d.x, goal_2d.y)
                .into();

            let goal_grid_pt = crate::coordinates::MapPoint::new(goal_2d.x, goal_2d.y);
            let target_sector = match self.fast_grid.get_sector(goal_grid_pt, goal_grid_pt, layer) {
                SectorHit::Found { sector_number, .. } => u16::try_from(sector_number.get())
                    .ok()
                    .and_then(crate::position_interface::SectorHandle::new),
                SectorHit::Blocked | SectorHit::None => None,
            };
            let coin = bow_shot::spawn_coin(
                Some(purse_id),
                source_pos,
                target_pos,
                layer,
                layer,
                target_sector,
                bow_shot::APEX_COIN,
                None,
            );
            let coin_id = self.add_entity(coin);
            self.attach_accessory_sprite(assets, coin_id);
            spawned_children.push(coin_id);
        }

        // ── Update the purse's bookkeeping ─────────────────────────
        //
        // The invariant `number_of_coins >= NUMBER_OF_COINS_IN_PURSE`
        // holds because `spawn_purse` initialises the counter to
        // `NUMBER_OF_COINS_IN_PURSE`.  We use `saturating_sub`
        // defensively so a save-loaded purse with a stale 0 counter
        // still terminates cleanly.  The purse switches into its
        // `ObjectBursting` animation row and becomes non-pickable; the
        // element itself stays alive (the empty purse sprite remains
        // as decoration).
        if let Some(Entity::Projectile(purse)) = self
            .entities
            .get_mut(purse_id.index() as usize)
            .and_then(|s| s.as_mut())
        {
            debug_assert!(
                purse.projectile.purse.number_of_coins >= NUMBER_OF_COINS_IN_PURSE,
                "purse {purse_id:?} should hold ≥ {NUMBER_OF_COINS_IN_PURSE} coins at burst time, \
                 found {}",
                purse.projectile.purse.number_of_coins
            );
            purse.projectile.purse.burst = true;
            purse.projectile.purse.child_coins = spawned_children;
            purse.projectile.purse.number_of_coins = purse
                .projectile
                .purse
                .number_of_coins
                .saturating_sub(NUMBER_OF_COINS_IN_PURSE);
            purse.object.animation = Animation::ObjectBursting;
            // Burst does NOT mark the purse as taken — the takable
            // flag only flips when the player actually clicks
            // (`take_purse`).  The auto-pickup proximity filter must
            // skip bursted purses explicitly (their value is in the
            // child coins now); see `tick_bonus_auto_pickup` in
            // `engine/combat.rs`.
        }

        tracing::debug!(
            ?purse_id,
            x = source_pos.x,
            y = source_pos.y,
            coins = NUMBER_OF_COINS_IN_PURSE,
            "Purse: burst on impact, scattered child coins"
        );
    }

    /// Handle a coin landing.  Snaps the coin to its goal sector /
    /// layer, switches to the bursting animation, and registers the
    /// coin as a `DETECTABLE_OBJECT` for every NPC so soldiers'
    /// `EventSeesObject` fires.  `layer` is the layer the trajectory
    /// finished at (used as fallback when no goal layer was recorded).
    fn coin_landed(&mut self, coin_id: EntityId, impact_pos: WorldPoint3D, layer: u16) {
        if let Some(Entity::Projectile(coin)) = self
            .entities
            .get_mut(coin_id.index() as usize)
            .and_then(|s| s.as_mut())
        {
            // Snap to the resolved goal stored at spawn.  Falls back to
            // the trajectory-end layer when the scatter-time
            // accessibility search couldn't pin a goal sector (no
            // shooter MoveBox / unreachable scatter target).
            coin.element
                .set_layer(if coin.projectile.purse.layer_goal != 0 {
                    coin.projectile.purse.layer_goal
                } else {
                    layer
                });
            if let Some(sg) = coin.projectile.purse.sector_goal {
                coin.element.set_sector(Some(sg));
            }
            // Eager `ObjectBursting` write — collapses what would
            // otherwise be a one-frame gap before the next animation
            // tick switches it; observably identical (the per-tick
            // animation driver in `animation.rs` only advances frames,
            // it doesn't re-check last-animation), and keeps the
            // impact handler the single source of truth for the
            // bursting state.
            coin.object.animation = Animation::ObjectBursting;
        }

        // Register as detectable — the AI distraction hook.  Per coin
        // landing (the coin-shower FX is on the *purse* burst, but
        // `DETECTABLE_OBJECT` is on each *coin* landing).
        self.add_detectable_for_all_npc(coin_id, DetectableType::Object);

        tracing::trace!(
            ?coin_id,
            x = impact_pos.x,
            y = impact_pos.y,
            "Coin: landed, registered as DETECTABLE_OBJECT"
        );
    }

    /// Take every still-active child coin attached to a purse and
    /// return the cumulative ransom value.
    ///
    /// Iterates the child-coin list, deactivates every coin still in
    /// the world, and returns `live_count * COIN_VALUE`.  The purse
    /// itself is flagged as taken so subsequent click-forwarding from
    /// a stray coin fall-through skips the forwarding branch.
    ///
    /// `purse_id` may point at a non-purse or absent entity, in which
    /// case 0 is returned and no state changes.
    pub(super) fn take_purse(&mut self, purse_id: EntityId) -> u32 {
        // Snapshot the child handles up front so we can deactivate them
        // without holding nested borrows on `self.entities`.
        let children: Vec<EntityId> = match self.get_entity(purse_id) {
            Some(Entity::Projectile(p))
                if p.object.object_type == crate::element::ObjectType::Purse =>
            {
                p.projectile.purse.child_coins.clone()
            }
            _ => return 0,
        };
        let mut collected: u32 = 0;
        for cid in children {
            if let Some(Entity::Projectile(c)) = self
                .entities
                .get_mut(cid.index() as usize)
                .and_then(|s| s.as_mut())
                && c.element.active
                && !c.object.taken
            {
                collected = collected.saturating_add(crate::inventory::COIN_VALUE);
                c.object.taken = true;
                c.element.active = false;
            }
        }
        if let Some(Entity::Projectile(purse)) = self
            .entities
            .get_mut(purse_id.index() as usize)
            .and_then(|s| s.as_mut())
        {
            purse.projectile.purse.child_coins.clear();
            // Flip the bonus taken flag so future click-forwarding from
            // a stray coin skips the purse path.
            purse.object.taken = true;
        }
        collected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ElementData, ElementProjectile, ObjectData, ProjectileData};

    /// Place a flying purse with an empty trajectory so it lands on the
    /// next tick — easy to test the burst → coin spawn pipeline without
    /// the throw-velocity setup.
    fn spawn_landing_purse(
        engine: &mut EngineInner,
        pos: WorldPoint3D,
        layer: u16,
        thrower: Option<EntityId>,
    ) -> EntityId {
        let mut projectile = ProjectileData {
            flying: true,
            shooter: thrower,
            ..ProjectileData::default()
        };
        projectile.purse.number_of_coins = NUMBER_OF_COINS_IN_PURSE;
        let mut element = ElementData {
            kind: crate::element::ElementKind::ObjectProjectile,
            active: true,
            ..Default::default()
        };
        element.set_position(pos);
        element.set_position_map(MapPoint { x: pos.x, y: pos.y });
        element.set_layer(layer);
        let entity = Entity::Projectile(ElementProjectile {
            element,
            object: ObjectData {
                associated_action: crate::profiles::Action::Purse,
                object_type: ObjectType::Purse,
                animation: Animation::ObjectFlying,
                quantity: 1,
                ..Default::default()
            },
            projectile,
        });
        engine.add_entity(entity)
    }

    #[test]
    fn purse_burst_spawns_child_coins_and_marks_purse() {
        let mut engine = EngineInner::new();
        let purse_id = spawn_landing_purse(
            &mut engine,
            WorldPoint3D {
                x: 100.0,
                y: 200.0,
                z: 0.0,
            },
            0,
            None,
        );

        let assets = crate::engine::LevelAssets::new();
        engine.tick_purses_and_coins(&assets);

        // Purse should be marked as burst with N child coin handles.
        let purse = engine.get_entity(purse_id).expect("purse still alive");
        let Entity::Projectile(p) = purse else {
            panic!("purse should still be a projectile entity");
        };
        assert!(p.projectile.purse.burst, "purse should be flagged burst");
        assert_eq!(
            p.projectile.purse.child_coins.len(),
            NUMBER_OF_COINS_IN_PURSE as usize,
            "purse should have N child coin handles after burst"
        );
        assert_eq!(p.projectile.purse.number_of_coins, 0);
        // Burst should NOT set `taken` — only `take_purse` does.
        assert!(
            !p.object.taken,
            "burst should leave the purse takable until take_purse fires"
        );
        assert!(!p.projectile.flying, "burst purse should no longer fly");
        // Animation switched to bursting row.
        assert_eq!(p.object.animation, Animation::ObjectBursting);

        // Each child coin should be present, point back at the purse,
        // and start out flying along its own trajectory.
        let child_ids = p.projectile.purse.child_coins.clone();
        for cid in &child_ids {
            let coin = engine.get_entity(*cid).expect("child coin alive");
            let Entity::Projectile(c) = coin else {
                panic!("child {cid:?} should be a projectile coin");
            };
            assert_eq!(c.object.object_type, ObjectType::Coin);
            assert_eq!(c.projectile.purse.source_purse, Some(purse_id));
        }
    }

    #[test]
    fn take_purse_collects_all_remaining_coins() {
        // `take_purse` returns `live_coin_count * COIN_VALUE`,
        // deactivates every active child coin, clears the child list,
        // and flips the purse's takable flag.
        let mut engine = EngineInner::new();
        let purse_id = spawn_landing_purse(
            &mut engine,
            WorldPoint3D {
                x: 100.0,
                y: 200.0,
                z: 0.0,
            },
            0,
            None,
        );
        let assets = crate::engine::LevelAssets::new();
        engine.tick_purses_and_coins(&assets);
        let coin_ids: Vec<EntityId> = match engine.get_entity(purse_id) {
            Some(Entity::Projectile(p)) => p.projectile.purse.child_coins.clone(),
            _ => panic!("purse missing"),
        };
        assert_eq!(coin_ids.len(), NUMBER_OF_COINS_IN_PURSE as usize);

        let collected = engine.take_purse(purse_id);
        assert_eq!(
            collected,
            NUMBER_OF_COINS_IN_PURSE as u32 * crate::inventory::COIN_VALUE,
            "should harvest every coin's value at once"
        );

        // All child coins should now be inactive + flagged taken.
        for cid in &coin_ids {
            let coin = engine.get_entity(*cid).expect("coin slot still present");
            assert!(
                !coin.element_data().active,
                "coin should be deactivated after take_purse"
            );
            if let Entity::Projectile(c) = coin {
                assert!(c.object.taken, "coin should be flagged taken");
            }
        }

        // Source purse should have an empty child list and be flagged taken.
        let purse = engine.get_entity(purse_id).expect("purse alive");
        let Entity::Projectile(p) = purse else {
            panic!("purse still projectile");
        };
        assert!(p.projectile.purse.child_coins.is_empty());
        assert!(p.object.taken, "purse should flip taken on take_purse");

        // Calling take_purse a second time should be a no-op (no coins left).
        assert_eq!(engine.take_purse(purse_id), 0);
    }

    #[test]
    fn purse_hourglass_prunes_dead_children_but_keeps_purse_alive() {
        // The bursted purse stays alive forever in the bursting
        // animation row with freeze-when-terminated; the empty pouch
        // sprite stays as visible decoration until level unload.
        // Child handles drain off the list, but the purse element
        // itself stays active.  The actual despawn path is
        // `take_purse` (player clicks the purse) which is wired
        // through the auto-pickup proximity in the bonus tick.
        let mut engine = EngineInner::new();
        let purse_id = spawn_landing_purse(
            &mut engine,
            WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            0,
            None,
        );
        let assets = crate::engine::LevelAssets::new();
        engine.tick_purses_and_coins(&assets);
        let coin_ids: Vec<EntityId> = match engine.get_entity(purse_id) {
            Some(Entity::Projectile(p)) => p.projectile.purse.child_coins.clone(),
            _ => panic!("purse missing"),
        };
        assert_eq!(coin_ids.len(), NUMBER_OF_COINS_IN_PURSE as usize);

        // Deactivate every child coin (simulating pickup-and-removal).
        for cid in &coin_ids {
            if let Some(Entity::Projectile(c)) = engine
                .entities
                .get_mut(cid.index() as usize)
                .and_then(|s| s.as_mut())
            {
                c.element.active = false;
            }
        }

        // Run a tick — Hourglass drain should prune the child list…
        let assets = crate::engine::LevelAssets::new();
        engine.tick_purses_and_coins(&assets);
        let purse = engine
            .get_entity(purse_id)
            .expect("purse slot still present");
        let Entity::Projectile(p) = purse else {
            panic!("purse still projectile");
        };
        assert!(
            p.projectile.purse.child_coins.is_empty(),
            "all dead/inactive children should be pruned from the list"
        );
        // …but the purse itself stays active.
        assert!(
            p.element.active,
            "purse element should remain active post-drain"
        );
        assert!(
            p.projectile.purse.burst,
            "purse should still be flagged as burst"
        );
    }
}
