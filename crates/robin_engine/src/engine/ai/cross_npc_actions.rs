use super::*;

impl EngineInner {
    fn required_cross_npc_enemy_mut(
        &mut self,
        target: u32,
        operation: &str,
    ) -> &mut crate::ai_enemy::EnemyAi {
        // Original-game combat-neighbor operations take human-actor references.
        // `HumanHandle` is the raw sparse element slot, not a SoldierId; an
        // AI-controlled hero therefore has to retain its ActorPc entity kind here.
        let target_id = self.expect_human_id_for_ai_handle(target, operation);
        self.entities_mut().expect_enemy_ai_mut(
            target_id,
            format_args!("cross-NPC {operation} target human {target}"),
        )
    }

    /// Execute the complete original-game patrol clearing made by the
    /// `RemoveAllSubordinates` script native.
    ///
    /// Clearing an AI patrol clears each member's chief
    /// reference and forces a return to duty before clearing the chief's
    /// lists. This is a direct return, not an `EVENT_RETURN_TO_DUTY` decision: in
    /// particular, it bypasses decision-tick admission's script-lock refusal. Keep the
    /// direct duty transition, movement construction, and recursive callbacks
    /// inside this engine-owned script barrier while leaving ordinary owner
    /// instruction to subsequent sequence processing.
    pub(crate) fn script_remove_all_subordinates(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        chief: EntityId,
    ) {
        let member_count = self
            .ai(chief, "RemoveAllSubordinates chief")
            .theoretical_patrol
            .len();

        for index in 0..member_count {
            let member = *self
                .ai(chief, "RemoveAllSubordinates chief")
                .theoretical_patrol
                .get(index)
                .expect("RemoveAllSubordinates callback shortened the captured patrol prefix");
            let should_return = {
                let ai = self.entities_mut().expect_ai_controller_mut(
                    member,
                    format_args!(
                        "RemoveAllSubordinates chief {} references missing NPC member {}",
                        chief.index(),
                        member.index()
                    ),
                );
                ai.patrol_chief = None;
                ai.current_state == crate::ai::AiState::Default
            };
            if !should_return {
                continue;
            }
            self.execute_ai_return_to_duty(sim, assets, member, crate::ai::DutyFlags::empty());
            // A forced duty call does not close a Think frame. Keep its
            // close-post latch available for the actor's actual completion.
        }

        self.entities_mut()
            .get_mut(chief)
            .and_then(Entity::ai_controller_mut)
            .expect("validated RemoveAllSubordinates chief vanished")
            .clear_patrol();
    }

    // ─── One-shot noise broadcast ──────────────────────────────────

    pub(crate) fn one_shot_noise(
        &self,
        noise_type: crate::ai::NoiseType,
        origin: crate::coordinates::MapPoint,
        origin_layer: Option<crate::position_interface::Layer>,
        volume: u16,
        elevation: u16,
        source_entity: Option<EntityId>,
    ) -> crate::ai::Noise {
        use crate::ai::{Noise, NoiseType};

        let element_id = match noise_type {
            NoiseType::TapTapTap | NoiseType::ZingZing | NoiseType::Aaargh | NoiseType::Heeelp => {
                source_entity.map(|id| id.index() as u16).unwrap_or(0)
            }
            _ => 0,
        };

        // The noise record keeps the complete position supplied by the source,
        // including its motion-sector pointer. Delayed reactions later feed
        // that position through world-point conversion, so dropping the sector
        // also drops authored elevation. Only inherit it when the supplied
        // source still describes this exact noise origin.
        let origin_sector = source_entity
            .and_then(|id| self.entities().get(id))
            .filter(|entity| {
                entity.element_data().position_map() == origin
                    && entity.element_data().optional_layer() == origin_layer
            })
            .and_then(|entity| entity.element_data().sector());

        Noise {
            origin: crate::ai::NoiseOrigin {
                x: origin.x,
                y: origin.y,
                sector: origin_sector,
                layer: origin_layer,
            },
            noise_type,
            volume,
            elevation,
            element_id,
        }
    }

    /// Compute one listener's live subjective copy of a one-shot noise.
    ///
    /// This deliberately mutates deafness at the listener slot. Original
    /// Noise handling computes heard volume immediately before that listener's
    /// synchronous `Think`, so earlier listeners may alter world state before
    /// this method is called for the next registration-array entry.
    pub(crate) fn subjective_one_shot_noise_for(
        &mut self,
        npc_id: EntityId,
        noise: crate::ai::Noise,
    ) -> Option<crate::ai::Noise> {
        const HEARING_FACTOR: f32 = 1.0;

        let (npc_pos, npc_world) = {
            let entity = self.entities().get(npc_id)?;
            let include = match entity {
                Entity::Civilian(_) => true,
                Entity::Soldier(s) => self.camps_are_hostile(
                    s.soldier.cached_camp,
                    crate::element_kinds::Camp::Royalists,
                ),
                _ => false,
            };
            if !include {
                return None;
            }

            // Do not pre-filter inactive or unconscious NPCs. Original runs
            // heard-volume calculation for every registered civilian/Lacklandist and
            // leaves refusal to decision-tick admission, after the deafness read.
            let elem = entity.element_data();
            (elem.position_map(), elem.position())
        };

        let source_elev = noise.elevation as f32;
        let modified_volume = noise.volume as f32 * HEARING_FACTOR;
        // The original game's hearing-volume calculation subtracts the source point from the
        // listener's authoritative world position. Do not rebuild Y
        // from `position_map + elevation`: a 3D-authored position projected
        // into map space can reconstruct one bit away, which is observable
        // when the positive remainder truncates to 16 bits at volume 1.
        let dx = npc_world.x - noise.origin.x;
        let dy_world = npc_world.y - noise.origin.y - source_elev;
        let dz = npc_world.z - source_elev;

        // Original compares the full 3D points before range and deafness
        // work. A wounded or trapped source therefore cannot hear its own
        // AAARGH/HEEELP broadcast.
        if dx == 0.0 && dy_world == 0.0 && dz == 0.0 {
            return None;
        }

        let dy_stretched = dy_world * crate::position_interface::INVERSE_ASPECT_RATIO;
        if dx.abs().max(dy_stretched.abs()).max(dz.abs()) > modified_volume {
            return None;
        }

        let distance = (dx * dx + dy_stretched * dy_stretched + dz * dz).sqrt();
        // Heard-volume calculation returns before checking deafness when the Euclidean
        // remainder is non-positive, even if the earlier max-norm range test
        // admitted the source.
        if modified_volume - distance <= 0.0 {
            return None;
        }

        let cover_volume = self
            .feedback
            .sound_sim
            .sources
            .max_noise_covering_volume_for_3d(npc_pos.x, npc_pos.y, npc_world.z);
        let frame = self.control.frame_counter;
        let deafness = self
            .entities_mut()
            .expect_ai_actor_data_mut(
                npc_id,
                format_args!(
                    "one-shot noise listener {} lost its required AI actor state",
                    npc_id.index()
                ),
            )
            .get_deafness(frame, cover_volume);

        let subjective = subjective_hear_volume(modified_volume, distance, deafness);
        (subjective != 0).then_some(crate::ai::Noise {
            volume: subjective,
            ..noise
        })
    }

    fn display_one_shot_noise(&mut self, noise: crate::ai::Noise) {
        // The original game displays noise only after every listener's AI update.
        self.feedback
            .pending_side_effects
            .displayed_noises
            .push(noise);
    }

    /// Broadcast a one-shot noise and synchronously run each listener's new
    /// hearing event, in original-game NPC registration order.
    ///
    /// Original-game NPC noise handling invokes AI inside the broadcast
    /// loop. Script natives and other in-frame callbacks therefore observe
    /// the listeners' RNG draws, state transitions, and launched sequences
    /// before returning.
    pub(in crate::engine) fn broadcast_noise_synchronously(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        noise_type: crate::ai::NoiseType,
        origin: crate::coordinates::MapPoint,
        origin_layer: Option<crate::position_interface::Layer>,
        volume: u16,
        elevation: u16,
        source_entity: Option<EntityId>,
    ) {
        use crate::ai::{Stimulus, StimulusType};

        let noise = self.one_shot_noise(
            noise_type,
            origin,
            origin_layer,
            volume,
            elevation,
            source_entity,
        );

        let count = self.world.npc_registry_ids.len();
        for index in 0..count {
            let npc_id = self.world.npc_registry_ids[index];
            let Some(subjective_noise) = self.subjective_one_shot_noise_for(npc_id, noise) else {
                continue;
            };
            let stimulus = Stimulus::with_noise(StimulusType::EventHear, subjective_noise);

            // Each listener observes all mutations from the preceding call.
            self.execute_ai_callback(sim, assets, npc_id, &stimulus);
        }
        self.display_one_shot_noise(noise);
    }

    pub(in crate::engine) fn apply_update_left_combat_neighbour(
        &mut self,
        target: u32,
        old_left: Option<crate::ai::AiEntityHandle>,
        new_left: Option<crate::ai::AiEntityHandle>,
    ) {
        if let Some(old_left) = old_left {
            self.required_cross_npc_enemy_mut(old_left.get(), "unlink-old-left-neighbour")
                .right_combat_neighbour = None;
        }
        self.required_cross_npc_enemy_mut(target, "update-left-combat-neighbour")
            .left_combat_neighbour = new_left;
        if let Some(new_left) = new_left {
            let new_lefts_old_right = self
                .required_cross_npc_enemy_mut(new_left.get(), "inspect-new-left-neighbour")
                .right_combat_neighbour;
            if let Some(new_lefts_old_right) = new_lefts_old_right {
                self.required_cross_npc_enemy_mut(
                    new_lefts_old_right.get(),
                    "unlink-new-left-old-right-neighbour",
                )
                .left_combat_neighbour = None;
            }
            self.required_cross_npc_enemy_mut(new_left.get(), "link-new-left-neighbour")
                .right_combat_neighbour = Some(crate::ai::AiEntityHandle::new(target));
        }
    }

    pub(in crate::engine) fn apply_update_right_combat_neighbour(
        &mut self,
        target: u32,
        old_right: Option<crate::ai::AiEntityHandle>,
        new_right: Option<crate::ai::AiEntityHandle>,
    ) {
        if let Some(old_right) = old_right {
            self.required_cross_npc_enemy_mut(old_right.get(), "unlink-old-right-neighbour")
                .left_combat_neighbour = None;
        }
        self.required_cross_npc_enemy_mut(target, "update-right-combat-neighbour")
            .right_combat_neighbour = new_right;
        if let Some(new_right) = new_right {
            let new_rights_old_left = self
                .required_cross_npc_enemy_mut(new_right.get(), "inspect-new-right-neighbour")
                .left_combat_neighbour;
            if let Some(new_rights_old_left) = new_rights_old_left {
                self.required_cross_npc_enemy_mut(
                    new_rights_old_left.get(),
                    "unlink-new-right-old-left-neighbour",
                )
                .right_combat_neighbour = None;
            }
            self.required_cross_npc_enemy_mut(new_right.get(), "link-new-right-neighbour")
                .left_combat_neighbour = Some(crate::ai::AiEntityHandle::new(target));
        }
    }

    /// Run a decision and deliver the sequence completion callbacks it causes
    /// before returning its handled result.
    pub(in crate::engine) fn dispatch_think_with_drain(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        assets: &LevelAssets,
    ) -> bool {
        let had_ai_at_entry = self
            .entities()
            .get(npc_id)
            .and_then(Entity::ai_controller)
            .is_some();
        let handled = self.dispatch_filtered_stimulus_inner(sim, assets, npc_id, stimulus);

        // PCs can participate in direct swordfights but have no NPC AI
        // controller or AI-owned recovery effects to drain.
        if !had_ai_at_entry && matches!(self.world.entities.get(npc_id), Some(Entity::Pc(_))) {
            return handled;
        }

        // Decision-tick admission applies view status synchronously for
        // LOSE_CONSCIOUSNESS, WASP, and NET. FITAGAIN can publish its
        // resurrection work at this same boundary. The typed AI records
        // those engine-owned writes while its controller is borrowed; commit
        // them immediately after Think returns, before waypoint callbacks or
        // any other pending/re-entrant work can observe stale NPC state.

        handled
    }

    pub(in crate::engine) fn execute_ai_look_there(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        source_id: EntityId,
        position: crate::ai::Position,
        radius: u16,
    ) {
        let camp = self.expect_entity(source_id, "look-there caller").camp();
        let count = self.world.soldier_registry.all().len();
        let radius_squared = f32::from(radius).powi(2);
        let stimulus = crate::ai::Stimulus {
            info: crate::ai::StimulusInfo::Hint(crate::ai::Hint {
                seek_point: position,
                seek_flags: 0,
                who_tells_me: crate::ai::AiEntityHandle::new(source_id.index()),
            }),
            ..crate::ai::Stimulus::new(crate::ai::StimulusType::CallLookThere)
        };
        for index in 0..count {
            let handle = *self
                .world
                .soldier_registry
                .all()
                .get(index)
                .expect("look-there soldier registry shortened during callback");
            let target_id = EntityId::Soldier(crate::entity_id::SoldierId(handle));
            if target_id == source_id {
                continue;
            }
            let entity = self.expect_entity(target_id, "look-there soldier");
            if !self.camps_are_allied(entity.camp(), camp) {
                continue;
            }
            let ai = entity
                .enemy_ai()
                .expect("look-there soldier requires enemy AI");
            if !matches!(
                ai.base.current_state,
                crate::ai::AiState::Default | crate::ai::AiState::Wondering
            ) && !(ai.base.current_state == crate::ai::AiState::Seeking
                && matches!(
                    ai.base.current_substate,
                    crate::ai::Substate::SeekingJustWatching
                        | crate::ai::Substate::SeekingJustWatchingSidewards
                ))
            {
                continue;
            }
            let target = entity.element_data().position();
            let caller = self
                .expect_entity(source_id, "look-there range caller")
                .element_data()
                .position();
            let dx = target.x - caller.x;
            let dy = target.y - caller.y;
            let dz = target.z - caller.z;
            if look_there_target_is_inside_radius(dx * dx + dy * dy + dz * dz, radius_squared) {
                self.execute_ai_callback(sim, assets, target_id, &stimulus);
            }
        }
    }
}
