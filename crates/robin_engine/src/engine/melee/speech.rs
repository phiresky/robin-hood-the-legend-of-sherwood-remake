//! Hero speech, expressions, tiredness, tie-up.
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;
use crate::element::{Command, Entity, EntityId};

impl EngineInner {
    // ─── Speech / sound effects ─────────────────────────────────────

    /// Reproduce `RHElementActorPC::SetLifePoints`' speech edge.
    ///
    /// This is separate from virtual `SayOuch`: PCs inherit the human
    /// no-op for that callback. The PC life-point setter compares the value
    /// that was actually stored, so protection, clamps, and the amulet's
    /// five-HP floor must be reflected before deciding whether to speak.
    pub(super) fn pc_life_points_speech(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        life_points_before: i16,
        life_points_after: i16,
    ) {
        let Some(Entity::Pc(pc)) = self.get_entity(entity_id) else {
            return;
        };
        let in_coma = self
            .pc_description_for_pc_data(&pc.pc)
            .is_some_and(|description| description.status.in_coma);
        if life_points_after == 0 && !in_coma {
            self.hero_speaking(assets, entity_id, HERO_DIE);
        } else if life_points_after < life_points_before - 20 {
            self.hero_speaking(assets, entity_id, HERO_HURT);
        }
    }

    /// Play the "ouch" expression for an entity (PC or NPC).
    ///
    /// For PCs, the life-point edge triggers (`HERO_DIE` when life == 0
    /// outside coma, `HERO_HURT` when the drop > 20) are applied
    /// here: `damage` is the amount just inflicted on the victim, or
    /// `None` when the caller doesn't have it on hand (push /
    /// shoulder paths), in which case the HERO_HURT gate defaults to
    /// "fire" to match the previous behaviour.
    pub(crate) fn say_ouch(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        damage: Option<u16>,
    ) {
        #[derive(Clone, Copy)]
        enum OuchOwner {
            Pc,
            Soldier(crate::profiles::SoldierProfileIdx),
            Civilian(crate::profiles::CivilianProfileIdx),
        }

        let (owner, position, is_dead, is_unconscious, is_npc_busy) = {
            let entity = match self.get_entity(entity_id) {
                Some(e) => e,
                None => return,
            };
            let pos = entity.element_data().position_map();
            let dead = entity.is_dead();
            let unc = entity.human_data().map(|h| h.unconscious).unwrap_or(false);
            // Brawling / looting NPCs (any take-money or
            // fight-for-money substate) skip the wounded remark
            // entirely — they're focused on the money / fight and
            // shouldn't shout WOUNDED/DIES.  Only NPC paths gate on
            // this; the PC arm below has no equivalent.
            let busy = entity
                .npc_data()
                .map(|n| {
                    let sub = n.ai_substate();
                    sub.is_take_money() || sub.is_fight_for_money()
                })
                .unwrap_or(false);
            let owner = match entity {
                Entity::Pc(_) => OuchOwner::Pc,
                Entity::Soldier(s) => OuchOwner::Soldier(s.soldier.soldier_profile_index),
                Entity::Civilian(c) => OuchOwner::Civilian(c.civilian.civilian_profile_index),
                _ => return,
            };
            (owner, pos, dead, unc, busy)
        };

        // Brawling / looting NPCs silently swallow the hit.
        if is_npc_busy {
            return;
        }

        // Unconscious NPCs go silent.  Yank any in-flight exclamation
        // on the way out (stop the sound and reset
        // `current_remark = TheSoundOfSilence`), clearing both the
        // host-side channel and the sim-side scheduled finish.
        if is_unconscious {
            if !matches!(owner, OuchOwner::Pc) {
                let was_speaking = self
                    .world
                    .entities
                    .get(entity_id)
                    .and_then(Entity::ai_controller)
                    .is_some_and(|base| {
                        base.current_remark != crate::ai::Remark::TheSoundOfSilence
                    });
                if was_speaking {
                    self.debug_speech_lifecycle(
                        entity_id.index(),
                        "unconscious_cancel_before",
                        "say_ouch",
                    );
                    self.feedback.pending_side_effects.sounds.push(
                        super::SoundCommand::StopExclamation {
                            actor_id: entity_id,
                        },
                    );
                    let base = self
                        .world
                        .entities
                        .get_mut(entity_id)
                        .and_then(Entity::ai_controller_mut)
                        .unwrap_or_else(|| {
                            panic!("unconscious NPC {} lost its AI", entity_id.index())
                        });
                    base.current_remark = crate::ai::Remark::TheSoundOfSilence;
                    base.current_remark_flags = 0;
                    self.cancel_exclamation_callbacks(entity_id.index());
                    self.debug_speech_lifecycle(
                        entity_id.index(),
                        "unconscious_cancel_after",
                        "say_ouch",
                    );
                }
            }
            return;
        }

        // PCs route through `hero_speaking` with emergency priority.
        // HERO_DIE only fires when the PC reaches 0 HP *and* is not
        // in coma; HERO_HURT only fires when the single-hit drop
        // exceeds 20 HP — smaller ticks (stones at 5, sword glances
        // at 10) stay silent.  `damage = None` falls back to "always
        // fire" for paths that haven't threaded the pre-damage LP
        // through (shoulder, push visuals); both of those follow the
        // main damage apply call which already gated the speech
        // correctly.
        if let OuchOwner::Pc = owner {
            if is_dead {
                let in_coma = self
                    .get_entity(entity_id)
                    .and_then(|e| match e {
                        Entity::Pc(pc) => self.pc_description_for_pc_data(&pc.pc),
                        _ => None,
                    })
                    .map(|d| d.status.in_coma)
                    .unwrap_or(false);
                if !in_coma {
                    self.hero_speaking_ex(assets, entity_id, HERO_DIE, SPEECH_EMERGENCY);
                }
            } else if damage.map(|d| d > 20).unwrap_or(true) {
                self.hero_speaking_ex(assets, entity_id, HERO_HURT, SPEECH_EMERGENCY);
            }
            return;
        }

        // NPC SayOuch routes through the ordinary Say filters with EMERGENCY,
        // exactly like RHElementActorNPC::SayOuch. Profile lookup happens only
        // after the money-fight and unconscious skip gates above.
        let (is_vip, is_civilian) = match owner {
            OuchOwner::Soldier(profile_index) => (
                assets
                    .profile_manager
                    .get_soldier(profile_index)
                    .unwrap_or_else(|| {
                        panic!(
                            "SayOuch owner {} requires missing soldier profile {}",
                            entity_id.index(),
                            profile_index
                        )
                    })
                    .vip,
                false,
            ),
            OuchOwner::Civilian(profile_index) => (
                assets
                    .profile_manager
                    .get_civilian(profile_index)
                    .unwrap_or_else(|| {
                        panic!(
                            "SayOuch owner {} requires missing civilian profile {}",
                            entity_id.index(),
                            profile_index
                        )
                    })
                    .civilian_type
                    == crate::profiles::CivilianType::Vip,
                true,
            ),
            OuchOwner::Pc => unreachable!(),
        };
        let remark = if is_vip {
            if is_dead {
                crate::ai::Remark::VipDies
            } else {
                crate::ai::Remark::VipWounded
            }
        } else if is_civilian {
            if is_dead {
                crate::ai::Remark::CivDies
            } else {
                crate::ai::Remark::CivWounded
            }
        } else if is_dead {
            crate::ai::Remark::Dies
        } else {
            crate::ai::Remark::Wounded
        };
        self.world
            .entities
            .get_mut(entity_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| panic!("SayOuch owner {} has no AI", entity_id.index()))
            .say_with_flags(remark, crate::ai::SpeechFlags::EMERGENCY);
        self.drain_ai_owner_work_for(sim, assets, entity_id);

        // Broadcast the AAARGH so nearby NPCs notice the cry.
        let (layer, elevation) = self
            .get_entity(entity_id)
            .map(|e| {
                (
                    e.element_data().layer(),
                    e.element_data().position().z.max(0.0) as u16,
                )
            })
            .unwrap_or((0, 0));
        self.broadcast_noise_synchronously(
            sim,
            assets,
            crate::ai::NoiseType::Aaargh,
            position,
            layer,
            crate::parameters_ai::NOISE_VOLUME_AAARGH as u16,
            elevation,
            Some(entity_id),
        );
    }

    /// Play a hero speech expression for a PC.
    ///
    /// Respects:
    /// - SoundConfig.amount_of_speaking (0-8) filtering by expression category
    /// - CanHeroSay (chorus timer + forbidden expression list)
    /// - Adds the expression to the forbidden list on playback
    pub(crate) fn hero_speaking(&mut self, assets: &LevelAssets, pc_id: EntityId, expression: u16) {
        self.hero_speaking_ex_with_variant(assets, pc_id, expression, SPEECH_NORMAL, None);
    }

    /// Scripted `RecordSpeakPC` path.  legacy implementation calls
    /// `HeroSpeaking(id, SPEECH_SCRIPT, forced_variant)` from
    /// `RHElementActorPC::ExecuteImmediately`.
    pub(crate) fn hero_speaking_script(
        &mut self,
        assets: &LevelAssets,
        pc_id: EntityId,
        expression: u16,
        forced_variant: Option<i32>,
    ) {
        self.hero_speaking_ex_with_variant(
            assets,
            pc_id,
            expression,
            SPEECH_SCRIPT,
            forced_variant,
        );
    }

    /// Full-signature version with priority flags.
    pub(super) fn hero_speaking_ex(
        &mut self,
        assets: &LevelAssets,
        pc_id: EntityId,
        expression: u16,
        priority: u16,
    ) {
        self.hero_speaking_ex_with_variant(assets, pc_id, expression, priority, None);
    }

    fn hero_speaking_ex_with_variant(
        &mut self,
        assets: &LevelAssets,
        pc_id: EntityId,
        expression: u16,
        priority: u16,
        forced_variant: Option<i32>,
    ) {
        use crate::sound::ExclamationGroup;

        let amount_of_speaking = self.control.sim_config.amount_of_speaking;

        // Priority filtering — the cascade returns hard with no
        // priority exemption.  `SPEECH_ALWAYS` bypasses only the
        // `CanHeroSay` (chorus + forbidden-list) checks below, NOT
        // this filter — including the ALWAYS bypass here would let
        // emergency hero speech fire at low amount_of_speaking
        // settings where it should have stayed silent.
        if !expression_allowed_by_amount(expression, amount_of_speaking) {
            tracing::trace!(
                pc = ?pc_id,
                expression,
                priority,
                amount_of_speaking,
                "hero_speaking: rejected by amount_of_speaking",
            );
            return;
        }

        // CanHeroSay check: chorus timer + forbidden expression list
        if self.control.chorus_timer > 0 && (priority & SPEECH_ALWAYS) == 0 {
            tracing::trace!(
                pc = ?pc_id,
                expression,
                priority,
                chorus_timer = self.control.chorus_timer,
                "hero_speaking: rejected by chorus timer",
            );
            return;
        }

        // Check forbidden list on PC
        let (profile_id, position, is_forbidden) = {
            let entity = match self.get_entity(pc_id) {
                Some(e) => e,
                None => return,
            };
            match entity {
                Entity::Pc(pc) => {
                    let profile = assets
                        .profile_manager
                        .get_character(pc.pc.profile_index)
                        .unwrap_or_else(|| {
                            panic!(
                                "missing PC profile {:?} for hero speech",
                                pc.pc.profile_index
                            )
                        });
                    let pos = entity.element_data().position_map();
                    let forbidden = pc
                        .pc
                        .forbidden_expressions
                        .iter()
                        .any(|(e, _)| *e == expression);
                    (profile.exclamation_id, pos, forbidden)
                }
                _ => return,
            }
        };

        if is_forbidden && (priority & SPEECH_ALWAYS) == 0 {
            tracing::trace!(
                pc = ?pc_id,
                expression,
                priority,
                "hero_speaking: rejected by forbidden-expression list",
            );
            return;
        }

        tracing::trace!(
            pc = ?pc_id,
            expression,
            priority,
            frame = self.control.frame_counter,
            "hero_speaking: queued exclamation",
        );

        // Queue the expression (drained after tick)
        self.feedback
            .pending_side_effects
            .sounds
            .push(super::SoundCommand::Exclamation {
                group: ExclamationGroup::Pc,
                profile_id,
                exclamation_id: expression,
                variant: forced_variant.unwrap_or(-1),
                position,
                actor_id: Some(pc_id),
            });
        self.feedback
            .sound_sim
            .pending_exclamations
            .push(crate::sound::PendingExclamation {
                actor_id: pc_id.index(),
                group: ExclamationGroup::Pc,
                profile_id,
                exclamation_id: expression,
                variant: forced_variant.map_or(-1, i32::from),
            });

        // Add to forbidden list + set anti-chorus timer
        let forbid_timer = match expression {
            HERO_SELECT => TIME_FORBID_HERO_SELECT,
            _ => HERO_EXPRESSION_DEFAULT_FORBID,
        };
        if let Some(Entity::Pc(pc)) = self.world.entities.get_mut(pc_id) {
            pc.pc.forbidden_expressions.push((expression, forbid_timer));
        }
        self.control.chorus_timer = DEFAULT_ANTI_CHORUS_TIMER;
    }

    /// Per-frame refresh of all PCs' forbidden expression list counters.
    /// Runs after the whole simulation frame (the Original ages these in its
    /// per-frame PC render refresh), so same-frame barks always age once.
    pub(in crate::engine) fn tick_refresh_hero_mouth(&mut self) {
        for (_, pc) in self.world.entities.pcs_mut() {
            pc.pc.forbidden_expressions.retain_mut(|(_, timer)| {
                *timer = timer.saturating_sub(1);
                *timer > 0
            });
        }
    }

    /// Fire combat-animation hero-speech triggers when a PC's
    /// `combat_anim` transitions.  Compares each PC's current
    /// `combat_anim` id against the previously observed id: a change
    /// to a *new* anim id is equivalent to MotionState::Start; a
    /// change to `None` (id 0) is MotionState::Done.  Filtering
    /// (chorus / forbidden / amount-of-speaking) is applied inside
    /// `hero_speaking`.
    pub(super) fn tick_pc_combat_anim_speech(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        self.tick_pc_combat_anim_speech_matching(sim, assets, None);
    }

    /// Run the PC Execute-owned combat-speech edge for one actor immediately
    /// after its sprite reports `MotionState::Start`.
    ///
    /// Original: `RHElementActorPC::Execute` uses
    /// `DoActionAndEventuallyPlayRemark`, so its RNG draw and `HeroSpeaking`
    /// side effects happen before the next element's Hourglass slot.
    pub(crate) fn tick_pc_combat_anim_speech_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.tick_pc_combat_anim_speech_matching(sim, assets, Some(owner));
    }

    fn tick_pc_combat_anim_speech_matching(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        only_owner: Option<EntityId>,
    ) {
        use crate::order::OrderType as OT;

        // Collect transitions first to avoid borrow conflicts with hero_speaking.
        let mut start_immediate: Vec<(EntityId, u16)> = Vec::new();
        let mut start_eventual: Vec<(EntityId, u16)> = Vec::new();
        let mut on_done: Vec<(EntityId, u16)> = Vec::new();

        // Snapshot each PC's current front order so the speech gate
        // can diff against the previous tick's observation without
        // needing the sequence manager held across the entity-mut loop.
        let cur_orders: std::collections::HashMap<
            EntityId,
            (std::num::NonZeroU32, OrderType, Command),
        > = {
            let mut m = std::collections::HashMap::new();
            for &pc_id in &self.world.pc_ids {
                if only_owner.is_some_and(|owner| owner != pc_id) {
                    continue;
                }
                if let Some((seq_id, elem_idx, o)) =
                    self.orders.sequence_manager.current_order_for_actor(pc_id)
                {
                    let command = self
                        .orders
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .map(|elem| elem.command)
                        .unwrap_or(Command::Null);
                    m.insert(pc_id, (o.order_id, o.order_type, command));
                }
            }
            m
        };

        for (id, pc) in self.world.entities.pcs_mut() {
            if only_owner.is_some_and(|owner| owner != EntityId::from(id)) {
                continue;
            }
            let (cur_id, cur_ot, cur_command) = match cur_orders.get(&id.into()) {
                Some((id, ot, command)) => (id.get(), Some(*ot), Some(*command)),
                None => (0, None, None),
            };
            let prev_id = pc.pc.prev_combat_anim_id;
            let prev_ot = pc.pc.prev_combat_anim_ot;
            let current_order_started = pc.element.sprite.last_processed_order_id == cur_id
                && pc.element.sprite.last_motion_state == Some(crate::sprite::MotionState::Start);

            if cur_id != prev_id && (cur_id == 0 || current_order_started) {
                // START means the new order actually reached the PC Execute
                // arm and its sprite returned MotionState::Start. Merely
                // becoming the selected front order during the manager tail
                // is one frame earlier and must not own speech/RNG yet.
                if let Some(ot) = cur_ot {
                    match ot {
                        OT::TransitionRaisingSword if cur_command != Some(Command::HitTarget) => {
                            start_immediate.push((id.into(), HERO_PROVOKE_DUEL));
                        }
                        OT::Provoking => start_immediate.push((id.into(), HERO_PROVOKE_OPPONENT)),
                        OT::StrikingLeftSmalltalk
                        | OT::StrikingRightSmalltalk
                        | OT::StrikingLowLeftSmalltalk
                        | OT::StrikingLowRightSmalltalk => {
                            start_eventual.push((id.into(), HERO_SWEAR_AT));
                        }
                        OT::StrikingRoundLeftSword
                        | OT::StrikingRoundRightSword
                        | OT::ExecutingSword => {
                            start_eventual.push((id.into(), HERO_WARCRY));
                        }
                        _ => {}
                    }
                }
                // DONE: anim finished (current is None, previous was set).
                if cur_id == 0
                    && matches!(
                        prev_ot,
                        Some(
                            OT::ExtractingArrowUpright
                                | OT::ExtractingArrowBow
                                | OT::ExtractingArrowSword
                        )
                    )
                {
                    on_done.push((id.into(), HERO_PROVOKE_OPPONENT));
                }
                pc.pc.prev_combat_anim_id = cur_id;
                pc.pc.prev_combat_anim_ot = cur_ot;
            }
        }

        for (id, expr) in start_immediate {
            self.hero_speaking(assets, id, expr);
        }
        for (id, expr) in start_eventual {
            // Original's DoActionAndEventuallyPlayRemark tests
            // `rand() > RAND_MAX / 2`, not the low bit. Both are unbiased in
            // a fresh stream, but replaying a concrete libc draw must preserve
            // the exact predicate.
            if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::HeroSpeech, 0..=2_147_483_647)
                > 2_147_483_647 / 2
            {
                self.hero_speaking(assets, id, expr);
            }
        }
        for (id, expr) in on_done {
            self.hero_speaking(assets, id, expr);
        }
    }

    /// Launch a provoke (taunt) sequence element on an entity.  The
    /// dispatcher in `tick.rs` wires the Provoking animation through
    /// `active_ai_anim` + `do_next_order`.
    pub(super) fn launch_provoke(&mut self, entity_id: EntityId) {
        let elem = crate::sequence::SequenceElement::new(
            1,
            crate::element::Command::Provoke,
            Some(entity_id),
        );
        self.launch_element(elem);
    }

    pub(crate) fn dispatch_provoke(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        sequence_id: crate::sequence::SequenceId,
        element_index: usize,
    ) {
        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(ai) = entity.ai_controller_mut()
        {
            ai.say(crate::ai::Remark::ProvokesCombat);
        }
        self.drain_ai_owner_work_for(sim, assets, owner);
        let mut order = crate::order::Order::new(
            crate::order::OrderType::Provoking,
            0.0,
            0.0,
            self.orders.allocate_order_id(),
        );
        order.compute_direction = false;
        self.orders
            .sequence_manager
            .push_order_on(sequence_id, element_index, order);
        self.orders
            .sequence_manager
            .element_in_progress(sequence_id, element_index);
    }

    // ─── AI stimulus dispatch ─────────────────────────────────────────

    /// Queue a stimulus on an actor's common AI controller.
    ///
    /// Original human concussion paths use `IsNPC()`, covering soldiers and
    /// civilians alike. Custom battle PCs may explicitly own that same
    /// controller; ordinary player PCs remain the only intentional no-op.
    pub(crate) fn dispatch_ai_stimulus(
        &mut self,
        entity_id: EntityId,
        stimulus: crate::ai::Stimulus,
    ) {
        let entity = self.world.entities.get_mut(entity_id).unwrap_or_else(|| {
            panic!(
                "NPC {} disappeared while queueing {:?}",
                entity_id.index(),
                stimulus.stimulus_type
            )
        });
        if matches!(entity, Entity::Pc(pc) if pc.pc.ai.is_none()) {
            return;
        }
        let ai_actor = entity.ai_actor_data_mut().unwrap_or_else(|| {
            panic!(
                "AI actor {} lost its required AI data while queueing {:?}",
                entity_id.index(),
                stimulus.stimulus_type
            )
        });
        let ai = ai_actor.ai_brain.base_mut().unwrap_or_else(|| {
            panic!(
                "AI actor {} is missing its required AI controller while queueing {:?}",
                entity_id.index(),
                stimulus.stimulus_type
            )
        });
        ai.outbox.detection.stimuli.push(stimulus);
    }

    // ─── Tiredness tick ──────────────────────────────────────────────

    /// Per-frame tiredness recovery.
    ///
    /// `if !IsSwordfighting() && !IsMoving() { muwTiredness -= GetEndurance()/10 }`
    /// (`original-code/RHelementactorhuman.cpp:496-513`).
    /// Apply the human tiredness tail for one live owner.
    pub(crate) fn tick_tiredness_for(&mut self, id: EntityId, assets: &LevelAssets) {
        let frame = self.control.frame_counter;
        // Original staggers this tail by RHElement::mulCreationOrder, not by
        // the port's kind-local entity slot. Saved games can restore an
        // authored creation order which differs from `EntityId::index()`.
        let creation_order = self.world.original_creation_order(id);
        if (frame & 63) != (creation_order & 31) {
            return;
        }
        let entity = self.world.entities.get_mut(id).unwrap_or_else(|| {
            panic!(
                "tiredness owner {} disappeared from its legacy slot",
                id.index()
            )
        });
        assert!(
            entity.human_data().is_some(),
            "tiredness owner {} is not human",
            id.index()
        );
        let is_swordfighting = entity
            .human_data()
            .is_some_and(|human| !human.opponents.is_empty());
        // `RHElement::IsMoving()` (`original-code/RHElement.h:355`) forwards to
        // `RHPositionInterface::IsMoving()`
        // (`original-code/RHpositioninterface.h:189`), which is the 3D
        // position-versus-old-position test, NOT an action-state test. A
        // swordfighter that walks a step every other frame is "not moving" on
        // the frames where the two agree, and the Original recuperates on
        // exactly those frames.
        let is_moving = entity.position_iface().is_moving();
        let probe = crate::combat::tiredness_debug_matches(creation_order);
        if is_swordfighting || is_moving {
            if probe {
                let tiredness = entity.human_data().map_or(0, |human| human.tiredness);
                eprintln!(
                    "RUST_TIREDNESS frame={frame} co={creation_order} site=recuperation_skipped \
                     tiredness={tiredness} swordfighting={} moving={}",
                    is_swordfighting as u8, is_moving as u8
                );
            }
            return;
        }
        let endurance = endurance_from_profile(entity, &assets.profile_manager);
        let recuperation = endurance / 10;
        let human = entity
            .human_data_mut()
            .expect("validated human tiredness owner lost HumanData");
        let before = human.tiredness;
        human.tiredness = human.tiredness.saturating_sub(recuperation);
        if probe {
            eprintln!(
                "RUST_TIREDNESS frame={frame} co={creation_order} site=recuperation \
                 before={before} after={} endurance={endurance} recuperation={recuperation}",
                human.tiredness
            );
        }
    }

    // ─── Tie-up (public, called from natives/UI) ────────────────────
}
