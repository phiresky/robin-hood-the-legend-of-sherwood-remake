//! Damage application, death handling, and knockout effects.
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct TestSwordDamageObservation {
    pub victim_id: EntityId,
    pub attacker_id: EntityId,
    pub strike: SwordStrike,
    pub attacker_direction: i16,
    pub active_rider_charge: bool,
    pub pending_victims: Vec<EntityId>,
    pub life_points_before: i16,
    pub life_points_after: i16,
    pub victim_direction_after: i16,
}

#[cfg(test)]
thread_local! {
    static TEST_SWORD_DAMAGE_OBSERVATIONS: std::cell::RefCell<Vec<TestSwordDamageObservation>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn clear_test_sword_damage_observations() {
    TEST_SWORD_DAMAGE_OBSERVATIONS.with(|observations| observations.borrow_mut().clear());
}

#[cfg(test)]
pub(crate) fn take_test_sword_damage_observations() -> Vec<TestSwordDamageObservation> {
    TEST_SWORD_DAMAGE_OBSERVATIONS.with(|observations| observations.take())
}

#[cfg(test)]
pub(crate) fn test_human_life_points(entity: &crate::element::Entity) -> Option<i16> {
    match entity {
        crate::element::Entity::Pc(pc) => Some(pc.pc.life_points),
        crate::element::Entity::Soldier(soldier) => Some(soldier.npc.life_points),
        crate::element::Entity::Civilian(civilian) => Some(civilian.npc.life_points),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn record_test_sword_damage_observation(
    engine: &EngineInner,
    victim_id: EntityId,
    attacker_id: EntityId,
    strike: SwordStrike,
    life_points_before: i16,
) {
    let attacker = engine
        .get_entity(attacker_id)
        .expect("sword damage test attacker exists");
    let actor = attacker
        .actor_data()
        .expect("sword damage attacker is actor");
    let observation = TestSwordDamageObservation {
        victim_id,
        attacker_id,
        strike,
        attacker_direction: attacker.element_data().direction(),
        active_rider_charge: actor.active_rider_charge.is_some(),
        pending_victims: actor
            .active_rider_charge
            .as_ref()
            .map(|charge| charge.pending_victims.clone())
            .unwrap_or_default(),
        life_points_before,
        life_points_after: engine
            .get_entity(victim_id)
            .and_then(test_human_life_points)
            .expect("sword damage test victim remains human"),
        victim_direction_after: engine
            .get_entity(victim_id)
            .expect("sword damage test victim remains present")
            .element_data()
            .direction(),
    };
    TEST_SWORD_DAMAGE_OBSERVATIONS.with(|observations| {
        observations.borrow_mut().push(observation);
    });
}
use crate::combat::{self, SwordAttackerContext, SwordDamageParams, SwordDefenderContext};
use crate::element::{ActionState, Camp, Entity, EntityId, EyeStatus, Posture};
use crate::weapons::SwordStrike;

impl EngineInner {
    /// Nudge the victim to the nearest authorised position so the
    /// corpse doesn't overlap props or other actors.  Invoked at
    /// dispatch time instead of on every `DYING_*` / `FALLING_BACK_*`
    /// Execute init pass — same effect, fewer per-frame redundant
    /// relocations.
    pub(crate) fn find_place_to_die(&mut self, victim_id: EntityId) {
        const BOX_LYING_X: f32 = 10.0;
        const BOX_LYING_Y: f32 = 5.0;
        let (start, layer) = match self.get_entity(victim_id) {
            Some(e) => (e.element_data().position_map(), e.element_data().layer()),
            None => return,
        };
        let mut bbox = crate::coordinates::MapBBox::from_corners(
            crate::coordinates::MapPoint::new(start.x - BOX_LYING_X, start.y - BOX_LYING_Y),
            crate::coordinates::MapPoint::new(start.x + BOX_LYING_X, start.y + BOX_LYING_Y),
        );
        // Use the click-biased `find_authorized_position_toward` so
        // the box stays pulled toward the actor's original spot; the
        // plain variant only gathers lines intersecting the moving
        // box and can drift further.
        let click = crate::coordinates::MapPoint::new(start.x, start.y);
        if self
            .world
            .fast_grid
            .find_authorized_position_toward(&mut bbox, click, layer)
        {
            let center = bbox.center();
            if let Some(entity) = self.world.entities.get_mut(victim_id) {
                entity
                    .element_data_mut()
                    .set_position_map(crate::coordinates::MapPoint {
                        x: center.x,
                        y: center.y,
                    });
            }
        }
    }

    /// Push `anim` as the next order on `damage_element` and bind the
    /// owning actor's `active_ai_anim` to it with SequenceElement
    /// completion.  The element transitions into the new order on the
    /// next `do_next_order` invocation.
    ///
    /// The element's `NonInterruptable` priority bump (when needed) is
    /// applied lazily by `anim_forces_non_interruptable_on_start` in
    /// `tick_actor_animation_for` on MotionState::Start of the new
    /// animation.
    pub(super) fn queue_damage_anim(
        &mut self,
        victim_id: EntityId,
        damage_element: (crate::sequence::SequenceId, usize),
        anim: OrderType,
    ) {
        let _ = victim_id;
        let (dseq, didx) = damage_element;
        self.push_new_order(dseq, didx, anim, 0.0, 0.0);
    }

    // ─── Damage application ─────────────────────────────────────────

    /// Build and queue a `ReceiveSwordDamage` sequence element targeting
    /// `victim_id`.
    ///
    /// Use this from sword-strike resolution paths
    /// (`tick_active_sweeps`, `tick_active_rider_charges`, etc.) so the
    /// hit-reaction animations flow through `do_next_order` instead of
    /// the legacy direct `combat_anim` writes. The element is deliberately
    /// only registered here: Original `LaunchSequenceElement` leaves it
    /// for `RHSequenceManager::Hourglass` after every actor has performed
    /// its frame, so the victim can finish its already-selected actor tick
    /// before the damage interrupts it.
    pub(crate) fn queue_sword_damage(
        &mut self,
        _sim: &crate::sim_rng::SimulationContext,
        _assets: &LevelAssets,
        victim_id: EntityId,
        attacker_id: EntityId,
        sword_strike: SwordStrike,
        attacker_profile_idx: u32,
    ) {
        let Some(victim) = self.get_entity(victim_id) else {
            return;
        };
        // Dead/unconscious humans are pruned from active swordfight
        // state: setting concussion calls quit_swordfight, victim
        // discovery rejects unconscious/dead, and the CheckOpponents
        // invariant asserts no such opponent remains.  Sweep/charge
        // queues can otherwise keep a stale victim into a later frame,
        // so do not launch fresh damage.
        if victim.is_dead() || victim.human_data().is_some_and(|h| h.unconscious) {
            tracing::debug!(
                ?victim_id,
                ?attacker_id,
                ?sword_strike,
                "sword damage skipped: victim already dead or unconscious"
            );
            return;
        }

        let mut elem = crate::sequence::SequenceElement::new(
            1,
            crate::element::Command::ReceiveSwordDamage,
            Some(victim_id),
        );
        elem.data = crate::sequence::SequenceElementData::new_sword_damage(
            attacker_id,
            sword_strike,
            attacker_profile_idx,
        );
        // Resolve the value normally determined by Instruct, but bypass the
        // engine's ordinary owned-element launcher because that path performs
        // synchronous arbitration. The manager-tail InstructOwner action owns
        // arbitration, transition generation, and damage dispatch.
        self.resolve_element_priority(&mut elem);
        self.orders.sequence_manager.launch_element(elem);
    }

    /// Apply sword damage to a victim.
    ///
    /// Reads attacker and defender profiles, calls `combat::receive_sword_damage`,
    /// then handles death/KO transitions.
    ///
    /// `damage_element` identifies the receive-damage sequence
    /// element this call is fulfilling.  The hit-reaction animations
    /// (simple-hit / standup / stunned-recovery) are pushed onto that
    /// element via `push_order_on` and consumed by `do_next_order`.
    /// Direct sword-strike resolution paths use
    /// `queue_sword_damage` to build and register a real
    /// `ReceiveSwordDamage` element, which the manager-tail instruction
    /// dispatches here with a valid `damage_element`.
    pub(super) fn apply_sword_damage(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        attacker_id: Option<EntityId>,
        sword_strike: Option<SwordStrike>,
        attacker_profile_idx: Option<u32>,
        damage_element: (crate::sequence::SequenceId, usize),
    ) {
        if self.is_scroll_protected_civilian(victim_id) {
            tracing::debug!(?victim_id, "sword damage blocked: scroll-carrying beggar");
            return;
        }
        let strike = match sword_strike {
            Some(s) => s,
            None => {
                tracing::warn!(?victim_id, "apply_sword_damage: no strike type");
                return;
            }
        };

        // Ladder/wall arm — route to `translate_ladder_wall_fall`
        // before any damage / push / hit-reaction work.  Same
        // early-out as `apply_generic_damage` and
        // `apply_piercing_damage`.
        let pre_drop_posture = self
            .get_entity(victim_id)
            .map(|e| e.element_data().posture)
            .unwrap_or_default();
        if matches!(pre_drop_posture, Posture::OnLadder | Posture::OnWall) {
            self.translate_ladder_wall_fall(victim_id, damage_element);
            return;
        }

        // CarryingCorpse arm — drop the corpse instantly (the
        // carrier then falls through to the base-class sword-damage
        // path which runs damage application + push handling + hit
        // reaction below). Done up-front so the carrier's posture is
        // already Upright by the time `apply_push_effect` and the
        // hit-reaction animation pick run.
        if pre_drop_posture == Posture::CarryingCorpse {
            self.force_drop_carried_corpse_instant(victim_id);
        }

        // Look up the attacker's weapon profile
        let attacker_profile = attacker_profile_idx
            .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
            .cloned();
        let default_profile;
        let attacker_profile = match attacker_profile {
            Some(p) => p,
            None => {
                tracing::warn!(
                    ?victim_id,
                    ?attacker_profile_idx,
                    "apply_sword_damage: no attacker profile, using defaults"
                );
                // Use a zeroed-out default so damage still flows
                // through the protection/concussion pipeline (which
                // may still produce knockdowns from concussion alone).
                default_profile = crate::profiles::HtHWeaponProfile::default();
                default_profile
            }
        };

        // Read attacker context — real fighting_ability from profile,
        // is_rank_soldier checks the RANK_SOLDIER flag from soldier
        // profile.  Note: the protection-direction sector is computed
        // from defender → attacker, not the other way around.
        let (
            attacker_dir,
            def_to_atk_dir,
            attacker_elevation,
            fighting_ability,
            atk_is_rank_soldier,
        ) = if let Some(attacker) = attacker_id {
            let (dir, elev) = self
                .get_entity(attacker)
                .map(|e| {
                    let elem = e.element_data();
                    (elem.direction(), elem.position().z)
                })
                .unwrap_or((0, 0.0));
            let def_to_atk = direction_to(&self.world.entities, victim_id, attacker);
            let ability = self
                .get_entity(attacker)
                .map(|e| {
                    fighting_ability_from_profile(
                        e,
                        &assets.profile_manager,
                        sim.config().difficulty,
                    )
                })
                .unwrap_or(50);
            let is_rank = self
                .get_entity(attacker)
                .map(|e| is_rank_soldier(e, &assets.profile_manager))
                .unwrap_or(false);
            (dir, def_to_atk, elev, ability, is_rank)
        } else {
            // No attacker (scripted damage): zero elevation — for
            // un-sited sources the elevated-defender branch only fires
            // when the defender truly stands higher.
            (0, 0, 0.0, 50, false)
        };

        // Read defender context
        let victim = match self.world.entities.get(victim_id) {
            Some(e) => e,
            None => return,
        };
        let defender_dir = victim.element_data().direction();
        let defender_elevation = victim.element_data().position().z;
        let defender_action = victim
            .actor_data()
            .map(|a| a.action_state)
            .unwrap_or(ActionState::Waiting);
        let life_points_before = get_life_points(victim);

        // Look up defender's weapon profile
        let defender_profile_idx = get_hth_weapon_id_full(victim, &assets.profile_manager);
        let defender_profile = defender_profile_idx
            .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
            .cloned();

        let ctx = concussion_ctx_full(
            victim,
            self.is_sherwood(&assets.profile_manager),
            Some(&self.mission_domain.campaign),
            self.control.sim_config.difficulty,
        );

        // Build params and apply damage
        let attacker_ctx = SwordAttackerContext {
            direction: attacker_dir,
            direction_to_attacker: def_to_atk_dir,
            elevation: attacker_elevation,
            fighting_ability,
            is_rank_soldier: atk_is_rank_soldier,
        };
        let defender_ctx = SwordDefenderContext {
            action_state: defender_action,
            direction: defender_dir,
            elevation: defender_elevation,
        };
        let victim_max_hp = get_max_life_points(victim);
        let params = SwordDamageParams {
            defender: &defender_ctx,
            defender_profile: defender_profile.as_ref(),
            attacker_profile: &attacker_profile,
            strike,
            attacker: &attacker_ctx,
            concussion_ctx: &ctx,
            max_life_points: victim_max_hp,
        };

        // Apply damage (requires mutable access to human_data + life_points)
        let victim = match self.world.entities.get_mut(victim_id) {
            Some(e) => e,
            None => return,
        };
        let victim_is_pc = victim.kind().is_pc();
        let (human, lp) = match victim.human_and_life_points_mut() {
            Some(pair) => pair,
            None => return,
        };

        let (result, cutting_inflicted) = combat::receive_sword_damage(sim, human, lp, &params);

        let raw_life_points_after = *lp;
        // RHElementActorHuman::ReceiveSwordDamage dispatches GetWounded
        // virtually. For a VIP PC, RHElementActorPC::GetWounded consumes an
        // amulet and establishes the 5-HP coma state *inside* the cutting
        // damage call, before ReceiveSwordDamage returns and before SayOuch
        // classifies the result as HERO_HURT or HERO_DIE. Rust's shared
        // damage primitive cannot mutate campaign state, so close that
        // virtual boundary here rather than deferring it to
        // handle_post_damage.
        let coma_saved = victim_is_pc
            && raw_life_points_after <= 0
            && cutting_inflicted > 0
            && self.try_pc_coma_save(sim, assets, victim_id, cutting_inflicted);
        if coma_saved && 5 < life_points_before - 20 {
            // PC::GetWounded marks the campaign coma, stores the 5-HP
            // floor through PC::SetLifePoints (which emits HERO_HURT for a
            // drop greater than twenty), and only then applies maximum
            // concussion. The shared SayOuch path below intentionally skips
            // unconscious actors, so preserve that virtual SetLifePoints
            // callback explicitly at this boundary.
            self.hero_speaking(assets, victim_id, HERO_HURT);
        }
        let life_points_after = self
            .get_entity(victim_id)
            .map(get_life_points)
            .unwrap_or(raw_life_points_after);
        // Use the attempted damage (not the clamped lp delta) so
        // overkill hits display the same number as a non-overkill hit
        // would have shown.
        if cutting_inflicted > 0 {
            self.add_damage_number(victim_id, cutting_inflicted);
        }

        tracing::debug!(
            ?victim_id,
            ?attacker_id,
            ?strike,
            ?result,
            life_points_after,
            "Sword damage applied"
        );

        // Soldier learning reads the attacker's *live* command, not the
        // strike stored in this damage payload. ReceiveSwordDamage can be
        // translated after the attacker has already selected its next
        // strike, and Original deliberately memorizes that newer command:
        // `MakeBadSwordstrikeExperience(pDamage->GetOrigin()->GetCommand())`.
        // This is before the sound/parry return in
        // `RHElementActorHuman::Instruct`, so even a fully parried hit updates
        // the defender's strike memory.
        if let Some(Entity::Soldier(_)) = self.world.entities.get(victim_id)
            && let Some(attacker_id) = attacker_id
            && let Some(live_strike) =
                crate::weapons::SwordStrike::from_command(self.actor_command(attacker_id))
        {
            self.make_bad_sword_strike_experience(assets, victim_id, live_strike, true);
        }

        // Play impact sound effect (queued for next audio hourglass).
        // Different sounds for parried vs armor hit, light vs heavy
        // strikes.
        {
            use crate::sound::ImpactKind;

            let victim_pos = self
                .get_entity(victim_id)
                .map(|e| e.element_data().position_map())
                .unwrap_or(crate::coordinates::MapPoint::ZERO);

            // Real weapon/armor materials from character/soldier profiles
            let atk_weapon_mat = attacker_id
                .and_then(|id| self.get_entity(id))
                .map(|e| weapon_material_from_profile(e, &assets.profile_manager))
                .unwrap_or(crate::profiles::WeaponMaterial::SteelAndWood);
            let def_armor_mat = self
                .get_entity(victim_id)
                .map(|e| armor_material_from_profile(e, &assets.profile_manager))
                .unwrap_or(crate::profiles::ArmorMaterial::Plate);

            // Light strikes (A, B, D, E) get light impact; others get
            // heavy.
            let impact_kind = match strike {
                SwordStrike::A | SwordStrike::B | SwordStrike::D | SwordStrike::E => {
                    ImpactKind::LightArmor
                }
                _ => ImpactKind::HeavyArmor,
            };

            // For parried strikes, play strike FX (parry sound)
            // instead of impact FX.
            if result.contains(combat::SwordDamageResult::NO_DAMAGE_PARRIED) {
                use crate::sound::StrikeKind;
                let parry_kind = match strike {
                    SwordStrike::A | SwordStrike::B | SwordStrike::D | SwordStrike::E => {
                        StrikeKind::LightParade
                    }
                    _ => StrikeKind::HeavyParade,
                };
                let def_weapon_mat = self
                    .get_entity(victim_id)
                    .map(|e| weapon_material_from_profile(e, &assets.profile_manager))
                    .unwrap_or(crate::profiles::WeaponMaterial::SteelAndWood);
                self.feedback
                    .pending_side_effects
                    .sounds
                    .push(super::SoundCommand::StrikeFx {
                        strike_kind: parry_kind,
                        weapon1: atk_weapon_mat,
                        weapon2: def_weapon_mat,
                        position: victim_pos,
                    });

                // Parry early-return: when the defender wasn't
                // already in a parry state, push the parry-to-waiting
                // transition anim onto the damage element; then return
                // immediately, skipping push/XP/SayOuch/hero-speech/
                // provoke/AI-stim and the regular hit-reaction.
                if defender_action != ActionState::ParryingSword
                    && defender_action != ActionState::ParryingSwordLow
                {
                    let (dseq, didx) = damage_element;
                    self.push_new_order(
                        dseq,
                        didx,
                        crate::order::OrderType::TransitionParryingSwordWaitingSword,
                        0.0,
                        0.0,
                    );
                }
                return;
            } else {
                self.feedback
                    .pending_side_effects
                    .sounds
                    .push(super::SoundCommand::ImpactFx {
                        impact_kind,
                        weapon: atk_weapon_mat,
                        armor: def_armor_mat,
                        position: victim_pos,
                    });
            }
        }

        // Handle push effect — the push path handles animations and
        // death/KO internally, so skip the regular hit anim and
        // `handle_post_damage` when pushed.
        let pushed = if combat::strike_has_push_effect(&attacker_profile, strike) {
            let thrust = &attacker_profile.thrusts[strike as usize];
            if let Some(attacker) = attacker_id {
                let push_info = PushStrikeInfo {
                    repulsion: thrust.repulsion,
                    kind: thrust.kind,
                    strike,
                    max_distance: thrust.maximal_distance as f32,
                };
                self.apply_push_effect(
                    sim,
                    assets,
                    victim_id,
                    attacker,
                    &push_info,
                    result,
                    damage_element,
                )
            } else {
                false
            }
        } else {
            false
        };

        // Award XP if the victim died
        let victim_died = self
            .get_entity(victim_id)
            .map(|e| get_life_points(e) <= 0)
            .unwrap_or(false);
        if victim_died && let Some(atk_id) = attacker_id {
            self.award_sword_kill_xp(assets, atk_id, victim_id);
        }

        // SayOuch on the victim (unless parried or push already said it).
        if !coma_saved
            && !pushed
            && !result.is_empty()
            && !result.contains(combat::SwordDamageResult::NO_DAMAGE_PARRIED)
        {
            self.say_ouch(sim, assets, victim_id, Some(cutting_inflicted));
        }

        // Provoke after sword strike — random taunt. Original evaluates this
        // gate before the later hero-speech checks.
        if !result.is_empty()
            && !result.contains(combat::SwordDamageResult::NO_DAMAGE_PARRIED)
            && !strike.is_smalltalk()
            && let Some(atk_id) = attacker_id
        {
            // Original evaluates the random chance before its selected-PC
            // suppression clause.  A controlled attacker therefore still
            // owns one global draw even though it can never launch Provoke.
            let provoke_chance = (0.2 * attacker_ctx.fighting_ability as f32) as u32;
            let provoke_roll =
                crate::sim_rng::u32(sim, crate::sim_rng::RngSite::MeleeProvoke, 0..100);

            // Suppress Provoke when the attacker is the currently-selected
            // PC — the player's controlled character shouldn't taunt on hit.
            let attacker_is_selected_pc = self
                .get_entity(atk_id)
                .map(|e| e.kind().is_pc())
                .unwrap_or(false)
                && self.selected_pc_ids().contains(&atk_id);
            if provoke_roll < provoke_chance && !attacker_is_selected_pc {
                self.launch_provoke(atk_id);
            }
        }

        // Hero speech for PC attacker:
        // - HERO_KILLED_OPPONENT if dead
        // - HERO_SUCCESSFULL_BLOW if unconscious + cutting > 50
        // - HERO_STUN_ENNEMY if unconscious otherwise
        let attacker_is_pc = attacker_id
            .and_then(|id| self.get_entity(id))
            .map(|e| e.kind().is_pc())
            .unwrap_or(false);
        let victim_is_unconscious = self
            .get_entity(victim_id)
            .and_then(|e| e.human_data())
            .map(|h| h.unconscious)
            .unwrap_or(false);
        let victim_is_lacklandist = self
            .get_entity(victim_id)
            .map(|e| match e {
                Entity::Soldier(s) => s.soldier.cached_camp == crate::element::Camp::Lacklandists,
                _ => false,
            })
            .unwrap_or(false);

        if attacker_is_pc
            && victim_is_lacklandist
            && !result.is_empty()
            && let Some(atk_id) = attacker_id
        {
            if victim_died {
                self.hero_speaking(assets, atk_id, HERO_KILLED_OPPONENT);
            } else if victim_is_unconscious {
                let cutting = combat::get_strike_cutting_effect(
                    &attacker_profile,
                    strike,
                    attacker_ctx.fighting_ability,
                    attacker_ctx.is_rank_soldier,
                );
                if cutting > 50 {
                    self.hero_speaking(assets, atk_id, HERO_SUCCESSFULL_BLOW);
                } else {
                    self.hero_speaking(assets, atk_id, HERO_STUN_ENNEMY);
                }
            }
        }

        // Play posture-based hit reaction animation for non-lethal hits
        // (BeingHitSword / FallingBackBow / etc.) when there IS damage
        // but the victim is still alive and conscious.  Skip for push
        // strikes — the push path handles its own anims.
        if !pushed
            && !result.is_empty()
            && !result.contains(combat::SwordDamageResult::NO_DAMAGE_PARRIED)
        {
            let still_alive = life_points_after > 0;
            let still_conscious = self
                .get_entity(victim_id)
                .and_then(|e| e.human_data())
                .map(|h| !h.unconscious)
                .unwrap_or(false);
            // Shoulder-posture victims route through
            // `translate_shoulder_damage` *unconditionally* — even for
            // lethal/KO hits — so the partner's carrier/carried
            // linkage still unwinds via the Fall sub-sequence that
            // helper launches.  The normal hit-reaction anim is only
            // picked for alive+conscious victims on a non-shoulder
            // posture; dead or KO'd non-shoulder victims fall through
            // to the regular post-damage pipeline below.
            let victim_posture = self
                .get_entity(victim_id)
                .map(|e| e.element_data().posture)
                .unwrap_or_default();
            let is_shoulder_posture = matches!(
                victim_posture,
                Posture::OnShoulders | Posture::CarryingOnShoulders | Posture::HelpingToClimb
            );
            if is_shoulder_posture {
                self.translate_shoulder_damage(sim, assets, victim_id, damage_element);
            } else if still_alive && still_conscious {
                let anims = self.get_entity(victim_id).and_then(|e| {
                    let posture = e.element_data().posture;
                    let action = e.actor_data().map(|a| a.action_state).unwrap_or_default();
                    select_combat_animations(posture, action)
                });
                if let Some(a) = anims {
                    let (dseq, didx) = damage_element;
                    if result.contains(combat::SwordDamageResult::STUNNING_DAMAGE) {
                        // Stunning hit chain: fall-back, roll,
                        // stand-up, optional in-place stun if the
                        // defender is mid-swordfight with concussion
                        // above the threshold.
                        self.push_new_order(dseq, didx, a.falling_back, 0.0, 0.0);
                        self.try_queue_roll(assets, victim_id, damage_element);
                        self.push_new_order(dseq, didx, a.standing_up, 0.0, 0.0);
                        let (is_swordfighting, concussion) = self
                            .get_entity(victim_id)
                            .and_then(|e| e.human_data())
                            .map(|h| (!h.opponents.is_empty(), h.concussion_of_the_brain))
                            .unwrap_or((false, 0));
                        if is_swordfighting && concussion > STUNNING_THRESHOLD {
                            self.push_new_order(
                                dseq,
                                didx,
                                crate::order::OrderType::BeingStunnedSword,
                                0.0,
                                0.0,
                            );
                        }
                    } else if result.contains(combat::SwordDamageResult::CUTTING_DAMAGE) {
                        // Cutting hit, no follow-up roll / stand-up.
                        self.push_new_order(dseq, didx, a.simple_hit, 0.0, 0.0);
                    }
                }
            }
        }

        // Dispatch combat stimulus to attacker's AI: EventLethalStrike
        // if victim died, EventGoodStrike if damage was dealt.
        //
        // Original provenance: `RHElementActorHuman::TranslateDamage` calls
        // the attacker's `Think(EVENT_{LETHAL,GOOD}_STRIKE)` inline
        // (`original-code/RHelementactorhuman.cpp:2633-2665`). In particular,
        // the good-strike handler must still observe
        // `SUBSTATE_ATTACKING_SWORDFIGHT_SPECIAL_STRIKE`; leaving this in the
        // ordinary NPC-phase queue can suppress the associated combat remark.
        if !result.is_empty()
            && !result.contains(combat::SwordDamageResult::NO_DAMAGE_PARRIED)
            && let Some(atk_id) = attacker_id
        {
            let stimulus_type = if victim_died {
                crate::ai::StimulusType::EventLethalStrike
            } else {
                crate::ai::StimulusType::EventGoodStrike
            };
            self.dispatch_ai_stimulus(atk_id, crate::ai::Stimulus::new(stimulus_type));
            self.tick_enemy_ai_drain_pending_stimuli_for_npc(sim, atk_id, assets, None, None);
        }

        // Death push-vs-drop selector: a non-rider killed by a strike
        // with positive stunning effect falls on his back rather than
        // dropping forward.
        let dying_anim_override = if victim_died {
            let is_rider = matches!(
                self.get_entity(victim_id),
                Some(Entity::Soldier(s)) if s.soldier.rider
            );
            let stunning_effect = combat::get_strike_stunning_effect(&attacker_profile, strike);
            if !is_rider && stunning_effect > 0 {
                self.get_entity(victim_id)
                    .and_then(|e| {
                        let posture = e.element_data().posture;
                        let action = e.actor_data().map(|a| a.action_state).unwrap_or_default();
                        select_combat_animations(posture, action)
                    })
                    .map(|a| a.falling_back)
            } else {
                None
            }
        } else {
            None
        };

        // Handle state transitions after damage — skip for push strikes,
        // since apply_push_effect already handled death/KO transitions.
        if !pushed {
            self.handle_post_damage(
                sim,
                assets,
                victim_id,
                attacker_id,
                result.is_empty(),
                damage_element,
                dying_anim_override,
            );
        }
    }

    /// Forced instantaneous corpse drop.  Used when a PC carrying a
    /// body takes any damage (arrow/stone, generic, hit, push, sword
    /// fall-through arms) and when an `EnterSwordfight` transition
    /// fires while the PC is still carrying a body: the corpse is
    /// snapped to the carrier's feet, postures are reset, and the
    /// carried link is cleared so the next action can proceed on an
    /// un-carriered PC.
    pub(crate) fn force_drop_carried_corpse_instant(&mut self, carrier_id: EntityId) {
        let (
            carrier_pos,
            carrier_layer,
            carrier_sector,
            carrier_obstacle,
            carrier_plane,
            carrier_dir,
            carried_id,
            carried_posture,
        ) = {
            let carrier = match self.get_entity(carrier_id) {
                Some(e) => e,
                None => return,
            };
            if !carrier.is_pc() {
                return;
            }
            let pc = match carrier.pc_data() {
                Some(p) => p,
                None => return,
            };
            let carried_id = match pc.carried {
                Some(id) => id,
                None => return,
            };
            let elem = carrier.element_data();
            // `sync_carried_positions` runs every tick while the body is
            // held and copies the carrier's plane Z onto the carried;
            // reading it from the carrier's already-resolved
            // `PositionInterface` here mirrors that path and avoids
            // re-resolving from `assets.static_sight_obstacles`.
            let plane = carrier.position_iface().get_plane().copied();
            (
                elem.position_map(),
                elem.layer(),
                elem.sector(),
                elem.obstacle_index(),
                plane,
                elem.direction(),
                carried_id,
                pc.live_carried_posture(),
            )
        };

        // Gate the post-drop hulk flash on dead/unconscious bodies
        // dropped inside a building so they remain visible through
        // walls.
        let in_building = is_in_building_sector(carrier_sector, &self.world.fast_grid);

        if let Some(carried) = self.get_entity_mut(carried_id) {
            let elem = carried.element_data_mut();
            elem.set_obstacle_index(carrier_obstacle, carrier_plane);
            elem.set_layer(carrier_layer);
            elem.set_sector(carrier_sector);
            elem.set_position_map(carrier_pos);
            // direction = (carrier_dir + 12) & 15.
            elem.set_direction_instantly((carrier_dir + 12) & 15);
            // Stop tracking the carrier's display order.
            elem.sprite.display_order_ref = None;
            elem.sprite.behind_display_order_ref = false;
            carried.set_posture(carried_posture);
            if let Some(human) = carried.human_data_mut() {
                human.carrier = None;
            }
            if let Some(actor) = carried.actor_data_mut() {
                actor.execution_frozen = false;
                actor.action_state = ActionState::Waiting;
            }
        }

        if let Some(carrier) = self.get_entity_mut(carrier_id) {
            carrier.set_posture(Posture::Upright);
            if let Some(actor) = carrier.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            if let Some(pc) = carrier.pc_data_mut() {
                pc.carried = None;
            }
        }

        // Inside a building, dead/unconscious bodies get a hulk flash
        // and `SetActive(true)` so they stay visible through walls.
        // Mirrors the same fan-out the animated `DropDone` handler
        // runs in `engine/combat.rs`.
        if in_building && let Some(carried) = self.get_entity_mut(carried_id) {
            let is_dead = carried.is_dead();
            let is_unconscious = carried.human_data().is_some_and(|h| h.unconscious);
            if is_dead || is_unconscious {
                crate::engine::door_pass::start_hulk_on(carried, 1.0);
                let elem = carried.element_data_mut();
                elem.hidden_in_building = false;
                elem.active = true;
            }
        }

        // Low-priority Wait element so the dropped body re-enters an
        // idle state.
        let mut wait_elem = crate::sequence::SequenceElement::new(
            1,
            crate::element::Command::Wait,
            Some(carried_id),
        );
        wait_elem.priority = crate::sequence::SequencePriority::Wait;
        self.launch_element(wait_elem);
    }

    /// Apply generic damage (falling, environmental, mobile collision).
    pub(super) fn apply_generic_damage(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        damage: u16,
        concussion: u16,
        damage_element: (crate::sequence::SequenceId, usize),
    ) {
        if self.is_scroll_protected_civilian(victim_id) {
            tracing::debug!(?victim_id, "generic damage blocked: scroll-carrying beggar");
            return;
        }

        // Generic-damage early-out arms before damage math:
        //   1. OnLadder / OnWall → translate_ladder_wall_fall.
        //   2. Already-lying non-rider with life_points <= 0 →
        //      terminate immediately.  Uses `life_points > 0`, not
        //      `!is_dead()`, to keep parity with the original guard.
        let pre_posture = self
            .get_entity(victim_id)
            .map(|e| e.element_data().posture)
            .unwrap_or_default();
        if matches!(pre_posture, Posture::OnLadder | Posture::OnWall) {
            self.translate_ladder_wall_fall(victim_id, damage_element);
            return;
        }
        if pre_posture.is_lying() {
            let still_alive = self
                .get_entity(victim_id)
                .map(|e| get_life_points(e) > 0)
                .unwrap_or(false);
            let is_rider = matches!(
                self.get_entity(victim_id),
                Some(Entity::Soldier(s)) if s.soldier.rider
            );
            if !is_rider && !still_alive {
                let (dseq, didx) = damage_element;
                self.orders.sequence_manager.element_terminated(dseq, didx);
                return;
            }
        }

        let victim = match self.world.entities.get(victim_id) {
            Some(e) => e,
            None => return,
        };
        let ctx = concussion_ctx_full(
            victim,
            self.is_sherwood(&assets.profile_manager),
            Some(&self.mission_domain.campaign),
            self.control.sim_config.difficulty,
        );
        let max_lp = get_max_life_points(victim);

        let victim = match self.world.entities.get_mut(victim_id) {
            Some(e) => e,
            None => return,
        };
        let (human, lp) = match victim.human_and_life_points_mut() {
            Some(pair) => pair,
            None => return,
        };

        let _died = combat::receive_generic_damage(human, lp, damage, concussion, max_lp, &ctx);

        // Shoulder-posture victims route through
        // `translate_shoulder_damage` instead of the base-class
        // handler.
        let victim_posture = self
            .get_entity(victim_id)
            .map(|e| e.element_data().posture)
            .unwrap_or_default();
        if matches!(
            victim_posture,
            Posture::OnShoulders | Posture::CarryingOnShoulders | Posture::HelpingToClimb
        ) {
            self.translate_shoulder_damage(sim, assets, victim_id, damage_element);
            self.handle_post_damage(sim, assets, victim_id, None, false, damage_element, None);
            return;
        }

        // CarryingCorpse arm — forces an instant corpse drop and
        // falls through to the default damage path.
        if victim_posture == Posture::CarryingCorpse {
            self.force_drop_carried_corpse_instant(victim_id);
        }

        self.say_ouch(sim, assets, victim_id, None);

        // Alive-conscious branch: queue the posture-dependent
        // simple-hit animation onto the damage element and fire the
        // roll helper.  The death / knockout outcomes are handled
        // downstream in `handle_post_damage` →
        // `handle_death_with_damage_element` / `handle_knockout`,
        // which push their own animations.
        let (life_points_after, still_conscious, still_on_ground) = {
            let victim = self.get_entity(victim_id);
            (
                victim.map(get_life_points).unwrap_or(0),
                victim
                    .and_then(|e| e.human_data())
                    .map(|h| !h.unconscious)
                    .unwrap_or(false),
                victim
                    .map(|e| !e.element_data().posture.is_lying())
                    .unwrap_or(false),
            )
        };
        if life_points_after > 0 && still_conscious && still_on_ground {
            let hit_anim = self
                .get_entity(victim_id)
                .and_then(|e| {
                    let posture = e.element_data().posture;
                    let action = e.actor_data().map(|a| a.action_state).unwrap_or_default();
                    select_combat_animations(posture, action)
                })
                .map(|a| a.simple_hit);
            if let Some(anim) = hit_anim {
                let (dseq, didx) = damage_element;
                self.push_new_order(dseq, didx, anim, 0.0, 0.0);
            }
            // Unconditional roll attempt (except for net damage, which
            // routes through a different path that never reaches
            // `apply_generic_damage`).
            self.try_queue_roll(assets, victim_id, damage_element);
        }

        self.handle_post_damage(sim, assets, victim_id, None, false, damage_element, None);
    }

    /// Apply piercing damage (arrows, stones).
    ///
    /// `is_arrow_damage` distinguishes the two entry points: arrow
    /// damage queues `EXTRACTING_ARROW_*` for survivors; stone damage
    /// queues the generic posture-dependent simple-hit anim.  Both
    /// share the same piercing damage math.
    pub(super) fn apply_piercing_damage(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        damage: u16,
        concussion: u16,
        is_arrow_damage: bool,
        damage_element: (crate::sequence::SequenceId, usize),
    ) {
        if self.is_scroll_protected_civilian(victim_id) {
            tracing::debug!(
                ?victim_id,
                "piercing damage blocked: scroll-carrying beggar"
            );
            return;
        }

        // Ladder/wall arm — route to `translate_ladder_wall_fall`
        // before damage math, like `apply_generic_damage` and
        // `apply_push_effect`.
        let pre_posture = self
            .get_entity(victim_id)
            .map(|e| e.element_data().posture)
            .unwrap_or_default();
        if matches!(pre_posture, Posture::OnLadder | Posture::OnWall) {
            self.translate_ladder_wall_fall(victim_id, damage_element);
            return;
        }

        let victim = match self.world.entities.get(victim_id) {
            Some(e) => e,
            None => return,
        };
        let ctx = concussion_ctx_full(
            victim,
            self.is_sherwood(&assets.profile_manager),
            Some(&self.mission_domain.campaign),
            self.control.sim_config.difficulty,
        );
        let max_lp = get_max_life_points(victim);

        let victim = match self.world.entities.get_mut(victim_id) {
            Some(e) => e,
            None => return,
        };
        let (human, lp) = match victim.human_and_life_points_mut() {
            Some(pair) => pair,
            None => return,
        };

        let _died = combat::receive_piercing_damage(human, lp, damage, concussion, max_lp, &ctx);
        // Raw attempted damage — overkill hits show the same number
        // as a non-overkill hit would.  `add_damage_number` no-ops on 0.
        self.add_damage_number(victim_id, damage);

        // Already-lying arm: `Lying/UnderNet/Flying/Carried/
        // OnShoulders/Tied` falls through to `Dead/DeadBack`, which
        // terminates the element when not a dying rider — i.e. for
        // everything except a sleeping rider dying.  This stops arrow
        // / stone damage that lands on an already-on-the-ground
        // victim from pushing a fresh dying order onto the damage
        // element.
        if pre_posture.is_lying() {
            let post_dead = self
                .get_entity(victim_id)
                .map(|e| get_life_points(e) <= 0)
                .unwrap_or(false);
            let is_rider = matches!(
                self.get_entity(victim_id),
                Some(Entity::Soldier(s)) if s.soldier.rider
            );
            if !is_rider || !post_dead {
                let (dseq, didx) = damage_element;
                self.orders.sequence_manager.element_terminated(dseq, didx);
                return;
            }
            // TODO: match the Original sleeping-rider special case,
            // which selects an upright dying animation even from a
            // lying posture.
        }

        // Shoulder-posture victims route through
        // `translate_shoulder_damage`.
        if matches!(
            pre_posture,
            Posture::OnShoulders | Posture::CarryingOnShoulders | Posture::HelpingToClimb
        ) {
            self.translate_shoulder_damage(sim, assets, victim_id, damage_element);
            self.handle_post_damage(sim, assets, victim_id, None, false, damage_element, None);
            return;
        }

        // CarryingCorpse arm — forces an instant corpse drop and
        // falls through to the default damage path.
        if pre_posture == Posture::CarryingCorpse {
            self.force_drop_carried_corpse_instant(victim_id);
        }

        self.say_ouch(sim, assets, victim_id, Some(damage));

        // Alive-conscious branch: queue the posture-dependent
        // extract-arrow animation onto the damage element, then fire
        // the roll helper.  For stones we push the `simple_hit`
        // variant from the same selector.  The death / knockout
        // outcomes are handled downstream in `handle_post_damage` →
        // `handle_death_with_damage_element` / `handle_knockout`,
        // which push their own animations.
        let (life_points_after, still_conscious, still_on_ground) = {
            let victim = self.get_entity(victim_id);
            (
                victim.map(get_life_points).unwrap_or(0),
                victim
                    .and_then(|e| e.human_data())
                    .map(|h| !h.unconscious)
                    .unwrap_or(false),
                victim
                    .map(|e| !e.element_data().posture.is_lying())
                    .unwrap_or(false),
            )
        };
        if life_points_after > 0 && still_conscious && still_on_ground {
            let hit_anim = self
                .get_entity(victim_id)
                .and_then(|e| {
                    let posture = e.element_data().posture;
                    let action = e.actor_data().map(|a| a.action_state).unwrap_or_default();
                    select_combat_animations(posture, action)
                })
                .map(|a| {
                    if is_arrow_damage {
                        a.arrow_extract
                    } else {
                        a.simple_hit
                    }
                });
            if let Some(anim) = hit_anim {
                let (dseq, didx) = damage_element;
                self.push_new_order(dseq, didx, anim, 0.0, 0.0);
            }
            // Unconditional roll attempt.
            self.try_queue_roll(assets, victim_id, damage_element);
        }

        self.handle_post_damage(sim, assets, victim_id, None, false, damage_element, None);
    }

    /// Apply hit damage (fist/club, concussion only).
    ///
    /// `damage_element` (when set) identifies the receive-damage sequence
    /// element this call is fulfilling — see `apply_sword_damage` for the
    /// same threading; the hit-fall animation gets pushed onto the
    /// element so `do_next_order` advances naturally instead of writing
    /// `combat_anim` directly.
    pub(super) fn apply_hit_damage(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        attacker_id: Option<EntityId>,
        concussion: u16,
        is_harder_hit: bool,
        damage_element: (crate::sequence::SequenceId, usize),
    ) {
        if self.is_scroll_protected_civilian(victim_id) {
            tracing::debug!(?victim_id, "hit damage blocked: scroll-carrying beggar");
            return;
        }
        let victim = match self.world.entities.get(victim_id) {
            Some(e) => e,
            None => return,
        };
        let ctx = concussion_ctx_full(
            victim,
            self.is_sherwood(&assets.profile_manager),
            Some(&self.mission_domain.campaign),
            self.control.sim_config.difficulty,
        );
        let life_points = get_life_points(victim);
        let is_lacklandist = victim.is_soldier()
            && victim.soldier_data().map(|s| s.cached_camp)
                == Some(crate::element::Camp::Lacklandists);

        let victim = match self.world.entities.get_mut(victim_id) {
            Some(e) => e,
            None => return,
        };
        let human = match victim.human_data_mut() {
            Some(h) => h,
            None => return,
        };

        let outcome =
            combat::receive_hit_damage(human, life_points, concussion, is_lacklandist, &ctx);
        let went_unconscious = outcome == combat::ConcussionOutcome::WentUnconscious;

        // SetConcussionOfTheBrain performs the knockout transition inline:
        // QuitSwordFight (including its synchronous AI callback), add the
        // unconscious titbit, then the NPC override synchronously Thinks
        // EVENT_LOSE_CONSCIOUSNESS.
        let attacker_is_pc = attacker_id
            .and_then(|id| self.get_entity(id))
            .map(|e| e.kind().is_pc())
            .unwrap_or(false);
        if went_unconscious {
            self.apply_knockout_side_effects(sim, assets, victim_id, attacker_is_pc, false);

            if let Some(atk_id) = attacker_id {
                let same_camp_soldier = {
                    let attacker = self.get_entity(atk_id);
                    let victim = self.get_entity(victim_id);
                    matches!(attacker, Some(Entity::Soldier(_)))
                        && attacker.map(Entity::camp) == victim.map(Entity::camp)
                };
                if same_camp_soldier {
                    let ai = self
                        .get_entity_mut(victim_id)
                        .and_then(Entity::ai_controller_mut)
                        .expect("knocked-out soldier lost its AI controller");
                    ai.knocked_out_in_money_fight = true;
                    ai.looted_after_money_fight = false;
                } else if attacker_is_pc {
                    self.hero_speaking(assets, atk_id, HERO_STUN_ENNEMY);
                }
            }

            // RECEIVE_HIT_DAMAGE explicitly calls QuitSwordFight once more
            // after SetConcussionOfTheBrain.  The relationship list is
            // already empty, but the soldier's synchronous
            // EVENT_QUIT_SWORDFIGHT callback still occurs (and is refused
            // while unconscious).
            self.quit_swordfight(sim, assets, victim_id);
        } else {
            let still_conscious = self
                .get_entity(victim_id)
                .and_then(Entity::human_data)
                .is_some_and(|human| !human.unconscious);
            if still_conscious {
                // Original always notifies a conscious NPC of the hit. It
                // attaches the hitter only when the origin is a human;
                // missing and non-human origins use the context-free event.
                let stimulus = match attacker_id.filter(|&id| {
                    self.get_entity(id)
                        .is_some_and(|attacker| attacker.kind().is_human())
                }) {
                    Some(atk_id) => crate::ai::Stimulus::with_human(
                        crate::ai::StimulusType::EventGotHit,
                        atk_id.index(),
                    ),
                    None => crate::ai::Stimulus::new(crate::ai::StimulusType::EventGotHit),
                };
                self.dispatch_synchronous_ai_think_preserving_detection_fifo(
                    sim,
                    victim_id,
                    assets,
                    stimulus,
                );
            }
        }

        // Shoulder fall routing — if the victim is currently on a
        // carrier's shoulders, the visual goes to
        // `translate_shoulder_damage` instead of the normal hit-fall
        // path.
        let victim_posture = self
            .get_entity(victim_id)
            .map(|e| e.element_data().posture)
            .unwrap_or_default();
        if matches!(
            victim_posture,
            Posture::OnShoulders | Posture::CarryingOnShoulders | Posture::HelpingToClimb
        ) {
            self.translate_shoulder_damage(sim, assets, victim_id, damage_element);
            return;
        }

        // Original `RHElementActorHuman::TranslateHitDamage` calls
        // `SayOuch()` synchronously before it appends the FALLING_HIT order.
        // This is particularly important when the hit interrupts speech
        // queued earlier in the same engine frame: SPEECH_EMERGENCY removes
        // that pending exclamation before Sound::Hourglass resolves the
        // replacement.  The already-down/flying postures terminate the
        // damage element without entering this default arm and stay silent.
        if !matches!(
            victim_posture,
            Posture::Lying
                | Posture::StuckUnderNet
                | Posture::Flying
                | Posture::Carried
                | Posture::OnShoulders
                | Posture::Tied
                | Posture::Dead
                | Posture::DeadBack
        ) {
            self.say_ouch(sim, assets, victim_id, None);
        }

        // CarryingCorpse arm — drop the corpse instantly (the
        // carrier then falls through to the base-class hit-damage
        // path which dispatches the regular hit-fall animation
        // below).
        if victim_posture == Posture::CarryingCorpse {
            self.force_drop_carried_corpse_instant(victim_id);
        }

        // OnLadder / OnWall fall routing — these postures route
        // through `translate_ladder_wall_fall`, matching the parallel
        // push-path routing.
        if matches!(victim_posture, Posture::OnLadder | Posture::OnWall) {
            self.translate_ladder_wall_fall(victim_id, damage_element);
            return;
        }

        // Play the FALLING_HIT_* animation.  Non-harder hits flight
        // 30 units away from the antagonist under non-interruptable
        // priority and end lying; harder hits play in place and
        // collapse to lying on completion.
        self.dispatch_hit_fall_animation(
            assets,
            victim_id,
            attacker_id,
            is_harder_hit,
            damage_element,
        );

        // Hit damage changes concussion only.  Its knockout transition was
        // completed synchronously above, and TranslateHitDamage's
        // FALLING_HIT_* wrapper already owns the entire fall animation.
        // Routing through handle_post_damage would append a second,
        // standalone FALLING_BACK_* order that the Original never creates.
    }

    /// TranslateHitDamage selects and appends the `FALLING_HIT_*` order.
    ///
    /// This deliberately does not initialize non-hard flight state.
    /// Original `TranslateHitDamage` only stores the antagonist and an
    /// order with `bComputeDirection = false`; `ExecuteFallingHit` samples
    /// the live positions, changes facing, and calls `ReadyForTakeOff` when
    /// that order first executes.
    pub(super) fn dispatch_hit_fall_animation(
        &mut self,
        assets: &LevelAssets,
        victim_id: EntityId,
        attacker_id: Option<EntityId>,
        is_harder_hit: bool,
        damage_element: (crate::sequence::SequenceId, usize),
    ) {
        // Read victim posture + action state to pick the right animation.
        let (victim_posture, victim_action) = {
            let v = match self.get_entity(victim_id) {
                Some(e) => e,
                None => return,
            };
            let posture = v.element_data().posture;
            let action = v.actor_data().map(|a| a.action_state).unwrap_or_default();
            (posture, action)
        };

        // Early-out: posture is already falling, carried, or dead —
        // nothing to animate.
        let anim = match select_hit_fall_animation(victim_posture, victim_action, is_harder_hit) {
            Some(a) => a,
            None => return,
        };

        let (dseq, didx) = damage_element;
        let id = self.orders.allocate_order_id();
        let mut order = crate::order::Order::new(anim, 0.0, 0.0, id);
        order.compute_direction = false;
        order.antagonist = attacker_id;
        self.orders
            .sequence_manager
            .push_order_on(dseq, didx, order);

        tracing::debug!(
            victim = ?victim_id,
            attacker = ?attacker_id,
            ?anim,
            harder = is_harder_hit,
            "Hit fall animation queued"
        );

        // The default arm of the hit-damage path unconditionally
        // attempts a roll, so any upright/crouched/sitting hit on a
        // sloped obstacle queues a `Rolling` sub-animation.  The
        // death/KO branches in `handle_post_damage` will queue a
        // separate roll after death/unconscious posture, but a
        // non-fatal hit on a slope must roll too.
        self.try_queue_roll(assets, victim_id, damage_element);
    }

    /// Initialize a non-hard `FALLING_HIT_*` order on its first Execute.
    ///
    /// This is the `IsInitialisation()` body of Original
    /// `ExecuteFallingHit`: it samples live actor positions, sets the
    /// back-first facing, and prepares the flight. Translation must not call
    /// it because the victim's creation slot may already have run this frame.
    pub(crate) fn initialize_hit_flight(
        &mut self,
        assets: &LevelAssets,
        victim_id: EntityId,
        attacker_id: Option<EntityId>,
        anim: OrderType,
    ) {
        if !matches!(
            anim,
            OrderType::FallingHitUpright
                | OrderType::FallingHitWithBow
                | OrderType::FallingHitWithSword
                | OrderType::FallingHitCrouched
        ) {
            return;
        }

        let (
            victim_pos,
            victim_z,
            victim_layer,
            victim_sector,
            victim_move_box,
            victim_dir,
            frames,
        ) = {
            let victim = self
                .get_entity(victim_id)
                .unwrap_or_else(|| panic!("falling-hit victim {victim_id:?} is missing"));
            let sprite_anim = crate::engine::animation::sprite_anim_for_order(
                victim.sprite(),
                anim,
                victim.is_pc(),
            );
            let ticks = victim
                .sprite()
                .ready_for_takeoff_ticks_for_anim(sprite_anim);
            (
                victim.element_data().position_map(),
                victim.position_iface().get_elevation(),
                victim.element_data().layer(),
                victim.element_data().sector(),
                *victim.position_iface().get_move_box(),
                victim.element_data().direction(),
                if ticks > 1 { ticks } else { 8 },
            )
        };

        // A charging rider throws along its facing. Every other antagonist
        // throws radially away from its live position.
        let charging_rider_dir =
            attacker_id
                .and_then(|id| self.get_entity(id))
                .and_then(|attacker| {
                    let rider = attacker
                        .soldier_data()
                        .map(|soldier| soldier.rider)
                        .unwrap_or(false);
                    let charging = attacker
                        .actor_data()
                        .is_some_and(|actor| actor.active_rider_charge.is_some());
                    (rider && charging).then_some(attacker.element_data().direction() as u16)
                });
        let attacker_pos = attacker_id
            .and_then(|id| self.get_entity(id))
            .map(|attacker| attacker.element_data().position_map());
        let (unit_x, unit_y) = if let Some(rider_dir) = charging_rider_dir {
            sector_to_vector_iso(rider_dir, ASPECT_RATIO)
        } else if let Some(attacker_pos) = attacker_pos {
            let dx = victim_pos.x - attacker_pos.x;
            let dy = victim_pos.y - attacker_pos.y;
            let distance = (dx * dx + dy * dy).sqrt();
            if distance < 0.01 {
                sector_to_vector_iso((victim_dir as u16 + 8) % 16, ASPECT_RATIO)
            } else {
                (dx / distance, dy / distance)
            }
        } else {
            sector_to_vector_iso((victim_dir as u16 + 8) % 16, ASPECT_RATIO)
        };
        let flight_x = unit_x * 30.0;
        let flight_y = unit_y * 30.0;

        let candidate = |fraction: f32| {
            crate::coordinates::MapPoint::new(
                victim_pos.x + flight_x * fraction,
                victim_pos.y + flight_y * fraction,
            )
        };
        let authorized = |point| {
            self.world.fast_grid.is_straight_movement_authorized(
                victim_pos,
                point,
                victim_layer,
                &victim_move_box,
            )
        };
        let chosen = if authorized(candidate(1.0)) {
            Some(candidate(1.0))
        } else if authorized(candidate(0.5)) {
            Some(if authorized(candidate(0.75)) {
                candidate(0.75)
            } else {
                candidate(0.5)
            })
        } else if authorized(candidate(0.25)) {
            Some(candidate(0.25))
        } else {
            None
        };
        let chosen = chosen.filter(|point| {
            victim_sector
                .and_then(|sector| {
                    self.world
                        .fast_grid
                        .level
                        .sector_number_map
                        .get(&crate::sector::SectorNumber::new(u16::from(sector) as i16))
                        .copied()
                })
                .and_then(|index| self.world.fast_grid.level.sectors.get(index))
                .map(|sector| sector.contains_point(*point))
                .unwrap_or(true)
        });
        let goal = chosen.unwrap_or(victim_pos);
        let goal_x = goal.x;
        let goal_y = goal.y;
        let (goal_obstacle, goal_z) = match victim_sector {
            Some(sector) => {
                match self.get_projection_area_index(assets, sector.get(), victim_layer, goal) {
                    Some(obstacle_index) => {
                        let z = self
                            .sight_obstacles(assets)
                            .get(obstacle_index as usize)
                            .unwrap_or_else(|| {
                                panic!(
                                    "falling-hit goal references missing obstacle {obstacle_index}"
                                )
                            })
                            .compute_top_z_from_projection(goal_x, goal_y);
                        (
                            crate::position_interface::ObstacleHandle::new(obstacle_index),
                            z,
                        )
                    }
                    None => (None, 0.0),
                }
            }
            None => (None, 0.0),
        };

        // ReadyForTakeOff installs the endpoint obstacle immediately.
        self.set_obstacle_and_material(
            assets,
            victim_id,
            goal_obstacle.map(|obstacle| obstacle.get()),
        );

        let flight_sector = crate::position_interface::vector_to_sector_0_to_15(flight_x, flight_y);
        let facing_sector = (flight_sector + 8) % 16;
        let victim = self
            .world
            .entities
            .get_mut(victim_id)
            .unwrap_or_else(|| panic!("falling-hit victim {victim_id:?} vanished"));
        victim.position_iface_mut().set_direction_instantly(
            crate::position_interface::Direction::from_raw(facing_sector as i32),
        );
        let dx = goal_x - victim_pos.x;
        let dy = goal_y - victim_pos.y;
        let dz = goal_z - victim_z;
        if dx.abs() > 0.01 || dy.abs() > 0.01 || dz.abs() > 0.01 {
            victim
                .actor_data_mut()
                .expect("falling-hit victim lost actor data")
                .active_flight = Some(crate::element::ActiveFlight {
                geometry: crate::element::FlightGeometry::World3d,
                increment_x: dx / frames as f32,
                increment_y: dy / frames as f32,
                goal_x,
                goal_y,
                frames_remaining: frames,
                antagonist: attacker_id,
                increment_z: dz / frames as f32,
                goal_z,
                goal_layer: victim_layer,
                goal_sector: victim_sector,
                obstacle: goal_obstacle,
            });
        } else {
            // TODO: Verify whether a completely blocked zero-distance
            // ReadyForTakeOff retains a zero-increment flight object.
            tracing::debug!(
                ?victim_id,
                "falling-hit flight has no authorized displacement"
            );
        }
    }

    /// Per-frame net-capture execute handler.
    ///
    /// Counter increment is **not** done here —
    /// `EngineInner::apply_net_falling_effect` already bumped it
    /// eagerly on capture.  This handler only runs the next-frame
    /// work: posture snap, detectable broadcast, and AI stimulus
    /// dispatch.
    ///
    /// Two guards apply:
    /// 1. Skip entirely if already netted (re-execution from a
    ///    duplicate element would otherwise double-broadcast).
    /// 2. Skip the posture snap + AI work when the victim is in a
    ///    posture that can't transition to StuckUnderNet (tied, KO,
    ///    dead).  Counter still tracks though, so the same victim
    ///    netted while tied gets released correctly on un-apply.
    pub(super) fn apply_net(&mut self, victim_id: EntityId) {
        let (already_stuck, can_transition) = match self.get_entity(victim_id) {
            Some(e) => {
                let posture = e.element_data().posture;
                let already_stuck = posture == Posture::StuckUnderNet;
                let unconscious = e.human_data().is_some_and(|h| h.unconscious);
                let dead = e.is_dead();
                let can_transition = posture != Posture::Tied && !unconscious && !dead;
                (already_stuck, can_transition)
            }
            None => return,
        };
        if already_stuck || !can_transition {
            return;
        }

        // SetStates(StuckUnderNet, Waiting).
        if let Some(entity) = self.world.entities.get_mut(victim_id) {
            entity.set_posture_stuck_under_net_for_human();
        }

        // Netted NPCs broadcast as DetectableType::Body to every NPC
        // *immediately* (not deferred via inform_my_friends) and
        // dispatch EventNet to their own AI so they transition to
        // Substate::WonderingUnderNet.
        let victim_is_npc = self
            .get_entity(victim_id)
            .map(|e| e.is_npc())
            .unwrap_or(false);
        if victim_is_npc {
            self.broadcast_body_detectable(victim_id);
            self.dispatch_ai_stimulus(
                victim_id,
                crate::ai::Stimulus::new(crate::ai::StimulusType::EventNet),
            );
        }
    }

    // ─── Post-damage state transitions ──────────────────────────────

    /// Handle death and knockout transitions after damage is applied.
    ///
    /// Checks the entity's life points and concussion, applying the
    /// appropriate state change:
    /// - Life points <= 0 → death (posture Dead, quit swordfight)
    /// - Concussion >= threshold → knockout (unconscious, posture Lying)
    pub(super) fn handle_post_damage(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        attacker_id: Option<EntityId>,
        no_damage: bool,
        damage_element: (crate::sequence::SequenceId, usize),
        dying_anim_override: Option<crate::order::OrderType>,
    ) {
        if no_damage {
            return;
        }

        // Read state without holding a borrow on self
        let (life_points, is_unconscious, is_pc, in_coma) = {
            let victim = match self.world.entities.get(victim_id) {
                Some(e) => e,
                None => return,
            };
            (
                get_life_points(victim),
                victim.human_data().map(|h| h.unconscious).unwrap_or(false),
                victim.kind().is_pc(),
                victim
                    .pc_data()
                    .and_then(|pc| self.pc_description_for_pc_data(pc))
                    .map(|desc| desc.status.in_coma)
                    .unwrap_or(false),
            )
        };
        let is_dead = life_points <= 0;

        // Outer-gate: if the PC is already in coma, the whole
        // coma-save/parent-wounded tree is skipped — a comatose PC is
        // unkillable by further damage.  The subtraction is applied
        // upstream of this function, so short-circuit the
        // death/knockout branches here to preserve the
        // no-op-on-already-comatose semantics.
        if is_pc && in_coma {
            return;
        }

        if is_dead {
            // Check for PC coma save before death
            let saved = if is_pc {
                self.try_pc_coma_save(sim, assets, victim_id, life_points.unsigned_abs())
            } else {
                false
            };
            if !saved {
                self.handle_death_with_damage_element(
                    sim,
                    assets,
                    victim_id,
                    damage_element,
                    dying_anim_override,
                );
            }
        } else if is_unconscious {
            // `inform_my_friends` only fires when the attacker is a
            // PC.  Resolve the attacker identity here so
            // `handle_knockout` can gate the broadcast.
            let attacker_is_pc = attacker_id
                .and_then(|id| self.world.entities.get(id))
                .map(|e| e.kind().is_pc())
                .unwrap_or(false);
            self.handle_knockout(sim, assets, victim_id, damage_element, attacker_is_pc);
        }
    }

    /// True if `victim_id` is a civilian carrying an unrevealed beggar
    /// scroll.  A beggar mid-reveal is immune to wound / concussion
    /// damage.  Callers use this to short-circuit damage entry points.
    pub(crate) fn is_scroll_protected_civilian(&self, victim_id: EntityId) -> bool {
        match self.get_entity(victim_id) {
            Some(Entity::Civilian(c)) => c.npc.attached_scroll.is_some(),
            _ => false,
        }
    }

    /// Handle entity death.
    ///
    /// Sets posture to Dead, quits swordfight, closes eyes for NPCs,
    /// and flags the entity as dead for the game state checks.
    pub(crate) fn handle_death(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
    ) {
        // Scripted-death entry (e.g. natives `HandleDeath` cheat).
        // Launch a synthetic `ReceiveDamage` element targeting the
        // victim with full life points and dispatch it synchronously
        // — this routes through `dispatch_receive_damage` →
        // `apply_generic_damage` → `handle_post_damage` → death path,
        // pushing the dying order onto the element so it participates
        // in the normal actor/sequence lifecycle.
        let life_points = self.get_entity(victim_id).map(get_life_points).unwrap_or(0);
        let lethal_damage = if life_points > 0 {
            life_points as u16
        } else {
            1
        };
        let elem = crate::sequence::SequenceElement::new_damage(
            1,
            crate::element::Command::ReceiveDamage,
            Some(victim_id),
            None,
            lethal_damage,
            0,
        );
        let seq_id = self.launch_element(elem);
        let elem_idx = 0;
        if !self.arbitrate_instruct(seq_id, elem_idx) {
            return;
        }
        self.dispatch_receive_damage(sim, assets, victim_id, seq_id, elem_idx);
    }

    /// Register a projectile damage sequence for the sequence-manager phase.
    ///
    /// Original `RHElementArrow::HitHuman` and `RHElementStone::HitHuman`
    /// call `LaunchSequenceElement` from the projectile's entity-Hourglass
    /// slot. That only appends the damage element to
    /// `mlistSequenceElementsToGo`; `RHSequenceManager::Hourglass` instructs
    /// it after every entity has run. In particular, an arrow dispatches the
    /// victim's `EVENT_GET_ARROW` between registration and damage.
    pub(crate) fn queue_projectile_damage(
        &mut self,
        victim_id: EntityId,
        shooter_id: EntityId,
        command: crate::element::Command,
        damage: u16,
        concussion: u16,
        projectile_id: Option<EntityId>,
    ) {
        debug_assert!(matches!(
            command,
            crate::element::Command::ReceiveArrowDamage
                | crate::element::Command::ReceiveStoneDamage
        ));

        let mut elem = crate::sequence::SequenceElement::new_damage(
            1,
            command,
            Some(victim_id),
            Some(shooter_id),
            damage,
            concussion,
        );
        let crate::sequence::SequenceElementData::Damage { projectile, .. } = &mut elem.data else {
            unreachable!("new_damage must create damage element data");
        };
        *projectile = projectile_id;
        self.register_owned_element_deferred(elem);
    }

    /// Internal entry point for `handle_death` that accepts the active
    /// receive-damage element so the dying animation chains via
    /// `do_next_order` instead of `combat_anim`.  The death path
    /// inserts `animDyingForward` onto the current sequence element.
    /// Once DYING terminates, the damage element terminates too; the
    /// actor's next Hourglass installs its ordinary `Wait`, whose
    /// dead-posture translation selects the matching `BEING_DEAD_*`
    /// corpse hold. Keeping that hold inside the damage element would
    /// incorrectly expose `Receive*Damage` as the corpse's permanent
    /// logical command.
    ///
    /// Remove `subject` from every other NPC's `detectable_lists[kind]`.
    /// `nets.rs::delete_body_detectable_for_all_npc` is the
    /// `DetectableType::Body` inline version; this is the general
    /// helper the death path uses for `Friend` / `MissedFriend`.
    pub(super) fn delete_detectable_for_all_npc(
        &mut self,
        subject: EntityId,
        kind: crate::element::DetectableType,
    ) {
        let det_idx = kind as usize;
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for npc_id in npc_ids {
            if npc_id == subject {
                continue;
            }
            if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(npc_id)
                && det_idx < s.npc.detectable_lists.len()
            {
                s.npc.detectable_lists[det_idx].retain(|d| d.element != Some(subject));
            } else if let Some(Entity::Civilian(c)) = self.world.entities.get_mut(npc_id)
                && det_idx < c.npc.detectable_lists.len()
            {
                c.npc.detectable_lists[det_idx].retain(|d| d.element != Some(subject));
            }
        }
    }

    /// PC-only portion of the kill cascade.
    ///
    /// Called from every PC death site so the cascade runs once per
    /// kill regardless of which damage path triggered it:
    /// - Gate `dead_pc = victim` on `is_vip && amulets == 0` (the
    ///   `MSG_CHARACTER_KILLED` handler only sets `dead_pc` when the
    ///   victim is a VIP, combined with the `!is_vip || amulets == 0`
    ///   guard the net condition is `is_vip && amulets == 0`).
    /// - When `!is_vip || amulets == 0`, drop the PC from the gang.
    /// - When `!is_vip` and a peasant replacement exists, enable the
    ///   trumpet portrait and bump the killed-peasant mission stat.
    /// - Always: decrement the new-PC mission stat.
    /// - Always: burn the three macro slots belonging to the dead PC.
    pub(super) fn apply_pc_kill_cascade(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
    ) {
        let pc_info = self
            .world
            .entities
            .get(victim_id)
            .and_then(|e| e.pc_data())
            .map(|pc| pc.profile_index);
        let Some(profile_idx) = pc_info else {
            return;
        };
        let (is_vip, profile_name) = Some(&self.mission_domain.campaign)
            .and(assets.profile_manager.get_character(profile_idx))
            .map(|cp| (cp.vip, cp.profile_name.clone()))
            .unwrap_or((false, String::new()));
        let amulets = Some(&self.mission_domain.campaign)
            .map(|c| c.values[crate::campaign::CampaignValue::Amulets])
            .unwrap_or(0);
        let char_idx = Some(&self.mission_domain.campaign)
            .and_then(|c| c.get_character_by_profile(profile_idx));

        // `!is_vip || amulets == 0` forwards the kill message and
        // gang removal.  The dead-PC slot only latches when the
        // victim is a VIP — net effect: `dead_pc = victim` iff
        // `is_vip && amulets == 0`.
        if !is_vip || amulets == 0 {
            if let (Some(idx), Some(c)) = (char_idx, Some(&mut self.mission_domain.campaign)) {
                c.remove_from_gang(idx);
            }
            if is_vip {
                self.mission_domain.dead_pc = Some(victim_id);
            }
        }

        // Peasant trumpet + killed-peasant stat.
        if !is_vip {
            let has_replacement = Some(&self.mission_domain.campaign)
                .and_then(|c| {
                    c.get_random_peasant_from_gang(sim, Some(profile_idx), &assets.profile_manager)
                })
                .is_some();
            if has_replacement
                && let Some(entity) = self.world.entities.get_mut(victim_id)
                && let Some(pc) = entity.pc_data_mut()
            {
                pc.trumpet_enabled = true;
            }
            self.mission_domain.mission_stat.add_killed_peasant();
        }

        // Unconditional new-PC mission-stat decrement.
        if !profile_name.is_empty() {
            self.mission_domain
                .mission_stat
                .remove_new_pc(&profile_name);
        }

        // Burn the three macro slots belonging to the dead PC.  We
        // dispatch directly into `abort_quick_action` rather than
        // enqueuing `PlayerCommand::DeleteMacro` to keep the cascade
        // synchronous with the kill (the player-command queue would
        // defer by a frame).  The single-PC path skips the macro
        // tetris fold-up that the all-PCs broadcast path performs.
        for slot in 0..=2u8 {
            self.abort_quick_action(victim_id, slot);
        }
        if let Some(entity) = self.world.entities.get_mut(victim_id)
            && let Some(pc) = entity.pc_data_mut()
        {
            pc.portrait.burned = true;
            pc.portrait.open = false;
        }
    }

    /// Apply the reciprocal/global relationship work that the original
    /// `SetState(SLEEPING, SLEEPING_FOREVER)` performs synchronously.
    ///
    /// Normal AI transitions defer these writes through the actor outbox,
    /// but death clears that entire outbox to prevent pre-death work from
    /// reaching the corpse.  Extract both the current relationship and any
    /// already-queued old relationship before that reset so teardown cannot
    /// leave a PC or archery reservation pointing at the dead soldier.
    fn detach_npc_death_relationships(&mut self, victim_id: EntityId) {
        let (guarded_pcs, shooting_points, archery_sector) = {
            let Some(Entity::Soldier(soldier)) = self.world.entities.get_mut(victim_id) else {
                return;
            };
            let Some(enemy) = soldier.npc.ai_brain.enemy_mut() else {
                return;
            };

            let guard_effect = enemy.base.outbox.actor.set_guarded_pc.take();
            let mut guarded_pcs = Vec::new();
            for guarded_pc in [
                enemy.guarded_pc.take(),
                guard_effect.and_then(|effect| effect.old),
                guard_effect.and_then(|effect| effect.new),
            ]
            .into_iter()
            .flatten()
            {
                if !guarded_pcs.contains(&guarded_pc) {
                    guarded_pcs.push(guarded_pc);
                }
            }

            let release = enemy.base.outbox.actor.take_archery_reservation_release();
            let mut shooting_points = Vec::new();
            for shooting_point in [
                enemy.my_shooting_point.take().map(Into::into),
                release.shooting_point,
            ]
            .into_iter()
            .flatten()
            {
                if !shooting_points.contains(&shooting_point) {
                    shooting_points.push(shooting_point);
                }
            }

            let archery_sector = enemy.my_archery_sector.take();
            debug_assert!(
                !release.release_sector || archery_sector.is_some(),
                "queued archery-sector release has no owned sector on death"
            );

            (guarded_pcs, shooting_points, archery_sector)
        };

        for guarded_pc in guarded_pcs {
            let guarded_pc_id = EntityId::Pc(guarded_pc);
            match self.world.entities.get_mut(guarded_pc_id) {
                Some(Entity::Pc(pc)) if pc.pc.guard == Some(victim_id) => {
                    pc.pc.guard = None;
                }
                Some(Entity::Pc(_)) => {}
                Some(_) => unreachable!("typed PcId resolved to a non-PC entity"),
                None => tracing::warn!(
                    ?victim_id,
                    ?guarded_pc_id,
                    "dead soldier's guarded PC no longer exists"
                ),
            }
        }

        for shooting_point in shooting_points {
            let Some(sector) = self
                .ai
                .global
                .archery_sectors
                .get_mut(shooting_point.sector_index as usize)
            else {
                tracing::warn!(
                    ?victim_id,
                    sector = shooting_point.sector_index,
                    point = usize::from(shooting_point.point_index),
                    "dead soldier's shooting-point sector no longer exists"
                );
                continue;
            };
            let Some(point) = sector
                .points
                .get_mut(usize::from(shooting_point.point_index))
            else {
                tracing::warn!(
                    ?victim_id,
                    sector = shooting_point.sector_index,
                    point = usize::from(shooting_point.point_index),
                    "dead soldier's shooting point no longer exists"
                );
                continue;
            };
            if point.owner == Some(victim_id) {
                point.owner = None;
            } else if let Some(owner) = point.owner {
                tracing::warn!(
                    ?victim_id,
                    ?owner,
                    sector = shooting_point.sector_index,
                    point = usize::from(shooting_point.point_index),
                    "dead soldier's shooting point is owned by another entity"
                );
            }
        }

        if let Some(sector_index) = archery_sector {
            let Some(sector) = self
                .ai
                .global
                .archery_sectors
                .get_mut(sector_index as usize)
            else {
                tracing::warn!(
                    ?victim_id,
                    sector = sector_index,
                    "dead soldier's archery sector no longer exists"
                );
                return;
            };
            sector.decrement_owner_counter();
        }
    }

    pub(crate) fn handle_death_with_damage_element(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        damage_element: (crate::sequence::SequenceId, usize),
        dying_anim_override: Option<crate::order::OrderType>,
    ) {
        tracing::info!(entity = ?victim_id, "Entity died");

        // Select dying animation (None when posture is already
        // Lying/Dead/Carried — the "already on the ground" case).
        // When the caller supplied an override (sword strike with a
        // positive stunning effect against a non-rider), use it in
        // place of the default `dying_forward`; the override is
        // itself None when the selector returns None for the
        // posture.
        let dying_anim = self.get_entity(victim_id).and_then(|e| {
            let posture = e.element_data().posture;
            let action = e.actor_data().map(|a| a.action_state).unwrap_or_default();
            select_combat_animations(posture, action)
                .map(|a| dying_anim_override.unwrap_or(a.dying_forward))
        });

        if let Some(anim) = dying_anim {
            self.queue_damage_anim(victim_id, damage_element, anim);
        }

        // Read the killer from the damage element so we can set the
        // `inform_my_friends` flag below (true when the killer is a
        // PC).  Done before `kill_owner_sequences` so we still have
        // the damage-element data handy.
        let killer_is_pc = self
            .orders
            .sequence_manager
            .get_element(damage_element.0, damage_element.1)
            .and_then(|e| match &e.data {
                crate::sequence::SequenceElementData::Damage { origin, .. } => *origin,
                _ => None,
            })
            .and_then(|k| self.world.entities.get(k))
            .map(|e| e.is_pc())
            .unwrap_or(false);

        // Throw away every sequence element the victim owns except the
        // damage sequence (which just had its dying order queued).
        // The general-purpose `stop_owner` path is
        // wrong for death — it calls `stop_movement_for_owner` which
        // rewrites a walking order to a `TransitionWalking*Waiting*`
        // stop-animation and lets the movement element keep playing,
        // producing a "corpse walks a few more frames" visual.  We want
        // a hard interrupt instead, so the only InProgress element
        // `current_element_for_actor` finds is the damage element, and
        // its `DyingSword` order becomes the actor's current order.
        self.orders
            .sequence_manager
            .kill_owner_sequences(victim_id, damage_element.0);

        // Remove the dying soldier from every other NPC's
        // friend/missed-friend tracker so they don't keep looking for
        // him after he's on the ground.
        self.delete_detectable_for_all_npc(victim_id, crate::element::DetectableType::Friend);
        self.delete_detectable_for_all_npc(victim_id, crate::element::DetectableType::MissedFriend);

        // The original SetState(SLEEPING_FOREVER) immediately clears the
        // guarded-PC reciprocal pointer and archery ownership.  Apply those
        // invariants before the broad outbox reset below discards stale work.
        self.detach_npc_death_relationships(victim_id);

        let victim = match self.world.entities.get_mut(victim_id) {
            Some(e) => e,
            None => return,
        };

        // Clear movement-side state on the actor so no stale path or
        // active-movement handle is left pointing at the torn-down
        // walk sequence.  We intentionally do NOT set `posture` or
        // `action_state` here — the dying animation's
        // `apply_dying_start_side_effect` (in `animation.rs`) sets
        // them when the anim starts.  Setting posture=Dead eagerly
        // makes the sprite snap to the corpse pose before the dying
        // transition plays, skipping the visible animation.
        if let Some(actor) = victim.actor_data_mut() {
            actor.active_movement.clear();
            actor.clear_path();
        }

        // NPC kill cascade: clear stale pre-death work, then enqueue the
        // death-owned alert/music transition and snap the terminal state.
        if let Some(ai) = victim.ai_controller_mut() {
            // Drop every remaining AI intent queued by the think that ran
            // earlier in this tick.  The relationship-maintenance effects
            // were applied synchronously above; all other channels must be
            // cauterised before death adds its own instant-music effect.
            ai.clear_all_pending();
            ai.set_alert_status_with_flags(
                crate::ai::AlertLevel::Green,
                crate::ai::AlertFlags::INSTANT_MUSIC_CHANGE,
                false,
            );
            ai.current_state = crate::ai::AiState::Sleeping;
            ai.current_substate = crate::ai::Substate::SleepingForever;
            ai.clear_emoticon();
        }

        if let Some(npc) = victim.npc_data_mut() {
            // Close eyes if not already closed — the guard prevents
            // re-triggering the eye-shut animation on an NPC killed
            // while already sleeping (e.g. assassinated in his cot).
            if npc.eye_status != EyeStatus::Closed {
                crate::ai_vision::set_view_status(npc, EyeStatus::DieOrGetUnconscious);
            }
            npc.alerted = false;
            // True when the killer is a PC. The flag is cleared when
            // `tick_inform_my_friends_for_npc` consumes it at this
            // victim's next owner slot.
            npc.inform_my_friends = killer_is_pc;
            if let Some(ai) = npc.ai_brain.base_mut() {
                ai.knocked_out_in_money_fight = false;
            }
        }

        // PC-only kill cascade — see `apply_pc_kill_cascade`.
        let is_pc = victim.kind().is_pc();
        if is_pc {
            self.apply_pc_kill_cascade(sim, assets, victim_id);
        }

        // Clear concussion / unconscious state and drop any
        // unconscious-stars titbit for this entity.  Without this a
        // KO'd human killed mid-stars would leave the titbit orphaned
        // and the dead body would serialize with residual concussion
        // / KO flags.
        //
        // Read the live unconscious flag *before* zeroing it below
        // so the predicate reflects actual entity state — the
        // unconscious-stars cleanup runs while the flag is still
        // true; the per-frame update reaps the titbit once the flag
        // turns false a tick later.
        let still_unconscious = self
            .world
            .entities
            .get(victim_id)
            .and_then(|e| e.human_data())
            .is_some_and(|h| h.unconscious);
        self.feedback.titbit_manager.remove_unconscious_stars_if(
            crate::titbit::ElementHandle(victim_id.index()),
            still_unconscious,
        );
        if let Some(entity) = self.world.entities.get_mut(victim_id)
            && let Some(human) = entity.human_data_mut()
        {
            human.unconscious = false;
            human.concussion_of_the_brain = 0;
            human.concussion_healing_timeout = 0;
        }

        // Quit swordfight (removes from all opponents' lists)
        self.quit_swordfight(sim, assets, victim_id);

        // Queue roll if on a slope.
        self.try_queue_roll(assets, victim_id, damage_element);

        // Mission-stat bump for Royalist soldier deaths.
        let bump_killed_allied = self
            .get_entity(victim_id)
            .map(|e| e.is_soldier() && e.camp() == Camp::Royalists)
            .unwrap_or(false);
        if bump_killed_allied {
            self.mission_domain.mission_stat.add_killed_allied();
        }

        // Campaign score bump for Lacklandist soldier deaths during
        // a sword/generic-damage interaction.  Projectile kills are
        // finalized through this same death path for visuals, but keep
        // the old score scoping and only award bow XP at the projectile
        // call site.
        const SCORE_SOLDIER_KILLED_DURING_FIGHT: i32 = 50;
        let projectile_death = self
            .orders
            .sequence_manager
            .get_element(damage_element.0, damage_element.1)
            .map(|e| {
                matches!(
                    e.command,
                    crate::element::Command::ReceiveArrowDamage
                        | crate::element::Command::ReceiveStoneDamage
                )
            })
            .unwrap_or(false);
        let bump_lacklandist_score = self
            .get_entity(victim_id)
            .map(|e| e.is_soldier() && e.camp() == Camp::Lacklandists)
            .unwrap_or(false);
        if bump_lacklandist_score
            && !projectile_death
            && let Some(campaign) = Some(&mut self.mission_domain.campaign)
        {
            campaign.add_value(
                crate::campaign::CampaignValue::Score,
                SCORE_SOLDIER_KILLED_DURING_FIGHT,
            );
        }
    }

    /// Handle entity knockout (went unconscious from concussion).
    ///
    /// Sets posture to Lying, quits swordfight, closes eyes for NPCs.
    pub(super) fn handle_knockout(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        damage_element: (crate::sequence::SequenceId, usize),
        attacker_is_pc: bool,
    ) {
        let concussion = self
            .get_entity(victim_id)
            .and_then(|e| e.human_data())
            .map(|h| h.concussion_of_the_brain)
            .unwrap_or(0);

        tracing::info!(entity = ?victim_id, concussion, "Entity knocked out");

        // Select falling-back animation.  The animation is inserted
        // as the next order on the active damage element, with the
        // element's priority lifted to NonInterruptable.
        let falling_anim = self
            .get_entity(victim_id)
            .and_then(|e| {
                let posture = e.element_data().posture;
                let action = e.actor_data().map(|a| a.action_state).unwrap_or_default();
                select_combat_animations(posture, action)
            })
            .map(|a| a.falling_back);
        if let Some(anim) = falling_anim {
            tracing::trace!(
                ?victim_id,
                ?anim,
                "handle_knockout: queuing falling animation"
            );
            self.queue_damage_anim(victim_id, damage_element, anim);
        } else {
            tracing::warn!(
                ?victim_id,
                "handle_knockout: no falling_anim selected — entity will snap to ground without animation"
            );
        }

        // If a falling_back animation is going to play, leave the
        // posture where it is — the animation-completion handler in
        // tick_actor_animation_for will set Posture::Lying when the anim
        // terminates.  Setting Lying now would snap the
        // unconscious-star titbit to the crawling-offset position
        // while the sprite is still visually standing through the
        // falling animation, producing a floating-above-nothing
        // star.  If no falling animation was selected (e.g. entity
        // was in a posture that doesn't map to one), fall back to
        // the original immediate-lying behavior so downstream code
        // that assumes Lying for unconscious humans still works.
        self.apply_knockout_side_effects(
            sim,
            assets,
            victim_id,
            attacker_is_pc,
            falling_anim.is_none(),
        );

        // Queue roll if on a slope.
        self.try_queue_roll(assets, victim_id, damage_element);
    }

    pub(super) fn apply_knockout_side_effects(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        attacker_is_pc: bool,
        set_lying_now: bool,
    ) {
        let healing_speed = concussion_healing_speed_for_entity(
            self.get_entity(victim_id)
                .expect("knockout victim disappeared before healing-timeout setup"),
            &assets.profile_manager,
        );

        // Quit swordfight (removes from all opponents' lists).
        self.quit_swordfight(sim, assets, victim_id);

        // Add unconscious star titbit (event-driven creation).
        self.add_unconscious_star(victim_id);

        let victim = self
            .world
            .entities
            .get_mut(victim_id)
            .expect("knockout victim disappeared during side effects");
        let human = victim
            .human_data_mut()
            .expect("knockout victim lost required HumanData");
        if human.concussion_healing_timeout == 0 {
            human.concussion_healing_timeout = healing_speed;
        }
        if let Some(npc) = victim.npc_data_mut() {
            // The NPC override performs this after the base-human
            // QuitSwordFight/titbit/healing work and before Think.
            npc.clear_all_suspects();
        }

        // RHElementActorNPC::SetConcussionOfTheBrain calls Think inline
        // after the base-human relationship/titbit work.
        if matches!(
            self.world.entities.get(victim_id),
            Some(Entity::Soldier(_) | Entity::Civilian(_))
        ) {
            self.dispatch_synchronous_ai_think_preserving_detection_fifo(
                sim,
                victim_id,
                assets,
                crate::ai::Stimulus::new(crate::ai::StimulusType::EventLoseConsciousness),
            );
            // StartThink handles EVENT_LOSE_CONSCIOUSNESS by calling
            // SetViewStatus(EYES_DIE_OR_GET_UNCONSCIOUS) inline.  Rust's AI
            // borrow boundary represents that write in the recovery outbox,
            // so consume it before returning from the same synchronous Think.
            self.tick_ai_pending_resurrection_and_eyes_for_npc(victim_id);
        }

        let victim = self
            .world
            .entities
            .get_mut(victim_id)
            .expect("knockout victim disappeared after AI callback");
        if let Some(npc) = victim.npc_data_mut() {
            // The Original assigns this only after EVENT_LOSE_CONSCIOUSNESS
            // returns.
            npc.inform_my_friends = attacker_is_pc;
        }
        if set_lying_now && !victim.element_data().posture.is_lying() {
            victim.set_posture(Posture::Lying);
        }
    }
}
