//! Damage application, death handling, and knockout effects.
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;

#[inline]
fn provoke_roll_succeeds(roll: u32, fighting_ability: u16) -> bool {
    (roll as f32) < 0.2_f32 * f32::from(fighting_ability)
}

fn good_strike_lifecycle_debug_enabled() -> bool {
    std::env::var_os("PARITY_DEBUG_GOOD_STRIKE_LIFECYCLE").is_some()
}

fn good_strike_lifecycle_debug_matches(frame: u32, creation_order: u32) -> bool {
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for GOOD_STRIKE diagnostic: {error}")
            })
        })
    };
    parse_filter("PARITY_DEBUG_GOOD_STRIKE_FRAME").is_none_or(|expected| expected == frame)
        && parse_filter("PARITY_DEBUG_GOOD_STRIKE_CREATION_ORDER")
            .is_none_or(|expected| expected == creation_order)
}

/// Match `SBGeoVector2D::operator*=(30.0 / vtFlight.Norm())` for FallingHit's
/// sector vectors and live antagonist deltas. The vector's squared sum is
/// stored as `GEOTYPE` (`f32`), while `sqrt` and the unsuffixed `30.0`
/// division execute as doubles before the one shared scale is stored back to
/// `f32` and applied to both components.
fn scale_falling_hit_sector_vector(x: f32, y: f32) -> (f32, f32) {
    let squared_norm = x * x + y * y;
    let distance = f64::from(squared_norm).sqrt() as f32;
    let scale = (30.0_f64 / f64::from(distance)) as f32;
    (x * scale, y * scale)
}

/// Normalize the live antagonist-to-victim delta used by
/// `RHElementActorHuman::ExecuteFallingHit`.
///
/// The Original does not special-case coincident actors: multiplying its
/// zero vector by `30 / 0` produces unordered components, and
/// `SBGeoVector2D::GetSector0to15` consequently leaves every comparison bit
/// clear (sector 0). The observable result is a stationary flight initialized
/// facing sector 8. Keep the zero displacement explicitly instead of storing
/// NaNs in Rust state; the shared sector classifier still yields the same
/// facing. Non-zero deltas, including arbitrarily small ones, use the Original
/// normalization rather than an epsilon fallback.
fn normalize_falling_hit_antagonist_delta(x: f32, y: f32) -> (f32, f32) {
    if x == 0.0 && y == 0.0 {
        (0.0, 0.0)
    } else {
        scale_falling_hit_sector_vector(x, y)
    }
}

#[cfg(test)]
mod falling_hit_vector_tests {
    use super::normalize_falling_hit_antagonist_delta;

    #[test]
    fn coincident_hit_uses_original_zero_vector_facing() {
        let (x, y) = normalize_falling_hit_antagonist_delta(0.0, 0.0);
        assert_eq!((x.to_bits(), y.to_bits()), (0, 0));
        let flight_sector = crate::position_interface::vector_to_sector_0_to_15(x, y);
        assert_eq!(flight_sector, 0);
        assert_eq!((flight_sector + 8) % 16, 8);
    }

    #[test]
    fn near_coincident_hit_still_normalizes_live_delta() {
        let (x, y) = normalize_falling_hit_antagonist_delta(0.001, 0.0);
        assert_eq!(x.to_bits(), 30.0_f32.to_bits());
        assert_eq!(y.to_bits(), 0.0_f32.to_bits());
    }
}

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

/// Original's RECEIVE_HIT_DAMAGE sends EVENT_GOTHIT only through the NPC
/// override. PCs share the human concussion and translation work but have no
/// AI controller to notify.
fn conscious_hit_notifies_ai(victim: &Entity) -> bool {
    victim.is_npc() && victim.human_data().is_some_and(|human| !human.unconscious)
}

#[cfg(test)]
mod conscious_hit_notification_tests {
    use super::*;
    use crate::element::{
        ActorData, ActorPc, ActorSoldier, ElementData, ElementKind, HumanData, NpcData, PcData,
        SoldierData,
    };

    #[test]
    fn conscious_hit_notifies_npc_but_not_pc() {
        let pc = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        assert!(!conscious_hit_notifies_ai(&pc));

        let mut soldier = Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        });
        assert!(conscious_hit_notifies_ai(&soldier));

        soldier.human_data_mut().unwrap().unconscious = true;
        assert!(!conscious_hit_notifies_ai(&soldier));
    }
}

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
        self.push_translated_damage_order(damage_element, anim);
    }

    /// Append an order authored by one of Original's human damage
    /// translators. TranslateDamage, TranslateArrowDamage,
    /// TranslateSwordDamage, TranslateHitDamage, and TranslatePushDamage
    /// explicitly set `RHOrder::bComputeDirection = false` on their direct
    /// reaction orders; those animations preserve the actor's facing rather
    /// than deriving it from the dummy `(0, 0)` point. Orders appended by the
    /// separate TranslateRoll helper deliberately retain the default `true`.
    pub(super) fn push_translated_damage_order(
        &mut self,
        damage_element: (crate::sequence::SequenceId, usize),
        anim: OrderType,
    ) {
        let (dseq, didx) = damage_element;
        let id = self.orders.allocate_order_id();
        let mut order = crate::order::Order::new(anim, 0.0, 0.0, id);
        order.compute_direction = false;
        self.orders
            .sequence_manager
            .push_order_on(dseq, didx, order);
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
        // Dead and unconscious victims are *not* filtered out here: the
        // sweep launchers hand every in-sector victim to the sequence
        // manager, and a dead / unconscious human still admits
        // `ReceiveSwordDamage` through its own instruction gate.  The
        // decision to decline belongs to arbitration, not to the
        // launcher.
        if self.get_entity(victim_id).is_none() {
            tracing::warn!(
                ?victim_id,
                ?attacker_id,
                ?sword_strike,
                "sword damage victim vanished before its element could be launched"
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
        self.trace_reactive_sword_topology("before_damage_registration", victim_id, None);
        // Resolve the value normally determined by Instruct, but bypass the
        // engine's ordinary owned-element launcher because that path performs
        // synchronous arbitration. The manager-tail InstructOwner action owns
        // arbitration, transition generation, and damage dispatch.
        self.resolve_element_priority(&mut elem);
        let sequence_id = self.orders.sequence_manager.launch_element(elem);
        self.trace_reactive_sword_topology(
            "after_damage_registration",
            victim_id,
            Some((sequence_id, 0)),
        );
        self.trace_sword_damage_lifecycle(
            "queued",
            victim_id,
            Some(attacker_id),
            Some(sword_strike),
            Some((sequence_id, 0)),
            None,
        );
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
        // A scroll-carrying civilian is not short-circuited here: the
        // immunity lives in the wounding/concussion primitives
        // (`ConcussionContext::scroll_attached`), so the protection
        // rolls and the rest of the pipeline still run.
        let strike = match sword_strike {
            Some(s) => s,
            None => {
                tracing::warn!(?victim_id, "apply_sword_damage: no strike type");
                return;
            }
        };

        // Ladder/wall victims are *not* short-circuited here: the
        // protection rolls happen first and only the hit-reaction
        // translation further down routes them to
        // `translate_ladder_wall_fall`.  Push strikes reach the same
        // helper through `apply_push_effect`.
        if super::strikes::sword_damage_debug_enabled() {
            eprintln!(
                "[SWORDDMG f={} victim={:?} (co {}) attacker={:?} (co {:?}) strike={:?}]",
                self.control.frame_counter,
                victim_id,
                self.world.original_creation_order(victim_id),
                attacker_id,
                attacker_id.map(|id| self.world.original_creation_order(id)),
                strike,
            );
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
            let atk = self.expect_entity(attacker, "apply_sword_damage attacker");
            let elem = atk.element_data();
            let (dir, elev) = (elem.direction(), elem.position().z);
            let ability = fighting_ability_from_profile(
                atk,
                &assets.profile_manager,
                sim.config().difficulty,
            );
            let is_rank = is_rank_soldier(atk, &assets.profile_manager);
            let def_to_atk = direction_to(&self.world.entities, victim_id, attacker);
            (dir, def_to_atk, elev, ability, is_rank)
        } else {
            // No attacker (scripted damage): zero elevation — for
            // un-sited sources the elevated-defender branch only fires
            // when the defender truly stands higher.
            (0, 0, 0.0, 50, false)
        };

        // Read defender context
        let victim = self.expect_entity(victim_id, "apply_sword_damage victim");
        let defender_dir = victim.element_data().direction();
        let defender_elevation = victim.element_data().position().z;
        let defender_action = victim
            .actor_data()
            .map(|a| a.action_state)
            .unwrap_or(ActionState::Waiting);
        let victim_was_unconscious = victim.human_data().is_some_and(|human| human.unconscious);
        self.trace_sword_damage_lifecycle(
            "apply-before",
            victim_id,
            attacker_id,
            Some(strike),
            Some(damage_element),
            None,
        );
        let life_points_before = get_life_points(victim);
        // `SetConcussionOfTheBrain` (RHelementactorhuman.cpp:456-505) only runs
        // the knock-out cascade from its `else` arm, i.e. when the victim was
        // still conscious before this hit. A victim that was already
        // unconscious never re-enters it.
        let unconscious_before = victim
            .human_data()
            .map(|human| human.unconscious)
            .unwrap_or(false);

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
        let (human, lp) = match victim.human_and_life_points_mut() {
            Some(pair) => pair,
            None => return,
        };

        let (result, cutting_inflicted) = combat::receive_sword_damage(sim, human, lp, &params);

        let raw_life_points_after = *lp;
        let coma_saved = self.close_pc_wounded_coma_boundary(
            sim,
            assets,
            victim_id,
            cutting_inflicted,
            life_points_before,
            raw_life_points_after,
        );
        let life_points_after = self
            .get_entity(victim_id)
            .map(get_life_points)
            .unwrap_or(raw_life_points_after);

        // Human::SetLifePoints invokes virtual Kill synchronously, before
        // ReceiveSwordDamage returns to TranslateSwordDamage.  In
        // particular, Kill clears an unconscious victim before the later
        // virtual SayOuch call, so a lethal hit says DIES rather than taking
        // SayOuch's unconscious early return.  Push strikes already close
        // this boundary below because their translator owns the sole falling
        // order; close it here for ordinary strikes while leaving the dying
        // animation/roll to the later translation phase.
        let push_strike = combat::strike_has_push_effect(&attacker_profile, strike);
        let fresh_lethal_ordinary =
            !push_strike && !coma_saved && life_points_before > 0 && life_points_after <= 0;
        if fresh_lethal_ordinary {
            let killer_is_pc = attacker_id
                .map(|id| {
                    self.expect_entity(id, "lethal sword attacker")
                        .kind()
                        .is_pc()
                })
                .unwrap_or(false);
            self.apply_nonvisual_death_cascade(
                sim,
                assets,
                victim_id,
                damage_element,
                killer_is_pc,
            );
        }
        let victim_went_unconscious = !victim_was_unconscious
            && self
                .expect_entity(victim_id, "sword-damage concussion victim")
                .human_data()
                .is_some_and(|human| human.unconscious);
        if victim_went_unconscious {
            let attacker_is_pc = attacker_id
                .map(|id| {
                    self.expect_entity(id, "sword-damage knockout attacker")
                        .kind()
                        .is_pc()
                })
                .unwrap_or(false);
            // `SetConcussionOfTheBrain` closes its KO side effects inline,
            // before control returns to `TranslateSwordDamage`: first
            // QuitSwordFight, stars/healing setup, then the NPC override.
            self.apply_knockout_side_effects(sim, assets, victim_id, attacker_is_pc, false);
        }
        // PC hurt/death speech belongs to the virtual SetLifePoints call
        // inside ReceiveSwordDamage. It uses the applied LP delta, before
        // TranslateSwordDamage later invokes the (PC no-op) SayOuch.
        if !coma_saved {
            self.pc_life_points_speech(assets, victim_id, life_points_before, life_points_after);
        }
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
                .expect_entity(victim_id, "apply_sword_damage impact sound")
                .element_data()
                .position_map();

            // Real weapon/armor materials from character/soldier profiles.
            // Scripted damage without an attacker keeps the default weapon
            // material; a live attacker id must resolve.
            let atk_weapon_mat = attacker_id
                .map(|id| {
                    weapon_material_from_profile(
                        self.expect_entity(id, "apply_sword_damage impact sound attacker"),
                        &assets.profile_manager,
                    )
                })
                .unwrap_or(crate::profiles::WeaponMaterial::SteelAndWood);
            let def_armor_mat = armor_material_from_profile(
                self.expect_entity(victim_id, "apply_sword_damage impact sound"),
                &assets.profile_manager,
            );

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
                let def_weapon_mat = weapon_material_from_profile(
                    self.expect_entity(victim_id, "apply_sword_damage parry sound"),
                    &assets.profile_manager,
                );
                self.feedback
                    .pending_side_effects
                    .sounds
                    .push(super::SoundCommand::StrikeFx {
                        strike_kind: parry_kind,
                        weapon1: atk_weapon_mat,
                        weapon2: def_weapon_mat,
                        position: victim_pos,
                    });

                // When the defender wasn't already in a parry state, push the
                // parry-to-waiting transition onto the damage element.
                if defender_action != ActionState::ParryingSword
                    && defender_action != ActionState::ParryingSwordLow
                {
                    self.push_translated_damage_order(
                        damage_element,
                        crate::order::OrderType::TransitionParryingSwordWaitingSword,
                    );
                }

                // Original tests push-strike kind before handling the
                // NO_DAMAGE_PARRIED result. PushAside, circle, and charge
                // strikes therefore still run TranslatePushDamage (including
                // its fall/flight and SayOuch) when parried. Only an ordinary
                // parried strike returns here with the transition above.
                if !combat::strike_has_push_effect(&attacker_profile, strike) {
                    self.trace_sword_damage_lifecycle(
                        "apply-after-parry",
                        victim_id,
                        attacker_id,
                        Some(strike),
                        Some(damage_element),
                        Some(result),
                    );
                    return;
                }
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

        // PC::TranslateSwordDamage / TranslatePushDamage tests the *live*
        // posture after virtual GetWounded and the impact-sound prefix have
        // returned. Usually that is still CarryingCorpse and the override
        // drops the body before falling through to Human translation. A
        // lethal hit saved by an amulet is different: PC::GetWounded
        // establishes the coma and calls SetPosture(LYING) inside that
        // virtual boundary, so the later PC posture switch takes its grounded
        // default arm and deliberately leaves the body attached. Checking
        // before the coma boundary made Rust eagerly drop the body one
        // callback too soon.
        if self
            .expect_entity(victim_id, "apply_sword_damage pre-translation victim")
            .element_data()
            .posture
            == Posture::CarryingCorpse
        {
            self.force_drop_carried_corpse_instant(victim_id);
        }

        // SetLifePoints invokes virtual Kill synchronously before
        // ReceiveSwordDamage returns to TranslatePushDamage. Apply that
        // nonvisual cascade first; the push translator below remains the sole
        // owner of the falling order.
        let fresh_lethal_push = push_strike
            && attacker_id.is_some()
            && !coma_saved
            && life_points_before > 0
            && life_points_after <= 0;
        if fresh_lethal_push {
            let killer_is_pc = attacker_id
                .map(|id| {
                    self.expect_entity(id, "lethal push attacker")
                        .kind()
                        .is_pc()
                })
                .unwrap_or(false);
            self.apply_nonvisual_death_cascade(
                sim,
                assets,
                victim_id,
                damage_element,
                killer_is_pc,
            );
        }
        // Human::SetLifePoints returns immediately when the victim was
        // already dead. Its earlier Kill call has therefore already owned
        // the one-shot death cascade; TranslatePushDamage may still author
        // the visual push response, but must not replay that cascade.
        let push_death_cascade_already_applied = fresh_lethal_push || life_points_before <= 0;

        // Handle push effect — the push path handles animations and skips the
        // regular hit animation / post-damage translation below.
        let pushed = if push_strike {
            let thrust = &attacker_profile.thrusts[strike as usize];
            if let Some(attacker) = attacker_id {
                let push_info = PushStrikeInfo {
                    repulsion: thrust.repulsion,
                };
                self.apply_push_effect(
                    sim,
                    assets,
                    victim_id,
                    attacker,
                    &push_info,
                    result,
                    damage_element,
                    push_death_cascade_already_applied,
                )
            } else {
                false
            }
        } else {
            false
        };

        // TranslateSwordDamage terminates immediately for a human who is
        // already down (including a VIP PC moved to Lying by an amulet save),
        // except for the Original's dead-rider fall-through. The element is
        // still Actor::Instruct's selected pointer at this point, so its
        // synchronous condolence makes Instruct return before the ordinary
        // mmotionState=IN_PROGRESS epilogue. This is observably different
        // from a true accepted-empty translation such as an already-held
        // parry, which publishes InProgress before detaching the element.
        let (grounded_translation_terminates, grounded_posture, is_rider) = if pushed
            || result.is_empty()
        {
            (false, Posture::Upright, false)
        } else {
            let victim = self.get_entity(victim_id).unwrap_or_else(|| {
                    panic!(
                        "ReceiveSwordDamage victim {victim_id:?} vanished before grounded-posture translation"
                    )
                });
            let posture = victim.element_data().posture;
            let is_rider = matches!(victim, Entity::Soldier(s) if s.soldier.rider);
            let dead_rider = is_rider && life_points_after <= 0;
            (
                matches!(
                    posture,
                    Posture::Lying
                        | Posture::StuckUnderNet
                        | Posture::Flying
                        | Posture::Carried
                        | Posture::OnShoulders
                        | Posture::Tied
                        | Posture::Dead
                        | Posture::DeadBack
                ) && !dead_rider,
                posture,
                is_rider,
            )
        };
        if grounded_translation_terminates {
            // TranslateSwordDamage changes a dead, grounded non-rider to the
            // canonical Dead posture before falling through to the common
            // terminating arm. This publication is observable even though no
            // replacement animation/order is authored.
            if life_points_after <= 0
                && !is_rider
                && matches!(
                    grounded_posture,
                    Posture::Lying
                        | Posture::StuckUnderNet
                        | Posture::Flying
                        | Posture::Carried
                        | Posture::OnShoulders
                        | Posture::Tied
                )
            {
                self.expect_entity_mut(
                    victim_id,
                    "ReceiveSwordDamage grounded lethal posture publication",
                )
                .element_data_mut()
                .posture = Posture::Dead;
            }
            let (dseq, didx) = damage_element;
            self.orders.sequence_manager.element_terminated(dseq, didx);
        }

        // Award XP if the victim died
        let victim_died =
            get_life_points(self.expect_entity(victim_id, "apply_sword_damage death check")) <= 0;
        if victim_died && let Some(atk_id) = attacker_id {
            self.award_sword_kill_xp(assets, atk_id, victim_id);
        }

        // TranslateSwordDamage calls virtual SayOuch on the victim (unless
        // parried or push already said it). PCs inherit the human no-op;
        // their SetLifePoints speech edge was handled above.
        if !coma_saved
            && !pushed
            && !result.is_empty()
            && !result.contains(combat::SwordDamageResult::NO_DAMAGE_PARRIED)
            && matches!(
                self.expect_entity(victim_id, "apply_sword_damage say-ouch"),
                Entity::Soldier(_) | Entity::Civilian(_)
            )
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
            let provoke_roll =
                crate::sim_rng::u32(sim, crate::sim_rng::RngSite::MeleeProvoke, 0..100);

            // Suppress Provoke when the attacker is the currently-selected
            // PC — the player's controlled character shouldn't taunt on hit.
            let attacker_is_selected_pc = self
                .expect_entity(atk_id, "sword-damage Provoke attacker")
                .kind()
                .is_pc()
                && self.selected_hero_ids().contains(&atk_id);
            if provoke_roll_succeeds(provoke_roll, attacker_ctx.fighting_ability)
                && !attacker_is_selected_pc
            {
                self.launch_provoke(atk_id);
            }
        }

        // Hero speech for PC attacker:
        // - HERO_KILLED_OPPONENT if dead
        // - HERO_SUCCESSFULL_BLOW if unconscious + cutting > 50
        // - HERO_STUN_ENNEMY if unconscious otherwise
        // Attacker-less (scripted) damage deliberately takes the non-PC
        // branch — the original hero-speech gate explicitly treats a
        // missing damage origin as "no PC attacker".  A present attacker
        // id must resolve to a live entity.
        let attacker_is_pc = attacker_id
            .map(|id| {
                self.expect_entity(id, "sword-damage hero-speech attacker")
                    .kind()
                    .is_pc()
            })
            .unwrap_or(false);
        let victim_is_unconscious = self
            .expect_entity(victim_id, "sword-damage hero-speech victim")
            .human_data()
            .map(|h| h.unconscious)
            .unwrap_or(false);
        let victim_is_lacklandist =
            match self.expect_entity(victim_id, "sword-damage hero-speech victim") {
                Entity::Soldier(s) => s
                    .soldier
                    .cached_camp
                    .is_hostile_to(crate::element::Camp::Royalists),
                _ => false,
            };

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
                .expect_entity(victim_id, "sword-damage hit-reaction victim")
                .human_data()
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
                .expect_entity(victim_id, "sword-damage hit-reaction victim")
                .element_data()
                .posture;
            let is_shoulder_posture = matches!(
                victim_posture,
                Posture::OnShoulders | Posture::CarryingOnShoulders | Posture::HelpingToClimb
            );
            if is_shoulder_posture {
                self.translate_shoulder_damage(sim, assets, victim_id, damage_element);
            } else if matches!(victim_posture, Posture::OnLadder | Posture::OnWall) {
                // Ladder/wall arm of the hit-reaction posture switch.
                // Like the shoulder arm this fires for lethal and KO
                // hits too — the fall itself resolves the victim's fate.
                self.translate_ladder_wall_fall(assets, victim_id, damage_element);
            } else if still_alive && still_conscious {
                let anims = {
                    let e = self.expect_entity(victim_id, "sword-damage hit-reaction victim");
                    let posture = e.element_data().posture;
                    let action = e.actor_data().map(|a| a.action_state).unwrap_or_default();
                    select_combat_animations(posture, action)
                };
                if let Some(a) = anims {
                    if result.contains(combat::SwordDamageResult::STUNNING_DAMAGE) {
                        // Stunning hit chain: fall-back, roll,
                        // stand-up, optional in-place stun if the
                        // defender is mid-swordfight with concussion
                        // above the threshold.
                        self.push_translated_damage_order(damage_element, a.falling_back);
                        self.try_queue_roll(assets, victim_id, damage_element);
                        self.push_translated_damage_order(damage_element, a.standing_up);
                        let (is_swordfighting, concussion) = self
                            .expect_entity(victim_id, "sword-damage stun chain victim")
                            .human_data()
                            .map(|h| (!h.opponents.is_empty(), h.concussion_of_the_brain))
                            .unwrap_or((false, 0));
                        if is_swordfighting && concussion > STUNNING_THRESHOLD {
                            self.push_translated_damage_order(
                                damage_element,
                                crate::order::OrderType::BeingStunnedSword,
                            );
                        }
                    } else if result.contains(combat::SwordDamageResult::CUTTING_DAMAGE) {
                        // Cutting hit, no follow-up roll / stand-up.
                        self.push_translated_damage_order(damage_element, a.simple_hit);
                    }
                }
            }
        }

        // TranslateSwordDamage dispatches combat stimuli to a soldier
        // attacker's AI: EventLethalStrike if the victim died, or
        // EventGoodStrike for a surviving victim that took cutting damage.
        //
        // The damage translation runs this inline, so the stimulus is
        // dispatched synchronously — the good-strike handler must still
        // observe the special-strike substate, and leaving this in the
        // ordinary NPC-phase queue can suppress the associated combat
        // remark.
        //
        // Push strikes never enter TranslateSwordDamage. Its posture switch
        // also returns before these informs for grounded non-riders and for
        // ladder/wall victims.
        let informer_reachable = {
            let victim = self.expect_entity(victim_id, "sword-damage informer victim");
            let posture = victim.element_data().posture;
            let is_dead_rider =
                matches!(victim, Entity::Soldier(s) if s.soldier.rider) && victim_died;
            // RHElementActorPC overrides TranslateSwordDamage and routes all
            // shoulder-lifecycle postures directly through
            // TranslateShoulderDamage. It therefore never reaches the base
            // human implementation's EventGoodStrike dispatch. Non-PC
            // humans inherit the base implementation, whose posture switch
            // does admit the carrier-side postures.
            let pc_uses_shoulder_translation = matches!(victim, Entity::Pc(_))
                && matches!(
                    posture,
                    Posture::HelpingToClimb | Posture::CarryingOnShoulders
                );
            (matches!(
                posture,
                Posture::Upright
                    | Posture::Spy
                    | Posture::Cloaked
                    | Posture::LeaningOut
                    | Posture::Leisure
                    | Posture::Siesta
                    | Posture::CarryingCorpse
                    | Posture::HelpingToClimb
                    | Posture::CarryingOnShoulders
                    | Posture::AnonymousArcher
                    | Posture::Sitting
                    | Posture::Crouched
                    | Posture::Tree
                    | Posture::SimulatingBeggar
            ) && !pc_uses_shoulder_translation)
                || is_dead_rider
        };
        let soldier_attacker = attacker_id.is_some_and(|id| {
            matches!(
                self.expect_entity(id, "sword-damage informer attacker"),
                Entity::Soldier(_)
            )
        });
        if !pushed
            && !result.is_empty()
            && !result.contains(combat::SwordDamageResult::NO_DAMAGE_PARRIED)
            && informer_reachable
        {
            if victim_died {
                // Dead people can't fight: the victim leaves the swordfight
                // before the hitter is informed. The quit notification
                // reaches the attacker first and moves it out of its
                // special-strike substate, which is what keeps the kill
                // remark from firing on a blow that ends the fight.
                self.quit_swordfight(sim, assets, victim_id);
            }
            let stimulus_type = if victim_died {
                Some(crate::ai::StimulusType::EventLethalStrike)
            } else if result.contains(combat::SwordDamageResult::CUTTING_DAMAGE) {
                Some(crate::ai::StimulusType::EventGoodStrike)
            } else {
                None
            };
            if soldier_attacker
                && let (Some(atk_id), Some(stimulus_type)) = (attacker_id, stimulus_type)
            {
                if stimulus_type == crate::ai::StimulusType::EventGoodStrike
                    && good_strike_lifecycle_debug_enabled()
                {
                    let creation_order = self.world.original_creation_order(atk_id);
                    if good_strike_lifecycle_debug_matches(
                        self.control.frame_counter,
                        creation_order,
                    ) {
                        let attacker = self
                            .get_entity(atk_id)
                            .unwrap_or_else(|| panic!("GOOD_STRIKE attacker {atk_id:?} vanished"));
                        let enemy = attacker.enemy_ai().unwrap_or_else(|| {
                            panic!("GOOD_STRIKE attacker {atk_id:?} has no enemy AI")
                        });
                        let victim = self
                            .get_entity(victim_id)
                            .unwrap_or_else(|| panic!("GOOD_STRIKE victim {victim_id:?} vanished"));
                        eprintln!(
                            "[GOOD_STRIKE frame={} owner={} owner_co={} phase=translate_before_dispatch victim={} result={:?} victim_dead={} victim_unconscious={} victim_posture={:?} owner_state={:?} owner_substate={:?} owner_installed_order={:?}]",
                            self.control.frame_counter,
                            atk_id.index(),
                            creation_order,
                            victim_id.index(),
                            result,
                            victim_died,
                            victim.human_data().is_some_and(|human| human.unconscious),
                            victim.element_data().posture,
                            enemy.base.current_state,
                            enemy.base.current_substate,
                            attacker
                                .actor_data()
                                .and_then(|actor| actor.installed_order),
                        );
                    }
                }
                self.dispatch_ai_stimulus(atk_id, crate::ai::Stimulus::new(stimulus_type));
                self.tick_enemy_ai_drain_pending_stimuli_for_npc(sim, atk_id, assets, None, None);
            }

            // Original `RHElementActorHuman::TranslateSwordDamage` sends
            // EVENT_GOOD_STRIKE to a soldier origin before testing the
            // surviving victim's unconscious flag.  A knocked-out human
            // then leaves the swordfight synchronously, before the
            // FallingBack/Roll orders are translated below
            // (`original-code/RHelementactorhuman.cpp:2643-2694`).
            if !victim_died
                && self
                    .expect_entity(victim_id, "sword-damage knockout victim")
                    .human_data()
                    .is_some_and(|human| human.unconscious)
            {
                self.quit_swordfight(sim, assets, victim_id);
            }
        }

        // Death push-vs-drop selector: a non-rider killed by a strike
        // with positive stunning effect falls on his back rather than
        // dropping forward.
        let dying_anim_override = if victim_died {
            let is_rider = matches!(
                self.expect_entity(victim_id, "sword-damage death anim victim"),
                Entity::Soldier(s) if s.soldier.rider
            );
            let stunning_effect = combat::get_strike_stunning_effect(&attacker_profile, strike);
            if !is_rider && stunning_effect > 0 {
                let e = self.expect_entity(victim_id, "sword-damage death anim victim");
                let posture = e.element_data().posture;
                let action = e.actor_data().map(|a| a.action_state).unwrap_or_default();
                select_combat_animations(posture, action).map(|a| a.falling_back)
            } else {
                None
            }
        } else {
            None
        };

        // `SetLifePoints` refuses to run `Kill` twice, but
        // `TranslateSwordDamage` still runs after that early return.  Until
        // the first dying order actually starts, the corpse therefore keeps
        // its standing posture/action and a queued same-frame cutting hit
        // books a fresh dying order of its own.  That second element becomes
        // Actor::mpSequenceElement/mpOrder after it interrupts the first one.
        // Keep this separate from `handle_post_damage`: that helper correctly
        // suppresses the repeated Kill cascade for an already-dead victim.
        if life_points_before <= 0
            && victim_died
            && !pushed
            && !result.is_empty()
            && !result.contains(combat::SwordDamageResult::NO_DAMAGE_PARRIED)
        {
            let repeated_dying_anim = self.get_entity(victim_id).and_then(|victim| {
                let posture = victim.element_data().posture;
                let action = victim
                    .actor_data()
                    .map(|actor| actor.action_state)
                    .unwrap_or_default();
                select_combat_animations(posture, action)
                    .map(|animations| dying_anim_override.unwrap_or(animations.dying_forward))
            });
            if let Some(anim) = repeated_dying_anim {
                self.queue_damage_anim(victim_id, damage_element, anim);
                self.try_queue_roll(assets, victim_id, damage_element);
            }
        }

        // Handle state transitions after damage — skip for push strikes,
        // since apply_push_effect already handled death/KO transitions.
        if !pushed && !result.is_empty() {
            if victim_went_unconscious {
                self.queue_knockout_orders(assets, victim_id, damage_element);
            } else if victim_was_unconscious && !victim_died {
                // SetConcussionOfTheBrain only runs its knockout cascade on the
                // conscious-to-unconscious edge. A later ordinary sword hit still
                // reaches TranslateSwordDamage: upright victims leave their
                // swordfight and receive FallingBack/Roll, while an already-down
                // posture has synchronously terminated above. Do not route either
                // case back through handle_post_damage's full KO side effects.
                if !grounded_translation_terminates {
                    self.queue_knockout_orders(assets, victim_id, damage_element);
                }
            } else {
                if fresh_lethal_ordinary {
                    self.queue_death_visuals_with_damage_element(
                        assets,
                        victim_id,
                        damage_element,
                        dying_anim_override,
                    );
                } else {
                    self.handle_post_damage(
                        sim,
                        assets,
                        victim_id,
                        life_points_before,
                        unconscious_before,
                        attacker_id,
                        false,
                        damage_element,
                        dying_anim_override,
                    );
                }
            }
        }
        self.trace_sword_damage_lifecycle(
            "apply-after",
            victim_id,
            attacker_id,
            Some(strike),
            Some(damage_element),
            Some(result),
        );
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
            // `DropCorpse` ends with `mpCarried->SetCarrier( 0 )`
            // (RHelementactorpc.cpp:6505), and
            // `RHElementActorHuman::SetCarrier( NULL )` runs
            // `SetDirection( mpCarrier->GetDirection() )`
            // (RHelementactorhuman.cpp:5800) before clearing the
            // back-reference.  `SetDirection` writes only the *goal*
            // (RHpositioninterface.h:248), so the body keeps the
            // `carrier_dir + 12` facing stamped above and turns toward
            // the carrier's own heading afterwards.
            if carried.human_data().is_some_and(|h| h.carrier.is_some()) {
                carried.element_data_mut().set_direction_goal(carrier_dir);
                carried
                    .human_data_mut()
                    .expect("carried body keeps its human payload across the carrier unlink")
                    .carrier = None;
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
        // Do not short-circuit an attached-scroll civilian here. Original's
        // RHElementActorCivilian overrides only GetWounded and
        // AddConcussionOfTheBrain; RHElementActorHuman::Translate still runs
        // the command-specific reaction after those no-op primitives return
        // (rhelementactorcivilian.cpp:901-924,
        // RHelementactorhuman.cpp:1608-1620).

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
            self.translate_ladder_wall_fall(assets, victim_id, damage_element);
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
        let life_points_before = get_life_points(victim);
        // `SetConcussionOfTheBrain` (RHelementactorhuman.cpp:456-505) only runs
        // the knock-out cascade from its `else` arm, i.e. when the victim was
        // still conscious before this hit. A victim that was already
        // unconscious never re-enters it.
        let unconscious_before = victim
            .human_data()
            .map(|human| human.unconscious)
            .unwrap_or(false);

        let victim = match self.world.entities.get_mut(victim_id) {
            Some(e) => e,
            None => return,
        };
        let (human, lp) = match victim.human_and_life_points_mut() {
            Some(pair) => pair,
            None => return,
        };

        let _died = combat::receive_generic_damage(human, lp, damage, concussion, max_lp, &ctx);
        let raw_life_points_after = *lp;
        self.close_pc_wounded_coma_boundary(
            sim,
            assets,
            victim_id,
            damage,
            life_points_before,
            raw_life_points_after,
        );

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
            self.handle_post_damage(
                sim,
                assets,
                victim_id,
                life_points_before,
                unconscious_before,
                None,
                false,
                damage_element,
                None,
            );
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
                self.push_translated_damage_order(damage_element, anim);
            }
            // Unconditional roll attempt (except for net damage, which
            // routes through a different path that never reaches
            // `apply_generic_damage`).
            self.try_queue_roll(assets, victim_id, damage_element);
        }

        self.handle_post_damage(
            sim,
            assets,
            victim_id,
            life_points_before,
            unconscious_before,
            None,
            false,
            damage_element,
            None,
        );
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
        // Attached-scroll civilian immunity belongs to the virtual wound and
        // concussion primitives, not around ReceivePiercingDamage's later
        // translation. See the matching generic-damage source-order note.

        // Preserve the incoming posture only for the ladder/wall reaction.
        // Original applies ReceivePiercingDamage before translating the hit,
        // so every other TranslateArrowDamage / TranslateDamage arm switches
        // on the posture produced by the damage (notably amulet coma -> Lying).
        let pre_posture = self
            .get_entity(victim_id)
            .map(|e| e.element_data().posture)
            .unwrap_or_default();

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
        let life_points_before = get_life_points(victim);
        // `SetConcussionOfTheBrain` (RHelementactorhuman.cpp:456-505) only runs
        // the knock-out cascade from its `else` arm, i.e. when the victim was
        // still conscious before this hit. A victim that was already
        // unconscious never re-enters it.
        let unconscious_before = victim
            .human_data()
            .map(|human| human.unconscious)
            .unwrap_or(false);

        let victim = match self.world.entities.get_mut(victim_id) {
            Some(e) => e,
            None => return,
        };
        let (human, lp) = match victim.human_and_life_points_mut() {
            Some(pair) => pair,
            None => return,
        };

        let _died = combat::receive_piercing_damage(human, lp, damage, concussion, max_lp, &ctx);
        let raw_life_points_after = *lp;
        let coma_saved = self.close_pc_wounded_coma_boundary(
            sim,
            assets,
            victim_id,
            damage,
            life_points_before,
            raw_life_points_after,
        );

        // `RHElementActorHuman::ReceivePiercingDamage`
        // (original-code/RHelementactorhuman.cpp:8945) wounds the victim
        // before `TranslateArrowDamage` runs, and `Human::SetLifePoints`
        // (RHelementactorhuman.cpp:10674) invokes the virtual `Kill` chain
        // synchronously inside that call. `Kill`
        // (RHelementactorhuman.cpp:9414) quits the swordfight and snaps the
        // AI to SLEEPING_FOREVER regardless of the victim's posture, while
        // `TranslateArrowDamage` (RHelementactorhuman.cpp:2358) merely
        // terminates the element without authoring an animation for a
        // victim that is flying, lying, carried, on shoulders, or tied.
        // Close that boundary here so those postures still run the kill
        // cascade instead of leaving a corpse linked to its opponents.
        let life_points_after = self
            .get_entity(victim_id)
            .map(get_life_points)
            .unwrap_or(raw_life_points_after);
        // `RHElementActorPC::SetLifePoints` (RHelementactorpc.cpp:6316) speaks
        // HERO_DIE / HERO_HURT off the stored life-point edge, inside the same
        // wounding call. `RHElementActorHuman::SayOuch`
        // (RHelementactorhuman.h:514) is an empty no-op for PCs, so the
        // translation below must not speak for them again.
        if !coma_saved {
            self.pc_life_points_speech(assets, victim_id, life_points_before, life_points_after);
        }
        let fresh_lethal = !coma_saved && life_points_before > 0 && life_points_after <= 0;
        if fresh_lethal {
            let killer_is_pc = self
                .orders
                .sequence_manager
                .get_element(damage_element.0, damage_element.1)
                .and_then(|element| match &element.data {
                    crate::sequence::SequenceElementData::Damage { origin, .. } => *origin,
                    _ => None,
                })
                .map(|killer| {
                    self.expect_entity(killer, "lethal piercing attacker")
                        .is_pc()
                })
                .unwrap_or(false);
            self.apply_nonvisual_death_cascade(
                sim,
                assets,
                victim_id,
                damage_element,
                killer_is_pc,
            );
        }

        let translation_posture = self
            .get_entity(victim_id)
            .map(|e| e.element_data().posture)
            .unwrap_or_default();
        // Raw attempted damage — overkill hits show the same number
        // as a non-overkill hit would.  `add_damage_number` no-ops on 0.
        self.add_damage_number(victim_id, damage);

        // Human::TranslateArrowDamage opens with virtual SayOuch before its
        // posture switch (RHelementactorhuman.cpp:2417-2424).  In particular,
        // a living NPC that is already Lying/Flying/etc. still speaks before
        // the damage element terminates without an order.  Keep PCs out of
        // this call: PC::TranslateArrowDamage may take its shoulder override,
        // and Human::SayOuch is a no-op for every PC path anyway.
        if !self
            .expect_entity(victim_id, "piercing-damage say-ouch")
            .is_pc()
        {
            self.say_ouch(sim, assets, victim_id, Some(damage));
        }

        // The ladder/wall translation is an arrow/stone hit reaction, not a
        // damage-immunity arm: ReceivePiercingDamage has already subtracted
        // life and applied concussion when Original gets here.
        if matches!(pre_posture, Posture::OnLadder | Posture::OnWall) {
            self.translate_ladder_wall_fall(assets, victim_id, damage_element);
            return;
        }

        // RHElementActorPC overrides TranslateArrowDamage/TranslateDamage:
        // these three PC postures never enter the Human posture switch below,
        // but instead dispatch TranslateShoulderDamage.  Keep this virtual
        // branch ahead of Human's OnShoulders -> Dead fallthrough.
        let pc_shoulder_override = matches!(self.get_entity(victim_id), Some(Entity::Pc(_)))
            && matches!(
                translation_posture,
                Posture::OnShoulders | Posture::CarryingOnShoulders | Posture::HelpingToClimb
            );
        if pc_shoulder_override {
            self.translate_shoulder_damage(sim, assets, victim_id, damage_element);
            if fresh_lethal {
                self.queue_death_visuals_with_damage_element(
                    assets,
                    victim_id,
                    damage_element,
                    None,
                );
            } else {
                self.handle_post_damage(
                    sim,
                    assets,
                    victim_id,
                    life_points_before,
                    unconscious_before,
                    None,
                    false,
                    damage_element,
                    None,
                );
            }
            return;
        }

        // Lying/UnderNet/Flying/Carried/OnShoulders/Tied falls through to
        // the Dead/DeadBack arm.  An already-dead non-rider is first changed
        // to Dead, then the element terminates without orders.  Do not use
        // Posture::is_lying here: Original's switch also includes Flying,
        // Carried, and OnShoulders, which that semantic helper deliberately
        // does not classify as ground corpses.
        if matches!(
            translation_posture,
            Posture::Lying
                | Posture::StuckUnderNet
                | Posture::Flying
                | Posture::Carried
                | Posture::OnShoulders
                | Posture::Tied
                | Posture::Dead
                | Posture::DeadBack
        ) {
            let post_dead = self
                .get_entity(victim_id)
                .map(|e| get_life_points(e) <= 0)
                .unwrap_or(false);
            let is_rider = matches!(
                self.get_entity(victim_id),
                Some(Entity::Soldier(s)) if s.soldier.rider
            );
            if post_dead
                && !is_rider
                && matches!(
                    translation_posture,
                    Posture::Lying
                        | Posture::StuckUnderNet
                        | Posture::Flying
                        | Posture::Carried
                        | Posture::OnShoulders
                        | Posture::Tied
                )
            {
                self.get_entity_mut(victim_id)
                    .expect("piercing-damage victim disappeared during translation")
                    .element_data_mut()
                    .posture = Posture::Dead;
            }
            if !is_rider || !post_dead {
                let (dseq, didx) = damage_element;
                self.orders.sequence_manager.element_terminated(dseq, didx);
                return;
            }
            // TODO: match the Original sleeping-rider special case,
            // which selects an upright dying animation even from a
            // lying posture.
        }

        // CarryingCorpse arm — forces an instant corpse drop and
        // falls through to the default damage path.
        if translation_posture == Posture::CarryingCorpse {
            self.force_drop_carried_corpse_instant(victim_id);
        }

        // TranslateArrowDamage / TranslateDamage always select an authored
        // reaction for an upright actor.  In particular, an arrow element
        // admitted after an earlier same-frame arrow already killed its
        // owner still receives Dying*; Human::SetLifePoints no-ops for the
        // already-dead actor, but translation continues.  This matters for
        // volleys: the later injury element interrupts the first dying
        // element and must replace it with another dying order rather than
        // becoming an accepted empty element.
        //
        // A newly lethal element gets its dying order from the virtual Kill
        // path below.  An already-dead element cannot re-enter Kill, so it
        // authors that order here, followed by TranslateRoll.
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
        let animations = self.get_entity(victim_id).and_then(|e| {
            let posture = e.element_data().posture;
            let action = e.actor_data().map(|a| a.action_state).unwrap_or_default();
            select_combat_animations(posture, action)
        });
        if still_on_ground {
            let translated_anim = if life_points_before <= 0 && life_points_after <= 0 {
                animations.map(|a| a.dying_forward)
            } else if life_points_after > 0 && (is_arrow_damage || still_conscious) {
                // `TranslateArrowDamage` (RHelementactorhuman.cpp:2399-2410)
                // tests only `IsDead()`: a surviving victim always extracts
                // the arrow, even while unconscious. Only the generic
                // `TranslateDamage` (RHelementactorhuman.cpp:2559-2576) has
                // the consciousness test.
                animations.map(|a| {
                    if is_arrow_damage {
                        a.arrow_extract
                    } else {
                        a.simple_hit
                    }
                })
            } else {
                // TODO: the unconscious non-arrow survivor should get
                // `TranslateDamage`'s `animFallingBack` plus `QuitSwordFight`
                // (RHelementactorhuman.cpp:2566-2575); today it gets no order
                // at all. Out of scope for the arrow fix above.
                None
            };
            if let Some(anim) = translated_anim {
                self.push_translated_damage_order(damage_element, anim);
            }

            if life_points_after > 0 || life_points_before <= 0 {
                self.try_queue_roll(assets, victim_id, damage_element);
            }
        }

        if fresh_lethal {
            // The nonvisual half of virtual `Kill` already ran inside
            // `ReceivePiercingDamage` above; only the translation-owned
            // dying animation and roll are still outstanding.
            self.queue_death_visuals_with_damage_element(assets, victim_id, damage_element, None);
        } else {
            self.handle_post_damage(
                sim,
                assets,
                victim_id,
                life_points_before,
                unconscious_before,
                None,
                false,
                damage_element,
                None,
            );
        }
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
        // RHCOMMAND_RECEIVE_HIT_DAMAGE unconditionally continues from
        // AddConcussionOfTheBrain into TranslateHitDamage. A civilian with
        // an attached scroll suppresses the concussion primitive, but still
        // authors the falling-hit order (RHelementactorhuman.cpp:1745-1791;
        // rhelementactorcivilian.cpp:901-909).
        let victim = self.expect_entity(victim_id, "apply_hit_damage victim");
        let ctx = concussion_ctx_full(
            victim,
            self.is_sherwood(&assets.profile_manager),
            Some(&self.mission_domain.campaign),
            self.control.sim_config.difficulty,
        );
        let life_points = get_life_points(victim);
        let victim = self.expect_entity_mut(victim_id, "apply_hit_damage victim");
        let human = match victim.human_data_mut() {
            Some(h) => h,
            None => return,
        };

        let outcome = combat::receive_hit_damage(human, life_points, concussion, &ctx);
        let went_unconscious = outcome == combat::ConcussionOutcome::WentUnconscious;

        // SetConcussionOfTheBrain performs the knockout transition inline:
        // QuitSwordFight (including its synchronous AI callback), add the
        // unconscious titbit, then the NPC override synchronously Thinks
        // EVENT_LOSE_CONSCIOUSNESS.
        // A missing damage origin (scripted/environmental hit) deliberately
        // counts as "no PC attacker" — the original knockout side-effect
        // chain explicitly null-checks the hitter.  A present attacker id
        // must resolve to a live entity.
        let attacker_is_pc = attacker_id
            .map(|id| {
                self.expect_entity(id, "apply_hit_damage attacker")
                    .kind()
                    .is_pc()
            })
            .unwrap_or(false);
        if went_unconscious {
            self.apply_knockout_side_effects(sim, assets, victim_id, attacker_is_pc, false);

            if let Some(atk_id) = attacker_id {
                let same_camp_soldier = {
                    let attacker = self.expect_entity(atk_id, "apply_hit_damage attacker");
                    let victim = self.expect_entity(victim_id, "apply_hit_damage victim");
                    matches!(attacker, Entity::Soldier(_)) && attacker.camp() == victim.camp()
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
            let conscious_npc =
                conscious_hit_notifies_ai(self.expect_entity(victim_id, "apply_hit_damage victim"));
            if conscious_npc {
                // Original always notifies a conscious NPC of the hit. It
                // attaches the hitter only when the origin is a human;
                // missing and non-human origins use the context-free event.
                let stimulus = match attacker_id.filter(|&id| {
                    self.expect_entity(id, "apply_hit_damage attacker")
                        .kind()
                        .is_human()
                }) {
                    Some(atk_id) => crate::ai::Stimulus::with_human(
                        crate::ai::StimulusType::EventGotHit,
                        atk_id.index(),
                    ),
                    None => crate::ai::Stimulus::new(crate::ai::StimulusType::EventGotHit),
                };
                self.dispatch_synchronous_ai_think_preserving_detection_fifo(
                    sim, victim_id, assets, stimulus,
                );
                // EVENT_GOTHIT applies SetViewStatus inline before
                // RHElementActorHuman::TranslateHitDamage continues to the
                // fall animation. Rust represents that synchronous write in
                // the AI recovery outbox, so consume it at the same boundary.
                self.tick_ai_pending_resurrection_and_eyes_for_npc(victim_id);
            }
        }

        // Shoulder fall routing — if the victim is currently on a
        // carrier's shoulders, the visual goes to
        // `translate_shoulder_damage` instead of the normal hit-fall
        // path.
        let victim_posture = self
            .expect_entity(victim_id, "apply_hit_damage victim")
            .element_data()
            .posture;

        // Human::Translate(RECEIVE_HIT_DAMAGE) adds concussion and sends the
        // NPC's synchronous EVENT_GOTHIT before testing for an already-lying
        // posture. Only the visual hit translation is skipped in that case.
        // This ordering matters when a postponed second hit resumes on the
        // same frame that the first hit's flight lands: EVENT_GOTHIT must
        // still restore EYES_DIE_OR_GET_UNCONSCIOUS before the element ends.
        if victim_posture == Posture::Lying {
            self.orders
                .sequence_manager
                .element_terminated(damage_element.0, damage_element.1);
            return;
        }

        if matches!(
            victim_posture,
            Posture::OnShoulders | Posture::CarryingOnShoulders | Posture::HelpingToClimb
        ) {
            self.translate_shoulder_damage(sim, assets, victim_id, damage_element);
            return;
        }

        // Original `RHElementActorHuman::TranslateHitDamage` calls virtual
        // `SayOuch()` synchronously before it appends the FALLING_HIT order.
        // PCs inherit Human's empty implementation; only NPCs override it.
        // This is particularly important when the hit interrupts speech
        // queued earlier in the same engine frame: SPEECH_EMERGENCY removes
        // that pending exclamation before Sound::Hourglass resolves the
        // replacement.  The already-down/flying postures terminate the
        // damage element without entering this default arm and stay silent.
        let victim_is_npc = matches!(
            self.expect_entity(victim_id, "apply_hit_damage victim"),
            Entity::Soldier(_) | Entity::Civilian(_)
        );
        if victim_is_npc
            && !matches!(
                victim_posture,
                Posture::Lying
                    | Posture::StuckUnderNet
                    | Posture::Flying
                    | Posture::Carried
                    | Posture::OnShoulders
                    | Posture::Tied
                    | Posture::Dead
                    | Posture::DeadBack
            )
        {
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
            self.translate_ladder_wall_fall(assets, victim_id, damage_element);
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
            None => {
                // TranslateHitDamage calls SetState(TERMINATED) for these
                // postures. Transition orders may already have been authored
                // before command translation; leaving the element live would
                // mistake one of those stale orders for a hit reaction and
                // run Actor::Instruct's IN_PROGRESS epilogue.
                self.orders
                    .sequence_manager
                    .element_terminated(damage_element.0, damage_element.1);
                return;
            }
        };

        self.push_translated_damage_order(damage_element, anim);
        self.orders
            .sequence_manager
            .get_element_mut(damage_element.0, damage_element.1)
            .and_then(|element| element.orders.back_mut())
            .expect("translated hit-fall order vanished")
            .antagonist = attacker_id;

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
        let charging_rider_dir = attacker_id
            .map(|id| self.expect_entity(id, "hit-flight attacker"))
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
        let attacker_pos = attacker_id.map(|id| {
            self.expect_entity(id, "hit-flight attacker")
                .element_data()
                .position_map()
        });
        let (flight_x, flight_y) = if let Some(rider_dir) = charging_rider_dir {
            let (x, y) = sector_to_vector_iso(rider_dir, ASPECT_RATIO);
            scale_falling_hit_sector_vector(x, y)
        } else if let Some(attacker_pos) = attacker_pos {
            let dx = victim_pos.x - attacker_pos.x;
            let dy = victim_pos.y - attacker_pos.y;
            normalize_falling_hit_antagonist_delta(dx, dy)
        } else {
            let (x, y) = sector_to_vector_iso((victim_dir as u16 + 8) % 16, ASPECT_RATIO);
            scale_falling_hit_sector_vector(x, y)
        };

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

        // ReadyForTakeOff retains its already-cached 3D takeoff point while
        // SetObstacle invalidates the lazy cache underneath it. Rust installs
        // obstacles eagerly, so preserve that captured point explicitly.
        let takeoff_position = self
            .get_entity(victim_id)
            .unwrap_or_else(|| panic!("falling-hit victim {victim_id:?} vanished"))
            .position_iface()
            .get_position();
        self.set_obstacle_and_material(
            assets,
            victim_id,
            goal_obstacle.map(|obstacle| obstacle.get()),
        );
        self.get_entity_mut(victim_id)
            .unwrap_or_else(|| panic!("falling-hit victim {victim_id:?} vanished"))
            .position_iface_mut()
            .set_position(takeoff_position);

        let flight_sector = crate::position_interface::vector_to_sector_0_to_15(flight_x, flight_y);
        let facing_sector = (flight_sector + 8) % 16;
        let victim = self
            .world
            .entities
            .get_mut(victim_id)
            .unwrap_or_else(|| panic!("falling-hit victim {victim_id:?} vanished"));
        victim.position_iface_mut().set_layer_goal(
            crate::position_interface::Layer::new(victim_layer)
                .expect("falling-hit victim layer cannot be the no-layer sentinel"),
        );
        victim.position_iface_mut().set_direction_instantly(
            crate::position_interface::Direction::from_raw(facing_sector as i32),
        );
        let dx = goal_x - victim_pos.x;
        let dy = goal_y - victim_pos.y;
        let dz = goal_z - victim_z;
        let dy_world = (goal_y + goal_z) - (victim_pos.y + victim_z);
        if dx.abs() > 0.01 || dy.abs() > 0.01 || dz.abs() > 0.01 {
            victim
                .actor_data_mut()
                .expect("falling-hit victim lost actor data")
                .active_flight = Some(crate::element::ActiveFlight {
                geometry: crate::element::FlightGeometry::World3d,
                increment_x: dx / frames as f32,
                increment_y: dy_world / frames as f32,
                goal_x,
                goal_y,
                frames_remaining: frames,
                antagonist: attacker_id,
                increment_z: dz / frames as f32,
                goal_z,
                goal_layer: victim_layer,
                goal_sector: victim_sector,
                obstacle: goal_obstacle,
                ladder_fall: false,
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
        life_points_before: i16,
        unconscious_before: bool,
        attacker_id: Option<EntityId>,
        no_damage: bool,
        damage_element: (crate::sequence::SequenceId, usize),
        dying_anim_override: Option<crate::order::OrderType>,
    ) {
        // Human::SetLifePoints begins with `if (IsDead()) return;`, so damage
        // delivered to an already-dead actor may still perform its authored
        // protection rolls but cannot invoke the virtual Kill cascade again.
        if no_damage || life_points_before <= 0 {
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
        } else if is_unconscious && !unconscious_before {
            // `inform_my_friends` only fires when the attacker is a
            // PC.  Resolve the attacker identity here so
            // `handle_knockout` can gate the broadcast.  Attacker-less
            // (scripted) damage deliberately counts as "no PC attacker";
            // a present attacker id must resolve.
            let attacker_is_pc = attacker_id
                .map(|id| {
                    self.expect_entity(id, "post-damage knockout attacker")
                        .kind()
                        .is_pc()
                })
                .unwrap_or(false);
            self.handle_knockout(sim, assets, victim_id, damage_element, attacker_is_pc);
        }
    }

    /// True if `victim_id` is a civilian carrying an unrevealed beggar
    /// scroll. The civilian's virtual wound / concussion primitives are
    /// immune, but command translation still runs; callers must not use this
    /// to skip a registered damage element's translation.
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
        let npc_ids: Vec<_> = self.world.entities.ai_owner_ids().collect();
        for npc_id in npc_ids {
            if npc_id == subject {
                continue;
            }
            if let Some(ai_actor) = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_actor_data_mut)
                && det_idx < ai_actor.detectable_lists.len()
            {
                ai_actor.delete_detectable(subject, kind);
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
    /// leave a PC, combat neighbour, or archery reservation pointing at the
    /// dead AI-controlled human.
    fn detach_npc_death_relationships(&mut self, victim_id: EntityId) {
        let (
            guarded_pcs,
            shooting_points,
            archery_sector,
            left_neighbours,
            right_neighbours,
            shield_bearers,
            archers,
        ) = {
            let Some(enemy) = self
                .world
                .entities
                .get_mut(victim_id)
                .and_then(Entity::enemy_ai_mut)
            else {
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
            // RHArtificialMalignity::SetState(SLEEPING_FOREVER) leaves every
            // combat-line mode and therefore calls both reciprocal neighbour
            // updates synchronously.  The lethal-damage path writes the
            // terminal AI state directly, so it must perform the same teardown
            // here before `clear_all_pending` discards the victim's outbox.
            let mut left_neighbours = Vec::new();
            let mut right_neighbours = Vec::new();
            let left_neighbour = std::mem::take(&mut enemy.left_combat_neighbour);
            if left_neighbour != 0 {
                left_neighbours.push(left_neighbour);
            }
            let right_neighbour = std::mem::take(&mut enemy.right_combat_neighbour);
            if right_neighbour != 0 {
                right_neighbours.push(right_neighbour);
            }
            // `EnemyAi::set_state` zeroes the victim's local fields eagerly,
            // but queues the reciprocal zero writes. Death clears that queue
            // below, so preserve every old neighbour target first.
            for action in &enemy.base.outbox.reentrant.cross_npc_actions {
                match *action {
                    crate::ai::CrossNpcAction::SetRightCombatNeighbour {
                        target,
                        neighbour: 0,
                    } => {
                        if target != 0 && !left_neighbours.contains(&target) {
                            left_neighbours.push(target);
                        }
                    }
                    crate::ai::CrossNpcAction::SetLeftCombatNeighbour {
                        target,
                        neighbour: 0,
                    } if target != 0 && !right_neighbours.contains(&target) => {
                        right_neighbours.push(target);
                    }
                    _ => {}
                }
            }
            // SUBSTATE_SLEEPING_FOREVER is neither a bow substate nor a
            // shield-protect/phalanx substate, so the same
            // `RHArtificialMalignity::SetState` constraint block also runs
            // `UpdateArcherBehindMe( NULL )` and
            // `UpdateShieldBearerBeforeMe( NULL )`
            // (RHartificialmalignity.cpp:9459-9494), each of which clears the
            // partner's reciprocal pointer
            // (RHartificialmalignity.cpp:17635-17662, 17675-17702). Tear the
            // archer/shield-bearer pairing down here for the same reason the
            // combat neighbours are handled above.
            let mut shield_bearers = Vec::new();
            let mut archers = Vec::new();
            let shield_bearer = std::mem::take(&mut enemy.shield_bearer_before_me);
            if shield_bearer != 0 {
                shield_bearers.push(shield_bearer);
            }
            let archer = std::mem::take(&mut enemy.archer_behind_me);
            if archer != 0 {
                archers.push(archer);
            }
            for action in &enemy.base.outbox.reentrant.cross_npc_actions {
                match *action {
                    crate::ai::CrossNpcAction::SetArcherBehindMe { target, archer: 0 } => {
                        if target != 0 && !shield_bearers.contains(&target) {
                            shield_bearers.push(target);
                        }
                    }
                    crate::ai::CrossNpcAction::SetShieldBearerBeforeMe {
                        target,
                        shield_bearer: 0,
                    } if target != 0 && !archers.contains(&target) => {
                        archers.push(target);
                    }
                    _ => {}
                }
            }
            debug_assert!(
                !release.release_sector || archery_sector.is_some(),
                "queued archery-sector release has no owned sector on death"
            );

            (
                guarded_pcs,
                shooting_points,
                archery_sector,
                left_neighbours,
                right_neighbours,
                shield_bearers,
                archers,
            )
        };

        for shield_bearer in shield_bearers {
            let shield_bearer_id =
                self.expect_human_id_for_ai_handle(shield_bearer, "dead AI owner's shield bearer");
            let enemy = self
                .world
                .entities
                .get_mut(shield_bearer_id)
                .and_then(Entity::enemy_ai_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "dead AI owner {victim_id:?}'s shield bearer {shield_bearer} has no EnemyAi"
                    )
                });
            enemy.archer_behind_me = 0;
        }
        for archer in archers {
            let archer_id = self.expect_human_id_for_ai_handle(archer, "dead AI owner's archer");
            let enemy = self
                .world
                .entities
                .get_mut(archer_id)
                .and_then(Entity::enemy_ai_mut)
                .unwrap_or_else(|| {
                    panic!("dead AI owner {victim_id:?}'s archer {archer} has no EnemyAi")
                });
            enemy.shield_bearer_before_me = 0;
        }

        for left_neighbour in left_neighbours {
            let left_id = self.expect_human_id_for_ai_handle(
                left_neighbour,
                "dead AI owner's left combat neighbour",
            );
            let enemy = self
                .world
                .entities
                .get_mut(left_id)
                .and_then(Entity::enemy_ai_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "dead AI owner {victim_id:?}'s left combat neighbour {left_neighbour} has \
                         no EnemyAi"
                    )
                });
            enemy.right_combat_neighbour = 0;
        }
        for right_neighbour in right_neighbours {
            let right_id = self.expect_human_id_for_ai_handle(
                right_neighbour,
                "dead AI owner's right combat neighbour",
            );
            let enemy = self
                .world
                .entities
                .get_mut(right_id)
                .and_then(Entity::enemy_ai_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "dead AI owner {victim_id:?}'s right combat neighbour {right_neighbour} has \
                         no EnemyAi"
                    )
                });
            enemy.left_combat_neighbour = 0;
        }

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
        self.queue_death_animation_with_damage_element(
            victim_id,
            damage_element,
            dying_anim_override,
        );

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

        self.apply_nonvisual_death_cascade(sim, assets, victim_id, damage_element, killer_is_pc);

        // Queue roll only after virtual Kill has synchronously completed.
        self.try_queue_roll(assets, victim_id, damage_element);
    }

    /// Finish the TranslateDamage-owned visual half of a death.  This is
    /// separate from virtual Kill because SetLifePoints completes the
    /// nonvisual cascade synchronously before translation resumes.
    fn queue_death_visuals_with_damage_element(
        &mut self,
        assets: &LevelAssets,
        victim_id: EntityId,
        damage_element: (crate::sequence::SequenceId, usize),
        dying_anim_override: Option<crate::order::OrderType>,
    ) {
        self.queue_death_animation_with_damage_element(
            victim_id,
            damage_element,
            dying_anim_override,
        );
        self.try_queue_roll(assets, victim_id, damage_element);
    }

    fn queue_death_animation_with_damage_element(
        &mut self,
        victim_id: EntityId,
        damage_element: (crate::sequence::SequenceId, usize),
        dying_anim_override: Option<crate::order::OrderType>,
    ) {
        tracing::info!(entity = ?victim_id, "Entity died");

        // Select dying animation (None when posture is already
        // Lying/Dead/Carried — the "already on the ground" case).
        let dying_anim = self.get_entity(victim_id).and_then(|e| {
            let posture = e.element_data().posture;
            let action = e.actor_data().map(|a| a.action_state).unwrap_or_default();
            select_combat_animations(posture, action)
                .map(|a| dying_anim_override.unwrap_or(a.dying_forward))
        });
        if let Some(anim) = dying_anim {
            self.queue_damage_anim(victim_id, damage_element, anim);
        }
    }

    /// Apply the synchronous virtual `Kill` cascade without translating a
    /// dying animation. `SetLifePoints` reaches this work before the caller
    /// resumes damage translation; push strikes therefore use it before
    /// `TranslatePushDamage` authors their sole falling order.
    pub(super) fn apply_nonvisual_death_cascade(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        damage_element: (crate::sequence::SequenceId, usize),
        killer_is_pc: bool,
    ) {
        // Throw away unrelated sequence work the victim owns. The active
        // damage sequence (which just had its dying order queued) and Todo
        // commands Original admits while dead remain in the manager FIFO.
        // The general-purpose `stop_owner` path is
        // wrong for death — it calls `stop_movement_for_owner` which
        // rewrites a walking order to a `TransitionWalking*Waiting*`
        // stop-animation and lets the movement element keep playing,
        // producing a "corpse walks a few more frames" visual.  We want
        // a hard interrupt instead, so the damage element's `DyingSword`
        // order becomes current without deleting a simultaneous pending hit.
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
        // Original `RHElementActorNPC::Kill` calls virtual `SetState`; the
        // enemy override clears the Beggar detectable bucket when this death
        // leaves STATE_SEEKING.  Rust writes the terminal controller state
        // directly below, so preserve that override side effect explicitly.
        let clear_beggar_detectables = victim
            .enemy_ai()
            .is_some_and(|ai| ai.base.current_state == crate::ai::AiState::Seeking);
        let forced_attentive = if victim.is_soldier() {
            victim
                .enemy_ai()
                .expect("dying soldier NPC has no EnemyAi")
                .forced_attentive
        } else {
            false
        };
        if let Some(ai) = victim.ai_controller_mut() {
            // Drop every remaining AI intent queued by the think that ran
            // earlier in this tick.  The relationship-maintenance effects
            // were applied synchronously above; all other channels must be
            // cauterised before death adds its own instant-music effect.
            ai.clear_all_pending();
            ai.set_alert_status_with_flags(
                crate::ai::AlertLevel::Green,
                crate::ai::AlertFlags::INSTANT_MUSIC_CHANGE,
                forced_attentive,
            );
            ai.current_state = crate::ai::AiState::Sleeping;
            ai.current_substate = crate::ai::Substate::SleepingForever;
            ai.clear_emoticon();
        }

        if let Some(npc) = victim.ai_actor_data_mut() {
            if clear_beggar_detectables {
                npc.detectable_lists[crate::element::DetectableType::Beggar as usize].clear();
            }
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
            .map(|e| e.is_soldier() && e.camp().is_hostile_to(Camp::Royalists))
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

        let falling_anim = self.queue_knockout_orders(assets, victim_id, damage_element);

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
        self.apply_knockout_side_effects(sim, assets, victim_id, attacker_is_pc, !falling_anim);
    }

    fn queue_knockout_orders(
        &mut self,
        assets: &LevelAssets,
        victim_id: EntityId,
        damage_element: (crate::sequence::SequenceId, usize),
    ) -> bool {
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

        if falling_anim.is_none() {
            self.expect_entity_mut(victim_id, "knockout victim posture fallback")
                .element_data_mut()
                .posture = Posture::Lying;
        }

        // Queue roll if on a slope.
        self.try_queue_roll(assets, victim_id, damage_element);
        falling_anim.is_some()
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
        if let Some(npc) = victim.ai_actor_data_mut() {
            // The NPC override performs this after the base-human
            // QuitSwordFight/titbit/healing work and before Think.
            npc.clear_all_suspects();
        }

        // RHElementActorNPC::SetConcussionOfTheBrain calls Think inline
        // after the base-human relationship/titbit work.
        if self
            .world
            .entities
            .get(victim_id)
            .is_some_and(|entity| entity.ai_controller().is_some())
        {
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
        if let Some(npc) = victim.ai_actor_data_mut() {
            // The Original assigns this only after EVENT_LOSE_CONSCIOUSNESS
            // returns.
            npc.inform_my_friends = attacker_is_pc;
        }
        if set_lying_now && !victim.element_data().posture.is_lying() {
            victim.set_posture(Posture::Lying);
        }
    }
}

#[cfg(test)]
mod provoke_tests {
    use super::provoke_roll_succeeds;

    #[test]
    fn fractional_provoke_threshold_is_not_truncated_before_comparison() {
        assert!(provoke_roll_succeeds(10, 51));
        assert!(provoke_roll_succeeds(0, 1));
    }

    #[test]
    fn exact_provoke_threshold_remains_exclusive() {
        assert!(provoke_roll_succeeds(9, 50));
        assert!(!provoke_roll_succeeds(10, 50));
    }
}
