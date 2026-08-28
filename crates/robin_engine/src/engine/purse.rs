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
//! 3. **Coin pickup**: PCs explicitly seek and take coins.  When the
//!    picked-up coin has a `source_purse`, the pickup routes through
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
use crate::element::{Animation, DetectableType, ElementProjectile, Entity, EntityId, ObjectType};
use crate::entity_id::EntityIdKind;

/// Purse-impact FX id.
const FX_PURSE_IMPACT: u32 = 506;

#[cfg(test)]
thread_local! {
    static WATER_IMPACT_ORDER: std::cell::RefCell<Vec<&'static str>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

#[cfg(test)]
fn observe_water_impact_stage(stage: &'static str) {
    WATER_IMPACT_ORDER.with(|order| order.borrow_mut().push(stage));
}

#[cfg(not(test))]
#[inline]
fn observe_water_impact_stage(_stage: &'static str) {}

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
    fn add_projectile_water_titbit(&mut self, position: WorldPoint3D, layer: u16) {
        use crate::titbit::{ElementHandle, INVALID_ID, TitbitKind};
        self.feedback.titbit_manager.add_titbit(
            position,
            layer,
            TitbitKind::Plouf,
            ElementHandle::INVALID,
            0,
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            None,
            None,
        );
        observe_water_impact_stage("titbit");
    }

    fn finish_projectile_water_impact(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        projectile_id: EntityId,
        position: WorldPoint3D,
        layer: u16,
    ) {
        let map = MapPoint::from_world_xyz(position.x, position.y, position.z);
        self.broadcast_noise_synchronously(
            sim,
            assets,
            crate::ai::NoiseType::Plouf,
            map,
            crate::position_interface::Layer::new(layer),
            crate::parameters_ai::NOISE_VOLUME_PLOUF as u16,
            position.z.max(0.0) as u16,
            Some(projectile_id),
        );
        observe_water_impact_stage("noise");
        self.feedback
            .pending_side_effects
            .sounds
            .push(super::SoundCommand::Fx {
                fx_id: 470,
                position: map,
                material: None,
            });
        observe_water_impact_stage("fx");
    }

    /// Execute Original's complete constructor/virtual-Hourglass/AddElement
    /// boundary for a freshly thrown purse. The purse's creation order is
    /// consumed first, but its entity-array slot is assigned only after any
    /// child coins created by `HitObstacle` have been published.
    pub(super) fn publish_new_purse(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        thrower: EntityId,
        mut entity: Entity,
    ) -> EntityId {
        let creation_order = self.world.reserve_next_original_creation_order();
        let (origin_map, origin_sector, origin_layer) = {
            let thrower = self
                .get_entity(thrower)
                .unwrap_or_else(|| panic!("ThrowPurseAt lost required thrower {thrower}"));
            let element = thrower.element_data();
            (element.position_map(), element.sector(), element.layer())
        };
        let Entity::Projectile(purse) = &mut entity else {
            panic!("ThrowPurseAt received a non-projectile entity")
        };
        assert_eq!(purse.object.object_type, ObjectType::Purse);
        self.hydrate_unpublished_projectile(assets, purse);
        self.materialize_terminal_projectile_runtime(assets, purse);

        let exhausted = purse.advance_projectile_hourglass();
        let old = purse.element.sprite.position_iface.old_position();
        let new = purse.element.position();
        let shield = bow_shot::projectile_shield_holder(
            &self.world.entities,
            purse.projectile.shooter,
            old,
            new,
            purse.projectile.velocity_increment,
        );

        if let Some(holder) = shield {
            let future_id =
                EntityId::new(self.world.entities.len() as u32, EntityIdKind::Projectile);
            self.process_projectile_tick_results(
                sim,
                assets,
                vec![bow_shot::ArrowTickResult {
                    arrow: future_id,
                    hit_target: None,
                    shield_hit: Some(holder),
                    fx_target_hit: None,
                    despawn: false,
                    damage: 0,
                    impact_fx: Some(FX_PURSE_IMPACT),
                    impact_pos: self
                        .expect_entity(holder, "purse primer shield holder")
                        .element_data()
                        .position_map(),
                    human_hit_old_position: None,
                }],
            );
        }
        // TODO(original parity): Projectile::FindHumanVictim has no PURSE arm
        // in its target-point switch and reads an uninitialized C++ point.
        // Do not invent a belt/eyes anchor without a binary diagnostic that
        // pins this undefined behavior for the shipped executable.

        if exhausted && shield.is_none() {
            let material = purse.element.material();
            if material == crate::element::GameMaterial::Water || purse.projectile.dive {
                let future_purse_id =
                    EntityId::new(self.world.entities.len() as u32, EntityIdKind::Projectile);
                let position = purse.element.position();
                let layer = purse.element.layer();
                self.add_projectile_water_titbit(position, layer);
                purse.projectile.trajectory_frame_count = 0;
                purse.projectile.trajectory.clear();
                purse.projectile.trajectory_runtime.clear();
                observe_water_impact_stage("reset");
                self.finish_projectile_water_impact(sim, assets, future_purse_id, position, layer);
            } else if material != crate::element::GameMaterial::Hole && !purse.projectile.disappear
            {
                let future_purse_id = EntityId::new(
                    self.world.entities.len() as u32 + u32::from(NUMBER_OF_COINS_IN_PURSE),
                    EntityIdKind::Projectile,
                );
                let impact_sound_position =
                    self.burst_unpublished_purse(sim, assets, future_purse_id, purse);
                self.feedback
                    .pending_side_effects
                    .sounds
                    .push(super::SoundCommand::Fx {
                        fx_id: FX_PURSE_IMPACT,
                        position: impact_sound_position,
                        material: None,
                    });
            }
        }

        // Derived Purse::Hourglass animation runs after every base branch,
        // including synchronous HitObstacle child publication.
        purse
            .element
            .sprite
            .perform_virgin_increment(sim, crate::sprite::FrameProgression::SkipShadow);

        // Original SetStartOfTrajectory follows the entire virtual call,
        // including any synchronous obstacle/coin side effects.
        purse.projectile.start_of_trajectory_x = origin_map.x;
        purse.projectile.start_of_trajectory_y = origin_map.y;
        purse.projectile.trajectory_origin_sector = origin_sector.map(|sector| sector.get());
        purse.projectile.trajectory_origin_layer =
            crate::position_interface::Layer::new(origin_layer);
        // TODO: retain the arena half of `origin_sector`; the legacy field is
        // currently only the public u16 and must not be spatially guessed.

        let id = self.add_entity_with_reserved_creation_order(entity, creation_order);
        if let Entity::Projectile(purse) = self.expect_entity(id, "published primed purse") {
            for &child in &purse.projectile.purse.child_coins {
                let Entity::Projectile(coin) = self.expect_entity(child, "published purse child")
                else {
                    panic!("purse child {child} is not a projectile")
                };
                assert_eq!(coin.projectile.purse.source_purse, Some(id));
            }
        }
        id
    }

    fn hydrate_unpublished_projectile(
        &self,
        assets: &crate::engine::LevelAssets,
        projectile: &mut ElementProjectile,
    ) {
        let prototype = assets
            .accessory_sprite_prototypes
            .get(&projectile.object.object_type)
            .unwrap_or_else(|| {
                panic!(
                    "missing {:?} accessory master during pre-publication Hourglass",
                    projectile.object.object_type
                )
            });
        let position_iface = projectile.element.sprite.position_iface.clone();
        projectile.element.sprite = prototype.clone();
        projectile.element.sprite.position_iface = position_iface;
        projectile
            .element
            .sprite
            .force_animation(Animation::ObjectFlying, 0);
    }

    /// Replace the generated free-flight sentinel on the exact terminal raw
    /// impact point. ComputeTrajectory has already bound the terminal
    /// obstacle, so this lookup must use that identity and must not resolve
    /// membership again.
    fn materialize_terminal_projectile_runtime(
        &self,
        assets: &crate::engine::LevelAssets,
        projectile: &mut ElementProjectile,
    ) {
        if !projectile.projectile.terminal_material_pending {
            return;
        }
        assert_eq!(
            projectile.projectile.trajectory_runtime.len(),
            projectile.projectile.trajectory.len(),
            "generated projectile trajectory/runtime arrays lost lockstep"
        );
        let impact_index = usize::from(
            projectile
                .projectile
                .terminal_material_impact_index
                .take()
                .expect("terminal material pending without exact raw collision waypoint"),
        );
        assert!(
            impact_index < projectile.projectile.trajectory.len(),
            "terminal raw collision waypoint is outside the generated trajectory"
        );
        let raw_impact = projectile.projectile.trajectory[impact_index]
            .position
            .to_map();
        let obstacles = self.sight_obstacles(assets);
        let obstacle = projectile.element.obstacle_index().map(|handle| {
            obstacles
                .get(usize::from(handle))
                .unwrap_or_else(|| panic!("terminal projectile obstacle {handle} disappeared"))
        });
        let material = if projectile.projectile.dive {
            crate::element::GameMaterial::Water
        } else if projectile.projectile.disappear {
            crate::element::GameMaterial::Hole
        } else {
            assets
                .material_sectors
                .material_at_with_obstacle(obstacle, raw_impact)
        };
        projectile.projectile.trajectory_runtime[impact_index].bounce = true;
        projectile.projectile.trajectory_runtime[impact_index].material = material.as_u32();
        projectile.projectile.terminal_material_pending = false;
    }

    pub(super) fn publish_primed_coin(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        mut entity: Entity,
        corrected_source: MapPoint,
        source_sector: Option<crate::position_interface::SectorHandle>,
        source_layer: u16,
    ) -> EntityId {
        let creation_order = self.world.reserve_next_original_creation_order();
        let future_id = EntityId::new(self.world.entities.len() as u32, EntityIdKind::Projectile);
        let Entity::Projectile(coin) = &mut entity else {
            panic!("coin primer received non-projectile entity")
        };
        assert_eq!(coin.object.object_type, ObjectType::Coin);
        self.hydrate_unpublished_projectile(assets, coin);
        self.materialize_terminal_projectile_runtime(assets, coin);
        let exhausted = coin.advance_projectile_hourglass();
        let shield = bow_shot::projectile_shield_holder(
            &self.world.entities,
            coin.projectile.shooter,
            coin.element.sprite.position_iface.old_position(),
            coin.element.position(),
            coin.projectile.velocity_increment,
        );
        if let Some(holder) = shield {
            self.process_projectile_tick_results(
                sim,
                assets,
                vec![bow_shot::ArrowTickResult {
                    arrow: future_id,
                    hit_target: None,
                    shield_hit: Some(holder),
                    fx_target_hit: None,
                    despawn: false,
                    damage: 0,
                    impact_fx: None,
                    impact_pos: self
                        .expect_entity(holder, "coin primer shield holder")
                        .element_data()
                        .position_map(),
                    human_hit_old_position: None,
                }],
            );
        }
        if exhausted && shield.is_none() {
            let material = coin.element.material();
            if material == crate::element::GameMaterial::Water || coin.projectile.dive {
                let position = coin.element.position();
                let layer = coin.element.layer();
                self.add_projectile_water_titbit(position, layer);
                coin.projectile.trajectory_frame_count = 0;
                coin.projectile.trajectory.clear();
                coin.projectile.trajectory_runtime.clear();
                observe_water_impact_stage("reset");
                self.finish_projectile_water_impact(sim, assets, future_id, position, layer);
            } else if material != crate::element::GameMaterial::Hole && !coin.projectile.disappear {
                match coin.projectile.purse.layer_goal {
                    Some(layer) => coin.element.set_layer(layer.get()),
                    None => coin.element.clear_layer(),
                }
                coin.element.set_sector(coin.projectile.purse.sector_goal);
                self.add_detectable_for_all_npc(future_id, DetectableType::Object);
            }
        }
        coin.element
            .sprite
            .perform_virgin_increment(sim, crate::sprite::FrameProgression::SkipShadow);
        coin.projectile.start_of_trajectory_x = corrected_source.x;
        coin.projectile.start_of_trajectory_y = corrected_source.y;
        coin.projectile.trajectory_origin_sector = source_sector.map(|sector| sector.get());
        coin.projectile.trajectory_origin_layer =
            crate::position_interface::Layer::new(source_layer);
        let id = self.add_entity_with_reserved_creation_order(entity, creation_order);
        assert_eq!(
            id, future_id,
            "coin publication order changed during its virtual primer"
        );
        id
    }

    fn burst_unpublished_purse(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        future_purse_id: EntityId,
        purse: &mut ElementProjectile,
    ) -> MapPoint {
        assert!(
            purse.projectile.purse.number_of_coins >= NUMBER_OF_COINS_IN_PURSE,
            "purse {future_purse_id} has {} coins before required three-coin burst",
            purse.projectile.purse.number_of_coins
        );
        let shooter = purse
            .projectile
            .shooter
            .unwrap_or_else(|| panic!("new purse {future_purse_id} lacks required shooter"));
        let move_box = *self
            .expect_entity(shooter, "purse burst shooter")
            .position_iface()
            .get_move_box();
        let raw_impact = purse.element.position();
        let raw_map = purse.element.position_map();
        let layer = purse.element.layer();
        let sector = purse.element.sector();
        let mut box_at_pos = move_box.translated(raw_map);
        let corrected_map = if self
            .world
            .fast_grid
            .find_authorized_position(&mut box_at_pos, layer)
        {
            box_at_pos.center()
        } else {
            raw_map
        };
        self.broadcast_noise_synchronously(
            sim,
            assets,
            crate::ai::NoiseType::Pling,
            corrected_map,
            crate::position_interface::Layer::new(layer),
            crate::parameters_ai::NOISE_VOLUME_PLING as u16,
            raw_impact.z.max(0.0) as u16,
            Some(future_purse_id),
        );

        let material = purse.element.material();

        let mut children = Vec::with_capacity(usize::from(NUMBER_OF_COINS_IN_PURSE));
        for _ in 0..NUMBER_OF_COINS_IN_PURSE {
            let mut vector = crate::coordinates::MapVec::ZERO;
            for _ in 0..bow_shot::COIN_SCATTER_ATTEMPTS {
                let direction =
                    (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::PurseCoinScatter, ..) & 15)
                        as i16;
                let magnitude = COIN_SCATTER_MIN
                    + (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::PurseCoinScatter, ..) & 31)
                        as f32;
                let (ux, uy) = crate::element::direction_vector_16(direction);
                let candidate_vector = crate::coordinates::MapVec {
                    x: ux * magnitude,
                    y: uy * magnitude * crate::position_interface::ASPECT_RATIO,
                };
                let candidate = MapPoint::new(
                    raw_map.x + candidate_vector.x,
                    raw_map.y + candidate_vector.y,
                );
                if self.world.fast_grid.is_straight_movement_authorized(
                    corrected_map,
                    candidate,
                    &move_box,
                ) {
                    vector = candidate_vector;
                    break;
                }
            }
            let stored_goal = MapPoint::new(corrected_map.x + vector.x, corrected_map.y + vector.y);
            let target =
                self.position_to_point_3d(assets, sector, layer, stored_goal.x, stored_goal.y);
            let coin = {
                let obstacle_check = bow_shot::TrajectoryObstacleCheck {
                    fast_find_grid: &self.world.fast_grid,
                    sight_obstacles: self.sight_obstacles(assets),
                    water_zones: Some(&assets.water_zones),
                };
                bow_shot::spawn_coin(
                    Some(future_purse_id),
                    raw_impact,
                    target,
                    layer,
                    crate::position_interface::Layer::new(layer),
                    sector,
                    bow_shot::APEX_COIN,
                    Some(&obstacle_check),
                )
            };
            children.push(self.publish_primed_coin(
                sim,
                assets,
                coin,
                corrected_map,
                sector,
                layer,
            ));
        }
        assert_eq!(
            self.world.entities.len() as u32,
            future_purse_id.index(),
            "synchronous purse burst published an unexpected number of elements"
        );
        purse.projectile.purse.number_of_coins -= NUMBER_OF_COINS_IN_PURSE;
        purse.projectile.purse.child_coins = children;
        purse.projectile.purse.burst = true;
        // Original decrements/books every child before PlayCoinFx, then
        // deactivates the purse after the sound call.
        self.feedback
            .pending_side_effects
            .sounds
            .push(super::SoundCommand::Fx {
                fx_id: coin_fx_for_material(material),
                position: raw_map,
                material: None,
            });
        purse.element.active = false;
        raw_map
    }

    /// Per-frame tick for purses and coins.  Drives:
    ///
    /// * **Purse trajectory advancement** until impact, then burst.
    /// * **Coin trajectory advancement** until landing, then detectable
    ///   broadcast.
    /// * **Purse Hourglass** post-burst — prunes dead/taken child handles
    ///   off the purse's `child_coins` list (the empty pouch stays alive
    ///   forever as decoration).
    #[cfg(test)]
    pub(super) fn tick_purses_and_coins(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
    ) {
        let mut slot = 0;
        while slot < self.world.entities.len() {
            if let Some(id) = self.world.entities.id_at_legacy_slot(slot as u32) {
                self.tick_purse_or_coin(sim, assets, id);
            }
            slot += 1;
        }
    }

    /// Advance one purse or coin at its creation-order position.
    ///
    /// `RHEngine::PerformHourglass` rechecks `marrayElements.Size()` after
    /// every virtual call. Keeping this operation per entity lets the main
    /// tick reach coins appended by a purse impact later in the same pass.
    pub(super) fn tick_purse_or_coin(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        id: EntityId,
    ) -> bool {
        let (was_flying, object_type, mut base_result) = match self.get_entity(id) {
            Some(Entity::Projectile(projectile)) => (
                projectile.projectile.flying,
                projectile.object.object_type,
                projectile.element.active,
            ),
            _ => return false,
        };
        // ── Phase 1: trajectory advancement + impact detection ──────
        //
        // Pop trajectory waypoints and interpolate per frame until the
        // trajectory list is empty.  We replicate the minimum needed for
        // purses/coins inline here — the bow_shot::tick_arrows path
        // already filters us out by `object_type != Arrow`, so no double
        // motion update.

        enum ImpactKind {
            PurseLanded { pos: WorldPoint3D, layer: u16 },
            CoinLanded { pos: WorldPoint3D, layer: u16 },
        }
        let (mut impact, segment) = {
            let Some(Entity::Projectile(proj)) = self.world.entities.get_mut(id) else {
                return false;
            };
            let object_type = proj.object.object_type;
            if !matches!(object_type, ObjectType::Purse | ObjectType::Coin)
                || !proj.element.active
                || !proj.projectile.flying
            {
                (None, None)
            } else {
                // Advance trajectory by one frame via the shared helper that
                // also drives arrow ticks. Returns true when the trajectory
                // ran out — the projectile has landed.
                let exhausted = proj.advance_projectile_hourglass();
                let impact = exhausted.then(|| match object_type {
                    ObjectType::Purse => ImpactKind::PurseLanded {
                        pos: proj.element.position(),
                        layer: proj.element.layer(),
                    },
                    ObjectType::Coin => ImpactKind::CoinLanded {
                        pos: proj.element.position(),
                        layer: proj.element.layer(),
                    },
                    _ => unreachable!(),
                });
                (
                    impact,
                    Some((
                        proj.projectile.shooter,
                        proj.element.sprite.position_iface.old_position(),
                        proj.element.position(),
                        proj.projectile.velocity_increment,
                    )),
                )
            }
        };

        // Projectile::Hourglass checks shields after movement and returns
        // before HitObstacle. Purse/Coin inherit the base no-op HitShield;
        // only the impact sound/parry side effect and early return apply.
        if let Some((shooter, old, new, increment)) = segment
            && let Some(holder) = crate::bow_shot::projectile_shield_holder(
                &self.world.entities,
                shooter,
                old,
                new,
                increment,
            )
        {
            impact = None;
            self.process_projectile_tick_results(
                sim,
                assets,
                vec![crate::bow_shot::ArrowTickResult {
                    arrow: id,
                    hit_target: None,
                    shield_hit: Some(holder),
                    fx_target_hit: None,
                    despawn: false,
                    damage: 0,
                    impact_fx: (object_type == ObjectType::Purse).then_some(FX_PURSE_IMPACT),
                    impact_pos: self
                        .get_entity(holder)
                        .expect("projectile shield holder vanished during Hourglass")
                        .element_data()
                        .position_map(),
                    human_hit_old_position: None,
                }],
            );
        }
        // TODO(original parity): Purse FindHumanVictim is formal C++ UB
        // because its switch initializes target points only for
        // Arrow/Apple/Stone. Skip rather than inventing a belt anchor.

        // ── Phase 2: handle impacts ────────────────────────────────
        //
        // The mutable-borrow on `self.world.entities` is released; we can now
        // call back into `&mut self` for noise broadcasts, detectable
        // dispatch, and child-coin spawning.
        if let Some(kind) = impact {
            match kind {
                ImpactKind::PurseLanded { pos, layer } => {
                    let (material, dive, disappear) =
                        match self.expect_entity(id, "registered purse terminal continuation") {
                            Entity::Projectile(purse) => (
                                purse.element.material(),
                                purse.projectile.dive,
                                purse.projectile.disappear,
                            ),
                            _ => panic!("registered purse terminal changed entity kind"),
                        };
                    if material == crate::element::GameMaterial::Water || dive {
                        self.add_projectile_water_titbit(pos, layer);
                        if let Some(Entity::Projectile(purse)) = self.world.entities.get_mut(id) {
                            purse.projectile.trajectory_frame_count = 0;
                            purse.projectile.trajectory.clear();
                            purse.projectile.trajectory_runtime.clear();
                            purse.element.active = false;
                        }
                        observe_water_impact_stage("reset");
                        self.finish_projectile_water_impact(sim, assets, id, pos, layer);
                        base_result = false;
                    } else if material == crate::element::GameMaterial::Hole || disappear {
                        let Some(Entity::Projectile(purse)) = self.world.entities.get_mut(id)
                        else {
                            unreachable!("registered hole purse changed entity kind")
                        };
                        purse.element.active = false;
                        base_result = false;
                    } else {
                        // ComputeTrajectory already bound exact dry terminal
                        // sector/layer/obstacle membership; do not re-query.
                        self.burst_purse(sim, assets, id, pos, layer);
                    }
                }
                ImpactKind::CoinLanded { pos, layer } => {
                    let (material, dive, disappear) =
                        match self.expect_entity(id, "registered coin terminal continuation") {
                            Entity::Projectile(coin) => (
                                coin.element.material(),
                                coin.projectile.dive,
                                coin.projectile.disappear,
                            ),
                            _ => panic!("registered coin terminal changed entity kind"),
                        };
                    if material == crate::element::GameMaterial::Water || dive {
                        self.add_projectile_water_titbit(pos, layer);
                        if let Some(Entity::Projectile(coin)) = self.world.entities.get_mut(id) {
                            coin.projectile.trajectory_frame_count = 0;
                            coin.projectile.trajectory.clear();
                            coin.projectile.trajectory_runtime.clear();
                        }
                        observe_water_impact_stage("reset");
                        self.finish_projectile_water_impact(sim, assets, id, pos, layer);
                    } else if material != crate::element::GameMaterial::Hole && !disappear {
                        self.coin_landed(id, pos, layer);
                    }
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
        let should_prune = matches!(
            self.get_entity(id),
            Some(Entity::Projectile(p))
                if p.object.object_type == ObjectType::Purse
                    && p.projectile.purse.burst
                    && !p.projectile.purse.child_coins.is_empty()
        );
        if should_prune {
            let children: Vec<EntityId> = match self.get_entity(id) {
                Some(Entity::Projectile(p)) => p.projectile.purse.child_coins.clone(),
                _ => unreachable!("purse prune predicate guaranteed a projectile"),
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
            if let Some(Entity::Projectile(purse)) = self.world.entities.get_mut(id) {
                purse.projectile.purse.child_coins = alive;
            }
        }

        // Derived Purse/Coin Hourglass animation follows the base call.  The
        // branch is selected from IsFlying at virtual-entry time, so a
        // projectile that lands in the base call still receives the flying
        // skip-shadow increment once on that frame.
        let frozen = self.actors_frozen();
        if let Some(Entity::Projectile(projectile)) = self.get_entity_mut(id)
            && !frozen
        {
            super::tick::observe_projectile_derived_tail(id, object_type);
            let progression = if was_flying {
                crate::sprite::FrameProgression::SkipShadow
            } else {
                if projectile.element.sprite.last_action
                    != crate::element::Animation::ObjectBursting
                {
                    projectile.object.animation = crate::element::Animation::ObjectBursting;
                    projectile
                        .element
                        .sprite
                        .force_animation(crate::element::Animation::ObjectBursting, 0);
                }
                crate::sprite::FrameProgression::FreezeWhenTerminated
            };
            projectile
                .element
                .sprite
                .perform_virgin_increment(sim, progression);
        }

        match object_type {
            // RHElementCoin ignores its Projectile base result, and both
            // branches return true.
            ObjectType::Coin => true,
            // Grounded RHElementPurse skips Projectile::Hourglass entirely;
            // its bursting tail returns true even when already inactive.
            ObjectType::Purse if !was_flying => true,
            // The flying Purse branch returns the saved base result, but only
            // after its skip-shadow sprite tail.
            ObjectType::Purse => base_result,
            _ => unreachable!("validated purse/coin dispatcher changed type"),
        }
    }

    /// Burst a landed purse into [`NUMBER_OF_COINS_IN_PURSE`] child
    /// coins scattered around the impact point.
    fn burst_purse(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        purse_id: EntityId,
        impact_pos: WorldPoint3D,
        layer: u16,
    ) {
        // ── Resolve the shooter's MoveBox ──────────────────────────
        //
        // Used both for impact-position correction and the per-coin
        // accessibility loop. Original unconditionally dereferences
        // GetShooter here, so a missing thrower is corrupted state.
        let shooter_id = match self.expect_entity(purse_id, "purse burst owner") {
            Entity::Projectile(purse) => purse
                .projectile
                .shooter
                .unwrap_or_else(|| panic!("purse {purse_id} burst without required shooter")),
            _ => panic!("purse burst owner changed entity kind"),
        };
        let shooter_move_box = *self
            .expect_entity(shooter_id, "purse burst shooter")
            .position_iface()
            .get_move_box();

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
        let mut box_at_pos = shooter_move_box.translated(MapPoint::new(impact_pos.x, impact_pos.y));
        if self
            .world
            .fast_grid
            .find_authorized_position(&mut box_at_pos, layer)
        {
            corrected_2d = box_at_pos.center();
        }
        let source_pos = WorldPoint3D {
            x: corrected_2d.x,
            y: corrected_2d.y,
            z: impact_pos.z,
        };

        // PLING noise so nearby NPCs hear the impact.
        self.broadcast_noise_synchronously(
            sim,
            assets,
            crate::ai::NoiseType::Pling,
            crate::coordinates::MapPoint::new(source_pos.x, source_pos.y),
            crate::position_interface::Layer::new(layer),
            crate::parameters_ai::NOISE_VOLUME_PLING as u16,
            source_pos.z.max(0.0) as u16,
            Some(purse_id),
        );

        // ComputeTrajectory authored the terminal material in the waypoint
        // runtime and Projectile::Hourglass installed it before dispatching
        // HitObstacle. Do not perform a second spatial/material query here:
        // it can select a different overlapping obstacle after membership
        // has already been reconciled.
        let material = match self.expect_entity(purse_id, "purse impact material") {
            Entity::Projectile(purse) => purse.element.material(),
            _ => panic!("purse impact material owner changed entity kind"),
        };

        // ── Spawn child coins ──────────────────────────────────────
        //
        // For each coin, try up to `COIN_SCATTER_ATTEMPTS` random
        // scatter positions, accept the first one reachable from the
        // corrected source via `is_straight_movement_authorized`; if
        // none reachable, fall back to the source position itself.
        let mut spawned_children: Vec<EntityId> =
            Vec::with_capacity(NUMBER_OF_COINS_IN_PURSE as usize);
        for _ in 0..NUMBER_OF_COINS_IN_PURSE {
            let mut scatter = crate::coordinates::MapVec::ZERO;
            for _ in 0..bow_shot::COIN_SCATTER_ATTEMPTS {
                // Original: `RHElementPurse::Burst` in
                // `original-code/RHElementPurse.cpp:138-181` uses
                // `rand() & 15` for direction and `10 + (rand() & 31)`
                // for magnitude on each of seven attempts.
                let sector =
                    (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::PurseCoinScatter, ..) & 15)
                        as i16;
                let magnitude = COIN_SCATTER_MIN
                    + (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::PurseCoinScatter, ..) & 31)
                        as f32;
                let (ux, uy) = crate::element::direction_vector_16(sector);
                // Y is compressed by ASPECT_RATIO to match isometric ground.
                let scatter_x = ux * magnitude;
                let scatter_y = uy * magnitude * crate::position_interface::ASPECT_RATIO;
                let candidate = MapPoint {
                    x: impact_pos.x + scatter_x,
                    y: impact_pos.y + scatter_y,
                };
                if self.world.fast_grid.is_straight_movement_authorized(
                    MapPoint::new(corrected_2d.x, corrected_2d.y),
                    candidate,
                    layer,
                    &shooter_move_box,
                ) {
                    scatter = crate::coordinates::MapVec::new(scatter_x, scatter_y);
                    break;
                }
            }
            let goal_2d = MapPoint::new(corrected_2d.x + scatter.x, corrected_2d.y + scatter.y);
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
            let target_pos: WorldPoint3D =
                self.position_to_point_3d(assets, purse_sector, layer, goal_2d.x, goal_2d.y);

            let target_sector = purse_sector;
            let coin = {
                let obstacle_check = bow_shot::TrajectoryObstacleCheck {
                    fast_find_grid: &self.world.fast_grid,
                    layer,
                    sight_obstacles: self.sight_obstacles(assets),
                    water_zones: Some(&assets.water_zones),
                };
                bow_shot::spawn_coin(
                    Some(purse_id),
                    impact_pos,
                    target_pos,
                    layer,
                    crate::position_interface::Layer::new(layer),
                    target_sector,
                    bow_shot::APEX_COIN,
                    Some(&obstacle_check),
                )
            };
            let coin_id =
                self.publish_primed_coin(sim, assets, coin, corrected_2d, purse_sector, layer);
            spawned_children.push(coin_id);
        }

        // ── Update the purse's bookkeeping ─────────────────────────
        //
        // The invariant `number_of_coins >= NUMBER_OF_COINS_IN_PURSE`
        // holds because `spawn_purse` initialises the counter to
        // `NUMBER_OF_COINS_IN_PURSE`; a violated required state is a port/save
        // error, not a zero-coin gameplay branch. The inactive purse remains
        // published so child `source_purse` handles retain their identity.
        if let Some(Entity::Projectile(purse)) = self.world.entities.get_mut(purse_id) {
            assert!(
                purse.projectile.purse.number_of_coins >= NUMBER_OF_COINS_IN_PURSE,
                "purse {purse_id:?} should hold ≥ {NUMBER_OF_COINS_IN_PURSE} coins at burst time, \
                 found {}",
                purse.projectile.purse.number_of_coins
            );
            purse.projectile.purse.burst = true;
            purse.projectile.purse.child_coins = spawned_children;
            purse.projectile.purse.number_of_coins -= NUMBER_OF_COINS_IN_PURSE;
            // Burst does NOT mark the purse as taken — the takable
            // flag only flips when the player explicitly takes one of
            // its child coins and the pickup routes through `take_purse`.
        }

        // PlayCoinFx follows every child AddElement and the purse bookkeeping
        // in Original, but still precedes SetActive(false).
        self.feedback
            .pending_side_effects
            .sounds
            .push(super::SoundCommand::Fx {
                fx_id: coin_fx_for_material(material),
                position: MapPoint::new(impact_pos.x, impact_pos.y),
                material: None,
            });
        let Some(Entity::Projectile(purse)) = self.world.entities.get_mut(purse_id) else {
            unreachable!("bookkept purse changed entity kind before deactivation")
        };
        purse.element.active = false;

        // Projectile::Hourglass calls PlayImpactSound only after the virtual
        // purse HitObstacle returns (and therefore after deactivation).
        self.feedback
            .pending_side_effects
            .sounds
            .push(super::SoundCommand::Fx {
                fx_id: FX_PURSE_IMPACT,
                position: MapPoint::new(impact_pos.x, impact_pos.y),
                material: None,
            });

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
    fn coin_landed(&mut self, coin_id: EntityId, impact_pos: WorldPoint3D, _layer: u16) {
        if let Some(Entity::Projectile(coin)) = self.world.entities.get_mut(coin_id) {
            // Snap to the resolved goal stored at spawn.  Falls back to
            // the trajectory-end layer when the scatter-time
            // accessibility search couldn't pin a goal sector (no
            // shooter MoveBox / unreachable scatter target).
            match coin.projectile.purse.layer_goal {
                Some(layer) => coin.element.set_layer(layer.get()),
                None => coin.element.clear_layer(),
            }
            coin.element.set_sector(coin.projectile.purse.sector_goal);
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
        // without holding nested borrows on `self.world.entities`.
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
            if let Some(Entity::Projectile(c)) = self.world.entities.get_mut(cid)
                && c.element.active
                && !c.object.taken
            {
                collected = collected.saturating_add(crate::inventory::COIN_VALUE);
                c.object.taken = true;
                c.element.active = false;
            }
        }
        if let Some(Entity::Projectile(purse)) = self.world.entities.get_mut(purse_id) {
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

    fn purse_test_assets() -> crate::engine::LevelAssets {
        use crate::sprite::Sprite;
        use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};
        use std::sync::Arc;

        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[Animation::ObjectFlying as usize] = 16;
        conversion[Animation::ObjectBursting as usize] = 32;
        let script = SpriteScript {
            action_id: Animation::ObjectFlying as u16,
            action_done: 4,
            frame_ids: vec![1, 2, 3, 4, 5],
            delays: vec![0; 5],
            distances: vec![0; 5],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 5],
            sound_ids: vec![0; 5],
            ..Default::default()
        };
        let prototype = Sprite::new(Arc::new(vec![script; 33]), Arc::new(conversion));
        let mut assets = crate::engine::LevelAssets::new();
        assets
            .accessory_sprite_prototypes
            .insert(ObjectType::Purse, prototype.clone());
        assets
            .accessory_sprite_prototypes
            .insert(ObjectType::Coin, prototype);
        assets
    }

    /// Place a flying purse with an empty trajectory so it lands on the
    /// next tick — easy to test the burst → coin spawn pipeline without
    /// the throw-velocity setup.
    fn spawn_landing_purse(
        engine: &mut EngineInner,
        pos: WorldPoint3D,
        layer: u16,
        thrower: Option<EntityId>,
    ) -> EntityId {
        let thrower = thrower.or_else(|| {
            Some(engine.add_entity(Entity::Pc(crate::element::ActorPc {
                element: ElementData {
                    kind: crate::element::ElementKind::ActorPc,
                    active: true,
                    ..Default::default()
                },
                actor: Default::default(),
                human: Default::default(),
                pc: Default::default(),
            })))
        });
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

    fn landing_coin(material: crate::element::GameMaterial, dive: bool, disappear: bool) -> Entity {
        let pos = WorldPoint3D::new(100.0, 200.0, 0.0);
        let mut element = ElementData {
            kind: crate::element::ElementKind::ObjectProjectile,
            active: true,
            ..Default::default()
        };
        element.set_position(pos);
        element.set_position_map(MapPoint::from_world_xyz(pos.x, pos.y, pos.z));
        element.set_layer(0);
        element.set_material(material);
        Entity::Projectile(ElementProjectile {
            element,
            object: ObjectData {
                associated_action: crate::profiles::Action::Purse,
                object_type: ObjectType::Coin,
                animation: Animation::ObjectFlying,
                quantity: 1,
                ..Default::default()
            },
            projectile: ProjectileData {
                flying: true,
                dive,
                disappear,
                ..Default::default()
            },
        })
    }

    fn tick_purses(engine: &mut EngineInner, assets: &crate::engine::LevelAssets) {
        engine.with_simulation_context(|engine, sim| engine.tick_purses_and_coins(sim, assets));
    }

    #[test]
    fn generated_runtime_keeps_free_sentinel_and_materializes_exact_collision_point() {
        use crate::element::{GameMaterial, TrajectoryPoint, TrajectoryPointRuntime};

        let engine = EngineInner::new();
        let mut projectile = ElementProjectile {
            element: ElementData {
                kind: crate::element::ElementKind::ObjectProjectile,
                ..Default::default()
            },
            object: ObjectData {
                object_type: ObjectType::Coin,
                ..Default::default()
            },
            projectile: ProjectileData {
                trajectory: vec![
                    TrajectoryPoint {
                        position: WorldPoint3D::new(5.0, 0.0, 5.0),
                        time: 2,
                    },
                    TrajectoryPoint {
                        position: WorldPoint3D::new(10.0, 0.0, 0.0),
                        time: 1,
                    },
                ],
                trajectory_runtime: vec![
                    TrajectoryPointRuntime {
                        bounce: false,
                        material: GameMaterial::NumberOfMaterials.as_u32(),
                    };
                    2
                ],
                terminal_material_pending: true,
                terminal_material_impact_index: Some(1),
                ..Default::default()
            },
        };
        engine.materialize_terminal_projectile_runtime(
            &crate::engine::LevelAssets::new(),
            &mut projectile,
        );
        assert_eq!(
            GameMaterial::from_u32(projectile.projectile.trajectory_runtime[0].material),
            GameMaterial::NumberOfMaterials
        );
        assert!(!projectile.projectile.trajectory_runtime[0].bounce);
        assert_eq!(
            GameMaterial::from_u32(projectile.projectile.trajectory_runtime[1].material),
            GameMaterial::Ground
        );
        assert!(projectile.projectile.trajectory_runtime[1].bounce);
        assert!(!projectile.projectile.terminal_material_pending);
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

        let assets = purse_test_assets();
        tick_purses(&mut engine, &assets);

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
        // The same-call derived SkipShadow tail retains ObjectFlying;
        // ObjectBursting is selected on the next grounded Hourglass.
        assert_eq!(p.object.animation, Animation::ObjectFlying);

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
    fn inactive_grounded_purse_enters_bursting_once_then_keeps_progressing() {
        let mut engine = EngineInner::new();
        let assets = purse_test_assets();
        let purse_id =
            spawn_landing_purse(&mut engine, WorldPoint3D::new(100.0, 200.0, 0.0), 0, None);
        let prototype = assets
            .accessory_sprite_prototypes
            .get(&ObjectType::Purse)
            .expect("test purse sprite prototype")
            .clone();
        let Some(Entity::Projectile(purse)) = engine.world.entities.get_mut(purse_id) else {
            unreachable!()
        };
        purse.element.sprite = prototype;
        purse
            .element
            .sprite
            .force_animation(Animation::ObjectFlying, 0);
        crate::sim_rng::with_seed(0xB057, |sim| {
            engine.tick_purse_or_coin(sim, &assets, purse_id)
        });
        let Some(Entity::Projectile(purse)) = engine.get_entity(purse_id) else {
            panic!("burst purse disappeared after landing tick")
        };
        assert!(!purse.element.active);
        assert_eq!(purse.element.sprite.last_action, Animation::ObjectFlying);

        crate::sim_rng::with_seed(0xB057, |sim| {
            engine.tick_purse_or_coin(sim, &assets, purse_id)
        });
        let Some(Entity::Projectile(purse)) = engine.get_entity(purse_id) else {
            panic!("inactive purse disappeared on grounded Hourglass")
        };
        assert_eq!(purse.object.animation, Animation::ObjectBursting);
        assert_eq!(purse.element.sprite.last_action, Animation::ObjectBursting);
        assert_eq!(purse.element.sprite.current_row, 32);
        assert_eq!(purse.element.sprite.current_frame, 1);

        crate::sim_rng::with_seed(0xB057, |sim| {
            engine.tick_purse_or_coin(sim, &assets, purse_id)
        });
        let Some(Entity::Projectile(purse)) = engine.get_entity(purse_id) else {
            panic!("already-bursting purse disappeared")
        };
        assert_eq!(purse.element.sprite.current_row, 32);
        assert_eq!(
            purse.element.sprite.current_frame, 2,
            "already-bursting tail must not force frame zero again"
        );
    }

    #[test]
    fn registered_purse_water_and_hole_terminal_paths_never_burst() {
        for (material, dive, disappear) in [
            (crate::element::GameMaterial::Water, true, false),
            (crate::element::GameMaterial::Hole, false, true),
        ] {
            let mut engine = EngineInner::new();
            let purse_id =
                spawn_landing_purse(&mut engine, WorldPoint3D::new(100.0, 200.0, 0.0), 0, None);
            let Some(Entity::Projectile(purse)) = engine.world.entities.get_mut(purse_id) else {
                unreachable!()
            };
            purse.element.set_material(material);
            purse.projectile.dive = dive;
            purse.projectile.disappear = disappear;

            let assets = purse_test_assets();
            let result = engine.with_simulation_context(|engine, sim| {
                engine.tick_purse_or_coin(sim, &assets, purse_id)
            });
            assert!(!result, "flying purse WATER/HOLE base result must be false");
            let Some(Entity::Projectile(purse)) = engine.get_entity(purse_id) else {
                panic!("registered water/hole purse disappeared")
            };
            assert!(!purse.element.active);
            assert!(!purse.projectile.purse.burst);
            assert!(purse.projectile.purse.child_coins.is_empty());
            assert_eq!(
                purse.projectile.trajectory_frame_count,
                if dive { 0 } else { u16::MAX }
            );
            if dive {
                assert!(
                    engine
                        .feedback
                        .pending_side_effects
                        .sounds
                        .iter()
                        .any(|sound| matches!(
                            sound,
                            super::super::SoundCommand::Fx { fx_id: 470, .. }
                        ))
                );
            }
        }
    }

    #[test]
    fn coin_water_runs_base_plouf_but_hole_skips_it_and_both_remain_active() {
        for (material, dive, disappear, expect_plouf) in [
            (crate::element::GameMaterial::Water, true, false, true),
            (crate::element::GameMaterial::Hole, false, true, false),
        ] {
            let assets = purse_test_assets();

            let mut registered = EngineInner::new();
            let coin_id = registered.add_entity(landing_coin(material, dive, disappear));
            WATER_IMPACT_ORDER.with(|order| order.borrow_mut().clear());
            let result = registered.with_simulation_context(|engine, sim| {
                engine.tick_purse_or_coin(sim, &assets, coin_id)
            });
            assert!(result, "Coin ignores Projectile's terminal false result");
            let Some(Entity::Projectile(coin)) = registered.get_entity(coin_id) else {
                panic!("registered terminal coin disappeared")
            };
            assert!(coin.element.active);
            assert_eq!(
                coin.projectile.trajectory_frame_count,
                if dive { 0 } else { u16::MAX }
            );
            assert_eq!(
                registered.feedback.titbit_manager.titbits().len(),
                usize::from(expect_plouf)
            );
            assert_eq!(
                registered
                    .feedback
                    .pending_side_effects
                    .sounds
                    .iter()
                    .filter(|sound| matches!(
                        sound,
                        super::super::SoundCommand::Fx { fx_id: 470, .. }
                    ))
                    .count(),
                usize::from(expect_plouf)
            );
            WATER_IMPACT_ORDER.with(|order| {
                assert_eq!(
                    order.borrow().as_slice(),
                    if expect_plouf {
                        &["titbit", "reset", "noise", "fx"][..]
                    } else {
                        &[]
                    }
                )
            });

            let mut unpublished = EngineInner::new();
            WATER_IMPACT_ORDER.with(|order| order.borrow_mut().clear());
            let new_id = unpublished.with_simulation_context(|engine, sim| {
                engine.publish_primed_coin(
                    sim,
                    &assets,
                    landing_coin(material, dive, disappear),
                    MapPoint::new(100.0, 200.0),
                    None,
                    0,
                )
            });
            let Some(Entity::Projectile(coin)) = unpublished.get_entity(new_id) else {
                panic!("prepublication terminal coin disappeared")
            };
            assert!(coin.element.active);
            assert_eq!(
                coin.projectile.trajectory_frame_count,
                if dive { 0 } else { u16::MAX }
            );
            assert_eq!(
                unpublished.feedback.titbit_manager.titbits().len(),
                usize::from(expect_plouf)
            );
            WATER_IMPACT_ORDER.with(|order| {
                assert_eq!(
                    order.borrow().as_slice(),
                    if expect_plouf {
                        &["titbit", "reset", "noise", "fx"][..]
                    } else {
                        &[]
                    }
                )
            });
        }

        let assets = purse_test_assets();
        let mut engine = EngineInner::new();
        let mut dry = landing_coin(crate::element::GameMaterial::Ground, false, false);
        let Entity::Projectile(coin) = &mut dry else {
            unreachable!()
        };
        coin.element
            .set_sector(crate::position_interface::SectorHandle::new(7));
        coin.projectile.purse.sector_goal = None;
        let coin_id = engine.add_entity(dry);
        engine.with_simulation_context(|engine, sim| {
            engine.tick_purse_or_coin(sim, &assets, coin_id)
        });
        let Some(Entity::Projectile(coin)) = engine.get_entity(coin_id) else {
            panic!("ordinary landed coin disappeared")
        };
        assert_eq!(
            coin.element.sector(),
            None,
            "None goal must clear terminal sector"
        );
    }

    #[test]
    fn live_purse_pass_reaches_newly_appended_coins_in_the_same_frame() {
        let mut parent_only = EngineInner::new();
        let purse_id = spawn_landing_purse(
            &mut parent_only,
            WorldPoint3D {
                x: 100.0,
                y: 200.0,
                z: 0.0,
            },
            0,
            None,
        );
        let mut live_pass = parent_only.clone();
        let assets = purse_test_assets();

        crate::sim_rng::with_seed(0xC01A, |sim| {
            parent_only.tick_purse_or_coin(sim, &assets, purse_id)
        });
        crate::sim_rng::with_seed(0xC01A, |sim| live_pass.tick_purses_and_coins(sim, &assets));

        let child_ids = match parent_only.get_entity(purse_id) {
            Some(Entity::Projectile(purse)) => purse.projectile.purse.child_coins.clone(),
            _ => panic!("parent-only purse missing after burst"),
        };
        assert_eq!(child_ids.len(), NUMBER_OF_COINS_IN_PURSE as usize);
        for child_id in child_ids {
            let primed = match parent_only.get_entity(child_id) {
                Some(Entity::Projectile(coin)) => (
                    coin.projectile.frame_count,
                    coin.object.animation,
                    coin.element.sprite.current_frame,
                ),
                _ => panic!("primed coin missing"),
            };
            let same_frame = match live_pass.get_entity(child_id) {
                Some(Entity::Projectile(coin)) => (
                    coin.projectile.frame_count,
                    coin.object.animation,
                    coin.element.sprite.current_frame,
                ),
                _ => panic!("same-frame coin missing"),
            };
            assert_ne!(
                same_frame, primed,
                "coin {child_id:?} must receive its element-array Hourglass after the explicit pre-insertion primer"
            );
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
        let assets = purse_test_assets();
        tick_purses(&mut engine, &assets);
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
        // itself stays active.  Explicitly taking a child coin routes
        // through `take_purse` and clears the remaining children.
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
        let assets = purse_test_assets();
        tick_purses(&mut engine, &assets);
        let coin_ids: Vec<EntityId> = match engine.get_entity(purse_id) {
            Some(Entity::Projectile(p)) => p.projectile.purse.child_coins.clone(),
            _ => panic!("purse missing"),
        };
        assert_eq!(coin_ids.len(), NUMBER_OF_COINS_IN_PURSE as usize);

        // Deactivate every child coin (simulating pickup-and-removal).
        for &cid in &coin_ids {
            if let Some(Entity::Projectile(c)) = engine.world.entities.get_mut(cid) {
                c.element.active = false;
            }
        }

        // Run a tick — Hourglass drain should prune the child list…
        let assets = purse_test_assets();
        tick_purses(&mut engine, &assets);
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
        // …and the source-exact inactive purse remains published so coin
        // back-references retain a stable identity.
        assert!(
            !p.element.active,
            "HitObstacle must leave the retained purse inactive post-drain"
        );
        assert!(
            p.projectile.purse.burst,
            "purse should still be flagged as burst"
        );
    }

    #[test]
    fn purse_rng_replay_from_clone_is_deterministic() {
        let mut live = EngineInner::new();
        live.restore_rng_from_seed(0xA11C_E5E1_1234_5678);
        spawn_landing_purse(
            &mut live,
            WorldPoint3D {
                x: 125.0,
                y: 275.0,
                z: 0.0,
            },
            0,
            None,
        );
        let initial_seed = live.rng_seed();
        let mut replay = live.clone();
        let assets = purse_test_assets();

        tick_purses(&mut live, &assets);
        tick_purses(&mut replay, &assets);

        assert_ne!(live.rng_seed(), initial_seed, "purse burst must draw RNG");
        assert_eq!(live.rng_seed(), replay.rng_seed());
        assert_eq!(
            crate::replay::state_hash(&live),
            crate::replay::state_hash(&replay),
            "rollback replay must reproduce coin scatter and RNG state"
        );
    }

    #[test]
    fn purse_rng_save_restore_is_deterministic() {
        let mut continuous = EngineInner::new();
        continuous.restore_rng_from_seed(0x5A7E_CAFE_89AB_CDEF);
        spawn_landing_purse(
            &mut continuous,
            WorldPoint3D {
                x: 75.0,
                y: 150.0,
                z: 0.0,
            },
            0,
            None,
        );
        let json = serde_json::to_string(&continuous).expect("serialize pre-burst engine");
        let mut restored: EngineInner =
            serde_json::from_str(&json).expect("deserialize pre-burst engine");
        let assets = purse_test_assets();

        tick_purses(&mut continuous, &assets);
        tick_purses(&mut restored, &assets);

        assert_eq!(continuous.rng_seed(), restored.rng_seed());
        assert_eq!(
            crate::replay::state_hash(&continuous),
            crate::replay::state_hash(&restored),
            "save restore must reproduce coin scatter and RNG state"
        );
    }
}
