//! Selected ability execution and immediate cross-entity effects.

#[cfg(test)]
use super::archery::{RECEIVE_PURSE_REVEALS, ReceivePurseRevealObservation};
use super::*;
use crate::abilities::{SelectedAbility, ability_order_type, selected_ability};
use crate::coordinates::MapPoint;
use crate::element::{ActionState, Entity, EntityId, Posture};
use crate::movement::AbilityKind;
use crate::order::OrderType;
use crate::sprite::MotionState as SpriteMotionState;

impl EngineInner {
    /// Drop the corpse before advancing the order, including transition prefixes
    /// whose selected command belongs to the following ability.
    pub(super) fn apply_completed_corpse_drop(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        carrier_id: EntityId,
        target_id: EntityId,
        drop_posture: crate::element::Posture,
        carrier_pos: crate::coordinates::MapPoint,
        carrier_direction: u16,
    ) {
        let (carrier_sector, carrier_layer, carrier_obstacle, carrier_plane, drop_box_origin) =
            self.get_entity(carrier_id)
                .map(|e| {
                    (
                        e.element_data().sector(),
                        e.element_data().layer(),
                        e.position_iface().get_obstacle(),
                        e.position_iface().get_plane().copied(),
                        e.current_gameplay_point_map().unwrap_or_else(|| {
                            panic!("corpse-drop carrier {carrier_id:?} has no current action point")
                        }),
                    )
                })
                .unwrap_or_else(|| panic!("corpse-drop carrier {carrier_id:?} disappeared"));
        let in_building = carrier_sector
            .and_then(|s| {
                self.grid_sector_by_number(crate::sector::SectorNumber::new(i16::from(s)))
            })
            .map(|gs| gs.sector_type.is_building())
            .unwrap_or(false);

        let drop_pos = if in_building {
            carrier_pos
        } else {
            let target_box = self
                .get_entity(target_id)
                .map(|e| e.position_iface())
                .map(|pi| *pi.get_move_box())
                .filter(|b| b.is_somewhere());
            match target_box {
                Some(b) => {
                    // Original translates the corpse box by
                    // the live map-space animation hotspot, then
                    // searches toward the carrier's map origin.
                    let mut bbox = b.translated(drop_box_origin);
                    if self.world.fast_grid.find_authorized_position_toward(
                        &mut bbox,
                        carrier_pos,
                        carrier_layer,
                    ) {
                        bbox.center()
                    } else {
                        carrier_pos
                    }
                }
                None => carrier_pos,
            }
        };

        let preserved_outdoor_position = (!in_building).then(|| {
            self.get_entity(target_id)
                .unwrap_or_else(|| panic!("corpse-drop target {target_id:?} disappeared"))
                .element_data()
                .position()
        });

        if self.get_entity(target_id).is_some() {
            let target = self
                .get_entity_mut(target_id)
                .expect("corpse-drop target disappeared");
            // Dropping a corpse transfers the carrier's obstacle,
            // plane, layer, and sector before either the instant or delayed
            // position write. It deliberately does not replace the corpse's
            // material. In particular, an outdoor delayed drop must retain
            // the carrier's plane so next-frame delayed map positioning computes
            // elevation on that plane rather than falling back to z=0.
            let elem = target.element_data_mut();
            elem.set_obstacle_index(carrier_obstacle, carrier_plane);
            elem.set_layer(carrier_layer);
            elem.set_sector(carrier_sector);
            if in_building {
                elem.set_position_map(drop_pos);
                elem.sprite.compute_display_depth();
            } else {
                elem.set_position_map_delayed(drop_pos);
                if let Some(position) = preserved_outdoor_position {
                    // The original game's removal of the obstacle invalidates the cached 3D
                    // position without overwriting it. Delayed map positioning
                    // then queues next-frame work, and computed-position publication
                    // keeps the old elevated coordinate visible for this
                    // frame. Rust's eager obstacle setter has already
                    // projected the current map onto z=0, so restore only the
                    // cached 3D value/validity here; the delayed write will
                    // authoritatively recompute it on the next update.
                    elem.sprite
                        .position_iface
                        .restore_cached_position_all_computed(position);
                }
            }
            elem.set_direction_instantly(((carrier_direction.wrapping_add(12)) & 15) as i16);
            self.set_entity_posture(target_id, drop_posture);
            let target = self
                .get_entity_mut(target_id)
                .expect("corpse-drop target disappeared");
            if let Some(actor) = target.actor_data_mut() {
                actor.execution_frozen = false;
                actor.action_state = crate::element::ActionState::Waiting;
            }
        }
        self.set_entity_posture(carrier_id, crate::element::Posture::Upright);
        if let Some(actor) = self
            .get_entity_mut(carrier_id)
            .and_then(|entity| entity.actor_data_mut())
        {
            actor.action_state = crate::element::ActionState::Waiting;
        }
        self.actor_wait(sim, assets, target_id);

        if in_building && let Some(target) = self.get_entity_mut(target_id) {
            let is_dead = target.is_dead();
            let is_unconscious = target.human_data().is_some_and(|h| h.unconscious);
            if is_dead || is_unconscious {
                crate::engine::door_pass::start_hulk_on(target, 1.0);
                let elem = target.element_data_mut();
                elem.hidden_in_building = false;
                elem.active = true;
            }
        }
        let target = self
            .get_entity_mut(target_id)
            .expect("corpse-drop target disappeared before unlink");
        target
            .element_data_mut()
            .set_direction_goal(carrier_direction as i16);
        target
            .human_data_mut()
            .expect("corpse-drop target is not human")
            .carrier = None;
        self.get_entity_mut(carrier_id)
            .expect("corpse-drop carrier disappeared before unlink")
            .pc_data_mut()
            .expect("corpse-drop carrier is not a PC")
            .carried = None;
        tracing::debug!(
            carrier = ?carrier_id,
            target = ?target_id,
            "Drop: put down body"
        );
    }

    /// Apply the gameplay half of player execution's Listen-exit action message.
    /// The caller has already installed the explicit Wait successor.
    pub(super) fn apply_listen_done_action_handoff(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
    ) {
        self.get_entity(actor_id)
            .unwrap_or_else(|| panic!("ListenDone owner {actor_id:?} disappeared"))
            .pc_data()
            .unwrap_or_else(|| panic!("ListenDone owner {actor_id:?} is not a PC"));
        if self.players.seats[0].selection.contains(&actor_id) {
            // The messenger drops MSG_UNSELECT_ACTION unless its value is the
            // currently selected action. A newer action can be selected while
            // LeaveListen is postponed behind the entry transition; its late
            // Listen completion must not clear that newer action.
            if self.players.seats[0].selected_action == crate::profiles::Action::Listen {
                self.players.seats[0].selected_action = crate::profiles::Action::NoAction;
                self.unselect_action(sim, assets, actor_id);
            }
        } else if let Some(pc) = self
            .get_entity_mut(actor_id)
            .and_then(crate::element::Entity::pc_data_mut)
        {
            pc.current_action = crate::profiles::Action::NoAction;
        }
    }

    // ─── Shouldered-carry ceiling check ─────────────────────────────

    /// Check PCs whose movement action executed this frame for the original
    /// ceiling collision while walking with someone on the actor's shoulders.
    ///
    /// The original game checks shoulder-carrying only from that action's
    /// player-character execution arm, after motion. In particular,
    /// the persistent carrying-on-shoulders waiting posture does not
    /// run this check.
    /// Abort the walking arm as soon as its rider no longer fits overhead.
    pub(super) fn check_walking_shoulder_clearance(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        carrier_id: EntityId,
    ) -> bool {
        let carrier = self.expect_entity(carrier_id, "walking shoulder carrier");
        let position = carrier.element_data().position();
        if crate::abilities::can_carry_on_shoulders(position, self.sight_obstacles(assets)) {
            return true;
        }
        let victim = carrier
            .pc_data()
            .and_then(|pc| pc.carried)
            .expect("walking shoulder carrier has no rider");
        let damage = crate::sequence::SequenceElement::new_damage(
            1,
            crate::element::Command::ReceiveDamage,
            Some(victim),
            Some(victim),
            0,
            0,
        );
        self.launch_element(sim, assets, damage);
        false
    }
    pub(super) fn apply_ability_carry_done(&mut self, carrier_id: EntityId, target_id: EntityId) {
        self.set_entity_posture(carrier_id, Posture::CarryingCorpse);
        let carrier = self
            .get_entity_mut(carrier_id)
            .expect("Carry owner disappeared at completion");
        carrier
            .actor_data_mut()
            .expect("Carry owner lost actor state")
            .action_state = ActionState::Waiting;
        if self.get_entity(target_id).is_some() {
            self.set_entity_posture(target_id, crate::element::Posture::Carried);
            let target = self
                .get_entity_mut(target_id)
                .expect("carried target disappeared");
            if let Some(actor) = target.actor_data_mut() {
                actor.action_state = crate::element::ActionState::Waiting;
            }
        }
        tracing::debug!(
            carrier = ?carrier_id,
            target = ?target_id,
            "Carry: picked up body"
        );
        self.record_achievement_contribution(carrier_id);
    }

    pub(super) fn apply_ability_tie_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
        target_id: EntityId,
    ) {
        self.set_entity_posture(target_id, crate::element::Posture::Tied);
        if self
            .get_entity(target_id)
            .expect("tie target disappeared")
            .is_soldier()
        {
            self.execute_ai_speech(
                sim,
                assets,
                target_id,
                crate::ai::AiSpeechAttempt {
                    remark: crate::ai::Remark::TiedUp,
                    flags: 0,
                },
            );
        }
        // Player-character ability execution refreshes the victim
        // with Wait after applying the tied posture and remark.
        self.actor_wait(sim, assets, target_id);
        tracing::debug!(
            actor = ?actor_id,
            target = ?target_id,
            "Tie: enemy tied up"
        );
        self.record_achievement_tactical_effect(actor_id, target_id);
    }

    pub(super) fn apply_ability_untie_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
        target_id: EntityId,
    ) {
        let target = self
            .get_entity_mut(target_id)
            .unwrap_or_else(|| panic!("untie target {target_id:?} vanished at Done"));
        assert!(target.is_active(), "untie target became inactive at Done");
        assert!(target.is_npc(), "untie target stopped being an NPC at Done");
        assert!(!target.is_dead(), "untie target died before Done");
        assert_eq!(
            target.posture(),
            Posture::Tied,
            "cannot untie an untied entity"
        );
        self.set_entity_posture(target_id, Posture::Lying);
        // Preserve unconsciousness and concussion. The regular
        // human recovery tick remains the sole wake-up authority.
        self.actor_wait(sim, assets, target_id);
        tracing::debug!(
            actor = ?actor_id,
            target = ?target_id,
            "Untie: NPC released"
        );
    }

    pub(super) fn apply_ability_climb_on_shoulders_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        climber_id: EntityId,
        helper_id: EntityId,
    ) {
        // Postures were latched on init by
        // `begin_climb_on_shoulders`.  Terminate the
        // climber's sequence element so the post-seek
        // sequence advances and park the helper on a
        // low-priority Wait so its frozen-execution can
        // re-enter the idle loop while still
        // `CarryingOnShoulders`.
        self.actor_wait(sim, assets, helper_id);
        tracing::debug!(
            climber = ?climber_id,
            helper = ?helper_id,
            "ClimbOnShoulders: PC mounted helper's shoulders"
        );
    }

    pub(super) fn apply_ability_climb_down_from_shoulders_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        climber_id: EntityId,
        helper_id: EntityId,
    ) {
        // On the climbing-down completion: reset paired
        // postures, sever the carrier ↔ carried link, copy
        // the carrier's plane/sector/material onto the
        // climber (so the dismount happens on the helper's
        // surface) and snap the climber to an authorised
        // landing slot adjacent to the helper.
        let helper_snapshot = self.get_entity(helper_id).map(|e| {
            (
                e.element_data().position_map(),
                e.element_data().layer(),
                e.element_data().sector(),
                e.element_data().material(),
                e.element_data().obstacle_index(),
                e.position_iface().get_plane().copied(),
                e.element_data().direction(),
            )
        });

        if let Some((
            helper_pos,
            helper_layer,
            helper_sector,
            helper_material,
            helper_obstacle,
            helper_plane,
            helper_dir,
        )) = helper_snapshot
        {
            // Resolve a landing slot using the climber's
            // upright move-box translated to the helper's
            // position.  We use the climber's current
            // move-box rather than re-deriving the upright
            // variant — the upright move-box was set when
            // the PC was last upright and isn't overwritten
            // while OnShoulders.
            let landing_pos = {
                let climber_box = self
                    .get_entity(climber_id)
                    .map(|e| e.position_iface())
                    .map(|pi| *pi.get_move_box())
                    .filter(|b| b.is_somewhere());
                match climber_box {
                    Some(b) => {
                        let mut bbox = b.translated(helper_pos);
                        if self
                            .world
                            .fast_grid
                            .find_authorized_position(&mut bbox, helper_layer)
                        {
                            bbox.center()
                        } else {
                            helper_pos
                        }
                    }
                    None => helper_pos,
                }
            };

            if self.get_entity(climber_id).is_some() {
                self.set_entity_posture(climber_id, crate::element::Posture::Upright);
                let climber = self
                    .get_entity_mut(climber_id)
                    .expect("climber disappeared");
                // Sever climber → carrier back-reference.
                if let Some(human) = climber.human_data_mut() {
                    human.carrier = None;
                }
                if let Some(actor) = climber.actor_data_mut() {
                    actor.execution_frozen = false;
                    actor.action_state = crate::element::ActionState::Waiting;
                }
                // Copy plane/sector/material/obstacle from
                // helper so the climber's reprojection lands
                // on the helper's surface.
                {
                    let elem = climber.element_data_mut();
                    elem.set_layer(helper_layer);
                    elem.set_sector(helper_sector);
                    elem.set_material(helper_material);
                }
                {
                    let pi = climber.position_iface_mut();
                    pi.set_obstacle(helper_obstacle, helper_plane);
                    pi.set_material(helper_material);
                }
                // Preserve the climber's facing through the
                // copy.  The helper's direction was set to
                // the opposite of the climber's at climb
                // start, so adding 8 (180°) recovers the
                // climber's original facing.
                let preserved_dir = (helper_dir + 8) & 15;
                climber
                    .element_data_mut()
                    .set_direction_instantly(preserved_dir);
                // Snap to landing slot.
                climber.element_data_mut().set_position_map(landing_pos);
            }
        }

        // Reset the helper to HelpingToClimb / Waiting and
        // sever the carrier-side link.
        if self.get_entity(helper_id).is_some() {
            self.set_entity_posture(helper_id, crate::element::Posture::HelpingToClimb);
            let helper = self
                .get_entity_mut(helper_id)
                .expect("climbing helper disappeared");
            if let Some(actor) = helper.actor_data_mut() {
                actor.execution_frozen = false;
                actor.action_state = crate::element::ActionState::Waiting;
            }
            if let Some(pc) = helper.pc_data_mut() {
                pc.carried = None;
                pc.set_live_carried_posture(crate::element::Posture::Lying);
            }
        }

        // Park the helper on a low-priority idle so it
        // doesn't immediately re-acquire its previous
        // element.
        self.actor_wait(sim, assets, helper_id);

        tracing::debug!(
            climber = ?climber_id,
            helper = ?helper_id,
            "ClimbDownFromShoulders: PC dismounted"
        );
    }

    pub(super) fn apply_ability_heal_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        healer_id: EntityId,
        target_id: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> SpriteMotionState {
        // Player-character execution checks Heal validity again in
        // the completed-motion arm, immediately before healing (or
        // FX activation) and consuming a plant. The target may
        // have moved out of the strict 40-unit action range while
        // the Healing animation played.
        let heal_still_valid = {
            let element = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .unwrap_or_else(|| {
                    panic!("Heal DONE owner {healer_id:?} lost element {seq_id:?}/{elem_idx}")
                });
            self.check_sequence_element_validity(assets, healer_id, element, true)
        };
        if !heal_still_valid {
            return SpriteMotionState::Terminated;
        }

        // Heal effect depends on the antagonist's type.
        let target_is_fx_target = self
            .get_entity(target_id)
            .is_some_and(|e| e.kind().is_fx_target());
        if target_is_fx_target {
            // FX target — launch `Command::ActivateHeal` so
            // the target's bound script's `ActivatedByHeal`
            // hook fires.
            let mut activation = crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::ActivateHeal,
                Some(target_id),
            );
            activation.data = crate::sequence::SequenceElementData::Interaction {
                antagonist: Some(healer_id),
            };
            self.launch_element(sim, assets, activation);
        } else if let Some(target) = self.get_entity_mut(target_id) {
            // Heal the target PC via the shared helper that
            // applies the heal + life-point clamp guards.
            if let Some(pc) = target.pc_data_mut() {
                crate::pc_status::heal(
                    &mut pc.life_points,
                    crate::abilities::HEAL_AMOUNT,
                    false, // invulnerable cheat unimplemented
                );
            }
            // Clear concussion.
            if let Some(human) = target.human_data_mut() {
                human.concussion_of_the_brain = 0;
            }
            // "Sexual healing" speech cue on the healed PC.
            self.hero_speaking(assets, target_id, crate::engine::melee::HERO_HEALED);
        }
        // Decrease healer's bandage ammo.
        self.decrement_ability_ammo(assets, healer_id, crate::profiles::Action::Heal);
        tracing::debug!(
            healer = ?healer_id,
            target = ?target_id,
            "Heal: restored HP"
        );
        self.record_achievement_contribution(healer_id);
        SpriteMotionState::Done
    }

    pub(super) fn apply_ability_eat_done(&mut self, assets: &LevelAssets, actor_id: EntityId) {
        // Re-check sequence-element validity by verifying
        // the actor still has Eat ammo — if it dropped to 0
        // mid-animation, skip the heal.
        //
        // Eat and Guzzle share the `num_rations` counter, so
        // the Guzzle branch only changes the heal amount
        // (80 vs 40); both end up decrementing the same
        // underlying field.
        let pc_status = self.get_entity(actor_id).and_then(|e| match e {
            Entity::Pc(pc) => Some((pc.pc.profile_index, self.pc_description_for_pc_data(&pc.pc))),
            _ => None,
        });
        if let Some((profile_idx, Some(pc_desc))) = pc_status {
            let still_has_ammo = pc_desc.status.get_ammo(crate::profiles::Action::Eat) > 0;
            if still_has_ammo {
                // Determine heal amount based on whether the
                // PC has the Guzzle action (gluttons heal
                // more).
                let has_guzzle = assets
                    .profile_manager
                    .get_character(profile_idx)
                    .map(|p| p.has_action(crate::profiles::Action::Guzzle))
                    .unwrap_or(false);
                let heal_amount: i16 = if has_guzzle { 80 } else { 40 };
                // Eating updates the ammunition amount here rather than
                // ammunition decrement. That distinction suppresses
                // HERO_OUT_OF_AMMO for the last ration. Gluttons
                // also address their Guzzle action slot even
                // though Eat and Guzzle share one counter.
                let ration_action = if has_guzzle {
                    crate::profiles::Action::Guzzle
                } else {
                    crate::profiles::Action::Eat
                };
                self.consume_ration_without_speech(assets, actor_id, ration_action);
                // Apply heal capped at LIFEPOINTS_PC.
                if let Some(target) = self.get_entity_mut(actor_id)
                    && let Some(pc) = target.pc_data_mut()
                {
                    crate::pc_status::heal(&mut pc.life_points, heal_amount, false);
                }
                tracing::debug!(
                    actor = ?actor_id,
                    heal_amount,
                    has_guzzle,
                    "Eat: ration consumed and HP restored"
                );
            }
        }
    }

    pub(super) fn apply_ability_whistle_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
        position: MapPoint,
    ) {
        // Emit a PFIIIT noise at the whistle position with
        // radius NOISE_VOLUME_PFIIIT (400).
        let (layer, elevation) = self
            .get_entity(actor_id)
            .map(|e| {
                (
                    e.element_data().layer(),
                    e.element_data().position().z.max(0.0) as u16,
                )
            })
            .unwrap_or_else(|| panic!("whistle noise owner {actor_id:?} disappeared"));
        self.broadcast_noise_synchronously(
            sim,
            assets,
            crate::ai::NoiseType::Pfiiit,
            crate::coordinates::MapPoint::new(position.x, position.y),
            crate::position_interface::Layer::new(layer),
            crate::abilities::NOISE_VOLUME_WHISTLE,
            elevation,
            Some(actor_id),
        );
        tracing::debug!(
            actor = ?actor_id,
            x = position.x,
            y = position.y,
            "Whistle: noise emitted to attract NPCs"
        );
    }

    pub(super) fn apply_ability_listen_entered(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
    ) {
        // Entry transition animation just finished; the
        // PC is now executing the Listening order. Forward
        // PcMessage::SelectAction(Listen) so HUD/UI
        // reflects the active listen.
        // The message's gameplay half runs inline, the same way
        // the beggar entry handoff applies it: for a selected PC
        // the action reselection stops the group at Normal
        // priority even though Listen is already the current
        // action, which discards anything the entry transition
        // postponed behind itself (a move instructed while the
        // PC was listening never resumes).  An unselected PC only
        // stores the action.
        self.set_pc_action_from_message(sim, assets, 0, actor_id, crate::profiles::Action::Listen);
        tracing::debug!(
            actor = ?actor_id,
            "Listen: entry transition done → CountingDown, MSG_SELECT_ACTION sent"
        );
    }

    pub(super) fn apply_ability_listen_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
    ) {
        // Player-character execution launches Wait synchronously
        // on the DONE edge of
        // TRANSITION_LISTENING_WAITING_UPRIGHT.  This is an
        // explicit priority-Wait launch, not the null-order
        // fallback installed at the start of the next actor
        // update: it must already be available when the exit
        // transition terminates and sends its consolation card.
        self.actor_wait(sim, assets, actor_id);
        // Listen branches immediately after waiting: an
        // unselected PC only stores NOACTION, while a selected PC
        // synchronously forwards MSG_UNSELECT_ACTION(Listen).
        // Apply that message's gameplay half inline, before a
        // later input-boundary SelectPC can restitute the stale
        // Listen action and Stop() the just-postponed Wait.
        self.apply_listen_done_action_handoff(sim, assets, actor_id);
        tracing::debug!(
            actor = ?actor_id,
            "Listen: exit transition done → Inactive, MSG_UNSELECT_ACTION sent"
        );
    }

    pub(super) fn apply_ability_throw_net_done(
        &mut self,
        assets: &LevelAssets,
        actor_id: EntityId,
        target_pos: MapPoint,
    ) {
        // Spawn a net projectile entity with ballistic
        // trajectory.  Launch origin is the thrower's hand
        // point, not their feet.
        let (throw_pos, layer) = self.projectile_throw_origin(actor_id, "ThrowNetDone");
        let target_3d = crate::coordinates::WorldPoint3D {
            x: target_pos.x,
            y: target_pos.y,
            z: 0.0,
        };
        let obstacle_check = crate::bow_shot::TrajectoryObstacleCheck {
            fast_find_grid: &self.world.fast_grid,
            sight_obstacles: self.sight_obstacles(assets),
            water_zones: Some(&assets.environment.water_zones),
        };
        let net_entity = crate::bow_shot::spawn_net(
            actor_id,
            throw_pos,
            target_3d,
            layer,
            Some(&obstacle_check),
        );
        let net_id = self.add_entity(net_entity);
        self.attach_accessory_sprite(assets, net_id);
        // Run the landing-site crumple test at spawn time.
        // We keep the ballistic trajectory inside
        // `spawn_net` and run the crumple check here, where
        // we have engine access to obstacles +
        // fast_find_grid.
        self.detect_initial_net_crumple(assets, net_id);
        tracing::debug!(
            actor = ?actor_id,
            x = target_pos.x,
            y = target_pos.y,
            "ThrowNet: spawned net projectile"
        );
        self.decrement_ability_ammo(assets, actor_id, crate::profiles::Action::Net);
    }

    pub(super) fn apply_ability_throw_purse_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
        target_pos: MapPoint,
    ) {
        // Spawn the purse projectile.  The trajectory is
        // computed against the current sight obstacles so
        // the purse arcs over walls / falls onto roofs the
        // same way other ground-targeted throwables do.
        // Launch origin is the thrower's hand point.
        let (throw_pos, layer) = self.projectile_throw_origin(actor_id, "ThrowPurseDone");
        let target_3d = crate::coordinates::WorldPoint3D {
            x: target_pos.x,
            y: target_pos.y,
            z: 0.0,
        };
        let obstacle_check = crate::bow_shot::TrajectoryObstacleCheck {
            fast_find_grid: &self.world.fast_grid,
            sight_obstacles: self.sight_obstacles(assets),
            water_zones: Some(&assets.environment.water_zones),
        };
        let purse_entity = crate::bow_shot::spawn_purse(
            actor_id,
            throw_pos,
            target_3d,
            layer,
            Some(&obstacle_check),
        );
        self.publish_new_purse(sim, assets, actor_id, purse_entity);
        tracing::debug!(
            actor = ?actor_id,
            x = target_pos.x,
            y = target_pos.y,
            "ThrowPurse: spawned purse projectile"
        );
        self.decrement_ability_ammo(assets, actor_id, crate::profiles::Action::Purse);
        // Deduct the thrown purse's face value from the
        // campaign ransom pool on throw.  Coin pickup later
        // credits `COIN_VALUE` per recovered coin, so
        // conservation holds: uncollected coins are a real
        // loss and fully-recovered purses wash out.
        let face_value =
            crate::inventory::COINS_PER_PURSE as i32 * crate::inventory::COIN_VALUE as i32;
        self.add_campaign_value(assets, crate::campaign::CampaignValue::Ransom, -face_value);
    }

    pub(super) fn apply_ability_throw_wasp_nest_done(
        &mut self,
        assets: &LevelAssets,
        actor_id: EntityId,
        target_pos: MapPoint,
    ) {
        // Spawn a wasp nest projectile entity with ballistic
        // trajectory.  Launch origin is the thrower's hand
        // point.
        let (throw_pos, layer) = self.projectile_throw_origin(actor_id, "ThrowWaspNestDone");
        let target_3d = crate::coordinates::WorldPoint3D {
            x: target_pos.x,
            y: target_pos.y,
            z: 0.0,
        };
        let obstacle_check = crate::bow_shot::TrajectoryObstacleCheck {
            fast_find_grid: &self.world.fast_grid,
            sight_obstacles: self.sight_obstacles(assets),
            water_zones: Some(&assets.environment.water_zones),
        };
        let wasp_entity = crate::bow_shot::spawn_wasp_nest(
            actor_id,
            throw_pos,
            target_3d,
            layer,
            Some(&obstacle_check),
        );
        let wasp_id = self.add_entity(wasp_entity);
        self.mission_domain
            .achievements
            .record_wasp_nest_throw(wasp_id);
        self.attach_accessory_sprite(assets, wasp_id);
        tracing::debug!(
            actor = ?actor_id,
            x = target_pos.x,
            y = target_pos.y,
            "ThrowWaspNest: spawned wasp nest projectile"
        );
        self.decrement_ability_ammo(assets, actor_id, crate::profiles::Action::WaspNest);
    }

    pub(super) fn apply_ability_pay_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        pc_id: EntityId,
        beggar_id: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> SpriteMotionState {
        // Paying validates again after action processing reports completion.
        // Ransom or distance may have changed while the PC was
        // turning/animating; invalid payment aborts before launching the
        // antagonist response or deducting any money in that case.
        let valid = {
            let element = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .unwrap_or_else(|| {
                    panic!("completed Pay owner {pc_id:?} lost element {seq_id:?}/{elem_idx}")
                });
            assert_eq!(element.owner, Some(pc_id));
            assert_eq!(element.command, crate::element::Command::Pay);
            let order = element.current_order().unwrap_or_else(|| {
                panic!("completed Pay element {seq_id:?}/{elem_idx} lost its selected order")
            });
            assert_eq!(order.order_type, crate::order::OrderType::Paying);
            assert_eq!(order.target_actor, Some(beggar_id.index()));
            self.check_sequence_element_validity(assets, pc_id, element, true)
        };
        if !valid {
            return SpriteMotionState::Aborted;
        }

        // On Paying-animation completion: deduct
        // BEGGAR_SALARY from the ransom, and either launch
        // `Command::ActivateMoney` on an FX-target antagonist
        // or a `Command::ReceivePurse` sequence element on a
        // beggar NPC.
        let antagonist_is_fx_target = self
            .get_entity(beggar_id)
            .is_some_and(|e| e.kind().is_fx_target());
        if antagonist_is_fx_target {
            // FX target — fire the script's ActivatedByMoney
            // hook via the central `Command::Activate*`
            // dispatch.
            let mut activation = crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::ActivateMoney,
                Some(beggar_id),
            );
            activation.data = crate::sequence::SequenceElementData::Interaction {
                antagonist: Some(pc_id),
            };
            self.launch_element(sim, assets, activation);
        } else {
            let mut receive = crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::ReceivePurse,
                Some(beggar_id),
            );
            receive.priority = crate::sequence::SequencePriority::Normal;
            self.launch_element(sim, assets, receive);
        }
        self.add_campaign_value(
            assets,
            crate::campaign::CampaignValue::Ransom,
            -crate::engine::BEGGAR_SALARY,
        );
        if !antagonist_is_fx_target {
            let exhausted = !self.are_there_revealable_scrolls(assets, beggar_id);
            self.mission_domain
                .achievements
                .record_beggar_payment(beggar_id, exhausted)
                .expect("beggar payment after finalization");
        }
        tracing::debug!(
            pc = ?pc_id,
            beggar = ?beggar_id,
            "Pay: salary deducted, ACTIVATE_MONEY / RECEIVE_PURSE launched"
        );
        SpriteMotionState::Done
    }

    pub(super) fn apply_ability_receive_purse_revealing(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        beggar_id: EntityId,
    ) {
        // Middle of the receive-purse chain — the beggar is
        // waving the purse.  `reveal_scrolls` runs on
        // WaitingWithPurse termination, driving the
        // delayed-highlight display flow.  The beggar's
        // CIV_REMARK_BEGGAR_* speech cue is queued inside
        // `reveal_scrolls` and later dispatched by
        // the owner-local speech drain.
        match self.reveal_scrolls(sim, assets, beggar_id) {
            Some(remark) => tracing::debug!(
                beggar = ?beggar_id,
                ?remark,
                "ReceivePurse: reveal_scrolls fired",
            ),
            None => tracing::debug!(
                beggar = ?beggar_id,
                "ReceivePurse: reveal_scrolls returned None \
                 (non-beggar?), ignoring"
            ),
        }
        #[cfg(test)]
        RECEIVE_PURSE_REVEALS.with(|reveals| {
            reveals.record_with(|| ReceivePurseRevealObservation {
                owner: beggar_id,
                current_order: self
                    .orders
                    .sequence_manager
                    .current_order_for_actor(&self.world.entities, beggar_id)
                    .map(|(_, _, order)| (order.order_id, order.order_type)),
            });
        });
    }

    pub(super) fn apply_ability_hit_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
        target_id: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        // Human hitting execution rechecks the live
        // interaction when motion completes, before applying damage.
        // Losing validity here merely makes the swing miss: the
        // Hitting order still completes through its normal Done
        // lifecycle and is not made Impossible.
        let valid = {
            let element = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .unwrap_or_else(|| {
                    panic!(
                        "completed Hit owner {actor_id:?} lost element \
                         {seq_id:?}/{elem_idx}"
                    )
                });
            assert_eq!(element.owner, Some(actor_id));
            assert_eq!(element.command, crate::element::Command::HitCmd);
            let order = element.current_order().unwrap_or_else(|| {
                panic!("completed Hit element {seq_id:?}/{elem_idx} lost its selected order")
            });
            assert_eq!(order.order_type, crate::order::OrderType::Hitting);
            assert_eq!(order.target_actor, Some(target_id.index()));
            self.check_sequence_element_validity(assets, actor_id, element, true)
        };
        if !valid {
            let sprite = &mut self
                .get_entity_mut(actor_id)
                .unwrap_or_else(|| {
                    panic!("completed Hit owner {actor_id:?} vanished after validation")
                })
                .element_data_mut()
                .sprite;
            sprite.perform_virgin_increment(sim, crate::sprite::FrameProgression::Default);
            tracing::debug!(
                attacker = ?actor_id,
                target = ?target_id,
                "Hit: terminal validity failed; suppressing damage"
            );
            return;
        }

        // Resolve the final concussion payload from the attacker:
        //   if PC has HitHard action → 150,
        //   else PC → 80,
        //   else NPC hitter → 40.
        // Human hitting execution applies Hard's
        // enemy-life-point multiplier here, while the PC authors
        // the damage element. The receive side consumes this
        // stored payload verbatim, including NPC/domino hits.
        let (concussion, is_harder_hit) = {
            let attacker = self.get_entity(actor_id);
            if attacker.is_some_and(|e| e.kind().is_pc()) {
                let has_hit_hard = attacker
                    .and_then(|e| e.pc_data())
                    .map(|pc| pc.profile_index)
                    .and_then(|idx| assets.profile_manager.get_character(idx))
                    .is_some_and(|cp| cp.has_action(crate::profiles::Action::HitHard));
                let base = if has_hit_hard {
                    (150u16, true)
                } else {
                    (80u16, false)
                };
                let percent = self
                    .control
                    .sim_config
                    .difficulty
                    .rules()
                    .pc_punch_concussion_percent;
                (
                    (u32::from(base.0) * u32::from(percent) / 100) as u16,
                    base.1,
                )
            } else {
                (40u16, false)
            }
        };

        // Launch a damage element on the target carrying the
        // attacker as antagonist and the resolved
        // concussion.
        let mut dmg = crate::sequence::SequenceElement::new_damage(
            1,
            crate::element::Command::ReceiveHitDamage,
            Some(target_id),
            Some(actor_id),
            0,
            concussion,
        );
        if let crate::sequence::SequenceElementData::Damage {
            is_harder_hit: ih, ..
        } = &mut dmg.data
        {
            *ih = is_harder_hit;
        }
        self.launch_element(sim, assets, dmg);

        tracing::debug!(
            attacker = ?actor_id,
            target = ?target_id,
            concussion,
            is_harder_hit,
            "Hit: launched RECEIVE_HIT_DAMAGE"
        );
    }

    pub(super) fn apply_ability_strangle_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
        target_id: EntityId,
    ) {
        self.find_place_to_die(target_id);
        // A soldier flagged not-stranglable in their profile
        // survives the strangle — the AI lock is released
        // and the soldier gets an EventGotHit stimulus so
        // it retaliates.
        let stranglable = match self.get_entity(target_id) {
            Some(crate::element::Entity::Soldier(s)) => {
                assets
                    .profile_manager
                    .get_soldier(s.soldier.soldier_profile_index)
                    .unwrap_or_else(|| {
                        panic!(
                            "strangle victim {target_id:?} has missing soldier profile {}",
                            s.soldier.soldier_profile_index
                        )
                    })
                    .strangle
            }
            Some(crate::element::Entity::Civilian(_)) => true,
            Some(_) => panic!("strangle victim {target_id:?} is not an NPC human"),
            None => panic!("strangle victim {target_id:?} disappeared at termination"),
        };

        if !stranglable {
            self.get_entity_mut(target_id)
                .expect("validated non-stranglable victim disappeared")
                .ai_controller_mut()
                .expect("non-stranglable victim must have AI")
                .non_script_unlock(crate::ai::AiLockFlags::FREEZE);
            let stimulus = crate::ai::Stimulus::with_human(
                crate::ai::StimulusType::EventGotHit,
                actor_id.index(),
            );
            self.execute_ai_callback(sim, assets, target_id, &stimulus);
            #[cfg(test)]
            crate::engine::soldier_helpers::observe_strangle_condolation_step(
                "TerminalEventGotHit",
            );
            tracing::debug!(
                attacker = ?actor_id,
                target = ?target_id,
                "Strangle: target not stranglable, completed EVENT_GOTHIT Think"
            );
            return;
        }

        // Full-life-points kill — launch ReceiveDamage on
        // the victim with damage = current life and
        // concussion = 0 and no damage origin.
        let life = match self.get_entity(target_id) {
            Some(crate::element::Entity::Soldier(s)) => s.npc.life_points,
            Some(crate::element::Entity::Civilian(c)) => c.npc.life_points,
            Some(_) => unreachable!("strangle victim kind validated above"),
            None => panic!("strangle victim {target_id:?} disappeared before damage"),
        };
        let life = u16::try_from(life)
            .unwrap_or_else(|_| panic!("strangle victim {target_id:?} has invalid life {life}"));
        let dmg = crate::sequence::SequenceElement::new_damage(
            1,
            crate::element::Command::ReceiveDamage,
            Some(target_id),
            None,
            life,
            0,
        );
        self.launch_element(sim, assets, dmg);

        tracing::debug!(
            attacker = ?actor_id,
            target = ?target_id,
            life,
            "Strangle: launched RECEIVE_DAMAGE for kill"
        );
    }

    // Sequence identity and the pre-tick freeze sample belong to this exact DONE edge.
    pub(super) fn apply_ability_strangle_setup_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
        target_id: EntityId,
        sprite_frozen: bool,
    ) -> SpriteMotionState {
        let (position, action_point, direction, layer, sector, obstacle, plane) = {
            let attacker = self
                .get_entity(actor_id)
                .unwrap_or_else(|| panic!("strangler {actor_id:?} vanished at Done"));
            let position = attacker.element_data().position_map();
            let hotspot = attacker
                .sprite()
                .current_hotspot()
                .expect("strangler current animation has no action point");
            let sprite_pos = attacker.gameplay_sprite_position();
            (
                position,
                crate::coordinates::MapPoint::new(
                    sprite_pos.x + hotspot.x,
                    sprite_pos.y + hotspot.y,
                ),
                u16::try_from(attacker.element_data().direction())
                    .expect("strangler direction must be in the canonical 0..=15 range"),
                attacker.element_data().layer(),
                attacker.element_data().sector(),
                attacker.element_data().obstacle_index(),
                attacker.position_iface().get_plane().copied(),
            )
        };
        {
            let victim = self
                .get_entity_mut(target_id)
                .unwrap_or_else(|| panic!("strangle victim {target_id:?} vanished at Done"));
            victim
                .element_data_mut()
                .set_obstacle_index(obstacle, plane);
            victim.element_data_mut().set_layer(layer);
            victim.element_data_mut().set_sector(sector);
            victim.element_data_mut().set_position_map(action_point);
            victim
                .element_data_mut()
                .set_direction_instantly(direction as i16);
        }
        let victim_move_box = {
            let victim = self
                .get_entity(target_id)
                .unwrap_or_else(|| panic!("strangle victim {target_id:?} vanished at Done"));
            *victim.position_iface().get_move_box()
        };
        let mut victim_box = victim_move_box.translated(action_point);
        if !victim_move_box.is_somewhere()
            || !self.world.fast_grid.find_authorized_position_toward(
                &mut victim_box,
                position,
                layer,
            )
        {
            return SpriteMotionState::Aborted;
        }
        let authorized_position = victim_box.center();
        {
            let victim = self
                .get_entity_mut(target_id)
                .unwrap_or_else(|| panic!("strangle victim {target_id:?} vanished at Done"));
            victim
                .element_data_mut()
                .set_position_map(authorized_position);
            victim.sprite_mut().compute_display_depth();
        }
        self.actor_freeze_execution(sim, assets, target_id);
        let victim = self
            .get_entity_mut(target_id)
            .unwrap_or_else(|| panic!("strangle victim {target_id:?} vanished at Done"));
        victim
            .element_data_mut()
            .sprite
            .force_animation(crate::order::OrderType::BeingStrangled, direction);
        let remark = if matches!(victim, crate::element::Entity::Civilian(_)) {
            crate::ai::Remark::CivDies
        } else {
            crate::ai::Remark::Strangled
        };
        self.execute_ai_speech(
            sim,
            assets,
            target_id,
            crate::ai::AiSpeechAttempt {
                remark,
                flags: crate::ai::SpeechFlags::EMERGENCY.bits(),
            },
        );
        if !sprite_frozen {
            self.get_entity_mut(target_id)
                .expect("strangle victim disappeared after speech")
                .element_data_mut()
                .sprite
                .perform_virgin_increment(sim, crate::sprite::FrameProgression::Default);
        }
        SpriteMotionState::Done
    }

    pub(super) fn initialize_ability_pay_init(
        &mut self,
        assets: &LevelAssets,
        actor_id: EntityId,
        ability: &crate::abilities::SelectedAbility,
    ) -> bool {
        let seq_id = ability.sequence_id;
        let elem_idx = ability.element_index;
        let beggar_id = ability.target.expect("Pay order requires an antagonist");
        let order_id = ability.order_id;
        let valid = {
            let element = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .unwrap_or_else(|| {
                    panic!("pending Pay owner {actor_id:?} lost element {seq_id:?}/{elem_idx}")
                });
            assert_eq!(element.owner, Some(actor_id));
            assert_eq!(element.command, crate::element::Command::Pay);
            let order = element.current_order().unwrap_or_else(|| {
                panic!("pending Pay element {seq_id:?}/{elem_idx} lost its selected order")
            });
            assert_eq!(order.order_id, order_id);
            assert_eq!(order.target_actor, Some(beggar_id.index()));
            self.check_sequence_element_validity(assets, actor_id, element, true)
        };
        if !valid {
            return false;
        }

        let beggar_direction = self
            .get_entity(beggar_id)
            .unwrap_or_else(|| {
                panic!("validated Pay beggar {beggar_id:?} vanished during initialization")
            })
            .element_data()
            .direction();
        // The player-character paying action samples the antagonist's live
        // direction and changes only the progressive goal on the first
        // Execute. Translation may happen after this PC's owner slot and
        // therefore must not expose this facing one frame early.
        self.get_entity_mut(actor_id)
            .expect("validated Pay owner vanished before direction initialization")
            .element_data_mut()
            .set_direction_goal((beggar_direction + 8).rem_euclid(16));
        self.hero_speaking(assets, actor_id, crate::engine::melee::HERO_GIVE_MONEY);
        true
    }

    pub(super) fn initialize_ability_hit_init(
        &mut self,
        assets: &LevelAssets,
        actor_id: EntityId,
        ability: &crate::abilities::SelectedAbility,
    ) -> bool {
        let seq_id = ability.sequence_id;
        let elem_idx = ability.element_index;
        let victim_id = ability.target.expect("Hit order requires an antagonist");
        let order_id = ability.order_id;
        let attacker_ground = self
            .get_entity(actor_id)
            .expect("Hit owner vanished during initialization")
            .ground_position();
        let victim_ground = self
            .get_entity(victim_id)
            .unwrap_or_else(|| panic!("Hit victim {victim_id:?} vanished during initialization"))
            .ground_position();
        let facing = crate::position_interface::vector_to_sector_0_to_15(
            victim_ground.x - attacker_ground.x,
            victim_ground.y - attacker_ground.y,
        );
        // Hit initialization changes only the progressive
        // direction goal, before checking whether the interaction remains
        // valid. The later Turn call owns the current-direction change.
        self.get_entity_mut(actor_id)
            .expect("Hit owner vanished before direction initialization")
            .element_data_mut()
            .set_direction_goal(facing);

        let valid = {
            let element = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .unwrap_or_else(|| {
                    panic!("pending Hit owner {actor_id:?} lost element {seq_id:?}/{elem_idx}")
                });
            assert_eq!(element.owner, Some(actor_id));
            assert_eq!(element.command, crate::element::Command::HitCmd);
            let order = element.current_order().unwrap_or_else(|| {
                panic!("pending Hit element {seq_id:?}/{elem_idx} lost its selected order")
            });
            assert_eq!(order.order_id, order_id);
            assert_eq!(order.target_actor, Some(victim_id.index()));
            self.check_sequence_element_validity(assets, actor_id, element, true)
        };
        if !valid {
            return false;
        }
        true
    }

    pub(super) fn initialize_ability_tying_init(
        &mut self,
        assets: &LevelAssets,
        actor_id: EntityId,
        ability: &crate::abilities::SelectedAbility,
    ) -> bool {
        let kind = ability.kind;
        let seq_id = ability.sequence_id;
        let elem_idx = ability.element_index;
        let target_id = ability.target.expect("Tying order requires an antagonist");
        let order_id = ability.order_id;
        let command = match kind {
            crate::movement::AbilityKind::Tie => crate::element::Command::TieCmd,
            crate::movement::AbilityKind::Untie => crate::element::Command::Untie,
            _ => unreachable!("pending tying initializer accepted a non-tying ability"),
        };
        let valid = {
            let element = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .unwrap_or_else(|| {
                    panic!("pending {kind:?} owner {actor_id:?} lost element {seq_id:?}/{elem_idx}")
                });
            assert_eq!(element.owner, Some(actor_id));
            assert_eq!(element.command, command);
            let order = element.current_order().unwrap_or_else(|| {
                panic!("pending {kind:?} element {seq_id:?}/{elem_idx} lost its selected order")
            });
            assert_eq!(order.order_id, order_id);
            assert_eq!(order.target_actor, Some(target_id.index()));
            self.check_sequence_element_validity(assets, actor_id, element, true)
        };
        if !valid {
            return false;
        }

        let actor_pos = self
            .get_entity(actor_id)
            .expect("validated tying owner vanished during initialization")
            .element_data()
            .position_map();
        let target_pos = self
            .get_entity(target_id)
            .expect("validated tying target vanished during initialization")
            .element_data()
            .position_map();
        let facing = crate::position_interface::vector_to_sector_0_to_15_iso(
            target_pos.x - actor_pos.x,
            target_pos.y - actor_pos.y,
        );
        // The player-character tying action installs only the progressive
        // direction goal during the order's first Execute. Translation
        // itself must not rotate or stop a moving PC: the Tie can still
        // be interrupted by an earlier manager-FIFO continuation before
        // its animation ever owns an actor slot.
        self.get_entity_mut(actor_id)
            .expect("validated tying owner vanished before direction initialization")
            .element_data_mut()
            .set_direction_goal(facing);
        true
    }

    pub(super) fn initialize_ability_carry_init(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
        ability: &crate::abilities::SelectedAbility,
    ) -> bool {
        let target_id = ability.target.expect("Carry order requires an antagonist");
        if !self.ability_carry_target_valid(assets, actor_id, ability) {
            return false;
        }
        // The pickup transition's first Execute is where the carried
        // body stops running its own sequence element and starts being
        // driven by the carrier, and where an indoor pickup re-selects
        // the pair and lights the body's hulk. Both are visible one
        // frame later than the element's translation, which happens in
        // the manager pass after the carrier's own slot.
        crate::abilities::initialize_carry_relationship(
            &mut self.world.entities,
            actor_id,
            target_id,
        );
        self.actor_freeze_execution(sim, assets, target_id);
        let carrier = self
            .get_entity(actor_id)
            .expect("Carry owner disappeared after freezing body");
        let position = carrier.element_data().position_map();
        let direction = carrier.element_data().direction().wrapping_sub(4) & 15;
        let target = self
            .get_entity_mut(target_id)
            .expect("Carry target disappeared after freeze");
        target.element_data_mut().set_direction_instantly(direction);
        target.element_data_mut().set_position_map(position);
        self.apply_carry_building_hulk(actor_id, target_id);
        true
    }

    pub(super) fn initialize_ability_climb_on_shoulders_init(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
        ability: &crate::abilities::SelectedAbility,
    ) -> bool {
        let helper_id = ability.target.expect("Climb order requires a helper");
        // Translate only appends the climbing order. Execution links the
        // pair, changes posture, snaps the climber, and freezes the helper
        // when that order reaches its first Execute in the climber's later
        // owner slot.
        crate::abilities::initialize_climb_on_shoulders_relationship(self, actor_id, helper_id);
        self.actor_freeze_execution(sim, assets, helper_id);
        true
    }

    pub(super) fn initialize_ability_heal_facing(
        &mut self,
        actor_id: EntityId,
        ability: &crate::abilities::SelectedAbility,
    ) -> bool {
        let target_id = ability
            .target
            .expect("Healing interaction requires a target");
        if target_id == actor_id {
            return true;
        }
        let healer_pos = self
            .get_entity(actor_id)
            .expect("Heal owner vanished during initialization")
            .element_data()
            .position_map();
        let target_pos = self
            .get_entity(target_id)
            .unwrap_or_else(|| panic!("Heal target {target_id:?} vanished during initialization"))
            .element_data()
            .position_map();
        let facing = crate::position_interface::vector_to_sector_0_to_15_iso(
            target_pos.x - healer_pos.x,
            target_pos.y - healer_pos.y,
        );
        // The healing animation computes the goal on its first execution,
        // then calls Turn before advancing the animation. Selection of
        // the interaction alone must not rotate the actor a frame early.
        self.get_entity_mut(actor_id)
            .expect("Heal owner vanished before direction initialization")
            .element_data_mut()
            .set_direction_goal(facing);
        true
    }

    pub(super) fn initialize_ability_strangle_init(
        &mut self,
        assets: &LevelAssets,
        actor_id: EntityId,
        ability: &crate::abilities::SelectedAbility,
    ) -> bool {
        let seq_id = ability.sequence_id;
        let elem_idx = ability.element_index;
        let victim_id = ability
            .target
            .expect("Strangle order requires an antagonist");
        let order_id = ability.order_id;
        let valid = {
            let element = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .unwrap_or_else(|| {
                    panic!("pending Strangle owner {actor_id:?} lost element {seq_id:?}/{elem_idx}")
                });
            assert_eq!(element.owner, Some(actor_id));
            assert_eq!(element.command, crate::element::Command::StrangleCmd);
            let order = element.current_order().unwrap_or_else(|| {
                panic!("pending Strangle element {seq_id:?}/{elem_idx} lost its selected order")
            });
            assert_eq!(order.order_id, order_id);
            assert_eq!(order.target_actor, Some(victim_id.index()));
            self.check_sequence_element_validity(assets, actor_id, element, true)
        };
        if !valid {
            return false;
        }

        let attacker_pos = self
            .get_entity(actor_id)
            .expect("validated strangler vanished during initialization")
            .element_data()
            .position_map();
        let victim_pos = self
            .get_entity(victim_id)
            .expect("validated Strangle victim vanished during initialization")
            .element_data()
            .position_map();
        let facing = crate::position_interface::vector_to_sector_0_to_15_iso(
            victim_pos.x - attacker_pos.x,
            victim_pos.y - attacker_pos.y,
        );
        self.get_entity_mut(victim_id)
            .expect("validated Strangle victim vanished before FREEZE")
            .ai_controller_mut()
            .expect("validated Strangle victim lost AI before FREEZE")
            .non_script_lock(crate::ai::AiLockFlags::FREEZE);
        self.get_entity_mut(actor_id)
            .expect("validated strangler vanished before direction initialization")
            .element_data_mut()
            .set_direction_goal(facing);
        self.get_entity_mut(victim_id)
            .expect("validated Strangle victim vanished before direction initialization")
            .element_data_mut()
            .set_direction_goal(facing);
        true
    }

    /// Advance the active ability for one actor.
    ///
    /// This is the per-owner unit used by the engine's creation-ordered element
    /// pass.
    pub(crate) fn tick_selected_ability(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        requested_actor: EntityId,
        sprite_frozen: bool,
    ) -> Option<SpriteMotionState> {
        let entity = self
            .world
            .entities
            .get(requested_actor)
            .unwrap_or_else(|| panic!("ability owner {requested_actor:?} disappeared"));
        assert!(
            entity.actor_data().is_some(),
            "ability owner {requested_actor:?} is not an actor"
        );
        let entity_id = requested_actor;
        let Some(ability) = selected_ability(
            &self.world.entities,
            &self.orders.sequence_manager,
            requested_actor,
        ) else {
            return None;
        };
        let initialising = self
            .world
            .entities
            .get(requested_actor)
            .expect("ability owner disappeared")
            .actor_data()
            .expect("ability owner lost actor state")
            .execute_order_initialising;
        let initialized = !initialising
            || match ability.kind {
                AbilityKind::Pay => {
                    self.initialize_ability_pay_init(assets, requested_actor, &ability)
                }
                AbilityKind::Hit => {
                    self.initialize_ability_hit_init(assets, requested_actor, &ability)
                }
                AbilityKind::Tie | AbilityKind::Untie => {
                    self.initialize_ability_tying_init(assets, requested_actor, &ability)
                }
                AbilityKind::Carry => {
                    self.initialize_ability_carry_init(sim, assets, requested_actor, &ability)
                }
                AbilityKind::ClimbOnShoulders => self.initialize_ability_climb_on_shoulders_init(
                    sim,
                    assets,
                    requested_actor,
                    &ability,
                ),
                AbilityKind::Heal => self.initialize_ability_heal_facing(requested_actor, &ability),
                AbilityKind::Strangle => {
                    self.initialize_ability_strangle_init(assets, requested_actor, &ability)
                }
                _ => true,
            };
        if !initialized {
            return Some(SpriteMotionState::Aborted);
        }
        // Execute retains the entry order and its antagonist through initialization.
        let kind = ability.kind;

        if let Some(motion) = self.tick_ability_pre_action(sim, assets, requested_actor, &ability) {
            return Some(motion);
        }

        let entity = self
            .world
            .entities
            .get_mut(requested_actor)
            .unwrap_or_else(|| panic!("ability owner {requested_actor:?} disappeared after setup"));

        // ── Listen: phase-aware animation dispatch ──
        //
        // Listen has three animation phases tracked by
        // the current order. The entry and exit transitions
        // are one-shot animations driven here; the middle CountingDown
        // phase is a loop driven by the idle-pose animation driver
        // plus the shared `wait_time` countdown in
        // the selected PC owner arm.
        if kind == AbilityKind::Listen {
            return Some(self.tick_listen(sim, assets, entity_id, &ability, sprite_frozen));
        }

        if kind == AbilityKind::ReceivePurse {
            return Some(self.tick_receive_purse(sim, assets, entity_id, &ability, sprite_frozen));
        }

        let order_id = Some(ability.order_id);
        // Self-heal swaps Healing → Eating; all other abilities use
        // the canonical per-kind animation.
        let entity_id_here = entity_id;
        let order_type = if kind == AbilityKind::Heal && ability.target == Some(entity_id_here) {
            OrderType::Eating
        } else {
            ability_order_type(kind)
        };
        // These ability arms turn progressively toward the direction installed at
        // Execute-time initialization. The throws, `Hit` and `Pay` freeze the first
        // sprite frame until alignment; the rest turn for its side effect and
        // advance the action unconditionally. `Carry`, `Drop`, `Whistle` and
        // `ClimbDownFromShoulders` do not turn at all.
        let turning = matches!(
            kind,
            AbilityKind::Hit
                | AbilityKind::Heal
                | AbilityKind::Pay
                | AbilityKind::Tie
                | AbilityKind::Untie
                | AbilityKind::Eat
                | AbilityKind::ClimbOnShoulders
                | AbilityKind::ThrowApple
                | AbilityKind::ThrowStone
                | AbilityKind::ThrowPurse
                | AbilityKind::ThrowWaspNest
                | AbilityKind::ThrowNet
        ) && entity.position_iface_mut().turn();
        let frame_progression = if kind == AbilityKind::Untie {
            crate::sprite::FrameProgression::Reversed
        } else if matches!(
            kind,
            AbilityKind::Hit
                | AbilityKind::Pay
                | AbilityKind::ThrowApple
                | AbilityKind::ThrowStone
                | AbilityKind::ThrowPurse
                | AbilityKind::ThrowWaspNest
                | AbilityKind::ThrowNet
        ) && turning
        {
            crate::sprite::FrameProgression::FrozenFirstFrame
        } else {
            crate::sprite::FrameProgression::Default
        };
        let direction = u16::try_from(entity.element_data().direction()).unwrap_or_else(|_| {
            panic!("{kind:?} owner {entity_id:?} has invalid animation direction")
        });
        if kind == AbilityKind::Whistle && entity.actor_data().unwrap().execute_order_initialising {
            entity.actor_data_mut().unwrap().wait_time = crate::abilities::TIME_LISTEN_WAIT;
        }

        // Drive the animation through the sprite state machine.
        let motion = if sprite_frozen {
            SpriteMotionState::InProgress
        } else {
            let elem = entity.element_data_mut();
            elem.sprite.perform_action(
                sim,
                order_id,
                order_type,
                direction,
                frame_progression,
                false,
            )
        };

        // Whistle wait-time countdown.  Drives the expanding
        // noise-ellipse render in `render_listen_ping`; armed to
        // `TIME_LISTEN_WAIT` in `begin_whistle`.
        if kind == AbilityKind::Whistle {
            let actor = entity.actor_data_mut().unwrap();
            if actor.wait_time != 0 {
                actor.wait_time -= 1;
            }
        }

        // Carried sprites follow the action before its completion can change the relationship.
        match kind {
            AbilityKind::Carry | AbilityKind::Drop => {
                crate::abilities::sync_corpse_animation_for_carrier(
                    &mut self.world.entities,
                    &assets.profile_manager,
                    entity_id,
                    order_type,
                )
            }
            AbilityKind::ClimbOnShoulders | AbilityKind::ClimbDownFromShoulders => {
                crate::abilities::sync_shoulder_climb_animation(
                    &mut self.world.entities,
                    entity_id,
                    order_type,
                )
            }
            _ => {}
        }

        // DONE setup forces the victim's animation and advances it itself. Other
        // motion states still run the tail increment before termination effects.
        if kind == AbilityKind::Strangle && motion != SpriteMotionState::Done && !sprite_frozen {
            self.advance_pre_action_strangle_victim_if_due(
                sim,
                entity_id,
                ability.target.expect("strangle target"),
            );
        }

        // Only act on completion states.
        if !matches!(
            motion,
            SpriteMotionState::Done | SpriteMotionState::Terminated | SpriteMotionState::Aborted
        ) {
            return Some(motion);
        }

        if motion == SpriteMotionState::Aborted {
            return Some(motion);
        }
        if motion == SpriteMotionState::Terminated {
            match kind {
                AbilityKind::Drop => self.execute_corpse_drop_done(sim, assets, entity_id),
                AbilityKind::ClimbOnShoulders => self.apply_ability_climb_on_shoulders_done(
                    sim,
                    assets,
                    entity_id,
                    ability.target.expect("climb helper"),
                ),
                AbilityKind::ClimbDownFromShoulders => self
                    .apply_ability_climb_down_from_shoulders_done(
                        sim,
                        assets,
                        entity_id,
                        ability.target.expect("dismount helper"),
                    ),
                AbilityKind::Strangle => self.apply_ability_strangle_done(
                    sim,
                    assets,
                    entity_id,
                    ability.target.expect("strangle target"),
                ),
                _ => {}
            }
            return Some(motion);
        }

        if matches!(
            kind,
            AbilityKind::Drop | AbilityKind::ClimbOnShoulders | AbilityKind::ClimbDownFromShoulders
        ) {
            return Some(motion);
        }

        Some(self.execute_ability_done(sim, assets, entity_id, &ability, sprite_frozen))
    }

    fn tick_ability_pre_action(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        requested_actor: EntityId,
        ability: &SelectedAbility,
    ) -> Option<SpriteMotionState> {
        let kind = ability.kind;
        if kind == AbilityKind::Carry
            && !ability.order_done
            && !self.ability_carry_target_valid(assets, requested_actor, ability)
        {
            return Some(SpriteMotionState::Aborted);
        }
        // Player-character tying execution revalidates the antagonist every
        // frame. The DONE callback itself changes Lying -> Tied, so the next
        // Execute deliberately fails this check and aborts/releases the Tie
        // element instead of playing the unused animation tail.
        if kind == AbilityKind::Tie {
            let target_id = ability
                .target
                .expect("active Tie ability must retain its antagonist");
            let target_valid = self.world.entities.get(target_id).is_some_and(|target| {
                target.human_data().is_some_and(|human| human.unconscious)
                    && target.element_data().posture() == Posture::Lying
            });
            if !target_valid {
                return Some(SpriteMotionState::Aborted);
            }
        }
        if kind == AbilityKind::Untie && !ability.order_done {
            let target_id = ability
                .target
                .expect("active Untie ability must retain its antagonist");
            let target_valid = self.world.entities.get(target_id).is_some_and(|target| {
                target.is_active()
                    && target.is_npc()
                    && !target.is_dead()
                    && target.human_data().is_some()
                    && target.element_data().posture() == Posture::Tied
            });
            if !target_valid {
                return Some(SpriteMotionState::Aborted);
            }
        }

        // Strangling uses left-to-right conditional ordering:
        // attacker fast turning runs first, and the victim is not advanced until a
        // later tick where the attacker was already aligned. Action processing is
        // likewise deferred until both calls return false. Direction goals and
        // the victim FREEZE lock are installed by the engine at the
        // post-translation initialization boundary.
        if kind == AbilityKind::Strangle {
            let victim_id = ability
                .target
                .expect("active Strangle ability must retain its antagonist");
            if self
                .world
                .entities
                .get_mut(requested_actor)
                .expect("validated strangle owner vanished before fast turning")
                .position_iface_mut()
                .turn_fast()
            {
                self.advance_pre_action_strangle_victim_if_due(sim, requested_actor, victim_id);
                return Some(SpriteMotionState::InProgress);
            }
            let victim =
                self.world.entities.get_mut(victim_id).unwrap_or_else(|| {
                    panic!("strangle victim {victim_id:?} vanished while turning")
                });
            assert!(
                victim.actor_data().is_some(),
                "strangle victim {victim_id:?} lost required actor state while turning"
            );
            if victim.position_iface_mut().turn_fast() {
                self.advance_pre_action_strangle_victim_if_due(sim, requested_actor, victim_id);
                return Some(SpriteMotionState::InProgress);
            }
        }

        if kind == AbilityKind::ClimbOnShoulders {
            let helper_id = ability
                .target
                .expect("active ClimbOnShoulders ability must retain its helper");
            let helper_direction = self
                .world
                .entities
                .get(helper_id)
                .unwrap_or_else(|| {
                    panic!("climb-on-shoulders helper {helper_id:?} vanished during Execute")
                })
                .element_data()
                .direction();
            // Reissue this progressive facing goal before Turn on every
            // execution of the shoulder-climbing animation.
            self.world
                .entities
                .get_mut(requested_actor)
                .expect("climb-on-shoulders owner vanished before facing update")
                .element_data_mut()
                .set_direction_goal((helper_direction + 8) & 15);
        }

        None
    }

    fn ability_carry_target_valid(
        &self,
        assets: &LevelAssets,
        actor_id: EntityId,
        ability: &SelectedAbility,
    ) -> bool {
        let element = self
            .orders
            .sequence_manager
            .get_element(ability.sequence_id, ability.element_index)
            .expect("Carry execution lost its selected element");
        self.check_sequence_element_validity(assets, actor_id, element, true)
    }

    fn tick_listen(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        ability: &SelectedAbility,
        sprite_frozen: bool,
    ) -> SpriteMotionState {
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("ability owner disappeared");
        let order_type = ability.order_type;
        // All three listen arms call `Turn()` ahead of their sprite action, so
        // the row played this tick belongs to the already-stepped direction.
        let _ = entity.position_iface_mut().turn();
        let direction = u16::try_from(entity.element_data().direction()).unwrap_or_else(|_| {
            panic!("Listen owner {entity_id:?} has invalid animation direction")
        });
        let order_id = Some(ability.order_id);

        let motion = if sprite_frozen {
            SpriteMotionState::InProgress
        } else {
            let elem = entity.element_data_mut();
            elem.sprite.perform_action(
                sim,
                order_id,
                order_type,
                direction,
                crate::sprite::FrameProgression::Default,
                false,
            )
        };
        if !matches!(
            motion,
            SpriteMotionState::Done | SpriteMotionState::Terminated | SpriteMotionState::Aborted
        ) {
            return motion;
        }
        let actor = entity.actor_data_mut().unwrap_or_else(|| {
            panic!("asserted Listen owner {entity_id:?} lost required actor state")
        });
        match motion {
            SpriteMotionState::Done => {
                if order_type == OrderType::TransitionWaitingUprightListening {
                    // Switch to the listening pose (driven by
                    // animation.rs idle-pose fallback) and hand off
                    // to the ai.rs countdown.
                    actor.action_state = ActionState::Listening;
                    self.apply_ability_listen_entered(sim, assets, entity_id);
                } else if order_type == OrderType::TransitionListeningWaitingUpright {
                    actor.action_state = ActionState::Waiting;
                    self.apply_ability_listen_done(sim, assets, entity_id);
                }
            }
            _ => {}
        }
        motion
    }

    fn tick_receive_purse(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        ability: &SelectedAbility,
        sprite_frozen: bool,
    ) -> SpriteMotionState {
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("ability owner disappeared");
        let order_type = ability.order_type;
        let direction = u16::try_from(entity.element_data().direction()).unwrap_or_else(|_| {
            panic!("ReceivePurse owner {entity_id:?} has invalid animation direction")
        });
        let order_id = Some(ability.order_id);

        let motion = if sprite_frozen {
            SpriteMotionState::InProgress
        } else {
            let elem = entity.element_data_mut();
            elem.sprite.perform_action(
                sim,
                order_id,
                order_type,
                direction,
                crate::sprite::FrameProgression::Default,
                false,
            )
        };
        if !matches!(
            motion,
            SpriteMotionState::Terminated | SpriteMotionState::Aborted
        ) {
            return motion;
        }

        let actor = entity.actor_data_mut().unwrap_or_else(|| {
            panic!("asserted ReceivePurse owner {entity_id:?} lost required actor state")
        });
        if motion == SpriteMotionState::Aborted {
            return motion;
        }
        match order_type {
            OrderType::ReceivingPurse => {}
            OrderType::WaitingWithPurse => {
                self.apply_ability_receive_purse_revealing(sim, assets, entity_id);
            }
            OrderType::TransitionWaitingWithPurseWaitingUpright => {
                actor.action_state = ActionState::Waiting;
                tracing::debug!(beggar = ?entity_id, "ReceivePurse: animation chain complete");
            }
            _ => unreachable!("selected purse order changed kind"),
        }
        motion
    }

    fn execute_ability_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        ability: &SelectedAbility,
        sprite_frozen: bool,
    ) -> SpriteMotionState {
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("ability owner disappeared");
        let kind = ability.kind;
        let seq_id = ability.sequence_id;
        let elem_idx = ability.element_index;
        // Apply the selected order's effect at its animation boundary.
        let actor_pos = entity.element_data().position_map();

        let target = || {
            ability
                .target
                .expect("target-bearing completion requires its antagonist")
        };
        match kind {
            AbilityKind::Carry => self.apply_ability_carry_done(entity_id, target()),
            AbilityKind::Drop => unreachable!("drop completion runs at animation termination"),
            AbilityKind::Tie => self.apply_ability_tie_done(sim, assets, entity_id, target()),
            AbilityKind::Untie => self.apply_ability_untie_done(sim, assets, entity_id, target()),
            AbilityKind::Heal => {
                return self.apply_ability_heal_done(
                    sim,
                    assets,
                    entity_id,
                    target(),
                    seq_id,
                    elem_idx,
                );
            }
            AbilityKind::Whistle => {
                self.apply_ability_whistle_done(sim, assets, entity_id, actor_pos)
            }
            AbilityKind::Pay => {
                return self.apply_ability_pay_done(
                    sim,
                    assets,
                    entity_id,
                    ability
                        .target
                        .expect("AbilityKind::Pay must carry a beggar target (set in begin_pay)"),
                    seq_id,
                    elem_idx,
                );
            }
            AbilityKind::Listen | AbilityKind::ReceivePurse => unreachable!(
                "{kind:?} is handled by the phase-aware inline branch earlier \
                 in tick_selected_ability and never reaches the generic completion match"
            ),
            AbilityKind::ThrowNet => {
                // Target position was stored in the order on the
                // owning sequence element.
                let target_pos = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.current_order())
                    .map(|o| MapPoint {
                        x: o.target_x,
                        y: o.target_y,
                    })
                    .unwrap_or_else(|| panic!("ThrowNet selected without its required live order"));
                self.apply_ability_throw_net_done(assets, entity_id, target_pos)
            }
            AbilityKind::ThrowWaspNest => {
                let target_pos = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.current_order())
                    .map(|o| MapPoint {
                        x: o.target_x,
                        y: o.target_y,
                    })
                    .unwrap_or_else(|| {
                        panic!("ThrowWaspNest selected without its required live order")
                    });
                self.apply_ability_throw_wasp_nest_done(assets, entity_id, target_pos)
            }
            AbilityKind::ThrowPurse => {
                let target_pos = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.current_order())
                    .map(|o| MapPoint {
                        x: o.target_x,
                        y: o.target_y,
                    })
                    .unwrap_or_else(|| {
                        panic!("ThrowPurse selected without its required live order")
                    });
                self.apply_ability_throw_purse_done(sim, assets, entity_id, target_pos)
            }
            AbilityKind::ThrowApple => self.on_throw_projectile_done(
                assets,
                entity_id,
                ability.target,
                crate::profiles::Action::Apple,
                crate::element::ObjectType::Apple,
            ),
            AbilityKind::ThrowStone => {
                let ground_target = ability.target.is_none().then(|| {
                    self.orders
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .and_then(|element| {
                            element.get_property(crate::sequence::Field::NoiseDistractionTarget)
                        })
                        .and_then(|value| match value {
                            crate::sequence::FieldValue::Point3D { x, y, z } => {
                                Some(crate::coordinates::WorldPoint3D::new(*x, *y, *z))
                            }
                            _ => None,
                        })
                        .unwrap_or_else(|| {
                            panic!("ground ThrowStone selected without its required 3D target")
                        })
                });
                match (ability.target, ground_target) {
                    (Some(target), None) => self.on_throw_projectile_done(
                        assets,
                        entity_id,
                        Some(target),
                        crate::profiles::Action::Stone,
                        crate::element::ObjectType::Stone,
                    ),
                    (None, Some(target)) => {
                        self.on_throw_noise_distraction_done(assets, entity_id, target)
                    }
                    pair => panic!(
                        "completed ThrowStone must carry exactly one target kind, got {pair:?}"
                    ),
                }
            }
            AbilityKind::Hit => self.apply_ability_hit_done(
                sim,
                assets,
                entity_id,
                ability
                    .target
                    .expect("AbilityKind::Hit must carry a target (set in begin_hit)"),
                seq_id,
                elem_idx,
            ),
            AbilityKind::Strangle => {
                return self.apply_ability_strangle_setup_done(
                    sim,
                    assets,
                    entity_id,
                    ability.target.expect(
                        "AbilityKind::Strangle must carry a target (set in begin_strangle)",
                    ),
                    sprite_frozen,
                );
            }
            AbilityKind::Eat => self.apply_ability_eat_done(assets, entity_id),
            AbilityKind::ClimbOnShoulders | AbilityKind::ClimbDownFromShoulders => {
                unreachable!("shoulder completion runs at animation termination")
            }
        }
        SpriteMotionState::Done
    }

    /// Match the strangling tail while its attacker-and-victim fast-turn guard
    /// short-circuits before action processing.
    ///
    /// A completed attacker sprite row advances the victim without other effects. This is
    /// observable when Strangle follows an already-done walk-to-wait transition:
    /// the victim's independent turning animation advances a second time in the
    /// same frame even though the Strangling animation has not started yet.
    fn advance_pre_action_strangle_victim_if_due(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        attacker_id: EntityId,
        victim_id: EntityId,
    ) {
        let due = {
            let attacker = self.world.entities.get(attacker_id).unwrap_or_else(|| {
                panic!("strangle attacker {attacker_id:?} vanished while turning")
            });
            let sprite = attacker.sprite();
            !sprite.current_scripts().is_empty()
                && sprite.current_frame >= sprite.action_done_for_row(sprite.current_row)
        };
        if due {
            self.world
                .entities
                .get_mut(victim_id)
                .unwrap_or_else(|| {
                    panic!("strangle victim {victim_id:?} vanished before increment")
                })
                .element_data_mut()
                .sprite
                .perform_virgin_increment(sim, crate::sprite::FrameProgression::Default);
        }
    }
}
