use super::*;

impl EngineInner {
    pub(super) fn execute_teleport(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        // Read destination + layer + sector off the
        // movement element, snap the actor there, and spawn
        // the two 5-star bursts (old → new) at feet-to-eyes.
        // The element's `sector` field is ignored; sector +
        // layer are re-derived from the destination via
        // `get_sector_screen_accessible`.  Only the
        // destination point is read off the element here;
        // `dest_layer` is kept as a fallback for the
        // new-side star burst when the validation step
        // gives up.
        let (dest, dest_layer) = {
            let elem = self.orders.sequence_manager.get_element(seq_id, elem_idx);
            match elem.map(|e| &e.data) {
                Some(crate::sequence::SequenceElementData::Movement {
                    destination, layer, ..
                }) => (Some(*destination), Some(*layer)),
                _ => (None, None),
            }
        };
        if let Some(dest) = dest {
            self.mission_domain.cheat_used_flags |= 0x0000_0001; // CHEAT_TELEPORT

            // `stop_owner` cleans up any in-flight
            // movement / active element before the teleport
            // so the actor doesn't resume pathing toward
            // its old destination on the next tick.
            self.stop_owner(owner, crate::sequence::SequencePriority::Normal);

            // Snapshot old position & whether this is a PC
            // before any mutation; also capture eyes/feet
            // points for the old-position star burst.
            let (old_pos, old_feet, old_eyes, is_pc) = {
                let entity = match self.get_entity(owner) {
                    Some(e) => e,
                    None => {
                        self.orders
                            .sequence_manager
                            .element_terminated(seq_id, elem_idx);
                        return;
                    }
                };
                let ed = entity.element_data();
                let feet = entity.compute_feet_point();
                let eyes = entity.compute_eyes_point(None);
                (
                    ed.position_map(),
                    feet,
                    eyes,
                    matches!(entity, crate::element::Entity::Pc(_)),
                )
            };

            let zero_teleport = (dest.x - old_pos.x).abs() < f32::EPSILON
                && (dest.y - old_pos.y).abs() < f32::EPSILON;

            // Helper: emit 5 UnconsciousStar titbits from
            // feet → eyes with the canonical phases.
            let emit_stars = |mgr: &mut crate::titbit::TitbitManager,
                              feet: crate::coordinates::WorldPoint3D,
                              eyes: crate::coordinates::WorldPoint3D,
                              layer: u16| {
                let feet = crate::coordinates::WorldPoint3D {
                    x: feet.x,
                    y: feet.y,
                    z: feet.z,
                };
                let eyes = crate::coordinates::WorldPoint3D {
                    x: eyes.x,
                    y: eyes.y,
                    z: eyes.z,
                };
                let inc = crate::coordinates::WorldPoint3D {
                    x: (eyes.x - feet.x) * 0.25,
                    y: (eyes.y - feet.y) * 0.25,
                    z: (eyes.z - feet.z) * 0.25,
                };
                let mut p = crate::coordinates::WorldPoint3D {
                    x: feet.x - 4.0,
                    y: feet.y - 4.0,
                    z: feet.z,
                };
                for &phase in &[4u16, 12, 20, 12, 4] {
                    mgr.add_titbit(
                        p,
                        layer,
                        crate::titbit::TitbitKind::UnconsciousStar,
                        crate::titbit::ElementHandle::INVALID,
                        phase,
                        crate::titbit::ElementHandle::INVALID,
                        false,
                        crate::titbit::INVALID_ID,
                        false,
                        None,
                        None,
                    );
                    p.x += inc.x;
                    p.y += inc.y;
                    p.z += inc.z;
                }
            };

            // The old-position star burst is gated by
            // `bstars = !set_teleport_stuff(position_map, 20)`.
            // `set_teleport_stuff(pt_old, 20)`:
            //   ret = (teleport_counter > 0);
            //   if position_before_teleport == position_map:
            //       return ret  // already snapshot, leave counter
            //   position_before_teleport = pt_old;
            //   max_teleport_counter = teleport_counter = 20;
            //   return ret;
            // `bstars` is `true` only when no prior
            // teleport-fade is active — a re-teleport
            // during the 20-frame fade window suppresses
            // the second star burst.  The render-side
            // hulk-rebuild that consumes `teleport_counter`
            // lives in `game_render.rs::render_entities_gpu`.
            const TELEPORT_FADE_FRAMES: u16 = 20;
            let mut bstars = true;
            if is_pc
                && let Some(entity) = self.world.entities.get_mut(owner)
                && let Some(pc) = entity.pc_data_mut()
            {
                let breturn = pc.teleport_counter > 0;
                if pc.position_before_teleport.x == old_pos.x
                    && pc.position_before_teleport.y == old_pos.y
                {
                    // Already snapshot at this position — keep
                    // the existing counter, return prior state.
                } else {
                    pc.position_before_teleport = old_pos;
                    pc.max_teleport_counter = TELEPORT_FADE_FRAMES;
                    pc.teleport_counter = TELEPORT_FADE_FRAMES;
                }
                bstars = !breturn;
            }
            if is_pc
                && !zero_teleport
                && bstars
                && let (Some(f), Some(e)) = (old_feet, old_eyes)
            {
                emit_stars(
                    &mut self.feedback.titbit_manager,
                    f,
                    e,
                    dest_layer.unwrap_or(0),
                );
            }

            // Probe the destination sector via
            // `get_sector_screen_accessible`, then nudge
            // the actor's move-box onto a walkable cell
            // with `find_authorized_position_toward`.
            // When either step fails the entire apply
            // block is skipped — the actor stays put but
            // the new-position star burst still fires.
            let probe = self.world.fast_grid.get_sector_screen_accessible(dest);
            let move_box = self
                .get_entity(owner)
                .map(|e| *e.position_iface().get_move_box());
            let validated = if let (Some(_sector_idx), Some(sector_number), Some(move_box)) =
                (probe.sector_idx, probe.sector, move_box)
            {
                let mut box_at = move_box.translated(dest);
                if self.world.fast_grid.find_authorized_position_toward(
                    &mut box_at,
                    dest,
                    probe.layer,
                ) {
                    let dest_pt = box_at.center();
                    let sector_handle =
                        crate::position_interface::SectorHandle::new(u16::from(sector_number));
                    Some((dest_pt, probe.layer, sector_handle, sector_number))
                } else {
                    None
                }
            } else {
                None
            };

            let final_dest_layer = if let Some(v) = validated.as_ref() {
                Some(v.1)
            } else {
                dest_layer
            };

            if let Some((final_dest, final_layer, final_sector_handle, final_sector_number)) =
                validated
            {
                // Apply new position + layer/sector and
                // re-resolve projection/material through the
                // same finalization path used by jump and
                // door/lift transitions.
                self.finalize_special_move_position(
                    assets,
                    owner,
                    super::special_motion::SpecialMovePosition::Map(final_dest),
                    Some(final_layer),
                    Some(u16::from(final_sector_number)),
                    Some(final_dest),
                    "script teleport",
                );

                if let Some(entity) = self.world.entities.get_mut(owner) {
                    entity.element_data_mut().set_sector(final_sector_handle);
                }

                // Landing in a lift sector snaps posture
                // / action-state: LIFT_LADDER →
                // (OnLadder, Waiting); LIFT_WALL →
                // (OnWall, Waiting); LIFT_STAIRS leaves
                // it alone.
                if final_sector_handle.is_some() {
                    let lift = self.get_sector_lift_type(final_sector_number);
                    match lift {
                        Some(crate::sector::LiftType::Ladder) => {
                            if let Some(entity) = self.world.entities.get_mut(owner) {
                                entity.set_posture(crate::element::Posture::OnLadder);
                                if let Some(actor) = entity.actor_data_mut() {
                                    actor.action_state = crate::element::ActionState::Waiting;
                                }
                            }
                        }
                        Some(crate::sector::LiftType::Wall) => {
                            if let Some(entity) = self.world.entities.get_mut(owner) {
                                entity.set_posture(crate::element::Posture::OnWall);
                                if let Some(actor) = entity.actor_data_mut() {
                                    actor.action_state = crate::element::ActionState::Waiting;
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // If this PC carries another PC or is
                // being carried, copy the new position /
                // layer / sector onto the partner so the
                // carry link stays synced after the
                // teleport.  Route partner snaps through the
                // same finalizer so obstacle/material are
                // refreshed too.
                if is_pc {
                    let (carried, carrier) = self
                        .get_entity(owner)
                        .map(|e| {
                            let pc = e.pc_data();
                            let human = e.human_data();
                            (pc.and_then(|pc| pc.carried), human.and_then(|h| h.carrier))
                        })
                        .unwrap_or((None, None));
                    for partner in [carried, carrier].into_iter().flatten() {
                        self.finalize_special_move_position(
                            assets,
                            partner,
                            super::special_motion::SpecialMovePosition::Map(final_dest),
                            Some(final_layer),
                            Some(u16::from(final_sector_number)),
                            Some(final_dest),
                            "script teleport carry partner",
                        );
                        if let Some(partner_entity) = self.get_entity_mut(partner) {
                            partner_entity
                                .element_data_mut()
                                .set_sector(final_sector_handle);
                        }
                    }
                }
            }

            // After a layer/sector swap, refresh
            // `update_opponents_jump_lines` for both the
            // teleporter and any carry partner that was
            // synced above.
            self.update_opponents_jump_lines(assets, owner);
            if is_pc {
                let (carried, carrier) = self
                    .get_entity(owner)
                    .map(|e| {
                        let pc = e.pc_data();
                        let human = e.human_data();
                        (pc.and_then(|pc| pc.carried), human.and_then(|h| h.carrier))
                    })
                    .unwrap_or((None, None));
                for partner in [carried, carrier].into_iter().flatten() {
                    self.update_opponents_jump_lines(assets, partner);
                }
            }

            // New-position star burst after the snap.
            // Gated by `is_pc && !zero_teleport &&
            // bstars` — the same hulk-fade suppression
            // as the old-side burst.  Fires regardless
            // of whether the position write happened.
            if is_pc && !zero_teleport && bstars {
                let (new_feet, new_eyes) = match self.get_entity(owner) {
                    Some(e) => (e.compute_feet_point(), e.compute_eyes_point(None)),
                    None => (None, None),
                };
                if let (Some(f), Some(e)) = (new_feet, new_eyes) {
                    emit_stars(
                        &mut self.feedback.titbit_manager,
                        f,
                        e,
                        final_dest_layer.unwrap_or(0),
                    );
                }
            }
        }
        self.orders
            .sequence_manager
            .element_terminated(seq_id, elem_idx);
        // `actor_wait` parks the actor in a low-priority
        // idle element after the teleport so the AI
        // re-enters its default loop instead of resuming
        // whatever command was running before.
        self.actor_wait(owner);
    }
}
