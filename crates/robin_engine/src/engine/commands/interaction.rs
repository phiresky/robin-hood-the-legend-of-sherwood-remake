//! Interaction construction after command preflight and marker recording.

use super::{recorded_ground_target_titbit_layer, recorded_interaction_quick_phase};
use crate::element::{Command, EntityId};
use crate::engine::EngineInner;
use crate::engine::TickCtx;
use crate::profiles::Action;
use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

impl EngineInner {
    pub(super) fn dispatch_target_interaction(
        &mut self,
        tcx: TickCtx<'_>,
        actor: &EntityId,
        target: &EntityId,
        command: &Command,
        running: &bool,
    ) {
        let recording_interaction = self.players.qa_recording_for.contains(actor);
        if recording_interaction
            && *command == Command::EnterSwordfight
            && self.get_entity(*target).is_some_and(|entity| {
                crate::engine::melee::is_vip_from_profile(entity, &tcx.assets.profile_manager)
            })
            && !self
                .get_entity(*actor)
                .and_then(|entity| entity.pc_data())
                .is_some_and(|pc| pc.robin)
        {
            self.apply_enter_swordfight(tcx, *actor, *target, *running);
            return;
        }
        // Macro recording: if `actor` is in the recording set
        // and a slot is armed, append this interaction as a step.
        if recording_interaction {
            let (pos, tgt_layer, tgt_is_pc, tgt_is_object, tgt_target_filter) = self
                .get_entity(*target)
                .map(|e| {
                    let target_filter = match e {
                        crate::element::Entity::Target(t) => Some(t.target.action_filter),
                        _ => None,
                    };
                    (
                        e.element_data().position_map(),
                        e.element_data().layer(),
                        e.pc_data().is_some(),
                        matches!(
                            e,
                            crate::element::Entity::Bonus(_)
                                | crate::element::Entity::Scroll(_)
                                | crate::element::Entity::Projectile(_)
                                | crate::element::Entity::Net(_)
                        ),
                        target_filter,
                    )
                })
                .expect("recorded interaction target passed strict preflight");
            let action = self
                .get_entity(*actor)
                .and_then(|e| e.pc_data())
                .map(|pc| pc.current_action)
                .expect("recorded interaction PC passed strict preflight");
            // Pick the QuickAction ordinal.  Priority:
            //   1. A command-authored Original phase (direct Bow,
            //      TakeCorpse, or climb-up-on-shoulders).
            //   2. `Command::Take` on an object target → Take.
            //   3. FX-target interaction → walk the target's
            //      filter ladder so levers, cut/handle/take
            //      targets, pay-targets, bow targets, etc. pick
            //      the per-filter icon instead of the
            //      action-bar default.
            //   4. Action-specific icon when the PC is in an
            //      armed action mode (bow, stone, etc.).
            //   5. Fallback `InteractPc` / `InteractNpc`.
            let fallback_quick = if tgt_is_pc {
                crate::titbit::QuickAction::InteractPc as u16
            } else {
                crate::titbit::QuickAction::InteractNpc as u16
            };
            let quick = if let Some(phase) = recorded_interaction_quick_phase(*command) {
                phase as u16
            } else if *command == Command::Take && tgt_is_object {
                crate::titbit::QuickAction::Take as u16
            } else if let Some(filter) = tgt_target_filter {
                let pc_char_profile = self
                    .get_entity(*actor)
                    .and_then(|e| e.pc_data())
                    .and_then(|pc| tcx.assets.profile_manager.get_character(pc.profile_index));
                let pc_has_search =
                    pc_char_profile.is_some_and(|p| p.has_contextual_action(Action::Search));
                let pc_is_vip = self
                    .get_entity(*actor)
                    .is_some_and(|e| self.is_entity_vip(tcx.assets, e));
                crate::engine::target_interaction::target_qa_titbit(
                    filter,
                    pc_has_search,
                    pc_is_vip,
                )
            } else {
                crate::macro_store::action_to_qa_frame(action).unwrap_or(fallback_quick)
            };
            // Drop any titbit still sitting in this QA slot before
            // we allocate a new one.
            let slot = self.players.qa_recording_slot;
            self.remove_quick_action_titbits_for(*actor, slot);
            // Register a QuickAction titbit on the target so
            // the renderer can look it up by id.
            let tgt_handle = crate::titbit::ElementHandle(target.index());
            let pc_handle = crate::titbit::ElementHandle(actor.index());
            let titbit_id = self.feedback.titbit_manager.add_titbit(
                // Seek-interaction construction passes a zero point: the
                // target supplier owns both the rendered position and
                // layer in Original.
                crate::coordinates::WorldPoint3D::ZERO,
                tgt_layer,
                crate::titbit::TitbitKind::QuickAction,
                tgt_handle,
                quick,
                pc_handle,
                *running,
                crate::titbit::INVALID_ID,
                true,
                Some(pos.y),
                Some(tgt_layer),
            );
            // Write the new titbit id into the slot.  Only
            // overwrite when the titbit manager returned a real
            // id, to avoid clobbering with INVALID.
            if let Some(tb) = titbit_id {
                self.players
                    .macro_store
                    .get_or_insert(*actor)
                    .set_slot_titbit(slot as usize, tb);
            }
        }
        if recording_interaction {
            if matches!(
                self.get_entity(*target),
                Some(crate::element::Entity::Target(_))
            ) && matches!(
                command,
                Command::SearchCmd
                    | Command::UseLever
                    | Command::HitTarget
                    | Command::HandleTarget
                    | Command::TakeTarget
                    | Command::Pay
            ) {
                let target_entity = self.get_entity(*target).expect("recorded target exists");
                let destination = target_entity.element_data().position_map();
                let sector = target_entity.element_data().sector();
                let layer = target_entity.element_data().layer();
                let turn_point = target_entity
                    .current_gameplay_point_map()
                    .expect("recorded target has an interaction point");
                let action = if *running {
                    crate::order::OrderType::RunningUpright
                } else if self
                    .get_entity(*actor)
                    .expect("recorded actor exists")
                    .element_data()
                    .posture()
                    == crate::element::Posture::Crouched
                {
                    crate::order::OrderType::WalkingCrouched
                } else {
                    crate::order::OrderType::WalkingUpright
                };
                self.replay_recorded_target_interaction(
                    tcx,
                    *actor,
                    *target,
                    *command,
                    destination,
                    sector,
                    layer,
                    action,
                    turn_point,
                );
            } else if *command == Command::EnterSwordfight {
                self.apply_enter_swordfight(tcx, *actor, *target, *running);
                return;
            } else {
                self.apply_interaction_with_seek(tcx, *actor, *target, *command, *running);
            }
            self.stop_recording_macro();
            return;
        }
        // Schema-9 records every resolved interaction under one
        // command shape, although the Original has several route
        // constructors. Commands resolved against target elements
        // use the target's ordinary click path,
        // which directly constructs movement rather than
        // constructing a seek interaction. Keep the command gate
        // aligned with target command selection so a malformed
        // command/target pairing does not silently acquire this
        // route.
        // ENTER_SWORDFIGHT is likewise unambiguous: it is only ever
        // resolved by a soldier click, whose route is the classical
        // sword seek (tolerance = the PC's own sword range) plus the
        // VIP, table-swordfight and cross-gate forks. The generic
        // Seek-interaction construction has no sword-range entry and
        // would seek at the 30-unit interaction default instead,
        // stopping the PC short of — or past — the opponent.
        if *command == Command::EnterSwordfight {
            self.apply_enter_swordfight(tcx, *actor, *target, *running);
        } else if matches!(
            command,
            Command::SearchCmd
                | Command::UseLever
                | Command::HitTarget
                | Command::HandleTarget
                | Command::TakeTarget
                | Command::Pay
        ) && matches!(
            self.get_entity(*target),
            Some(crate::element::Entity::Target(_))
        ) && !self.players.qa_recording_for.contains(actor)
        {
            if *running {
                self.actor_make_fast(tcx.sim, *actor);
            } else if self.apply_target_interaction_route(tcx, *actor, *target, *command, *running)
            {
                self.hero_speaking(
                    tcx.assets,
                    *actor,
                    crate::engine::melee::HERO_ACCEPT_COMMAND,
                );
            }
        } else {
            self.apply_interaction_with_seek(tcx, *actor, *target, *command, *running);
        }
    }

    pub(super) fn dispatch_ground_target(
        &mut self,
        tcx: TickCtx<'_>,
        actor: &EntityId,
        target_pos: &crate::coordinates::WorldPoint3D,
        command: &Command,
        target_field: &Field,
        titbit_layer: &u16,
    ) {
        let recording = self.players.qa_recording_for.contains(actor);
        if self.players.qa_recording_for.contains(actor) {
            // Each Original ground-target input handler authors its
            // dedicated phase directly; it does not derive the icon
            // from mutable actor action state.
            let quick = match command {
                Command::ThrowNet => crate::titbit::QuickAction::Net as u16,
                Command::ThrowWaspNest => crate::titbit::QuickAction::Wasp as u16,
                Command::ThrowPurse => crate::titbit::QuickAction::Purse as u16,
                Command::ThrowStone => crate::titbit::QuickAction::Stone as u16,
                _ => panic!("recorded ground-target command {command:?} has no Original QA titbit"),
            };
            // Drop any titbit still sitting in this QA slot.
            let slot = self.players.qa_recording_slot;
            self.remove_quick_action_titbits_for(*actor, slot);
            let pc_handle = crate::titbit::ElementHandle(actor.index());
            // The replay command retains the captured selected layer
            // needed by the thrown object. Original's marker layer is
            // authored separately: Purse and Net use literal zero,
            // while Wasp uses the selected layer.
            let marker_layer = recorded_ground_target_titbit_layer(*command, *titbit_layer);
            let titbit_pos = crate::coordinates::WorldPoint3D {
                x: target_pos.x,
                y: target_pos.y,
                z: target_pos.z,
            };
            let titbit_id = self.feedback.titbit_manager.add_titbit(
                titbit_pos,
                marker_layer,
                crate::titbit::TitbitKind::QuickAction,
                crate::titbit::ElementHandle::INVALID,
                quick,
                pc_handle,
                false,
                crate::titbit::INVALID_ID,
                true,
                None,
                None,
            );
            // Write the new titbit id into the slot.  Skip INVALID.
            if let Some(tb) = titbit_id {
                self.players
                    .macro_store
                    .get_or_insert(*actor)
                    .set_slot_titbit(slot as usize, tb);
            }
        }
        let mut elem = SequenceElement::new_generic(1, *command, Some(*actor));
        // The sequence field is the full 3D throw target (the
        // downstream `ThrowNet/Purse/WaspNest` tick arms read
        // the x/y and drop z, so the Point3D variant stays
        // compatible while preserving the true altitude for
        // any future consumer).
        elem.set_property(
            *target_field,
            FieldValue::Point3D {
                x: target_pos.x,
                y: target_pos.y,
                z: target_pos.z,
            },
        );
        // Purse/wasp/net ground-target handlers call
        // sequence-element launch in the original game. Keep their owner
        // instruction at the post-entity manager boundary.
        let mut seq = Sequence::new();
        seq.append_element(elem);
        self.launch_or_record_quick_action_sequence(tcx, *actor, seq);
        if recording {
            self.stop_recording_macro();
        }
    }
}
