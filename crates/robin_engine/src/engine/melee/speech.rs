//! Hero speech, expressions, tiredness, tie-up.
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;
use crate::element::{Command, Entity, EntityId};

impl EngineInner {
    // ─── Speech / sound effects ─────────────────────────────────────

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
        assets: &LevelAssets,
        entity_id: EntityId,
        damage: Option<u16>,
    ) {
        use crate::sound::ExclamationGroup;

        let (group, profile_id, position, is_dead, is_unconscious, is_vip, is_npc_busy) = {
            let entity = match self.get_entity(entity_id) {
                Some(e) => e,
                None => return,
            };
            let pos = entity.element_data().position_map();
            let dead = entity.is_dead();
            let unc = entity.human_data().map(|h| h.unconscious).unwrap_or(false);
            let vip = is_vip_from_profile(entity, &assets.profile_manager);
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
            match entity {
                Entity::Pc(pc) => {
                    let profile = assets
                        .profile_manager
                        .get_character(pc.pc.profile_index)
                        .unwrap_or_else(|| {
                            panic!(
                                "missing PC profile {:?} for melee speech",
                                pc.pc.profile_index
                            )
                        });
                    (
                        ExclamationGroup::Pc,
                        profile.exclamation_id,
                        pos,
                        dead,
                        unc,
                        vip,
                        busy,
                    )
                }
                Entity::Soldier(s) => {
                    let profile = assets
                        .profile_manager
                        .get_soldier(s.soldier.soldier_profile_index)
                        .unwrap_or_else(|| {
                            panic!(
                                "missing soldier profile {:?} for melee speech",
                                s.soldier.soldier_profile_index
                            )
                        });
                    (
                        ExclamationGroup::Soldier,
                        profile.exclamation_id,
                        pos,
                        dead,
                        unc,
                        vip,
                        busy,
                    )
                }
                Entity::Civilian(c) => {
                    let profile = assets
                        .profile_manager
                        .get_civilian(c.civilian.civilian_profile_index)
                        .unwrap_or_else(|| {
                            panic!(
                                "missing civilian profile {:?} for melee speech",
                                c.civilian.civilian_profile_index
                            )
                        });
                    (
                        ExclamationGroup::Civilian,
                        profile.exclamation_id,
                        pos,
                        dead,
                        unc,
                        profile.civilian_type == crate::profiles::CivilianType::Vip,
                        busy,
                    )
                }
                _ => return,
            }
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
            if group != ExclamationGroup::Pc {
                self.feedback.pending_side_effects.sounds.push(
                    super::SoundCommand::StopExclamation {
                        actor_id: entity_id,
                    },
                );
                if let Some(entity) = self.world.entities.get_mut(entity_id)
                    && let Some(npc) = entity.npc_data_mut()
                    && let Some(base) = npc.ai_brain.base_mut()
                    && (base.current_remark != crate::ai::Remark::TheSoundOfSilence
                        || base.speech_in_flight)
                {
                    base.current_remark = crate::ai::Remark::TheSoundOfSilence;
                    base.current_remark_flags = 0;
                    base.speech_in_flight = false;
                }
                self.feedback
                    .sound_sim
                    .playing_exclamations
                    .retain(|p| p.actor_id != entity_id.index());
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
        if group == ExclamationGroup::Pc {
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

        // NPC remarks
        let remark = if is_vip {
            if is_dead {
                VIP_REMARK_DIES
            } else {
                VIP_REMARK_WOUNDED
            }
        } else if group == ExclamationGroup::Civilian {
            if is_dead {
                CIV_REMARK_DIES
            } else {
                CIV_REMARK_WOUNDED
            }
        } else if is_dead {
            REMARK_DIES
        } else {
            REMARK_WOUNDED
        };

        self.feedback
            .pending_side_effects
            .sounds
            .push(super::SoundCommand::StopExclamation {
                actor_id: entity_id,
            });
        self.feedback
            .sound_sim
            .playing_exclamations
            .retain(|p| p.actor_id != entity_id.index());
        self.feedback
            .pending_side_effects
            .sounds
            .push(super::SoundCommand::Exclamation {
                group,
                profile_id,
                exclamation_id: remark,
                variant: -1,
                position,
                actor_id: Some(entity_id),
            });
        let duration =
            exclamation_duration_frames(&assets.exclamation_durations, group, profile_id, remark);
        self.feedback
            .sound_sim
            .playing_exclamations
            .push(crate::sound::PlayingExclamation {
                actor_id: entity_id.index(),
                exclamation_id: remark as u32,
                finish_frame: self.control.frame_counter + duration,
            });

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
        self.broadcast_noise(
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

        // Get SoundConfig amount_of_speaking
        let amount_of_speaking = {
            let ppm = crate::player_profile::PlayerProfileManager::global();
            ppm.as_ref()
                .and_then(|mgr| mgr.get_active())
                .map(|p| p.sound_config.amount_of_speaking)
                .unwrap_or(5)
        };

        // Priority filtering — the cascade returns hard with no
        // priority exemption.  `SPEECH_ALWAYS` bypasses only the
        // `CanHeroSay` (chorus + forbidden-list) checks below, NOT
        // this filter — including the ALWAYS bypass here would let
        // emergency hero speech fire at low amount_of_speaking
        // settings where it should have stayed silent.
        if !expression_allowed_by_amount(expression, amount_of_speaking) {
            return;
        }

        // CanHeroSay check: chorus timer + forbidden expression list
        if self.control.chorus_timer > 0 && (priority & SPEECH_ALWAYS) == 0 {
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
            return;
        }

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
        let duration = exclamation_duration_frames(
            &assets.exclamation_durations,
            ExclamationGroup::Pc,
            profile_id,
            expression,
        );
        self.feedback
            .sound_sim
            .playing_exclamations
            .push(crate::sound::PlayingExclamation {
                actor_id: pc_id.index(),
                exclamation_id: expression as u32,
                finish_frame: self.control.frame_counter + duration,
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
    pub(super) fn tick_refresh_hero_mouth(&mut self) {
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
    pub(super) fn tick_pc_combat_anim_speech(&mut self, assets: &LevelAssets) {
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
            let (cur_id, cur_ot, cur_command) = match cur_orders.get(&id.into()) {
                Some((id, ot, command)) => (id.get(), Some(*ot), Some(*command)),
                None => (0, None, None),
            };
            let prev_id = pc.pc.prev_combat_anim_id;
            let prev_ot = pc.pc.prev_combat_anim_ot;

            if cur_id != prev_id {
                // START: a new anim took over.
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
            // 50% chance.
            if crate::sim_rng::bool(crate::sim_rng::RngSite::HeroSpeech) {
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

    // ─── AI stimulus dispatch ─────────────────────────────────────────

    /// Send a stimulus to an NPC soldier's AI controller.
    ///
    /// Used to notify the attacker's AI of combat events
    /// (EventGoodStrike, EventLethalStrike, etc.).
    pub(crate) fn dispatch_ai_stimulus(
        &mut self,
        entity_id: EntityId,
        stimulus: crate::ai::Stimulus,
    ) {
        let Some(Entity::Soldier(s)) = self.world.entities.get_mut(entity_id) else {
            return;
        };
        if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
            // Queue the stimulus for the next AI tick rather than calling
            // think() inline (avoids re-entrant borrow issues). The engine's
            // detection loop will pick up any pending stimuli.
            enemy_ai.base.pending_stimuli.push(stimulus);
        }
    }

    // ─── Tiredness tick ──────────────────────────────────────────────

    /// Per-frame tiredness recovery.
    ///
    /// `if !is_swordfighting && !is_moving { tiredness -= endurance/10 }`.
    pub(crate) fn tick_tiredness(&mut self, assets: &LevelAssets) {
        let frame = self.control.frame_counter;
        for (id, entity) in self.world.entities.humans_mut() {
            let idx = id.index();
            // Spread the work — only every 64 frames per entity
            if (frame & 63) != (idx & 31) {
                continue;
            }
            if entity.is_dead() {
                continue;
            }
            let is_swordfighting = entity
                .human_data()
                .map(|h| !h.opponents.is_empty())
                .unwrap_or(false);
            let is_moving = entity
                .actor_data()
                .map(|a| a.action_state.is_moving())
                .unwrap_or(false);
            if is_swordfighting || is_moving {
                continue;
            }
            // Real endurance from profile
            let endurance = endurance_from_profile(entity, &assets.profile_manager);
            let recuperation = endurance / 10;
            if let Some(human) = entity.human_data_mut() {
                human.tiredness = human.tiredness.saturating_sub(recuperation);
            }
        }
    }

    // ─── Tie-up (public, called from natives/UI) ────────────────────
}
