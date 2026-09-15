use super::*;

impl EngineInner {
    /// Run the waypoint VM inside the caller's existing decision frame.
    pub(in crate::engine) fn execute_ai_waypoint_script(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        path_idx: crate::ai::PathId,
        wp_idx: u8,
    ) {
        if !sim.config().script_enabled {
            return;
        }
        let actor_handle = crate::natives::ScriptHandleCodec::actor_handle(npc_id);
        tracing::trace!(
            frame = self.control.frame_counter,
            owner = npc_id.index(),
            path = ?path_idx,
            wp = wp_idx,
            "waypoint ReachPoint dispatch"
        );
        if let Err(error) = self.call_script_vm(
            sim,
            assets,
            ScriptVmKey::Waypoint(path_idx, wp_idx),
            "ReachPoint",
            &[actor_handle],
            crate::natives::ScriptCallFrame::default(),
        ) {
            tracing::warn!(
                "Waypoint ReachPoint (path {path_idx}, wp {wp_idx}, actor {actor_handle}): {error}"
            );
            debug_assert!(
                false,
                "Waypoint ReachPoint (path {path_idx}, wp {wp_idx}, actor {actor_handle}): {error}"
            );
        }

        // The script may change the owner's state before this continuation.
        let script_driven = self
            .world
            .entities
            .get(npc_id)
            .and_then(Entity::ai_controller)
            .is_none_or(|ai| ai.current_substate == crate::ai::Substate::DefaultScriptDriven);
        if script_driven {
            return;
        }
        let stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::EventAfterScriptGoOn);
        self.dispatch_think_with_drain(sim, npc_id, &stimulus, assets);
    }

    /// Execute Original's route-arrival call stack without detaching the handler.
    /// Actor borrows end before callbacks; post-callback path reads use the
    /// authoritative actor. Actual Turn execution remains owned by SequenceManager.
    pub(in crate::engine) fn think_patrol_arrival(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &crate::ai::Stimulus,
    ) -> bool {
        let admitted = if self
            .expect_entity(owner, "patrol arrival")
            .enemy_ai()
            .is_some()
        {
            self.begin_enemy_think(sim, assets, owner, stimulus)
        } else {
            self.begin_friendly_think(sim, assets, owner, stimulus)
        };
        if !admitted {
            self.execute_ai_end_think(sim, assets, owner);
            return true;
        }

        // A ReachPoint admitted by the role gates retains the selected arm.
        assert_eq!(
            self.world
                .entities
                .expect_ai_controller(owner, format_args!("admitted patrol arrival"))
                .current_substate,
            crate::ai::Substate::DefaultGotoRoute
        );
        let handle = crate::natives::ScriptHandleCodec::actor_handle(owner);
        // State-change notifications ignore the callback's return value.
        self.call_ai_event_filter(
            sim,
            assets,
            handle,
            handle,
            crate::ai::AiState::Default.state_change_event_code(),
        );
        {
            let ai = self
                .world
                .entities
                .expect_ai_controller_mut(owner, format_args!("patrol state after callback"));
            ai.set_ai_state(crate::ai::AiState::Default);
            ai.current_substate = crate::ai::Substate::DefaultGotoRouteTurn;
        }
        self.initialize_patrol_for_npc(assets, owner);
        let position = self.live_ai_position(owner);
        let direction = self
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("patrol path"))
            .route_arrival_turn_direction(position, &assets.navigation.hiking_paths);
        if let Some(direction) = direction {
            let mut turn = crate::sequence::SequenceElement::new_generic(
                1,
                crate::element::Command::Turn,
                Some(owner),
            );
            turn.set_property(
                crate::sequence::Field::Direction,
                crate::sequence::FieldValue::Integer(u32::from(direction)),
            );
            self.launch_element(sim, assets, turn);
        } else {
            self.dispatch_filtered_stimulus_inner(
                sim,
                assets,
                owner,
                &crate::ai::Stimulus::new(crate::ai::StimulusType::EventDone),
            );
        }
        self.execute_ai_end_think(sim, assets, owner);
        false
    }

    /// Initialize the current theoretical patrol at this owner boundary.
    pub(in crate::engine) fn initialize_patrol_for_npc(
        &mut self,
        assets: &LevelAssets,
        chief_id: EntityId,
    ) {
        let member_count = self
            .world
            .entities
            .expect_ai_controller(
                chief_id,
                format_args!("synchronous patrol initialization owner"),
            )
            .theoretical_patrol
            .len();
        self.initialize_patrol_for_npc_prefix(assets, chief_id, member_count);
    }

    /// Initialize the captured prefix, reading each member from the live roster.
    /// Sort by raw world distance and arrange pairs using AI positions.
    pub(in crate::engine) fn initialize_patrol_for_npc_prefix(
        &mut self,
        assets: &LevelAssets,
        chief_id: EntityId,
        member_count: usize,
    ) {
        let chief_position = self.live_ai_position(chief_id);
        let chief_world = self
            .expect_entity(chief_id, "patrol assembly chief")
            .element_data()
            .position();
        let mut patrol = Vec::new();
        let mut missed = Vec::new();
        for index in 0..member_count {
            let id = *self
                .world
                .entities
                .expect_ai_controller(chief_id, format_args!("patrol assembly chief"))
                .theoretical_patrol
                .get(index)
                .expect("patrol assembly lost a member from its captured prefix");
            if id == chief_id {
                continue;
            }
            let entity = self.expect_entity(id, "patrol assembly member");
            let world = entity.element_data().position();
            let dx = world.x - chief_world.x;
            let dy = (world.y - chief_world.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = world.z - chief_world.z;
            let distance = dx * dx + dy * dy + dz * dz;
            self.world
                .entities
                .expect_entity_mut(id, format_args!("patrol sorting key"))
                .human_data_mut()
                .expect("patrol member human data")
                .sorting_distance = distance;
            let visible = self.patrol_member_visible(assets, chief_id, id);
            let entity = self.expect_entity(id, "patrol member admission");
            let state = self
                .world
                .entities
                .expect_ai_controller(id, format_args!("patrol member admission"))
                .current_state;
            let able = match entity {
                Entity::Soldier(soldier) => crate::element::Human::is_able_to_fight(soldier),
                Entity::Pc(pc) => crate::element::Human::is_able_to_fight(pc),
                _ => false,
            };
            if visible && state == crate::ai::AiState::Default && (entity.is_civilian() || able) {
                let index = patrol
                    .iter()
                    .position(|&prior| {
                        !(distance
                            > self
                                .expect_entity(prior, "prior patrol sorting key")
                                .human_data()
                                .expect("patrol member human data")
                                .sorting_distance)
                    })
                    .unwrap_or(patrol.len());
                patrol.insert(index, id);
                self.world
                    .entities
                    .expect_ai_controller_mut(id, format_args!("admitted patrol member"))
                    .patrol_chief = Some(chief_id);
            } else if !entity.is_dead() {
                missed.push(id);
            }
        }
        for pair_end in (1..patrol.len()).step_by(2) {
            let even = self.live_ai_position(patrol[pair_end - 1]);
            let odd = self.live_ai_position(patrol[pair_end]);
            let ex = even.x - chief_position.x;
            let ey = even.y - chief_position.y;
            let ox = odd.x - chief_position.x;
            let oy = odd.y - chief_position.y;
            if ex * oy - ey * ox < 0.0 {
                patrol.swap(pair_end - 1, pair_end);
            }
        }
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(chief_id, format_args!("patrol assembly chief"));
        ai.needs_patrol_reinit = false;
        ai.patrol = patrol;
        ai.missed_patrol_members = missed;
    }

    /// Patrol visibility borrows authoritative geometry only for the duration
    /// of the query; no actor or obstacle projection survives a callback.
    pub(super) fn patrol_member_visible(
        &self,
        assets: &LevelAssets,
        chief: EntityId,
        member: EntityId,
    ) -> bool {
        let chief = self.expect_entity(chief, "patrol visibility chief");
        let member = self.expect_entity(member, "patrol visibility member");
        if !chief.is_active() || !member.is_active() {
            return false;
        }
        let chief_element = chief.element_data();
        let member_element = member.element_data();
        patrol_member_visible_from_raw_world(
            chief_element.position(),
            chief.soldier_data().is_some_and(|soldier| soldier.rider),
            chief
                .ai_actor_data()
                .expect("patrol chief has no AI actor data")
                .view_radius,
            self.entity_data_in_building_sector(chief_element),
            member_element.position(),
            member_element.posture(),
            member.soldier_data().is_some_and(|soldier| soldier.rider),
            member_element.direction(),
            self.entity_data_in_building_sector(member_element),
            self.world.sight_obstacles(assets),
        )
    }

    // ── Every-16-frame AI tasks (staggered) ──────────────
    //
    // `the_16th_frame` runs every 16th frame from the NPC's
    // `hourglass`, staggered by NPC index so not all soldiers run on
    // the same frame.

    pub(in crate::engine) fn tick_periodic_ai_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        let current_frame = self.control.frame_counter;

        let entity = self.expect_entity(npc_id, "periodic NPC");

        // Exact original phase:
        //   (frame & 255) - ((register_number + 100) & 255)
        // with unsigned-byte wrap. Passing the full phase matters:
        // The periodic update uses bits 4..5 to reduce some work to every
        // 64th frame, so substituting `frame % 16` ran that work 4x.
        let register_number = entity
            .ai_actor_data()
            .unwrap_or_else(|| panic!("periodic entity {} is not an AI owner", npc_id.index()))
            .register_number;
        let frame_phase = npc_hourglass_frame_phase(current_frame, u32::from(register_number));
        if (frame_phase & 15) != 0 {
            return;
        }

        if entity.is_dead() {
            return;
        }

        let civilian = entity.is_civilian();
        if !civilian {
            self.run_enemy_periodic_prefix(sim, npc_id, assets);
            self.refresh_ai_arrow_protection(sim, assets, npc_id, true);
        }
        if frame_phase & 63 == 0 {
            self.finish_enemy_periodic_stuck_suffix_after_refresh(sim, npc_id, assets, frame_phase);
        }
    }

    fn run_enemy_periodic_prefix(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        if self
            .world
            .entities
            .expect_ai_controller(npc_id, format_args!("periodic wasp owner"))
            .current_substate
            == crate::ai::Substate::WonderingWaspInArmour
            && self.actor_command(npc_id) != crate::element::Command::ReceiveWaspSting
        {
            self.dispatch_think_with_drain(
                sim,
                npc_id,
                &crate::ai::Stimulus::new(crate::ai::StimulusType::EventWaspAway),
                assets,
            );
        }
        if self
            .world
            .entities
            .expect_ai_controller(npc_id, format_args!("periodic retreat owner"))
            .current_substate
            == crate::ai::Substate::FleeingMerryManRunToLeaveMap
            && self.actor_command(npc_id) == crate::element::Command::Wait
        {
            self.execute_ai_merry_man_forest_cassos(sim, assets, npc_id);
        }
        let frame = self.control.frame_counter;
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(npc_id, format_args!("periodic timer owner"));
        if !ai.timer_is_running
            && !self.ai.global.freeze
            && matches!(
                ai.current_substate,
                crate::ai::Substate::AttackingSwordfight | crate::ai::Substate::AttackingObserve
            )
        {
            ai.launch_timer(10, frame);
        }
        if self.live_actor_animation(npc_id) == Some(crate::order::OrderType::WaitingUprightBored)
            && self
                .world
                .entities
                .expect_ai_controller(npc_id, format_args!("periodic remark owner"))
                .current_state
                == crate::ai::AiState::Default
            && crate::sim_rng::u32(sim, crate::sim_rng::RngSite::VipIdleRemark, 0..12) == 0
        {
            let ai = self
                .world
                .entities
                .expect_enemy_ai(npc_id, format_args!("periodic remark owner"));
            let remark =
                if ai.get_rank(&assets.profile_manager) == crate::profiles::ProfileRank::Officer {
                    Some(crate::ai::Remark::OfficerComplains)
                } else if ai.is_vip {
                    Some(crate::ai::Remark::VipSpeaksToHimself)
                } else {
                    None
                };
            if let Some(remark) = remark {
                self.execute_ai_speech(
                    sim,
                    assets,
                    npc_id,
                    crate::ai::AiSpeechAttempt { remark, flags: 0 },
                );
            }
        }
    }

    /// Resume the watchdog after protection's synchronous movement and callbacks.
    pub(in crate::engine) fn finish_enemy_periodic_stuck_suffix_after_refresh(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        frame_phase: u8,
    ) {
        use crate::ai::{AiState, AlertLevel, Remark, Stimulus, StimulusType, Substate};
        let civilian = self
            .expect_entity(npc_id, "periodic watchdog owner")
            .is_civilian();
        let substate = self
            .world
            .entities
            .expect_ai_controller(npc_id, format_args!("periodic watchdog owner"))
            .current_substate;
        let enemy_reachpoint = matches!(
            substate,
            Substate::DefaultGotoPost
                | Substate::DefaultGotoRoute
                | Substate::DefaultEnroute
                | Substate::DefaultPatrolEnroute
                | Substate::DefaultPatrolEnrouteRunning
                | Substate::DefaultGotoChief
                | Substate::DefaultPatrolChiefReturnToPatrol
                | Substate::WonderingApproachingAle
                | Substate::WonderingApproachingMoney
                | Substate::WonderingRunningForMoney
                | Substate::WonderingApproachingToLoot
                | Substate::WonderingBrawlApproaching
                | Substate::WonderingOfficerApproachingBrawl
                | Substate::WonderingApproachingBrawlVictim
                | Substate::SeekingHeardsteps
                | Substate::SeekingArrow
                | Substate::SeekingBody
                | Substate::SeekingNet
                | Substate::SeekingSeekpoint
                | Substate::SeekingSeekpointPassedAmbushPointLeft
                | Substate::SeekingSeekpointPassedAmbushPointRight
                | Substate::SeekingSeekpointApproachingBeggar
                | Substate::SeekingSoldierGoToOfficer
                | Substate::SeekingSoldierReturnToOfficer
                | Substate::SeekingOfficerLeavingHouseToInstructGroup
                | Substate::SeekingGroupGoToOfficer
                | Substate::SeekingRunningToOfficer
                | Substate::SeekingRunningToOfficerSeen
                | Substate::SeekingCharly
                | Substate::SeekingCharlyGoToOfficer
                | Substate::SeekingCharlyGoToOfficerSeen
                | Substate::SeekingCombatAlert
                | Substate::AttackingRunningToEnemy
                | Substate::AttackingWalkingToEnemy
                | Substate::AttackingChargingEnemy
                | Substate::AttackingSwordfightStepBack
                | Substate::AttackingTooProudToAttackRetire
                | Substate::AttackingTooProudToAttackApproach
                | Substate::AttackingObserveAndMove
                | Substate::AttackingApproachingNewEnemy
                | Substate::AttackingMovingAroundOldEnemy
                | Substate::AttackingApproachingSleepingEnemy
                | Substate::AttackingArcherRetireFromCombat
                | Substate::AttackingRunningToPhalanx
                | Substate::AttackingArcherRunOnShootingPath
                | Substate::AttackingArcherRunOnShootingPathFinalSprint
                | Substate::AttackingDoorFightLeaving
                | Substate::AttackingRiderChargingApproaching
                | Substate::AttackingRiderChargingPassing
                | Substate::AttackingRiderChargingGettingDistance
                | Substate::AttackingRiderChargingApproachingBlindly
                | Substate::AttackingRunningToLadder
                | Substate::AttackingRunToAvengerOnRoof
                | Substate::FleeingPanic
                | Substate::FleeingRunToHide
                | Substate::FleeingRunToDoor
                | Substate::FleeingHiding
                | Substate::FleeingRunToAlertSoldiers
                | Substate::FleeingRetireFromCombat
                | Substate::FleeingMerryManRunToLeaveMap
                | Substate::FleeingRunForArrowReserves,
        );
        let in_reachpoint_arm = if civilian {
            matches!(
                substate,
                Substate::DefaultPatrolEnroute
                    | Substate::DefaultPatrolEnrouteRunning
                    | Substate::WonderingChildApproachingWhistling
                    | Substate::SeekingCivilianRunningToSoldier
                    | Substate::SeekingCivilianRunningToSoldierSeen
                    | Substate::FleeingChildChased
                    | Substate::FleeingChildChasedSupplementalRuns
                    | Substate::FleeingChildFriendChased
                    | Substate::DefaultGotoPost
                    | Substate::DefaultGotoRoute
                    | Substate::DefaultEnroute
                    | Substate::FleeingRunToHide
                    | Substate::FleeingRunToDoor
                    | Substate::FleeingPanic
            )
        } else {
            enemy_reachpoint
        };
        let command = self.actor_command(npc_id);
        let stuck_command = command == crate::element::Command::Wait
            || !civilian
                && matches!(
                    command,
                    crate::element::Command::SwordstrikeSmalltalkLeft
                        | crate::element::Command::SwordstrikeSmalltalkRight
                        | crate::element::Command::ParrySmalltalkLeft
                        | crate::element::Command::ParrySmalltalkRight
                );
        if !in_reachpoint_arm {
            self.world
                .entities
                .expect_ai_controller_mut(npc_id, format_args!("periodic watchdog reset"))
                .stuck_counter = 0;
        } else if stuck_command {
            let pending = self
                .orders
                .sequence_manager
                .element_is_about_to_be_launched(npc_id, crate::element::Command::Null);
            let ai = self
                .world
                .entities
                .expect_ai_controller_mut(npc_id, format_args!("periodic watchdog counter"));
            if pending {
                ai.stuck_counter = 0;
            } else if ai.stuck_counter < 3 {
                ai.stuck_counter += 1;
            } else {
                let destination = ai.last_goto_destination;
                let flags = ai.last_goto_flags;
                if destination.sector.is_some() {
                    self.duty_go_to(sim, assets, npc_id, destination, flags);
                } else {
                    self.dispatch_think_with_drain(
                        sim,
                        npc_id,
                        &Stimulus::new(StimulusType::EventCouldntReachPoint),
                        assets,
                    );
                }
                self.world
                    .entities
                    .expect_ai_controller_mut(
                        npc_id,
                        format_args!("periodic watchdog callback return"),
                    )
                    .stuck_counter = 0;
            }
        }
        if !civilian && frame_phase == 0 {
            let ai = self
                .world
                .entities
                .expect_ai_controller_mut(npc_id, format_args!("periodic alcohol owner"));
            if ai.blood_alcohol > 0 {
                if ai.current_music_alert_status == AlertLevel::Green
                    && ai.current_state != AiState::Sleeping
                    && ai.blood_alcohol > 20
                {
                    self.execute_ai_speech(
                        sim,
                        assets,
                        npc_id,
                        crate::ai::AiSpeechAttempt {
                            remark: Remark::Drunken,
                            flags: 0,
                        },
                    );
                }
                self.world
                    .entities
                    .expect_ai_controller_mut(npc_id, format_args!("periodic alcohol decay"))
                    .blood_alcohol -= 1;
            }
        }
    }

    /// Civilian random speech during the NPC update, keyed by frame phase.
    /// It sits before the lock gate and only acts at exact phase zero.
    pub(in crate::engine) fn tick_civilian_random_speech_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        let current_frame = self.control.frame_counter;
        let entity = self.expect_entity(npc_id, "random-speech NPC");
        let Entity::Civilian(civilian) = entity else {
            return;
        };
        let register_number = civilian.npc.register_number;
        let frame_phase = npc_hourglass_frame_phase(current_frame, u32::from(register_number));
        let debug_creation_order = {
            let gate = civilian_random_speech_debug_gate();
            if !gate.matches([Some(current_frame), None]) {
                None
            } else {
                let creation_order = self.world.original_creation_order(npc_id);
                gate.matches([None, Some(creation_order)])
                    .then_some(creation_order)
            }
        };
        if let Some(creation_order) = debug_creation_order {
            Self::trace_civilian_random_speech_eligibility(
                [current_frame, creation_order],
                npc_id,
                entity,
                register_number,
                (frame_phase, frame_phase == 0),
            );
        }
        if frame_phase != 0 {
            return;
        }

        let is_beggar =
            civilian.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar;
        // The former full owner context required a position layer even when
        // the spatial view was unavailable. Preserve that invariant check.
        let _ = entity.element_data().layer();
        let animation = entity_has_ai_view(entity).then(|| {
            self.live_actor_animation(npc_id)
                .unwrap_or(crate::order::OrderType::NonanimationEnd)
        });
        let entity = self.expect_entity_mut(npc_id, "random-speech NPC before call");
        if let Some(creation_order) = debug_creation_order {
            Self::trace_civilian_random_speech_before_call(
                [current_frame, creation_order],
                npc_id,
                entity,
                animation,
            );
        }
        if is_beggar {
            let ai = self.reporting_civilian_mut(npc_id);
            if ai.beggar_dont_talk_counter > 0 {
                ai.beggar_dont_talk_counter -= 1;
            } else if ai.base.current_remark == crate::ai::Remark::TheSoundOfSilence
                && crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CivilianBeggarSpeechGate, 0..3)
                    == 0
            {
                let remark = match crate::sim_rng::u32(
                    sim,
                    crate::sim_rng::RngSite::CivilianBeggarSpeechChoice,
                    0..5,
                ) {
                    0..=2 => crate::ai::Remark::CivBeggarBegging,
                    3 => crate::ai::Remark::CivUnderNet,
                    4 => crate::ai::Remark::CivCries,
                    _ => unreachable!(),
                };
                self.execute_ai_speech(
                    sim,
                    assets,
                    npc_id,
                    crate::ai::AiSpeechAttempt { remark, flags: 0 },
                );
            }
        }
        if self.live_actor_animation(npc_id) == Some(crate::order::OrderType::Weeping) {
            self.execute_ai_speech(
                sim,
                assets,
                npc_id,
                crate::ai::AiSpeechAttempt {
                    remark: crate::ai::Remark::CivCries,
                    flags: 0,
                },
            );
        }
        if let Some(creation_order) = debug_creation_order {
            self.trace_civilian_random_speech_after_call(current_frame, creation_order, npc_id);
        }
        // Complete actor effects before the following NPC lock gate.
        if let Some(creation_order) = debug_creation_order {
            self.trace_civilian_random_speech_after_drain(current_frame, creation_order, npc_id);
        }
    }

    /// `[frame, creation order]`; `frame_phase` pairs the phase with its
    /// `== 0` call verdict.
    #[inline(never)]
    fn trace_civilian_random_speech_eligibility(
        [current_frame, creation_order]: [u32; 2],
        npc_id: EntityId,
        entity: &Entity,
        register_number: impl std::fmt::Display,
        (frame_phase, will_call): (impl std::fmt::Display, bool),
    ) {
        let Entity::Civilian(civilian) = entity else {
            panic!("random-speech NPC {} is not a civilian", npc_id.index())
        };
        let crate::element::AiBrain::Friendly(ai) = &civilian.npc.ai_brain else {
            panic!(
                "random-speech civilian {} has non-friendly AI",
                npc_id.index()
            )
        };
        eprintln!(
            "[CIVRANDSPEECH frame={current_frame} co={creation_order} owner={} phase=eligibility register={register_number} frame_phase={frame_phase} human_hourglass_continued=true active={} profile={} civilian_type={:?} is_beggar={} dont_talk={} current_remark={:?} remark_flags={} ai_locks={:?} script_locked={} will_call={} will_draw_gate={}]",
            npc_id.index(),
            civilian.element.active,
            civilian.civilian.civilian_profile_index.0,
            civilian.civilian.cached_civilian_type,
            civilian.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar,
            ai.beggar_dont_talk_counter,
            ai.base.current_remark,
            ai.base.current_remark_flags,
            ai.base.locks_flag_field,
            ai.base.script_locked,
            will_call,
            will_call
                && civilian.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar
                && ai.beggar_dont_talk_counter == 0
                && ai.base.current_remark == crate::ai::Remark::TheSoundOfSilence,
        );
    }

    #[inline(never)]
    fn trace_civilian_random_speech_before_call(
        [current_frame, creation_order]: [u32; 2],
        npc_id: EntityId,
        entity: &Entity,
        source_animation: Option<crate::order::OrderType>,
    ) {
        let Entity::Civilian(civilian) = entity else {
            panic!(
                "random-speech civilian {} changed entity kind before call",
                npc_id.index()
            )
        };
        let crate::element::AiBrain::Friendly(ai) = &civilian.npc.ai_brain else {
            panic!("random-speech civilian {} changed AI kind", npc_id.index())
        };
        eprintln!(
            "[CIVRANDSPEECH frame={current_frame} co={creation_order} owner={} phase=before_call source_animation={source_animation:?} source_is_weeping={} live_animation={:?} current_remark={:?}]",
            npc_id.index(),
            source_animation == Some(crate::order::OrderType::Weeping),
            civilian.element.sprite.last_action,
            ai.base.current_remark,
        );
    }

    #[inline(never)]
    fn trace_civilian_random_speech_after_call(
        &self,
        current_frame: u32,
        creation_order: u32,
        npc_id: EntityId,
    ) {
        let Entity::Civilian(civilian) = self.expect_entity(npc_id, "random-speech civilian")
        else {
            panic!(
                "random-speech civilian {} changed entity kind",
                npc_id.index()
            )
        };
        let crate::element::AiBrain::Friendly(ai) = &civilian.npc.ai_brain else {
            panic!("random-speech civilian {} changed AI kind", npc_id.index())
        };
        eprintln!(
            "[CIVRANDSPEECH frame={current_frame} co={creation_order} owner={} phase=after_call_before_drain dont_talk={} current_remark={:?} remark_flags={} live_animation={:?}]",
            npc_id.index(),
            ai.beggar_dont_talk_counter,
            ai.base.current_remark,
            ai.base.current_remark_flags,
            civilian.element.sprite.last_action,
        );
    }

    #[inline(never)]
    fn trace_civilian_random_speech_after_drain(
        &self,
        current_frame: u32,
        creation_order: u32,
        npc_id: EntityId,
    ) {
        let Entity::Civilian(civilian) =
            self.expect_entity(npc_id, "random-speech civilian after drain")
        else {
            panic!(
                "random-speech civilian {} changed entity kind after drain",
                npc_id.index()
            )
        };
        let crate::element::AiBrain::Friendly(ai) = &civilian.npc.ai_brain else {
            panic!("random-speech civilian {} changed AI kind", npc_id.index())
        };
        eprintln!(
            "[CIVRANDSPEECH frame={current_frame} co={creation_order} owner={} phase=after_drain current_remark={:?} remark_flags={} live_animation={:?}]",
            npc_id.index(),
            ai.base.current_remark,
            ai.base.current_remark_flags,
            civilian.element.sprite.last_action,
        );
    }

    // ── Per-frame ambush-point peek scan ─────────
    //
    // `refresh_ambush_points` runs every frame for each NPC from
    // `hourglass`. Civilians have no corresponding response, so this only
    // fires for enemies (soldiers).  The per-NPC method updates the
    // slot status vector and may transition the AI substate via
    // `check_ambush_point`.

    pub(in crate::engine) fn tick_refresh_ambush_points_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        use crate::ai::{AiState, LookDirection, Substate};
        use crate::ai_enemy::AmbushPointStatus;
        if self.actors_frozen() || self.ai.global.ambush_points.is_empty() {
            return;
        }
        let owner = self.expect_entity(npc_id, "ambush-refresh NPC");
        if matches!(owner, Entity::Civilian(_)) {
            return;
        }
        let ai = owner.enemy_ai().expect("ambush refresh requires enemy AI");
        let iq = ai.iq_for_difficulty(
            &assets.profile_manager,
            self.control.sim_config.difficulty,
            self.mission_domain
                .diplomacy
                .relationship_to_player(owner.camp())
                == crate::diplomacy::Relationship::Hostile,
        );
        if iq <= crate::parameters_ai::AI_MIN_IQ_TO_CONTROL_AMBUSH_POINTS as u16 {
            return;
        }
        let substate = ai.base.current_substate;
        if !matches!(
            substate,
            Substate::SeekingSeekpoint
                | Substate::SeekingSeekpointPassedAmbushPointLeft
                | Substate::SeekingSeekpointPassedAmbushPointRight
        ) {
            let ai = self.observation_ai_mut(npc_id);
            if !ai.ambush_point_array_reset {
                ai.ambush_point_status.fill(AmbushPointStatus::Far);
                ai.ambush_point_array_reset = true;
            }
            return;
        }
        let more_than_one = substate == Substate::SeekingSeekpoint
            && ai
                .ambush_point_status
                .iter()
                .filter(|s| **s == AmbushPointStatus::Near)
                .count()
                > 1;
        let element = owner.element_data();
        let point = element.position_map();
        let level = element.layer();
        let sector = element.sector();
        let count = self.ai.global.ambush_points.len();
        assert_eq!(
            ai.ambush_point_status.len(),
            count,
            "ambush status slots must match authored points"
        );
        for idx in 0..count {
            let near = self.ai.global.ambush_points[idx].is_near(point, level, sector);
            let status = self.observation_ai(npc_id).ambush_point_status[idx];
            if !near {
                if status != AmbushPointStatus::Far {
                    self.observation_ai_mut(npc_id).ambush_point_status[idx] =
                        AmbushPointStatus::Far;
                }
                continue;
            }
            if status == AmbushPointStatus::Checked {
                continue;
            }
            let eyes = self
                .expect_entity(npc_id, "ambush eyes")
                .compute_eyes_point(None)
                .expect("ambush owner requires eyes");
            let anchor = self.ai.global.ambush_points[idx].position_3d;
            let reachable = crate::sight_obstacle::is_reachable_3d(
                crate::sight_obstacle::ObstacleList {
                    static_obstacles: &assets.environment.static_sight_obstacles,
                    dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                    static_active: &self.world.static_sight_obstacle_active,
                },
                [eyes.x, eyes.y, eyes.z],
                [anchor.x, anchor.y, anchor.z],
                crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
            );
            if status == AmbushPointStatus::Far {
                let ai = self.observation_ai_mut(npc_id);
                ai.ambush_point_status[idx] = if reachable {
                    AmbushPointStatus::Checked
                } else {
                    AmbushPointStatus::Near
                };
                ai.ambush_point_array_reset = false;
            } else if reachable {
                let position = self.live_ai_position(npc_id);
                let direction = self
                    .expect_entity(npc_id, "ambush direction")
                    .element_data()
                    .direction();
                let (dx, dy) = crate::element::direction_vector_16(direction as i16);
                let ambush = self.ai.global.ambush_points[idx].position;
                let right = dx * (ambush.y - position.y) - dy * (ambush.x - position.x) > 0.0;
                let substate = self.observation_ai(npc_id).base.current_substate;
                let opposite = if right {
                    Substate::SeekingSeekpointPassedAmbushPointLeft
                } else {
                    Substate::SeekingSeekpointPassedAmbushPointRight
                };
                let look = if substate == opposite {
                    Some(LookDirection::LeftRight)
                } else if !more_than_one {
                    Some(if right {
                        LookDirection::Right
                    } else {
                        LookDirection::Left
                    })
                } else {
                    None
                };
                let next = if look.is_some() {
                    Substate::SeekingSeekpointCheckingAmbushPoint
                } else if right {
                    Substate::SeekingSeekpointPassedAmbushPointRight
                } else {
                    Substate::SeekingSeekpointPassedAmbushPointLeft
                };
                self.duty_set_state(sim, assets, npc_id, AiState::Seeking, next);
                if let Some(look) = look {
                    self.execute_ai_look_sidewards(sim, assets, npc_id, look);
                } else {
                    let frame = self.control.frame_counter;
                    self.observation_ai_mut(npc_id).base.launch_timer(3, frame);
                }
                self.observation_ai_mut(npc_id).ambush_point_status[idx] =
                    AmbushPointStatus::Checked;
            }
        }
    }

    // ── Macro timer hourglass ────────────────────────────────────
    //
    // `hourglass` polls `macro_timer_is_running` each frame and, when
    // the timer has rung and the NPC is still in
    // `SUBSTATE_DEFAULT_INMACRO`, calls `run_ai_macro`
    // directly — **bypassing** decision stimulus dispatch so
    // CMD_WAIT / CMD_BEND resume without going through EVENT_TIMER.
    //
    // We iterate both soldier and civilian NPCs because civilians use
    // the common macro opcodes too (REVERSE_PATH, WAIT, GOTO_POINT,
    // FACE_TO, ...).
    pub(in crate::engine) fn tick_ai_macro_timer_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        let current_frame = self.control.frame_counter;

        // Read macro-timer state without holding a borrow. The original stops
        // an elapsed macro timer even outside DefaultInMacro; only execution
        // is substate-gated.
        let (fire, execute) = {
            let ai = self
                .world
                .entities
                .expect_ai_controller(npc_id, format_args!("macro-timer NPC"));
            let fire = ai.macro_timer_is_running && ai.when_does_macro_timer_ring <= current_frame;
            (
                fire,
                fire && ai.current_substate == crate::ai::Substate::DefaultInMacro,
            )
        };
        if !fire {
            return;
        }

        self.world
            .entities
            .expect_ai_controller_mut(npc_id, format_args!("macro-timer NPC"))
            .macro_timer_is_running = false;
        if execute {
            self.run_ai_macro(sim, assets, npc_id);
        }
    }

    // ── Locked-frame timer bumps ─────────────────────────────────
    //
    // `hourglass` short-circuits the post-Refresh tail when any lock
    // is held (`locks_flag_field > 0 || script_locked || frozen_all`)
    // but still bumps `when_does_timer_ring`,
    // `when_does_macro_timer_ring`, and `emoticon_expiration_date`
    // per locked frame.  Without this, the per-piece tick guards
    // skip everything (no bumps), so ring-times shift -N once the
    // lock clears — a script-locked civilian's EVENT_TIMER would
    // fire immediately on unlock instead of N frames later.
    //
    // The decision returned here is the one and only lock sample for this
    // owner suffix. Once it is false, later periodic-update/AI-decision side effects
    // may acquire locks or FrozenAll without suppressing the already-entered
    // normal timer, macro timer, or emoticon phases. Only the retained FIFO
    // intentionally samples AI/script locks again before every item.
    /// Original-game deafness query immediately after
    /// ambush-point refresh. This runs for every non-frozen owner even when
    /// acoustic detection's staggered cadence did not open this frame.
    pub(in crate::engine) fn tick_npc_refresh_deafness_for_npc(&mut self, npc_id: EntityId) {
        if self.actors_frozen() {
            return;
        }
        let (position, elevation) = {
            let entity = self.expect_entity(npc_id, "deafness-refresh NPC");
            assert!(
                entity.ai_actor_data().is_some(),
                "deafness-refresh owner {} has no AI data",
                npc_id.index()
            );
            (
                entity.element_data().position_map(),
                entity.element_data().position().z,
            )
        };
        let cover_volume = self
            .feedback
            .sound_sim
            .sources
            .max_noise_covering_volume_for_3d(position.x, position.y, elevation);
        let current_frame = self.control.frame_counter;
        self.world
            .entities
            .expect_ai_actor_data_mut(npc_id, format_args!("deafness-refresh NPC before apply"))
            .get_deafness(current_frame, cover_volume);
    }

    pub(in crate::engine) fn tick_npc_lock_gate_for_npc(&mut self, npc_id: EntityId) -> bool {
        let frozen = self.actors_frozen();
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(npc_id, format_args!("lock-gate NPC"));
        let locked = frozen || !ai.locks_flag_field.is_empty() || ai.script_locked;
        if locked {
            // The original game's unsigned increment wraps. Saturation would pin a deadline forever
            // after one overflow and break the later elapsed checks.
            ai.when_does_timer_ring = ai.when_does_timer_ring.wrapping_add(1);
            ai.when_does_macro_timer_ring = ai.when_does_macro_timer_ring.wrapping_add(1);
            ai.emoticon_expiration_date = ai.emoticon_expiration_date.wrapping_add(1);
        }
        locked
    }

    pub(in crate::engine) fn tick_npc_emoticon_expiration_for_npc(&mut self, npc_id: EntityId) {
        let current_frame = self.control.frame_counter;
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(npc_id, format_args!("emoticon-expiry NPC"));
        if ai.emoticon_has_expiration_date && ai.emoticon_expiration_date <= current_frame {
            ai.set_emoticon(crate::ai::EmoticonType::None);
            assert!(!ai.emoticon_has_expiration_date);
        }
    }

    // ── Stuck-on-ladder emergency counter ────────────────────────
    //
    // `hourglass` bumps `stuck_on_ladder_emergency_counter` every
    // frame an NPC is on a ladder in a non-building sector with
    // command `Wait`/`MoveWaiting` and not script-locked; otherwise
    // resets to 0.  After 25 frames it calls `force_return_to_duty()`
    // (== `return_to_duty(sim, )`) and resets the counter so
    // outdoor-ladder hangs self-recover.
    //
    // Note: this checks only `script_locked`, *not* `locks_flag_field`
    // — so the freshly-set BUSY lock from the edge detector earlier in
    // the same frame does not suppress this counter (the BUSY lock is
    // exactly what we want to escape from).
    pub(in crate::engine) fn tick_npc_stuck_on_ladder_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        // Snapshot the gating predicates without holding a borrow.
        let entity = self.expect_entity(npc_id, "ladder-tail NPC");
        let on_ladder = entity.element_data().posture() == crate::element::Posture::OnLadder;
        let cmd = self.actor_command(npc_id);
        let in_wait_or_move_waiting = matches!(
            cmd,
            crate::element::Command::Wait | crate::element::Command::MoveWaiting
        );
        let script_locked = entity
            .ai_controller()
            .unwrap_or_else(|| panic!("ladder-tail NPC {} has no AI", npc_id.index()))
            .script_locked;
        let in_building = self.entity_data_in_building_sector(entity.element_data());
        let qualifies = on_ladder && in_wait_or_move_waiting && !script_locked && !in_building;

        // Bump or reset the counter; remember whether to fire.
        let trigger = {
            let npc = self
                .world
                .entities
                .expect_ai_actor_data_mut(npc_id, format_args!("ladder-tail NPC before counter"));
            if qualifies {
                npc.stuck_on_ladder_emergency_counter =
                    npc.stuck_on_ladder_emergency_counter.saturating_add(1);
                if npc.stuck_on_ladder_emergency_counter > 25 {
                    npc.stuck_on_ladder_emergency_counter = 0;
                    true
                } else {
                    false
                }
            } else {
                npc.stuck_on_ladder_emergency_counter = 0;
                false
            }
        };
        if !trigger {
            return;
        }

        self.execute_ai_return_to_duty(sim, assets, npc_id, crate::ai::DutyFlags::empty());
    }
}
