use super::*;

impl EngineInner {
    /// Recompute overall villain alert status from soldier NPCs, updating
    /// global counters and triggering combat/alert music transitions.
    ///
    /// Ports the per-NPC work of `change_alert_status` into a
    /// single-shot sweep that runs once per frame. The per-NPC
    /// `set_alert_status` already writes `current_music_alert_status`
    /// but doesn't touch the global counters or call
    /// `set_music_mode`; this method fills that gap.
    ///
    /// Call once per frame before the sound `hourglass` so a transition
    /// to yellow/red promptly bumps the music pool weight.
    pub(crate) fn update_overall_villain_alert(
        &mut self,
        profiles: &crate::profiles::ProfileManager,
    ) {
        let mut yellow = 0u16;
        let mut red = 0u16;
        let mut green = 0u16;
        // Per-call `ALERT_INSTANT_MUSIC_CHANGE` flag is staged on each
        // AiController by `set_alert_status_with_flags`; OR it across
        // soldiers here and clear after consumption.  Non-soldier flags
        // are ignored to match the soldier-only gate.
        let mut any_instant_change = false;
        for (_, soldier) in self.world.entities.soldiers_mut() {
            let Some(ai) = soldier.npc.ai_brain.base_mut() else {
                continue;
            };
            match ai.current_music_alert_status {
                crate::ai::AlertLevel::Green => green += 1,
                crate::ai::AlertLevel::Yellow => yellow += 1,
                crate::ai::AlertLevel::Red => red += 1,
            }
            if ai.outbox.music.instant_change {
                any_instant_change = true;
                ai.outbox.music.instant_change = false;
            }
        }
        self.ai.global.green_alert_soldiers = green;
        self.ai.global.yellow_alert_soldiers = yellow;
        self.ai.global.red_alert_soldiers = red;

        let new_overall = self.ai.global.overall_villain_alert();
        if new_overall == self.ai.global.overall_villain_alert_status {
            return;
        }
        let prev = self.ai.global.overall_villain_alert_status;
        self.ai.global.overall_villain_alert_status = new_overall;
        self.ai.global.overall_alert_status = new_overall;

        // Only call `set_music_mode` when not in Sherwood.  Sherwood
        // has its own ambient track and shouldn't hear combat/alert
        // cues even if a soldier briefly goes yellow.
        let is_sherwood = Some(&self.mission_domain.campaign)
            .and_then(|c| c.current_mission_idx)
            .and_then(|idx| Some(&self.mission_domain.campaign).and_then(|c| c.missions.get(idx)))
            .is_some_and(|m| {
                m.profile(profiles).location == crate::profiles::MissionLocation::Sherwood
            });

        if !is_sherwood {
            use crate::sound::MusicMode;
            // On the Green arm, forest levels keep the alert track
            // instead of dropping to quiet so the woodland ambient
            // layer keeps playing under any residual yellow soldiers.
            let mode = match new_overall {
                crate::ai::AlertLevel::Green => {
                    if self.world.weather.is_forest_level {
                        MusicMode::Alert
                    } else {
                        MusicMode::Quiet
                    }
                }
                crate::ai::AlertLevel::Yellow => MusicMode::Alert,
                crate::ai::AlertLevel::Red => MusicMode::Fight,
            };
            // `set_alert_status` calls `force_music_mode` when the
            // caller passes `ALERT_INSTANT_MUSIC_CHANGE`.  Known
            // shipped call sites are all Green-target (two AI sites
            // and the NPC death path).  The flag is now staged per-NPC
            // on `AiController::pending_instant_music_change` by
            // `set_alert_status_with_flags`; the sweep above OR'd it
            // across soldiers into `any_instant_change`, so any
            // transition direction passing the flag forces immediately.
            let cmd = if any_instant_change {
                super::SoundCommand::ForceMusicMode(mode)
            } else {
                super::SoundCommand::SetMusicMode(mode)
            };
            self.feedback.pending_side_effects.sounds.push(cmd);
        }

        tracing::debug!(
            "Overall villain alert {:?} → {:?} (green={green} yellow={yellow} red={red})",
            prev,
            new_overall,
        );
    }

    // ─── Turn order processing ──────────────────────────────────

    /// Process pending turn orders from NPC order queues.
    ///
    /// `face_direction` / `face_position` produce `Turning` orders that
    /// `process_pending_ai_orders` routes to `actor.order_queue`.
    /// These become `Turn` sequence elements that complete in one
    /// frame and fire `EventDone`.  We replicate that here: set the
    /// entity's direction toward the target position, then dispatch
    /// `EventDone` so the AI state machine continues.
    /// Drain animation-type orders (Pointing, RaisingShield, LoweringShield,
    /// Menacing, etc.) from NPC order queues and start them as `active_ai_anim`.
    /// Like `process_turn_orders` but for multi-frame animations that
    /// need EventDone when the sprite animation completes.
    pub(in crate::engine) fn process_animation_orders(&mut self) {
        // Legacy entry point — left as a no-op now that the animation
        // driver reads the front order directly via
        // `current_order_for_actor`.  Animations booked onto sequence
        // elements are picked up automatically; there is no longer a
        // separate drain-and-rebook step.
    }

    // ─── EventGaloppLoopEnd dispatch ────────────────────────────

    #[cfg(test)]
    pub(in crate::engine) fn set_galopp_dispatch_observer(
        observer: Option<Box<dyn FnMut(&EngineInner, EntityId)>>,
    ) {
        GALOPP_DISPATCH_OBSERVER.with(|slot| *slot.borrow_mut() = observer);
    }

    /// Dispatch `EventGaloppLoopEnd` to riders with `RHMOVE_RIDER_CHARGE`
    /// flag that reached an intermediate waypoint during movement.
    ///
    /// When a rider's running animation reaches half/end frame with
    /// the `RIDER_CHARGE` move flag, `think(EVENT_GALOPP_LOOP_END)`
    /// fires so the AI can call `maybe_make_rider_attack()` to check
    /// if it's close enough to begin the actual charge pass.
    pub(in crate::engine) fn dispatch_galopp_loop_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let scratch = self.build_sim_scratch(sim, assets);
        let current_frame = self.control.frame_counter;
        let entity = self.world.entities.get(entity_id).unwrap_or_else(|| {
            panic!("rider {entity_id:?} disappeared before its synchronous GALOPP Execute callback")
        });
        let soldier = entity.soldier_data().unwrap_or_else(|| {
            panic!("GALOPP Execute callback owner {entity_id:?} is not a soldier")
        });
        assert!(
            soldier.rider,
            "GALOPP Execute callback owner {entity_id:?} is not a rider"
        );
        let ctx = build_ai_context_from_entity(
            entity,
            current_frame,
            None,
            self.world.weather.is_forest_level,
            self.world.weather.ambiance,
            self.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &self.world.fast_grid,
            &assets.hiking_paths,
            &assets.hiking_waypoint_sectors,
            &self.ai.global.all_soldier_handles,
            self.control.sim_config.difficulty,
        );

        let stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::EventGaloppLoopEnd);
        // EventGaloppLoopEnd fires on enemy riders mid-charge towards their
        // primary target. Think and every order/script callback it creates
        // close here, before Actor::Hourglass can complete this movement or
        // the mutable legacy walk can advance to the next owner.
        let tick_data = self.build_npc_tick_data(sim, entity_id, &scratch, assets);
        self.dispatch_think_with_drain(sim, entity_id, &stimulus, &ctx, &tick_data, assets);
        #[cfg(test)]
        GALOPP_DISPATCH_OBSERVER.with(|observer| {
            if let Some(observer) = observer.borrow_mut().as_mut() {
                observer(self, entity_id);
            }
        });
    }

    /// Map a PC's currently-executing animation (`OrderType`) to the
    /// noise volume they produce, via a per-animation switch in
    /// `refresh_produced_noise`.
    ///
    /// `refresh_produced_noise` runs from `hourglass()` each frame and
    /// reads `get_animation()` — the currently-running animation — to
    /// set `currently_produced_noise.volume`.  We reproduce the same
    /// lookup here from the PC's active `OrderType` (peeked from
    /// `actor.order_queue`, the equivalent of the sequence slot that
    /// `get_animation` reads).
    ///
    /// Material selects the walk/run/drop volume (wood = loud, grass =
    /// quiet, water = noisiest, light-shadow = silent).  The jump,
    /// sword-fight and breath volumes are material-independent.
    ///
    /// Returns the volume and whether Original reaches the final
    /// `mboxHearMyNoiseBox` rebuild. The inactive/building and breath arms
    /// return early and deliberately preserve the previous box.
    fn pc_noise_volume(
        order_type: crate::order::OrderType,
        material: crate::element::GameMaterial,
        in_building: bool,
        active: bool,
        prev_volume: u16,
    ) -> (u16, bool) {
        use crate::element::GameMaterial as Material;
        use crate::order::OrderType as OT;

        // When the actor is inside a building or inactive, the volume
        // is forced to 0.  Hearing then becomes impossible because the
        // current noise is silent, but Original returns before updating the
        // persistent hear-my-noise box.
        if in_building || !active {
            return (0, false);
        }

        // Walk/run/drop volumes per material.
        let (walk, run, drop) = match material {
            Material::Ground => (20, 70, 50),
            Material::Wood => (40, 150, 100),
            Material::Stone => (20, 75, 50),
            Material::Grass => (40, 150, 100), // GRASS_DRY
            Material::Leaves => (10, 50, 30),  // GRASS_FRESH
            Material::Water => (200, 400, 300),
            Material::Bush => (40, 150, 100),
            Material::Ice => (20, 75, 50),
            // LightShadow has no assignment in either the walk or run
            // switch, so `volume` keeps whatever it was on the prior
            // frame.  Substitute `prev_volume` for the walk and run
            // slots.  `drop` (Rolling / CarryingCorpse) has no
            // counterpart in `refresh_produced_noise`, so keep the
            // pre-existing 50 fallback.
            Material::LightShadow => (prev_volume, prev_volume, 50),
            _ => (20, 70, 50), // default = ground
        };

        // NOISE_VOLUME_* constants.
        const BREATH: u16 = 15;
        const SWORDFIGHT: u16 = 200;
        const JUMP_UP: u16 = 50;
        const JUMP_LONG: u16 = 50;
        const JUMP_DOWN: u16 = 80;

        match order_type {
            // ── BREATH: idle, bow aim, sitting, freezing ──
            OT::WaitingUprightBored
            | OT::WaitingUprightBoredRandom
            | OT::WaitingUpright
            | OT::WaitingCrouched
            | OT::TransitionEquipBow
            | OT::TransitionUnequipBow
            | OT::TransitionLoadingBow
            | OT::TransitionUnloadBow
            | OT::TransitionRaisingBow
            | OT::TransitionLoweringBow
            | OT::AimingWithBow
            | OT::AimingWithBowUp
            | OT::ShootingWithBow
            | OT::ShootingWithBowUp
            | OT::Freezing
            | OT::WaitingFreeLift
            | OT::Sitting
            | OT::TransitionWaitingUprightSitting
            | OT::TransitionSittingWaitingUpright => (BREATH, false),

            // ── WALK (material-dependent) ──
            OT::WalkingUpright
            | OT::TransitionWaitingUprightBoredWaitingUpright
            | OT::TransitionWaitingUprightWaitingUprightBored
            | OT::TransitionWaitingUprightWalkingUpright
            | OT::WalkingStairs
            | OT::TransitionCrouchingUp
            | OT::TransitionCrouchingDown
            | OT::TransitionWaitingUprightClimbingWallUp
            | OT::ClimbingWallUp
            | OT::ClimbingWallDown
            | OT::TransitionClimbingWallUpWaitingCrouchedCrenel
            | OT::TransitionWaitingCrouchedClimbingWallDownCrenel
            | OT::TransitionClimbingWallUpWaitingCrouched
            | OT::TransitionClimbingWallDownWaitingUpright
            | OT::TransitionWaitingCrouchedClimbingWallDown
            | OT::TransitionWaitingUprightClimbingLadderUp
            | OT::ClimbingLadderUp
            | OT::TransitionClimbingLadderUpWaitingCrouched
            | OT::TransitionWaitingCrouchedClimbingLadderDown
            | OT::ClimbingLadderDown
            | OT::TransitionClimbingLadderDownWaitingUpright
            | OT::StandingUp
            | OT::Turning
            | OT::TransitionWalkingUprightWaitingUpright
            | OT::PassingDoor
            | OT::WalkingWithSword
            | OT::TransitionWaitingCrouchedWalkingCrouched
            | OT::WalkingCrouched
            | OT::TransitionWalkingCrouchedWaitingCrouched
            | OT::TransitionWalkingUprightWalkingCrouched
            | OT::TransitionWalkingCrouchedWalkingUpright => (walk, true),

            // ── RUN (material-dependent) ──
            OT::RunningUpright
            | OT::TransitionWalkingUprightRunningUpright
            | OT::TransitionRunningUprightWalkingUpright
            | OT::TransitionRunningUprightWaitingUpright
            | OT::TransitionWaitingUprightRunningUpright
            | OT::TransitionRunningUprightWalkingCrouched
            | OT::TransitionWalkingCrouchedRunningUpright
            | OT::RunningStairs
            | OT::ClimbingLadderUpFast
            | OT::ClimbingLadderDownFast
            | OT::RunningWithSword => (run, true),

            // ── JUMP land transitions ──
            OT::TransitionJumpingUpWaitingCrouched => (JUMP_UP, true),
            OT::TransitionJumpingLongWaitingUpright
            | OT::TransitionJumpingLongSwordWaitingSword => (JUMP_LONG, true),
            OT::TransitionJumpingDownWaitingCrouched => (JUMP_DOWN, true),

            // ── SWORDFIGHT ──
            OT::StrikingRightSmalltalk
            | OT::StrikingLeftSmalltalk
            | OT::ParryingRightSmalltalk
            | OT::ParryingLeftSmalltalk
            | OT::StrikingLowRightSmalltalk
            | OT::StrikingLowLeftSmalltalk
            | OT::ParryingLowRightSmalltalk
            | OT::ParryingLowLeftSmalltalk
            | OT::StrikingStraightSword
            | OT::StrikingStraightStrongSword
            | OT::StrikingRightSword
            | OT::StrikingLeftSword
            | OT::StrikingRoundRightSword
            | OT::StrikingRoundLeftSword
            | OT::StrikingSemiroundRightSword
            | OT::StrikingSemiroundLeftSword
            | OT::ExecutingSword
            | OT::TransitionWaitingSwordParryingSword
            | OT::ParryingSword
            | OT::TransitionParryingSwordWaitingSword
            | OT::ParryingLowSword
            | OT::Provoking
            | OT::StrikingDownSword => (SWORDFIGHT, true),

            // ── DROP (material-dependent) ──
            OT::Rolling | OT::TransitionCarryingCorpseWaitingUpright => (drop, true),

            // Everything else (injuries, death, bow injuries, menacing,
            // beggar, climbing shoulders, drinking, etc.) — silent.
            _ => (0, true),
        }
    }

    /// Refresh the produced-noise state at one PC's live human-Hourglass
    /// boundary. Original `RefreshProducedNoise` follows the base Actor slice,
    /// so only NPC slots after this PC may observe the new volume this frame.
    pub(in crate::engine) fn refresh_pc_produced_noise_for(&mut self, pc_id: EntityId) {
        let order_type = self
            .orders
            .sequence_manager
            .current_order_for_actor(pc_id)
            .map(|(_, _, order)| order.order_type)
            .unwrap_or(crate::order::OrderType::Invalid);
        self.refresh_pc_produced_noise_for_with_order(pc_id, order_type);
    }

    /// Refresh produced noise using the `mpOrder` animation visible to the
    /// Original Human::Hourglass tail.
    ///
    /// Actor completion may already have instructed a different sequence
    /// element by this boundary. In that case the sequence manager's current
    /// order is newer than Original's latched `mpOrder`; the fused owner walk
    /// supplies the correctly stale animation explicitly.
    pub(in crate::engine) fn refresh_pc_produced_noise_for_with_order(
        &mut self,
        pc_id: EntityId,
        order_type: crate::order::OrderType,
    ) {
        let (material, in_building, active, previous, noise) = {
            let entity = self.world.entities.get(pc_id).unwrap_or_else(|| {
                panic!(
                    "PC produced-noise owner {} disappeared from its legacy slot",
                    pc_id.index()
                )
            });
            let Entity::Pc(pc) = entity else {
                panic!("produced-noise owner {} is not a PC actor", pc_id.index());
            };
            let position = pc.element.position_map();
            let noise = crate::ai::Noise {
                origin: crate::ai::Position {
                    x: position.x,
                    y: position.y,
                    sector: pc.element.sector(),
                    level: pc.element.layer(),
                },
                noise_type: if pc.human.opponents.is_empty() {
                    crate::ai::NoiseType::TapTapTap
                } else {
                    crate::ai::NoiseType::ZingZing
                },
                volume: 0,
                elevation: pc.element.sprite.position_iface.get_elevation() as u16,
                element_id: u16::try_from(pc_id.index()).unwrap_or_else(|_| {
                    panic!(
                        "PC produced-noise owner {} exceeds noise element-id range",
                        pc_id.index()
                    )
                }),
            };
            (
                pc.element.sprite.position_iface.get_material(),
                self.entity_building_sector(pc.element.sector()).is_some(),
                pc.element.active,
                pc.actor.last_noise_volume,
                noise,
            )
        };
        let (volume, refresh_hear_box) =
            Self::pc_noise_volume(order_type, material, in_building, active, previous);
        let Entity::Pc(pc) = self.world.entities.get_mut(pc_id).unwrap_or_else(|| {
            panic!(
                "PC produced-noise owner {} disappeared before write-back",
                pc_id.index()
            )
        }) else {
            panic!(
                "produced-noise owner {} changed kind before write-back",
                pc_id.index()
            );
        };
        pc.actor.last_noise_volume = volume;
        if refresh_hear_box {
            let half_x = volume as f32 + 100.0;
            let half_y = volume as f32 * crate::position_interface::ASPECT_RATIO + 100.0;
            pc.actor.hear_noise_box = crate::coordinates::MapBBox::from_coords(
                noise.origin.x - half_x,
                noise.origin.y - half_y,
                noise.origin.x + half_x,
                noise.origin.y + half_y,
            );
        }
        pc.actor.produced_noise = Some(crate::ai::Noise { volume, ..noise });
    }

    /// Rebuild every PC's non-serialized produced-noise fields after Original
    /// save pointer fixup.
    ///
    /// `RHEngine::Serialize` walks every human here, but
    /// `RHElementActorHuman::RefreshProducedNoise` immediately returns for
    /// NPCs. The remaining PC walk must use stable Original creation order.
    pub(crate) fn refresh_legacy_loaded_produced_noise(&mut self) {
        let pc_ids = self
            .world
            .entities
            .occupied()
            .filter_map(|(id, entity)| entity.is_pc().then_some(id))
            .collect::<Vec<_>>();
        for pc_id in pc_ids {
            self.refresh_pc_produced_noise_for(pc_id);
        }
    }

    /// Complete remarks which were active in an Original save.
    ///
    /// Original clears the remark latch and invokes
    /// `InformAIOnFinishedRemark` inline during local-AI deserialization.
    /// Ordinary state adoption has already installed the cleared latch; this
    /// method reproduces only the synchronous MYTALK callback, in serialized
    /// element order, before the later global RNG reseed.
    pub(crate) fn complete_legacy_loaded_remarks(
        &mut self,
        completions: &[(EntityId, u16)],
        assets: &LevelAssets,
    ) {
        self.with_simulation_context(|engine, sim| {
            for &(owner, raw_flags) in completions {
                let (current_remark, current_flags) = engine
                    .world
                    .entities
                    .get(owner)
                    .unwrap_or_else(|| {
                        panic!(
                            "preflighted loaded-remark owner {} disappeared",
                            owner.index()
                        )
                    })
                    .ai_controller()
                    .map(|ai| (ai.current_remark, ai.current_remark_flags))
                    .unwrap_or_else(|| {
                        panic!(
                            "preflighted loaded-remark owner {} lost its AI",
                            owner.index()
                        )
                    });
                assert_eq!(
                    current_remark,
                    crate::ai::Remark::TheSoundOfSilence,
                    "loaded-remark owner {} must be cleared before its callback",
                    owner.index()
                );
                assert_eq!(
                    current_flags,
                    0,
                    "loaded-remark owner {} flags must be cleared before its callback",
                    owner.index()
                );

                let Some(stimulus_type) = Self::speech_finished_stimulus(
                    crate::ai::SpeechFlags::from_bits_truncate(raw_flags),
                ) else {
                    continue;
                };
                let scratch = engine.build_sim_scratch(sim, assets);
                let in_uninterruptible_command = engine.is_very_very_busy(owner);
                let entity =
                    engine.world.entities.get(owner).unwrap_or_else(|| {
                        panic!("loaded-remark owner {} disappeared", owner.index())
                    });
                let building_sector = engine.entity_building_sector(entity.element_data().sector());
                let mut ctx = build_ai_context_from_entity(
                    entity,
                    engine.control.frame_counter,
                    building_sector,
                    engine.world.weather.is_forest_level,
                    engine.world.weather.ambiance,
                    engine.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &engine.world.fast_grid,
                    &assets.hiking_paths,
                    &assets.hiking_waypoint_sectors,
                    &engine.ai.global.all_soldier_handles,
                    engine.control.sim_config.difficulty,
                );
                ctx.in_uninterruptible_command = in_uninterruptible_command;
                let tick_data = engine.build_npc_tick_data(sim, owner, &scratch, assets);
                let stimulus = crate::ai::Stimulus::new(stimulus_type);
                engine.dispatch_think_with_drain(sim, owner, &stimulus, &ctx, &tick_data, assets);
            }
        });
    }

    /// The wide "indoors" test: the building-sector flag OR the
    /// door-transit branch — true during the few frames an actor is on a
    /// door whose inside-sector is a building but whose current sector
    /// pointer has not yet been swapped.
    ///
    /// This is the predicate that governs whether the view polygon is
    /// deactivated, so only the detection refresh may use it. Every other
    /// indoor test in the AI resolves the sector alone; use
    /// [`Self::entity_data_in_building_sector`] there.
    pub(in crate::engine) fn entity_data_inside_building(
        &self,
        elem: &crate::element::ElementData,
    ) -> bool {
        self.entity_data_in_building_sector(elem) || elem.is_in_door_transit()
    }

    /// The narrow "indoors" test: the entity's current sector is a
    /// building. An actor still on a door rail is outdoors by this
    /// measure, which is what the 180°/360° detection short-circuits, the
    /// them/us-list membership scans, the outdoor question gate, and the
    /// stuck-on-ladder counter all ask for.
    pub(in crate::engine) fn entity_data_in_building_sector(
        &self,
        elem: &crate::element::ElementData,
    ) -> bool {
        self.entity_building_sector(elem.sector()).is_some()
    }

    /// Consume one NPC's deferred `inform_my_friends` edge at that NPC's
    /// creation-order Hourglass boundary.
    ///
    /// Original `RHElementActorNPC::Hourglass` clears the flag and calls
    /// `MyDearFriendsPleasePleaseDetectMe` immediately before that same NPC's
    /// `RefreshView` / `RefreshDetection` (`RHelementactornpc.cpp:3534-3546`).
    pub(in crate::engine) fn tick_inform_my_friends_for_npc(&mut self, npc_id: EntityId) {
        if self.actors_frozen() {
            return;
        }

        let should_broadcast = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::npc_data_mut)
            .is_some_and(|npc| {
                let pending = npc.inform_my_friends;
                npc.inform_my_friends = false;
                pending
            });
        if should_broadcast {
            self.broadcast_body_detectable(npc_id);
        }
    }

    /// Dispatch this NPC's natural-wakeup `EVENT_FITAGAIN` synchronously at
    /// its base-human → NPC Hourglass boundary.
    ///
    /// `tick_concussion_healing` runs the globally batched stand-in for
    /// `RHElementActorHuman::Hourglass` and queues the event. The original
    /// calls `Think(EVENT_FITAGAIN)` inline before `mbInformMyFriends`,
    /// `RefreshView`, and `RefreshDetection` (human.cpp:335-390;
    /// npc.cpp:3528-3544). Drain the existing FIFO prefix through that wake
    /// event here; never pluck it ahead of older stimuli. The suffix remains
    /// queued for `RefreshDetection`'s ordinary drain.
    pub(in crate::engine) fn dispatch_pending_fit_again_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) -> bool {
        let (prefix_through_wake, mut suffix) = {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return false;
            };
            let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                panic!(
                    "NPC {} is missing its required AI controller while dispatching wakeup",
                    npc_id.index()
                )
            });
            let mut queued = std::mem::take(&mut ai.outbox.detection.stimuli);
            let Some(wake_index) = queued
                .iter()
                .position(|stimulus| stimulus.stimulus_type == StimulusType::EventFitAgain)
            else {
                ai.outbox.detection.stimuli = queued;
                return false;
            };
            let suffix = queued.split_off(wake_index + 1);
            assert!(
                !suffix
                    .iter()
                    .any(|stimulus| stimulus.stimulus_type == StimulusType::EventFitAgain),
                "NPC {} queued more than one EVENT_FITAGAIN before its Hourglass slot",
                npc_id.index()
            );
            (queued, suffix)
        };

        {
            let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                panic!(
                    "NPC {} disappeared before its wakeup stimulus prefix",
                    npc_id.index()
                )
            });
            let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                panic!(
                    "NPC {} lost its AI controller before its wakeup stimulus prefix",
                    npc_id.index()
                )
            });
            ai.outbox.detection.stimuli = prefix_through_wake;
        }
        self.tick_enemy_ai_drain_pending_stimuli_for_npc(sim, npc_id, assets, None, None);

        let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
            panic!(
                "NPC {} disappeared after synchronous EVENT_FITAGAIN",
                npc_id.index()
            )
        });
        let ai = entity.ai_controller_mut().unwrap_or_else(|| {
            panic!(
                "NPC {} lost its AI controller after synchronous EVENT_FITAGAIN",
                npc_id.index()
            )
        });
        suffix.append(&mut ai.outbox.detection.stimuli);
        ai.outbox.detection.stimuli = suffix;
        true
    }

    /// Iterates every NPC except the body itself and registers the
    /// body under DETECTABLE_BODY.
    #[tracing::instrument(level = "trace", skip_all, fields(body = body_id.index()))]
    pub(in crate::engine) fn broadcast_body_detectable(&mut self, body_id: EntityId) {
        use crate::element::DetectableType;

        let mutation_debug_enabled = detection::detectable_mutation_debug_enabled();
        let mutation_target_creation_order = (mutation_debug_enabled
            && detection::detectable_mutation_debug_target_slot_matches(body_id.index()))
        .then(|| self.original_static_creation_order(body_id))
        .unwrap_or(0);

        // Snapshot the body's position + `knocked_out_in_money_fight`
        // flag for the per-friend radius check below.
        let (body_pos, body_knocked_out_in_money_fight, body_is_soldier) = {
            let entity = self.expect_entity(body_id, "broadcast_body_detectable body");
            let is_soldier = matches!(entity, Entity::Soldier(_));
            let pos = entity.element_data().position_map();
            let ko = entity
                .npc_data()
                .and_then(|n| n.ai_brain.base())
                .map(|b| b.knocked_out_in_money_fight)
                .unwrap_or(false);
            (pos, ko, is_soldier)
        };

        // Append to every other NPC's Body detectable list (skip duplicates).
        // The NPC list holds both soldiers and civilians, so civilian
        // NPCs must receive the body detectable too — otherwise
        // `get_worst_detected_type` never climbs past DETECTABLE_FRIEND
        // for civilians, dropping their emoticon / alert reactions to
        // nearby bodies.
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        let det_idx = DetectableType::Body as usize;
        for friend_id in npc_ids {
            if friend_id == body_id {
                continue;
            }
            let mutation_owner_creation_order = (mutation_debug_enabled
                && detection::detectable_mutation_debug_owner_slot_matches(friend_id.index()))
            .then(|| self.original_static_creation_order(friend_id))
            .unwrap_or(0);
            let Some(entity) = self.world.entities.get_mut(friend_id) else {
                continue;
            };
            let friend_pos = entity.element_data().position_map();
            let Some(npc) = entity.npc_data_mut() else {
                continue;
            };

            // If this body was knocked out during a money fight, only
            // register the body with friends beyond
            // `AI_DOLLAR_FIGHT_IGNORE_BODY_RADIUS` (Chebyshev
            // distance).  Close-by money-fight participants
            // deliberately ignore the downed fighter.
            let add_detectable = if body_knocked_out_in_money_fight {
                let dx = (body_pos.x - friend_pos.x).abs();
                let dy = (body_pos.y - friend_pos.y).abs();
                dx.max(dy) > crate::parameters_ai::AI_DOLLAR_FIGHT_IGNORE_BODY_RADIUS as f32
            } else {
                true
            };

            if add_detectable && det_idx < npc.detectable_lists.len() {
                let already = npc.detectable_lists[det_idx]
                    .iter()
                    .any(|d| d.element == Some(body_id));
                if !already {
                    let mutation_length_before =
                        (detection::detectable_mutation_debug_owner_matches(
                            friend_id.index(),
                            mutation_owner_creation_order,
                        ) && detection::detectable_mutation_debug_target_matches(
                            body_id.index(),
                            mutation_target_creation_order,
                        ))
                        .then(|| npc.detectable_lists[det_idx].len());
                    npc.detectable_lists[det_idx].push(crate::element::Detectable {
                        element: Some(body_id),
                        detectable_type: DetectableType::Body,
                        ..Default::default()
                    });
                    if let Some(length_before) = mutation_length_before {
                        detection::debug_detectable_mutation_event(
                            "add",
                            "broadcast_body_detectable",
                            self.control.frame_counter,
                            friend_id.index(),
                            mutation_owner_creation_order,
                            det_idx,
                            body_id.index(),
                            mutation_target_creation_order,
                            false,
                            true,
                            length_before,
                            npc.detectable_lists[det_idx].len(),
                        );
                    }
                }
            }

            // Also remove the body from the friend's
            // money-fight-enemies list when both are soldiers.  Runs
            // unconditionally of the radius check.  Civilians have no
            // `EnemyAi`, so `enemy_mut()` is None and this arm is a
            // natural no-op for them — only soldiers track money-fight
            // enemies.
            if body_is_soldier && let Some(enemy_ai) = npc.ai_brain.enemy_mut() {
                enemy_ai
                    .money_fight_enemies
                    .retain(|h| *h != body_id.index());
            }
        }
    }

    /// Remove `beggar_id` from every NPC's `DETECTABLE_BEGGAR` list.
    /// Once any seek-area soldier has claimed the PC-beggar (queued it into
    /// `beggars_to_control`), this sweeps the beggar out of every
    /// soldier's and civilian's BEGGAR list so no other soldier fires
    /// a duplicate `EVENT_SEES_BEGGAR` on subsequent frames.
    ///
    /// Modelled on `engine/nets.rs:delete_body_detectable_for_all_npc`
    /// but hardcoded to `DetectableType::Beggar`.
    #[tracing::instrument(level = "trace", skip_all, fields(beggar = beggar_id.index()))]
    pub(in crate::engine) fn delete_beggar_detectable_for_all_npc(&mut self, beggar_id: EntityId) {
        use crate::element::DetectableType;
        let det_idx = DetectableType::Beggar as usize;
        let mutation_debug_enabled = detection::detectable_mutation_debug_enabled();
        let mutation_target_creation_order = (mutation_debug_enabled
            && detection::detectable_mutation_debug_target_slot_matches(beggar_id.index()))
        .then(|| self.original_static_creation_order(beggar_id))
        .unwrap_or(0);
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for friend_id in npc_ids {
            let mutation_owner_creation_order = (mutation_debug_enabled
                && detection::detectable_mutation_debug_owner_slot_matches(friend_id.index()))
            .then(|| self.original_static_creation_order(friend_id))
            .unwrap_or(0);
            let Some(entity) = self.world.entities.get_mut(friend_id) else {
                continue;
            };
            let Some(npc) = entity.npc_data_mut() else {
                continue;
            };
            if det_idx < npc.detectable_lists.len() {
                let mutation_before = (detection::detectable_mutation_debug_owner_matches(
                    friend_id.index(),
                    mutation_owner_creation_order,
                ) && detection::detectable_mutation_debug_target_matches(
                    beggar_id.index(),
                    mutation_target_creation_order,
                ))
                .then(|| {
                    (
                        npc.detectable_lists[det_idx].len(),
                        npc.detectable_lists[det_idx]
                            .iter()
                            .any(|detectable| detectable.element == Some(beggar_id)),
                    )
                });
                npc.delete_detectable(beggar_id, DetectableType::Beggar);
                if let Some((before, present_before)) = mutation_before {
                    let present_after = npc.detectable_lists[det_idx]
                        .iter()
                        .any(|detectable| detectable.element == Some(beggar_id));
                    detection::debug_detectable_mutation_event(
                        "delete",
                        "delete_beggar_detectable_for_all_npc",
                        self.control.frame_counter,
                        friend_id.index(),
                        mutation_owner_creation_order,
                        det_idx,
                        beggar_id.index(),
                        mutation_target_creation_order,
                        present_before,
                        present_after,
                        before,
                        npc.detectable_lists[det_idx].len(),
                    );
                }
            }
        }
    }

    /// Original `RestoreDetectableObjects`, executed inline by the waking
    /// soldier before resurrection fan-out and any SetState callback.
    pub(in crate::engine) fn restore_detectable_objects_for_npc(
        &mut self,
        npc_id: EntityId,
        knocked_out_in_money_fight: bool,
    ) {
        use crate::element::DetectableType;
        use crate::element_kinds::ObjectType;

        let mut to_add = Vec::new();
        for (entity_id, entity) in self.world.entities.objects() {
            if !entity.is_active() {
                continue;
            }
            let object = entity.object_data().unwrap_or_else(|| {
                panic!(
                    "object slot {} lost object data during recovery for NPC {}",
                    entity_id.index(),
                    npc_id.index()
                )
            });
            if matches!(object.object_type, ObjectType::Ale)
                || matches!(object.object_type, ObjectType::Coin) && !knocked_out_in_money_fight
            {
                to_add.push(EntityId::from(entity_id));
            }
        }

        let npc = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::npc_data_mut)
            .unwrap_or_else(|| {
                panic!(
                    "recovery owner {} vanished before RestoreDetectableObjects",
                    npc_id.index()
                )
            });
        let objects = npc
            .detectable_lists
            .get_mut(DetectableType::Object as usize)
            .unwrap_or_else(|| {
                panic!(
                    "recovery owner {} has no DETECTABLE_OBJECT list",
                    npc_id.index()
                )
            });
        for element in to_add {
            if !objects
                .iter()
                .any(|detectable| detectable.element == Some(element))
            {
                objects.push(crate::element::Detectable {
                    element: Some(element),
                    detectable_type: DetectableType::Object,
                    ..Default::default()
                });
            }
        }
    }

    /// Apply the resurrection fan-out and eye-status writes produced by
    /// `EVENT_FITAGAIN`. The caller invokes this immediately after the
    /// synchronous Think drain, before returning to Human/Actor Hourglass.
    pub(in crate::engine) fn tick_ai_pending_resurrection_and_eyes_for_npc(
        &mut self,
        npc_id: EntityId,
    ) {
        let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
            panic!(
                "NPC {} disappeared while applying synchronous recovery state",
                npc_id.index()
            )
        });
        let ai = entity.ai_controller_mut().unwrap_or_else(|| {
            panic!(
                "NPC {} is missing its required AI controller while applying recovery state",
                npc_id.index()
            )
        });
        let inform_resurrection = ai.outbox.recovery.inform_resurrection;
        ai.outbox.recovery.inform_resurrection = false;
        let eye_status = ai.outbox.recovery.set_eye_status.take();

        if inform_resurrection {
            self.broadcast_resurrection(npc_id);
        }
        if let Some(status) = eye_status {
            let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                panic!(
                    "NPC {} disappeared while applying its pending eye status",
                    npc_id.index()
                )
            });
            let npc = entity.npc_data_mut().unwrap_or_else(|| {
                panic!(
                    "entity {} lost its NPC data while applying its pending eye status",
                    npc_id.index()
                )
            });
            crate::ai_vision::set_view_status(npc, status);
        }
    }

    /// Remove `resurrected_id` from every other NPC's
    /// `DETECTABLE_BODY` list.  The per-NPC body of
    /// `inform_on_resurrection` — the engine-side fan-out triggered by
    /// `inform_everyone_on_my_resurrection`.
    #[tracing::instrument(level = "trace", skip_all, fields(resurrected = resurrected_id.index()))]
    pub(in crate::engine) fn broadcast_resurrection(&mut self, resurrected_id: EntityId) {
        use crate::element::DetectableType;
        let det_idx = DetectableType::Body as usize;
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for friend_id in npc_ids {
            if friend_id == resurrected_id {
                continue;
            }
            let Some(entity) = self.world.entities.get_mut(friend_id) else {
                continue;
            };
            let Some(npc) = entity.npc_data_mut() else {
                continue;
            };
            if det_idx < npc.detectable_lists.len() {
                npc.delete_detectable(resurrected_id, DetectableType::Body);
            }
        }
    }

    /// Per-frame view parameter refresh for every NPC.
    ///
    /// This test-facing wrapper preserves the focused EYES_FOLLOW oracle;
    /// production coordinates the extracted per-NPC helper directly with
    /// that NPC's `RefreshDetection` slot.
    #[cfg(test)]
    pub(in crate::engine) fn refresh_npc_views(
        &mut self,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
    ) {
        if self.actors_frozen() {
            return;
        }

        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for npc_id in npc_ids {
            self.refresh_npc_view_for_npc(npc_id, positions_before_movement);
        }
    }

    /// Refresh one NPC's view immediately before its own creation-ordered
    /// `RefreshDetection` call.
    pub(in crate::engine) fn refresh_npc_view_for_npc(
        &mut self,
        npc_id: EntityId,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
    ) {
        if self.actors_frozen() {
            return;
        }

        // ── Phase 1: read-only — gather context ──
        let ctx = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            let Some(npc) = entity.npc_data() else {
                return;
            };

            let edata = entity.element_data();
            let own_world = entity.position_iface().get_position();
            let pos = crate::coordinates::GroundPoint::new(own_world.x, own_world.y);

            // RHElement::IsActiveAndOutsideBuilding is deliberately narrower
            // than IsInsideBuilding: it only inspects the current sector's
            // BUILDING flag. A sprite door pointer makes IsInsideBuilding
            // true while traversing an outdoor approach rail, but RefreshView
            // must still follow the actor's turning body on that rail.
            let is_active_and_outside_building =
                edata.active && self.entity_building_sector(edata.sector()).is_none();

            let animation = self
                .orders
                .sequence_manager
                .current_order_for_actor(npc_id)
                .map(|(_, _, o)| o.order_type);

            let is_unconscious = entity.human_data().map(|h| h.unconscious).unwrap_or(false);

            let follow_target_position = npc.follow_target.and_then(|target_id| {
                self.world.entities.get(target_id).map(|target| {
                    // Original provenance:
                    // - RHEngine::PerformHourglass walks marrayElements in
                    //   creation order (RHengine.cpp:3715-3724,7909-7944).
                    // - RHElementActorNPC::Hourglass delegates to the base
                    //   human Hourglass before RefreshView
                    //   (RHelementactornpc.cpp:3528-3544).
                    // - EYES_FOLLOW reads pMobileTarget->GetPositionGround
                    //   inside RefreshView (RHelementactornpc.cpp:1012-1018).
                    // Thus a later-created target has not moved yet, while
                    // an earlier-created target has. EntityId::index is the
                    // append-only legacy creation slot in this port.
                    let boundary = if target_id.index() > npc_id.index() {
                        positions_before_movement
                            .get(target_id)
                            .copied()
                            .flatten()
                            .unwrap_or_else(|| {
                                panic!(
                                    "NPC {npc_id:?} follows later-created target {target_id:?}, \
                                         but the required pre-movement position snapshot is missing"
                                )
                            })
                    } else {
                        crate::entities::BoundaryPosition::of(target.element_data())
                    };
                    // The recorded boundary carries the target's own 3D
                    // position, so jump/flying Z survives and nothing has to
                    // be re-derived from the plane.
                    crate::coordinates::GroundPoint::new(boundary.world.x, boundary.world.y)
                })
            });

            let blood_alcohol = entity
                .enemy_ai()
                .map(|enemy| enemy.base.blood_alcohol)
                .unwrap_or(0);

            ai_vision::RefreshViewContext {
                body_direction: edata.direction(),
                posture: edata.posture,
                animation,
                is_unconscious,
                is_tied: edata.posture == crate::element::Posture::Tied,
                is_dead: entity.is_dead(),
                is_active_and_outside_building,
                is_rider: matches!(entity, Entity::Soldier(s) if s.soldier.rider),
                blood_alcohol,
                own_position: pos,
                follow_target_position,
            }
        };
        // shared borrow dropped ──

        // ── Phase 2: mutable — apply RefreshView ──
        self.debug_refresh_view_lifecycle("refresh_view_before", npc_id, None);
        {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };
            if let Some(npc) = entity.npc_data_mut() {
                let span = tracing::trace_span!("refresh_npc_view", npc = npc_id.index());
                let _guard = span.enter();
                ai_vision::refresh_view(npc, &ctx);
            }
        }
        self.debug_refresh_view_lifecycle("refresh_view_after", npc_id, None);
    }

    // ─── Owner-local NPC speech ─────────────────────────────────

    pub(in crate::engine) fn debug_speech_lifecycle(
        &self,
        actor_id: u32,
        phase: &str,
        detail: impl std::fmt::Debug,
    ) {
        let config = speech_lifecycle_debug_config();
        if !config.enabled {
            return;
        }
        let frame = self.control.frame_counter;
        if !config.frame.is_none_or(|expected| expected == frame)
            || !config.actor.is_none_or(|expected| expected == actor_id)
        {
            return;
        }
        let sound = &self.feedback.sound_sim;
        let pending = sound
            .pending_exclamations
            .iter()
            .filter(|item| item.actor_id == actor_id)
            .map(|item| (item.exclamation_id, item.profile_id, item.variant))
            .collect::<Vec<_>>();
        let playing = sound
            .playing_exclamations
            .iter()
            .filter(|item| item.actor_id == actor_id)
            .map(|item| (item.exclamation_id, item.finish_frame))
            .collect::<Vec<_>>();
        let resolved = sound
            .resolved_exclamations
            .iter()
            .filter(|item| item.actor_id == actor_id)
            .map(|item| (item.exclamation_id, item.identifier, item.duration_frames))
            .collect::<Vec<_>>();
        eprintln!(
            "SPEECHLIFE frame={frame} actor={actor_id} phase={phase} detail={detail:?} pending={pending:?} playing={playing:?} resolved={resolved:?}"
        );
    }

    fn debug_speech_attempt_gate_snapshot(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
        attempt: crate::ai::AiSpeechAttempt,
    ) {
        use crate::ai::RemarkTargetFlags;

        let config = speech_lifecycle_debug_config();
        if !config.enabled {
            return;
        }
        let frame = self.control.frame_counter;
        if !config.frame.is_none_or(|expected| expected == frame)
            || !config
                .actor
                .is_none_or(|expected| expected == owner.index())
        {
            return;
        }

        let entity = self
            .world
            .entities
            .get(owner)
            .unwrap_or_else(|| panic!("speech gate diagnostic lost owner {owner:?}"));
        let ai = entity
            .ai_controller()
            .unwrap_or_else(|| panic!("speech gate diagnostic owner {owner:?} lost AI"));
        let (is_soldier, speech_id) = match entity {
            Entity::Soldier(soldier) => {
                let profile = assets
                    .profile_manager
                    .get_soldier(soldier.soldier.soldier_profile_index)
                    .unwrap_or_else(|| {
                        panic!("speech gate diagnostic owner {owner:?} lost soldier profile")
                    });
                (true, profile.exclamation_id)
            }
            Entity::Civilian(civilian) => {
                let profile = assets
                    .profile_manager
                    .civilians
                    .get(usize::from(civilian.civilian.civilian_profile_index))
                    .unwrap_or_else(|| {
                        panic!("speech gate diagnostic owner {owner:?} lost civilian profile")
                    });
                (false, profile.exclamation_id)
            }
            other => panic!(
                "speech gate diagnostic owner {owner:?} has invalid kind {:?}",
                other.element_data().kind
            ),
        };
        let creation_order = self.world.original_creation_order(owner);
        let matching_forbidden = self
            .ai
            .global
            .forbidden_remarks
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.remark == attempt.remark)
            .map(|(index, entry)| {
                let scope = RemarkTargetFlags::from_bits_truncate(entry.flags);
                let applies = (scope.contains(RemarkTargetFlags::THIS_TYPE)
                    && entry.bad_guy == is_soldier
                    && entry.speech_id == speech_id)
                    || (scope.contains(RemarkTargetFlags::THIS_GUY)
                        && u32::from(entry.guy_index) == creation_order)
                    || (is_soldier && scope.contains(RemarkTargetFlags::VILLAINS))
                    || (!is_soldier && scope.contains(RemarkTargetFlags::CIVILIANS));
                (
                    index,
                    entry.flags,
                    entry.speech_id,
                    entry.guy_index,
                    entry.bad_guy,
                    entry.forbidden_till_frame,
                    entry.forbidden_till_frame < frame,
                    applies,
                )
            })
            .collect::<Vec<_>>();
        let element = entity.element_data();
        eprintln!(
            "SPEECHLIFE frame={frame} actor={} phase=attempt_gate_snapshot remark={:?} flags={} state={:?} substate={:?} locks={:?} current_remark={:?} current_remark_flags={} blipped={} sector={:?} in_door_transit={} posture={:?} action={:?} animation={:?} creation_order={creation_order} is_soldier={is_soldier} speech_id={speech_id} script_forbidden={} matching_forbidden={matching_forbidden:?}",
            owner.index(),
            attempt.remark,
            attempt.flags,
            ai.current_state,
            ai.current_substate,
            ai.locks_flag_field,
            ai.current_remark,
            ai.current_remark_flags,
            element.blipped,
            element.sector(),
            element.is_in_door_transit(),
            element.posture,
            entity.actor_data().map(|actor| actor.action_state),
            element.sprite.last_action,
            ai.forbidden_remark_ids.contains(&(attempt.remark as u32)),
        );
    }

    fn speech_finished_stimulus(flags: crate::ai::SpeechFlags) -> Option<StimulusType> {
        use crate::ai::SpeechFlags;
        if flags.contains(SpeechFlags::MYTALK_1) {
            Some(StimulusType::EventMyTalk1)
        } else if flags.contains(SpeechFlags::MYTALK_2) {
            Some(StimulusType::EventMyTalk2)
        } else if flags.contains(SpeechFlags::MYTALK_3) {
            Some(StimulusType::EventMyTalk3)
        } else if flags.contains(SpeechFlags::MYTALK_0) {
            Some(StimulusType::EventMyTalk0)
        } else {
            None
        }
    }

    fn reject_npc_speech_attempt(
        &mut self,
        owner: EntityId,
        flags: crate::ai::SpeechFlags,
        reason: u16,
    ) -> NpcSpeechSettlement {
        let ai = self
            .world
            .entities
            .get_mut(owner)
            .unwrap_or_else(|| {
                panic!(
                    "speech owner {} disappeared during rejection",
                    owner.index()
                )
            })
            .ai_controller_mut()
            .unwrap_or_else(|| {
                panic!(
                    "speech owner {} lost its AI during rejection",
                    owner.index()
                )
            });
        ai.cached_frame = self.control.frame_counter;
        ai.register_log_line(crate::ai::LogLineType::SpeakImpossible, reason);
        tracing::trace!(
            target: "robin_engine::engine::speech",
            frame = self.control.frame_counter,
            owner = owner.index(),
            reason,
            "Say rejected"
        );
        let invoke_finished_callback = if let Some(stimulus) = Self::speech_finished_stimulus(flags)
        {
            ai.outbox.reentrant.self_stimuli.insert(0, stimulus.into());
            true
        } else {
            false
        };
        self.debug_speech_lifecycle(
            owner.index(),
            "attempt_rejected",
            (reason, flags.bits(), invoke_finished_callback),
        );
        NpcSpeechSettlement {
            invoke_finished_callback,
            category_rejection: None,
        }
    }

    /// Settle one queued Say invocation at the current AI owner's return
    /// barrier.
    ///
    /// Ordering follows `RHArtificialIntelligence::Say`
    /// (`original-code/RHartificialintelligence.cpp:5846-6178`): blip,
    /// script forbid, recent-remark forbid, house, CYCLE_3 advance,
    /// active-speech arbitration, active remark assignment, speech-profile
    /// category dispatch, screen remark, then automatic forbidding.
    pub(in crate::engine) fn settle_npc_speech_attempt(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        attempt: crate::ai::AiSpeechAttempt,
    ) -> NpcSpeechSettlement {
        use crate::ai::{Remark, RemarkTargetFlags, SpeechFlags};
        use crate::sound::ExclamationGroup;

        let flags = SpeechFlags::from_bits_truncate(attempt.flags);
        self.debug_speech_lifecycle(
            owner.index(),
            "attempt_enter",
            (attempt.remark, attempt.flags),
        );
        self.debug_speech_attempt_gate_snapshot(assets, owner, attempt);
        #[derive(Clone, Copy)]
        enum OwnerProfile {
            Soldier(crate::profiles::SoldierProfileIdx),
            Civilian(crate::profiles::CivilianProfileIdx),
        }

        let (
            owner_profile,
            blipped,
            sector,
            in_door_transit,
            position,
            frame_profile_name,
            script_forbidden,
            active_remark,
        ) = {
            let entity = self
                .world
                .entities
                .get(owner)
                .unwrap_or_else(|| panic!("queued speech owner {} is missing", owner.index()));
            let owner_profile = match entity {
                Entity::Soldier(s) => OwnerProfile::Soldier(s.soldier.soldier_profile_index),
                Entity::Civilian(c) => OwnerProfile::Civilian(c.civilian.civilian_profile_index),
                other => panic!(
                    "queued NPC speech owner {} has invalid entity kind {:?}",
                    owner.index(),
                    other.element_data().kind
                ),
            };
            let ai = entity.ai_controller().unwrap_or_else(|| {
                panic!("queued speech owner {} has no AI controller", owner.index())
            });
            (
                owner_profile,
                entity.element_data().blipped,
                entity.element_data().sector(),
                entity.element_data().is_in_door_transit(),
                entity.element_data().position_map(),
                entity.element_data().sprite.frame_profile_name.clone(),
                ai.forbidden_remark_ids.contains(&(attempt.remark as u32)),
                ai.current_remark,
            )
        };
        let is_soldier = matches!(owner_profile, OwnerProfile::Soldier(_));
        let mut resolved_profile: Option<(bool, u32)> = None;
        let resolve_profile = |cached: &mut Option<(bool, u32)>| {
            if cached.is_none() {
                *cached = Some(match owner_profile {
                    OwnerProfile::Soldier(profile_index) => {
                        let profile = assets
                            .profile_manager
                            .get_soldier(profile_index)
                            .unwrap_or_else(|| {
                                panic!(
                                    "speech owner {} requires missing soldier profile {} after early gates",
                                    owner.index(),
                                    profile_index
                                )
                            });
                        (profile.vip, profile.exclamation_id)
                    }
                    OwnerProfile::Civilian(profile_index) => {
                        let profile = assets
                            .profile_manager
                            .civilians
                            .get(usize::from(profile_index))
                            .unwrap_or_else(|| {
                                panic!(
                                    "speech owner {} requires missing civilian profile {} after early gates",
                                    owner.index(),
                                    profile_index
                                )
                            });
                        (
                            profile.civilian_type == crate::profiles::CivilianType::Vip,
                            profile.exclamation_id,
                        )
                    }
                });
            }
            cached.clone().expect("speech profile cache was populated")
        };

        {
            let ai = self
                .world
                .entities
                .get_mut(owner)
                .unwrap_or_else(|| {
                    panic!(
                        "speech owner {} disappeared before Speak log",
                        owner.index()
                    )
                })
                .ai_controller_mut()
                .unwrap_or_else(|| {
                    panic!("speech owner {} lost AI before Speak log", owner.index())
                });
            ai.cached_frame = self.control.frame_counter;
            ai.register_log_line(crate::ai::LogLineType::Speak, attempt.remark as u16);
        }
        tracing::trace!(
            target: "robin_engine::engine::speech",
            frame = self.control.frame_counter,
            owner = owner.index(),
            remark = ?attempt.remark,
            flags = attempt.flags,
            "Say attempt"
        );

        if blipped {
            return self.reject_npc_speech_attempt(owner, flags, 0);
        }
        if script_forbidden {
            return self.reject_npc_speech_attempt(owner, flags, 1);
        }

        if !flags.contains(SpeechFlags::ALWAYS) {
            let frame = self.control.frame_counter;
            let owner_creation_order = self.world.original_creation_order(owner);
            // Original scans lazily in list order. It deletes expired entries
            // only as encountered and returns on the first live match, leaving
            // every later entry (including expired ones) untouched.
            let mut forbidden = false;
            let mut index = 0;
            while index < self.ai.global.forbidden_remarks.len() {
                if self.ai.global.forbidden_remarks[index].forbidden_till_frame < frame {
                    self.ai.global.forbidden_remarks.remove(index);
                    continue;
                }
                let entry = &self.ai.global.forbidden_remarks[index];
                if entry.remark == attempt.remark {
                    let scope = RemarkTargetFlags::from_bits_truncate(entry.flags);
                    if scope.contains(RemarkTargetFlags::THIS_TYPE)
                        && entry.bad_guy == is_soldier
                        && entry.speech_id == resolve_profile(&mut resolved_profile).1
                    {
                        forbidden = true;
                    } else if scope.contains(RemarkTargetFlags::THIS_GUY)
                        && u32::from(entry.guy_index) == owner_creation_order
                    {
                        forbidden = true;
                    } else if is_soldier && scope.contains(RemarkTargetFlags::VILLAINS) {
                        forbidden = true;
                    } else if !is_soldier && scope.contains(RemarkTargetFlags::CIVILIANS) {
                        forbidden = true;
                    }
                }
                if forbidden {
                    break;
                }
                index += 1;
            }
            if forbidden {
                return self.reject_npc_speech_attempt(owner, flags, 2);
            }
        }

        if !flags.contains(SpeechFlags::HOUSE)
            && (self.entity_building_sector(sector).is_some() || in_door_transit)
        {
            return self.reject_npc_speech_attempt(owner, flags, 3);
        }

        // This is deliberately before the already-speaking gate, exactly as
        // in Original Say. Rejected overlapping attempts still consume one
        // shared CYCLE_3 slot.
        let variant = if flags.contains(SpeechFlags::CYCLE_3_VARIANTS) {
            self.ai.global.current_speech_variant = (self.ai.global.current_speech_variant + 1) % 3;
            self.ai.global.current_speech_variant as i32
        } else {
            -1
        };

        if active_remark != Remark::TheSoundOfSilence {
            if flags.contains(SpeechFlags::EMERGENCY) {
                self.debug_speech_lifecycle(
                    owner.index(),
                    "emergency_cancel_before",
                    (attempt.remark, attempt.flags),
                );
                self.feedback
                    .pending_side_effects
                    .sounds
                    .push(super::SoundCommand::StopExclamation { actor_id: owner });
                // StopExclamation removes the old pending/playing line without
                // calling SoundIsFinished, so its MYTALK callback is discarded.
                self.cancel_exclamation_callbacks(owner.index());
                self.debug_speech_lifecycle(
                    owner.index(),
                    "emergency_cancel_after",
                    (attempt.remark, attempt.flags),
                );
            } else {
                return self.reject_npc_speech_attempt(owner, flags, 4);
            }
        }

        {
            let ai = self
                .world
                .entities
                .get_mut(owner)
                .unwrap_or_else(|| {
                    panic!("speech owner {} disappeared before latch", owner.index())
                })
                .ai_controller_mut()
                .unwrap_or_else(|| panic!("speech owner {} lost AI before latch", owner.index()));
            ai.current_remark = attempt.remark;
            ai.current_remark_flags = attempt.flags;
        }

        let (is_vip, speech_id) = resolve_profile(&mut resolved_profile);

        // Original skips the entire category/sound branch for speech ID zero,
        // but still leaves current_remark latched and performs the display and
        // auto-forbid tail. With no SoundIsFinished callback this can remain
        // active indefinitely.
        if speech_id != 0 {
            let raw = attempt.remark as u32;
            let first_vip = Remark::FIRST_VIP as u32;
            let first_civilian = Remark::FIRST_CIVILIAN as u32;
            let prefix = if flags.contains(SpeechFlags::SCRIPT) {
                "Script error"
            } else {
                "AI error"
            };
            let resolved = if raw >= first_vip {
                if !is_vip {
                    tracing::warn!(
                        target: "ai_speech_mismatch",
                        "{}: VIP remark [{}] for non-VIP NPC {} at ({},{})",
                        prefix,
                        attempt.remark.speech(),
                        owner.index(),
                        position.x as u16,
                        position.y as u16
                    );
                    None
                } else {
                    Some((ExclamationGroup::Vip, raw.wrapping_sub(first_vip) as u16))
                }
            } else if raw >= first_civilian {
                if is_soldier || is_vip {
                    if is_soldier {
                        tracing::warn!(
                            target: "ai_speech_mismatch",
                            "{}: civilian remark [{}] for soldier {} at ({},{})",
                            prefix,
                            attempt.remark.speech(),
                            owner.index(),
                            position.x as u16,
                            position.y as u16
                        );
                    }
                    None
                } else {
                    Some((
                        ExclamationGroup::Civilian,
                        raw.wrapping_sub(first_civilian) as u16,
                    ))
                }
            } else if !is_soldier || is_vip {
                if !is_soldier {
                    tracing::warn!(
                        target: "ai_speech_mismatch",
                        "{}: soldier remark [{}] for civilian {} at ({},{})",
                        prefix,
                        attempt.remark.speech(),
                        owner.index(),
                        position.x as u16,
                        position.y as u16
                    );
                }
                None
            } else {
                // Original's ordinary soldier bank uses EXCLAMATION_CIVILIAN.
                Some((ExclamationGroup::Civilian, raw as u16))
            };

            let Some((group, exclamation_id)) = resolved else {
                let reason = if raw >= first_vip {
                    if is_soldier { 5 } else { 6 }
                } else if raw >= first_civilian {
                    if is_soldier { 7 } else { 8 }
                } else if !is_soldier {
                    9
                } else {
                    10
                };
                let log_before_callback = !matches!(reason, 8 | 9);
                let ai = self
                    .world
                    .entities
                    .get_mut(owner)
                    .unwrap_or_else(|| {
                        panic!(
                            "speech owner {} disappeared after category rejection",
                            owner.index()
                        )
                    })
                    .ai_controller_mut()
                    .unwrap_or_else(|| {
                        panic!(
                            "speech owner {} lost AI after category rejection",
                            owner.index()
                        )
                    });
                if log_before_callback {
                    ai.register_log_line(crate::ai::LogLineType::SpeakImpossible, reason);
                }
                let invoke_finished_callback =
                    if let Some(stimulus) = Self::speech_finished_stimulus(flags) {
                        ai.outbox.reentrant.self_stimuli.insert(0, stimulus.into());
                        true
                    } else {
                        false
                    };
                self.debug_speech_lifecycle(
                    owner.index(),
                    "attempt_category_rejected",
                    (
                        reason,
                        attempt.remark,
                        attempt.flags,
                        invoke_finished_callback,
                    ),
                );
                return NpcSpeechSettlement {
                    invoke_finished_callback,
                    category_rejection: Some(CategorySpeechRejectionFinalization {
                        reason_after_callback: (!log_before_callback).then_some(reason),
                    }),
                };
            };

            self.feedback
                .pending_side_effects
                .sounds
                .push(super::SoundCommand::Exclamation {
                    group,
                    profile_id: speech_id,
                    exclamation_id,
                    variant,
                    position,
                    actor_id: Some(owner),
                });
            self.feedback
                .sound_sim
                .pending_exclamations
                .push(crate::sound::PendingExclamation {
                    actor_id: owner.index(),
                    group,
                    profile_id: speech_id,
                    exclamation_id,
                    variant,
                });
            self.debug_speech_lifecycle(
                owner.index(),
                "attempt_accepted",
                (
                    attempt.remark,
                    attempt.flags,
                    exclamation_id,
                    speech_id,
                    variant,
                ),
            );
        }

        self.ai.global.screen_remarks.push(crate::ai::ScreenRemark {
            timer: 100,
            prefix: frame_profile_name,
            remark: attempt.remark,
        });
        Self::auto_forbid_remark(
            &mut self.ai.global.forbidden_remarks,
            attempt.remark,
            speech_id,
            self.world.original_creation_order(owner) as u16,
            is_soldier,
            self.control.frame_counter,
        );
        NpcSpeechSettlement::default()
    }

    /// Finish the unconditional tail of a category-rejected Original `Say`.
    /// Reasons 8/9 log only after `InformAIOnFinishedRemark`; every category
    /// rejection clears the latch after that callback returns, overwriting any
    /// recursively started emergency line.
    pub(in crate::engine) fn finalize_category_speech_rejection(
        &mut self,
        owner: EntityId,
        finalization: CategorySpeechRejectionFinalization,
    ) {
        use crate::ai::Remark;

        let ai = self
            .world
            .entities
            .get_mut(owner)
            .unwrap_or_else(|| {
                panic!(
                    "speech owner {} disappeared during category-rejection tail",
                    owner.index()
                )
            })
            .ai_controller_mut()
            .unwrap_or_else(|| {
                panic!(
                    "speech owner {} lost AI during category-rejection tail",
                    owner.index()
                )
            });
        if let Some(reason) = finalization.reason_after_callback {
            ai.register_log_line(crate::ai::LogLineType::SpeakImpossible, reason);
        }
        ai.current_remark = Remark::TheSoundOfSilence;
        ai.current_remark_flags = 0;
    }

    /// Deliver deterministic SoundIsFinished callbacks at the first mutation
    /// of the `PerformHourglass` deferred-effects phase where matured
    /// exclamations are collected.
    ///
    /// `RHElementActorNPC::SoundIsFinished`
    /// (`original-code/RHelementactornpc.cpp:6473-6511`) converts the
    /// currently active remark through the owner's category and clears it only
    /// when the callback's exact exclamation ID matches. A stale/mismatched
    /// completion is logged and deliberately retains the active line.
    pub(in crate::engine) fn settle_npc_speech_completions(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        use crate::ai::{Remark, SpeechFlags};

        let completions = std::mem::take(&mut self.feedback.sound_sim.finished_exclamations);
        for (actor_slot, completed_id) in completions {
            let actor_id = self
                .world
                .entities
                .id_at_legacy_slot(actor_slot)
                .unwrap_or_else(|| {
                    panic!(
                        "speech completion references missing legacy actor slot {} (id {})",
                        actor_slot, completed_id
                    )
                });
            let (active, expected_id, flags, is_pc) = {
                let entity = self.world.entities.get(actor_id).unwrap_or_else(|| {
                    panic!(
                        "speech completion owner {} vanished after slot resolution",
                        actor_id.index()
                    )
                });
                if entity.is_pc() {
                    (Remark::TheSoundOfSilence, 0, 0, true)
                } else {
                    let ai = entity.ai_controller().unwrap_or_else(|| {
                        panic!(
                            "speech completion owner {} is neither PC nor an NPC with AI",
                            actor_id.index()
                        )
                    });
                    let active = ai.current_remark;
                    let raw = active as u32;
                    let expected = match entity {
                        Entity::Soldier(s) => {
                            let profile = assets
                                .profile_manager
                                .get_soldier(s.soldier.soldier_profile_index)
                                .unwrap_or_else(|| {
                                    panic!(
                                        "speech completion owner {} requires missing soldier profile {}",
                                        actor_id.index(),
                                        s.soldier.soldier_profile_index
                                    )
                                });
                            if profile.vip {
                                raw.wrapping_sub(Remark::FIRST_VIP as u32)
                            } else {
                                raw
                            }
                        }
                        Entity::Civilian(c) => {
                            let profile = assets
                                .profile_manager
                                .civilians
                                .get(usize::from(c.civilian.civilian_profile_index))
                                .unwrap_or_else(|| {
                                    panic!(
                                        "speech completion owner {} requires missing civilian profile {}",
                                        actor_id.index(),
                                        c.civilian.civilian_profile_index
                                    )
                                });
                            if profile.civilian_type == crate::profiles::CivilianType::Vip {
                                raw.wrapping_sub(Remark::FIRST_VIP as u32)
                            } else {
                                raw.wrapping_sub(Remark::FIRST_CIVILIAN as u32)
                            }
                        }
                        other => panic!(
                            "speech completion owner {} has invalid entity kind {:?}",
                            actor_id.index(),
                            other.element_data().kind
                        ),
                    };
                    (active, expected, ai.current_remark_flags, false)
                }
            };
            if is_pc {
                continue;
            }
            if active == Remark::TheSoundOfSilence || expected_id != completed_id {
                tracing::warn!(
                    actor = actor_id.index(),
                    ?active,
                    expected_id,
                    completed_id,
                    "stale or mismatched NPC speech completion retained active speech"
                );
                continue;
            }

            let ai = self
                .world
                .entities
                .get_mut(actor_id)
                .unwrap_or_else(|| {
                    panic!("speech completion owner {} disappeared", actor_id.index())
                })
                .ai_controller_mut()
                .unwrap_or_else(|| panic!("speech completion owner {} lost AI", actor_id.index()));
            ai.current_remark = Remark::TheSoundOfSilence;
            ai.current_remark_flags = 0;
            ai.register_log_line(crate::ai::LogLineType::SpeakFinished, 0);
            if let Some(stimulus) =
                Self::speech_finished_stimulus(SpeechFlags::from_bits_truncate(flags))
            {
                ai.outbox.reentrant.self_stimuli.push(stimulus.into());
            }
            self.drain_direct_ai_owner_boundary(sim, actor_id, assets);
        }
    }

    /// Per-tick decay + eviction of the screen-remark HUD overlay list.
    /// The timer half of `display_screen_remarks`: each entry's timer
    /// is decremented and entries whose timer reaches zero are
    /// dropped.  Without this the list grows unbounded for the
    /// lifetime of the mission (one entry per accepted remark).  The
    /// rendering half lives in `hud_text::render_screen_remarks`.
    pub(in crate::engine) fn tick_screen_remarks(&mut self) {
        self.ai.global.screen_remarks.retain_mut(|r| {
            r.timer = r.timer.saturating_sub(1);
            r.timer > 0
        });
    }

    /// Auto-forbid a remark after speaking, with per-remark duration and scope.
    fn auto_forbid_remark(
        forbidden_remarks: &mut Vec<crate::ai::ForbiddenRemark>,
        remark: crate::ai::Remark,
        speech_id: u32,
        guy_index: u16,
        is_soldier: bool,
        current_frame: u32,
    ) {
        use crate::ai::{ForbiddenRemark, Remark, RemarkTargetFlags};
        use crate::parameters_ai::{
            AI_DRUNKEN_REMARK_FORBIDDEN_TIME, AI_REMARK_FORBIDDEN_TIME,
            AI_SHORT_REMARK_FORBIDDEN_TIME,
        };

        let push = |list: &mut Vec<ForbiddenRemark>, frames: i32, scope: RemarkTargetFlags| {
            list.push(ForbiddenRemark {
                remark,
                flags: scope.bits(),
                speech_id,
                guy_index,
                bad_guy: is_soldier,
                forbidden_till_frame: current_frame + frames as u32,
            });
        };

        match remark {
            // Never forbid — one-shot dialogue remarks.
            // These are used inside scripted conversations where a
            // second line in the same window must still play; forbidding
            // them would break multi-line officer/charly/beggar dialogs
            // and civ/vip wounded/dies pairs.
            Remark::Dies
            | Remark::Strangled
            | Remark::CivWounded
            | Remark::CivDies
            | Remark::VipWounded
            | Remark::VipDies
            | Remark::BadExcuse
            | Remark::CivBeggarBegging
            | Remark::CivBeggarGivesInfo
            | Remark::CivBeggarWantsMore
            | Remark::CivBeggarGivesLastInfo
            | Remark::CivBeggarThanx
            | Remark::OfficerStopsPatrol
            | Remark::OfficerStartsPatrol
            | Remark::OfficerAsksWhatsup
            | Remark::OfficerAsksWhere
            | Remark::OfficerEndsConversation
            | Remark::OfficerCallsSoldier
            | Remark::OfficerSendsOutSoldier
            | Remark::OfficerCallsGroup
            | Remark::OfficerSendsOutGroup
            | Remark::OfficerSendsOutGroupForCharly
            | Remark::OfficerRebukesCharly
            | Remark::OfficerRebukesCharlyEnd
            | Remark::OfficerGivesAttackOrder
            | Remark::OfficerSeesBrawl
            | Remark::OfficerEndsBrawl
            | Remark::GiveOrReceiveOrder
            | Remark::CallsOfficer
            | Remark::TellsOfficerBody
            | Remark::TellsOfficerEnemy
            | Remark::TellsOfficerOther
            | Remark::TellsOfficerCharlyAway
            | Remark::TellsOfficerWhere
            | Remark::AwaitsOrders
            | Remark::TellsOfficerNothing
            | Remark::CharlyDefendsHimself
            | Remark::MissesCharly
            | Remark::DidntFindCharly
            | Remark::FoundCharly
            | Remark::SendsCharlyToOfficer => {}

            // Short forbidden time.
            Remark::Wounded => {
                push(
                    forbidden_remarks,
                    AI_SHORT_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::THIS_TYPE,
                );
            }

            // Civilian sees body/dead body: ALL_NPC scope.
            Remark::CivSeesBody | Remark::CivSeesDeadBody => {
                push(
                    forbidden_remarks,
                    AI_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::ALL_NPC,
                );
            }

            // Drunken: double forbid — type + personal.
            Remark::Drunken => {
                push(
                    forbidden_remarks,
                    AI_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::THIS_TYPE,
                );
                push(
                    forbidden_remarks,
                    AI_DRUNKEN_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::THIS_GUY,
                );
            }

            // Standard THIS_TYPE list from Original Say's switch.
            Remark::AwakensSleeperr
            | Remark::HuntsEnemy
            | Remark::StartsCombat
            | Remark::ProvokesCombat
            | Remark::GoodStrikeCombat
            | Remark::CombatInsult
            | Remark::Warcry
            | Remark::KilledAdversary
            | Remark::Cassos
            | Remark::WaspSting
            | Remark::UnderNet
            | Remark::SeesFriendUnderNet
            | Remark::Arrow
            | Remark::TiedUp
            | Remark::SeesObject
            | Remark::AleYes
            | Remark::AleNo
            | Remark::HitByApple
            | Remark::ChasesChild
            | Remark::CaughtChild
            | Remark::GoldYes
            | Remark::GoldNo
            | Remark::GoldBrawl
            | Remark::SearchingSoldierGold
            | Remark::SearchingSoldierNothing
            | Remark::EndsSearch
            | Remark::Panic
            | Remark::ControlsBeggar
            | Remark::MenacesPcInComa
            | Remark::CryAlert
            | Remark::ShieldBearerCovers
            | Remark::ProudDontFight
            | Remark::ProudFinallyFight
            | Remark::OfficerComplains
            | Remark::OutOfAmmunition
            | Remark::AdmiresObjectScript
            | Remark::MissesObjectScript
            | Remark::CivCallsSoldier
            | Remark::ShieldBearersLineFormation
            | Remark::ArchersBehindShieldBearers
            | Remark::CivDenunciates
            | Remark::CivAdmiresRobin
            | Remark::CivPanic
            | Remark::CivThanx
            | Remark::CivCries
            | Remark::CivBeerYes
            | Remark::CivBeerNo
            | Remark::CivSeesSoldiersUnderNet
            | Remark::CivUnderNet
            | Remark::CivApple
            | Remark::CivWasps
            | Remark::CivWhistling
            | Remark::CivSeesBrawl
            | Remark::CivGoldYes
            | Remark::CivGoldNo
            | Remark::CivBeggarIdentifiesHimself
            | Remark::CivChildCaughtBySoldier
            | Remark::CivChildChasedBySoldier
            | Remark::VipProudDontFight
            | Remark::VipProudFinallyFight
            | Remark::VipStartsCombat
            | Remark::VipGoodStrikeCombat
            | Remark::VipWarcry
            | Remark::VipVictory
            | Remark::VipSpeaksToHimself
            | Remark::VipAleNo
            | Remark::VipNetNo
            | Remark::VipAppleNo
            | Remark::VipWaspsNo
            | Remark::VipGoldNo
            | Remark::HearsNoise
            | Remark::SeesEnemy
            | Remark::SeesBody
            | Remark::BahIlBougePus
            | Remark::SpecialAction => {
                push(
                    forbidden_remarks,
                    AI_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::THIS_TYPE,
                );
            }
            Remark::NumberOfRemarks | Remark::TheSoundOfSilence => {
                panic!("invalid automatic-forbid remark {remark:?}")
            }
        }
    }
}
