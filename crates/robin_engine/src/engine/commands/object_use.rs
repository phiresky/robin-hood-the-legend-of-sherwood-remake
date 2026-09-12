//! Object pickup/use policy and ale-drop route execution.
//! The dispatcher invokes reachability checks before recording; execution comes after.

use crate::coordinates::MapPoint;
use crate::element::{Command, EntityId};
use crate::engine::{EngineInner, LevelAssets};
use crate::profiles::Action;
use crate::sequence::{
    Field, FieldValue, MoveFlags, Sequence, SequenceElement, SequenceElementData,
};

impl EngineInner {
    pub(super) fn dispatch_drop_ale(
        &mut self,
        actor: &EntityId,
        target_pos: &MapPoint,
        running: &bool,
        already_authorized: &bool,
        goal_override: &Option<(crate::sector::SectorNumber, u16)>,
        goal_sector_index_override: &Option<crate::fast_find_grid::SectorIndex>,
        recorded_gate_path: &Option<crate::gate::RecordedGatePath>,
    ) {
        if self.players.qa_recording_for.contains(actor) {
            // The original game's ale input handling gives the constructed
            // Seek→DropAle sequence to quick-action assignment, sends
            // STOP_RECORDING_MACRO, and does not launch it live. The
            // parent dispatcher's shared recording hook has already retained the
            // complete resolved route and installed its titbit.
            self.stop_recording_macro();
            return;
        }
        self.apply_drop_ale_at(
            *actor,
            *target_pos,
            *running,
            *already_authorized,
            *goal_override,
            *goal_sector_index_override,
            recorded_gate_path.clone(),
        );
    }

    pub(super) fn dispatch_drop_ammo(&mut self, pc_id: &EntityId, action_id: &u32, amount: &u32) {
        let mut elem = SequenceElement::new_generic(1, Command::DropAmmo, Some(*pc_id));
        elem.set_property(Field::ActionId, FieldValue::Integer(*action_id));
        elem.set_property(Field::Amount, FieldValue::Integer(*amount));
        // Ammo-drop messages use sequence-element launch, not a direct
        // actor instruction call.
        let mut seq = Sequence::new();
        seq.append_element(elem);
        self.launch_sequence(seq);
    }

    pub(super) fn dispatch_scroll_read(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        actor: &EntityId,
        target: &EntityId,
        running: &bool,
    ) {
        if self.players.qa_recording_for.contains(actor) {
            let Some(pos) = self
                .get_entity(*target)
                .map(|e| e.element_data().position_map())
            else {
                return;
            };
            let slot = self.players.qa_recording_slot;
            self.remove_quick_action_titbits_for(*actor, slot);
            let pc_handle = crate::titbit::ElementHandle(actor.index());
            let target_layer = self
                .get_entity(*target)
                .map(|e| e.element_data().layer())
                .unwrap_or(0);
            let titbit_id = self.feedback.titbit_manager.add_titbit(
                // Generic seek construction passes a zero point and lets the
                // antagonist supplier drive rendered placement.
                crate::coordinates::WorldPoint3D::ZERO,
                target_layer,
                crate::titbit::TitbitKind::QuickAction,
                crate::titbit::ElementHandle(target.index()),
                crate::titbit::QuickAction::Speak as u16,
                pc_handle,
                *running,
                crate::titbit::INVALID_ID,
                true,
                Some(pos.y),
                Some(target_layer),
            );
            if let Some(tb) = titbit_id {
                self.players
                    .macro_store
                    .get_or_insert(*actor)
                    .set_slot_titbit(slot as usize, tb);
            }
            self.stop_recording_macro();
            return;
        }
        self.apply_scroll_read_with_seek(sim, *actor, *target, *running);
    }

    pub(super) fn dispatch_self_ability(
        &mut self,
        assets: &LevelAssets,
        actor: &EntityId,
        command: &Command,
    ) {
        if self.players.qa_recording_for.contains(actor) {
            // The shared recorder already captured the step. Manual
            // QA recording stores this ability instead of applying it
            // to the live PC.
            self.stop_recording_macro();
            return;
        }
        if *command == Command::EnterCloak {
            self.try_enter_reusable_cloak(assets, *actor);
            return;
        }
        let elem = SequenceElement::new(1, *command, Some(*actor));
        // The corresponding Original input handlers use
        // sequence-element launch. Registration is immediate, but the
        // owner's instruction boundary belongs to the manager pass after
        // this frame's entity loop; do not interrupt its current
        // order before that final Execute tick.
        let mut seq = Sequence::new();
        seq.append_element(elem);
        self.launch_sequence(seq);
    }

    /// Is `target` an object-class entity whose click routes through
    /// the `find_authorized_position` pre-flight?
    ///
    /// Matches the Bonus / Scroll / Projectile / Net arms of
    /// `object_pickup_command`.
    pub(super) fn is_object_take_target(&self, target: EntityId) -> bool {
        matches!(
            self.get_entity(target),
            Some(
                crate::element::Entity::Bonus(_)
                    | crate::element::Entity::Scroll(_)
                    | crate::element::Entity::Projectile(_)
                    | crate::element::Entity::Net(_)
            )
        )
    }

    /// Pre-flight reachability check for object Take clicks.
    ///
    /// Translates the PC's move-box to the target's map position and
    /// calls `find_authorized_position` with the target's layer.  A
    /// `false` return tells the caller to silently no-op — neither
    /// launching the seek sequence nor installing the QA titbit.
    pub(super) fn object_take_reachable(&self, actor: EntityId, target: EntityId) -> bool {
        let Some(actor_entity) = self.get_entity(actor) else {
            return false;
        };
        let Some(target_entity) = self.get_entity(target) else {
            return false;
        };
        let move_box = actor_entity.position_iface().get_move_box();
        if !move_box.is_somewhere() {
            return false;
        }
        let tgt_pos = target_entity.position_iface().map_position();
        let tgt_layer = target_entity.element_data().layer();
        let mut box_at_target = move_box.translated(tgt_pos);
        self.world
            .fast_grid
            .find_authorized_position(&mut box_at_target, tgt_layer)
    }

    /// Resolve the concrete point/sector retained by the original game's ale-drop action
    /// movement element.
    ///
    /// This is deliberately shared by live launch and quick-action recording:
    /// Original constructs the same movement element in both cases and only
    /// then chooses between quick-action assignment and sequence launch.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_drop_ale_target(
        &self,
        actor: EntityId,
        target_pos: crate::coordinates::MapPoint,
        already_authorized: bool,
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
        goal_sector_index_override: Option<crate::fast_find_grid::SectorIndex>,
    ) -> Option<(
        crate::coordinates::MapPoint,
        Option<crate::position_interface::SectorHandle>,
        u16,
    )> {
        let move_box = self.get_entity(actor)?.position_iface().get_move_box();

        // The drop point's exact goal, sector, and layer come from the cursor,
        // not from the actor: take the
        // topmost sector under the point, resolve a patch overlay to the
        // sector it covers, and resolve a jump sector to the sector it sits
        // in. Jump sectors carry no sector number of their own, so reading the
        // number off the raw hit loses the goal entirely and the seek never
        // learns that it has to cross a gate.
        //
        assert_eq!(
            already_authorized,
            goal_override.is_some(),
            "resolved DropAle commands must carry both already_authorized and goal_override"
        );
        assert!(
            goal_sector_index_override.is_none() || goal_override.is_some(),
            "DropAle exact goal-sector identity requires a goal_override"
        );
        let (goal_sector, goal_layer) = if let Some((goal_sector, goal_layer)) = goal_override {
            assert!(
                goal_sector.get() >= 0,
                "DropAle goal_override has invalid public sector {goal_sector}"
            );
            let public_sector = u16::from(goal_sector);
            let mut sector = crate::position_interface::SectorHandle::new(public_sector)
                .unwrap_or_else(|| {
                    panic!("DropAle goal_override has invalid public sector {public_sector}")
                });
            if let Some(index) = goal_sector_index_override {
                let indexed_sector = self
                    .world
                    .fast_grid
                    .level
                    .sectors
                    .get(usize::from(index))
                    .unwrap_or_else(|| {
                        panic!(
                            "DropAle exact goal-sector arena {index} is outside the FastFindGrid sector table"
                        )
                    });
                assert_eq!(
                    indexed_sector.sector_number, goal_sector,
                    "DropAle exact goal-sector arena {index} has public sector {}, expected {goal_sector}",
                    indexed_sector.sector_number,
                );
                sector = sector.with_arena_index(index);
            }
            (Some(sector), goal_layer)
        } else {
            let reference = self
                .get_entity(actor)
                .map(|e| e.element_data().position_map())
                .unwrap_or(target_pos);
            let hit = self
                .world
                .fast_grid
                .get_sector_screen(target_pos, reference);
            let resolved = hit.sector_idx.map(|idx| {
                let sector = self
                    .world
                    .fast_grid
                    .level
                    .sectors
                    .get(usize::from(idx))
                    .unwrap_or_else(|| panic!("DropAle sector hit references missing arena {idx}"));
                if sector.sector_type.is_patch() || sector.sector_type.is_jump() {
                    let under_idx = sector.underlying_sector.unwrap_or_else(|| {
                        panic!("DropAle overlay sector arena {idx} has no underlying sector")
                    });
                    let under = self
                        .world
                        .fast_grid
                        .level
                        .sectors
                        .get(usize::from(under_idx))
                        .unwrap_or_else(|| {
                            panic!(
                                "DropAle overlay sector arena {idx} references missing underlying arena {under_idx}"
                            )
                        });
                    let handle = crate::position_interface::SectorHandle::new(u16::from(
                        under.sector_number,
                    ))
                    .unwrap_or_else(|| {
                        panic!(
                            "DropAle overlay sector arena {idx} resolved invalid underlying public sector {}",
                            under.sector_number
                        )
                    })
                    .with_arena_index(under_idx);
                    (handle, sector.layer)
                } else {
                    (
                        hit.sector_handle().unwrap_or_else(|| {
                            panic!("DropAle valid sector hit arena {idx} has no public handle")
                        }),
                        hit.layer,
                    )
                }
            });
            match resolved {
                Some((sector, layer)) => (Some(sector), layer),
                None => (hit.sector_handle(), hit.layer),
            }
        };

        if let Some(index) = goal_sector.and_then(|sector| sector.arena_index()) {
            let sector = self
                .world
                .fast_grid
                .level
                .sectors
                .get(usize::from(index))
                .unwrap_or_else(|| panic!("DropAle goal arena {index} disappeared"));
            if sector.sector_type.is_door()
                || (sector.sector_type.is_lift()
                    && sector
                        .lift_type
                        .is_some_and(crate::sector::LiftType::is_wall_or_ladder))
            {
                return None;
            }
        } else if goal_sector.is_some() {
            // TODO(parity-schema): legacy number-only DropAle commands cannot
            // identify which duplicate sector object supplied the target, so
            // they cannot authoritatively reproduce this rejection guard.
        }

        let mut destination_pos = target_pos;
        if !already_authorized && move_box.is_somewhere() {
            let mut box_at_target = move_box.translated(target_pos);
            if self
                .world
                .fast_grid
                .find_authorized_position(&mut box_at_target, goal_layer)
            {
                destination_pos = box_at_target.center();
            } else {
                tracing::warn!(
                    ?actor,
                    goal_layer,
                    target_x = target_pos.x,
                    target_y = target_pos.y,
                    "resolve_drop_ale_target: target move box has no authorized position"
                );
                return None;
            }
        }

        Some((destination_pos, goal_sector, goal_layer))
    }

    /// Build a `Seek(dest) → DropAle` compound sequence and launch
    /// it.
    ///
    pub(super) fn apply_drop_ale_at(
        &mut self,
        actor: EntityId,
        target_pos: crate::coordinates::MapPoint,
        running: bool,
        already_authorized: bool,
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
        goal_sector_index_override: Option<crate::fast_find_grid::SectorIndex>,
        recorded_gate_path: Option<crate::gate::RecordedGatePath>,
    ) {
        self.apply_drop_ale_at_with_recovery(
            actor,
            target_pos,
            running,
            false,
            already_authorized,
            goal_override,
            goal_sector_index_override,
            recorded_gate_path,
        );
    }

    /// Build the ordinary DropAle route, optionally retaining quick-action
    /// posture recovery in its post-seek sequence.
    pub(super) fn apply_drop_ale_at_with_recovery(
        &mut self,
        actor: EntityId,
        target_pos: crate::coordinates::MapPoint,
        running: bool,
        append_posture_recovery: bool,
        already_authorized: bool,
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
        goal_sector_index_override: Option<crate::fast_find_grid::SectorIndex>,
        recorded_gate_path: Option<crate::gate::RecordedGatePath>,
    ) {
        use crate::order::OrderType;

        let (posture, layer, action_distance) = match self.get_entity(actor) {
            Some(e) => {
                let action_distance = match e.sprite().action_distance(OrderType::DroppingAle) {
                    Ok(distance) => distance,
                    Err(err) => {
                        tracing::warn!(
                            ?actor,
                            error = %err,
                            "apply_drop_ale_at: missing DroppingAle action distance"
                        );
                        return;
                    }
                };
                (
                    e.element_data().posture(),
                    e.element_data().layer(),
                    action_distance,
                )
            }
            None => {
                tracing::warn!(
                    ?actor,
                    "DropAle actor disappeared before route construction"
                );
                return;
            }
        };

        // running → RunningUpright, else crouched → WalkingCrouched,
        // else WalkingUpright.
        let action_style = if running {
            OrderType::RunningUpright
        } else if posture == crate::element::Posture::Crouched {
            OrderType::WalkingCrouched
        } else {
            OrderType::WalkingUpright
        };

        let Some((destination_pos, goal_sector, goal_layer)) = self.resolve_drop_ale_target(
            actor,
            target_pos,
            already_authorized,
            goal_override,
            goal_sector_index_override,
        ) else {
            return;
        };

        tracing::trace!(
            ?actor,
            actor_sector = ?self.get_entity(actor).and_then(|e| e.element_data().sector()),
            actor_layer = layer,
            ?goal_sector,
            goal_layer,
            target_x = target_pos.x,
            target_y = target_pos.y,
            dest_x = destination_pos.x,
            dest_y = destination_pos.y,
            "apply_drop_ale_at: resolved drop goal"
        );

        let mut move_elem =
            SequenceElement::new_movement(1, Command::Seek, Some(actor), action_style);
        move_elem.recorded_gate_path = recorded_gate_path;
        move_elem.point_seek_route_provenance = if already_authorized {
            crate::sequence::PointSeekRouteProvenance::OriginalReplay
        } else {
            crate::sequence::PointSeekRouteProvenance::Live
        };
        if let SequenceElementData::Movement {
            destination,
            tolerance,
            flags,
            post_seek_sequence,
            sector,
            layer: elem_layer,
            ..
        } = &mut move_elem.data
        {
            *destination = destination_pos;
            *tolerance = action_distance;
            *flags |= MoveFlags::SEEK;
            *sector = goal_sector;
            *elem_layer = goal_layer;
            let mut post_seek = Sequence::new();
            post_seek.append_element(SequenceElement::new(1, Command::DropAle, Some(actor)));
            if append_posture_recovery {
                self.append_posture_recovery(actor, &mut post_seek);
            }
            *post_seek_sequence = Some(post_seek.into_post_seek());
        }

        let mut sequence = Sequence::new();
        sequence.append_element(move_elem);
        self.launch_sequence(sequence);
    }
}

/// Rewrite a focused object id so coin clicks target the whole
/// source purse when that purse is still standing.
///
/// Before launching the Take sequence, if the coin was ejected from
/// a not-yet-taken purse, route the click to the purse — the
/// follow-up pickup then runs the purse take handler on arrival and
/// sweeps every still-active sibling coin in one call via
/// [`EngineInner::take_purse`].  Loose coins (no source purse) and
/// coins whose source purse has already been taken pass through
/// unchanged and go through the base coin pickup.
pub fn coin_pickup_target(engine: &EngineInner, target_id: EntityId) -> EntityId {
    let Some(crate::element::Entity::Projectile(p)) = engine.get_entity(target_id) else {
        return target_id;
    };
    if p.object.object_type != crate::element::ObjectType::Coin {
        return target_id;
    }
    let Some(purse_id) = p.projectile.purse.source_purse else {
        return target_id;
    };
    // Purse missing / already-taken → stay on the coin.
    match engine.get_entity(purse_id) {
        Some(crate::element::Entity::Projectile(purse))
            if matches!(
                purse.object.object_type,
                crate::element::ObjectType::Purse | crate::element::ObjectType::BonusPurse
            ) && !purse.object.taken
                && purse.element.active =>
        {
            purse_id
        }
        _ => target_id,
    }
}

/// Object pickup gate.
///
/// Returns `true` when the given PC can pick up the given object right
/// now:
///
/// * `associated_action == NoAction` (scrolls, relics, amulets, ransom
///   bags, coins — anything that doesn't fill an ammo slot) → always
///   takable.
/// * PC has the associated action AND the legacy unsigned storage-left
///   calculation is nonzero → takable. This also admits loaded inventories
///   above the difficulty-adjusted maximum.
/// * Fallback for Eat bonuses: when the PC has Guzzle instead, the
///   bonus still picks up if the guzzle slot has room.
pub(in crate::engine) fn is_pc_takable(
    engine: &EngineInner,
    assets: &LevelAssets,
    object: &crate::element::Entity,
    pc_id: EntityId,
) -> bool {
    use crate::profiles::Action;

    let Some(obj) = object.object_data() else {
        return false;
    };
    // Amulet max-count gate — refuse any further amulet pickups
    // once the campaign's Amulets counter reaches the maximum.
    // Runs before the `NoAction → true` fast-path because amulets
    // themselves carry `Action::NoAction`.
    if obj.object_type == crate::element::ObjectType::BonusAmulet
        && let Some(campaign) = Some(&engine.mission_domain.campaign)
        && campaign.get_value(crate::campaign::CampaignValue::Amulets)
            >= crate::campaign::MAXIMUM_AMULETS_NUMBER
    {
        return false;
    }
    let assoc = obj.associated_action;
    if assoc == Action::NoAction {
        return true;
    }
    let Some(pc) = engine.get_entity(pc_id) else {
        return false;
    };
    let Some(pc_data) = pc.pc_data() else {
        return false;
    };
    let Some(profile) = assets
        .profile_manager
        .characters
        .get(usize::from(pc_data.profile_index))
    else {
        return false;
    };

    let difficulty = engine.control.sim_config.difficulty;

    // Resolve PC status to read current ammo.  Pulled lazily because
    // not every branch needs it (NoAction returns early above).
    let Some(pc_desc) = engine.pc_description_for_pc_data(pc_data) else {
        return false;
    };
    let status = &pc_desc.status;

    let storage_left_for = |action: Action| -> u16 {
        let max = crate::inventory::max_ammo_for_action(profile, action, difficulty);
        let current = status.get_ammo(action);
        // Object takeability stores maximum ammo minus current ammo
        // ammunition amount in an unsigned 32-bit value. The 16-bit operands promote to signed
        // int first, so an over-cap loaded inventory becomes a large unsigned
        // value rather than clamping to zero.
        max.wrapping_sub(current)
    };

    // `find_action_slot` already folds Eat→Guzzle, so the explicit
    // Guzzle fallback is unnecessary here.
    if crate::inventory::find_action_slot(profile, assoc).is_some() {
        return storage_left_for(assoc) > 0;
    }
    false
}

/// Click-to-pickup dispatch for an object-class entity.
///
/// Per-subclass behaviour:
///
/// * Net — landed nets, always takable (the net is never stored as
///   ammo directly; the action check lives upstream in
///   `is_object_focusable`).
/// * Coin — forwards to the source purse when the purse hasn't been
///   taken yet; when the purse is still live
///   [`coin_pickup_target`] rewrites the target to the purse id, so
///   the Take sequence lands on the whole purse instead of one coin.
/// * Bonus / Scroll / landed Projectile — base path: `Seek` to object,
///   `Take` on arrival.
///
/// Upstream focus checks (`engine::input::is_object_focusable`)
/// already gated everything we care about, so this helper is narrow:
/// apply the per-type takability gate and translate the focused
/// object into the `Take` command the caller feeds to
/// `apply_interaction_with_seek`.
///
/// Returns `None` when the entity isn't a pickup-style object, or
/// when the object isn't currently in a takable state (e.g. an
/// Invisible scroll, a flying projectile, a taken bonus, or a bonus
/// whose PC already has a full inventory slot for its action).
pub fn object_pickup_command(
    engine: &EngineInner,
    assets: &LevelAssets,
    target_id: EntityId,
    pc_id: EntityId,
) -> Option<Command> {
    use crate::element::{Entity, ObjectType};

    let entity = engine.get_entity(target_id)?;

    // Macro-record escape hatch: when recording AND the PC owns the
    // object's associated action, bypass the full-inventory takable
    // gate so the step gets captured into the macro (the replay will
    // re-check takability at firing time).  The cursor path mirrors
    // this in `engine::input::choose_object_cursor`.
    let macro_override = || -> bool {
        if !engine.is_recording_macro() {
            return false;
        }
        let Some(obj) = entity.object_data() else {
            return false;
        };
        let Some(pc) = engine.get_entity(pc_id).and_then(|e| e.pc_data()) else {
            return false;
        };
        assets
            .profile_manager
            .get_character(pc.profile_index)
            .is_some_and(|profile| profile.has_action(obj.associated_action))
    };

    match entity {
        // Net: skips the takable gate entirely; the action-ownership
        // check is handled upstream in `is_object_focusable`.
        Entity::Net(n) if !n.projectile.flying => Some(Command::Take),

        // Bonus items: route through `is_pc_takable` — a full
        // inventory slot means the click is a no-op unless the
        // macro-record escape hatch fires.
        Entity::Bonus(b) => (b.is_takable()
            && (is_pc_takable(engine, assets, entity, pc_id) || macro_override()))
        .then_some(Command::Take),

        // Scrolls: no associated action; takable is vacuously true
        // once status is Visible / Opened.
        Entity::Scroll(_) => {
            use crate::engine::scroll_reveal::ScrollStatus;
            matches!(
                engine.scroll_status(target_id),
                ScrollStatus::Visible | ScrollStatus::Opened
            )
            .then_some(Command::Take)
        }

        // Projectile (landed coin/purse/stone/arrow/etc.): per-type
        // filter (Apple/WaspNest/Wasp never focusable) + `is_pc_takable`
        // (with the same macro-record escape hatch).
        Entity::Projectile(p) if !p.projectile.flying && !p.object.taken => {
            match p.object.object_type {
                ObjectType::Apple
                | ObjectType::BonusApple
                | ObjectType::WaspNest
                | ObjectType::BonusWaspNest
                | ObjectType::Wasp => None,
                _ => (is_pc_takable(engine, assets, entity, pc_id) || macro_override())
                    .then_some(Command::Take),
            }
        }
        _ => None,
    }
}

/// Determine which Use command to launch on a target entity.
/// Shared by swordfight fallback and quick-action replay.
pub(super) fn determine_use_command(
    engine: &EngineInner,
    assets: &LevelAssets,
    pc_id: EntityId,
    target_id: EntityId,
) -> Option<Command> {
    let entity = engine.get_entity(target_id)?;

    // FX targets — walk the target's command-selection filter ladder.
    // Search / Lever / Money are gated on the PC's contextual
    // abilities and VIP flag.
    if let crate::element::Entity::Target(t) = entity {
        let pc_char_profile = engine
            .get_entity(pc_id)
            .and_then(|e| e.pc_data())
            .and_then(|pc| assets.profile_manager.get_character(pc.profile_index));
        let pc_has_search =
            pc_char_profile.is_some_and(|p| p.has_contextual_action(Action::Search));
        let pc_has_lever = pc_char_profile.is_some_and(|p| p.has_contextual_action(Action::Lever));
        let pc_is_vip = engine
            .get_entity(pc_id)
            .is_some_and(|e| engine.is_entity_vip(assets, e));
        return crate::engine::target_interaction::target_use_command(
            t.target.action_filter,
            pc_has_search,
            pc_has_lever,
            pc_is_vip,
        );
    }

    // Object-class targets (Net, Bonus, Scroll, landed Projectile)
    // route through the shared per-type dispatch.
    if let Some(cmd) = object_pickup_command(engine, assets, target_id, pc_id) {
        return Some(cmd);
    }

    // Scroll / Bonus / landed Projectile pickup.
    // `is_object_focusable(Focus::Use)` already gated status / focus.
    if let crate::element::Entity::Scroll(_) = entity {
        return Some(Command::Take);
    }
    if let crate::element::Entity::Bonus(_) = entity {
        return Some(Command::Take);
    }
    if let crate::element::Entity::Projectile(p) = entity
        && !p.projectile.flying
    {
        return Some(Command::Take);
    }

    let is_dead = entity.is_dead();
    let posture = entity.element_data().posture();
    let is_unconscious = entity.human_data().is_some_and(|h| h.unconscious);
    let is_tied = posture == crate::element::Posture::Tied;

    // PC override fires before the human fallback.  When the target
    // PC is in HelpingToClimb posture and the selector PC has Jump,
    // dispatch the climb-up-on-shoulders sequence.
    // `is_entity_focusable(Focus::Use)` already gates on
    // `posture == HelpingToClimb && has_jump && !selector_swordfighting`
    // (engine/input.rs:508-524).
    if matches!(entity, crate::element::Entity::Pc(_))
        && posture == crate::element::Posture::HelpingToClimb
    {
        if engine.selected_pc_has_contextual_action(
            assets,
            Some(pc_id),
            crate::profiles::Action::Jump,
        ) {
            return Some(Command::ClimbUpOnShoulders);
        }
        return None;
    }

    // Pay beggar — alive, conscious beggar civilian whose VIP
    // selector has enough ransom.  Silently no-op when
    // ransom < BEGGAR_SALARY, even though the focus and cursor still
    // light up (PayNo).  The ransom check therefore lives here, not
    // in `is_entity_focusable`.
    if !is_dead
        && !is_unconscious
        && posture != crate::element::Posture::Carried
        && matches!(entity, crate::element::Entity::Civilian(c)
            if c.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar
                && c.npc.attached_scroll.is_none())
    {
        let ransom = Some(&engine.mission_domain.campaign)
            .map(|c| c.get_value(crate::campaign::CampaignValue::Ransom))
            .unwrap_or(0);
        if ransom >= crate::engine::BEGGAR_SALARY {
            return Some(Command::Pay);
        }
        return None;
    }

    if engine.control.sim_config.enable_unbinding && is_tied && entity.is_npc() && !is_dead {
        let npc_money = match entity {
            crate::element::Entity::Soldier(s) => s.npc.money,
            crate::element::Entity::Civilian(c) => c.npc.money,
            _ => unreachable!("NPC human interaction target must be soldier or civilian"),
        };
        if npc_money != 0
            && engine.selected_pc_has_contextual_action(
                assets,
                Some(pc_id),
                crate::profiles::Action::Search,
            )
            && (!engine.is_entity_vip(assets, entity)
                || engine
                    .get_entity(pc_id)
                    .and_then(|selected| selected.pc_data())
                    .is_some_and(|pc| pc.robin))
        {
            return Some(Command::SearchCmd);
        }
        if engine.selected_pc_has_contextual_action(
            assets,
            Some(pc_id),
            crate::profiles::Action::Tie,
        ) {
            return Some(Command::Untie);
        }
    }

    if is_dead {
        return Some(Command::SearchCmd);
    }
    if !is_dead && !is_unconscious && posture == crate::element::Posture::Lying {
        return Some(Command::SearchCmd);
    }

    // Wake-Up arm: target and selected PC must share an allegiance,
    // and the selector must have Resuscitate.
    if is_unconscious
        && engine.selected_pc_has_contextual_action(
            assets,
            Some(pc_id),
            crate::profiles::Action::Resuscitate,
        )
    {
        let selector_camp = engine
            .get_entity(pc_id)
            .unwrap_or_else(|| panic!("selected PC {pc_id:?} disappeared during WakeUp dispatch"))
            .camp();
        if entity.is_human() && engine.camps_are_allied(entity.camp(), selector_camp) {
            return Some(Command::WakeUp);
        }
    }

    // Take-Corpse arm: `(is_dead || is_unconscious) &&
    // (LittleJohnCarry || FarmerCarry) && !is_heavy`.  Ordered before
    // Tie so an unconscious soldier the PC can carry doesn't get
    // mis-routed to Tie when the PC lacks the Tie ability.
    if (is_unconscious || is_dead)
        && posture != crate::element::Posture::Carried
        && !is_tied
        && engine.selected_pc_can_carry(assets, Some(pc_id))
    {
        let is_heavy = match entity {
            crate::element::Entity::Soldier(s) => assets
                .profile_manager
                .get_soldier(s.soldier.soldier_profile_index)
                .map(|p| p.heavy)
                .unwrap_or(false),
            _ => false,
        };
        if !is_heavy {
            return Some(Command::TakeCorpse);
        }
    }

    // Tie arm: `is_unconscious && posture == Lying && selector has Tie`.
    // Without the carry path or the Tie ability, the click no-ops.
    if is_unconscious
        && !is_tied
        && posture != crate::element::Posture::Carried
        && engine.selected_pc_has_contextual_action(
            assets,
            Some(pc_id),
            crate::profiles::Action::Tie,
        )
    {
        return Some(Command::TieCmd);
    }
    None
}

/// Per-object Take seek tolerance = `radius + 15`.
///
/// Per-subclass radius:
///   * Ale → 5 (tolerance 20)
///   * Purse → 7 (tolerance 22)
///   * Coin → 3 (tolerance 18)
///   * Net → 40 uncrumpled / 10 crumpled (55 / 25)
///   * Everything else (plain bonus / scroll / arrow / stone / cape /
///     apple / wasp / waspnest) → 0 (tolerance 15).
pub(super) fn take_seek_tolerance(entity: &crate::element::Entity) -> f32 {
    use crate::element::{Entity, ObjectType};
    let radius: f32 = match entity {
        Entity::Bonus(b) => match b.object.object_type {
            ObjectType::Ale => 5.0,
            ObjectType::Purse => 7.0,
            _ => 0.0,
        },
        Entity::Projectile(p) => match p.object.object_type {
            ObjectType::Ale => 5.0,
            ObjectType::Purse => 7.0,
            ObjectType::Coin => 3.0,
            _ => 0.0,
        },
        Entity::Net(n) => {
            if n.net.crumpled {
                10.0
            } else {
                40.0
            }
        }
        _ => 0.0,
    };
    radius + 15.0
}
