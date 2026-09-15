//! Host-facing engine accessors and the host-driven mutations that were
//! historically grouped with them: seat/shoot-list/macro bookkeeping, entity
//! lookup, campaign value updates and post-load fixups. Moved out of
//! `engine/mod.rs` so that file holds mission lifecycle only.

use super::*;

impl EngineInner {
    // ─── Read-only accessors for host renderer / input ───────────

    /// Restore an original-game parity-session boundary field that the v48 save
    /// serializer omits. This is intentionally a replay-only seam: normal
    /// simulation updates the value during detection refresh.
    pub(crate) fn restore_parity_npc_maximal_visibility(&mut self, id: EntityId, value: u16) {
        let entity = self
            .world
            .entities
            .get_mut(id)
            .unwrap_or_else(|| panic!("parity NPC transient references missing entity {id:?}"));
        let npc = entity
            .npc_data_mut()
            .unwrap_or_else(|| panic!("parity NPC transient references non-NPC entity {id:?}"));
        let ai = npc
            .ai_brain
            .base_mut()
            .unwrap_or_else(|| panic!("parity NPC transient references AI-less entity {id:?}"));
        ai.max_visibility = u32::from(value);
    }

    /// Restore a dormant original-game waypoint-macro reference which survived an
    /// in-process v48 load even though that save omitted it because
    /// the patrol-path flag was false.
    pub(crate) fn restore_parity_npc_dormant_macro_cursor(
        &mut self,
        id: EntityId,
        path_id: crate::ai::PathId,
        waypoint_index: u8,
        offset: usize,
        assets: &LevelAssets,
    ) -> bool {
        let waypoint = assets
            .navigation
            .hiking_paths
            .get(usize::from(path_id))
            .and_then(|path| path.waypoints.get(usize::from(waypoint_index)))
            .unwrap_or_else(|| {
                panic!(
                    "parity dormant cursor references absent waypoint {path_id:?}/{waypoint_index}"
                )
            });
        let crate::level_data::WaypointCommand::Macro(command) = &waypoint.command else {
            panic!(
                "parity dormant cursor references non-macro waypoint {path_id:?}/{waypoint_index}"
            );
        };
        assert!(
            offset <= command.len(),
            "parity dormant cursor offset {offset} exceeds command length {}",
            command.len()
        );
        let entity =
            self.world.entities.get_mut(id).unwrap_or_else(|| {
                panic!("parity dormant cursor references missing entity {id:?}")
            });
        let ai = entity
            .npc_data_mut()
            .and_then(|npc| npc.ai_brain.base_mut())
            .unwrap_or_else(|| panic!("parity dormant cursor references AI-less NPC {id:?}"));
        if ai.has_patrol_path {
            return false;
        }
        ai.macro_command = command.clone();
        ai.macro_command_offset = offset;
        ai.macro_command_waypoint = Some((path_id, waypoint_index));
        true
    }

    /// Ensure a seat exists for `player_id`, growing `self.players.seats` with
    /// default [`SeatState`]s as needed, and return its index.
    ///
    /// New seats start empty (no selection, no hotgroups) — they only
    /// pick up state once the player issues commands.  This is the
    /// drop-in/drop-out hook: a peer that joins mid-mission gets a
    /// fresh seat, and a peer that leaves keeps its slot (their
    /// last-issued selection survives so the PCs stay where they
    /// were left, on autopilot).
    pub(crate) fn ensure_seat(&mut self, player_id: crate::player_command::PlayerId) -> usize {
        let idx = player_id.0 as usize;
        if idx >= self.players.seats.len() {
            self.players.seats.resize_with(idx + 1, SeatState::default);
        }
        idx
    }

    pub(in crate::engine) fn queue_pc_shoot_bow(
        &mut self,
        owner: EntityId,
        element_ref: crate::sequence::SequenceElementRef,
    ) {
        let human = self
            .world
            .entities
            .get_mut(owner)
            .and_then(|entity| entity.human_data_mut())
            .unwrap_or_else(|| panic!("shoot-list owner {} is not human", owner.index()));
        if !human.pending_shoots.contains(&element_ref) {
            human.pending_shoots.push(element_ref);
        }
    }

    /// Drop the retained human-instruction shoot FIFO without altering the
    /// sequence elements themselves, matching the original game's list clearing.
    pub(in crate::engine) fn clear_pc_shoot_list(&mut self, owner: EntityId) -> bool {
        let human = self
            .world
            .entities
            .get_mut(owner)
            .and_then(|entity| entity.human_data_mut())
            .unwrap_or_else(|| panic!("shoot-list owner {} is not human", owner.index()));
        let had_entries = !human.pending_shoots.is_empty();
        human.pending_shoots.clear();
        had_entries
    }

    /// Original-game shoot-list processing retries the oldest
    /// retained element synchronously, and remove it only when instruction handling
    /// accepts it. The sprite animation gate is deliberately exact.
    pub(crate) fn process_shoot_list_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let entity = self.world.entities.get(owner).unwrap_or_else(|| {
            panic!(
                "shoot-list owner {} disappeared from its legacy slot",
                owner.index()
            )
        });
        assert!(
            entity.human_data().is_some(),
            "shoot-list owner {} is not human",
            owner.index()
        );
        use crate::order::OrderType;
        if !matches!(
            entity.sprite().last_action,
            OrderType::AimingWithBow | OrderType::AimingWithBowUp
        ) {
            return;
        }
        let Some(element_ref) = entity
            .human_data()
            .and_then(|human| human.pending_shoots.first().copied())
        else {
            return;
        };
        let accepted = self.instruct_owner(
            sim,
            assets,
            &mut Vec::new(),
            owner,
            element_ref.sequence_id,
            element_ref.element_index,
        );
        if accepted {
            let human = self
                .world
                .entities
                .get_mut(owner)
                .and_then(|entity| entity.human_data_mut())
                .expect("validated human shoot-list owner disappeared");
            assert_eq!(human.pending_shoots.first(), Some(&element_ref));
            human.pending_shoots.remove(0);
        }
    }

    /// Install the titbit renderer's per-row frame counts.  Called at
    /// level load and whenever the ambience shadow colour changes (the
    /// titbit atlas is rebuilt host-side and hands fresh counts back).
    /// Safe to call mid-tick: `titbit_manager.row_frame_counts` is
    /// level renderer metadata and not part of the rollback hash.
    pub(crate) fn set_titbit_row_frame_counts(&mut self, counts: Vec<u16>) {
        self.feedback.titbit_manager.set_row_frame_counts(counts);
    }

    /// Remove all titbits owned by `pc` at QA slot `slot`.  Resolves
    /// the titbit id from the PC's per-slot titbit-id table, then
    /// drops every titbit whose id matches.  Returns `true` iff at
    /// least one titbit was removed (also `false` when the slot is
    /// empty).
    pub(crate) fn remove_quick_action_titbits_for(&mut self, pc: EntityId, slot: u8) -> bool {
        let Some(state) = self.players.macro_store.get(pc) else {
            return false;
        };
        let Some(titbit_id) = state.get_slot_titbit(slot as usize) else {
            return false;
        };
        self.feedback
            .titbit_manager
            .remove_quick_action_titbits_by_id(titbit_id)
    }

    /// Abort the macro at `(pc, slot)`: drop the slot's titbit and clear
    /// the slot's recorded steps + stored titbit id.  Returns `true` iff
    /// the slot had a macro before the call.
    ///
    pub(crate) fn abort_quick_action(&mut self, pc: EntityId, slot: u8) -> bool {
        if !self.has_quick_action(pc, slot) {
            return false;
        }
        self.remove_quick_action_titbits_for(pc, slot);
        if let Some(state) = self.players.macro_store.get_mut(pc) {
            state.clear_slot(slot as usize);
        }
        let saved_pc = self
            .get_entity_mut(pc)
            .and_then(|entity| entity.pc_data_mut())
            .unwrap_or_else(|| panic!("quick-action owner {pc:?} is not a PC"));
        let slot = slot as usize;
        saved_pc.quick_action_types[slot] = crate::element_kinds::QuickAction::None;
        saved_pc.quick_action_sequences[slot] = None;
        saved_pc.quick_seek_sequences[slot] = None;
        saved_pc.quick_action_special_counts[slot] = 0;
        saved_pc.quick_action_buttons[slot] = 0;
        saved_pc.quick_action_interactors[slot] = None;
        saved_pc.titbits[slot] = None;
        true
    }

    /// Tetris-shift slot `slot..NUMBER_OF_QA_MEMORY` on every PC.
    /// Called once all PCs have successfully launched their slot-`slot`
    /// macros — see `apply_start_macro` which drives the call.
    pub(crate) fn do_tetris_macro(&mut self, slot: u8) {
        let pcs = self.world.pc_ids.clone();
        for pc in pcs {
            if let Some(state) = self.players.macro_store.get_mut(pc) {
                state.do_tetris(slot as usize);
            }
            let saved_pc = self
                .get_entity_mut(pc)
                .and_then(|entity| entity.pc_data_mut())
                .unwrap_or_else(|| panic!("quick-action owner {pc:?} is not a PC"));
            let first = slot as usize;
            for index in first..crate::macro_store::NUMBER_OF_QA_MEMORY - 1 {
                saved_pc.quick_action_types[index] = saved_pc.quick_action_types[index + 1];
                saved_pc.quick_action_sequences[index] =
                    saved_pc.quick_action_sequences[index + 1].clone();
                saved_pc.quick_seek_sequences[index] =
                    saved_pc.quick_seek_sequences[index + 1].clone();
                saved_pc.titbits[index] = saved_pc.titbits[index + 1];
                saved_pc.quick_action_interactors[index] =
                    saved_pc.quick_action_interactors[index + 1];
                saved_pc.quick_action_buttons[index] = saved_pc.quick_action_buttons[index + 1];
                saved_pc.portrait.quick_icons[index] = saved_pc.portrait.quick_icons[index + 1];
            }
            let last = crate::macro_store::NUMBER_OF_QA_MEMORY - 1;
            saved_pc.quick_action_types[last] = crate::element_kinds::QuickAction::None;
            saved_pc.quick_action_sequences[last] = None;
            saved_pc.quick_seek_sequences[last] = None;
            saved_pc.titbits[last] = None;
            saved_pc.quick_action_interactors[last] = None;
            saved_pc.quick_action_buttons[last] = 0;
        }
        let slots = self.macro_slot_lengths();
        self.feedback
            .pending_side_effects
            .host_events
            .push(HostEvent::MacroUi(MacroUiHostEvent::RearmTetris {
                slot: slot as usize,
                slots,
            }));
    }

    /// Enable or disable the `--goldeneye` cheat (NPCs can't see the player).
    /// Set once at startup from CLI args.
    pub(crate) fn set_golden_eye_mode(&mut self, on: bool) {
        self.ai.global.golden_eye_mode = on;
    }

    /// Populate ground-mark sprite data at resource-load time (host-side
    /// call). Runtime writes into `ground_mark` are command-driven engine
    /// mutations; per-seat trajectory preview marks live on `Host`.
    pub(crate) fn set_ground_mark_sprite_data(
        &mut self,
        half_w: f32,
        half_h: f32,
        frame_sizes: Vec<(u16, u16)>,
        per_frame_offsets: Vec<(i16, i16)>,
    ) {
        self.feedback
            .ground_mark
            .set_sprite_data(half_w, half_h, frame_sizes, per_frame_offsets);
    }

    /// Combined static + dynamic sight obstacles; see
    /// [`WorldState::sight_obstacles`](super::state::WorldState::sight_obstacles).
    pub fn sight_obstacles<'a>(
        &'a self,
        assets: &'a LevelAssets,
    ) -> crate::sight_obstacle::ObstacleList<'a> {
        self.world.sight_obstacles(assets)
    }

    /// Mutator for the runtime active flag on a static sight obstacle.
    /// Out-of-range indices (including dynamic obstacles) silently no-op
    /// — dynamic obstacles are always implicitly active.
    pub(crate) fn set_sight_obstacle_active(&mut self, idx: u32, active: bool) {
        if let Some(slot) = self
            .world
            .static_sight_obstacle_active
            .get_mut(idx as usize)
        {
            *slot = active;
        }
    }

    /// Deliver a host-originated simulation message immediately.
    pub(crate) fn send_simple_message(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        msg: crate::messenger::SimpleMessage,
    ) {
        self.forward_message(
            sim,
            assets,
            crate::messenger::Message::new(crate::messenger::MessageType::Simple(msg)),
        );
    }

    /// Stop the in-progress quick-action macro recording (host-side
    /// portrait-click handler).  Idempotent.
    pub(crate) fn stop_recording_macro(&mut self) {
        let slot = self.players.qa_recording_slot as usize;
        let recording = self.players.qa_recording_for.clone();
        for pc_id in recording {
            if let Some(state) = self.players.macro_store.get_mut(pc_id) {
                state.stop_recording();
            }
            let (has_macro, titbit) = self
                .players
                .macro_store
                .get(pc_id)
                .map(|state| (state.has_macro(slot), state.get_slot_titbit(slot)))
                .unwrap_or((false, None));
            let icon = if has_macro {
                let titbit = titbit.unwrap_or_else(|| {
                    panic!("recorded quick-action PC {pc_id:?} slot {slot} has no titbit")
                });
                crate::element::PcPortraitQuickIconState {
                    titbit_id: Some(titbit),
                    running: self.feedback.titbit_manager.is_running_for_qa(titbit),
                }
            } else {
                Default::default()
            };
            let pc = self
                .get_entity_mut(pc_id)
                .and_then(|entity| entity.pc_data_mut())
                .unwrap_or_else(|| panic!("quick-action recording target {pc_id:?} is not a PC"));
            pc.portrait.quick_icons[slot] = icon;
        }
        self.players.qa_recording_for.clear();
    }

    /// Complete selection notifications before clearing the recorded action.
    pub(crate) fn emit_character_selection_followups(&mut self) {
        self.update_recording_after_selection_change();
        self.players.action_before_recording_macro = crate::profiles::Action::NoAction;
    }

    pub(crate) fn update_recording_after_selection_change(&mut self) {
        for seat in &mut self.players.seats {
            if seat
                .planned_shield_target
                .is_some_and(|(actor, _)| !seat.selection.contains(&actor))
            {
                seat.planned_shield_target = None;
            }
        }
        if self.players.qa_recording_for.is_empty() {
            return;
        }
        let slot = self.players.qa_recording_slot;
        let selected: Vec<EntityId> = self.players.seats[0].selection.clone();
        let current = self.players.qa_recording_for.clone();
        for pc_id in &current {
            if !selected.contains(pc_id)
                && let Some(state) = self.players.macro_store.get_mut(*pc_id)
            {
                state.stop_recording();
            }
        }
        for pc_id in &selected {
            if !current.contains(pc_id) {
                self.players
                    .macro_store
                    .get_or_insert(*pc_id)
                    .begin_recording(slot);
            }
        }
        self.players.qa_recording_for = selected;
    }

    /// Request the PC-info hover overlay to show (`Some(pc_id)`) or hide
    /// (`None`).  The host writes into this via its per-frame mouse
    /// handler; the renderer reads the overlay after the tick drains
    /// [`SideEffects::overlay`] into [`Host::pc_info_overlay`].
    ///
    /// Backed by the `MSG_SHOW_PC_INFORMATION` /
    /// `MSG_HIDE_PC_INFORMATION` messenger pair — the messenger
    /// indirection exists for engine-internal sites, but the host just
    /// writes the overlay directly because there's nothing else
    /// listening.
    ///
    /// Both show and hide handlers early-out unless we're in Sherwood,
    /// so the popup only ever appears in the Sherwood (HQ) mission.
    pub(crate) fn request_pc_info_overlay(
        &mut self,
        assets: &LevelAssets,
        focus: Option<EntityId>,
    ) {
        if !self.is_sherwood(&assets.profile_manager) {
            return;
        }
        self.feedback.pending_side_effects.overlay = Some(match focus {
            Some(pc_id) => OverlayChange::Show { pc_id },
            None => OverlayChange::Hide,
        });
    }

    /// `true` when the current mission is the Sherwood (HQ) hideout.
    pub fn is_sherwood(&self, profiles: &crate::profiles::ProfileManager) -> bool {
        self.is_sherwood_mission(&self.mission_domain.campaign, profiles)
    }

    /// Build a fresh `Order` (via `alloc_order_id` for the id) and push
    /// it. Shorthand for the common engine-side pattern of allocating a
    /// unique id, building an Order at `(x, y)` with `order_type`, and
    /// pushing it onto the given element.
    pub(crate) fn push_new_order(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        order_type: crate::order::OrderType,
        x: f32,
        y: f32,
    ) -> std::num::NonZeroU32 {
        let id = self.orders.allocate_order_id();
        self.orders.sequence_manager.push_order_on(
            seq_id,
            elem_idx,
            crate::order::Order::new(order_type, x, y, id),
        );
        id
    }

    /// Advance a sequence element to its next order, or terminate when
    /// the order list is exhausted.  Pops the front order; if a new
    /// front exists, the element keeps running with that order;
    /// otherwise the element terminates and `EventDone` fires up the
    /// chain.
    ///
    /// This runs whenever an order's animation completes with the
    /// default [`OrderCompletion::AdvanceElement`] hook.  When the
    /// queue drains for a non-wait element, we terminate it. The actor
    /// The actor update installs a fresh wait element at its next entry only when no
    /// synchronous condolence/AI callback instructed a real successor.
    ///
    /// The BORED ↔ BORED_RANDOM idle cycle does NOT route through here
    /// — its Execute arm consumes the event in
    /// `dispatch_arm_completion` (`engine/animation.rs`) and mutates
    /// the front order in place without popping.
    pub(crate) fn do_next_order(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        if tracing::enabled!(target: "parity_owner_handoff", tracing::Level::TRACE) {
            let element_state = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .map(|element| {
                    (
                        element.owner,
                        element.command,
                        element.state,
                        element
                            .orders
                            .front()
                            .map(|order| (order.order_type, order.order_id)),
                        element.orders.len(),
                        element
                            .postponed
                            .map(|reference| (reference.sequence_id, reference.element_index)),
                    )
                });
            let sequence_state =
                self.orders
                    .sequence_manager
                    .get_sequence(seq_id)
                    .map(|sequence| {
                        sequence
                            .elements
                            .iter()
                            .enumerate()
                            .map(|(index, element)| {
                                (
                                    index,
                                    element.owner,
                                    element.command,
                                    element.command_level,
                                    element.state,
                                    element.priority,
                                    element.orders.len(),
                                    element.postponed.map(|reference| {
                                        (reference.sequence_id, reference.element_index)
                                    }),
                                )
                            })
                            .collect::<Vec<_>>()
                    });
            let owner_state = element_state
                .and_then(|(owner, _, _, _, _, _)| owner)
                .map(|owner| {
                    let selected = self.world.entities.current_element_for_actor(owner);
                    let goal = self
                        .get_entity(owner)
                        .map(|entity| entity.position_iface().map_goal());
                    (selected, goal)
                });
            tracing::trace!(
                target: "parity_owner_handoff",
                frame = self.control.frame_counter,
                ?seq_id,
                elem_idx,
                ?element_state,
                ?sequence_state,
                ?owner_state,
                "do_next_order before popping front order"
            );
        }
        // Pop the just-completed front order, capture context.
        let Some((owner, next_order)) = self
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .map(|elem| {
                if !elem.orders.is_empty() {
                    let popped = elem.pop_current_order();
                    if tracing::enabled!(tracing::Level::TRACE) {
                        let remaining: Vec<(crate::order::OrderType, f32, f32)> = elem
                            .orders
                            .iter()
                            .map(|o| (o.order_type, o.target_x, o.target_y))
                            .collect();
                        tracing::trace!(
                            owner = ?elem.owner,
                            ?popped,
                            ?remaining,
                            "do_next_order: popped front order"
                        );
                    }
                }
                let next_order =
                    elem.current_order()
                        .map(|order| crate::element::InstalledActorOrder {
                            order_id: order.order_id,
                            order_type: order.order_type,
                        });
                (elem.owner, next_order)
            })
        else {
            return;
        };

        if let Some(owner) = owner {
            self.world
                .entities
                .get_mut(owner)
                .and_then(Entity::actor_data_mut)
                .expect("order advancement owner disappeared")
                .execute_order_initialising = true;
        }
        if let Some(next_order) = next_order {
            // Original-game order advancement publishes the next order immediately
            // and, when a successor exists, republishes the motion state as
            // an in-progress result before the update returns
            // during the terminated-action callback. Callers reach here right after
            // latching a terminated motion, so that latch has to be replaced
            // for the frame snapshot to show the successor's motion.
            if let Some(owner) = owner {
                let actor = self
                    .world
                    .entities
                    .get_mut(owner)
                    .and_then(Entity::actor_data_mut)
                    .expect("next-order owner disappeared before mpOrder publication");
                actor.installed_order = Some(next_order);
                actor.continuation.motion_state = crate::sprite::MotionState::InProgress;
            }
            return;
        }

        // Clear the exhausted order while retaining selection through the
        // termination callback, which owns goal and selection cleanup.
        if let Some(owner) = owner {
            self.world
                .entities
                .get_mut(owner)
                .and_then(Entity::actor_data_mut)
                .expect("exhausted-order owner disappeared before mpOrder clear")
                .installed_order = None;
        }

        // Terminate the element. Do not eagerly install Wait here:
        // Advancing actor orders terminates the element, whose
        // Removal-notification callback can synchronously instruct a real
        // successor. The actor's next update entry supplies Wait only if
        // that stack unwinds without one.
        self.element_terminated(sim, assets, &mut Vec::new(), seq_id, elem_idx);
    }

    /// Guarantee that `entity_id` has a live `Command::Wait` sequence
    /// element running at `SequencePriority::Wait`.  Launches a fresh
    /// wait element whenever the actor has no current order to
    /// execute.  No-op when a wait element already exists for this
    /// actor.
    ///
    /// Called by the null-order guard at the start of an actor update.
    /// Exhausting the final order does not call this again in the same slot:
    /// The original game leaves the actor order empty through ActionChange and creates the
    /// fallback Wait on the actor's next frame.
    pub(crate) fn ensure_wait_element(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        use crate::sequence::{SequenceElement, SequencePriority};

        // The original actor update installs Wait whenever the actor has no
        // current order. Future Todo/Postponed elements do not count: they may
        // sit behind an ownerless Timer while this actor idles. A concurrently
        // InProgress element is the only state that corresponds to the
        // the original game's live selected element and actor order.
        if self
            .world
            .entities
            .current_element_for_actor(entity_id)
            .is_some()
        {
            return;
        }

        let mut elem = SequenceElement::new(1, crate::element::Command::Wait, Some(entity_id));
        elem.priority = SequencePriority::Wait;
        // Actor waiting launches this through the normal owned-element
        // instruction path. That path stamps the current posture/action state
        // and, crucially, prepends the Waiting -> Bored transition orders.
        // Bypassing it made a freshly loaded upright NPC jump straight from
        // its authored WAITING_UPRIGHT pose to WAITING_UPRIGHT_BORED on the
        // first frame.
        self.launch_element(sim, assets, elem);
    }

    /// Consume the typed motion-stage input and feed it into
    /// the motion grid (pathfinder graph, lift tables, obstacle states).
    /// Called once during `Engine::new`; bridges background-load and
    /// motion-area initialisation.  Must run only during level load —
    /// it mutates hashed state and is not driven by the tick pipeline,
    /// so calling it during gameplay would desync rollback.
    pub(crate) fn build_motion_stage(
        &mut self,
        assets: &mut LevelAssets,
        staging: &mut LevelLoadStaging,
    ) {
        if let Some(motion_data) = staging.motion.motion_data.take() {
            let lifts = std::mem::take(&mut staging.motion.lifts);
            self.initialize_motion_from_level_data(assets, staging, &motion_data, &lifts);
        }
    }

    /// Reveal all blipped entities — backs the console `UNBLIP`
    /// command, which iterates every NPC and reveals it.
    pub(crate) fn reveal_all_blips(&mut self) {
        for (_, entity) in self.world.entities.npcs_mut() {
            if entity.element_data().blipped {
                entity.reveal_blip();
            }
        }
    }

    /// Get a mutable reference to an entity by ID.
    pub(crate) fn get_entity_mut<I: Into<EntityId>>(&mut self, id: I) -> Option<&mut Entity> {
        self.world.entities.get_mut(id)
    }

    /// Mutable counterpart of [`Self::expect_entity`]: the entity is required
    /// to exist and its absence is a sim-state corruption, not a normal
    /// condition.
    #[track_caller]
    pub(crate) fn expect_entity_mut<I: Into<EntityId>>(&mut self, id: I, ctx: &str) -> &mut Entity {
        let id = id.into();
        self.get_entity_mut(id)
            .unwrap_or_else(|| panic!("required entity {id:?} missing ({ctx})"))
    }

    /// Remove an entity. Leaves a None hole (IDs are stable).
    pub(crate) fn remove_entity<I: Into<EntityId>>(&mut self, id: I) {
        let id = id.into();
        // Alert counters track constructed soldier brains, not registry membership.
        // Removing an entity does not reverse its last music-alert contribution.
        self.world.entities.remove(id);
        self.world.soldier_registry.remove(id);
        self.world.npc_registry_ids.retain(|&member| member != id);
        self.world.actor_registry_ids.retain(|&member| member != id);
        self.world
            .fighter_registry_ids
            .retain(|&member| member != id);
        // Remove from index lists
        self.world.pc_ids.retain(|&i| i != id);
        self.world.original_pc_registry_ids.retain(|&i| i != id);
        self.players.remove_entity(id);
        self.orders.remove_entity(id);
        for (_, entity) in self.world.entities.occupied_mut() {
            if let Some(ai) = entity.ai_controller_mut() {
                ai.remove_entity(id);
            }
        }
    }

    /// Retire a replaced PC from Original's live party registry without
    /// deleting its corpse or its portrait-order entry.
    pub(crate) fn retire_replaced_pc(&mut self, id: EntityId) {
        self.world
            .original_pc_registry_ids
            .retain(|&pc_id| pc_id != id);
    }

    /// Number of live entities.
    pub fn entity_count(&self) -> usize {
        self.world.entities.occupied().count()
    }

    /// Remove a PC entity from the engine by its character profile index.
    ///
    /// 1. Look up the PC by profile index.
    /// 2. Clear it from the current selection (forwards
    ///    `MSG_UNSELECT_CHARACTER`).  `remove_entity` would retain it
    ///    out of `selected_hero_ids` too, but doing it here keeps any
    ///    intermediate inspection consistent.
    /// 3. Flag the PC as no longer playable.
    /// 4. Detach the entity slot from all ID lists
    ///    while retaining its script binding.
    ///
    /// Used by [`convert_selected_peasants_to_blazons`] (and any
    /// future peasant-liquidation path).  Returns `true` when a PC
    /// was actually removed, `false` when no matching entity was
    /// found.
    pub(crate) fn remove_pc_by_profile(
        &mut self,
        profile_idx: crate::profiles::CharacterProfileIdx,
    ) -> bool {
        let Some(pc_id) = self.world.pc_ids.iter().copied().find(|&id| {
            matches!(
                self.get_entity(id),
                Some(Entity::Pc(pc)) if pc.pc.profile_index == profile_idx,
            )
        }) else {
            return false;
        };

        // `MSG_UNSELECT_CHARACTER`: clears selection, hides portrait
        // highlight, etc.  The selection list is authoritative, so
        // removing the id here mirrors the message's observable effect.
        for seat in &mut self.players.seats {
            seat.selection.retain(|&id| id != pc_id);
        }

        // Non-playable status — survives into the handful of frames
        // between clearing selection and wiping the slot.  After
        // `remove_entity` the field is academic.
        if let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id) {
            pc.pc.playable = false;
        }

        // Remove the PC while retaining its script binding.
        self.remove_entity(pc_id);
        true
    }

    /// Convert selected peasants to blazons.
    ///
    /// Walks the mission team, sorting each peasant into reservists
    /// (random-weighted by life points) or straight removal, invokes
    /// `remove_pc_by_profile` per peasant, resets the mission team,
    /// and credits `BLAZON_VALUE`.
    ///
    /// Triggered from `MSG_START_MISSION` when
    /// `IsMenToBlazonConversionMode()` is set; the caller lives in
    /// `game_session.rs` on the Sherwood "StartMission" button path.
    pub(crate) fn convert_selected_peasants_to_blazons(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        profiles: &crate::profiles::ProfileManager,
    ) {
        let campaign = &self.mission_domain.campaign;
        let number_to_convert =
            campaign.get_number_of_peasants_to_convert_to_blazons(profiles) as usize;
        let quotation = {
            let next_idx = match campaign.next_mission_idx {
                Some(i) => i,
                None => {
                    tracing::warn!("convert_selected_peasants_to_blazons: no next mission");
                    return;
                }
            };
            campaign.missions[next_idx]
                .profile(profiles)
                .peasant_to_blazon_quotation
        };
        let mission_team: Vec<usize> = campaign.mission_team_indices.clone();

        // Snapshot life_points + profile_idx per team entry before we
        // start mutating the campaign.  `remove_pc_by_profile` takes a
        // profile index rather than a character index because the
        // engine-side entity is indexed by profile.
        let entries: Vec<(usize, Option<crate::profiles::CharacterProfileIdx>, i16)> = mission_team
            .iter()
            .map(|&char_idx| {
                let (profile_idx, life_points) = campaign
                    .characters
                    .get(char_idx)
                    .map(|desc| (desc.character_profile_idx, desc.status.life_points))
                    .unwrap_or((None, 0));
                (char_idx, profile_idx, life_points)
            })
            .collect();

        const LIFEPOINTS_PC_X2: u32 = (crate::pc_status::LIFEPOINTS_PC as u32) << 1;

        for (i, (char_idx, profile_idx_opt, life_points)) in entries.iter().enumerate() {
            if i >= number_to_convert {
                // The "Place those peasants on a free beam-me" branch
                // is inactive, so extra
                // peasants past the convert count stay in the team
                // untouched here.  The trailing `ResetMissionTeam()`
                // below wipes the team list so they don't carry into
                // the new mission.
                break;
            }

            // The original game's peasant conversion uses
            // `rand() % (LIFEPOINTS_PC << 1) < life_points`; healthier
            // peasants survive into reservists, frailer ones die outright.
            let roll = crate::sim_rng::u32(
                sim,
                crate::sim_rng::RngSite::PeasantReservistSurvival,
                0..LIFEPOINTS_PC_X2,
            ) as i32;
            let campaign = &mut self.mission_domain.campaign;
            if roll < *life_points as i32 {
                campaign.move_to_reservists(*char_idx);
            } else {
                campaign.deeds.lost_members.insert(*char_idx);
                campaign.remove_from_gang(*char_idx);
            }

            if let Some(profile_idx) = profile_idx_opt {
                self.remove_pc_by_profile(*profile_idx);
            }
        }

        // Reset the mission team.
        let campaign = &mut self.mission_domain.campaign;
        campaign.reset_mission_team();
        // Credit `floor(number_to_convert / quotation)` blazons.
        if quotation != 0 {
            let credited = (number_to_convert as i32) / (quotation as i32);
            campaign.add_value(crate::campaign::CampaignValue::Blazon, credited);
        }
    }

    // ─── Read-only accessors for host-side code ────────────────────

    /// Win/loss tracking and mission metadata.  Host UI reads these
    /// flags to render the HUD / debrief / quit buttons.
    pub fn mission(&self) -> &MissionState {
        &self.mission_domain.state
    }

    /// Current mission's background map name (without extension), as
    /// set by the mission profile at level-load.
    pub fn mission_map_name(&self) -> &str {
        &self.mission_domain.state.map_name
    }

    /// Original-compatible engine tick clock. Unlike host/network timeline
    /// frames, this advances only when the hourglass completes a simulation
    /// tick according to the Original's freeze rules.
    pub fn simulation_tick(&self) -> SimulationTick {
        SimulationTick::new(self.control.frame_counter)
    }

    /// Raw legacy accessor for the Original's universal frame counter.
    /// New host/timeline code should use [`Self::simulation_tick`] so it cannot
    /// be mistaken for a lockstep or replay frame identity.
    pub fn frame_counter(&self) -> u32 {
        self.simulation_tick().number()
    }

    /// Sim-state portion of the sound system (source list + finished
    /// exclamation queue).  Host sound pipeline reads this when
    /// flushing sources.
    pub fn sound_sim(&self) -> &crate::sound::SoundSimState {
        &self.feedback.sound_sim
    }

    /// Read-only access to the sample-length lookup used for
    /// Loaded mission script (bytecode + VM).  `None` if scripts are
    /// disabled or the level has no script.  Host renderers and the
    /// console read the script VM state for inspection.
    pub fn mission_script(&self) -> Option<&MissionScript> {
        self.scripts.mission.as_ref()
    }

    /// True iff men-to-blazon conversion mode is active. Read by titbit
    /// rendering to suppress the per-PC
    /// WorkIcon while the conversion screen is up.
    pub fn is_men_to_blazon_conversion_mode(&self) -> bool {
        self.script_domains.mission_ui.men_to_blazon_conversion_mode
    }

    /// Number of temporary blazon highlights active on this frame.
    pub fn active_blinking_blazons(&self) -> u32 {
        self.script_domains
            .mission_ui
            .active_blinking_blazons(self.control.frame_counter)
    }

    /// Toggle the engine-owned men-to-blazon conversion mode. Read by the
    /// `IsMenToBlazonConversionMode` native and the
    /// blazon-bar recomputation during information-bar updates.
    pub(crate) fn set_men_to_blazon_conversion_mode(&mut self, enabled: bool) {
        self.script_domains.mission_ui.men_to_blazon_conversion_mode = enabled;
    }

    /// Run the mission script's `PostInitialize` hook once, then no-op.
    /// The serialized flag keeps both live play and rollback replay
    /// idempotent; [`EngineInner::perform_post_initialize`] owns the
    /// original post-refresh host boundary.
    pub(crate) fn run_post_initialize_if_needed(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        if !sim.config().script_enabled || self.script_domains.mission_ui.game_post_initialized {
            return;
        }
        // Original RHGame owns this latch, setting it before the optional
        // callback. A missing VM/function must not leave the game uninitialized.
        self.script_domains.mission_ui.game_post_initialized = true;
        if self.scripts.mission.is_none() {
            return;
        }

        let result = self
            .call_script_vm(
                sim,
                assets,
                ScriptVmKey::Global,
                "PostInitialize",
                &[],
                crate::natives::ScriptCallFrame::default(),
            )
            .map(|_| ());

        if let Err(e) = result {
            tracing::warn!("Script PostInitialize failed: {e}");
        }
    }

    /// Mutate a campaign value with the usual addition side effects; see
    /// [`MissionDomain::add_campaign_value`](super::state::MissionDomain::add_campaign_value).
    pub(crate) fn add_campaign_value(&mut self, name: crate::campaign::CampaignValue, amount: i32) {
        self.mission_domain.add_campaign_value(
            &mut self.feedback.pending_side_effects,
            self.control.frame_counter,
            name,
            amount,
        );
    }

    /// Force a campaign value with the usual assignment side effects.
    /// RANSOM emits the `CashWon` jingle when the new value is greater
    /// than the old one (and the universal frame counter has advanced
    /// past 0).
    #[cfg(test)]
    pub(crate) fn set_campaign_value(&mut self, name: crate::campaign::CampaignValue, value: i32) {
        let old = self.mission_domain.campaign.values[name];
        self.mission_domain.campaign.values[name] = value;
        Self::apply_value_set_side_effects(
            &mut self.feedback.pending_side_effects,
            self.control.frame_counter,
            name,
            old,
            value,
        );
    }

    #[cfg(test)]
    fn apply_value_set_side_effects(
        side_effects: &mut SideEffects,
        frame_counter: u32,
        name: crate::campaign::CampaignValue,
        old: i32,
        new: i32,
    ) {
        if name == crate::campaign::CampaignValue::Ransom && new > old && frame_counter > 0 {
            side_effects
                .sounds
                .push(SoundCommand::Jingle(crate::sound::Jingle::CashWon));
        }
    }

    /// The campaign owned by this live mission engine.
    pub fn campaign(&self) -> &crate::campaign::Campaign {
        &self.mission_domain.campaign
    }

    /// Has the given peasant display name already been registered on
    /// the campaign's no-duplicates list?  Read-only.
    pub fn is_peasant_name_registered(&self, name: &str) -> bool {
        self.mission_domain
            .campaign
            .is_peasant_name_registered(name)
    }

    /// Add a display name to the campaign's peasant-name dedupe list.
    /// Called once per peasant at level-load, before the mission
    /// begins ticking.
    pub(crate) fn register_peasant_name(&mut self, name: String) {
        self.mission_domain
            .campaign_mut()
            .register_peasant_name(name);
    }

    /// Explicitly replace campaign progress for the `CAMPAIGN` developer
    /// console command. Mission construction and teardown never use this.
    pub(crate) fn replace_campaign(&mut self, campaign: crate::campaign::Campaign) {
        self.mission_domain.campaign = campaign;
    }

    /// Consume a finished engine and return its one campaign allocation.
    pub(crate) fn into_campaign(self) -> crate::campaign::Campaign {
        self.mission_domain.campaign
    }

    /// Reset transient runtime state that isn't — or shouldn't be —
    /// carried across a save/load boundary.  Called by
    /// [`Engine::restore`](crate::engine::Engine::restore) right after
    /// overlaying the saved engine's fields, so the next tick starts
    /// with a clean slate regardless of what the pre-load session was
    /// doing (mid-drag selection, mid-zoom, mid-tick side-effect
    /// queue, …).  This is the engine-owned half of the post-load
    /// resynchronisation.
    pub(crate) fn post_load_fixups(&mut self, display: &mut HostDisplayState) {
        // Alt-hover vision cone selection is host-owned now — the host
        // wipes `host.selected_view_element` in `Host::post_load_reset`.
        // The selection ring animation phase is host-owned now and is
        // reset in `Host::post_load_reset` too.

        // Per-frame / per-tick scratch flags.
        self.script_domains.mission_ui.force_check = false;
        self.control.chorus_timer = 0;
        self.control.fast_forward = false;
        self.orders.pending_path_requests.clear();
        self.orders.failed_path_requests.clear();

        // Force a full redraw on the next frame — the background cache
        // from the pre-load session is no longer valid for the restored
        // camera/mission state.
        display.display_op = DisplayOpCode::Redraw;

        // Abort any mid-zoom state carried over from the save.  Run
        // here so the restored engine starts the next tick with a
        // clean zoom state, rather than relying on a host-driven
        // cache-validity hook.
        if self.is_zooming() {
            let bg = &mut self.feedback.cutscene_camera.display.background_transform;
            bg.zoom_to_up = false;
            bg.zoom_to_down = false;
            bg.required_zoom_up = false;
            bg.required_zoom_down = false;
            self.feedback.cutscene_camera.display.display_op = DisplayOpCode::NoBackgroundMove;
            self.feedback.cutscene_camera.zoom_init_done = false;
        }

        // Drop any mid-tick side-effect scratch (sounds, UI requests,
        // …) that was being built before the quick-load.  Normally
        // drained by `perform_hourglass`; this covers the partial-tick
        // case where the load pre-empted the drain.
        self.feedback.pending_side_effects = SideEffects::default();

        // Anonymous sequence-timer entries are tied to `SequenceManager`
        // state that was just replaced; the reloaded manager rebuilds
        // its own timer list as sequences resume.
        // TODO(original-parity): verify whether the original load path clears
        // these timers or reconstructs their remaining duration from sequences.
        // Preserve the established Rust save-load behavior until that is known.
        self.orders.timer_elements.clear();

        // Walk every PC and reconcile the loaded selection list
        // against the per-PC `interface_hidden` / `playable` /
        // life-points flags.  The HUD is immediate-mode and re-derives
        // every frame, so the only state that can drift is
        // `selected_hero_ids` itself — serde restored it as it was at
        // save time, but the per-PC `interface_hidden` / `playable`
        // flags also restored from disk may now be inconsistent with
        // the cached selection (e.g. a mid-recording quick-save where
        // the messenger had a pending unselect).  Drop any selected id
        // whose PC has had its portrait hidden or been made unplayable.
        self.players.seats[0]
            .selection
            .retain(|&id| match self.world.entities.get(id) {
                Some(crate::element::Entity::Pc(pc)) => {
                    !pc.pc.interface_hidden && pc.pc.playable && pc.pc.life_points > 0
                }
                _ => false,
            });
    }
}
