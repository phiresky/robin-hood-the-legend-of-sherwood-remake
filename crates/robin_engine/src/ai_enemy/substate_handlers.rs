//! `EnemyAi::think_expected_event` — the substate state machine.
//!
//! Lifted out of `ai_enemy/mod.rs` to keep the giant per-substate match
//! manageable. Lives in a separate `impl EnemyAi` block; child modules
//! see the parent's private fields and helpers.

use crate::ai::*;
use crate::parameters_ai;

use super::util::{pos_diff, resolve_seek_point_id, vec_to_sector};
use super::{
    EnemyAi, PrimaryTargetFlags, ProfileRank, SeekFlags, UNDEFINED_DIRECTION, archer, combat,
    task_priority,
};

impl EnemyAi {
    // One dispatcher preserves the numeric Substate machine while the
    // implementations are owned by coherent state families. See
    // original-code/RHartificialmalignity.cpp:ThinkExpectedEvent.
    pub(super) fn think_expected_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        debug_assert_eq!(
            self.base.current_substate.ai_state_family(),
            Some(self.base.current_state),
            "EnemyAi expected-event dispatch received mismatched state/substate: {:?}/{:?}",
            self.base.current_state,
            self.base.current_substate
        );

        match self.base.current_state {
            AiState::Sleeping => {
                self.think_expected_sleeping_event(stimulus, global, ctx, tick, grid)
            }
            AiState::Default => {
                self.think_expected_default_event(sim, stimulus, global, ctx, tick, grid)
            }
            AiState::Wondering => {
                self.think_expected_wondering_event(sim, stimulus, global, ctx, tick, grid)
            }
            AiState::Seeking => {
                self.think_expected_seeking_event(sim, stimulus, global, ctx, tick, grid)
            }
            AiState::Attacking => {
                self.think_expected_attacking_event(sim, stimulus, global, ctx, tick, grid)
            }
            AiState::Menacing => {
                self.think_expected_menacing_event(sim, stimulus, global, ctx, tick, grid)
            }
            AiState::Fleeing => {
                self.think_expected_fleeing_event(sim, stimulus, global, ctx, tick, grid)
            }
        }
    }

    fn think_expected_sleeping_event(
        &mut self,
        stimulus: &Stimulus,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
        _tick: &AiPerTickData,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;
        match self.base.current_substate {
            Substate::SleepingAwakening => {
                if matches!(
                    stimulus_type,
                    StimulusType::EventDone | StimulusType::EventTimer
                ) {
                    if let Some(alert_path_id) = self.base.alert_path_id
                        && !self.changed_to_alert_path
                    {
                        self.changed_to_alert_path = true;
                        // Rebuild the patrol path from the alert-path
                        // hiking path index.
                        let hiking_paths = &ctx.hiking_paths;
                        self.base.patrol_path =
                            crate::ai::PatrolPath::new(alert_path_id, hiking_paths);
                        self.base.has_patrol_path = self.base.patrol_path.is_some();
                    }
                    self.base.set_emoticon(EmoticonType::QuestionMark);
                    self.set_state(AiState::Wondering, Substate::WonderingLooking1);
                    self.base.launch_timer(30, ctx.frame);
                }
            }

            // ============ DEFAULT (common) ============
            _ => {}
        }
        false
    }

    fn think_expected_default_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;
        match self.base.current_substate {
            Substate::DefaultGotoPost
            | Substate::DefaultGotoPostTurn
            | Substate::DefaultGotoRoute
            | Substate::DefaultGotoRouteTurn
            | Substate::DefaultOnPost
            | Substate::DefaultEnroute
            | Substate::DefaultInMacro
            | Substate::DefaultInMacroWaitingForDone => {
                // `think_expected_event_common_stuff` calls the virtual
                // `default_bored_standard_procedure` on timer expiry
                // during `DefaultOnPost`. Run it before delegating so
                // the subclass override takes effect; if it transitions
                // state we short-circuit, otherwise fall through to the
                // base timer.
                if self.base.current_substate == Substate::DefaultOnPost
                    && stimulus_type == StimulusType::EventTimer
                    && self.default_bored_standard_procedure(sim, ctx)
                {
                    return true;
                }
                return self
                    .base
                    .think_expected_event_common_stuff(sim, stimulus, ctx);
            }

            Substate::DefaultOnPostLookingSidewards => {
                if stimulus_type == StimulusType::EventDone {
                    self.set_state(AiState::Default, Substate::DefaultOnPost);
                    let bored = self.base.get_bored_time(sim, ctx);
                    tracing::trace!(
                        me = self.base.me,
                        bored,
                        frame = ctx.frame,
                        "look-sidewards done; relaunching bored timer"
                    );
                    self.base.launch_timer(bored as u32, ctx.frame);
                }
            }

            Substate::DefaultLookingOfficerForAdvice => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            Substate::DefaultLookingShadow => {
                // Keep watching as long as the shadow is still somewhat
                // visible. The engine updates `max_visibility` each
                // detection tick; if it drops to 0 the target is fully
                // hidden again.
                if stimulus_type == StimulusType::EventTimer {
                    if self.base.max_visibility > 0 {
                        // Target still partially visible — keep looking
                        self.base.launch_timer(10, ctx.frame);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }

            // ============ PATROL ENROUTE ============
            Substate::DefaultPatrolEnroute | Substate::DefaultPatrolEnrouteRunning => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // Reached our position in the formation — face
                    // patrol direction.  Only issue the `face_to` when
                    // the current facing differs, otherwise the no-op
                    // turn re-triggers a bogus `EventDone` through the
                    // sequence manager.
                    if self.base.patrol_direction != ctx.direction {
                        self.base.face_direction(self.base.patrol_direction, ctx);
                    }
                    self.set_state(AiState::Default, Substate::DefaultPatrolEnrouteWaiting);
                    self.base.launch_timer(200, ctx.frame);
                }
            }

            Substate::DefaultPatrolEnrouteWaiting => {
                if stimulus_type == StimulusType::EventTimer {
                    // Check patrol chief's AI state (cached by engine each patrol tick).
                    // If chief is in Default or Wondering, keep waiting for next
                    // coordinate call. Otherwise the chief is in trouble — abandon.
                    match tick.patrol_chief_state {
                        AiState::Default | AiState::Wondering => {
                            self.base.launch_timer(200, ctx.frame);
                        }
                        _ => {
                            // Chief is in combat or otherwise unavailable
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        }
                    }
                }
            }

            Substate::DefaultGotoChief => {
                if stimulus_type == StimulusType::EventReachPoint {
                    if self.base.patrol_chief.is_some() {
                        // Face toward the chief's position (cached by engine).
                        self.base
                            .face_position_with_ctx(tick.patrol_chief_position, ctx);
                        self.set_state(AiState::Default, Substate::DefaultPatrolEnrouteWaiting);
                        self.base.launch_timer(200, ctx.frame);
                    } else {
                        // Lost patrol chief — retry
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }

            // ============ PATROL CHIEF RETURN ============
            Substate::DefaultPatrolChiefReturnToPatrol => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            // ============ WONDERING ============
            Substate::DefaultScriptDriven => {}

            // Soldier keeps scanning for Charly while on duty.
            // Random sidewards look, sorrow-level accumulation, and
            // periodic re-seeking.
            Substate::DefaultLookingForCharly => {
                if stimulus_type == StimulusType::EventTimer {
                    let rand_sorrow =
                        crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CharlySorrow, 0..5000)
                            as u16;
                    if rand_sorrow < self.base.sorrow_level + 10 {
                        self.set_state(
                            AiState::Default,
                            Substate::DefaultLookingSidewardsForCharly,
                        );
                        self.base.outbox.actor.look_sidewards = Some(
                            if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CharlySorrow, 0..2)
                                != 0
                            {
                                LookDirection::LeftRight
                            } else {
                                LookDirection::RightLeft
                            },
                        );
                    }
                    self.base.sorrow_level = self
                        .base
                        .sorrow_level
                        .saturating_add(self.base.delta_sorrow_level);
                    if self.base.sorrow_level > 1000 {
                        self.base.sorrow_level = 0;
                        // search_charly(sim, ).
                        self.search_charly(sim, ctx, tick);
                    }
                    self.base
                        .launch_timer(parameters_ai::AI_CHECKFOR_TIME_INTERVAL as u32, ctx.frame);
                }
            }

            // Done sweeping eyes, back to baseline looking for Charly.
            Substate::DefaultLookingSidewardsForCharly => {
                if stimulus_type == StimulusType::EventDone {
                    self.set_state(AiState::Default, Substate::DefaultLookingForCharly);
                    self.base.launch_timer(10, ctx.frame);
                }
            }

            // Reacted to detecting Charly; either resume macro or
            // return to duty.
            Substate::DefaultDetectedCharly => {
                if stimulus_type == StimulusType::EventTimer {
                    if self.base.macro_in_progress {
                        self.set_state(AiState::Default, Substate::DefaultInMacro);
                        self.base.execute_next_macro_command(sim, ctx);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }

            // Synchronize with Charly; if he's gone astray, give up;
            // else wait for a SyncCharly event.
            Substate::DefaultSynchronizing => match stimulus_type {
                StimulusType::EventTimer => {
                    // If `synchronize_charly` is not in STATE_DEFAULT
                    // or is dead, return to duty; else re-arm the
                    // timer.
                    let sync_gone = ctx
                        .entity_view(self.base.synchronize_charly)
                        .map(|v| v.ai_state != AiState::Default || !v.is_able_to_fight)
                        .unwrap_or(true);
                    if sync_gone {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    } else {
                        self.base.launch_timer(20, ctx.frame);
                    }
                }
                StimulusType::EventSyncCharly => {
                    if let crate::ai::StimulusInfo::Index(idx) = stimulus.info
                        && idx == self.base.synchronize_index
                    {
                        // Assertion: `macro_in_progress` is true here.
                        self.set_state(AiState::Default, Substate::DefaultInMacro);
                        self.base.execute_next_macro_command(sim, ctx);
                    }
                }
                _ => {}
            },

            // WonderingLooking3 shares the timer-to-sidewards
            // transition with looking 1/2; the next state is
            // WonderingLooking3Sidewards.
            _ => {}
        }
        false
    }

    fn think_expected_wondering_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;
        match self.base.current_substate {
            Substate::WonderingWatching => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            Substate::WonderingLooking1 => {
                if stimulus_type == StimulusType::EventTimer {
                    self.set_state(AiState::Wondering, Substate::WonderingLooking1Sidewards);
                    // Random LR or RL.
                    self.base.outbox.actor.look_sidewards = Some(
                        if crate::sim_rng::u32(
                            sim,
                            crate::sim_rng::RngSite::EnemyWonderingLook,
                            0..2,
                        ) != 0
                        {
                            LookDirection::RightLeft
                        } else {
                            LookDirection::LeftRight
                        },
                    );
                    self.base
                        .launch_timer(parameters_ai::AI_LOOK_TIME as u32, ctx.frame);
                }
            }

            // Sidewards finished: transition to next looking stage,
            // FaceTo((dir+5)%16), launch_timer(30 + rand()&7).
            // Shared body for stages 1 & 2.
            Substate::WonderingLooking1Sidewards => {
                if stimulus_type == StimulusType::EventDone {
                    self.set_state(AiState::Wondering, Substate::WonderingLooking2);
                    let dir = (ctx.direction + 5) & 15;
                    self.base.face_direction(dir, ctx);
                    self.base.launch_timer(
                        30 + crate::sim_rng::u32(
                            sim,
                            crate::sim_rng::RngSite::EnemyWonderingLook,
                            0..8,
                        ),
                        ctx.frame,
                    );
                }
            }

            Substate::WonderingLooking2 => {
                if stimulus_type == StimulusType::EventTimer {
                    self.set_state(AiState::Wondering, Substate::WonderingLooking2Sidewards);
                    // Random LR or RL.
                    self.base.outbox.actor.look_sidewards = Some(
                        if crate::sim_rng::u32(
                            sim,
                            crate::sim_rng::RngSite::EnemyWonderingLook,
                            0..2,
                        ) != 0
                        {
                            LookDirection::RightLeft
                        } else {
                            LookDirection::LeftRight
                        },
                    );
                    self.base
                        .launch_timer(parameters_ai::AI_LOOK_TIME as u32, ctx.frame);
                }
            }

            // Same shared body for Looking2Sidewards: transition to
            // Looking3 (NOT ReturnToDuty), FaceTo((dir+5)%16),
            // 30 + rand()&7 timer.
            Substate::WonderingLooking2Sidewards => {
                if stimulus_type == StimulusType::EventDone {
                    self.set_state(AiState::Wondering, Substate::WonderingLooking3);
                    let dir = (ctx.direction + 5) & 15;
                    self.base.face_direction(dir, ctx);
                    self.base.launch_timer(
                        30 + crate::sim_rng::u32(
                            sim,
                            crate::sim_rng::RngSite::EnemyWonderingLook,
                            0..8,
                        ),
                        ctx.frame,
                    );
                }
            }

            // Money reactiontime:
            // YES branch: clean stale entries, switch state, SUN emoticon
            // (20-tick), Say(GoldYes), GoNear, 5-tick timer.
            // NO branch: CLOUD emoticon (50-tick), Say(GoldNo/VipGoldNo),
            // forget nearby coins, ReturnToDuty(KEEP_EMOTICON).
            Substate::WonderingMoneyReactiontime => {
                if stimulus_type == StimulusType::EventTimer {
                    let want_money = self.answer_question(Question::ShallITakeMoney, ctx);
                    let obj_pos = ctx.entity_position(self.base.interesting_object);
                    let officer_near = obj_pos
                        .map(|p| self.is_any_angry_officer_near(p, tick))
                        .unwrap_or(false);
                    if want_money
                        && let Some(obj_pos) = obj_pos
                        && !officer_near
                    {
                        // Drop destroyed entries.
                        self.clean_up_list_of_seen_money(ctx);
                        self.base.say(Remark::GoldYes);
                        self.set_state(AiState::Wondering, Substate::WonderingApproachingMoney);
                        self.base
                            .set_transient_emoticon(EmoticonType::Sun, 20, ctx.frame);
                        self.go_near(
                            AiState::Wondering,
                            Substate::WonderingApproachingMoney,
                            obj_pos,
                            parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                            GotoFlags::FIND_ACCESSIBLE,
                            ctx,
                        );
                        self.base.launch_timer(5, ctx.frame);
                    } else {
                        self.base
                            .set_transient_emoticon(EmoticonType::Cloud, 50, ctx.frame);
                        if self.is_vip {
                            self.base.say(Remark::VipGoldNo);
                        } else {
                            self.base.say(Remark::GoldNo);
                        }
                        // Clear other-seen-money list + forget nearby
                        // coins so this NPC doesn't re-trigger the
                        // money-want flow this tick.
                        self.other_seen_money.clear();
                        self.forget_all_nearby_coins(ctx);
                        self.return_to_duty(sim, DutyFlags::KEEP_EMOTICON, ctx, tick);
                    }
                }
            }

            Substate::WonderingApproachingMoney => {
                if stimulus_type == StimulusType::EventReachPoint
                    || stimulus_type == StimulusType::EventTimer
                {
                    self.set_state(AiState::Wondering, Substate::WonderingTakingMoney);
                    // Launch a single-element interaction sequence
                    // (Take, me, interesting_object) to trigger the
                    // pick-up animation on the coin.  The engine
                    // launches it at post-think time.
                    let obj = self.base.interesting_object;
                    if obj != 0 {
                        use crate::element::Command;
                        use crate::sequence::{Sequence, SequenceElement};
                        let owner = self.base.owner_entity_id;
                        let antagonist = Some(crate::element::EntityId::Bonus(
                            crate::entity_id::BonusId(obj),
                        ));
                        let mut seq = Sequence::new();
                        seq.append_element(SequenceElement::new_interaction(
                            1,
                            Command::Take,
                            owner,
                            antagonist,
                        ));
                        self.base.outbox.actor.launch_sequences.push(seq);
                    }
                    self.base.launch_timer(30, ctx.frame);
                }
            }

            Substate::WonderingTakingMoney => {
                if stimulus_type == StimulusType::EventTimer {
                    // Check for more money
                    self.set_state(AiState::Wondering, Substate::WonderingWatchingForMoreMoney);
                    self.base.launch_timer(
                        parameters_ai::AI_ARE_THERE_MORE_DOLLARS_LOOKS as u32 * 10,
                        ctx.frame,
                    );
                }
            }

            Substate::WonderingWatchingForMoreMoney => match stimulus_type {
                StimulusType::EventTimer => {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
                // When the LookSidewards sequence finishes, scan for
                // nearby KO'd money-fight victims and either approach
                // to loot or return to duty.
                StimulusType::EventDone => {
                    self.create_list_of_near_money_fight_victims(ctx, tick);

                    while self
                        .money_fight_victims
                        .first()
                        .and_then(|h| ctx.entity_view(*h as HumanHandle))
                        .map(|v| v.looted_after_money_fight)
                        .unwrap_or(false)
                    {
                        self.money_fight_victims.remove(0);
                    }

                    if !self.money_fight_victims.is_empty() {
                        let next = self.money_fight_victims.remove(0);
                        self.base.detected_body = next as HumanHandle;
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SetLootedAfterMoneyFight {
                                target: next,
                                looted: true,
                            },
                        );
                        self.set_state(AiState::Wondering, Substate::WonderingApproachingToLoot);
                        if let Some(view) = ctx.entity_view(next as HumanHandle) {
                            self.base.go_near(
                                view.position,
                                parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                                crate::ai::GotoFlags::empty(),
                                ctx,
                            );
                        }
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
                _ => {}
            },

            // Ale reactiontime: if shall-take-ale and beer is alive:
            // stash beer as object_of_desire, transition to
            // ApproachingAle, set SUN emoticon (20-tick), Say(AleYes),
            // GoNear, save return point, 20-tick timer.  Otherwise
            // CLOUD emoticon (50-tick) + Say(AleNo / VipAleNo) +
            // ReturnToDuty with KEEP_EMOTICON.
            Substate::WonderingAleReactiontime => {
                if stimulus_type == StimulusType::EventTimer {
                    if self.answer_question(Question::ShallITakeAle, ctx)
                        && let Some(obj_pos) = ctx.entity_position(self.base.interesting_object)
                    {
                        self.base.object_of_desire = self.base.interesting_object;
                        self.set_state(AiState::Wondering, Substate::WonderingApproachingAle);
                        self.base
                            .set_transient_emoticon(EmoticonType::Sun, 20, ctx.frame);
                        self.base.say(Remark::AleYes);
                        self.go_near(
                            AiState::Wondering,
                            Substate::WonderingApproachingAle,
                            obj_pos,
                            parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                            GotoFlags::FIND_ACCESSIBLE,
                            ctx,
                        );
                        self.return_to_patrol_point = ctx.position;
                        self.base.launch_timer(20, ctx.frame);
                    } else {
                        self.base
                            .set_transient_emoticon(EmoticonType::Cloud, 50, ctx.frame);
                        if self.is_vip {
                            self.base.say(Remark::VipAleNo);
                        } else {
                            self.base.say(Remark::AleNo);
                        }
                        self.return_to_duty(sim, DutyFlags::KEEP_EMOTICON, ctx, tick);
                    }
                }
            }

            Substate::WonderingApproachingAle => {
                // The TIMER and REACHPOINT arms both gate on
                // `is_beer_still_available`.  On failure (bottle gone
                // or stolen) both paths face the lost position, flip
                // to THUNDERSTORM, switch to `WonderingAleAway`, and
                // arm a 30-tick recovery timer.  On success the
                // TIMER arm re-arms a 20-tick poll, and the
                // REACHPOINT arm launches the drink-ale sequence and
                // transitions to `WonderingDrinkingAle`.
                match stimulus_type {
                    StimulusType::EventTimer => {
                        if let Some(lost_pos) = self.is_beer_still_available(ctx) {
                            self.base.face_position(lost_pos);
                            self.base.set_emoticon(EmoticonType::Thunderstorm);
                            self.set_state(AiState::Wondering, Substate::WonderingAleAway);
                            self.base.launch_timer(30, ctx.frame);
                        } else {
                            self.base.launch_timer(20, ctx.frame);
                        }
                    }
                    StimulusType::EventReachPoint => {
                        if let Some(lost_pos) = self.is_beer_still_available(ctx) {
                            self.base.face_position(lost_pos);
                            self.base.set_emoticon(EmoticonType::Thunderstorm);
                            self.set_state(AiState::Wondering, Substate::WonderingAleAway);
                            self.base.launch_timer(30, ctx.frame);
                        } else {
                            self.set_state(AiState::Wondering, Substate::WonderingDrinkingAle);
                            // Launch a DrinkAle interaction to trigger
                            // the drinking animation on the ale bottle.
                            let obj = self.base.interesting_object;
                            if obj != 0 {
                                use crate::element::Command;
                                use crate::sequence::{Sequence, SequenceElement};
                                let owner = self.base.owner_entity_id;
                                let antagonist = Some(crate::element::EntityId::Bonus(
                                    crate::entity_id::BonusId(obj),
                                ));
                                let mut seq = Sequence::new();
                                seq.append_element(SequenceElement::new_interaction(
                                    1,
                                    Command::DrinkAle,
                                    owner,
                                    antagonist,
                                ));
                                self.base.outbox.actor.launch_sequences.push(seq);
                            }
                            self.base.launch_timer(60, ctx.frame);
                        }
                    }
                    _ => {}
                }
            }

            Substate::WonderingDrinkingAle => {
                if stimulus_type == StimulusType::EventTimer {
                    self.base.blood_alcohol = self.base.blood_alcohol.saturating_add(30);
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            Substate::WonderingAppleSauceInTheVisor => {
                if stimulus_type == StimulusType::EventTimer {
                    // `get_angry_about_apple(seek_position)`: the
                    // apple-origin position stashed when the apple
                    // first landed.
                    let pos = self.base.seek_position;
                    self.get_angry_about_apple(&pos, ctx, tick);
                }
            }

            // Heard whistling: face the source, transition to
            // WatchingWhistling, launch 60-tick timer.  The
            // decide-to-follow logic happens in the WatchingWhistling
            // timer arm — we just stage here.
            Substate::WonderingHeardWhistling => {
                if stimulus_type == StimulusType::EventTimer {
                    self.base.face_position(self.base.seek_position);
                    self.set_state(AiState::Wondering, Substate::WonderingWatchingWhistling);
                    self.base
                        .launch_timer(parameters_ai::AI_FIRST_LOOK_TIME as u32, ctx.frame);
                }
            }

            Substate::WonderingWatchingTowerGuard => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            // ============ SEEKING ============

            // -- Seek-area substates --
            Substate::WonderingLooking3 => {
                if stimulus_type == StimulusType::EventTimer {
                    self.set_state(AiState::Wondering, Substate::WonderingLooking3Sidewards);
                    self.base.outbox.actor.look_sidewards = Some(
                        if crate::sim_rng::u32(
                            sim,
                            crate::sim_rng::RngSite::EnemyWonderingLook,
                            0..2,
                        ) != 0
                        {
                            LookDirection::RightLeft
                        } else {
                            LookDirection::LeftRight
                        },
                    );
                }
            }

            // Done sweeping eyes after awakening/wasp sting; return to duty.
            Substate::WonderingLooking3Sidewards => {
                if stimulus_type == StimulusType::EventDone {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            // Apple reaction: decide whether to chase, else return to duty.
            Substate::WonderingAppleReactiontime => {
                if stimulus_type == StimulusType::EventTimer {
                    // Logic:
                    //   if !ShallIReactOnApple || !ChaseChilds()
                    //     return_to_duty(sim, );
                    //
                    // ShallIReactOnApple outdoor answer:
                    //   soldier_profile_apple > 0
                    let shall_react = self.soldier_profile_apple > 0;
                    let chased = shall_react && self.chase_childs(ctx);
                    if !chased {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }

            // Chase child who threw the apple; panic-run counter
            // drives refreshes.
            Substate::WonderingAppleChasingChild => match stimulus_type {
                StimulusType::EventMyTalk1
                    // antagonist.think(CallYourTalk1)
                    if self.base.antagonist != 0 => {
                        self.base
                            .outbox.reentrant.cross_npc_actions
                            .push(CrossNpcAction::SendStimulus {
                                target: self.base.antagonist,
                                stimulus_type: StimulusType::CallYourTalk1,
                                info: crate::ai::StimulusInfo::None,
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                            });
                    }
                StimulusType::EventTimer => {
                    if self.base.lasting_panic_runs > 0 {
                        self.base.lasting_panic_runs -= 1;
                        // Re-issue `go_near(sim, antagonist_pos, 5, RUN |
                        // DONT_STOP)` from the same substate each
                        // panic tick.  Our Shape 1 contract requires
                        // every movement name its new substate (see
                        // comment at fn go_near above), so route the
                        // refresh through
                        // `WonderingAppleChasingChildWaiting` — its
                        // EventTimer transitions back to ChasingChild.
                        // Issue GoNear from here bundled with the
                        // Waiting transition so the movement order
                        // lives with its new substate.
                        if let Some(view) = ctx.entity_view(self.base.antagonist) {
                            self.go_near(                                AiState::Wondering,
                                Substate::WonderingAppleChasingChildWaiting,
                                view.position,
                                5,
                                crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::DONT_STOP,
                                ctx,
                            );
                        }
                        self.base.launch_timer(10, ctx.frame);
                    } else {
                        self.set_state(AiState::Wondering, Substate::WonderingAppleChasingChildEnd);
                        // Face(antagonist)
                        self.base.face_entity(self.base.antagonist, ctx);
                        self.base.launch_timer(30, ctx.frame);
                    }
                }
                StimulusType::EventReachPoint => {
                    self.set_state(
                        AiState::Wondering,
                        Substate::WonderingAppleChasingChildWaiting,
                    );
                    self.base.launch_timer(10, ctx.frame);
                }
                _ => {}
            },

            // Waiting between chase refreshes.
            Substate::WonderingAppleChasingChildWaiting => {
                if stimulus_type == StimulusType::EventTimer {
                    self.set_state(AiState::Wondering, Substate::WonderingAppleChasingChild);
                    self.base.launch_timer(1, ctx.frame);
                }
            }

            // End of apple chase.
            Substate::WonderingAppleChasingChildEnd => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            // Running for money: race rivals on timer, and on reach,
            // take it or look for more.
            Substate::WonderingRunningForMoney => match stimulus_type {
                StimulusType::EventTimer => {
                    // If another guy is in sight approaching the money,
                    // re-stage RunningForMoney and notify any patrol
                    // chief; otherwise just re-arm the timer.
                    //
                    // `there_is_another_guy_in_sight_approaching_to_money`
                    // walks same-camp soldiers and checks
                    // is_take_money || is_fight_for_money (minus
                    // MoneyReactiontime), not self, and
                    // `is_detecting_180_degrees`.
                    let another_guy_approaching =
                        self.there_is_another_guy_in_sight_approaching_to_money(ctx, tick);
                    if another_guy_approaching {
                        // GoNear(money, AI_STOP_BEFORE_MONEY_DISTANCE,
                        //         RUN | FIND_ACCESSIBLE)
                        //
                        // Shape 1 contract forbids same-substate
                        // re-issue; route through MoneyReactiontime,
                        // whose timer already advances back to
                        // RunningForMoney.
                        if let Some(obj_pos) = ctx.entity_position(self.base.interesting_object) {
                            self.go_near(
                                AiState::Wondering,
                                Substate::WonderingMoneyReactiontime,
                                obj_pos,
                                parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                                crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::FIND_ACCESSIBLE,
                                ctx,
                            );
                            self.base.launch_timer(1, ctx.frame);
                        }
                        // If my patrol chief is an officer whose 180°
                        // detects me, fire EVENT_SEES_BRAWL at them.
                        if let Some(chief_id) = self.base.patrol_chief
                            && let Some(chief_view) = ctx.entity_view(chief_id.index())
                            && chief_view.is_soldier()
                            && chief_view.is_able_to_fight
                            && chief_view.rank == ProfileRank::Officer
                        {
                            self.base.outbox.reentrant.cross_npc_actions.push(
                                CrossNpcAction::SendStimulus {
                                    target: chief_id.index(),
                                    stimulus_type: StimulusType::EventSeesBrawl,
                                    info: crate::ai::StimulusInfo::Human(
                                        self.base.me as HumanHandle,
                                    ),
                                    fallback_to_sender: None,
                                    to_whole_patrol: false,
                                },
                            );
                        }
                    } else {
                        self.base.launch_timer(20, ctx.frame);
                    }
                }
                StimulusType::EventReachPoint => {
                    // If money is still active and within 25 units
                    // (MaxNorm), take it + notify friends with
                    // EventObjectAway; else look for more.
                    let obj = self.base.interesting_object;
                    let close_enough = ctx
                        .entity_position(obj)
                        .map(|p| {
                            let dx = (p.x - ctx.position.x).abs();
                            let dy = (p.y - ctx.position.y).abs();
                            dx.max(dy) < 25.0
                        })
                        .unwrap_or(false);
                    if obj != 0 && close_enough {
                        // StopAll + Take sequence.
                        self.base.stop_all();
                        use crate::element::Command;
                        use crate::sequence::{Sequence, SequenceElement};
                        let owner = self.base.owner_entity_id;
                        let antagonist = Some(crate::element::EntityId::Bonus(
                            crate::entity_id::BonusId(obj),
                        ));
                        let mut seq = Sequence::new();
                        seq.append_element(SequenceElement::new_interaction(
                            1,
                            Command::Take,
                            owner,
                            antagonist,
                        ));
                        self.base.outbox.actor.launch_sequences.push(seq);

                        // Notify any same-camp soldier whose substate
                        // is take-money or fight-for-money with
                        // EventObjectAway carrying a StolenObject.
                        let stolen = crate::ai::StolenObject {
                            object: obj as crate::ai::ObjectHandle,
                            thief: self.base.me,
                        };
                        for cs in tick.camp_soldiers.iter() {
                            if cs.handle == self.base.me {
                                continue;
                            }
                            if cs.ai_substate.is_take_money() || cs.ai_substate.is_fight_for_money()
                            {
                                self.base.outbox.reentrant.cross_npc_actions.push(
                                    CrossNpcAction::SendStimulus {
                                        target: cs.handle,
                                        stimulus_type: StimulusType::EventObjectAway,
                                        info: crate::ai::StimulusInfo::Stolen(stolen),
                                        fallback_to_sender: None,
                                        to_whole_patrol: false,
                                    },
                                );
                            }
                        }

                        self.set_state(AiState::Wondering, Substate::WonderingTakingMoney);
                    } else {
                        // Transition to WatchingForMoreMoney + look
                        // sidewards.
                        self.set_state(AiState::Wondering, Substate::WonderingWatchingForMoreMoney);
                        self.base.outbox.actor.look_sidewards = Some(LookDirection::LeftRight);
                    }
                }
                _ => {}
            },

            // Brawl reaction: set mood, approach the friend in
            // trouble, run on timer tick.
            Substate::WonderingBrawlReactiontime => {
                if stimulus_type == StimulusType::EventTimer {
                    self.set_state(AiState::Wondering, Substate::WonderingBrawlApproaching);
                    self.base.set_emoticon(EmoticonType::Thunderstorm);
                    self.base.say(Remark::GoldBrawl);
                    //   seek_position = friend_in_trouble.position;
                    //   GoNear(seek_position, AI_HIT_DISTANCE, RUN);
                    if let Some(view) = ctx.entity_view(self.base.friend_in_trouble) {
                        self.base.seek_position = view.position;
                        self.base.go_near(
                            view.position,
                            parameters_ai::AI_HIT_DISTANCE,
                            crate::ai::GotoFlags::RUN,
                            ctx,
                        );
                    }
                    self.base.launch_timer(1, ctx.frame);
                }
            }

            // Brawl approach: refresh chase on timer; on reach,
            // attempt the hit.
            Substate::WonderingBrawlApproaching => match stimulus_type {
                StimulusType::EventTimer => {
                    // If target moved > 3 units from the seek position,
                    // update seek position and re-issue GoNear.
                    // Otherwise re-arm the timer.
                    if let Some(view) = ctx.entity_view(self.base.friend_in_trouble) {
                        let dx = (self.base.seek_position.x - view.position.x).abs();
                        let dy = (self.base.seek_position.y - view.position.y).abs();
                        if dx.max(dy) > 3.0 {
                            self.base.seek_position = view.position;
                            self.base.go_near(
                                view.position,
                                parameters_ai::AI_HIT_DISTANCE,
                                crate::ai::GotoFlags::RUN,
                                ctx,
                            );
                        }
                    }
                    self.base.launch_timer(1, ctx.frame);
                }
                StimulusType::EventReachPoint => {
                    // Check friend_in_trouble substate:
                    // - if asleep (Sleeping) → skip hit, go to hitting;
                    // - if distance > AI_HIT_DISTANCE+3 → re-issue GoNear;
                    // - else actually transition to hitting.
                    let friend_sleeping = ctx
                        .entity_view(self.base.friend_in_trouble)
                        .map(|v| v.ai_state == AiState::Sleeping)
                        .unwrap_or(false);
                    if friend_sleeping {
                        // Drop the sleeping friend from
                        // `money_fight_enemies` so subsequent brawl
                        // arms don't keep re-targeting a KO'd soldier
                        // (and so `wants_to_continue_money_fight`'s
                        // size-based threshold isn't skewed).
                        let fit = self.base.friend_in_trouble as NpcHandle;
                        self.money_fight_enemies.retain(|h| *h != fit);
                        self.base.friend_in_trouble = 0;
                    }
                    self.set_state(AiState::Wondering, Substate::WonderingBrawlHitting);
                }
                _ => {}
            },

            // Brawl hit resolution; civilians panic, chase chain continues.
            Substate::WonderingBrawlHitting => {
                if stimulus_type == StimulusType::EventDone {
                    // Scan camp for an officer who might hear/see me
                    // and alert them with EventSeesBrawl.
                    self.maybe_officer_sees_me_fighting(ctx, tick);
                    // Broadcast civilian panic for anyone in view
                    // radius.  Queued via `pending_broadcast_panic`.
                    self.nearby_civilians_panic();

                    // Remove KO'd target from the enemy list.
                    if self.base.friend_in_trouble != 0 {
                        let fit = self.base.friend_in_trouble as NpcHandle;
                        let is_unconscious = ctx
                            .entity_view(fit as HumanHandle)
                            .map(|v| v.is_unconscious)
                            .unwrap_or(false);
                        if is_unconscious {
                            self.money_fight_enemies.retain(|h| *h != fit);
                        }
                    }

                    // Refresh the enemy list if we've run out —
                    // picks up any same-camp soldier that joined the
                    // brawl after our initial snapshot.
                    if self.money_fight_enemies.is_empty() {
                        self.create_new_list_of_money_fight_enemies(tick, ctx);
                    }

                    // Morale-gated continue-or-stop.
                    if !self.wants_to_continue_money_fight(tick, ctx) {
                        self.money_fight_enemies.clear();
                        // stop_brawling_and_collect_money().
                        self.stop_brawling_and_collect_money(ctx, tick);
                    } else {
                        let fit_ok = self.base.friend_in_trouble != 0
                            && !ctx
                                .entity_view(self.base.friend_in_trouble)
                                .map(|v| v.is_unconscious)
                                .unwrap_or(true);
                        if fit_ok {
                            self.set_state(
                                AiState::Wondering,
                                Substate::WonderingBrawlReactiontime,
                            );
                            self.base.face_entity(self.base.friend_in_trouble, ctx);
                            self.base.launch_timer(30, ctx.frame);
                        } else if let Some(next) = self.get_nearest_money_fight_enemy(ctx) {
                            self.base.friend_in_trouble = next as HumanHandle;
                            self.set_state(
                                AiState::Wondering,
                                Substate::WonderingBrawlReactiontime,
                            );
                            self.base.launch_timer(10, ctx.frame);
                        } else {
                            // stop_brawling_and_collect_money().
                            self.stop_brawling_and_collect_money(ctx, tick);
                        }
                    }
                }
            }

            // Brawl-got-hit: pivot to BrawlRecovering, register
            // attacker as new money-fight enemy, set thunderstorm
            // emoticon. If the NPC is lying, queue StandUp; otherwise
            // self-fire EventDone so BrawlRecovering immediately picks
            // the next victim.
            Substate::WonderingBrawlGotHit => {
                if stimulus_type == StimulusType::EventDone {
                    self.set_state(AiState::Wondering, Substate::WonderingBrawlRecovering);
                    // maybe_officer_sees_me_fighting().
                    self.maybe_officer_sees_me_fighting(ctx, tick);
                    // SetEmoticon(Thunderstorm).
                    self.base.set_emoticon(EmoticonType::Thunderstorm);
                    // Insert friend_in_trouble into money_fight_enemies
                    // (asserts soldier + non-self).
                    let fit = self.base.friend_in_trouble;
                    if fit != 0 && fit != self.base.me {
                        let is_soldier = ctx
                            .entity_view(fit)
                            .map(|v| v.is_soldier())
                            .unwrap_or(false);
                        if is_soldier && !self.money_fight_enemies.contains(&(fit as NpcHandle)) {
                            self.money_fight_enemies.push(fit as NpcHandle);
                        }
                    }
                    // If lying, launch StandUp; else recurse
                    // Think(EventDone) into BrawlRecovering.
                    if ctx.posture == crate::element::Posture::Lying {
                        self.base.stop_all();
                        self.base
                            .outbox
                            .actor
                            .launch_commands
                            .push(crate::element::Command::StandUp);
                    } else {
                        // Self-fire EventDone so the new
                        // BrawlRecovering substate picks up the next
                        // victim immediately on this same tick.
                        self.base.fire_self_stimulus(StimulusType::EventDone);
                    }
                }
            }

            // Brawl recovery: go punch the next enemy, or stop brawling.
            Substate::WonderingBrawlRecovering => {
                if stimulus_type == StimulusType::EventDone {
                    // Pick nearest money-fight enemy and approach.
                    if let Some(next) = self.get_nearest_money_fight_enemy(ctx) {
                        self.base.friend_in_trouble = next as HumanHandle;
                        self.set_state(AiState::Wondering, Substate::WonderingBrawlApproaching);
                        if let Some(view) = ctx.entity_view(next as HumanHandle) {
                            self.base.go_near(
                                view.position,
                                parameters_ai::AI_HIT_DISTANCE,
                                crate::ai::GotoFlags::RUN,
                                ctx,
                            );
                        }
                        // maybe_officer_sees_me_fighting().
                        self.maybe_officer_sees_me_fighting(ctx, tick);
                    } else {
                        // stop_brawling_and_collect_money().
                        self.stop_brawling_and_collect_money(ctx, tick);
                    }
                }
            }

            // Reached looting body.  Either re-transition to loot
            // (distant), flag a tied body, or kick off the SEARCH
            // sequence.
            Substate::WonderingApproachingToLoot => {
                if stimulus_type == StimulusType::EventReachPoint {
                    let body = self.base.detected_body;
                    let view = ctx.entity_view(body);
                    let (body_pos, is_tied) = view
                        .map(|v| (v.position, v.posture == crate::element::Posture::Tied))
                        .unwrap_or((Position::default(), false));
                    let dx = body_pos.x - ctx.position.x;
                    let dy = body_pos.y - ctx.position.y;
                    let dist = dx.abs().max(dy.abs());
                    if body == 0 || dist > 100.0 {
                        // Too far — let Looting handle re-entry.
                        self.set_state(AiState::Wondering, Substate::WonderingLooting);
                        // Kick the state machine via a 1-tick timer;
                        // the Looting arm handles the follow-up.  We
                        // can't re-enter `think()` from inside an arm,
                        // so fall back to a short timer that reaches
                        // the same code path.
                        self.base.launch_timer(1, ctx.frame);
                    } else if is_tied {
                        // Spot the tied body and transition to
                        // body-seek; emit the reconnaissance report
                        // update.
                        self.base.my_reconnaissance_report.add_seen_body(body);
                        self.base
                            .my_reconnaissance_report
                            .update(ReportType::Body, body_pos);
                        self.set_state(AiState::Seeking, Substate::SeekingBody);
                        // Re-issue Think(EventReachPoint) via a 1-tick
                        // timer (see comment above).
                        self.base.launch_timer(1, ctx.frame);
                    } else {
                        // Start SEARCH sequence, transition to Looting.
                        use crate::element::Command;
                        use crate::sequence::{Sequence, SequenceElement};
                        self.old_money = ctx
                            .entity_view(self.base.me)
                            .map(|v| v.current_money.min(u16::MAX as u32) as u16)
                            .unwrap_or(0);
                        self.set_state(AiState::Wondering, Substate::WonderingLooting);
                        self.base.stop_all();
                        let owner = self.base.owner_entity_id;
                        let antagonist = Some(crate::element::EntityId::Soldier(
                            crate::entity_id::SoldierId(body),
                        ));
                        let mut seq = Sequence::new();
                        seq.append_element(SequenceElement::new_interaction(
                            1,
                            Command::SearchCmd,
                            owner,
                            antagonist,
                        ));
                        self.base.outbox.actor.launch_sequences.push(seq);
                    }
                }
            }

            // Looting: inspect gain, move to next victim or return to duty.
            Substate::WonderingLooting => {
                if stimulus_type == StimulusType::EventDone {
                    let current_money = ctx
                        .entity_view(self.base.me)
                        .map(|v| v.current_money.min(u16::MAX as u32) as u16)
                        .unwrap_or(0);
                    if current_money > self.old_money {
                        self.base
                            .set_transient_emoticon(EmoticonType::Sun, 20, ctx.frame);
                        self.base.say(Remark::SearchingSoldierGold);
                    } else {
                        self.base
                            .set_transient_emoticon(EmoticonType::Cloud, 20, ctx.frame);
                        self.base.say(Remark::SearchingSoldierNothing);
                    }

                    while self
                        .money_fight_victims
                        .first()
                        .and_then(|h| ctx.entity_view(*h as HumanHandle))
                        .map(|v| v.looted_after_money_fight)
                        .unwrap_or(false)
                    {
                        self.money_fight_victims.remove(0);
                    }
                    if !self.money_fight_victims.is_empty() {
                        let next = self.money_fight_victims.remove(0);
                        self.base.detected_body = next as HumanHandle;
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SetLootedAfterMoneyFight {
                                target: next,
                                looted: true,
                            },
                        );
                        self.set_state(AiState::Wondering, Substate::WonderingApproachingToLoot);
                        if let Some(view) = ctx.entity_view(next as HumanHandle) {
                            self.base.go_near(
                                view.position,
                                parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                                crate::ai::GotoFlags::empty(),
                                ctx,
                            );
                        }
                    } else {
                        self.return_to_duty(sim, DutyFlags::KEEP_EMOTICON, ctx, tick);
                    }
                }
            }

            // Beer went away: try next remembered beer, else return to duty.
            Substate::WonderingAleAway => {
                if stimulus_type == StimulusType::EventTimer {
                    if !self.other_seen_ale.is_empty() {
                        // Remember next beer as object of desire.
                        let next = self.other_seen_ale.remove(0);
                        self.base.object_of_desire = next;
                        self.base.interesting_object = next;
                        // SetState(Wondering, ApproachingAle).
                        self.set_state(AiState::Wondering, Substate::WonderingApproachingAle);
                        // GoNear(obj_pos, AI_STOP_BEFORE_MONEY_DISTANCE, FIND_ACCESSIBLE)
                        if let Some(pos) = ctx.entity_position(next) {
                            self.base.go_near(
                                pos,
                                parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                                crate::ai::GotoFlags::FIND_ACCESSIBLE,
                                ctx,
                            );
                        }
                        // Remember patrol return point
                        self.return_to_patrol_point = ctx.position;
                        // Quick recheck
                        self.base.launch_timer(1, ctx.frame);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }

            // Officer sees brawl: close distance, clear emoticon.
            Substate::WonderingOfficerSeeingBrawl => {
                if stimulus_type == StimulusType::EventTimer {
                    self.set_state(
                        AiState::Wondering,
                        Substate::WonderingOfficerApproachingBrawl,
                    );
                    self.base.set_emoticon(EmoticonType::None);
                    // GoNear(friend_in_trouble.position, 100);
                    if let Some(view) = ctx.entity_view(self.base.friend_in_trouble) {
                        self.base
                            .go_near(view.position, 100, crate::ai::GotoFlags::empty(), ctx);
                    }
                }
            }

            // Officer reached the brawl; enter finish-brawl state,
            // set thunderstorm mood.
            Substate::WonderingOfficerApproachingBrawl => {
                match stimulus_type {
                    StimulusType::EventReachPoint => {
                        // If already talking, delay finish.
                        if self.base.current_remark != Remark::TheSoundOfSilence {
                            self.base.launch_timer(50, ctx.frame);
                        } else {
                            self.begin_finishing_brawl(ctx, tick);
                        }
                    }
                    StimulusType::EventTimer => {
                        self.begin_finishing_brawl(ctx, tick);
                    }
                    _ => {}
                }
            }

            // Finishing-brawl orchestration: chain CallYourTalk1..3,
            // then timer dismisses soldiers and waits on the antagonist.
            Substate::WonderingOfficerFinishingBrawl => match stimulus_type {
                StimulusType::EventTimer | StimulusType::EventMyTalk2 => {
                    // forget_all_nearby_coins().
                    self.forget_all_nearby_coins(ctx);
                    // Walk list_us, send ReturnToDuty to each soldier
                    // that isn't the antagonist.
                    let antagonist = self.base.antagonist;
                    let us: Vec<HumanHandle> = self
                        .base
                        .list_us
                        .iter()
                        .copied()
                        .filter(|h| *h != antagonist && *h != self.base.me)
                        .collect();
                    for target in us {
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                target,
                                stimulus_type: StimulusType::EventReturnToDuty,
                                info: crate::ai::StimulusInfo::None,
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                            },
                        );
                    }
                    self.base.list_us.clear();

                    // CallCleanUpAfterBrawl to antagonist.
                    if antagonist != 0 {
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                target: antagonist,
                                stimulus_type: StimulusType::CallCleanUpAfterBrawl,
                                info: crate::ai::StimulusInfo::None,
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                            },
                        );
                        self.set_state(
                            AiState::Wondering,
                            Substate::WonderingOfficerFinishingBrawlWaiting,
                        );
                        self.base.launch_timer(10, ctx.frame);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
                StimulusType::CallYourTalk3 => {
                    self.base.say(Remark::OfficerEndsConversation);
                }
                _ => {}
            },

            // Keep waiting while antagonist still
            // approaching/awakening a victim; else end.
            Substate::WonderingOfficerFinishingBrawlWaiting => {
                if stimulus_type == StimulusType::EventTimer {
                    // If antagonist is still approaching or awakening
                    // the brawl victim, re-arm timer; else end.
                    let still_waiting = ctx
                        .entity_view(self.base.antagonist)
                        .map(|v| {
                            matches!(
                                v.ai_substate,
                                Substate::WonderingApproachingBrawlVictim
                                    | Substate::WonderingAwakenBrawlVictim
                            )
                        })
                        .unwrap_or(false);
                    if still_waiting {
                        self.base.launch_timer(10, ctx.frame);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }

            // Soldier side of the "officer finished brawl" lecture:
            // 3-variant excuse speeches until the timer fires.
            Substate::WonderingSoldierLookingOfficerWhoFinishedBrawl => {
                match stimulus_type {
                    StimulusType::EventTimer => {
                        // forget_all_nearby_coins(); return_to_duty(sim, );
                        self.forget_all_nearby_coins(ctx);
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                    StimulusType::EventMyTalk1
                    | StimulusType::EventMyTalk2
                    | StimulusType::EventMyTalk3 => {
                        self.base.set_emoticon(EmoticonType::None);
                        // antagonist.think(CallYourTalk1).
                        // Note: always forward as CallYourTalk1
                        // regardless of which MyTalk variant
                        // triggered — the 3 cycle variants just vary
                        // which BadExcuse sample plays; the callback
                        // is always CallYourTalk1 on the officer.
                        if self.base.antagonist != 0 {
                            self.base.outbox.reentrant.cross_npc_actions.push(
                                CrossNpcAction::SendStimulus {
                                    target: self.base.antagonist,
                                    stimulus_type: StimulusType::CallYourTalk1,
                                    info: crate::ai::StimulusInfo::None,
                                    fallback_to_sender: None,
                                    to_whole_patrol: false,
                                },
                            );
                        }
                    }
                    _ => {}
                }
            }

            // Listening after whistling sound: decide whether to
            // investigate or bail.
            Substate::WonderingWatchingWhistling => {
                if stimulus_type == StimulusType::EventTimer {
                    // ShallIFollowWhistle outdoor arm:
                    //   whistle > 1 && company_number != 100
                    let shall_follow =
                        self.soldier_profile_whistle > 1 && self.company_number != 100;
                    if !shall_follow {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        return false;
                    }

                    // Rank branch.
                    let rank = self.get_rank();
                    let mut near_officer: Option<NpcHandle> = None;
                    let mut look_for_soldiers = false;
                    match rank {
                        ProfileRank::Soldier => {
                            // `near_officer_who_is_wondering_about_the_same_noise`:
                            // scan same-camp officers who are able to
                            // fight, not script-locked, whose live
                            // `seek_position` matches mine (same
                            // noise), within 360° detection range.
                            // `has_as_seek_position(pos)` is a literal
                            // exact equality including layer / sector
                            // / x / y.
                            let my_seek = self.base.seek_position;
                            near_officer = tick
                                .camp_soldiers
                                .iter()
                                .find(|cs| {
                                    cs.rank == ProfileRank::Officer
                                        && cs.is_able_to_fight
                                        && !cs.script_locked
                                        && cs.seek_position.x == my_seek.x
                                        && cs.seek_position.y == my_seek.y
                                        && cs.seek_position.level == my_seek.level
                                        && self
                                            .is_detecting_360_degrees(cs.handle as HumanHandle, ctx)
                                })
                                .map(|cs| cs.handle);
                        }
                        ProfileRank::Officer => {
                            // ShallISendOutSoldier outdoor arm:
                            //   initiative < 50 || patrol.len() > 0
                            look_for_soldiers = self.soldier_profile_initiative < 50
                                || !self.base.patrol.is_empty();
                        }
                        ProfileRank::Knight | ProfileRank::None => {}
                    }

                    if let Some(officer) = near_officer {
                        // Face officer + transition to
                        // DefaultLookingOfficerForAdvice + ? emoticon
                        // + 100-tick timer.
                        self.base.face_entity(officer, ctx);
                        self.set_state(AiState::Default, Substate::DefaultLookingOfficerForAdvice);
                        self.base.set_emoticon(EmoticonType::QuestionMark);
                        self.base.launch_timer(100, ctx.frame);
                    } else if look_for_soldiers {
                        // OfficerLookForSoldier(ReportType::Noise).
                        self.officer_look_for_soldier(ReportType::Noise, ctx, tick);
                    } else {
                        // SeekArea(seek_position,
                        //   (MAX_WHISTLE_SEEK_RADIUS * (whistle - 2)) / 98,
                        //   LOCATION_FIRST | WALKING);
                        const MAX_WHISTLE_SEEK_RADIUS: u32 = 400;
                        let whistle = self.soldier_profile_whistle as u32;
                        let radius = if whistle >= 2 {
                            ((MAX_WHISTLE_SEEK_RADIUS * (whistle - 2)) / 98) as u16
                        } else {
                            0
                        };
                        self.seek_area(
                            sim,
                            self.base.seek_position,
                            radius,
                            SeekFlags::LOCATION_FIRST | SeekFlags::WALKING,
                            0,
                            global,
                            ctx,
                            tick,
                        );
                    }
                }
            }

            // Watcher finishes looking at tower guard; back to duty.
            Substate::WonderingApproachingBrawlVictim => {
                if stimulus_type == StimulusType::EventReachPoint {
                    use crate::element::Command;
                    use crate::sequence::{Sequence, SequenceElement};
                    self.set_state(AiState::Wondering, Substate::WonderingAwakenBrawlVictim);
                    self.base.stop_all();
                    let owner = self.base.owner_entity_id;
                    let body = self.base.detected_body;
                    if body != 0 {
                        let antagonist = Some(crate::element::EntityId::Soldier(
                            crate::entity_id::SoldierId(body),
                        ));
                        let mut seq = Sequence::new();
                        seq.append_element(SequenceElement::new_interaction(
                            1,
                            Command::WakeUp,
                            owner,
                            antagonist,
                        ));
                        self.base.outbox.actor.launch_sequences.push(seq);
                    }
                }
            }

            // Done awakening the victim: move to the next fight victim.
            Substate::WonderingAwakenBrawlVictim => {
                if stimulus_type == StimulusType::EventDone {
                    self.awake_next_money_fight_victim_if_any(sim, ctx, tick);
                }
            }

            // Attacker returns to another PC after menacing: begin a
            // swordfight.
            _ => {}
        }
        false
    }

    fn think_expected_seeking_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;
        match self.base.current_substate {
            Substate::SeekingSeekpoint => {
                if stimulus_type == StimulusType::EventReachPoint
                    && self.actual_seek_point.is_some()
                {
                    self.reached_seek_point(sim, global, ctx, tick);
                }
            }

            Substate::SeekingSeekpointWatching => {
                if stimulus_type == StimulusType::EventTimer {
                    // Random LR/RL.
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingSeekpointWatchingSidewards,
                    );
                    self.base.outbox.actor.look_sidewards = Some(
                        if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::EnemySeekLook, 0..2)
                            != 0
                        {
                            LookDirection::LeftRight
                        } else {
                            LookDirection::RightLeft
                        },
                    );
                }
            }

            Substate::SeekingSeekpointWatchingSidewards => {
                if stimulus_type == StimulusType::EventDone
                    || stimulus_type == StimulusType::EventTimer
                {
                    // Check if more directions to look
                    if let Some(&dir) = self.seek_point_view_directions.first() {
                        self.seek_point_view_directions.remove(0);
                        self.base.face_direction(dir, ctx);
                        self.base.number_of_looks = 0;
                        self.set_state(AiState::Seeking, Substate::SeekingSeekpointWatching);
                        self.base
                            .launch_timer(parameters_ai::AI_SEEKPOINT_LOOK_TIME as u32, ctx.frame);
                    } else {
                        // No directions left — move to next seek point
                        self.seek_next_point(sim, global, ctx, tick);
                    }
                }
            }

            Substate::SeekingSeekpointPassedAmbushPointLeft => {
                match stimulus_type {
                    StimulusType::EventReachPoint => {
                        self.set_state(AiState::Seeking, Substate::SeekingSeekpoint);
                        // Original calls Think(EVENT_REACHPOINT) re-entrantly
                        // here; do the same work inline instead of synthesizing
                        // a one-frame timer.
                        if self.actual_seek_point.is_some() {
                            self.reached_seek_point(sim, global, ctx, tick);
                        }
                    }
                    StimulusType::EventTimer => {
                        self.base.stop_all();
                        self.set_state(
                            AiState::Seeking,
                            Substate::SeekingSeekpointCheckingAmbushPoint,
                        );
                        // Look LEFT.
                        self.base.outbox.actor.look_sidewards = Some(LookDirection::Left);
                    }
                    _ => {}
                }
            }

            Substate::SeekingSeekpointPassedAmbushPointRight => {
                match stimulus_type {
                    StimulusType::EventReachPoint => {
                        self.set_state(AiState::Seeking, Substate::SeekingSeekpoint);
                        if self.actual_seek_point.is_some() {
                            self.reached_seek_point(sim, global, ctx, tick);
                        }
                    }
                    StimulusType::EventTimer => {
                        self.base.stop_all();
                        self.set_state(
                            AiState::Seeking,
                            Substate::SeekingSeekpointCheckingAmbushPoint,
                        );
                        // Look RIGHT.
                        self.base.outbox.actor.look_sidewards = Some(LookDirection::Right);
                    }
                    _ => {}
                }
            }

            Substate::SeekingSeekpointCheckingAmbushPoint => {
                if stimulus_type == StimulusType::EventDone {
                    // Resume walking to seek point
                    let goto_flags = if self.seek_flags.contains(SeekFlags::WALKING) {
                        GotoFlags::empty()
                    } else {
                        GotoFlags::RUN
                    };
                    let seek_point_id = self
                        .actual_seek_point
                        .expect("ambush-point check lost its actual seek point");
                    let seek_position = resolve_seek_point_id(
                        seek_point_id,
                        &self.personal_seek_point_1,
                        &self.personal_seek_point_2,
                        global,
                    )
                    .unwrap_or_else(|| {
                        panic!("actual seek point {seek_point_id:?} no longer resolves")
                    })
                    .position;
                    self.go_to(
                        AiState::Seeking,
                        Substate::SeekingSeekpoint,
                        seek_position,
                        goto_flags,
                        ctx,
                    );
                }
            }

            // -- Beggar identification substates --
            Substate::SeekingSeekpointApproachingBeggar => {
                // Soldier is walking toward the beggar's last known
                // position (set by seek_next_point → go_near).
                // On arrival, stop and begin identification.
                if stimulus_type == StimulusType::EventReachPoint {
                    // The reference checks MaxNormDistance(beggar) < 100;
                    // since we used go_near(sim, pos, 50), reaching means
                    // we're close enough. If we somehow aren't, resume.
                    self.base.stop_all();
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingSeekpointIdentifyingBeggar1,
                    );
                    self.base.say(Remark::ControlsBeggar);

                    // Face toward the beggar's position.
                    self.base.face_position(self.base.seek_position);

                    // Archers equip bow; melee soldiers menace.
                    // TurnFast + EquipBow for archers, StartMenace for melee.
                    // Timer = 50 (NPC target) / 100 (PC target) for archers, 30 for melee.
                    if self.is_archer() {
                        self.base
                            .outbox
                            .actor
                            .launch_commands
                            .push(crate::element::Command::TurnFast);
                        self.base
                            .outbox
                            .actor
                            .launch_commands
                            .push(crate::element::Command::EquipBow);
                        let timer = if self.beggar_is_npc { 50 } else { 100 };
                        self.base.launch_timer(timer, ctx.frame);
                    } else {
                        self.base
                            .outbox
                            .actor
                            .launch_commands
                            .push(crate::element::Command::TurnFast);
                        self.base
                            .outbox
                            .actor
                            .launch_commands
                            .push(crate::element::Command::StartMenace);
                        self.base.launch_timer(30, ctx.frame);
                    }
                }
            }

            Substate::SeekingSeekpointIdentifyingBeggar1 => {
                // First inspection phase: timer fires after the
                // menace/equip-bow animation completes.
                if stimulus_type == StimulusType::EventTimer {
                    if self.beggar_is_npc {
                        // Real beggar: NPC shows face and identifies
                        // themselves. Transition to phase 2 (wait,
                        // then resume seeking).
                        // Launch a `BeggarShowFace` sequence element on
                        // the beggar via `pending_launch_on_target`,
                        // which carries (target, cmd) to the
                        // engine-side sequence-manager drain.
                        self.base.outbox.actor.launch_on_target.push((
                            self.beggar_to_examine,
                            crate::element::Command::BeggarShowFace,
                        ));
                        self.set_state(
                            AiState::Seeking,
                            Substate::SeekingSeekpointIdentifyingBeggar2,
                        );
                        self.base.launch_timer(50, ctx.frame);
                    } else {
                        // Disguised PC detected! Set as primary target
                        // and begin combat.
                        self.base.primary_target = self.beggar_to_examine;
                        self.list_them.clear();
                        self.list_them.push(self.beggar_to_examine);

                        if self.is_archer() {
                            // False beggar stands up via `LeaveBeggar`,
                            // then the archer transitions to
                            // AttackingBowShooting and shoots.
                            self.base.outbox.actor.launch_on_target.push((
                                self.beggar_to_examine,
                                crate::element::Command::LeaveBeggar,
                            ));
                            self.set_state(AiState::Attacking, Substate::AttackingBowShooting);
                            self.shoot_arrow_at(self.base.primary_target, ctx, tick);
                        } else {
                            // Melee: call PC for duel.
                            self.begin_swordfight(ctx, tick);
                        }
                    }
                }
            }

            Substate::SeekingSeekpointIdentifyingBeggar2 => {
                // Second phase (NPC path only): the real beggar has
                // identified themselves. Timer fires → resume seeking.
                if stimulus_type == StimulusType::EventTimer {
                    self.seek_next_point(sim, global, ctx, tick);
                }
            }

            // Pre-reactiontime gates whether to investigate himself
            // or just watch:
            //   - SOLDIER/KNIGHT: AnswerQuestion(ShallIFollowSteps)
            //   - OFFICER: only if no patrol *and* close enough to noise.
            // If "do not investigate yourself" → JustWatching, else
            // HeardstepsReactiontime.  Both arms set Q-mark + face + 60-tick
            // timer.
            Substate::SeekingHeardstepsPreReactiontime => {
                if stimulus_type == StimulusType::EventTimer {
                    let do_not_investigate = match self.get_rank() {
                        ProfileRank::Officer => {
                            let me = crate::element::EntityId::Soldier(
                                crate::entity_id::SoldierId(self.base.me),
                            );
                            let has_patrol = tick
                                .camp_soldiers
                                .iter()
                                .any(|cs| cs.patrol_chief == Some(me));
                            let dx = (ctx.position.x - self.base.seek_position.x).abs();
                            let dy = (ctx.position.y - self.base.seek_position.y).abs();
                            const OFFICER_EXAMINE_NOISE_HIMSELF_DISTANCE: f32 = 100.0;
                            has_patrol || dx.max(dy) > OFFICER_EXAMINE_NOISE_HIMSELF_DISTANCE
                        }
                        // Soldier / knight: defer to ShallIFollowSteps.
                        _ => !self.answer_question(Question::ShallIFollowSteps, ctx),
                    };
                    self.base.set_emoticon(EmoticonType::QuestionMark);
                    if do_not_investigate {
                        self.set_state(AiState::Seeking, Substate::SeekingJustWatching);
                        self.base
                            .face_position_3d_with_ctx(self.base.seek_position, ctx);
                    } else {
                        self.base
                            .face_position_3d_with_ctx(self.base.seek_position, ctx);
                        self.set_state(AiState::Seeking, Substate::SeekingHeardstepsReactiontime);
                    }
                    self.base
                        .launch_timer(parameters_ai::AI_FIRST_LOOK_TIME as u32, ctx.frame);
                }
            }

            Substate::SeekingHeardstepsReactiontime => {
                if stimulus_type == StimulusType::EventTimer {
                    self.go_to(
                        AiState::Seeking,
                        Substate::SeekingHeardsteps,
                        self.base.seek_position,
                        GotoFlags::empty(),
                        ctx,
                    );
                    self.base.launch_timer(200, ctx.frame);
                }
            }

            Substate::SeekingHeardsteps => {
                match stimulus_type {
                    StimulusType::EventReachPoint | StimulusType::EventTimer => {
                        // Arrived at noise source — look around
                        self.set_state(AiState::Seeking, Substate::SeekingJustWatching);
                        self.base.face_position(self.base.seek_position);
                        self.base
                            .launch_timer(parameters_ai::AI_FIRST_LOOK_TIME as u32, ctx.frame);
                    }
                    _ => {}
                }
            }

            Substate::SeekingJustWatching => {
                if stimulus_type == StimulusType::EventTimer {
                    self.set_state(AiState::Seeking, Substate::SeekingJustWatchingSidewards);
                    // Randomly pick a two-step head-turn direction.
                    // The engine consumes `pending_look_sidewards`
                    // into a sequence of LookLeft / LookRight commands
                    // at post-think time.
                    self.base.outbox.actor.look_sidewards = Some(
                        if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::EnemySeekLook, 0..2)
                            != 0
                        {
                            LookDirection::RightLeft
                        } else {
                            LookDirection::LeftRight
                        },
                    );
                    self.base
                        .launch_timer(parameters_ai::AI_LOOK_TIME as u32, ctx.frame);
                }
            }

            Substate::SeekingJustWatchingSidewards => {
                if stimulus_type == StimulusType::EventDone {
                    // Original has explicit SOLDIER and OFFICER arms and no
                    // default arm. A knight can nevertheless inherit this
                    // substate from a legacy save, in which case EVENT_DONE
                    // deliberately leaves it unchanged.
                    match self.get_rank() {
                        ProfileRank::Soldier => {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        }
                        ProfileRank::Officer => {
                            self.officer_look_for_soldier(ReportType::Noise, ctx, tick);
                        }
                        ProfileRank::Knight | ProfileRank::None => {}
                    }
                }
            }

            Substate::SeekingBodyReactiontime => {
                if stimulus_type == StimulusType::EventTimer {
                    // Body-reactiontime expiry branches on rank:
                    //   SOLDIER → look for a near officer who's already informed
                    //             about this body (and stand by for instructions)
                    //   OFFICER → if body is far enough, delegate; else examine
                    //   KNIGHT  → examine themselves
                    let body = self.base.detected_body;
                    let mut nearby_officer: Option<NpcHandle> = None;
                    let mut look_for_soldiers = false;

                    match self.get_rank() {
                        ProfileRank::Soldier => {
                            // `near_officer_who_is_informed_about_this_body(body)`.
                            // Find the first same-camp officer who is
                            // able-to-fight, not script-locked, that
                            // we can detect 360°, and who has dropped
                            // `body` from their DETECTABLE_BODY list
                            // (i.e. has already processed it).  This
                            // uses the officer's live detectable-body
                            // snapshot in
                            // `CampSoldierInfo::detectable_bodies`.
                            nearby_officer = tick.camp_soldiers.iter().find_map(|cs| {
                                if cs.rank != ProfileRank::Officer
                                    || !cs.is_able_to_fight
                                    || cs.script_locked
                                {
                                    return None;
                                }
                                // Officer must have already processed
                                // this body (dropped it from their
                                // DETECTABLE_BODY list).
                                if cs.detectable_bodies.contains(&body) {
                                    return None;
                                }
                                // Approximate `IsDetecting360Degrees`
                                // with the standard view-radius square
                                // (same fallback used by `alert_officer`).
                                self.is_detecting_360_degrees(cs.handle, ctx)
                                    .then_some(cs.handle)
                            });
                        }
                        ProfileRank::Officer => {
                            // If body is further than
                            // OFFICER_EXAMINE_BODY_HIMSELF_DISTANCE,
                            // delegate via ShallISendOutSoldier.
                            let dist = ctx
                                .entity_view(body)
                                .map(|v| {
                                    let dx = (v.position.x - ctx.position.x).abs();
                                    let dy = (v.position.y - ctx.position.y).abs();
                                    dx.max(dy)
                                })
                                .unwrap_or(0.0);
                            if dist > combat::OFFICER_EXAMINE_BODY_HIMSELF_DISTANCE as f32 {
                                look_for_soldiers =
                                    self.answer_question(Question::ShallISendOutSoldier, ctx);
                            }
                        }
                        ProfileRank::Knight | ProfileRank::None => {}
                    }

                    if let Some(off) = nearby_officer {
                        // Face + go into DefaultLookingOfficerForAdvice.
                        self.base.face_entity(off, ctx);
                        self.set_state(AiState::Default, Substate::DefaultLookingOfficerForAdvice);
                        self.base.set_emoticon(EmoticonType::QuestionMark);
                        self.base.launch_timer(100, ctx.frame);
                    } else if look_for_soldiers {
                        // OfficerLookForSoldier(ReportType::Body).
                        self.officer_look_for_soldier(ReportType::Body, ctx, tick);
                    } else {
                        // RunToExamineBody(body).
                        self.run_to_examine_body(body, ctx, tick, grid);
                    }
                }
            }

            Substate::SeekingBody => {
                if stimulus_type == StimulusType::EventTimer {
                    // The timer only watches for a body that has recovered
                    // while we are travelling. Body examination itself is
                    // exclusively driven by EVENT_REACHPOINT in the original.
                    let body_handle = self.base.detected_body;
                    let view = ctx.entity_view(body_handle).unwrap_or_else(|| {
                        panic!(
                            "SeekingBody timer target {body_handle} has no typed live entity view"
                        )
                    });
                    if !view.is_dead && !view.is_unconscious && self.is_detecting(body_handle, ctx)
                    {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    } else {
                        self.base.launch_timer(10, ctx.frame);
                    }
                } else if stimulus_type == StimulusType::EventReachPoint {
                    self.base
                        .face_position_3d_with_ctx(self.base.seek_position, ctx);

                    // If the body is tied or unconscious, say
                    // AwakensSleeper, transition to
                    // `SeekingBodyAwakeningSleeper`, and launch
                    // `WakeUp` with the body as antagonist.  The
                    // entity-view map lets us check posture / substate
                    // without a second borrow on the entity store.
                    let body_handle = self.base.detected_body;
                    let view = ctx.entity_view(body_handle);
                    let is_tied = view
                        .map(|v| v.posture == crate::element::Posture::Tied)
                        .unwrap_or(false);
                    let is_unconscious = view
                        .map(|v| v.ai_substate == Substate::SleepingUnconscious)
                        .unwrap_or(false);

                    if body_handle != 0 && (is_tied || is_unconscious) {
                        use crate::element::Command;
                        use crate::sequence::{Sequence, SequenceElement};
                        self.base.say(Remark::AwakensSleeperr);
                        self.set_state(AiState::Seeking, Substate::SeekingBodyAwakeningSleeperr);
                        self.base.stop_all();
                        let owner = self.base.owner_entity_id;
                        let antagonist = Some(ctx.entity_id(body_handle).unwrap_or_else(|| {
                            panic!(
                                "SeekingBody wake-up target {body_handle} has no typed live entity view"
                            )
                        }));
                        let mut seq = Sequence::new();
                        seq.append_element(SequenceElement::new_interaction(
                            1,
                            Command::WakeUp,
                            owner,
                            antagonist,
                        ));
                        self.base.outbox.actor.launch_sequences.push(seq);
                        self.base.launch_timer(50, ctx.frame);
                        self.base.clear_emoticon();
                    } else {
                        // Dead-body / rider-confirms-dead branch: remember
                        // this body so a later officer "examine here" call
                        // can short-circuit (see the `CallYourTalk1` arm).
                        if body_handle != 0 && !self.already_seen_bodies.contains(&body_handle) {
                            self.already_seen_bodies.push(body_handle);
                        }
                        self.set_state(AiState::Seeking, Substate::SeekingBodyLookingDeadBody);
                        self.base.set_emoticon(EmoticonType::XMark);
                        self.base.launch_timer(
                            parameters_ai::AI_WATCH_DEADBODY_AGAIN_TIME as u32,
                            ctx.frame,
                        );
                    }
                }
            }

            Substate::SeekingBodyLookingDeadBody => {
                if stimulus_type == StimulusType::EventTimer {
                    if ctx.self_is_rider {
                        self.seek_area(
                            sim,
                            ctx.position,
                            parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                            SeekFlags::BODY_SEEK,
                            0,
                            global,
                            ctx,
                            tick,
                        );
                    } else {
                        if !self.seen_dead_body {
                            self.seen_dead_body = true;
                            self.base.say(Remark::BahIlBougePus);
                        }
                        if self.examine_other_bodies(ctx, tick) {
                            self.base
                                .my_reconnaissance_report
                                .update(ReportType::DeadBody, ctx.position);
                        } else {
                            self.dead_body_alert(
                                sim,
                                ctx.position,
                                SeekFlags::empty(),
                                global,
                                grid,
                                ctx,
                                tick,
                            );
                        }
                    }
                }
            }

            Substate::SeekingBodyAwakeningSleeperr => {
                // After waking a sleeper (or attempting to) the timer
                // fires; if any other bodies are still pending,
                // `examine_other_bodies` drives off to them;
                // otherwise the report-type governs whether we
                // escalate to a dead-body alert or simply return to
                // duty.
                if stimulus_type == StimulusType::EventTimer
                    && !self.examine_other_bodies(ctx, tick)
                {
                    if self.base.my_reconnaissance_report.report_type == ReportType::DeadBody {
                        let pos = self.base.my_reconnaissance_report.seek_position;
                        self.dead_body_alert(sim, pos, SeekFlags::empty(), global, grid, ctx, tick);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }

            // Arrow reactiontime: Say(Arrow), transition to
            // SeekingArrow, run to noise, broadcast HeyFolksLookThere,
            // launch 200-tick timer.
            Substate::SeekingArrowReactiontime => {
                if stimulus_type == StimulusType::EventTimer {
                    self.base.say(Remark::Arrow);
                    self.go_near(
                        AiState::Seeking,
                        Substate::SeekingArrow,
                        self.base.seek_position,
                        parameters_ai::AI_MIN_SEARCHNOISE_DISTANCE,
                        GotoFlags::RUN,
                        ctx,
                    );
                    let seek_pos = self.base.seek_position;
                    self.hey_folks_look_there(&seek_pos, 100, ctx);
                    self.base.launch_timer(200, ctx.frame);
                }
            }

            // At noise origin: SeekArea around current position.
            // SOLDIER also sets LOOK_FOR_HELP_AFTER_SEEKING; OFFICER
            // does not.
            Substate::SeekingArrow => {
                if stimulus_type == StimulusType::EventReachPoint
                    || stimulus_type == StimulusType::EventTimer
                {
                    let mut flags = SeekFlags::LOCATION_FIRST | SeekFlags::WALKING;
                    if self.get_rank() == ProfileRank::Soldier {
                        flags |= SeekFlags::LOOK_FOR_HELP_AFTER;
                    }
                    let here = ctx.position;
                    self.seek_area(sim, here, 0, flags, 0, global, ctx, tick);
                }
            }

            // Arrow just-watching:
            // EVENT_TIMER → Say(Arrow, MYTALK_1) (officer-only
            // soliloquy; the Say wrapper triggers MyTalk1 callback).
            // EVENT_MYTALK_1 → AlertSoldiers (officers only); if no
            // soldier reachable, ReturnToDuty.
            Substate::SeekingArrowJustWatching | Substate::SeekingArrowJustWatchingSidewards => {
                match stimulus_type {
                    StimulusType::EventTimer => {
                        self.base
                            .say_with_flags(Remark::Arrow, SpeechFlags::MYTALK_1);
                    }
                    StimulusType::EventMyTalk1 => {
                        // Asserts officer; in non-officer cases the
                        // MYTALK won't fire, so this branch is
                        // officer-only.
                        let center = self.base.seek_position;
                        let flags =
                            (SeekFlags::LOCATION_FIRST | SeekFlags::REPORT_OFFICER_AFTER).bits();
                        if !self.alert_soldiers(
                            center,
                            flags,
                            global,
                            grid,
                            ctx,
                            tick,
                            AlertSoldiersFailureContinuation::ReturnToDuty,
                        ) {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        }
                    }
                    _ => {}
                }
            }

            Substate::SeekingCombatAlertReactiontime => {
                if stimulus_type == StimulusType::EventTimer {
                    self.go_near(
                        AiState::Seeking,
                        Substate::SeekingCombatAlert,
                        self.base.seek_position,
                        parameters_ai::AI_HELP_FRIEND_IN_TROUBLE_DISTANCE,
                        GotoFlags::RUN,
                        ctx,
                    );
                    self.base.launch_timer(10, ctx.frame);
                }
            }

            Substate::SeekingCombatAlert => {
                if stimulus_type == StimulusType::EventReachPoint
                    || stimulus_type == StimulusType::EventTimer
                {
                    self.get_battle_overview(0x0001, ctx, tick); // FAST_OVERVIEW
                }
            }

            Substate::SeekingGotStopEvent => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            // ============ CIVILIAN-ALERTS-SOLDIER ================
            // A civilian has run up to this soldier with a CALL_ALERT;
            // these substates walk the soldier through the "listen
            // to civilian → act on report" flow.
            Substate::SeekingWaitForAlertingCivilian => {
                match stimulus_type {
                    StimulusType::EventTimer => {
                        // Re-check the civilian is still on the
                        // alerting path; face + re-arm timer if so,
                        // else give up.
                        let civilian_substate =
                            ctx.entity_view(self.base.antagonist).map(|v| v.ai_substate);
                        let still_alerting = matches!(
                            civilian_substate,
                            Some(
                                Substate::SeekingCivilianRunningToSoldierSeen
                                    | Substate::SeekingCivilianGiveAlertingReportToSoldierStart
                                    | Substate::SeekingCivilianGiveAlertingReportToSoldierPoint
                                    | Substate::SeekingCivilianGiveAlertingReportToSoldierEnd
                            )
                        );
                        if still_alerting {
                            self.base.face_entity(self.base.antagonist, ctx);
                            self.base.launch_timer(20, ctx.frame);
                        } else {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        }
                    }
                    StimulusType::CallReport => {
                        // `get_report_from_civilian`: pull the
                        // civilian's `ReconnaissanceReport`, merge
                        // bodies/charly/type into ours (with the
                        // standard delete/add detectable side
                        // effects via `consider_report_merged`), then
                        // either transition to the alerting state or
                        // fall through to the non-alerting "listen
                        // and return-to-duty" timer.
                        if let StimulusInfo::Hint(ref hint) = stimulus.info {
                            let my_old_report_type = self.base.my_reconnaissance_report.report_type;
                            // Read the civilian's report off their
                            // entity-view snapshot — we can't borrow the
                            // brain mid-think.
                            let civ_view = ctx.entity_view(hint.who_tells_me).unwrap_or_else(|| {
                                panic!(
                                    "soldier {} receiving CALL_REPORT requires civilian {} view",
                                    self.base.me, hint.who_tells_me
                                )
                            });
                            let civ_report = crate::ai::ReconnaissanceReport {
                                report_type: civ_view.report_type,
                                seek_position: civ_view.report_seek_position,
                                seen_bodies: civ_view.report_seen_bodies.clone(),
                                charly: civ_view.report_charly,
                                charly_seen: civ_view.report_charly != 0,
                            };
                            // Merge with all three flags.
                            self.base.consider_report_merged(
                                &civ_report,
                                1 | 2 | 4,
                                ctx.entity_views.as_ref(),
                            );

                            // Alerting transition when the civilian's
                            // report strictly out-ranks ours and is
                            // at least Body.
                            let alerting = civ_report.report_type > my_old_report_type
                                && civ_report.report_type >= ReportType::Body;
                            if alerting {
                                // Original GetReportFromCivilian performs
                                // SetState before Face/LaunchTimer. SetState
                                // cancels the previous substate's timer, so
                                // launching first would silently discard this
                                // talk deadline.
                                self.set_state(
                                    AiState::Seeking,
                                    Substate::SeekingGetAlertingReportFromCivilian,
                                );
                                self.base.antagonist = hint.who_tells_me;
                                self.base.face_entity(hint.who_tells_me, ctx);
                                self.base.seek_position = civ_report.seek_position;
                                self.base
                                    .my_reconnaissance_report
                                    .update(civ_report.report_type, civ_report.seek_position);
                            } else {
                                // Non-alerting branch — wait out the
                                // talk timer in
                                // `SeekingGetReportFromCivilian` then
                                // `ReturnToDuty`.
                                self.set_state(
                                    AiState::Seeking,
                                    Substate::SeekingGetReportFromCivilian,
                                );
                                self.base.face_entity(self.base.antagonist, ctx);
                            }
                            self.base
                                .launch_timer(combat::STANDARD_TALK_TIME as u32, ctx.frame);
                        }
                    }
                    _ => {}
                }
            }

            Substate::SeekingGetReportFromCivilian => {
                // Non-alerting civilian report — wait out the
                // talk time and return to duty.
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            Substate::SeekingGetAlertingReportFromCivilian => {
                // After the talk timer, turn toward the seek point
                // and enter the LOOK substate for a 30-frame reaction
                // window.
                if stimulus_type == StimulusType::EventTimer {
                    let seek_pos = self.base.seek_position;
                    self.base.face_position_3d_with_ctx(seek_pos, ctx);
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingGetAlertingReportFromCivilianLook,
                    );
                    self.base.launch_timer(30, ctx.frame);
                }
            }

            Substate::SeekingGetAlertingReportFromCivilianLook => {
                // Act on the civilian's report based on rank.
                if stimulus_type == StimulusType::EventTimer {
                    let seek_pos = self.base.seek_position;
                    match self.get_rank() {
                        ProfileRank::Officer => {
                            if self.answer_question(Question::ShallISeekBeforeAlertingSoldiers, ctx)
                            {
                                self.seek_area(
                                    sim,
                                    seek_pos,
                                    0,
                                    SeekFlags::LOCATION_FIRST | SeekFlags::LOOK_FOR_HELP_AFTER,
                                    0,
                                    global,
                                    ctx,
                                    tick,
                                );
                            } else if !self.alert_soldiers(
                                seek_pos,
                                0,
                                global,
                                grid,
                                ctx,
                                tick,
                                AlertSoldiersFailureContinuation::ReturnToDuty,
                            ) {
                                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                            }
                        }
                        ProfileRank::Soldier => {
                            if self.answer_question(Question::ShallISeekBeforeAlertingOfficer, ctx)
                            {
                                self.seek_area(
                                    sim,
                                    seek_pos,
                                    parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                                    SeekFlags::LOCATION_FIRST | SeekFlags::LOOK_FOR_HELP_AFTER,
                                    0,
                                    global,
                                    ctx,
                                    tick,
                                );
                            } else if !self.alert_officer(sim, seek_pos, 0, ctx, tick) {
                                self.seek_area(
                                    sim,
                                    seek_pos,
                                    parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                                    SeekFlags::LOCATION_FIRST,
                                    0,
                                    global,
                                    ctx,
                                    tick,
                                );
                            }
                        }
                        ProfileRank::Knight => {
                            self.seek_area(
                                sim,
                                seek_pos,
                                parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                                SeekFlags::LOCATION_FIRST,
                                0,
                                global,
                                ctx,
                                tick,
                            );
                        }
                        _ => {}
                    }
                }
            }

            // ============ OFFICER-SOLDIER COORDINATION ============

            // -------- Officer gives instructions to individual soldier --------
            Substate::SeekingOfficerCallSoldier => {
                // Officer turned to face soldier, now calls them
                if stimulus_type == StimulusType::EventDone {
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::RequestThinkResult {
                            target: self.base.antagonist,
                            caller: self.base.me,
                            stimulus_type: StimulusType::CallHey,
                            info: StimulusInfo::Human(self.base.me),
                            continuation: ThinkResultContinuation::OfficerCalledSoldier,
                        },
                    );
                }
            }

            Substate::SeekingOfficerWaitForSoldier => {
                // Officer waits for soldier to approach
                match stimulus_type {
                    StimulusType::EventTimer => {
                        let ant_substate = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist)
                            .map(|cs| cs.ai_substate);
                        match ant_substate {
                            Some(
                                Substate::SeekingSoldierCalledByOfficer
                                | Substate::SeekingSoldierGoToOfficer,
                            ) => {
                                self.face_npc(self.base.antagonist, ctx);
                                self.base.launch_timer(20, ctx.frame);
                            }
                            _ => {
                                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                            }
                        }
                    }
                    StimulusType::CallCoordinate => {
                        // Soldier has arrived and reported
                        self.set_state(AiState::Seeking, Substate::SeekingOfficerInstructSoldier);
                        self.base.point_to(self.base.alert_soldiers_point, ctx);
                        self.base.launch_timer(20, ctx.frame);
                    }
                    _ => {}
                }
            }

            Substate::SeekingOfficerInstructSoldier => {
                // Officer instructs soldier via dialogue
                match stimulus_type {
                    StimulusType::CallYourTalk1 => {
                        // Soldier said "What's your order, Sir?"
                        self.base
                            .say_with_flags(Remark::OfficerSendsOutSoldier, SpeechFlags::MYTALK_1);
                    }
                    StimulusType::EventMyTalk1 => {
                        // I said "Soldier! Examine this place!"
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                // Original directly calls
                                // `mpAntagonist->Think(CALL_YOURTALK_1)` and
                                // does not retry the call on the speaker.
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                                target: self.base.antagonist,
                                stimulus_type: StimulusType::CallYourTalk1,
                                info: StimulusInfo::None,
                            },
                        );
                    }
                    StimulusType::CallYourTalk2 => {
                        // Soldier said "Sir, yes, Sir!"
                        self.set_state(
                            AiState::Seeking,
                            Substate::SeekingOfficerWaitForInstructedSoldier,
                        );
                        self.missed_soldier_timer = 0;
                        self.base.launch_timer(30, ctx.frame);
                    }
                    StimulusType::EventTimer => {
                        let ant_substate = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist)
                            .map(|cs| cs.ai_substate);
                        if ant_substate == Some(Substate::SeekingSoldierGetInstructedByOfficer) {
                            self.base.launch_timer(20, ctx.frame);
                        } else {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        }
                    }
                    _ => {}
                }
            }

            Substate::SeekingOfficerWaitForInstructedSoldier => {
                // Officer waits for soldier to return from search
                match stimulus_type {
                    StimulusType::CallYourTalk1 => {
                        self.base.say(Remark::OfficerAsksWhatsup);
                    }
                    StimulusType::EventTimer => {
                        let ant = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist);
                        if let Some(ant) = ant {
                            let alive_and_conscious = ctx
                                .entity_view(ant.handle)
                                .is_some_and(|view| !view.is_dead && !view.is_unconscious);
                            let visible = alive_and_conscious
                                && self.is_detecting_180_degrees(ant.handle as HumanHandle, ctx);
                            if visible && ant.ai_state == AiState::Seeking {
                                self.missed_soldier_timer = 0;
                                self.base.launch_timer(30, ctx.frame);
                            } else if visible {
                                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                            } else {
                                self.missed_soldier_timer += 1;
                                if self.missed_soldier_timer > 100 {
                                    // Enough waiting — seek ourselves or alert soldiers
                                    if !self.alert_soldiers(
                                        ctx.position,
                                        0,
                                        global,
                                        grid,
                                        ctx,
                                        tick,
                                        AlertSoldiersFailureContinuation::SeekMissingInstructedSoldier,
                                    ) {
                                        self.seek_area(
                                            sim,
                                            ctx.position,
                                            parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                                            SeekFlags::LOCATION_FIRST | self.seek_flags,
                                            0,
                                            global,
                                            ctx,
                                            tick,
                                        );
                                    }
                                } else {
                                    self.base.launch_timer(30, ctx.frame);
                                }
                            }
                        } else {
                            self.missed_soldier_timer += 1;
                            if self.missed_soldier_timer > 100 {
                                // Enough waiting — seek ourselves or alert soldiers
                                if !self.alert_soldiers(
                                    ctx.position,
                                    0,
                                    global,
                                    grid,
                                    ctx,
                                    tick,
                                    AlertSoldiersFailureContinuation::SeekMissingInstructedSoldier,
                                ) {
                                    self.seek_area(
                                        sim,
                                        ctx.position,
                                        parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                                        SeekFlags::LOCATION_FIRST | self.seek_flags,
                                        0,
                                        global,
                                        ctx,
                                        tick,
                                    );
                                }
                            } else {
                                self.base.launch_timer(30, ctx.frame);
                            }
                        }
                    }
                    StimulusType::CallReport => {
                        let soldier = match stimulus.info {
                            StimulusInfo::Human(h) => h,
                            _ => self.base.antagonist,
                        };
                        if !self.get_report_from_soldier(soldier, false, ctx, tick) {
                            // Nothing special discovered
                            self.set_state(
                                AiState::Seeking,
                                Substate::SeekingOfficerGetReportFromSoldier,
                            );
                            self.face_npc(self.base.antagonist, ctx);
                            self.base.launch_timer(100, ctx.frame);
                        }
                    }
                    _ => {}
                }
            }

            Substate::SeekingOfficerGetReportFromSoldier => {
                // Officer received report, wrapping up
                match stimulus_type {
                    StimulusType::CallYourTalk1 => {
                        self.base
                            .say_with_flags(Remark::OfficerEndsConversation, SpeechFlags::MYTALK_1);
                    }
                    StimulusType::EventTimer | StimulusType::EventMyTalk1 => {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                    _ => {}
                }
            }

            // -------- Soldier receives instructions from officer --------
            Substate::SeekingSoldierCalledByOfficer => {
                // Soldier called by officer, approach on timer
                if stimulus_type == StimulusType::EventTimer {
                    let officer_pos = tick
                        .camp_soldiers
                        .iter()
                        .find(|cs| cs.handle == self.base.antagonist)
                        .map(|cs| cs.position)
                        .unwrap_or(self.officers_position);
                    self.go_near(
                        AiState::Seeking,
                        Substate::SeekingSoldierGoToOfficer,
                        officer_pos,
                        40,
                        // Original's CALLED_BY_OFFICER timer calls
                        // `GoNear(Position(mpAntagonist), 40)` without
                        // `GOTO_RUN`; the separate return-to-officer path is
                        // the one that explicitly runs.
                        GotoFlags::empty(),
                        ctx,
                    );
                    self.base.launch_timer(20, ctx.frame);
                }
            }

            Substate::SeekingSoldierGoToOfficer => {
                // Soldier walking to officer
                match stimulus_type {
                    StimulusType::EventTimer => {
                        let ant_substate = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist)
                            .map(|cs| cs.ai_substate);
                        if ant_substate == Some(Substate::SeekingOfficerWaitForSoldier) {
                            self.base.launch_timer(20, ctx.frame);
                        } else {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        }
                    }
                    StimulusType::EventReachPoint => {
                        let ant_substate = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist)
                            .map(|cs| cs.ai_substate);
                        if ant_substate == Some(Substate::SeekingOfficerWaitForSoldier) {
                            self.base.outbox.reentrant.cross_npc_actions.push(
                                CrossNpcAction::SendStimulus {
                                    fallback_to_sender: None,
                                    to_whole_patrol: false,
                                    target: self.base.antagonist,
                                    stimulus_type: StimulusType::CallCoordinate,
                                    info: StimulusInfo::Human(self.base.me),
                                },
                            );
                            self.set_state(
                                AiState::Seeking,
                                Substate::SeekingSoldierGetInstructedByOfficer,
                            );
                            self.base.launch_timer(20, ctx.frame);
                            self.base
                                .say_with_flags(Remark::AwaitsOrders, SpeechFlags::MYTALK_1);
                        } else {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        }
                    }
                    _ => {}
                }
            }

            Substate::SeekingSoldierGetInstructedByOfficer => {
                // Soldier receiving instructions from officer
                match stimulus_type {
                    StimulusType::EventMyTalk1 => {
                        // I said "What's your order, Sir?"
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                // Direct antagonist Think; no sender fallback
                                // in the Original conversation chain.
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                                target: self.base.antagonist,
                                stimulus_type: StimulusType::CallYourTalk1,
                                info: StimulusInfo::None,
                            },
                        );
                    }
                    StimulusType::CallYourTalk1 => {
                        // Officer said "Soldier! Examine this place!"
                        // Check if body was already examined
                        if self.base.detected_body != 0
                            && self.already_seen_bodies.contains(&self.base.detected_body)
                        {
                            // Already examined — skip search, return to officer
                            self.base.outbox.reentrant.cross_npc_actions.push(
                                CrossNpcAction::SendStimulus {
                                    // Direct antagonist Think; no sender
                                    // fallback in the Original.
                                    fallback_to_sender: None,
                                    to_whole_patrol: false,
                                    target: self.base.antagonist,
                                    stimulus_type: StimulusType::CallYourTalk2,
                                    info: StimulusInfo::None,
                                },
                            );
                            self.set_state(
                                AiState::Seeking,
                                Substate::SeekingSoldierReturnToOfficer,
                            );
                            // Re-dispatch as reachpoint.
                            self.base.launch_timer(1, ctx.frame);
                        } else {
                            self.base
                                .say_with_flags(Remark::GiveOrReceiveOrder, SpeechFlags::MYTALK_2);
                        }
                    }
                    StimulusType::EventMyTalk2 => {
                        // I said "Sir, yes, Sir!"
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                // Direct antagonist Think; no sender fallback
                                // in the Original conversation chain.
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                                target: self.base.antagonist,
                                stimulus_type: StimulusType::CallYourTalk2,
                                info: StimulusInfo::None,
                            },
                        );
                        // Add the body to our detection list so we don't
                        // re-react when detecting it later.
                        if let StimulusInfo::Human(body_handle) = stimulus.info {
                            self.base.outbox.actor.add_detectables.push((
                                ctx.entity_id(body_handle).unwrap_or_else(|| {
                                    panic!(
                                        "CallYourTalk2 body {body_handle} has no typed live entity view"
                                    )
                                }),
                                crate::element::DetectableType::Body,
                            ));
                        }
                        self.current_task_priority = task_priority::SEEKING;
                        // Read alert_soldiers_point from officer
                        if let Some(officer) = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist)
                        {
                            self.base.alert_soldiers_point = officer.alert_soldiers_point;
                        }
                        self.officers_position = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist)
                            .map(|cs| cs.position)
                            .unwrap_or(self.officers_position);
                        self.seek_area(
                            sim,
                            self.base.alert_soldiers_point,
                            0,
                            SeekFlags::LOCATION_FIRST | SeekFlags::REPORT_OFFICER_AFTER,
                            UNDEFINED_DIRECTION,
                            global,
                            ctx,
                            tick,
                        );
                    }
                    StimulusType::EventTimer => {
                        let ant_substate = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist)
                            .map(|cs| cs.ai_substate);
                        if ant_substate == Some(Substate::SeekingOfficerInstructSoldier) {
                            self.base.launch_timer(20, ctx.frame);
                        } else {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        }
                    }
                    _ => {}
                }
            }

            Substate::SeekingSoldierReturnToOfficer => {
                // Soldier returning to officer after search
                match stimulus_type {
                    StimulusType::EventTimer => {
                        let ant = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist);
                        if let Some(ant) = ant {
                            match ant.ai_substate {
                                Substate::SeekingOfficerWaitForInstructedSoldier
                                | Substate::SeekingOfficerWaitForInstructedGroup
                                | Substate::SeekingDetectedCharly => {
                                    self.base.launch_timer(20, ctx.frame);
                                }
                                _ => {
                                    // Are we near the officer's last known position?
                                    let dx = ctx.position.x - self.officers_position.x;
                                    let dy = ctx.position.y - self.officers_position.y;
                                    let sq_dist = dx * dx + dy * dy;
                                    if sq_dist < ctx.sq_standard_view_radius {
                                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                                    } else {
                                        // Not near enough to know officer left
                                        self.base.launch_timer(20, ctx.frame);
                                    }
                                }
                            }
                        } else {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        }
                    }
                    StimulusType::EventReachPoint => {
                        let ant_substate = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist)
                            .map(|cs| cs.ai_substate);
                        match ant_substate {
                            Some(
                                Substate::SeekingOfficerWaitForInstructedSoldier
                                | Substate::SeekingOfficerWaitForInstructedGroup,
                            ) => {
                                self.base.outbox.reentrant.cross_npc_actions.push(
                                    CrossNpcAction::SendStimulus {
                                        fallback_to_sender: None,
                                        to_whole_patrol: false,
                                        target: self.base.antagonist,
                                        stimulus_type: StimulusType::CallReport,
                                        info: StimulusInfo::Human(self.base.me),
                                    },
                                );
                                self.base.say_with_flags(
                                    Remark::TellsOfficerNothing,
                                    crate::ai::SpeechFlags::MYTALK_1,
                                );
                                self.set_state(
                                    AiState::Seeking,
                                    Substate::SeekingSoldierGiveReportToOfficer,
                                );
                                self.base.launch_timer(100, ctx.frame);
                            }
                            _ => {
                                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                            }
                        }
                    }
                    _ => {}
                }
            }

            Substate::SeekingSoldierGiveReportToOfficer => {
                // Soldier gives report to officer
                match stimulus_type {
                    StimulusType::EventMyTalk1 => {
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                // Original directly calls
                                // `mpAntagonist->Think(CALL_YOURTALK_1, mpMe)`;
                                // it never retries on the reporting soldier.
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                                target: self.base.antagonist,
                                stimulus_type: StimulusType::CallYourTalk1,
                                info: StimulusInfo::Human(self.base.me),
                            },
                        );
                        self.base.launch_timer(20, ctx.frame);
                    }
                    StimulusType::EventTimer => {
                        self.seek_flags = SeekFlags::empty();
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                    _ => {}
                }
            }

            // -------- Officer calls a group --------
            Substate::SeekingOfficerCallGroup => {
                if stimulus_type == StimulusType::EventTimer
                    && !self.alert_soldiers(
                        self.base.seek_position,
                        self.seek_flags.bits(),
                        global,
                        grid,
                        ctx,
                        tick,
                        AlertSoldiersFailureContinuation::ReturnToDuty,
                    )
                {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            Substate::SeekingOfficerWaitForGroup => {
                // Officer waits for group to assemble
                if matches!(
                    stimulus_type,
                    StimulusType::CallCoordinate | StimulusType::EventTimer
                ) {
                    // Check if anyone is still approaching
                    let mut wait_longer = false;
                    self.alerted_us.retain(|&handle| {
                        let substate = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == handle)
                            .map(|cs| cs.ai_substate);
                        match substate {
                            Some(
                                Substate::SeekingGroupCalledByOfficer
                                | Substate::SeekingGroupGoToOfficer,
                            ) => {
                                wait_longer = true;
                                true // keep in list
                            }
                            Some(Substate::SeekingGroupGetInstructedByOfficer) => {
                                true // arrived, keep
                            }
                            _ => false, // remove from list
                        }
                    });

                    if !wait_longer {
                        if !self.alerted_us.is_empty() {
                            self.set_state(AiState::Seeking, Substate::SeekingOfficerInstructGroup);
                            self.base.launch_timer(10, ctx.frame);
                        } else {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        }
                    }
                }
            }

            Substate::SeekingOfficerInstructGroup => {
                // Officer instructs group — point to position
                if stimulus_type == StimulusType::EventTimer {
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingOfficerInstructGroupPointing,
                    );
                    if self.base.my_reconnaissance_report.report_type == ReportType::MissedCharly {
                        self.base.say(Remark::OfficerSendsOutGroupForCharly);
                    } else {
                        self.base.say(Remark::OfficerSendsOutGroup);
                    }
                    self.base.point_to(self.base.seek_position, ctx);
                }
            }

            Substate::SeekingOfficerInstructGroupPointing => {
                // Officer done pointing, instruct each soldier
                if stimulus_type == StimulusType::EventDone {
                    let mut seek_flags = SeekFlags::REPORT_OFFICER_AFTER;
                    let seek_pos = self.base.seek_position;
                    if (seek_pos.x - ctx.position.x)
                        .abs()
                        .max((seek_pos.y - ctx.position.y).abs())
                        > 100.0
                    {
                        seek_flags |= SeekFlags::LOCATION_FIRST;
                    }
                    if self.base.my_reconnaissance_report.report_type == ReportType::MissedCharly {
                        seek_flags |= SeekFlags::CHARLY_SEEK | SeekFlags::LOCATION_FIRST;
                    }

                    // Instruct each soldier via direct CALL_INSTRUCTION and
                    // prune refusals from the live list before finalising.
                    let instructed = self.alerted_us.clone();
                    for (index, handle) in instructed.iter().copied().enumerate() {
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::RequestThinkResult {
                                target: handle,
                                caller: self.base.me,
                                stimulus_type: StimulusType::CallInstruction,
                                info: StimulusInfo::Hint(Hint {
                                    seek_point: seek_pos,
                                    seek_flags: seek_flags.bits(),
                                    who_tells_me: self.base.me,
                                }),
                                continuation:
                                    ThinkResultContinuation::OfficerInstructedGroupSoldier {
                                        last: index + 1 == instructed.len(),
                                    },
                            },
                        );
                    }
                    if instructed.is_empty() {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }

            Substate::SeekingOfficerWaitForInstructedGroup => {
                // Officer waits for group to report back.
                match stimulus_type {
                    StimulusType::CallReport => {
                        let soldier = match stimulus.info {
                            StimulusInfo::Human(h) => h,
                            _ => self.base.antagonist,
                        };
                        if !self.get_report_from_soldier(soldier, true, ctx, tick) {
                            // Nothing special detected
                            self.face_npc(soldier, ctx);
                            self.base
                                .launch_timer(combat::STANDARD_TALK_TIME as u32, ctx.frame);
                        }
                    }
                    StimulusType::EventTimer => {
                        // Check if there are still seeking soldiers
                        self.alerted_us.retain(|&handle| {
                            let substate = tick
                                .camp_soldiers
                                .iter()
                                .find(|cs| cs.handle == handle)
                                .map(|cs| cs.ai_substate);
                            matches!(
                                substate,
                                Some(
                                    Substate::SeekingSeekpoint
                                        | Substate::SeekingSeekpointWatching
                                        | Substate::SeekingSeekpointWatchingSidewards
                                        | Substate::SeekingSoldierReturnToOfficer
                                        | Substate::SeekingSoldierGiveReportToOfficer
                                        | Substate::SeekingRunningToOfficer
                                        | Substate::SeekingRunningToOfficerSeen
                                        | Substate::SeekingBodyReactiontime
                                        | Substate::SeekingBody
                                        | Substate::SeekingTakingNet
                                        | Substate::SeekingBodyLookingDeadBody
                                        | Substate::SeekingBodyAwakeningSleeperr
                                        | Substate::SeekingDetectedCharly
                                )
                            )
                        });

                        if self.alerted_us.is_empty() {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        } else {
                            self.base.launch_timer(30, ctx.frame);
                        }
                    }
                    _ => {}
                }
            }

            // -------- Officer alerted group inside house --------
            Substate::SeekingOfficerWaitInsideHouseToInstructGroup => {
                if stimulus_type == StimulusType::EventTimer {
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingOfficerLeavingHouseToInstructGroup,
                    );
                    self.base
                        .go_to(self.gather_position, GotoFlags::empty(), ctx);
                }
            }

            Substate::SeekingOfficerLeavingHouseToInstructGroup => match stimulus_type {
                StimulusType::EventReachPoint => {
                    self.base.face_direction(self.gather_direction, ctx);
                }
                StimulusType::EventDone => {
                    self.set_state(AiState::Seeking, Substate::SeekingOfficerWaitForGroup);
                    self.base.launch_timer(1, ctx.frame);
                }
                _ => {}
            },

            // -------- Group called by officer --------
            Substate::SeekingGroupCalledByOfficer => {
                if stimulus_type == StimulusType::EventTimer {
                    if self.gather_position_instructed {
                        self.go_to(
                            self.base.current_state,
                            self.base.current_substate,
                            self.gather_position,
                            GotoFlags::RUN,
                            ctx,
                        );
                    } else {
                        let officer_pos = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist)
                            .map(|cs| cs.position)
                            .unwrap_or(self.gather_position);
                        self.go_near(
                            self.base.current_state,
                            self.base.current_substate,
                            officer_pos,
                            parameters_ai::AI_TALK_DISTANCE,
                            GotoFlags::RUN,
                            ctx,
                        );
                    }
                    self.set_state(AiState::Seeking, Substate::SeekingGroupGoToOfficer);
                    self.base.launch_timer(20, ctx.frame);
                }
            }

            Substate::SeekingGroupGoToOfficer => match stimulus_type {
                StimulusType::EventTimer => {
                    let ant_substate = tick
                        .camp_soldiers
                        .iter()
                        .find(|cs| cs.handle == self.base.antagonist)
                        .map(|cs| cs.ai_substate);
                    match ant_substate {
                        Some(
                            Substate::SeekingOfficerWaitForGroup
                            | Substate::SeekingDetectedCharly
                            | Substate::SeekingOfficerWaitInsideHouseToInstructGroup
                            | Substate::SeekingOfficerLeavingHouseToInstructGroup,
                        ) => {
                            self.base.launch_timer(20, ctx.frame);
                        }
                        _ => {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        }
                    }
                }
                StimulusType::EventReachPoint => {
                    if self.gather_position_instructed {
                        self.base.face_direction(self.gather_direction, ctx);
                    } else {
                        self.face_npc(self.base.antagonist, ctx);
                    }
                }
                StimulusType::EventDone => {
                    let ant_substate = tick
                        .camp_soldiers
                        .iter()
                        .find(|cs| cs.handle == self.base.antagonist)
                        .map(|cs| cs.ai_substate);
                    match ant_substate {
                        Some(
                            Substate::SeekingOfficerWaitForGroup
                            | Substate::SeekingDetectedCharly
                            | Substate::SeekingOfficerWaitInsideHouseToInstructGroup
                            | Substate::SeekingOfficerLeavingHouseToInstructGroup,
                        ) => {
                            self.set_state(
                                AiState::Seeking,
                                Substate::SeekingGroupGetInstructedByOfficer,
                            );
                            self.base.outbox.reentrant.cross_npc_actions.push(
                                CrossNpcAction::SendStimulus {
                                    fallback_to_sender: None,
                                    to_whole_patrol: false,
                                    target: self.base.antagonist,
                                    stimulus_type: StimulusType::CallCoordinate,
                                    info: StimulusInfo::Human(self.base.me),
                                },
                            );
                        }
                        _ => {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        }
                    }
                }
                _ => {}
            },

            Substate::SeekingGroupGetInstructedByOfficer => {
                // Group member receives seek instruction
                if stimulus_type == StimulusType::CallInstruction
                    && let StimulusInfo::Hint(ref hint) = stimulus.info
                {
                    self.base.alert_soldiers_point = hint.seek_point;
                    self.officers_position = tick
                        .camp_soldiers
                        .iter()
                        .find(|cs| cs.handle == hint.who_tells_me)
                        .map(|cs| cs.position)
                        .unwrap_or(self.officers_position);
                    // Push a 30-frame THIS_GUY-scoped forbid so this
                    // NPC doesn't also auto-speak the same line when
                    // the group-instruction chain re-enters
                    // SeekingSoldierGiveReportToOfficer.  THIS_GUY
                    // scope matches on `guy_index` only, so
                    // `speech_id=0` is harmless here.
                    self.forbid_remark(
                        global,
                        Remark::TellsOfficerNothing,
                        30,
                        crate::ai::RemarkTargetFlags::THIS_GUY.bits(),
                        0,
                        ctx.original_creation_order
                            .expect("ForbidRemark requires the AI owner's Original creation order"),
                        ctx.frame,
                    );
                    self.seek_area(
                        sim,
                        hint.seek_point,
                        parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                        SeekFlags::from_bits_truncate(hint.seek_flags),
                        0,
                        global,
                        ctx,
                        tick,
                    );
                    return true;
                }
            }

            // -------- Soldier alerts officer --------
            Substate::SeekingRunningToOfficer => {
                // Soldier running to officer to alert them
                match stimulus_type {
                    StimulusType::EventTimer => {
                        // Check if officer has moved
                        if let Some(pos) = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist)
                            .map(|cs| cs.position)
                        {
                            let dx = pos.x - self.gather_position.x;
                            let dy = pos.y - self.gather_position.y;
                            let talk_sq = (parameters_ai::AI_TALK_DISTANCE as f32)
                                * (parameters_ai::AI_TALK_DISTANCE as f32);
                            if dx * dx + dy * dy > talk_sq {
                                // Officer moved — update and retry
                                self.gather_position = pos;
                                self.go_near(
                                    self.base.current_state,
                                    self.base.current_substate,
                                    pos,
                                    parameters_ai::AI_TALK_DISTANCE,
                                    GotoFlags::RUN,
                                    ctx,
                                );
                            }
                        }
                        self.base.launch_timer(50, ctx.frame);
                    }
                    StimulusType::EventReachPoint => {
                        let ant = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist);
                        let officer_ok = ant.is_some_and(|a| {
                            a.ai_state == AiState::Default
                                || a.ai_substate == Substate::SeekingOfficerWaitForInstructedSoldier
                                || a.ai_substate == Substate::SeekingDetectedCharly
                                || a.ai_substate == Substate::SeekingOfficerWaitForInstructedGroup
                        });
                        if officer_ok {
                            let officer_pos = ant.unwrap().position;
                            let dx = officer_pos.x - ctx.position.x;
                            let dy = officer_pos.y - ctx.position.y;
                            let talk_sq = (parameters_ai::AI_TALK_DISTANCE as f32)
                                * (parameters_ai::AI_TALK_DISTANCE as f32);
                            if dx * dx + dy * dy > talk_sq {
                                // Too far — retry
                                self.gather_position = officer_pos;
                                self.go_near(
                                    self.base.current_state,
                                    self.base.current_substate,
                                    officer_pos,
                                    parameters_ai::AI_TALK_DISTANCE,
                                    GotoFlags::RUN,
                                    ctx,
                                );
                            } else {
                                // Close enough — treat as seen
                                // Clear friend detection list — we've reached the officer.
                                self.base
                                    .outbox
                                    .actor
                                    .delete_detectables
                                    .push(crate::element::DetectableType::Friend);
                                self.set_state(
                                    AiState::Seeking,
                                    Substate::SeekingRunningToOfficerSeen,
                                );
                                // Re-dispatch.
                                self.base.launch_timer(1, ctx.frame);
                            }
                        } else {
                            // Officer busy — look for another
                            if !self.alert_officer(sim, self.base.seek_position, 0, ctx, tick) {
                                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                            }
                        }
                    }
                    _ => {}
                }
            }

            Substate::SeekingRunningToOfficerSeen => {
                // Soldier reached officer, starting report
                match stimulus_type {
                    StimulusType::EventMyTalk0 => {
                        // Forward talk to officer
                        let ant_substate = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist)
                            .map(|cs| cs.ai_substate);
                        if matches!(
                            ant_substate,
                            Some(
                                Substate::SeekingOfficerWaitForInstructedSoldier
                                    | Substate::SeekingOfficerWaitForAlertingSoldier
                                    | Substate::SeekingDetectedCharly
                            )
                        ) {
                            self.base.outbox.reentrant.cross_npc_actions.push(
                                CrossNpcAction::SendStimulus {
                                    fallback_to_sender: None,
                                    to_whole_patrol: false,
                                    target: self.base.antagonist,
                                    stimulus_type: StimulusType::CallYourTalk0,
                                    info: StimulusInfo::None,
                                },
                            );
                        }
                    }
                    StimulusType::EventTimer => {
                        let ant_substate = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist)
                            .map(|cs| cs.ai_substate);
                        match ant_substate {
                            Some(
                                Substate::SeekingOfficerWaitForInstructedSoldier
                                | Substate::SeekingOfficerWaitForAlertingSoldier
                                | Substate::SeekingDetectedCharly,
                            ) => {
                                self.base.launch_timer(20, ctx.frame);
                            }
                            _ => {
                                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                            }
                        }
                    }
                    StimulusType::EventReachPoint => {
                        let ant_substate = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist)
                            .map(|cs| cs.ai_substate);
                        match ant_substate {
                            Some(
                                Substate::SeekingOfficerWaitForInstructedSoldier
                                | Substate::SeekingOfficerWaitForAlertingSoldier
                                | Substate::SeekingDetectedCharly,
                            ) => {
                                self.set_state(
                                    AiState::Seeking,
                                    Substate::SeekingSoldierGiveAlertingReportToOfficerStart,
                                );
                                // Say remark based on report type
                                let speech = SpeechFlags::MYTALK_1 | SpeechFlags::EMERGENCY;
                                match self.base.my_reconnaissance_report.report_type {
                                    ReportType::Body | ReportType::DeadBody => {
                                        self.base.say_with_flags(Remark::TellsOfficerBody, speech);
                                    }
                                    ReportType::Enemy => {
                                        self.base.say_with_flags(Remark::TellsOfficerEnemy, speech);
                                    }
                                    ReportType::MissedCharly => {
                                        self.base
                                            .say_with_flags(Remark::TellsOfficerCharlyAway, speech);
                                    }
                                    _ => {
                                        self.base.say_with_flags(Remark::TellsOfficerOther, speech);
                                    }
                                }
                                self.base.launch_timer(150, ctx.frame);
                            }
                            _ => {
                                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                            }
                        }
                    }
                    _ => {}
                }
            }

            Substate::SeekingSoldierGiveAlertingReportToOfficerStart => {
                // Start of alerting report to officer.
                // Compare our report type vs officer's to decide whether
                // to point direction (new info) or just end (redundant).
                if matches!(
                    stimulus_type,
                    StimulusType::EventMyTalk1 | StimulusType::EventTimer
                ) {
                    let officer_report = tick
                        .camp_soldiers
                        .iter()
                        .find(|cs| cs.handle == self.base.antagonist)
                        .map(|cs| cs.report_type)
                        .unwrap_or(ReportType::Nothing);

                    let point_direction = match self.base.my_reconnaissance_report.report_type {
                        ReportType::Nothing => false,
                        ReportType::Noise => officer_report == ReportType::Nothing,
                        ReportType::Body | ReportType::DeadBody => {
                            officer_report <= ReportType::Noise
                        }
                        ReportType::Enemy => officer_report <= ReportType::DeadBody,
                        ReportType::MissedCharly => officer_report == ReportType::Nothing,
                    };

                    if point_direction {
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                                target: self.base.antagonist,
                                stimulus_type: StimulusType::CallReport,
                                info: StimulusInfo::Human(self.base.me),
                            },
                        );
                        // Original invokes both recipient Think calls before
                        // changing the caller's substate. The officer's
                        // CALL_YOURTALK_1 handling can synchronously call this
                        // soldier back, and that callback must still observe
                        // the report-start substate.
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::RequestThinkResult {
                                target: self.base.antagonist,
                                caller: self.base.me,
                                stimulus_type: StimulusType::CallYourTalk1,
                                info: StimulusInfo::None,
                                continuation:
                                    ThinkResultContinuation::SoldierFinishedAlertReportStart,
                            },
                        );
                    } else {
                        self.set_state(
                            AiState::Seeking,
                            Substate::SeekingSoldierGiveAlertingReportToOfficerEnd,
                        );
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                                target: self.base.antagonist,
                                stimulus_type: StimulusType::CallReport,
                                info: StimulusInfo::Human(self.base.me),
                            },
                        );
                        self.base
                            .launch_timer(combat::STANDARD_TALK_TIME as u32, ctx.frame);
                    }
                }
            }

            Substate::SeekingSoldierGiveAlertingReportToOfficerPoint => {
                // Soldier points to location
                match stimulus_type {
                    StimulusType::CallYourTalk1 | StimulusType::EventTimer => {
                        self.base.say(Remark::TellsOfficerWhere);
                        self.base.point_to(self.base.seek_position, ctx);
                    }
                    StimulusType::EventDone => {
                        self.set_state(
                            AiState::Seeking,
                            Substate::SeekingSoldierGiveAlertingReportToOfficerEnd,
                        );
                        self.face_npc(self.base.antagonist, ctx);
                        self.base
                            .launch_timer(combat::STANDARD_TALK_TIME as u32, ctx.frame);
                    }
                    _ => {}
                }
            }

            Substate::SeekingSoldierGiveAlertingReportToOfficerEnd => {
                // End of alerting report
                if stimulus_type == StimulusType::EventTimer {
                    self.seek_flags = SeekFlags::empty();
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            // -------- Officer is alerted by soldier --------
            Substate::SeekingOfficerWaitForAlertingSoldier => {
                match stimulus_type {
                    StimulusType::CallYourTalk0 => {
                        self.base.say(Remark::OfficerAsksWhatsup);
                    }
                    StimulusType::EventTimer => {
                        let ant_substate = tick
                            .camp_soldiers
                            .iter()
                            .find(|cs| cs.handle == self.base.antagonist)
                            .map(|cs| cs.ai_substate);
                        match ant_substate {
                            Some(
                                Substate::SeekingRunningToOfficerSeen
                                | Substate::SeekingSoldierGiveAlertingReportToOfficerStart
                                | Substate::SeekingSoldierGiveAlertingReportToOfficerPoint
                                | Substate::SeekingSoldierGiveAlertingReportToOfficerEnd,
                            ) => {
                                self.face_npc(self.base.antagonist, ctx);
                                self.base.launch_timer(20, ctx.frame);
                            }
                            _ => {
                                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                            }
                        }
                    }
                    StimulusType::CallReport => {
                        let soldier = match stimulus.info {
                            StimulusInfo::Human(h) => h,
                            _ => self.base.antagonist,
                        };
                        if !self.get_report_from_soldier(soldier, false, ctx, tick) {
                            // Nothing really alerting
                            self.set_state(
                                AiState::Seeking,
                                Substate::SeekingOfficerGetReportFromSoldier,
                            );
                            self.face_npc(self.base.antagonist, ctx);
                            self.base
                                .launch_timer(combat::STANDARD_TALK_TIME as u32, ctx.frame);
                        }
                    }
                    _ => {}
                }
            }

            Substate::SeekingOfficerGetAlertingReportFromSoldier => {
                // Officer processes alerting report
                match stimulus_type {
                    StimulusType::CallYourTalk1 => {
                        self.base.say_with_flags(
                            Remark::OfficerAsksWhere,
                            SpeechFlags::MYTALK_1 | SpeechFlags::EMERGENCY,
                        );
                    }
                    StimulusType::EventMyTalk1 => {
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                                target: self.base.antagonist,
                                stimulus_type: StimulusType::CallYourTalk1,
                                info: StimulusInfo::None,
                            },
                        );
                    }
                    StimulusType::EventTimer
                        if !self.alert_soldiers(
                            self.base.seek_position,
                            0,
                            global,
                            grid,
                            ctx,
                            tick,
                            AlertSoldiersFailureContinuation::ReturnToDuty,
                        ) =>
                    {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                    _ => {}
                }
            }

            // ============ ATTACKING ============
            Substate::SeekingKnightWatchingTowerGuard => {
                if stimulus_type == StimulusType::EventTimer {
                    // Knight reacts directly on alerts:
                    //   SeekArea(seek_position, AI_HINT_SEEK_RADIUS, LOCATION_FIRST);
                    self.seek_area(
                        sim,
                        self.base.seek_position,
                        parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                        SeekFlags::LOCATION_FIRST,
                        0,
                        global,
                        ctx,
                        tick,
                    );
                }
            }

            // Freeing someone from the net: wait out, or reach point
            // and take the net.
            Substate::SeekingNet => match stimulus_type {
                StimulusType::EventTimer => {
                    // If detected body is no longer stuck under net
                    // AND I'm detecting them → resurrected,
                    // ReturnToDuty; else re-arm timer.
                    let body_stuck = ctx
                        .entity_view(self.base.detected_body)
                        .map(|v| v.stuck_under_net)
                        .unwrap_or(false);
                    let detecting =
                        self.is_detecting_360_degrees(self.base.detected_body as HumanHandle, ctx);
                    if !body_stuck && detecting {
                        // Resurrected.
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    } else {
                        self.base.launch_timer(10, ctx.frame);
                    }
                }
                StimulusType::EventReachPoint => {
                    // If detected body is still under net, riders just
                    // SeekArea around themselves; foot units launch
                    // the SEARCH×4+TAKE sequence + transition to
                    // SeekingTakingNet.  Otherwise ReturnToDuty.
                    let body_stuck = ctx
                        .entity_view(self.base.detected_body)
                        .map(|v| v.stuck_under_net)
                        .unwrap_or(false);
                    if body_stuck {
                        if ctx.self_is_rider {
                            // Rider can't dismount to take the net;
                            // expand the seek radius and look.
                            let here = ctx.position;
                            self.seek_area(
                                sim,
                                here,
                                parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                                SeekFlags::BODY_SEEK,
                                0,
                                global,
                                ctx,
                                tick,
                            );
                        } else {
                            // SEARCH×4 + TAKE on interesting_object
                            // (the net).  Only fire the sequence if
                            // the object is still active.
                            let net_obj = self.base.interesting_object;
                            let active = net_obj != 0 && ctx.entity_position(net_obj).is_some();
                            if active {
                                self.set_state(AiState::Seeking, Substate::SeekingTakingNet);
                                self.base.stop_all();
                                let owner = self.base.owner_entity_id;
                                let antagonist = Some(crate::element::EntityId::Net(
                                    crate::entity_id::NetId(net_obj),
                                ));
                                let mut seq = crate::sequence::Sequence::new();
                                seq.append_element(
                                    crate::sequence::SequenceElement::new_interaction(
                                        1,
                                        crate::element::Command::SearchCmd,
                                        owner,
                                        None,
                                    ),
                                );
                                seq.append_element(
                                    crate::sequence::SequenceElement::new_interaction(
                                        2,
                                        crate::element::Command::SearchCmd,
                                        owner,
                                        None,
                                    ),
                                );
                                seq.append_element(
                                    crate::sequence::SequenceElement::new_interaction(
                                        3,
                                        crate::element::Command::SearchCmd,
                                        owner,
                                        None,
                                    ),
                                );
                                seq.append_element(
                                    crate::sequence::SequenceElement::new_interaction(
                                        4,
                                        crate::element::Command::SearchCmd,
                                        owner,
                                        None,
                                    ),
                                );
                                seq.append_element(
                                    crate::sequence::SequenceElement::new_interaction(
                                        5,
                                        crate::element::Command::Take,
                                        owner,
                                        antagonist,
                                    ),
                                );
                                self.base.outbox.actor.launch_sequences.push(seq);
                                self.base.set_emoticon(EmoticonType::None);
                            }
                        }
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
                _ => {}
            },

            // Finished removing a net: free another, examine body,
            // or return to duty.
            Substate::SeekingTakingNet => {
                if stimulus_type == StimulusType::EventDone {
                    // 3-way branch on detected body state:
                    //   stuck-under-net → RunToFreeNetVictim (still
                    //     another net on top)
                    //   dead|unconscious → RunToExamineBody
                    //   else → ReturnToDuty
                    let body = self.base.detected_body;
                    let view = ctx.entity_view(body);
                    let stuck = view.map(|v| v.stuck_under_net).unwrap_or(false);
                    let examine = view
                        .map(|v| !v.is_able_to_fight && !v.stuck_under_net)
                        .unwrap_or(false);
                    if stuck || examine {
                        // `run_to_examine_body` internally forks on
                        // `stuck_under_net` and transitions into
                        // SeekingNet for the net-takedown path, or
                        // SeekingBody for the examine path.
                        self.run_to_examine_body(body, ctx, tick, grid);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }

            // Officer scanning for free soldiers (three stages).
            Substate::SeekingOfficerLookingForSoldiers1
            | Substate::SeekingOfficerLookingForSoldiers2
            | Substate::SeekingOfficerLookingForSoldiers3 => {
                if stimulus_type == StimulusType::EventTimer {
                    let next = match self.base.current_substate {
                        Substate::SeekingOfficerLookingForSoldiers1 => {
                            Substate::SeekingOfficerLookingForSoldiers1Sidewards
                        }
                        Substate::SeekingOfficerLookingForSoldiers2 => {
                            Substate::SeekingOfficerLookingForSoldiers2Sidewards
                        }
                        _ => Substate::SeekingOfficerLookingForSoldiers3Sidewards,
                    };
                    self.set_state(AiState::Seeking, next);
                    self.base.outbox.actor.look_sidewards = Some(
                        if crate::sim_rng::u32(
                            sim,
                            crate::sim_rng::RngSite::OfficerSearchLook,
                            0..2,
                        ) != 0
                        {
                            LookDirection::RightLeft
                        } else {
                            LookDirection::LeftRight
                        },
                    );
                }
            }

            // Sidewards 1/2 advance to next look stage, face 5/16
            // rotation, delay 30.
            Substate::SeekingOfficerLookingForSoldiers1Sidewards
            | Substate::SeekingOfficerLookingForSoldiers2Sidewards => {
                if stimulus_type == StimulusType::EventDone {
                    let next = match self.base.current_substate {
                        Substate::SeekingOfficerLookingForSoldiers1Sidewards => {
                            Substate::SeekingOfficerLookingForSoldiers2
                        }
                        _ => Substate::SeekingOfficerLookingForSoldiers3,
                    };
                    self.set_state(AiState::Seeking, next);
                    // FaceTo((direction + 5) % 16)
                    let new_dir = (ctx.direction + 5) % 16;
                    self.base.face_direction(new_dir, ctx);
                    self.base.launch_timer(30, ctx.frame);
                }
            }

            // Stage-3 sidewards complete; done looking.
            Substate::SeekingOfficerLookingForSoldiers3Sidewards => {
                if stimulus_type == StimulusType::EventDone {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            // Charly search path: step through `search_charly_way`
            // on each reach point.
            Substate::SeekingCharly => {
                if stimulus_type == StimulusType::EventReachPoint {
                    if !self.search_charly_way.is_empty() {
                        self.search_charly_way.remove(0);
                    }
                    if self.search_charly_way.is_empty() {
                        // If checkpoint_charly == 0 → ReturnToDuty;
                        // else transition to CharlyWatching +
                        // LookSidewards(LeftRight).
                        if self.base.checkpoint_charly == 0 {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        } else {
                            self.set_state(AiState::Seeking, Substate::SeekingCharlyWatching);
                            self.base.outbox.actor.look_sidewards = Some(LookDirection::LeftRight);
                        }
                    } else {
                        // GoTo next waypoint with RUN (+ DONT_STOP
                        // if more than one remains).
                        let next = self.search_charly_way[0];
                        let flags = if self.search_charly_way.len() > 1 {
                            crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::DONT_STOP
                        } else {
                            crate::ai::GotoFlags::RUN
                        };
                        self.base.go_to(next, flags, ctx);
                    }
                }
            }

            // Done watching at charly checkpoint: trigger
            // missed-charly alert.
            Substate::SeekingCharlyWatching => {
                if stimulus_type == StimulusType::EventDone {
                    // Inlined `missed_charly_alert()`: a ~40-line
                    // sequence of recon-report updates, rank branch
                    // (AlertOfficer/AlertSoldiers), and a SeekArea
                    // fallback.  The "set_reported_to_officer(false)"
                    // on `checkpoint_charly` writes to another NPC's
                    // field; queued via
                    // `pending_set_reported_to_officer` and drained
                    // by the engine after the think pass.
                    self.base.say(Remark::DidntFindCharly);
                    let my_pos = ctx.position;
                    self.base.seek_position = my_pos;
                    self.base
                        .my_reconnaissance_report
                        .update(ReportType::MissedCharly, my_pos);
                    self.base.my_reconnaissance_report.charly = self.base.checkpoint_charly;
                    self.base.frame_when_enemy_detected = ctx.frame;
                    // charly.set_reported_to_officer(false).
                    // Queued via `pending_set_reported_to_officer`;
                    // the engine drains these pairs after the think
                    // pass.
                    if self.base.checkpoint_charly != 0 {
                        self.base
                            .outbox
                            .actor
                            .set_reported_to_officer
                            .push((self.base.checkpoint_charly as NpcHandle, false));
                    }
                    let alert_handled = match self.get_rank() {
                        ProfileRank::Soldier => self.alert_officer(
                            sim,
                            my_pos,
                            SeekFlags::CHARLY_SEEK.bits(),
                            ctx,
                            tick,
                        ),
                        ProfileRank::Officer => self.alert_soldiers(
                            my_pos,
                            SeekFlags::CHARLY_SEEK.bits(),
                            global,
                            grid,
                            ctx,
                            tick,
                            AlertSoldiersFailureContinuation::SeekMissedCharly { center: my_pos },
                        ),
                        ProfileRank::Knight | ProfileRank::None => false,
                    };
                    if !alert_handled {
                        // Seek yourself fallback.  Uses the FIX radius
                        // if the checkpoint charly has no patrol path,
                        // else PATROL radius.
                        let charly_has_path = ctx
                            .entity_view(self.base.checkpoint_charly)
                            .map(|v| v.has_patrol_path)
                            .unwrap_or(false);
                        let radius = if charly_has_path {
                            parameters_ai::AI_PATROL_CHARLY_SEEK_RADIUS as u16
                        } else {
                            parameters_ai::AI_FIX_CHARLY_SEEK_RADIUS as u16
                        };
                        self.seek_area(
                            sim,
                            my_pos,
                            radius,
                            SeekFlags::LOCATION_FIRST | SeekFlags::CHARLY_SEEK,
                            0,
                            global,
                            ctx,
                            tick,
                        );
                    }
                }
            }

            // Detected charly reaction; rank-dependent follow-up.
            Substate::SeekingDetectedCharly => {
                if stimulus_type == StimulusType::EventTimer {
                    // Mark charly seen, then branch on rank.
                    self.base.my_reconnaissance_report.charly_seen = true;
                    match self.get_rank() {
                        ProfileRank::Officer if !self.alerted_us.is_empty() => {
                            // Reload previous state + short timer.
                            // The reference leaves this branch as "tell
                            // all soldiers to go home" without an
                            // implementation; preserve the shipped
                            // reload-and-wait behavior.
                            let previous_state =
                                AiState::try_from(self.previous_state as u32).unwrap_or_else(|_| {
                                    panic!(
                                        "live mPreviousState contains invalid Original enum word {}",
                                        self.previous_state
                                    )
                                });
                            let previous_substate = Substate::try_from(
                                self.previous_substate as u32,
                            )
                            .unwrap_or_else(|_| {
                                panic!(
                                    "live mPreviousSubstate contains invalid Original enum word {}",
                                    self.previous_substate
                                )
                            });
                            self.set_state(previous_state, previous_substate);
                            self.base.launch_timer(10, ctx.frame);
                        }
                        _ => {
                            // Soldier, Knight, or officer with no alerted
                            // soldiers all fall through to ReturnToDuty.
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        }
                    }
                }
            }

            // Officer sends charly away toward another officer.
            Substate::SeekingSendCharlyToOfficer => match stimulus_type {
                StimulusType::EventMyTalk1 => {
                    let charly = self.base.friend_in_trouble;
                    if charly == 0 {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    } else {
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::RequestThinkResult {
                                target: charly,
                                caller: self.base.me,
                                stimulus_type: StimulusType::CallGoToOfficer,
                                info: crate::ai::StimulusInfo::Human(self.base.antagonist),
                                continuation: ThinkResultContinuation::OfficerSentCharlyToOfficer,
                            },
                        );
                    }
                }
                StimulusType::EventMyTalk2 => {
                    self.set_state(AiState::Seeking, Substate::SeekingLookingResurrectedCharly);
                    self.base.launch_timer(100, ctx.frame);
                }
                _ => {}
            },

            // Watching a charly who's been sent off; timer returns
            // to duty.
            Substate::SeekingLookingResurrectedCharly => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            // Charly was sent to officer; go near the officer.
            Substate::SeekingCharlySentToOfficer => {
                if stimulus_type == StimulusType::EventTimer {
                    self.set_state(AiState::Seeking, Substate::SeekingCharlyGoToOfficer);
                    // GoNear(antagonist.position, 40);
                    if let Some(view) = ctx.entity_view(self.base.antagonist) {
                        self.base
                            .go_near(view.position, 40, crate::ai::GotoFlags::empty(), ctx);
                    }
                    // unalert_all_near_charly_seekers(me).
                    // Drained engine-side via
                    // `pending_unalert_near_charly_seekers` — the
                    // engine walks all soldiers and dispatches
                    // CallCharlyIsBack to ones detecting me 180°.
                    self.base.outbox.actor.unalert_near_charly_seekers =
                        Some(CharlySeekerTarget::SelfNpc);
                    self.base.launch_timer(10, ctx.frame);
                }
            }

            // Charly on the way to officer; timer either transitions
            // to "seen" or retries.
            Substate::SeekingCharlyGoToOfficer => match stimulus_type {
                StimulusType::EventTimer => {
                    // Original: mpMe->IsDetecting(mpAntagonist). This is
                    // the normal live view cone, not the 360° helper.
                    if self.is_detecting(self.base.antagonist as HumanHandle, ctx) {
                        // The engine delivers this action synchronously and
                        // feeds the officer's actual Think return value into
                        // `resolve_charly_officer_report`.
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::ReportBackToOfficer {
                                officer: self.base.antagonist,
                                charly: self.base.me,
                            },
                        );
                    } else {
                        // unalert_all_near_charly_seekers(me).
                        self.base.outbox.actor.unalert_near_charly_seekers =
                            Some(CharlySeekerTarget::SelfNpc);
                        self.base.launch_timer(10, ctx.frame);
                    }
                }
                StimulusType::EventReachPoint => {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
                _ => {}
            },

            // Charly reached officer; reach→CallCoordinate; timer
            // keeps polling.
            Substate::SeekingCharlyGoToOfficerSeen => match stimulus_type {
                StimulusType::EventTimer => {
                    // Only re-arm if antagonist is in
                    // `OfficerWaitForCharly`; else ReturnToDuty.
                    let waits_for_charly = ctx
                        .entity_view(self.base.antagonist)
                        .map(|v| v.ai_substate == Substate::SeekingOfficerWaitForCharly)
                        .unwrap_or(false);
                    if waits_for_charly {
                        // unalert_all_near_charly_seekers(me).
                        self.base.outbox.actor.unalert_near_charly_seekers =
                            Some(CharlySeekerTarget::SelfNpc);
                        self.base.launch_timer(20, ctx.frame);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
                StimulusType::EventReachPoint => {
                    // antagonist.think(CallCoordinate, me)
                    if self.base.antagonist != 0 {
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                target: self.base.antagonist,
                                stimulus_type: StimulusType::CallCoordinate,
                                info: crate::ai::StimulusInfo::Human(self.base.me as HumanHandle),
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                            },
                        );
                    }
                    self.set_state(AiState::Seeking, Substate::SeekingCharlyGetLectureByOfficer);
                }
                _ => {}
            },

            // Charly receives the officer's lecture and transitions
            // to stage 2 on talk.
            Substate::SeekingCharlyGetLectureByOfficer => {
                if stimulus_type == StimulusType::CallYourTalk1 {
                    self.base.say(Remark::CharlyDefendsHimself);
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingCharlyGetLectureByOfficer2,
                    );
                }
            }

            // Charly lecture stage 2: relays talk, ends on
            // CallYourTalk2.
            Substate::SeekingCharlyGetLectureByOfficer2 => match stimulus_type {
                StimulusType::EventMyTalk1
                    // antagonist.think(CallYourTalk1)
                    if self.base.antagonist != 0 => {
                        self.base
                            .outbox.reentrant.cross_npc_actions
                            .push(CrossNpcAction::SendStimulus {
                                target: self.base.antagonist,
                                stimulus_type: StimulusType::CallYourTalk1,
                                info: crate::ai::StimulusInfo::None,
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                            });
                    }
                StimulusType::CallYourTalk2 => {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
                _ => {}
            },

            // Officer waits for
            // charly: on timer inspect antagonist substate; on coordinate
            // call, rebuke.
            Substate::SeekingOfficerWaitForCharly => match stimulus_type {
                StimulusType::EventTimer => {
                    // If antagonist is still in one of the
                    // "on the way" charly substates, face them, clear the
                    // emoticon and re-arm the timer; else ReturnToDuty.
                    let is_on_the_way = ctx
                        .entity_view(self.base.antagonist)
                        .map(|v| {
                            matches!(
                                v.ai_substate,
                                Substate::SeekingCharlySentToOfficer
                                    | Substate::SeekingCharlyGoToOfficer
                                    | Substate::SeekingCharlyGoToOfficerSeen
                            )
                        })
                        .unwrap_or(false);
                    if is_on_the_way {
                        self.base.face_entity(self.base.antagonist, ctx);
                        self.base.set_emoticon(EmoticonType::None);
                        self.base.launch_timer(20, ctx.frame);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
                StimulusType::CallCoordinate => {
                    // If antagonist == stimulus_info.human
                    let human_matches = matches!(
                        stimulus.info,
                        crate::ai::StimulusInfo::Human(h) if h as NpcHandle == self.base.antagonist,
                    );
                    if human_matches {
                        self.base.face_entity(self.base.antagonist, ctx);
                        self.base.say_with_flags(
                            Remark::OfficerRebukesCharly,
                            crate::ai::SpeechFlags::MYTALK_1,
                        );
                        self.set_state(AiState::Seeking, Substate::SeekingOfficerLectureCharly);
                    }
                }
                _ => {}
            },

            // Officer lectures charly. MyTalk1 → CallYourTalk1 to charly;
            // CallYourTalk1 → end lecture; MyTalk2 → point to best-waypoint
            // and launch pointing timer.
            Substate::SeekingOfficerLectureCharly => match stimulus_type {
                StimulusType::EventMyTalk1 if self.base.antagonist != 0 => {
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            target: self.base.antagonist,
                            stimulus_type: StimulusType::CallYourTalk1,
                            info: crate::ai::StimulusInfo::None,
                            fallback_to_sender: None,
                            to_whole_patrol: false,
                        },
                    );
                }
                StimulusType::CallYourTalk1 => {
                    self.base.say_with_flags(
                        Remark::OfficerRebukesCharlyEnd,
                        crate::ai::SpeechFlags::MYTALK_2,
                    );
                }
                StimulusType::EventMyTalk2 => {
                    // If antagonist has a path, find the nearest waypoint to
                    // the officer's position and PointTo it; else PointTo
                    // the antagonist's initial position. The patrol-path
                    // waypoint list isn't exposed on AiEntityView (only the
                    // has_patrol_path flag) — when the antagonist has a path
                    // we fall back to their current position as a reasonable
                    // "where I want you to go" stand-in.  For the no-path
                    // case we use `initial_position` which is now available
                    // on the view.
                    if let Some(view) = ctx.entity_view(self.base.antagonist) {
                        let target = if view.has_patrol_path {
                            // Best proxy without per-waypoint list.
                            view.position
                        } else {
                            view.initial_position
                        };
                        self.base.point_to(target, ctx);
                    }
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingOfficerLectureCharlyPointing,
                    );
                    self.base.say_with_flags(
                        Remark::OfficerEndsConversation,
                        crate::ai::SpeechFlags::MYTALK_3,
                    );
                    self.base.launch_timer(20, ctx.frame);
                }
                _ => {}
            },

            // Pointing done: forward CALL_YOURTALK_2 and go home.
            Substate::SeekingOfficerLectureCharlyPointing => {
                if stimulus_type == StimulusType::EventMyTalk3 {
                    if self.base.antagonist != 0 {
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                target: self.base.antagonist,
                                stimulus_type: StimulusType::CallYourTalk2,
                                info: crate::ai::StimulusInfo::None,
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                            },
                        );
                    }
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            // Civilian reached soldier to report; waits for officer's
            // wait-state.
            Substate::SeekingCivilianRunningToSoldierSeen => match stimulus_type {
                StimulusType::EventTimer => {
                    // If antagonist is still WaitForAlertingCivilian, re-arm
                    // timer; else end.
                    let officer_waiting = ctx
                        .entity_view(self.base.antagonist)
                        .map(|v| v.ai_substate == Substate::SeekingWaitForAlertingCivilian)
                        .unwrap_or(false);
                    if officer_waiting {
                        self.base.launch_timer(20, ctx.frame);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
                StimulusType::EventReachPoint => {
                    // Only transition if antagonist still in
                    // wait-for-alerting-civilian; else ReturnToDuty.
                    let officer_waiting = ctx
                        .entity_view(self.base.antagonist)
                        .map(|v| v.ai_substate == Substate::SeekingWaitForAlertingCivilian)
                        .unwrap_or(false);
                    if officer_waiting {
                        self.set_state(
                            AiState::Seeking,
                            Substate::SeekingCivilianGiveAlertingReportToSoldierStart,
                        );
                        self.base.launch_timer(10, ctx.frame);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
                _ => {}
            },

            // Civilian begins report; denunciates and points.
            Substate::SeekingCivilianGiveAlertingReportToSoldierStart => {
                if stimulus_type == StimulusType::EventTimer {
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingCivilianGiveAlertingReportToSoldierPoint,
                    );
                    if self.base.antagonist != 0 {
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                target: self.base.antagonist,
                                stimulus_type: StimulusType::CallReport,
                                info: crate::ai::StimulusInfo::Human(self.base.me as HumanHandle),
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                            },
                        );
                    }
                    self.base.say(Remark::CivDenunciates);
                    self.base.point_to(self.base.seek_position, ctx);
                }
            }

            // Done pointing: transition to end and face antagonist.
            Substate::SeekingCivilianGiveAlertingReportToSoldierPoint => {
                if stimulus_type == StimulusType::EventDone {
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingCivilianGiveAlertingReportToSoldierEnd,
                    );
                    self.base.face_entity(self.base.antagonist, ctx);
                    self.base.launch_timer(30, ctx.frame);
                }
            }

            // Civilian panics after denunciation.
            Substate::SeekingCivilianGiveAlertingReportToSoldierEnd => {
                if stimulus_type == StimulusType::EventTimer {
                    self.panic_from_position(
                        self.base.seek_position,
                        parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
                    );
                }
            }

            // Reserve overview re-evaluates via BattleDecisions.
            _ => {}
        }
        false
    }

    /// Handle the body of Original `Think(EVENT_REACHPOINT)` while seeking an
    /// authored seek point. This is also called re-entrantly by the two
    /// passed-ambush substates, matching their direct `Think` call.
    fn reached_seek_point(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        let seek_point_id = self
            .actual_seek_point
            .expect("seek-point arrival without an actual seek point");
        let directions = resolve_seek_point_id(
            seek_point_id,
            &self.personal_seek_point_1,
            &self.personal_seek_point_2,
            global,
        )
        .unwrap_or_else(|| panic!("actual seek point {seek_point_id:?} no longer resolves"))
        .directions
        .clone();

        self.seek_point_view_directions.clear();
        for direction in directions {
            // `(direction + 16 - current_direction) ^ 8` is the exact
            // precedence of the Original expression. Directions within one
            // sector of the direction the soldier arrived from are skipped.
            let relative = ((i32::from(direction) + 16 - i32::from(ctx.direction)) ^ 8) & 15;
            if matches!(relative, 15 | 0 | 1) {
                continue;
            }

            // Original increments the count before `rand() % count`, so even
            // insertion into an empty list consumes one global RNG draw.
            let insertion = crate::sim_rng::usize(
                sim,
                crate::sim_rng::RngSite::EnemySeekDirectionShuffle,
                0..=self.seek_point_view_directions.len(),
            );
            self.seek_point_view_directions.insert(insertion, direction);
        }

        if let Some(&direction) = self.seek_point_view_directions.first() {
            self.seek_point_view_directions.remove(0);
            self.set_state(AiState::Seeking, Substate::SeekingSeekpointWatching);
            self.base.face_direction(direction, ctx);
            self.base
                .launch_timer(parameters_ai::AI_SEEKPOINT_LOOK_TIME as u32, ctx.frame);
        } else {
            self.seek_next_point(sim, global, ctx, tick);
        }
    }

    fn think_expected_attacking_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;
        match self.base.current_substate {
            Substate::AttackingReactiontimeTurning => {
                if stimulus_type == StimulusType::EventDone
                    || stimulus_type == StimulusType::EventTimer
                {
                    self.set_state(AiState::Attacking, Substate::AttackingReactiontime);

                    // Timer depends on target's current animation and
                    // distance.  Note: the inner-switch reads
                    // `current_state` AFTER `set_state(Attacking,
                    // ...)` above, so the STATE_ATTACKING arm always
                    // wins and the `default: React(AI_MAX_...)` arm
                    // is effectively dead code in this dispatch.  We
                    // used to call `react(AI_MAX_ENEMY_REACTIONTIME)`
                    // unconditionally, which over-delayed engagement.
                    let target_view = ctx.entity_view(self.base.primary_target);
                    let target_anim = target_view.map(|v| v.current_animation);
                    let distance = target_view
                        .map(|v| {
                            let dx = (v.position.x - ctx.position.x).abs();
                            let dy = (v.position.y - ctx.position.y).abs();
                            dx.max(dy)
                        })
                        .unwrap_or(f32::INFINITY);

                    if target_anim == Some(crate::order::OrderType::RunningUpright) {
                        // Enemy running — react fast to intercept.
                        self.base.launch_timer(
                            parameters_ai::AI_RUNNING_ENEMY_REACTIONTIME as u32,
                            ctx.frame,
                        );
                    } else if distance < 30.0 {
                        // Aaaaagh, he is too close!
                        self.base.launch_timer(1, ctx.frame);
                    } else {
                        self.base.launch_timer(
                            parameters_ai::AI_QUICK_ENEMY_REACTIONTIME as u32,
                            ctx.frame,
                        );
                    }
                }
            }

            Substate::AttackingReactiontime => {
                tracing::trace!(
                    me = self.base.me,
                    frame = ctx.frame,
                    ?stimulus_type,
                    timer_ring = self.base.when_does_timer_ring,
                    "reactiontime arm: stimulus received"
                );
                if stimulus_type == StimulusType::EventTimer {
                    // Archer leaning out has a special branch: re-init
                    // enemy list, transition to ReactiontimeBending,
                    // queue EquipBowDown command.  Otherwise fall
                    // through to the standard `i_am_in_trouble` +
                    // `battle_decisions`.
                    if ctx.posture == crate::element::Posture::LeaningOut && self.is_archer() {
                        self.reinitialize_them_list(ctx, tick);
                        self.set_state(AiState::Attacking, Substate::AttackingReactiontimeBending);
                        self.base
                            .outbox
                            .actor
                            .launch_commands
                            .push(crate::element::Command::EquipBowDown);
                    } else {
                        self.i_am_in_trouble(self.base.primary_target);
                        self.battle_decisions(sim, global, ctx, tick, grid);
                    }
                }
            }

            Substate::AttackingReactiontimeRunning => {
                if stimulus_type == StimulusType::EventTimer
                    || stimulus_type == StimulusType::EventReachPoint
                {
                    self.base.stop_all();
                    self.i_am_in_trouble(self.base.primary_target);
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
            }

            Substate::AttackingRunningToEnemy
            | Substate::AttackingWalkingToEnemy
            | Substate::AttackingChargingEnemy => match stimulus_type {
                StimulusType::EventReachPoint | StimulusType::EventTimer => {
                    let distance = {
                        let dx = self.base.seek_position.x - ctx.position.x;
                        let dy = (self.base.seek_position.y - ctx.position.y)
                            * crate::position_interface::INVERSE_ASPECT_RATIO;
                        (dx * dx + dy * dy).sqrt()
                    };
                    self.reconsider_enemy_approach(
                        stimulus_type == StimulusType::EventReachPoint,
                        distance,
                        ctx,
                        tick,
                        grid,
                    );
                }
                _ => {}
            },

            Substate::AttackingOverviewLookLeft => {
                if stimulus_type == StimulusType::EventDone {
                    self.set_state(AiState::Attacking, Substate::AttackingOverviewLookRight);
                    // Look RIGHT.
                    self.base.outbox.actor.look_sidewards = Some(LookDirection::Right);
                }
            }

            Substate::AttackingOverviewLookRight => match stimulus_type {
                // Look-sidewards finished — short delay before deciding.
                StimulusType::EventDone => {
                    self.base.launch_timer(10, ctx.frame);
                }
                // Timer fires → BattleDecisions.
                StimulusType::EventTimer => {
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
                _ => {}
            },

            // ── Rider charging substates ──

            // Approaching: rider is running toward the enemy with RIDER_CHARGE flag.
            // On GALOPP_LOOP_END: check if we can begin the actual charge.
            // On REACHPOINT: we arrived at the approach position; overview battle.
            Substate::AttackingRiderChargingApproaching
                if stimulus_type == StimulusType::EventGaloppLoopEnd =>
            {
                // If can't charge, fall back to normal attack
                if !self.maybe_make_rider_attack(ctx, tick, grid) {
                    self.set_state(AiState::Attacking, Substate::AttackingRunningToEnemy);
                    let distance = {
                        let dx = self.base.seek_position.x - ctx.position.x;
                        let dy = (self.base.seek_position.y - ctx.position.y)
                            * crate::position_interface::INVERSE_ASPECT_RATIO;
                        (dx * dx + dy * dy).sqrt()
                    };
                    self.reconsider_enemy_approach(true, distance, ctx, tick, grid);
                }
            }
            Substate::AttackingRiderChargingApproaching
                if stimulus_type == StimulusType::EventReachPoint =>
            {
                // Arrived at approach point
                self.get_battle_overview(0, ctx, tick);
            }

            // Passing: rider is doing the actual charge pass through the enemy.
            // On REACHPOINT: charge pass is done, get distance for reattack.
            Substate::AttackingRiderChargingPassing
                if stimulus_type == StimulusType::EventReachPoint =>
            {
                // Transition to getting distance
                self.set_state(
                    AiState::Attacking,
                    Substate::AttackingRiderChargingGettingDistance,
                );
                if let Some(goal) = self.get_good_rider_reattack_goal(ctx, grid) {
                    // Ride away for reattack distance
                    self.go_to(
                        self.base.current_state,
                        self.base.current_substate,
                        goal,
                        GotoFlags::RUN,
                        ctx,
                    );
                } else {
                    // Attack from here
                    self.base.fire_self_stimulus(StimulusType::EventReachPoint);
                }
            }

            // GettingDistance: rider is riding away after the charge pass.
            // On REACHPOINT: arrived at retreat point, turn to face enemy.
            Substate::AttackingRiderChargingGettingDistance => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // Face the seek position (enemy last known pos)
                    self.base.face_position(self.base.seek_position);
                    self.set_state(
                        AiState::Attacking,
                        Substate::AttackingRiderChargingReturning,
                    );
                }
            }

            // Returning: rider has turned to face enemy, waiting for turn to complete.
            // On EVENT_DONE: turn complete, try reattacking.
            Substate::AttackingRiderChargingReturning => {
                if stimulus_type == StimulusType::EventDone {
                    self.rider_reattack(sim, global, ctx, tick, grid);
                }
            }

            // ApproachingBlindly: rider lost sight of all enemies, riding
            // to last known position.
            // On REACHPOINT: arrived, look around wondering.
            Substate::AttackingRiderChargingApproachingBlindly => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // Enter wondering state
                    self.set_state(AiState::Wondering, Substate::WonderingLooking1);
                    self.base.launch_timer(30, ctx.frame);
                }
            }

            // `AttackingSwordfight` matches `EventReachPoint` /
            // `EventDone` / `EventTimer`, clears the emoticon, calls
            // `reconsider_swordfight`, and (if still in the same
            // substate) says `CombatInsult`.
            //
            // Special-strike work occupies its own legacy substate and
            // therefore does not enter this ordinary swordfight arm. The
            // lifecycle latch remains as a cancellation guard; reconciliation
            // restores this state and relaunches the 20-frame heartbeat.
            Substate::AttackingSwordfight => {
                if matches!(
                    stimulus_type,
                    StimulusType::EventTimer
                        | StimulusType::EventDone
                        | StimulusType::EventReachPoint
                ) && !self.pending_special_strike
                {
                    // SetEmoticon(None).
                    self.base.set_emoticon(EmoticonType::None);
                    self.reconsider_swordfight(sim, false, global, ctx, tick, grid);
                    // Original proposes a strike inside ReconsiderSwordfight.
                    // A successful proposal first changes to SpecialStrike,
                    // suppressing this taunt. Our proposal needs engine state,
                    // so defer the decision across that owner-local boundary.
                    if self.base.current_substate == Substate::AttackingSwordfight {
                        if self.pending_sword_strike_consideration {
                            self.pending_combat_insult_after_strike_consideration = true;
                        } else {
                            self.base.say(Remark::CombatInsult);
                        }
                    }
                }
            }

            // A completed (or timed-out) strike returns to the ordinary
            // swordfight heartbeat. This is an observable legacy substate,
            // not merely an internal sequence latch.
            Substate::AttackingSwordfightSpecialStrike => {
                if matches!(
                    stimulus_type,
                    StimulusType::EventDone | StimulusType::EventTimer
                ) {
                    self.finish_special_strike(ctx.frame);
                }
            }

            // `AttackingSwordfightParade`: on `EventTimer`, end the
            // parry only if it is still active, transition back to
            // `AttackingSwordfight`, and re-launch the 20-tick heartbeat.
            // Original checks exactly `RHACTIONSTATE_PARRYING_SWORD` before
            // queuing StopParrySword. Queuing it after the actor has already
            // returned to WaitingSword is not a harmless no-op: terminating
            // that element emits a synchronous EventDone and spuriously
            // reconsiders the swordfight.
            Substate::AttackingSwordfightParade => {
                if stimulus_type == StimulusType::EventTimer {
                    if ctx.self_action_state == crate::element::ActionState::ParryingSword {
                        self.base
                            .outbox
                            .actor
                            .launch_commands
                            .push(crate::element::Command::StopParrySword);
                    }
                    self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
                    self.base.launch_timer(20, ctx.frame);
                }
            }

            // Reached position near new enemy.
            Substate::AttackingApproachingNewEnemy => {
                if stimulus_type == StimulusType::EventReachPoint {
                    let sword_range = self
                        .find_fighter(self.base.me, tick)
                        .map(|f| f.sword_range_default)
                        .unwrap_or(self.sword_range);
                    let close_enough = self
                        .find_fighter(self.base.primary_target, tick)
                        .map(|t| {
                            let d = pos_diff(&t.position, &ctx.position);
                            let sq = d.0 * d.0 + d.1 * d.1;
                            sq < ((sword_range as f32 + 10.0) * (sword_range as f32 + 10.0))
                        })
                        .unwrap_or(false);

                    if close_enough {
                        self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
                        self.base.launch_timer(20, ctx.frame);
                        self.base.outbox.actor.set_principal = Some(self.base.primary_target);
                    } else {
                        // Re-approach
                        let target_pos = self
                            .find_fighter(self.base.primary_target, tick)
                            .map(|f| f.position)
                            .unwrap_or(ctx.position);
                        self.base
                            .go_near(target_pos, sword_range as i32, GotoFlags::RUN, ctx);
                        if self.base.already_on_point {
                            self.base.already_on_point = false;
                            self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
                            self.base.launch_timer(20, ctx.frame);
                            self.base.outbox.actor.set_principal = Some(self.base.primary_target);
                        }
                    }
                }
            }

            // Reached position around old enemy.
            Substate::AttackingMovingAroundOldEnemy => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
                    self.base.launch_timer(20, ctx.frame);
                    self.reconsider_swordfight(sim, false, global, ctx, tick, grid);
                }
            }

            // Quitting swordfight timer.
            Substate::AttackingQuittingSwordfight => {
                if stimulus_type == StimulusType::EventTimer {
                    // Original tests `_ANY_SWORD_ACTIONSTATE_`, not whether
                    // the opponent list is still populated. EndSwordfight
                    // removes opponents before the current sword animation
                    // necessarily finishes, so the action state remains the
                    // authoritative completion gate.
                    if ctx.self_action_state.is_sword() {
                        // Still in a sword action state — retry quit and wait.
                        self.end_swordfight(ctx, tick);
                        self.base.launch_timer(3, ctx.frame);
                    } else {
                        // Left sword state — proceed to battle overview.
                        self.get_battle_overview(0x0000, ctx, tick);
                    }
                }
            }

            Substate::AttackingReserve => match stimulus_type {
                // Fall-through to CallCoordinate: walk same-camp
                // soldiers in AttackingReserve and send each a
                // CallCoordinate to make them all begin their
                // overview together.
                StimulusType::EventTimer => {
                    let me = self.base.me;
                    let friends_to_coord: Vec<NpcHandle> = tick
                        .camp_soldiers
                        .iter()
                        .filter(|cs| {
                            cs.handle != me && cs.ai_substate == Substate::AttackingReserve
                        })
                        .map(|cs| cs.handle)
                        .collect();
                    for target in friends_to_coord {
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                                target,
                                stimulus_type: StimulusType::CallCoordinate,
                                info: crate::ai::StimulusInfo::Human(me),
                            },
                        );
                    }
                    // Fall through to CallCoordinate arm.
                    self.reinitialize_them_list(ctx, tick);
                    self.base.set_emoticon(EmoticonType::None);
                    self.set_state(AiState::Attacking, Substate::AttackingReserveOverview);
                    self.base.launch_timer(20, ctx.frame);
                }
                StimulusType::CallCoordinate => {
                    self.reinitialize_them_list(ctx, tick);
                    self.base.set_emoticon(EmoticonType::None);
                    self.set_state(AiState::Attacking, Substate::AttackingReserveOverview);
                    self.base.launch_timer(20, ctx.frame);
                }
                _ => {}
            },

            // Last reserve: BattleDecisions on timer
            // (AttackingReserveOverview is below).
            Substate::AttackingLastReserve => {
                if stimulus_type == StimulusType::EventTimer {
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
            }

            // Approach-to-observe: on EventTimer, face primary
            // target, stop, queue EnterSwordfight (no opponent —
            // sword-raise only), transition to Observe, clear
            // emoticon, launch 50-tick timer.
            Substate::AttackingApproachToObserve => {
                if stimulus_type == StimulusType::EventTimer {
                    if self.base.primary_target != 0 {
                        self.base
                            .set_direction_toward_entity(self.base.primary_target, ctx);
                    }
                    self.base.stop_all();
                    // Launch EnterSwordfight with opponent=0 (just
                    // rises the sword pose).
                    self.base.outbox.actor.enter_swordfight =
                        Some(EnterSwordfightRequest::RaiseSword);
                    self.base.outbox.actor.enter_swordfight_jump_line = None;
                    self.set_state(AiState::Attacking, Substate::AttackingObserve);
                    self.base.set_emoticon(EmoticonType::None);
                    self.base.launch_timer(50, ctx.frame);
                }
            }

            Substate::AttackingObserve => {
                if stimulus_type == StimulusType::EventTimer {
                    self.reconsider_swordfight_observation(sim, global, ctx, tick, grid);
                }
            }

            // ReconsiderSwordfightObservation: reached the observe-and-move
            // destination — immediately reconsider the current swordfight.
            Substate::AttackingObserveAndMove => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.reconsider_swordfight_observation(sim, global, ctx, tick, grid);
                }
            }

            // TooProud entry: reinit list, clear emoticon, transition
            // to Overview, 1/16 chance of LookSidewards else 20-tick
            // timer.
            Substate::AttackingTooProudToAttack => {
                if stimulus_type == StimulusType::EventTimer {
                    self.reinitialize_them_list(ctx, tick);
                    self.base.set_emoticon(EmoticonType::None);
                    self.set_state(
                        AiState::Attacking,
                        Substate::AttackingTooProudToAttackOverview,
                    );
                    if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::TooProudLook, 0..16) == 0 {
                        self.base.outbox.actor.look_sidewards = Some(LookDirection::LeftRight);
                    } else {
                        self.base.launch_timer(20, ctx.frame);
                    }
                }
            }

            Substate::AttackingTowerGuardAlert => {
                if stimulus_type == StimulusType::EventDone {
                    self.tower_guard_call_alert(self.base.seek_position, ctx, tick);
                    // TowerGuardCallAlert is synchronous in the reference;
                    // once every recipient has handled the cry, the guard
                    // immediately chooses its next battle decision.  Observe
                    // is only one possible decision and must not be forced
                    // here (an archer may, for example, lower and reload its
                    // bow instead).
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
            }

            Substate::AttackingTowerGuardObserve => {
                if stimulus_type == StimulusType::EventTimer {
                    self.reinitialize_them_list(ctx, tick);
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
            }

            // Shooting state:
            // CallCoordinate → StopAll + BattleDecisions; EventDone →
            // ReinitializeThemList + BattleDecisions (NO state
            // transition; BattleDecisions decides whether to reload,
            // observe, etc.).
            Substate::AttackingBowShooting => match stimulus_type {
                StimulusType::EventDone => {
                    self.reinitialize_them_list(ctx, tick);
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
                StimulusType::CallCoordinate => {
                    self.base.stop_all();
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
                _ => {}
            },

            // Aiming: clear emoticon, gate on tower-guard /
            // archer-distance: if safe, transition to Shooting +
            // ShootArrowAt; else clear enemy_seen_below +
            // BattleDecisions.
            Substate::AttackingBowAiming => match stimulus_type {
                StimulusType::EventTimer => {
                    self.base.set_emoticon(EmoticonType::None);
                    let safe_to_shoot = self.tower_guard
                        || !self.archer_is_too_near_to_enemy(
                            &ctx.position,
                            self.base.primary_target,
                            ctx,
                            tick,
                        );
                    if safe_to_shoot {
                        self.set_state(AiState::Attacking, Substate::AttackingBowShooting);
                        self.shoot_arrow_at(self.base.primary_target, ctx, tick);
                    } else {
                        self.enemy_seen_below = false;
                        self.battle_decisions(sim, global, ctx, tick, grid);
                    }
                }
                StimulusType::CallCoordinate => {
                    self.base.stop_all();
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
                _ => {}
            },

            // Loading complete: transition to Aiming, launch
            // AIMING_TIME_FORMULA timer.
            Substate::AttackingBowLoading => {
                match stimulus_type {
                    StimulusType::EventDone => {
                        self.set_state(AiState::Attacking, Substate::AttackingBowAiming);
                        // `AIMING_TIME_FORMULA = (110 -
                        // shooting_ability) / 2`.  Same formula as
                        // the Decision::Shoot site below — uses the
                        // soldier's modified shooting ability (with
                        // alcohol penalty), not IQ.
                        let aim_time =
                            ((110u32).saturating_sub(self.get_shooting_ability(ctx) as u32)) / 2;
                        self.base.launch_timer(aim_time.max(5), ctx.frame);
                    }
                    StimulusType::CallCoordinate => {
                        // Defensive: shield bearer moved, re-evaluate.
                        self.base.stop_all();
                        self.battle_decisions(sim, global, ctx, tick, grid);
                    }
                    _ => {}
                }
            }

            // Observing-loading: on EventDone, transition to
            // BowObserving + 50-tick timer.
            Substate::AttackingBowObservingLoading => match stimulus_type {
                StimulusType::EventDone => {
                    self.set_state(AiState::Attacking, Substate::AttackingBowObserving);
                    self.base.launch_timer(50, ctx.frame);
                }
                StimulusType::CallCoordinate => {
                    self.base.stop_all();
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
                _ => {}
            },

            // Observing: on timer, posture-conditional branch
            // (LeaningOut → ReinitializeThemList; else StopAll),
            // then BattleDecisions.
            Substate::AttackingBowObserving => {
                if stimulus_type == StimulusType::EventTimer {
                    if ctx.posture == crate::element::Posture::LeaningOut {
                        self.reinitialize_them_list(ctx, tick);
                    } else {
                        self.base.stop_all();
                    }
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
            }

            // Archer running to cover position behind a shield bearer.
            Substate::AttackingBowRunningBehindShieldBearer => {
                match stimulus_type {
                    StimulusType::EventReachPoint => {
                        // Arrived behind shield bearer — turn to face target.
                        let target_pos = self
                            .find_fighter(self.base.primary_target, tick)
                            .map(|f| f.position);
                        if let Some(tp) = target_pos {
                            let dx = tp.x - ctx.position.x;
                            let dy = tp.y - ctx.position.y;
                            let dir = vec_to_sector(dx, dy);
                            self.base.face_direction(dir, ctx);
                        }
                    }
                    StimulusType::EventDone => {
                        // Facing done — short delay to recenter viewcone
                        // before re-evaluating: `launch_timer(5)` +
                        // `focus(primary_target)`.
                        self.base.launch_timer(5, ctx.frame);
                        if self.base.primary_target != 0 {
                            self.base.outbox.actor.set_focus(self.base.primary_target);
                        }
                    }
                    StimulusType::EventTimer => {
                        // Re-evaluate: normally shoot at primary target.
                        self.reinitialize_them_list(ctx, tick);
                        self.battle_decisions(sim, global, ctx, tick, grid);
                    }
                    _ => {}
                }
            }

            Substate::AttackingDoorFightDelay => {
                if stimulus_type == StimulusType::EventTimer {
                    self.set_state(AiState::Attacking, Substate::AttackingDoorFightLeaving);
                    self.base
                        .go_to(self.base.seek_position, GotoFlags::RUN, ctx);
                }
            }

            Substate::AttackingDoorFightLeaving => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.set_state(AiState::Attacking, Substate::AttackingDoorFightTurning);
                    self.base.face_direction(self.gather_direction, ctx);
                }
            }

            // Door-fight turning complete: if no target, wait 150
            // ticks; else BeginSwordfight.
            Substate::AttackingDoorFightTurning => {
                if stimulus_type == StimulusType::EventDone {
                    if self.base.primary_target == 0 {
                        self.set_state(AiState::Attacking, Substate::AttackingDoorFightWaiting);
                        self.base.launch_timer(150, ctx.frame);
                    } else {
                        self.begin_swordfight(ctx, tick);
                    }
                }
            }

            Substate::AttackingDoorFightWaiting => {
                if stimulus_type == StimulusType::EventTimer {
                    self.reinitialize_them_list(ctx, tick);
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
            }

            // ============ PHALANX / SHIELD-BEARER ============
            Substate::AttackingProtectingWithShield => {
                // Stand in place with shield, timer-driven re-evaluation
                if stimulus_type == StimulusType::EventTimer {
                    let my_action = self
                        .find_fighter(self.base.me, tick)
                        .map(|f| f.action_state)
                        .unwrap_or_default();

                    if !my_action.is_shield() {
                        // Reestablish shield state
                        let (target_pos, target_elevation) = self
                            .find_fighter(self.base.primary_target, tick)
                            .map(|f| (f.position, f.elevation as f32))
                            .unwrap_or((ctx.position, ctx.elevation));
                        self.base.raise_shield(target_pos, target_elevation);
                        self.base.launch_timer(20, ctx.frame);
                    } else if self.left_combat_neighbour != 0 || self.right_combat_neighbour != 0 {
                        // You should be doing phalanx stuff
                        self.set_state(AiState::Attacking, Substate::AttackingPhalanx);
                        let target_pos = self
                            .find_fighter(self.base.primary_target, tick)
                            .map(|f| f.position)
                            .unwrap_or(ctx.position);
                        self.base.face_position(target_pos);
                        self.base.launch_timer(5, ctx.frame);
                    } else if self.archer_behind_me != 0 {
                        // Protecting an archer — update direction and stay
                        let target_pos = self
                            .find_fighter(self.base.primary_target, tick)
                            .map(|f| f.position)
                            .unwrap_or(ctx.position);
                        let dx = target_pos.x - ctx.position.x;
                        let dy = target_pos.y - ctx.position.y;
                        let dir = vec_to_sector(dx, dy);
                        self.base.set_direction_goal(dir);
                        // Shield obstacle is recomputed each frame by
                        // EngineInner::update_shield_obstacles — no explicit call needed.
                        self.base.launch_timer(30, ctx.frame);
                    } else {
                        // Check if enemy is still dangerous
                        let target = if self.base.primary_target != 0 {
                            self.base.primary_target
                        } else {
                            self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick)
                        };
                        if target != 0 {
                            let target_is_bow = self
                                .find_fighter(target, tick)
                                .map(|f| f.action_state.is_bow())
                                .unwrap_or(false);
                            if target_is_bow {
                                // Still danger
                                if crate::sim_rng::u32(
                                    sim,
                                    crate::sim_rng::RngSite::ShieldAdvance,
                                    0..4,
                                ) == 0
                                {
                                    // Lower shield to advance
                                    self.set_state(
                                        AiState::Attacking,
                                        Substate::AttackingAdvancingWithShield,
                                    );
                                    self.base.lower_shield();
                                } else {
                                    self.base.launch_timer(10, ctx.frame);
                                }
                            } else {
                                // Danger is over
                                self.get_battle_overview(0, ctx, tick);
                            }
                        } else {
                            self.get_battle_overview(0, ctx, tick);
                        }
                    }
                }
            }

            Substate::AttackingAdvancingWithShield => {
                // Step forward shield-first
                match stimulus_type {
                    StimulusType::EventDone => {
                        // Shield lowered, run some steps forward
                        let target_pos = self
                            .find_fighter(self.base.primary_target, tick)
                            .map(|f| f.position)
                            .unwrap_or(ctx.position);
                        self.go_near(
                            self.base.current_state,
                            self.base.current_substate,
                            target_pos,
                            archer::MIN_PROTECT_ARROW_DISTANCE / 2,
                            GotoFlags::RUN,
                            ctx,
                        );
                        self.base.launch_timer(10, ctx.frame);
                    }
                    StimulusType::EventTimer
                        if !self.refresh_arrow_protection(false, ctx, tick, grid) =>
                    {
                        self.get_battle_overview(0x0001, ctx, tick);
                    }
                    _ => {}
                }
            }

            Substate::AttackingRunningToPhalanx => {
                // Run to phalanx slot, then raise shield
                match stimulus_type {
                    StimulusType::EventReachPoint => {
                        // Arrived — face the shield direction
                        self.base.face_direction(self.shield_bearer_direction, ctx);
                    }
                    StimulusType::EventDone => {
                        // Turning done — get primary target from neighbours
                        let target = if self.left_combat_neighbour != 0 {
                            self.find_fighter(self.left_combat_neighbour, tick)
                                .and_then(|f| {
                                    if f.is_soldier {
                                        let pt = f.primary_target;
                                        if pt != 0 { Some(pt) } else { None }
                                    } else {
                                        None
                                    }
                                })
                        } else if self.right_combat_neighbour != 0 {
                            self.find_fighter(self.right_combat_neighbour, tick)
                                .and_then(|f| {
                                    if f.is_soldier {
                                        let pt = f.primary_target;
                                        if pt != 0 { Some(pt) } else { None }
                                    } else {
                                        None
                                    }
                                })
                        } else {
                            None
                        };

                        let target = target.unwrap_or_else(|| {
                            self.get_new_primary_target(PrimaryTargetFlags::empty(), ctx, tick)
                        });

                        if target != 0 {
                            self.base.primary_target = target;
                            self.set_state(AiState::Attacking, Substate::AttackingPhalanx);
                            let (target_pos, target_elevation) = self
                                .find_fighter(target, tick)
                                .map(|f| (f.position, f.elevation as f32))
                                .unwrap_or((ctx.position, ctx.elevation));
                            self.base.raise_shield(target_pos, target_elevation);
                            self.base.face_position(target_pos);
                            self.base.launch_timer(20, ctx.frame);
                        } else {
                            self.battle_decisions(sim, global, ctx, tick, grid);
                        }
                    }
                    _ => {}
                }
            }

            Substate::AttackingPhalanx => {
                // Stand in formation, reconsider periodically
                match stimulus_type {
                    StimulusType::EventTimer => {
                        let my_action = self
                            .find_fighter(self.base.me, tick)
                            .map(|f| f.action_state)
                            .unwrap_or_default();

                        if !my_action.is_shield() && self.base.primary_target != 0 {
                            // Reestablish shield state
                            let (target_pos, target_elevation) = self
                                .find_fighter(self.base.primary_target, tick)
                                .map(|f| (f.position, f.elevation as f32))
                                .unwrap_or((ctx.position, ctx.elevation));
                            self.base.raise_shield(target_pos, target_elevation);
                            self.base.launch_timer(20, ctx.frame);
                        } else if !self.reconsider_phalanx(sim, global, ctx, tick, grid) {
                            if self.base.primary_target != 0 {
                                // No phalanx correction — maybe correct direction
                                let target_pos = self
                                    .find_fighter(self.base.primary_target, tick)
                                    .map(|f| f.position)
                                    .unwrap_or(ctx.position);
                                let dx = target_pos.x - ctx.position.x;
                                let dy = target_pos.y - ctx.position.y;
                                let dir = vec_to_sector(dx, dy);
                                self.base.set_direction_goal(dir);
                                // Shield obstacle is recomputed each frame by
                                // EngineInner::update_shield_obstacles — no explicit call needed.
                                self.base.launch_timer(20, ctx.frame);
                            } else {
                                self.get_battle_overview(0x0001, ctx, tick);
                            }
                        }
                        // else: reconsider_phalanx changed substate
                    }
                    StimulusType::CallInstruction => {
                        // Received new position instruction from phalanx leader
                        self.shield_bearer_direction = self.gather_direction;
                        self.base.seek_position = self.gather_position;
                        self.set_state(AiState::Attacking, Substate::AttackingRunningToPhalanx);
                        self.base
                            .go_to(self.base.seek_position, GotoFlags::RUN, ctx);

                        // Notify archer behind us to re-evaluate, but
                        // only if they're actively shooting/loading/aiming.
                        if self.archer_behind_me != 0 {
                            let archer_in_bow = self
                                .find_fighter(self.archer_behind_me, tick)
                                .map(|f| {
                                    let s = f.current_substate;
                                    s == Substate::AttackingBowShooting as u32
                                        || s == Substate::AttackingBowLoading as u32
                                        || s == Substate::AttackingBowAiming as u32
                                })
                                .unwrap_or(false);
                            if archer_in_bow {
                                self.base.outbox.reentrant.cross_npc_actions.push(
                                    CrossNpcAction::SendStimulus {
                                        fallback_to_sender: None,
                                        to_whole_patrol: false,
                                        target: self.archer_behind_me,
                                        stimulus_type: StimulusType::CallCoordinate,
                                        info: StimulusInfo::None,
                                    },
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }

            // ============ FLEEING ============
            // The malignity arm adds a single tweak (reset of
            // `fleeing_seen_enemy_counter` on the PANIC arm when the
            // panic is over) and then falls through into
            // `think_expected_event_common_stuff`, which owns the
            // actual panic/hide/door/hiding state machine.
            Substate::AttackingReserveOverview => {
                if stimulus_type == StimulusType::EventTimer {
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
            }

            // Approaching a sleeping PC/NPC: reach → face target; done →
            // kill or menace depending on coma/distance.
            Substate::AttackingApproachingSleepingEnemy => match stimulus_type {
                StimulusType::EventReachPoint => {
                    self.base.face_entity(self.base.primary_target, ctx);
                }
                StimulusType::EventDone => {
                    // Decision tree:
                    //   if primary_target unconscious {
                    //     if is_pc && in_coma {
                    //       if pc.guard == None { menace }
                    //       else { ReturnToDuty }
                    //     } else if distance > 40 { GoNear 20 RUN }
                    //     else { SUBSTATE_KILLING_SLEEPING_ENEMY +
                    //            SWORDSTRIKE_DOWN on primary_target }
                    //   } else { GetBattleOverview }
                    let view = ctx.entity_view(self.base.primary_target);
                    let target_pos = view.map(|v| v.position);
                    let target_unconscious = view.map(|v| v.is_unconscious).unwrap_or(false);
                    let target_is_pc = view.map(|v| v.is_pc).unwrap_or(false);
                    let target_in_coma = view.map(|v| v.in_coma).unwrap_or(false);
                    let target_guard = view.and_then(|v| v.guard);

                    if !target_unconscious {
                        self.get_battle_overview(0, ctx, tick);
                    } else if target_is_pc && target_in_coma && target_guard.is_none() {
                        // Coma/menace branch — PC is in coma and not yet
                        // guarded.
                        self.base.stop_all();
                        self.set_state(AiState::Menacing, Substate::MenacingPcInComa);
                        if self.is_vip {
                            // VIP variant — launch an EnterSwordfight against
                            // the guarded PC to trigger the menace-variant
                            // sword draw.
                            self.base.outbox.actor.enter_swordfight =
                                Some(EnterSwordfightRequest::Engage(self.base.primary_target));
                            self.base.outbox.actor.enter_swordfight_jump_line = None;
                        } else {
                            // Normal variant — say, launch StartMenace
                            // command, set guard.
                            self.base.say(Remark::MenacesPcInComa);
                            self.base
                                .outbox
                                .actor
                                .launch_commands
                                .push(crate::element::Command::StartMenace);
                            // SetGuardedPC( pPC ) — assigns both the
                            // soldier's guarded_pc and the PC's reciprocal
                            // guard.
                            self.set_guarded_pc(Some(crate::entity_id::PcId(
                                self.base.primary_target,
                            )));
                        }
                        self.base.launch_timer(20, ctx.frame);
                    } else if target_is_pc && target_in_coma && target_guard.is_some() {
                        // PC already menaced by another guard — go home.
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    } else if let Some(p) = target_pos {
                        let dx = (p.x - ctx.position.x).abs();
                        let dy = (p.y - ctx.position.y).abs();
                        if dx.max(dy) > 40.0 {
                            // GoNear(target, 20, GOTO_RUN).
                            // The kill-sleeping substate itself re-fires
                            // on REACHPOINT/DONE, so route through
                            // Approaching (same substate we're in) by
                            // issuing the move via go_near-on-self.
                            self.go_near(
                                AiState::Attacking,
                                Substate::AttackingApproachingSleepingEnemy,
                                p,
                                20,
                                crate::ai::GotoFlags::RUN,
                                ctx,
                            );
                        } else {
                            // Close enough — switch to KillingSleepingEnemy
                            // and launch SwordstrikeDown on the target.
                            self.set_state(
                                AiState::Attacking,
                                Substate::AttackingKillingSleepingEnemy,
                            );
                            self.base.stop_all();
                            use crate::element::Command;
                            use crate::sequence::{Sequence, SequenceElement};
                            let owner = self.base.owner_entity_id;
                            let antagonist = Some(
                                ctx.entity_id(self.base.primary_target).unwrap_or_else(|| {
                                    panic!(
                                        "sleeping-enemy target {} has no typed live entity view",
                                        self.base.primary_target
                                    )
                                }),
                            );
                            let mut seq = Sequence::new();
                            seq.append_element(SequenceElement::new_interaction(
                                1,
                                Command::SwordstrikeDown,
                                owner,
                                antagonist,
                            ));
                            self.base.outbox.actor.launch_sequences.push(seq);
                        }
                    } else {
                        self.get_battle_overview(0, ctx, tick);
                    }
                }
                _ => {}
            },

            // Killing sleeping enemy done: say REMARK_KILLED_ADVERSARY and
            // overview.
            Substate::AttackingKillingSleepingEnemy => {
                if stimulus_type == StimulusType::EventDone {
                    self.base.say(Remark::KilledAdversary);
                    self.get_battle_overview(0, ctx, tick);
                }
            }

            // Archer retires from combat, then turns (fast) to primary
            // target or seek.
            Substate::AttackingArcherRetireFromCombat => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.set_state(
                        AiState::Attacking,
                        Substate::AttackingArcherRetireFromCombatTurn,
                    );
                    // Cheat-face primary target if still known, else the
                    // stored seek position. Rust doesn't yet distinguish the
                    // "fast" turn flag — the face command is consumed by
                    // the order system either way.
                    if self.base.primary_target != 0 {
                        self.base.face_entity(self.base.primary_target, ctx);
                    } else {
                        self.base.face_position(self.base.seek_position);
                    }
                }
            }

            // Done turning: re-engage via BattleDecisions or overview.
            Substate::AttackingArcherRetireFromCombatTurn => {
                if stimulus_type == StimulusType::EventDone {
                    // IsDetecting180Degrees(primary_target)?
                    //   BattleDecisions : GetBattleOverview.
                    if self.base.primary_target != 0
                        && self
                            .is_detecting_180_degrees(self.base.primary_target as HumanHandle, ctx)
                    {
                        self.battle_decisions(sim, global, ctx, tick, grid);
                    } else {
                        self.get_battle_overview(0, ctx, tick);
                    }
                }
            }

            // Officer giving orders done: transition to waiting, mark
            // friends alerted.
            Substate::AttackingOfficerGivingOrders => {
                if stimulus_type == StimulusType::EventDone {
                    self.set_state(
                        AiState::Attacking,
                        Substate::AttackingOfficerGivingOrdersWaiting,
                    );
                    self.base.friends_are_alerted = true;
                    self.base.launch_timer(20, ctx.frame);
                }
            }

            // Waiting after giving orders: recompute `them` list, either
            // battle or widen seek radius.
            Substate::AttackingOfficerGivingOrdersWaiting => {
                if stimulus_type == StimulusType::EventTimer {
                    self.reinitialize_them_list(ctx, tick);
                    if !self.list_them.is_empty() {
                        self.battle_decisions(sim, global, ctx, tick, grid);
                    } else {
                        self.seek_area(
                            sim,
                            self.base.seek_position,
                            parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                            SeekFlags::LOCATION_FIRST,
                            0,
                            global,
                            ctx,
                            tick,
                        );
                    }
                }
            }

            // Too-proud overview: on done, focus target and short timer; on
            // timer, BattleDecisions and maybe say
            // REMARK_PROUD_FINALLY_FIGHT.
            Substate::AttackingTooProudToAttackOverview => match stimulus_type {
                StimulusType::EventDone => {
                    if self.base.primary_target != 0 {
                        self.base.outbox.actor.set_focus(self.base.primary_target);
                    }
                    self.base.launch_timer(5, ctx.frame);
                }
                StimulusType::EventTimer => {
                    // BattleDecisions, then if the resulting substate is a
                    // swordfight (VIP variant says VIP_REMARK, otherwise
                    // REMARK_PROUD_FINALLY_FIGHT).
                    self.battle_decisions(sim, global, ctx, tick, grid);
                    if self.base.current_substate.is_any_swordfight() {
                        let remark = if self.is_vip {
                            Remark::VipProudFinallyFight
                        } else {
                            Remark::ProudFinallyFight
                        };
                        self.base.say(remark);
                    }
                }
                _ => {}
            },

            // Retiring: reach point triggers fast face-turn to seek
            // position.
            Substate::AttackingTooProudToAttackRetire => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.set_state(
                        AiState::Attacking,
                        Substate::AttackingTooProudToAttackRetireTurn,
                    );
                    self.base.face_position(self.base.seek_position);
                }
            }

            // Finished turning: re-engage with BattleDecisions or
            // GetBattleOverview.
            Substate::AttackingTooProudToAttackRetireTurn => {
                if stimulus_type == StimulusType::EventDone {
                    if self.base.primary_target != 0
                        && self
                            .is_detecting_180_degrees(self.base.primary_target as HumanHandle, ctx)
                    {
                        self.battle_decisions(sim, global, ctx, tick, grid);
                    } else {
                        self.get_battle_overview(0, ctx, tick);
                    }
                }
            }

            // Approach finished: BattleDecisions or overview based on
            // 180-degree detection.
            Substate::AttackingTooProudToAttackApproach => {
                if stimulus_type == StimulusType::EventReachPoint {
                    if self.base.primary_target != 0
                        && self
                            .is_detecting_180_degrees(self.base.primary_target as HumanHandle, ctx)
                    {
                        self.battle_decisions(sim, global, ctx, tick, grid);
                    } else {
                        self.get_battle_overview(0, ctx, tick);
                    }
                }
            }

            // Note: `AttackingRiderChargingApproaching` /
            // `AttackingRiderChargingPassing` were previously duplicated
            // here with a "rider infra not ported" stub.  The real port
            // lives further up in this same match (see
            // `Substate::AttackingRiderChargingApproaching` around the
            // rider-charging substates block), which takes precedence;
            // the stubs were dead arms.  Removed.

            // Archer running on archery path: iterate waypoints, skip
            // occupied, occupy the first free shooting point.
            Substate::AttackingArcherRunOnShootingPath => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // do/while loop across archery waypoints.  Bound it
                    // explicitly to the current sector's waypoint count so
                    // a pathological owner-map can't spin forever.
                    let sector_idx = self.my_archery_sector.unwrap_or_else(|| {
                        panic!(
                            "archer {} running on shooting path has no archery sector",
                            self.base.me
                        )
                    });
                    let max_iters = global
                        .archery_sectors
                        .get(sector_idx as usize)
                        .unwrap_or_else(|| {
                            panic!(
                                "archer {} references missing archery sector {}",
                                self.base.me, sector_idx
                            )
                        })
                        .points
                        .len()
                        + 1;
                    let mut resolved = false;
                    for _ in 0..max_iters {
                        self.archery_path_increment_waypoint();
                        let point = self.archery_path_get_waypoint(global);
                        let Some(point) = point else {
                            panic!(
                                "archer {} reached the unsupported end of archery path in sector {}",
                                self.base.me, sector_idx
                            );
                        };
                        if !point.is_shooting_point {
                            // GoTo next entry waypoint, RUN | DONTSTOP.
                            self.base.go_to(
                                point.position,
                                crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::DONT_STOP,
                                ctx,
                            );
                            resolved = true;
                            break;
                        }
                        // Is it free (or already owned by me)?
                        let me = crate::entity_id::EntityId::Soldier(crate::entity_id::SoldierId(
                            self.base.me,
                        ));
                        let owner = point.owner;
                        if owner.is_none() || owner == Some(me) {
                            // Occupy + transition + GoTo.
                            self.set_state(
                                AiState::Attacking,
                                Substate::AttackingArcherRunOnShootingPathFinalSprint,
                            );
                            // SetMyShootingPoint clears the prior point's
                            // owner and writes ours on the new one.
                            // `archery_path_get_waypoint` is a pure read,
                            // so `my_archery_point_index` identifies this
                            // point directly.
                            let pt_idx = u16::from(self.my_archery_point_index);
                            self.set_my_shooting_point(global, Some((sector_idx, pt_idx)));
                            self.base
                                .go_to(point.position, crate::ai::GotoFlags::RUN, ctx);
                            resolved = true;
                            break;
                        }
                        // Otherwise: occupied shooting point — skip
                        // (loop iterates through archery_path_get_waypoint
                        // which already advances the cursor).
                    }
                    assert!(
                        resolved,
                        "archer {} shooting-path scan exceeded sector {} point count",
                        self.base.me, sector_idx
                    );
                }
            }

            // Final sprint: face shooting-point direction.
            Substate::AttackingArcherRunOnShootingPathFinalSprint => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.set_state(
                        AiState::Attacking,
                        Substate::AttackingArcherRunOnShootingPathTurn,
                    );
                    // FaceTo the shooting point's direction.
                    // `my_shooting_point` carries the (sector, point) pair
                    // set by `set_my_shooting_point`, so we can look up the
                    // reserved point's direction directly.
                    let (sec_idx, pt_idx) = self.my_shooting_point.unwrap_or_else(|| {
                        panic!(
                            "archer {} final sprint has no reserved shooting point",
                            self.base.me
                        )
                    });
                    let sector =
                        global
                            .archery_sectors
                            .get(sec_idx as usize)
                            .unwrap_or_else(|| {
                                panic!(
                                    "archer {} final sprint references missing archery sector {}",
                                    self.base.me, sec_idx
                                )
                            });
                    let point = sector.points.get(pt_idx as usize).unwrap_or_else(|| {
                        panic!(
                            "archer {} final sprint references missing point {} in sector {}",
                            self.base.me, pt_idx, sec_idx
                        )
                    });
                    self.base.face_direction(point.direction, ctx);
                }
            }

            // Done turning on shooting point: BattleDecisions if high, else
            // overview.
            Substate::AttackingArcherRunOnShootingPathTurn => {
                if stimulus_type == StimulusType::EventDone {
                    // If my elevation >= enemy's + 50 then set
                    // enemy_seen_below and BattleDecisions; else
                    // GetBattleOverview.
                    let my_elevation = ctx.elevation as u16;
                    if my_elevation >= self.enemy_had_this_elevation + 50 {
                        self.enemy_seen_below = true;
                        self.battle_decisions(sim, global, ctx, tick, grid);
                    } else {
                        self.get_battle_overview(0, ctx, tick);
                    }
                }
            }

            // Archer finished bending (reactiontime bend): in-trouble +
            // BattleDecisions.
            Substate::AttackingReactiontimeBending => {
                if stimulus_type == StimulusType::EventDone {
                    self.i_am_in_trouble(self.base.primary_target);
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
            }

            // Shared "timer → ReturnToDuty" for bow archers waiting on
            // archery/bend points.
            Substate::AttackingArcherWaitOnBendPoint => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            // Dummy training behavior: rotate direction every EVENT_DONE.
            Substate::AttackingDummyBehaviour => {
                if stimulus_type == StimulusType::EventDone {
                    let new_dir = (ctx.direction + 3) & 15;
                    self.base.face_direction(new_dir, ctx);
                }
            }

            // Guarding a PC in coma: if still close & in coma keep
            // watching; else give up.
            Substate::AttackingSwordfightStepBack => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
                    self.base.launch_timer(20, ctx.frame);
                }
            }

            // Officer approached brawl victim: kick off wake-up sequence.
            Substate::AttackingReturnToOtherPcAfterMenacing => {
                if stimulus_type == StimulusType::EventDone {
                    self.begin_swordfight(ctx, tick);
                }
            }

            // Running to enemy on a ladder: reach → face + focus + wait;
            // timer → reconsider.
            Substate::AttackingRunningToLadder => match stimulus_type {
                StimulusType::EventReachPoint => {
                    if self.base.primary_target != 0 {
                        self.base.face_entity(self.base.primary_target, ctx);
                        self.base.outbox.actor.set_focus(self.base.primary_target);
                    }
                    self.set_state(AiState::Attacking, Substate::AttackingWaitingAtLadder);
                    self.base.launch_timer(1, ctx.frame);
                }
                StimulusType::EventTimer => {
                    self.reconsider_enemy_approach(false, 0.0, ctx, tick, grid);
                }
                _ => {}
            },

            // Waiting at ladder: if enemy still on lift, reface & rearm;
            // else reconsider.
            Substate::AttackingWaitingAtLadder => {
                if stimulus_type == StimulusType::EventTimer {
                    // If primary target is on a lift sector, face+focus+wait;
                    // else reconsider enemy approach.
                    // This source arm calls
                    // `mpPrimaryTarget->GetSector()->IsLift()` directly. It
                    // must inspect the target element's physical sector, not
                    // `RHArtificialIntelligence::Position(actor)`'s committed
                    // destination-side sector during a door pass.
                    let target_on_lift = grid
                        .and_then(|g| {
                            tick.primary_target_live_position
                                .and_then(|position| position.sector)
                                .map(|s| g.sector_type(u32::from(s)).is_lift())
                        })
                        .unwrap_or(false);
                    if target_on_lift {
                        self.base.face_entity(self.base.primary_target, ctx);
                        self.base.outbox.actor.set_focus(self.base.primary_target);
                        self.base.launch_timer(20, ctx.frame);
                    } else {
                        self.reconsider_enemy_approach(false, 0.0, ctx, tick, grid);
                    }
                }
            }

            // Avenger on roof: reached pos, face seek & wait.
            Substate::AttackingRunToAvengerOnRoof => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.base.face_position(self.base.seek_position);
                    self.set_state(AiState::Attacking, Substate::AttackingWaitForAvengerOnRoof);
                    self.base.launch_timer(100, ctx.frame);
                }
            }

            // Wait for avenger: either re-face if detected, or SeekArea on
            // lost sight.
            Substate::AttackingWaitForAvengerOnRoof => {
                if stimulus_type == StimulusType::EventTimer {
                    // IsDetecting180Degrees(primary_target)? re-face +
                    // 30-tick timer; else SeekArea(pos,
                    // AI_LOST_ENEMY_SEEK_RADIUS, 0).
                    if self.base.primary_target != 0
                        && self
                            .is_detecting_180_degrees(self.base.primary_target as HumanHandle, ctx)
                    {
                        self.base.face_entity(self.base.primary_target, ctx);
                        self.base.launch_timer(30, ctx.frame);
                    } else {
                        self.seek_area(
                            sim,
                            self.base.seek_position,
                            parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                            SeekFlags::empty(),
                            0,
                            global,
                            ctx,
                            tick,
                        );
                    }
                }
            }

            // No-op group — only substates that still genuinely have no
            // handler remain here.  Explicit enumeration (no `_ =>`
            // catch-all) so adding a new `Substate` variant is a compile
            // error forcing the author to decide where it belongs.  The
            // 83 variants ported above were previously swept into this
            // group by the exhaustive-match refactor; see commit fbda9e7a
            // (AttackingSwordfightParade) for the original motivating fix.
            Substate::AttackingRiderChargingApproaching
            | Substate::AttackingRiderChargingPassing => {}
            _ => {}
        }
        false
    }

    fn think_expected_menacing_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;
        match self.base.current_substate {
            Substate::MenacingPcInComa => {
                if stimulus_type == StimulusType::EventTimer {
                    // Assert IsPC + check MaxNormDistance < 100 &&
                    // IsUnconscious() && IsInComa(). `is_pc` and `in_coma`
                    // both live on `AiEntityView`, so the full triplet is
                    // checkable here without approximation.
                    let keep_watching = ctx
                        .entity_view(self.base.primary_target)
                        .map(|v| {
                            let dx = (v.position.x - ctx.position.x).abs();
                            let dy = (v.position.y - ctx.position.y).abs();
                            v.is_pc && v.is_unconscious && v.in_coma && dx.max(dy) < 100.0
                        })
                        .unwrap_or(false);
                    if keep_watching {
                        self.base.launch_timer(20, ctx.frame);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }

            // Reached arrow reserves: refill ammo and SeekArea around.
            _ => {}
        }
        false
    }

    fn think_expected_fleeing_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;
        match self.base.current_substate {
            Substate::FleeingPanic
            | Substate::FleeingRunToHide
            | Substate::FleeingRunToDoor
            | Substate::FleeingHiding => {
                if self.base.current_substate == Substate::FleeingPanic
                    && matches!(
                        stimulus_type,
                        StimulusType::EventReachPoint | StimulusType::EventCouldntReachPoint
                    )
                    && self.base.lasting_panic_runs == 0
                {
                    // Malignity-specific: when the panic is spent,
                    // clear the seen-enemy counter so the soldier
                    // can re-spook on the next sighting.  Friendly
                    // AI doesn't track this counter the same way
                    // (its own reset lives elsewhere).
                    self.fleeing_seen_enemy_counter = 0;
                }
                return self
                    .base
                    .think_expected_event_common_stuff(sim, stimulus, ctx);
            }

            // Merry man fleeing to map exit.
            Substate::FleeingMerryManRunToLeaveMap => {
                match stimulus_type {
                    StimulusType::EventTimer => {
                        // Stuck recovery: if we're not already sprinting
                        // toward the door, re-issue the GoTo.  Gated
                        // on `action_state != MovingFast` &&
                        // `last_goto_destination` set, so an actor
                        // mid-run doesn't get its sequence torn down
                        // every 30 frames.
                        let dest = self.base.last_goto_destination;
                        if ctx.self_action_state != crate::element::ActionState::MovingFast
                            && (dest.x != 0.0 || dest.y != 0.0)
                        {
                            self.base.stop_all();
                            self.go_to(
                                self.base.current_state,
                                self.base.current_substate,
                                dest,
                                crate::ai::GotoFlags::RUN,
                                ctx,
                            );
                        }
                        self.base.launch_timer(30, ctx.frame);
                    }
                    StimulusType::EventReachPoint => {
                        // Check if we're near the door.
                        let dest = self.base.last_goto_destination;
                        let dx = ctx.position.x - dest.x;
                        let dy = ctx.position.y - dest.y;
                        let dist = dx.abs().max(dy.abs());
                        if dist < 10.0 {
                            // Arrived at door PositionIn — now run to PointOut
                            // to exit the map (launches a sequence
                            // element targeting the door's PointOut).
                            self.set_state(AiState::Fleeing, Substate::FleeingMerryManLeaveMap);
                            if let Some(door_idx) = self.my_door_index {
                                // `my_door_index` is a global door-table
                                // index.  Find the matching reinforcement
                                // door entry (linear scan; small list)
                                // for the cached point_out geometry.
                                if let Some(door) = global
                                    .reinforcement_doors
                                    .iter()
                                    .find(|d| d.door_index.0 == door_idx)
                                {
                                    let point_out_pos = Position {
                                        x: door.point_out.x,
                                        y: door.point_out.y,
                                        ..dest
                                    };
                                    self.base.run_to_map_exit(point_out_pos);
                                } else {
                                    // Door gone — just lock and deactivate.
                                    self.base.non_script_lock(crate::ai::AiLockFlags::FREEZE);
                                    self.base.outbox.actor.deactivate = true;
                                }
                            } else {
                                // No door stored — just lock and deactivate.
                                self.base.non_script_lock(crate::ai::AiLockFlags::FREEZE);
                                self.base.outbox.actor.deactivate = true;
                            }
                        } else {
                            // Not there yet — retry.
                            self.go_to(
                                self.base.current_state,
                                self.base.current_substate,
                                dest,
                                crate::ai::GotoFlags::RUN,
                                ctx,
                            );
                            self.base.launch_timer(30, ctx.frame);
                        }
                    }
                    _ => {}
                }
            }

            // Merry man has reached PointOut — deactivate.
            Substate::FleeingMerryManLeaveMap => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // `non_script_lock(Freeze); set_active(false);`
                    self.base.non_script_lock(crate::ai::AiLockFlags::FREEZE);
                    self.base.outbox.actor.deactivate = true;
                }
            }

            // ============ BULK-PORTED HANDLERS ============
            //
            // The block below ports the ~83 substates that were
            // previously swept into the no-op group when the
            // exhaustive-match refactor landed.  Many of these arms
            // call helpers that were not yet ported when this block first
            // landed. The remaining fallback arms below are kept explicit
            // and should be replaced with exact handlers as each parity
            // path is audited.

            // Empty case; script-driven
            // substate is handled elsewhere.  Kept here as an explicit arm to
            // document the mapping.
            Substate::FleeingRunForArrowReserves => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // Flag the engine drain to refill the archer's arrows —
                    // the engine-side `pending_refill_bow_ammo` processor
                    // writes `NpcData::number_of_arrows = MAX_NPC_ARROWS`.
                    self.base.outbox.actor.refill_bow_ammo = true;
                    self.seek_area(
                        sim,
                        self.base.seek_position,
                        parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                        SeekFlags::LOCATION_FIRST,
                        0,
                        global,
                        ctx,
                        tick,
                    );
                }
            }

            // Run to alert soldiers: on reach, attempt alert; else fall
            // through hide.
            Substate::FleeingRunToAlertSoldiers => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // AlertSoldiers; on false → fall through to RUN_TO_DOOR
                    // and re-dispatch REACHPOINT.
                    //
                    // This substate is only reached via
                    // RunAndAlertSoldiers, which is officer-only ("an
                    // officer runs away and alerts soldiers").
                    // `alert_soldiers` debug-asserts Officer rank, matching
                    // that contract.
                    let seek_flags_bits = self.seek_flags.bits();
                    let alerted = self.alert_soldiers(
                        self.base.seek_position,
                        seek_flags_bits,
                        global,
                        grid,
                        ctx,
                        tick,
                        AlertSoldiersFailureContinuation::FleeingRunToDoor,
                    );
                    if !alerted {
                        // Fire a self-stimulus so the re-delivery happens
                        // on the next think rather than recursing here
                        // (the reference's direct Think() call is a
                        // self-recursion that the Rust state-machine
                        // contract avoids).
                        self.set_state(AiState::Fleeing, Substate::FleeingRunToDoor);
                        self.base.fire_self_stimulus(StimulusType::EventReachPoint);
                    }
                }
            }

            // Retire from combat: reach → fast-turn toward seek position.
            Substate::FleeingRetireFromCombat => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.set_state(AiState::Fleeing, Substate::FleeingRetireFromCombatTurn);
                    self.base
                        .face_position_with_ctx(self.base.seek_position, ctx);
                }
            }

            // Turned: if detecting target, BattleDecisions, else overview.
            Substate::FleeingRetireFromCombatTurn => {
                if stimulus_type == StimulusType::EventDone {
                    if self.base.primary_target != 0
                        && self
                            .is_detecting_180_degrees(self.base.primary_target as HumanHandle, ctx)
                    {
                        self.battle_decisions(sim, global, ctx, tick, grid);
                    } else {
                        self.get_battle_overview(0, ctx, tick);
                    }
                }
            }

            // Sword fight step back reached point: resume swordfight with
            // 20-tick timer.
            _ => {}
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parade_timer_stops_only_an_active_normal_parry() {
        let sim = crate::sim_rng::test_context();

        for (action_state, should_stop) in [
            (crate::element::ActionState::WaitingSword, false),
            (crate::element::ActionState::ParryingSword, true),
            (crate::element::ActionState::ParryingSwordLow, false),
        ] {
            let mut ai = EnemyAi::new(1);
            ai.set_state(AiState::Attacking, Substate::AttackingSwordfightParade);
            let ctx = AiContext {
                frame: 325,
                self_action_state: action_state,
                ..AiContext::default()
            };

            ai.think_expected_event(
                &sim,
                &Stimulus::new(StimulusType::EventTimer),
                &mut AiGlobalState::default(),
                &ctx,
                &AiPerTickData::stub(),
                None,
            );

            assert_eq!(
                ai.base.outbox.actor.launch_commands
                    == vec![crate::element::Command::StopParrySword],
                should_stop,
                "unexpected stop-parry emission for {action_state:?}"
            );
            assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
            assert_eq!(ai.base.when_does_timer_ring, 345);
        }
    }

    fn pc_view(posture: crate::element::Posture) -> crate::ai_entity_view::AiEntityView {
        let entity = crate::element::Entity::Pc(crate::element::ActorPc {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorPc,
                posture,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        });
        crate::ai_entity_view::entity_view_from_entity(&entity, 41, false, None, None)
    }

    #[test]
    fn seeking_body_wake_up_preserves_pc_target_kind() {
        let sim_context = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.base.owner_entity_id = Some(crate::element::EntityId::Soldier(
            crate::entity_id::SoldierId(1),
        ));
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingBody;
        ai.base.detected_body = 17;

        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(17, pc_view(crate::element::Posture::Tied));
        let ctx = AiContext {
            entity_views: std::sync::Arc::new(views),
            ..AiContext::default()
        };

        ai.think_expected_event(
            &sim_context,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        let seq = ai
            .base
            .outbox
            .actor
            .launch_sequences
            .first()
            .expect("wake-up sequence");
        assert!(matches!(
            seq.elements[0].data,
            crate::sequence::SequenceElementData::Interaction {
                antagonist: Some(crate::element::EntityId::Pc(crate::entity_id::PcId(17)))
            }
        ));
    }

    #[test]
    fn arrow_watching_ignores_event_done() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        // Original: RHArtificialMalignity::ThinkExpectedEvent handles only
        // EVENT_TIMER and EVENT_MYTALK_1 for the just-watching substate.
        for substate in [
            Substate::SeekingArrowJustWatching,
            Substate::SeekingArrowJustWatchingSidewards,
        ] {
            let mut ai = EnemyAi::new(1);
            ai.set_state(AiState::Seeking, substate);
            let mut global = AiGlobalState::default();

            ai.think_expected_event(
                sim,
                &Stimulus::new(StimulusType::EventDone),
                &mut global,
                &AiContext::default(),
                &AiPerTickData::stub(),
                None,
            );

            assert_eq!(ai.base.current_state, AiState::Seeking);
            assert_eq!(ai.base.current_substate, substate);
        }
    }

    #[test]
    fn goto_chief_reach_uses_face_same_direction_shortcut() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(89);
        ai.set_state(AiState::Default, Substate::DefaultGotoChief);
        ai.base.patrol_chief = Some(crate::element::EntityId::Soldier(
            crate::entity_id::SoldierId(91),
        ));
        let ctx = AiContext {
            frame: 502,
            position: Position {
                x: 1078.101_9,
                y: 1783.098,
                ..Position::default()
            },
            direction: 12,
            self_action_state: crate::element::ActionState::Waiting,
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.patrol_chief_position = Position {
            x: 1042.116_3,
            y: 1783.832_4,
            ..Position::default()
        };

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert!(ai.base.already_turned);
        assert!(ai.base.outbox.actor.orders.is_empty());
        assert_eq!(
            ai.base.current_substate,
            Substate::DefaultPatrolEnrouteWaiting
        );
        assert_eq!(ai.base.when_does_timer_ring, 702);
    }

    #[test]
    #[should_panic(expected = "running on shooting path has no archery sector")]
    fn shooting_path_does_not_fabricate_an_end_of_path_recovery() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.set_state(
            AiState::Attacking,
            Substate::AttackingArcherRunOnShootingPath,
        );
        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );
    }

    #[test]
    #[should_panic(expected = "final sprint has no reserved shooting point")]
    fn shooting_path_final_sprint_requires_its_reserved_point() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.set_state(
            AiState::Attacking,
            Substate::AttackingArcherRunOnShootingPathFinalSprint,
        );
        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );
    }

    #[test]
    #[should_panic(expected = "receiving CALL_REPORT requires civilian 42 view")]
    fn civilian_report_does_not_fabricate_enemy_data_when_sender_is_missing() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.set_state(AiState::Seeking, Substate::SeekingWaitForAlertingCivilian);
        let mut stimulus = Stimulus::new(StimulusType::CallReport);
        stimulus.info = StimulusInfo::Hint(Hint {
            seek_point: Position::default(),
            seek_flags: 0,
            who_tells_me: 42,
        });
        ai.think_expected_event(
            &sim,
            &stimulus,
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );
    }
}
