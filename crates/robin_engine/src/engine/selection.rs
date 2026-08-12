//! PC selection helpers.

use super::*;
use crate::element::{ActionState, Command, Entity, EntityId};
use crate::profiles::Action;
use crate::sequence::{SequenceElement, SequencePriority};

/// Combined "stand up / crouch" widget state. Returned from
/// [`EngineInner::retrieve_stature`] to drive the up / down arrow widgets on
/// the status bar.
///
/// - `None`: neither posture applicable (PC climbing / in a building / no
///   selection).
/// - `Up`: at least one selected PC is upright — up-arrow active.
/// - `Down`: at least one selected PC is crouched / lying — down-arrow active.
/// - `Both`: mix of postures — both arrows active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stature {
    None,
    Up,
    Down,
    Both,
}

impl EngineInner {
    // ─── Selection helpers ────────────────────────────────────────

    /// Check whether a PC entity is selectable (alive, active, playable).
    ///
    /// Conditions:
    /// - `(active || in a building sector) && (!vip || !men-to-blazon mode) && !out-of-order && playable`
    /// - "out of order" = dead / unconscious / stuck under net / posture
    ///   tied or carried / (PC && in coma).
    pub fn is_pc_selectable(&self, assets: &LevelAssets, id: EntityId) -> bool {
        let Some(Entity::Pc(pc)) = self.get_entity(id) else {
            return false;
        };
        if !pc.pc.playable {
            return false;
        }

        // Out of order: dead / unconscious / stuck / tied / carried / coma.
        let is_dead = pc.pc.life_points == 0;
        let is_stuck_under_net = pc.human.stuck_under_nets_counter > 0;
        let posture = pc.element.posture;
        let bad_posture = matches!(
            posture,
            crate::element::Posture::Tied | crate::element::Posture::Carried
        );
        let in_coma = self
            .pc_description_for_pc_data(&pc.pc)
            .map(|desc| desc.status.in_coma)
            .unwrap_or(false);
        if is_dead || pc.human.unconscious || is_stuck_under_net || bad_posture || in_coma {
            return false;
        }

        // Original RHElementActorPC::IsSelectable asks the PC's current
        // position-interface sector directly.  A point query is not
        // equivalent when building sectors overlap another sector at the
        // same map coordinate, which can make an inactive indoor PC appear
        // unselectable.
        let in_building = self.entity_building_sector(pc.element.sector()).is_some();
        if !(pc.element.active || in_building) {
            return false;
        }

        // A VIP is not selectable while the Men-To-Blazon cutscene is active.
        let is_vip = Some(&self.mission_domain.campaign)
            .and(assets.profile_manager.get_character(pc.pc.profile_index))
            .map(|p| p.vip)
            .unwrap_or(false);
        if is_vip {
            if self.script_domains.mission_ui.men_to_blazon_conversion_mode {
                return false;
            }
        }

        true
    }

    /// Test whether a PC is either climbing (`OnWall`/`OnLadder`) or
    /// standing inside a building sector.
    ///
    /// Callers are [`retrieve_stature`](Self::retrieve_stature) and the
    /// `AddSequenceWithSeek` overloads. The latter aren't ported yet, so this
    /// helper is currently unused but kept pure so that porting those sites is
    /// a drop-in.
    pub fn is_pc_climbing_or_in_building(&self, id: EntityId) -> bool {
        let Some(entity) = self.get_entity(id) else {
            return false;
        };
        let posture = entity.element_data().posture;
        if matches!(
            posture,
            crate::element::Posture::OnWall | crate::element::Posture::OnLadder
        ) {
            return true;
        }
        let elem = entity.element_data();
        self.entity_building_sector(elem.sector()).is_some() || elem.is_in_door_transit()
    }

    /// Select a single PC, optionally adding to the current selection.
    ///
    /// When `multi_select` is false, clears the existing selection first.
    ///
    /// Robin is always inserted at the front so `selected_pc_ids.first()`
    /// consistently resolves to Robin when he is in the selection.
    ///
    /// `speak` fires the `HERO_SELECT` exclamation after a successful add.
    pub(crate) fn select_pc(
        &mut self,
        assets: &LevelAssets,
        seat: usize,
        id: EntityId,
        multi_select: bool,
        speak: bool,
    ) {
        if !self.is_pc_selectable(assets, id) {
            return;
        }
        if !multi_select {
            let old_selection = self.players.seats[seat].selection.clone();
            for old_id in old_selection {
                if let Some(Entity::Pc(pc)) = self.get_entity_mut(old_id) {
                    pc.pc.portrait.open = false;
                }
            }
            self.players.seats[seat].selection.clear();
        }
        if !self.players.seats[seat].selection.contains(&id) {
            let is_robin = matches!(
                self.get_entity(id),
                Some(Entity::Pc(pc)) if pc.pc.robin,
            );
            if is_robin {
                self.players.seats[seat].selection.insert(0, id);
            } else {
                self.players.seats[seat].selection.push(id);
            }
        }
        // Sherwood-only: clear the `interface_hidden` flag on the selected PC
        // so the per-PC interface panel shows again. Non-Sherwood paths drive
        // portrait-widget open state which the HUD derives from live
        // selection.
        if self.is_sherwood(&assets.profile_manager)
            && let Some(Entity::Pc(pc)) = self.get_entity_mut(id)
        {
            pc.pc.interface_hidden = false;
        }
        if let Some(Entity::Pc(pc)) = self.get_entity_mut(id) {
            pc.pc.portrait.open = !pc.pc.portrait.burned;
        }
        if speak {
            self.hero_speaking(assets, id, crate::engine::melee::HERO_SELECT);
        }
        self.apply_post_select_action_fanout(assets, seat);
    }

    /// Post-selection bookkeeping. With >1 PCs selected, each PC's current
    /// action is cleared. With exactly one selected PC, Original forwards
    /// `MSG_SELECT_ACTION(pc, pc->GetCurrentAction())` even when the stored
    /// action did not change. That restitution still runs SelectAction's
    /// stop-in-place and per-action entry hooks, so it cannot be reduced to
    /// retaining `current_action` in place.
    fn apply_post_select_action_fanout(&mut self, assets: &LevelAssets, seat: usize) {
        if self.players.seats[seat].selection.len() > 1 {
            let ids = self.players.seats[seat].selection.clone();
            for id in ids {
                self.unselect_action(id);
            }
            self.players.seats[seat].selected_action = Action::NoAction;
        } else if let Some(&id) = self.players.seats[seat].selection.first() {
            let action = self
                .get_entity(id)
                .and_then(|entity| entity.pc_data())
                .map(|pc| pc.current_action)
                .unwrap_or(Action::NoAction);
            self.set_pc_action_from_message(assets, seat, id, action);
        }
    }

    /// Toggle a PC in/out of the current selection (Ctrl+click).
    pub(crate) fn toggle_pc_selection(&mut self, assets: &LevelAssets, seat: usize, id: EntityId) {
        if let Some(pos) = self.players.seats[seat]
            .selection
            .iter()
            .position(|&x| x == id)
        {
            self.players.seats[seat].selection.remove(pos);
            if self.players.seats[seat].selection.is_empty() {
                self.players.seats[seat].selected_action = Action::NoAction;
            }
            if let Some(Entity::Pc(pc)) = self.get_entity_mut(id) {
                pc.pc.portrait.open = false;
            }
        } else if self.is_pc_selectable(assets, id) {
            self.players.seats[seat].selection.push(id);
            if let Some(Entity::Pc(pc)) = self.get_entity_mut(id) {
                pc.pc.portrait.open = !pc.pc.portrait.burned;
            }
            // Ctrl-click addition routes through SelectPC in Original and
            // therefore performs the same post-selection action fanout.
            self.apply_post_select_action_fanout(assets, seat);
        }
    }

    /// Select all playable PCs. Robin is placed at the head of the list;
    /// everyone else preserves `pc_ids` order.
    pub(crate) fn select_all_pcs(&mut self, assets: &LevelAssets, seat: usize) {
        for pc_id in self.world.pc_ids.clone() {
            if let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id) {
                pc.pc.portrait.open = false;
            }
        }
        self.players.seats[seat].selection.clear();
        self.players.seats[seat].selected_action = Action::NoAction;
        let pc_ids: Vec<EntityId> = self.world.pc_ids.clone();
        for &pc_id in &pc_ids {
            if !self.is_pc_selectable(assets, pc_id) {
                continue;
            }
            let is_robin = matches!(
                self.get_entity(pc_id),
                Some(Entity::Pc(pc)) if pc.pc.robin,
            );
            if is_robin {
                self.players.seats[seat].selection.insert(0, pc_id);
            } else {
                self.players.seats[seat].selection.push(pc_id);
            }
            if let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id) {
                pc.pc.portrait.open = !pc.pc.portrait.burned;
            }
        }
        // Sherwood: clear the per-PC `interface_hidden` flag on every
        // selectable PC so each PC's HQ interface re-shows.
        if self.is_sherwood(&assets.profile_manager) {
            let selected = self.players.seats[seat].selection.clone();
            for id in selected {
                if let Some(Entity::Pc(pc)) = self.get_entity_mut(id) {
                    pc.pc.interface_hidden = false;
                }
            }
        }
        self.apply_post_select_action_fanout(assets, seat);
    }

    /// Clear the selection.
    pub(crate) fn unselect_all_pcs(&mut self, seat: usize) {
        let old_selection = self.players.seats[seat].selection.clone();
        for id in old_selection {
            if let Some(Entity::Pc(pc)) = self.get_entity_mut(id) {
                pc.pc.portrait.open = false;
            }
        }
        self.players.seats[seat].selection.clear();
        self.players.seats[seat].selected_action = Action::NoAction;
    }

    /// Remove a single PC from the selection.
    ///
    /// Called from the tick messenger drain on `PcMessage::UnselectCharacter`
    /// with a non-zero value (dying / downed PCs kick themselves out of the
    /// selection), and from `PcMessage::DisableCharacter`.
    pub(crate) fn unselect_single_pc(&mut self, id: EntityId) {
        let was_last = self.players.seats[0].selection.len() == 1
            && self.players.seats[0].selection.contains(&id);
        self.players.seats[0].selection.retain(|&x| x != id);
        if was_last {
            self.players.seats[0].selected_action = Action::NoAction;
        }
        if let Some(Entity::Pc(pc)) = self.get_entity_mut(id) {
            pc.pc.portrait.open = false;
        }
    }

    /// Save the current action on each selected PC.
    ///
    /// The ctrl key is the "move during action" modifier; saving lets
    /// ctrl-release restore the action that was active when ctrl was pressed.
    pub(crate) fn save_action_for_selected_pcs(&mut self, seat: usize) {
        let ids = self.players.seats[seat].selection.clone();
        for id in ids {
            if let Some(Entity::Pc(pc)) = self.get_entity_mut(id) {
                pc.pc.saved_action = pc.pc.current_action;
            }
        }
    }

    /// Temporarily disable every action on a specific PC or every selected
    /// PC. `target_pc = None` fans out over the selection; `Some(id)` targets
    /// a single PC.
    pub(crate) fn apply_disable_all_actions_temp(
        &mut self,
        seat: usize,
        target_pc: Option<EntityId>,
    ) {
        let targets: Vec<EntityId> = match target_pc {
            None => self.players.seats[seat].selection.clone(),
            Some(id) => vec![id],
        };
        for id in targets {
            let Some(Entity::Pc(pc)) = self.get_entity_mut(id) else {
                continue;
            };
            pc.pc.disable_all_actions_temp();
        }
    }

    /// Re-enable temporary actions on a specific PC or every selected PC.
    pub(crate) fn apply_enable_all_actions_temp(
        &mut self,
        assets: &LevelAssets,
        seat: usize,
        target_pc: Option<EntityId>,
    ) {
        let targets: Vec<EntityId> = match target_pc {
            None => self.players.seats[seat].selection.clone(),
            Some(id) => vec![id],
        };
        for id in targets {
            self.enable_pc_actions_temp(assets, seat, id);
        }
    }

    /// Re-enable one PC's temporary action mask and synchronously forward a
    /// saved authored action through Original's selected-PC action path.
    pub(crate) fn enable_pc_actions_temp(
        &mut self,
        assets: &LevelAssets,
        seat: usize,
        pc_id: EntityId,
    ) {
        let (profile_idx, is_swordfighting, playable) = match self.get_entity(pc_id) {
            Some(Entity::Pc(pc)) => (
                pc.pc.profile_index,
                !pc.human.opponents.is_empty(),
                pc.pc.playable,
            ),
            _ => return,
        };
        if is_swordfighting || !playable {
            return;
        }
        let actions = assets
            .profile_manager
            .get_character(profile_idx)
            .unwrap_or_else(|| {
                panic!("PC {pc_id:?} requires missing character profile {profile_idx:?}")
            })
            .actions;
        let restore_action = self
            .get_entity_mut(pc_id)
            .and_then(Entity::pc_data_mut)
            .and_then(|pc| pc.enable_all_actions_temp(is_swordfighting, &actions));

        // `EnableAllActionsTemp` forwards MSG_SELECT_ACTION with this PC as
        // the message target. When that target belongs to a multi-selection,
        // RHMessenger first reduces the selection to this PC; the engine then
        // performs the ordinary selected-action side effects for it alone.
        if let Some(action) = restore_action
            && self.players.seats[seat].selection.contains(&pc_id)
        {
            self.set_pc_action_from_message(assets, seat, pc_id, action);
        }
    }

    /// Set the blinking flag on every titbit the PC owns in the given QA
    /// slot. No-op if the PC has no titbit for that slot or the PC isn't
    /// known.
    pub(crate) fn set_blinking_for_slot(&mut self, pc_id: EntityId, slot: usize) {
        let Some(state) = self.players.macro_store.get(pc_id) else {
            return;
        };
        let Some(titbit_id) = state.get_slot_titbit(slot) else {
            return;
        };
        self.feedback.titbit_manager.set_blinking(titbit_id, true);
    }

    /// Auto-select the highest-priority playable PC and center the camera on
    /// them.
    ///
    /// After level load, pick the playable PC with the highest profile
    /// priority (Robin has priority 10) and center + select.
    pub(crate) fn select_highest_priority_pc(&mut self, assets: &LevelAssets, seat: usize) {
        if false {
            return;
        }
        let profiles = assets.profile_manager.clone();

        let mut best: Option<(EntityId, u16)> = None;
        for &pc_id in &self.world.pc_ids {
            let Some(Entity::Pc(pc)) = self.get_entity(pc_id) else {
                continue;
            };
            if !pc.pc.playable {
                continue;
            }
            let priority = match profiles.get_character(pc.pc.profile_index) {
                Some(p) => p.priority,
                None => continue,
            };
            if best.is_none_or(|(_, best_pri)| priority > best_pri) {
                best = Some((pc_id, priority));
            }
        }

        if let Some((pc_id, _)) = best {
            let pos = match self.get_entity(pc_id) {
                Some(Entity::Pc(pc)) => pc.element.position_map(),
                _ => return,
            };
            self.center_on_point(seat, pos);
            // Initialize-from-mission path selects without the speak flag.
            self.select_pc(assets, seat, pc_id, false, false);
        }
    }

    /// Reset a PC from coma state (amulet revival).
    ///
    /// - Clears `in_coma` flag in campaign PcStatus
    /// - Clears concussion_of_the_brain
    /// - Restores life_points to 50
    /// - Wakes the PC up (clears unconscious, sets posture)
    ///
    /// Called when the player clicks the amulet button on a coma portrait.
    pub(crate) fn reset_coma(&mut self, assets: &LevelAssets, pc_id: EntityId) {
        let status_idx = match self.get_entity(pc_id) {
            Some(Entity::Pc(pc)) => self.pc_description_index_for_pc_data(&pc.pc),
            _ => return,
        };
        let Some(status_idx) = status_idx else { return };

        tracing::info!(entity = ?pc_id, "reset_coma — reviving from coma");

        if let Some(Entity::Pc(pc)) = self.world.entities.get_mut(pc_id) {
            pc.pc.portrait.burned = false;
            pc.pc.portrait.open = false;
        }

        // Clear coma in campaign status
        if let Some(campaign) = Some(&mut self.mission_domain.campaign)
            && let Some(desc) = campaign.characters.get_mut(status_idx)
        {
            desc.status.in_coma = false;
        }

        // Order:
        //   set concussion to 0
        //   hero-speak HERO_USE_LEAF_CLOVER
        //   set life points to 50
        //   wait
        // The hero-speak fires *before* the life write — keep that order so
        // any speech queueing observes the pre-write state.
        if let Some(Entity::Pc(pc)) = self.world.entities.get_mut(pc_id) {
            pc.human.concussion_of_the_brain = 0;
            pc.human.unconscious = false;
            pc.element.set_posture(crate::element::Posture::Upright);
        }

        self.hero_speaking(assets, pc_id, crate::engine::melee::HERO_USE_LEAF_CLOVER);

        // Route the life write through `combat::set_life_points` so the
        // clamp + invulnerable + sherwood guards live in one place, even for
        // the heal path. The widget side relies on the per-frame HUD refresh.
        if let Some(Entity::Pc(pc)) = self.world.entities.get_mut(pc_id) {
            crate::combat::set_life_points(
                &mut pc.pc.life_points,
                50,
                false,
                crate::combat::LIFEPOINTS_PC,
                false,
            );
        }

        self.actor_wait(pc_id);
    }

    /// Request a reinforcement replacement for a dead PC (trumpet click).
    ///
    /// Posts `PcMessage::SendReinforcement` and disables the trumpet flag so
    /// the player can't queue a second replacement while the first is
    /// pending. The message handler in `tick.rs` sets
    /// `time_till_reinforcement` and plays the `NewPeasantCalled` jingle;
    /// the spawn itself happens when the cooldown hits zero.
    pub(crate) fn request_reinforcement(&mut self, pc_id: crate::element::EntityId) {
        use crate::messenger::{Message, PcMessage};

        // Ignore the click if there's no trumpet to send — e.g. replay
        // race or a revive-plus-die sequence that cleared the flag.
        let has_trumpet = matches!(
            self.get_entity(pc_id),
            Some(crate::element::Entity::Pc(pc)) if pc.pc.trumpet_enabled,
        );
        if !has_trumpet {
            return;
        }
        if let Some(crate::element::Entity::Pc(pc)) = self.get_entity_mut(pc_id) {
            pc.pc.trumpet_enabled = false;
        }

        self.orders
            .messenger
            .send(Message::pc(PcMessage::SendReinforcement, Some(pc_id)));
    }

    /// Get the world position of a PC's guard entity (for CenterOn).
    ///
    /// Returns the guard's position if the PC has an assigned guard.
    pub fn get_guard_position(&self, pc_id: EntityId) -> Option<crate::coordinates::MapPoint> {
        let guard_id = match self.get_entity(pc_id) {
            Some(Entity::Pc(pc)) => pc.pc.guard?,
            _ => return None,
        };
        let guard = self.get_entity(guard_id)?;
        Some(guard.position_iface().map_position())
    }

    /// Return the PC entity at the given 0-based portrait slot, or `None`
    /// when `index` is out of range.
    ///
    /// The portrait bar is not used as a retained source of truth — `pc_ids`
    /// is sorted by character profile priority at level load
    /// (`sort_pc_ids_by_priority`, called from `level_loading`). PCs whose
    /// portrait widget is not currently displayed (`interface_hidden ==
    /// true`) are skipped here so a hidden PC does not consume a slot
    /// index.
    pub fn character_for_portrait_index(&self, index: u8) -> Option<EntityId> {
        self.displayed_pc_ids().into_iter().nth(usize::from(index))
    }

    /// Iterator over PCs whose portrait widget is currently displayed.
    ///
    /// We don't keep a retained portrait bar — `pc_ids` always carries every
    /// PC in the mission and the HUD reconstitutes the displayed subset each
    /// frame by filtering on the per-PC `interface_hidden` flag. That flag
    /// is cleared on display and set on hide (e.g. on `DisableCharacter`
    /// outside Sherwood and on the reinforcement spawn for the dead PC).
    pub fn displayed_pc_ids(&self) -> Vec<EntityId> {
        self.world
            .pc_ids
            .iter()
            .copied()
            .filter(|&id| self.is_pc_interface_displayed(id))
            .collect()
    }

    /// Whether the per-PC portrait widget is currently displayed.
    ///
    /// Returns `false` when the PC entity is missing as well — a null
    /// portrait is treated as "not displayed".
    pub fn is_pc_interface_displayed(&self, id: EntityId) -> bool {
        match self.get_entity(id) {
            Some(Entity::Pc(pc)) => !pc.pc.interface_hidden,
            _ => false,
        }
    }

    /// Select a PC by portrait index (0-based).
    ///
    /// Used for character selection by key 1-5. When `multi_select` is true,
    /// adds to current selection instead of replacing.
    pub(crate) fn select_by_portrait_index(
        &mut self,
        assets: &LevelAssets,
        seat: usize,
        index: u8,
        multi_select: bool,
    ) {
        let pc_id = self.character_for_portrait_index(index);
        if let Some(id) = pc_id {
            if multi_select {
                self.toggle_pc_selection(assets, seat, id);
            } else {
                // Portrait clicks bark HERO_SELECT.
                self.select_pc(assets, seat, id, false, true);
            }
        }
    }

    /// Return `true` when every currently-selected PC is a member of the
    /// committed mission team. With no selection, returns `true` ("no
    /// mismatch yet" default).
    ///
    /// This is a pure read so the caller decides whether to gate on Sherwood.
    /// We still return `true` when the campaign or its mission-team is
    /// unavailable, because the downstream GoToExit gate uses `== false` to
    /// mean "at least one selected PC is missing from the team".
    pub fn are_selected_pc_in_mission_team(&self) -> bool {
        let Some(campaign) = Some(&self.mission_domain.campaign) else {
            return true;
        };
        let team_profiles = campaign.mission_team_profile_indices();
        for &id in &self.players.seats[0].selection {
            let profile_idx = match self.get_entity(id).and_then(|e| e.pc_data()) {
                Some(pc) => pc.profile_index,
                // No PC data for a selected id is an inconsistent
                // engine state — treat like "missing from team" so the
                // GoToExit gate stays conservative.
                None => return false,
            };
            if !team_profiles.contains(&profile_idx) {
                return false;
            }
        }
        true
    }

    /// Set the pending action for a PC via the select-action path.
    ///
    /// - If `pc_id` is not currently selected, only sets `current_action` on
    ///   that one PC.
    /// - A targeted non-NoAction message with a multi-selection first reduces
    ///   the selection to `pc_id`; otherwise the selected branch applies the
    ///   action to every selected PC.
    ///
    /// The selected branch also updates the seat's messenger-level armed
    /// action. The not-selected branch deliberately leaves it untouched.
    pub(crate) fn set_pc_action(
        &mut self,
        assets: &LevelAssets,
        input: &mut InputState,
        seat: usize,
        pc_id: EntityId,
        action: Action,
    ) {
        self.set_pc_action_inner(assets, Some(input), seat, pc_id, action);
    }

    /// Deliver a simulation-originated `MSG_SELECT_ACTION` when no mutable
    /// host input borrow is available. The gameplay mutations are identical
    /// to [`Self::set_pc_action`]; host-only rubber-band cleanup is emitted as
    /// a side effect for the host to consume after the tick.
    pub(crate) fn set_pc_action_from_message(
        &mut self,
        assets: &LevelAssets,
        seat: usize,
        pc_id: EntityId,
        action: Action,
    ) {
        self.set_pc_action_inner(assets, None, seat, pc_id, action);
    }

    fn set_pc_action_inner(
        &mut self,
        assets: &LevelAssets,
        input: Option<&mut InputState>,
        seat: usize,
        pc_id: EntityId,
        action: Action,
    ) {
        let parity_debug_stage_timing = std::env::var_os("PARITY_DEBUG_STAGE_TIMING").is_some();
        if parity_debug_stage_timing {
            eprintln!(
                "parity action: set_pc_action enter pc={pc_id:?} action={action:?} seat={seat}"
            );
        }

        // RHMessenger handles a non-NoAction MSG_SELECT_ACTION addressed to
        // one selected PC before RHEngine sees the action: with multiple PCs
        // selected it synchronously forwards UNSELECT_CHARACTER(all), then
        // SELECT_CHARACTER(target). SelectPC's single-selection restitution
        // therefore runs before the requested action is fanned out. Preserve
        // both that ordering and the case where the target became
        // unselectable between the two messages.
        if action != Action::NoAction
            && self.players.seats[seat].selection.len() > 1
            && self.players.seats[seat].selection.contains(&pc_id)
        {
            let old_selection = self.players.seats[seat].selection.clone();
            for old_id in old_selection {
                if let Some(Entity::Pc(pc)) = self.get_entity_mut(old_id) {
                    pc.pc.portrait.open = false;
                }
            }
            self.players.seats[seat].selection.clear();
            self.select_pc(assets, seat, pc_id, false, false);
        }

        if !self.players.seats[seat].selection.contains(&pc_id) {
            // "Not-selected" branch — only set the current action on the
            // single PC, no cleanup of the outgoing action. Don't call
            // `unselect_action` here: it would dispatch an extra
            // UnequipBow / LeaveBeggar / etc. command that this branch
            // must skip.
            if let Some(entity) = self.get_entity_mut(pc_id)
                && let Some(pc) = entity.pc_data_mut()
            {
                pc.current_action = action;
            }
            return;
        }

        self.players.seats[seat].selected_action = action;

        // Trajectory overlay cleanup on any action change from the selected
        // branch. The jumper trajectory, jumped trajectory, valid-trajectory
        // flag, and projectile preview must wipe immediately so the arc
        // doesn't linger for a frame; all four are folded into the host-side
        // preview state, which the `invalidate_trajectory_preview` side
        // effect flag wipes when consumed by `Host::apply_side_effects`.
        self.feedback
            .pending_side_effects
            .invalidate_trajectory_preview = true;

        // Cache the recording-macro state and use it to skip the
        // unselect-action / "stop in place" / pre-action-bow side effects
        // below — recording must not perturb the live sequence state of the
        // PC, only set its `current_action`.
        let record_qa = self.is_recording_macro();

        // For each selected PC, call `unselect_action` if the action is
        // changing (gated on `!record_qa`), then set the new action.
        for id in self.players.seats[seat].selection.clone() {
            let old_action = self
                .get_entity(id)
                .and_then(|e| e.pc_data())
                .map(|pc| pc.current_action)
                .unwrap_or(Action::NoAction);
            if old_action != action && !record_qa {
                if parity_debug_stage_timing {
                    eprintln!(
                        "parity action: before unselect_action pc={id:?} old={old_action:?} new={action:?}"
                    );
                }
                self.unselect_action(id);
                if parity_debug_stage_timing {
                    eprintln!(
                        "parity action: after unselect_action pc={id:?} old={old_action:?} new={action:?}"
                    );
                }
            }
            if let Some(entity) = self.get_entity_mut(id)
                && let Some(pc) = entity.pc_data_mut()
            {
                pc.current_action = action;
            }
        }

        // "Stop in place" group loop. When switching to a ranged / defensive
        // / interaction action that doesn't itself imply continued movement
        // (Hit / HitHard / Strangle / Heal / NoAction keep the PCs moving
        // so the seek path finishes), interrupt weaker-priority activity on
        // every selected PC. Bow has a carve-out: PCs already aiming keep
        // their aim so the player can chain shots without flicker. The
        // entire block is also gated on `!record_qa`.
        let should_stop_group = !record_qa
            && !matches!(
                action,
                Action::Hit | Action::HitHard | Action::Strangle | Action::NoAction | Action::Heal
            );
        if should_stop_group {
            for id in self.players.seats[seat].selection.clone() {
                if action == Action::Bow {
                    let state = self
                        .get_entity(id)
                        .and_then(|e| e.actor_data())
                        .map(|a| a.action_state)
                        .unwrap_or(ActionState::Waiting);
                    if matches!(
                        state,
                        ActionState::AimingWithBow
                            | ActionState::AimingWithBowUp
                            | ActionState::AimingWithBowDown
                    ) {
                        continue;
                    }
                }
                // `Normal` priority interrupts weaker-priority activity
                // but leaves Preference / Script / Injury / etc. running.
                if parity_debug_stage_timing {
                    eprintln!(
                        "parity action: before group stop_owner pc={id:?} priority=Normal action={action:?}"
                    );
                }
                self.stop_owner(id, SequencePriority::Normal);
                if parity_debug_stage_timing {
                    eprintln!(
                        "parity action: after group stop_owner pc={id:?} priority=Normal action={action:?}"
                    );
                }
            }
        }

        // Per-action entry hooks.
        match action {
            Action::Bow
                // Equip the bow on the single selected PC if not already
                // aiming. The projectile-trajectory object-type switch is
                // derived on-demand by `compute_trajectory_preview`, so we
                // don't need a separate state write.
                if !record_qa => {
                    self.manage_input_pre_action_bow(assets, seat);
                }
            Action::Shield | Action::BigShield => {
                // Reset shield protection for the fresh activation.
                self.world.shield.is_protected = true;
                self.world.shield.protected_pc = None;
                self.world.shield.danger_point = crate::coordinates::WorldPoint3D::default();
                self.world.shield.danger_point_layer = 0;
            }
            _ => {}
        }

        // Hide the drag-box immediately when an action is picked.
        if let Some(input) = input {
            input.multi_selection_active = false;
            input.multi_unselection_active = false;
            input.draw_multi_selection = false;

            // Set `next_left_double_is_simple` so the first click after
            // picking an action is not interpreted as the first half of a
            // double-click — otherwise a quick action-button tap would
            // immediately fire the action on the nearest target via the
            // double-click repeat path.
            input.ignore_mouse_event(false, false, true);
        } else {
            self.feedback.pending_side_effects.cancel_multi_selection = true;
            // TODO(input): expose the third IgnoreMouseEvent argument as a
            // host side effect too. Resolved-command replay is independent
            // of this raw double-click latch, but live simulation-originated
            // SelectAction messages should preserve it exactly.
        }
        if parity_debug_stage_timing {
            eprintln!("parity action: set_pc_action exit pc={pc_id:?} action={action:?}");
        }
    }

    /// Compute the [`Stature`] state for the up/down arrow widgets.
    ///
    /// `pc_id == None` is the "all-selected" sweep: iterate every selected
    /// PC, collect whether any are upright vs crouched/lying, and combine
    /// into a [`Stature`] enum. PCs on a wall / ladder / inside a building
    /// are skipped.
    ///
    /// `pc_id == Some(id)` is the single-PC branch: returns `Stature::None`
    /// when the PC is *not* climbing / in a building, otherwise the posture
    /// maps to Down for Lying/Crouched and Up otherwise.
    ///
    /// Consumed by `crates/robin_rs/src/stature_hud.rs` (drawing the up/down
    /// arrow widgets on the lower panel) — game_session polls
    /// `retrieve_stature(None)` every frame instead of going through an
    /// event-driven refresh.
    pub fn retrieve_stature(&self, pc_id: Option<EntityId>) -> Stature {
        use crate::element::Posture;

        // Engine-level climbing-or-in-building test — NOT the actor-level
        // variant (which has slightly different semantics).
        let is_climbing_or_in_building = |pc_id: EntityId| -> bool {
            let entity = self.expect_entity(pc_id, "retrieve_stature selected PC");
            let posture = entity.element_data().posture;
            if posture == Posture::OnWall || posture == Posture::OnLadder {
                return true;
            }
            let Some(sector_handle) = entity.element_data().sector() else {
                return false;
            };
            self.world
                .fast_grid
                .level
                .sector_number_map
                .get(&crate::sector::SectorNumber::new(
                    u16::from(sector_handle) as i16
                ))
                .and_then(|&idx| self.world.fast_grid.level.sectors.get(idx))
                .map(|gs| gs.sector_type.is_building())
                .unwrap_or(false)
        };

        if let Some(id) = pc_id {
            // Single-PC branch.
            if !is_climbing_or_in_building(id) {
                return Stature::None;
            }
            let posture = self
                .expect_entity(id, "retrieve_stature selected PC")
                .element_data()
                .posture;
            return match posture {
                Posture::Lying | Posture::Crouched => Stature::Down,
                _ => Stature::Up,
            };
        }

        // All-selected branch.
        let mut up = false;
        let mut down = false;
        for &pc_id in &self.players.seats[0].selection {
            if is_climbing_or_in_building(pc_id) {
                continue;
            }
            let posture = match self.get_entity(pc_id) {
                Some(e) => e.element_data().posture,
                None => continue,
            };
            match posture {
                Posture::Lying | Posture::Crouched => down = true,
                Posture::OnWall => {} // no-op
                _ => up = true,
            }
        }
        match (up, down) {
            (true, true) => Stature::Both,
            (true, false) => Stature::Up,
            (false, true) => Stature::Down,
            (false, false) => Stature::None,
        }
    }

    /// Called when the player picks the Bow action on a single selected
    /// PC: if the PC isn't already aiming, stop weaker-priority activity,
    /// launch an `EquipBow` sequence element, and play the
    /// `HERO_ACCEPT_COMMAND` bark.
    ///
    /// Original asserts that the caller has selected exactly one PC. The
    /// macro-recording short-circuit is enforced by the sole caller
    /// `set_pc_action`.
    fn manage_input_pre_action_bow(&mut self, assets: &LevelAssets, seat: usize) {
        let Some(&pc_id) = self.players.seats[seat].selection.first() else {
            return;
        };
        let action_state = match self.get_entity(pc_id).and_then(|e| e.actor_data()) {
            Some(a) => a.action_state,
            None => return,
        };
        if matches!(
            action_state,
            ActionState::AimingWithBow
                | ActionState::AimingWithBowUp
                | ActionState::AimingWithBowDown
        ) {
            return;
        }

        self.stop_owner(pc_id, SequencePriority::Preference);
        let elem = SequenceElement::new(1, Command::EquipBow, Some(pc_id));
        let mut sequence = crate::sequence::Sequence::new();
        sequence.append_element(elem);
        self.launch_sequence(sequence);
        self.hero_speaking(assets, pc_id, crate::engine::melee::HERO_ACCEPT_COMMAND);
    }

    /// Clean up side-effects of the current action before switching away.
    ///
    /// - Bow + aiming → launch UnequipBow sequence
    /// - HelpToClimb + helping posture → launch LeaveHelpingClimb
    /// - Beggar + beggar posture → launch LeaveBeggar
    /// - Listen + listening state → launch LeaveListen
    /// - Clears `valid_trajectory`
    /// - Sets `current_action = NoAction`
    ///
    /// Skips cleanup when the PC is swordfighting.
    pub(crate) fn unselect_action(&mut self, pc_id: EntityId) {
        use crate::element::{ActionState, Command, Posture};
        use crate::sequence::SequenceElement;

        let (is_swordfighting, old_action, action_state, posture) = match self.get_entity(pc_id) {
            Some(Entity::Pc(pc)) => (
                !pc.human.opponents.is_empty(),
                pc.pc.current_action,
                pc.actor.action_state,
                pc.element.posture,
            ),
            _ => return,
        };

        if !is_swordfighting {
            match old_action {
                Action::Bow
                    if action_state == ActionState::AimingWithBow
                        || action_state == ActionState::AimingWithBowUp =>
                {
                    // `Preference` priority interrupts anything weaker
                    // (Normal / Wait / None), then launches `UnequipBow`.
                    // Stronger priorities (Script, Injury, KO,
                    // NonInterruptable) are protected.
                    let parity_debug_stage_timing =
                        std::env::var_os("PARITY_DEBUG_STAGE_TIMING").is_some();
                    if parity_debug_stage_timing {
                        eprintln!(
                            "parity action: before bow cleanup stop_owner pc={pc_id:?} priority=Preference"
                        );
                    }
                    self.stop_owner(pc_id, crate::sequence::SequencePriority::Preference);
                    if parity_debug_stage_timing {
                        eprintln!(
                            "parity action: after bow cleanup stop_owner pc={pc_id:?} priority=Preference"
                        );
                        eprintln!(
                            "parity action: before bow cleanup launch UnequipBow pc={pc_id:?}"
                        );
                    }
                    let elem = SequenceElement::new(1, Command::UnequipBow, Some(pc_id));
                    let mut sequence = crate::sequence::Sequence::new();
                    sequence.append_element(elem);
                    self.launch_sequence(sequence);
                    if parity_debug_stage_timing {
                        eprintln!(
                            "parity action: after bow cleanup launch UnequipBow pc={pc_id:?}"
                        );
                    }
                    tracing::debug!(?pc_id, "UnSelectAction: unequipping bow");
                }
                Action::HelpToClimb
                    if posture == Posture::HelpingToClimb
                        || posture == Posture::CarryingOnShoulders =>
                {
                    let elem = SequenceElement::new(1, Command::LeaveHelpingClimb, Some(pc_id));
                    let mut sequence = crate::sequence::Sequence::new();
                    sequence.append_element(elem);
                    self.launch_sequence(sequence);
                    tracing::debug!(?pc_id, "UnSelectAction: leaving helping climb");
                }
                Action::Beggar if posture == Posture::SimulatingBeggar => {
                    let elem = SequenceElement::new(1, Command::LeaveBeggar, Some(pc_id));
                    let mut sequence = crate::sequence::Sequence::new();
                    sequence.append_element(elem);
                    self.launch_sequence(sequence);
                    tracing::debug!(?pc_id, "UnSelectAction: leaving beggar");
                }
                Action::Listen if action_state == ActionState::Listening => {
                    let elem = SequenceElement::new(1, Command::LeaveListen, Some(pc_id));
                    let mut sequence = crate::sequence::Sequence::new();
                    sequence.append_element(elem);
                    self.launch_sequence(sequence);
                    tracing::debug!(?pc_id, "UnSelectAction: leaving listen");
                }
                _ => {}
            }
        }

        if let Some(entity) = self.get_entity_mut(pc_id)
            && let Some(pc) = entity.pc_data_mut()
        {
            pc.current_action = Action::NoAction;
        }
    }

    /// Read-only predicate: would [`Self::select_pc_action_by_index`]
    /// actually dispatch an action for this PC+button-index?  Evaluates the
    /// same profile-action-present and not-disabled checks without mutating
    /// engine state.  Callers use this to decide whether to emit a
    /// `PlayerCommand::SelectAction` or fall back to selecting the PC.
    pub fn can_dispatch_pc_action(&self, assets: &LevelAssets, pc_id: EntityId, index: u8) -> bool {
        let idx = index as usize;

        let Some(pc) = self.get_entity(pc_id).and_then(|e| e.pc_data()) else {
            return false;
        };
        let profile_idx = pc.profile_index;

        let Some(profile) = assets.profile_manager.get_character(profile_idx) else {
            return false;
        };
        let action = profile
            .actions
            .get(idx)
            .copied()
            .unwrap_or(Action::NoAction);
        if action == Action::NoAction {
            return false;
        }
        // The widget enable bit is gated on **both** `disabled_actions` and
        // `disabled_actions_temp` being clear, so OR the persistent and temp
        // masks together.
        let disabled_persistent = pc.disabled_actions.get(idx).copied().unwrap_or(false);
        let disabled_temp = pc.disabled_actions_temp.get(idx).copied().unwrap_or(false);
        !(disabled_persistent || disabled_temp)
    }

    /// Whether `action` is in `pc_id`'s profile and currently enabled.
    ///
    /// Used by the `LEFTDOUBLE` mouse-button pre-process to abort a multi-PC
    /// double-click when any selected PC can't (or is currently barred from)
    /// the pending action. The action lookup includes the `Eat → Guzzle`
    /// fallback via `inventory::find_action_slot`.
    ///
    /// Returns `false` when the entity isn't a PC, the campaign profile is
    /// unavailable, the profile doesn't list the action, or either
    /// `disabled_actions` / `disabled_actions_temp` is set on the resolved
    /// slot.
    pub fn is_pc_action_available(
        &self,
        profiles: &crate::profiles::ProfileManager,
        pc_id: EntityId,
        action: Action,
    ) -> bool {
        let Some(pc) = self.get_entity(pc_id).and_then(|e| e.pc_data()) else {
            return false;
        };
        let Some(profile) = profiles.get_character(pc.profile_index) else {
            return false;
        };
        let Some(idx) = crate::inventory::find_action_slot(profile, action) else {
            return false;
        };
        let disabled_persistent = pc.disabled_actions.get(idx).copied().unwrap_or(false);
        let disabled_temp = pc.disabled_actions_temp.get(idx).copied().unwrap_or(false);
        !(disabled_persistent || disabled_temp)
    }

    /// Select an action slot on a PC by index (0..=2).
    ///
    /// Looks up `profile.actions[index]`, checks the button is enabled
    /// (`!disabled_actions[index] && !disabled_actions_temp[index]`), and
    /// forwards through [`Self::set_pc_action`].
    ///
    /// Used by both portrait action-button clicks and the keyboard 1/2/3
    /// shortcut.
    ///
    /// Returns `true` if the action was dispatched (i.e. the button maps to
    /// a real action and is enabled), `false` otherwise.
    pub(crate) fn select_pc_action_by_index(
        &mut self,
        assets: &LevelAssets,
        input: &mut InputState,
        seat: usize,
        pc_id: EntityId,
        index: u8,
    ) -> bool {
        let idx = index as usize;

        let profile_idx = match self.get_entity(pc_id).and_then(|e| e.pc_data()) {
            Some(pc) => pc.profile_index,
            None => return false,
        };

        let action = match assets.profile_manager.get_character(profile_idx) {
            Some(profile) => profile
                .actions
                .get(idx)
                .copied()
                .unwrap_or(Action::NoAction),
            None => return false,
        };

        if action == Action::NoAction {
            return false;
        }

        // The widget enable bit is gated on **both** `disabled_actions` and
        // `disabled_actions_temp` being clear, so OR both masks together.
        let (disabled_persistent, disabled_temp) = self
            .get_entity(pc_id)
            .and_then(|e| e.pc_data())
            .map(|pc| {
                (
                    pc.disabled_actions.get(idx).copied().unwrap_or(false),
                    pc.disabled_actions_temp.get(idx).copied().unwrap_or(false),
                )
            })
            .unwrap_or((false, false));
        if disabled_persistent || disabled_temp {
            return false;
        }

        self.set_pc_action(assets, input, seat, pc_id, action);
        true
    }

    /// Perform multi-selection: select all PCs whose position falls inside
    /// the drag-box defined by `multi_selection_pt1` and
    /// `multi_selection_pt2`.
    ///
    /// When `shift_held` is true, adds to the existing selection. Each newly
    /// added PC barks `HERO_SELECT`. Sets `next_left_double_is_simple` to
    /// suppress the next left-double promotion.
    pub(crate) fn perform_multi_selection(
        &mut self,
        assets: &LevelAssets,
        input: &mut InputState,
        seat: usize,
        shift_held: bool,
    ) {
        if !input.draw_multi_selection {
            // Drag was too small — treat as a click, not a box select
            input.multi_selection_active = false;
            return;
        }

        let p1 = input.multi_selection_pt1;
        let p2 = input.multi_selection_pt2;
        let box_multi_selection = crate::sprite::BBox::new(
            crate::coordinates::ScreenPoint::new(p1.x.min(p2.x), p1.y.min(p2.y)),
            crate::coordinates::ScreenPoint::new(p1.x.max(p2.x), p1.y.max(p2.y)),
        );

        let pc_ids: Vec<EntityId> = self.world.pc_ids.clone();
        let mut box_selected: Vec<EntityId> = Vec::new();
        for &pc_id in &pc_ids {
            if !self.is_pc_selectable(assets, pc_id) {
                continue;
            }
            if let Some(entity) = self.get_entity(pc_id) {
                // Sprite-AABB-vs-drag-box overlap, matching the same
                // hit-test already used by `perform_multi_unselection`
                // below.
                let map_pos = entity.element_data().position_map();
                let map_pt = crate::coordinates::ScreenPoint::new(map_pos.x, map_pos.y);
                let sprite_box = entity
                    .sprite()
                    .bounding_box_at(entity.cxx_position_sprite());
                if box_multi_selection.is_intersecting(&sprite_box)
                    || box_multi_selection.contains_point(map_pt)
                {
                    box_selected.push(pc_id);
                }
            }
        }

        // Original sends one synchronous UNSELECT_CHARACTER followed by an
        // ordered SELECT_ADD_CHARACTER_WITH_ECHO per intersecting PC. Route
        // through the same helpers so Robin ordering, portraits, barks, and
        // action restitution/unselection all occur at the same boundaries.
        if !shift_held {
            self.unselect_all_pcs(seat);
        }
        for pc_id in box_selected {
            self.select_pc(assets, seat, pc_id, true, true);
        }

        input.multi_selection_active = false;
        input.draw_multi_selection = false;
        // Suppress the double-click promotion on the next left-click so a
        // box-select immediately followed by a click does not run the
        // double-click repeat path.
        input.next_left_double_is_simple = true;
    }

    // ─── Multi-UN-selection (right-drag red box) ────────────────

    /// Perform multi-UNselection: deselect every selected, playable PC whose
    /// sprite bounding-box intersects the right-drag box.
    pub(crate) fn perform_multi_unselection(&mut self, input: &mut InputState, seat: usize) {
        if !input.draw_multi_selection {
            input.multi_unselection_active = false;
            return;
        }

        let p1 = input.multi_selection_pt1;
        let p2 = input.multi_selection_pt2;
        let box_multi_selection = crate::sprite::BBox::new(
            crate::coordinates::ScreenPoint::new(p1.x.min(p2.x), p1.y.min(p2.y)),
            crate::coordinates::ScreenPoint::new(p1.x.max(p2.x), p1.y.max(p2.y)),
        );

        let pc_ids: Vec<EntityId> = self.world.pc_ids.clone();
        for &pc_id in &pc_ids {
            if !self.players.seats[seat].selection.contains(&pc_id) {
                continue;
            }
            let Some(entity) = self.get_entity(pc_id) else {
                continue;
            };
            // Playable filter.
            let playable = matches!(entity, Entity::Pc(pc) if pc.pc.playable);
            if !playable {
                continue;
            }
            // Bbox-vs-bbox overlap, not point-in-rect.
            let map_pos = entity.element_data().position_map();
            let map_pt = crate::coordinates::ScreenPoint::new(map_pos.x, map_pos.y);
            let sprite_box = entity
                .sprite()
                .bounding_box_at(entity.cxx_position_sprite());
            if (box_multi_selection.is_intersecting(&sprite_box)
                || box_multi_selection.contains_point(map_pt))
                && let Some(idx) = self.players.seats[seat]
                    .selection
                    .iter()
                    .position(|&x| x == pc_id)
            {
                self.players.seats[seat].selection.remove(idx);
                if let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id) {
                    pc.pc.portrait.open = false;
                }
            }
        }
        if self.players.seats[seat].selection.is_empty() {
            self.players.seats[seat].selected_action = Action::NoAction;
        }

        input.multi_unselection_active = false;
        input.draw_multi_selection = false;
    }

    /// Whether any selected PC on the host seat is currently swordfighting
    /// (any selected PC has a non-empty opponents list).
    pub fn is_selected_pc_swordfighting(&self) -> bool {
        self.is_seat_selection_swordfighting(crate::player_command::PlayerId::HOST)
    }

    /// Whether any selected PC on `player_id`'s seat is currently
    /// swordfighting.
    pub fn is_seat_selection_swordfighting(
        &self,
        player_id: crate::player_command::PlayerId,
    ) -> bool {
        self.seat_selection(player_id).iter().any(|&id| {
            self.get_entity(id)
                .and_then(|e| e.human_data())
                .is_some_and(|h| !h.opponents.is_empty())
        })
    }

    /// Assign the current selection to a quick-select group slot (0-8).
    pub(crate) fn assign_quick_group(&mut self, seat: usize, slot: usize) {
        if slot < 9 {
            self.players.seats[seat].quick_select_groups[slot] =
                self.players.seats[seat].selection.clone();
            tracing::info!(
                "Assigned {} PCs to group {} (seat {})",
                self.players.seats[seat].selection.len(),
                slot + 1,
                seat,
            );
        }
    }

    /// Tick the selection outline fade animation for every PC.
    ///
    /// Called once per frame from `perform_hourglass` so the fade decrements
    /// at the same cadence as the rest of the per-frame game logic.
    ///
    /// State machine summary:
    /// - Not selected and no animation in flight → clear `already_selected`.
    /// - First frame selected (`!already_selected`) → seed
    ///   `running_hulk = time_hulk`.
    /// - Subsequent frames → decrement `running_hulk`.
    /// - While `running_hulk > 0` → compute `hulk_level` (40..=100) according
    ///   to `hulk_direction` (true = fade out, false = fade in).
    /// - When `running_hulk` reaches 0 → reset direction/speed defaults.
    pub(crate) fn refresh_pc_selection_hulk(&mut self) {
        let pc_ids: Vec<EntityId> = self.world.pc_ids.clone();
        for pc_id in pc_ids {
            let is_drawn_as_selected = self.players.seats[0].selection.contains(&pc_id);
            let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id) else {
                continue;
            };

            if is_drawn_as_selected || pc.human.running_hulk != 0 {
                if !pc.pc.already_selected {
                    pc.human.running_hulk = pc.human.time_hulk;
                    pc.pc.already_selected = true;
                } else if pc.human.running_hulk > 0 {
                    pc.human.running_hulk -= 1;
                }

                if pc.human.running_hulk > 0 {
                    let ratio = pc.human.running_hulk as f32 / pc.human.time_hulk as f32;
                    pc.human.hulk_level = if pc.human.hulk_direction {
                        40 + (60.0 * ratio) as u16
                    } else {
                        40 + (60.0 * (1.0 - ratio)) as u16
                    };
                } else {
                    pc.human.hulk_direction = true;
                }
            } else {
                pc.pc.already_selected = false;
            }
        }
    }

    /// Collect active PCs whose profile can perform `action`, appending them
    /// to `out`.
    ///
    /// Iterates every PC and matches on `has_action(action) ||
    /// has_contextual_action(action)`, with a special case for
    /// `LittleJohnCarry` that also accepts `FarmerCarry` (both sides, main
    /// + contextual).
    ///
    /// Sherwood gating and the actual mark application are left to the
    /// caller: the host-side UI decides when to invoke this (on
    /// requirements-bar hover), and writes the results into
    /// `InputState::marked_pc_ids` so the outline pass can read them.
    pub fn collect_pcs_with_action(
        &self,
        assets: &LevelAssets,
        action: Action,
        out: &mut Vec<EntityId>,
    ) {
        if false {
            return;
        }
        for &pc_id in &self.world.pc_ids {
            let Some(Entity::Pc(pc)) = self.get_entity(pc_id) else {
                continue;
            };
            if !pc.element.active {
                continue;
            }
            let Some(profile) = assets.profile_manager.get_character(pc.pc.profile_index) else {
                continue;
            };
            let matches = if action == Action::LittleJohnCarry {
                profile.can_carry()
            } else {
                profile.has_action(action) || profile.has_contextual_action(action)
            };
            if matches {
                out.push(pc_id);
            }
        }
    }

    /// Recall a quick-select group, replacing the current selection.
    /// Only recalls PCs that are still alive and selectable.
    pub(crate) fn recall_quick_group(&mut self, assets: &LevelAssets, seat: usize, slot: usize) {
        if slot < 9 {
            let group = self.players.seats[seat].quick_select_groups[slot].clone();
            self.players.seats[seat].selection.clear();
            for &pc_id in &group {
                if self.is_pc_selectable(assets, pc_id) {
                    self.players.seats[seat].selection.push(pc_id);
                }
            }
            tracing::info!(
                "Recalled group {} ({} PCs, seat {})",
                slot + 1,
                self.players.seats[seat].selection.len(),
                seat,
            );
        }
    }

    /// Sort `pc_ids` by character profile priority (descending).
    ///
    /// Highest priority first = leftmost. This ensures the portrait bar,
    /// keyboard shortcuts (1-5), and [`Self::select_by_portrait_index`] all
    /// use the priority-sorted order.
    pub(crate) fn sort_pc_ids_by_priority(&mut self, assets: &LevelAssets) {
        let entities = &self.world.entities;
        self.world.pc_ids.sort_by(|&a, &b| {
            let pri_a = Self::pc_priority_static(entities, &assets.profile_manager, a);
            let pri_b = Self::pc_priority_static(entities, &assets.profile_manager, b);
            pri_b.cmp(&pri_a) // descending
        });
        tracing::debug!("Sorted pc_ids by priority: {:?}", self.world.pc_ids);
    }

    /// Look up the profile priority for a PC entity without &self.
    fn pc_priority_static(
        entities: &crate::entities::Entities,
        profiles: &crate::profiles::ProfileManager,
        id: EntityId,
    ) -> u16 {
        let profile_idx = entities
            .get(id)
            .and_then(|e| e.pc_data())
            .map(|pc| pc.profile_index);
        let Some(idx) = profile_idx else { return 0 };
        profiles.get_character(idx).map(|p| p.priority).unwrap_or(0)
    }

    /// Tick the cheat-teleport hulk-rebuild fade for every PC.
    ///
    /// Decrement `teleport_counter` once per frame. The render layer is
    /// `&Engine`-only, so the decrement lives here on the engine and is
    /// driven from the renderer's pre-frame setup pass. The fade visual
    /// itself (vanishing ghost at `position_before_teleport` + appearing
    /// sprite at the current position) is rendered in
    /// `crates/robin_rs/src/game_render.rs::render_entities_gpu`.
    pub fn tick_pc_teleport_fades(&mut self) {
        let pc_ids: Vec<EntityId> = self.world.pc_ids.clone();
        for pc_id in pc_ids {
            let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id) else {
                continue;
            };
            if pc.pc.teleport_counter > 0 {
                pc.pc.teleport_counter -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ActorPc, ElementData, ElementKind, Posture};
    use crate::sequence::SequenceState;

    fn add_selectable_test_pc(engine: &mut EngineInner) -> EntityId {
        engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                active: true,
                kind: ElementKind::ActorPc,
                posture: Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        }))
    }

    #[test]
    fn single_selection_restitution_replays_current_action_side_effects() {
        let assets = LevelAssets::default();
        let mut engine = EngineInner::new();
        let pc_id = engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                active: true,
                kind: ElementKind::ActorPc,
                posture: Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            pc: crate::element::PcData {
                current_action: Action::Shield,
                ..Default::default()
            },
        }));

        let mut shooting = SequenceElement::new(1, Command::ShootBowOnce, Some(pc_id));
        shooting.priority = SequencePriority::Normal;
        let sequence_id = engine.orders.sequence_manager.launch_element(shooting);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);

        engine.select_pc(&assets, 0, pc_id, false, false);

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence_id, 0)
                .expect("shooting sequence remains inspectable")
                .state,
            SequenceState::Interrupted,
            "reselecting one PC must resend its action and run SelectAction's stop-in-place hook"
        );
    }

    #[test]
    fn targeted_bow_action_collapses_multi_selection_and_retains_other_pc_shot() {
        let assets = LevelAssets::default();
        let mut engine = EngineInner::new();
        let target_pc = add_selectable_test_pc(&mut engine);
        let shooting_pc = add_selectable_test_pc(&mut engine);
        engine.players.seats[0].selection = vec![target_pc, shooting_pc];

        let mut shooting = SequenceElement::new(1, Command::ShootBow, Some(shooting_pc));
        shooting.priority = SequencePriority::Normal;
        let shooting_sequence = engine.orders.sequence_manager.launch_element(shooting);
        engine
            .orders
            .sequence_manager
            .element_in_progress(shooting_sequence, 0);

        engine.set_pc_action_from_message(&assets, 0, target_pc, Action::Bow);

        assert_eq!(engine.players.seats[0].selection, vec![target_pc]);
        assert_eq!(
            engine
                .get_entity(target_pc)
                .and_then(Entity::pc_data)
                .expect("target PC exists")
                .current_action,
            Action::Bow
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(shooting_sequence, 0)
                .expect("other PC's shot remains registered")
                .state,
            SequenceState::InProgress,
            "the action fanout must not interrupt a retained shot owned by the unselected PC"
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .any(|sequence| {
                    sequence.elements.iter().any(|element| {
                        element.owner == Some(target_pc) && element.command == Command::EquipBow
                    })
                }),
            "the specialized single-PC Bow path must still launch EquipBow for the target"
        );
    }
}
