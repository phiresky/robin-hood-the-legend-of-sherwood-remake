//! Read-only engine queries used by presentation and command admission.
use super::*;

impl EngineInner {
    /// Iterate over all live entities (skipping `None` slots).
    pub fn entities_iter(&self) -> impl Iterator<Item = &Entity> + '_ {
        self.world.entities.occupied().map(|(_, entity)| entity)
    }

    /// Iterate over all live entities together with their typed table IDs.
    ///
    /// Diagnostic tools that compare two independently-built simulations use
    /// this to construct an isomorphism between entity tables; callers must
    /// not assume that the returned IDs have meaning outside this engine.
    pub fn entities_with_ids_iter(&self) -> impl Iterator<Item = (EntityId, &Entity)> + '_ {
        self.world.entities.occupied()
    }

    /// Active entity positions for debug overlays.
    pub fn active_entity_positions(
        &self,
    ) -> impl Iterator<Item = (EntityId, crate::coordinates::MapPoint)> + '_ {
        self.world.entities.occupied().filter_map(|(id, entity)| {
            entity
                .is_active()
                .then_some((id, entity.element_data().position_map()))
        })
    }

    /// All player characters (portrait order).
    pub fn pc_ids(&self) -> &[EntityId] {
        &self.world.pc_ids
    }

    /// All NPCs (soldiers + civilians).
    pub fn npc_ids(&self) -> Vec<EntityId> {
        self.world.entities.npc_ids().collect()
    }

    /// Currently selected PC ids for the [`PlayerId::HOST`] seat.
    ///
    /// Single-player host code (HUD, renderer, input translation)
    /// always reads this accessor — there's only one seat in
    /// single-player and it's the host.  Multi-seat callers should use
    /// [`Self::hero_selection`] with their own
    /// [`crate::player_command::PlayerId`].
    pub fn selected_hero_ids(&self) -> &[EntityId] {
        &self.players.seats[0].selection
    }

    /// Selection for a specific seat, or `&[]` if the seat hasn't
    /// joined yet.  Multi-seat read path.
    pub fn hero_selection(&self, player_id: crate::player_command::PlayerId) -> &[EntityId] {
        self.players
            .seats
            .get(player_id.0 as usize)
            .map(|s| s.selection.as_slice())
            .unwrap_or(&[])
    }

    /// Logical selection across both command surfaces. The two slices remain
    /// physically separate for save/replay compatibility, but gameplay which
    /// applies to every player-commandable actor should consume this view.
    pub fn controlled_selection(
        &self,
        player_id: crate::player_command::PlayerId,
    ) -> impl Iterator<Item = &EntityId> {
        self.hero_selection(player_id)
            .iter()
            .chain(self.tactical_selection(player_id))
    }

    /// Look up [`SeatState`] for a `PlayerId`.  `None` when the seat
    /// hasn't materialised — happens before the seat's first
    /// `ConnectSeat` (or, for non-host seats, before its first
    /// command of any kind).
    pub fn seat(&self, player_id: crate::player_command::PlayerId) -> Option<&SeatState> {
        self.players.seats.get(player_id.0 as usize)
    }

    /// All currently-existing seats (connected or disconnected) in
    /// `PlayerId` order.  Renderer uses this to walk every seat for
    /// the portrait "controlled by" overlay; transport uses it to
    /// drive seat-list UI.
    pub fn seats(&self) -> &[SeatState] {
        &self.players.seats
    }

    /// Iterate over `(PlayerId, &SeatState)` pairs for every seat
    /// that's currently active — i.e. the host seat (always) plus
    /// any peer seat with `connected = true`.  Disconnected peers
    /// are filtered out so the renderer doesn't draw stale
    /// "controlled by" labels.
    pub fn active_seats(
        &self,
    ) -> impl Iterator<Item = (crate::player_command::PlayerId, &SeatState)> {
        self.players.seats.iter().enumerate().filter_map(|(i, s)| {
            if s.is_active(i) {
                Some((crate::player_command::PlayerId(i as u8), s))
            } else {
                None
            }
        })
    }

    /// `true` if at least one selected PC currently has its rotating
    /// selection circle drawn this frame — i.e. the per-PC posture /
    /// in-building filter lets at least one PC through.
    ///
    /// Used host-side to gate `SelectionMark::tick` so the ping-pong
    /// animation freezes whenever no circle would be drawn — the
    /// frame counter advances only during drawing, so
    /// non-drawing periods naturally paused the animation.
    pub fn any_selected_pc_drawing_selection_mark(&self) -> bool {
        for &pc_id in &self.players.seats[0].selection {
            if self.pc_draws_selection_mark(pc_id) {
                return true;
            }
        }
        false
    }

    /// `true` when any player-commandable selection for this
    /// seat has at least one entity eligible for the persistent ground ring.
    pub fn any_selection_drawing_selection_mark(
        &self,
        seat: crate::player_command::PlayerId,
    ) -> bool {
        self.controlled_selection(seat)
            .copied()
            .any(|id| self.pc_draws_selection_mark(id))
    }

    /// Check whether the entity's cached sector (set during door-pass
    /// transitions) is a building sector.
    ///
    /// Takes the entity's exact `element.sector` identity and returns the same
    /// handle when that sector has the BUILDING flag, so callers can also
    /// compare "same building". The original game's building lookup
    /// checks the actor's sector reference directly; the public-number lookup is
    /// retained only for identity-less compatibility positions.
    pub(crate) fn entity_building_sector(
        &self,
        sector: Option<crate::position_interface::SectorHandle>,
    ) -> Option<crate::position_interface::SectorHandle> {
        let sector = sector?;
        let grid_sector =
            movement::grid_sector_for_position_handle(&self.world.fast_grid.level, sector)?;
        let public_number = crate::sector::SectorNumber::new(i16::from(sector));
        assert_eq!(
            grid_sector.sector_number, public_number,
            "exact sector arena identity disagrees with its public number"
        );
        grid_sector.sector_type.is_building().then_some(sector)
    }

    /// `true` when the rotating ground selection circle should be drawn
    /// for an actor. Despite the legacy name, the posture/building checks
    /// apply equally to PCs and directly controlled allied soldiers.
    pub fn pc_draws_selection_mark(&self, pc_id: EntityId) -> bool {
        let Some(entity) = self.get_entity(pc_id) else {
            return false;
        };
        if !entity.is_active() {
            return false;
        }

        let elem = entity.element_data();
        if elem.posture() == crate::element::Posture::Flying
            || elem.hidden_in_building
            || elem.is_in_door_transit()
        {
            return false;
        }

        self.entity_building_sector(elem.sector()).is_none()
    }

    /// `true` if `pc_id` has any queued `Command::ShootBow` sequence
    /// element.  Used by the right-click `Bow` arm to decide whether to
    /// drain the shoot-list (queue non-empty) or cancel the Bow action
    /// (queue empty).
    pub fn pc_has_pending_shoot_bow(&self, pc_id: EntityId) -> bool {
        self.get_entity(pc_id)
            .and_then(|entity| entity.human_data())
            .is_some_and(|human| !human.pending_shoots.is_empty())
            || self
                .orders
                .sequence_manager
                .queued_element_exists(pc_id, crate::element::Command::ShootBow)
    }

    pub(in crate::engine) fn pc_should_hold_shoot_bow(
        &self,
        owner: EntityId,
        command: crate::element::Command,
    ) -> bool {
        use crate::order::OrderType;
        command == crate::element::Command::ShootBow
            && self.get_entity(owner).is_some_and(|entity| entity.is_pc())
            && self.get_entity(owner).is_some_and(|entity| {
                matches!(
                    entity.sprite().last_action,
                    OrderType::ShootingWithBow
                        | OrderType::ShootingWithBowUp
                        | OrderType::TransitionLoadingBow
                        | OrderType::TransitionRaisingBow
                        | OrderType::TransitionEquipBow
                )
            })
    }

    /// Background animation entity ids in occupied-entity order, without a snapshot allocation.
    pub fn bg_animation_ids(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.world
            .entities
            .occupied()
            .filter_map(|(id, entity)| entity.is_background_animation().then_some(id))
    }

    /// Quick-select group `idx` (0 = group 1, 8 = group 9).
    pub fn quick_select_group(&self, idx: usize) -> &[EntityId] {
        &self.players.seats[0].quick_select_groups[idx]
    }

    /// Floating indicator manager (titbits: stars, emoticons, smoke, splashes).
    /// The host reads it every frame to drive the titbit renderer; scripts
    /// and input handlers add new titbits through [`EngineInner::titbit_manager_mut`].
    pub fn titbit_manager(&self) -> &crate::titbit::TitbitManager {
        &self.feedback.titbit_manager
    }

    /// Current dotted-chain animation phase, advanced by the engine
    /// tick.  Host renderers read this to chain dotted line segments
    /// within a frame; they do not write it back — next frame's
    /// `perform_hourglass` re-advances it via
    /// `TitbitManager::prepare_refresh`.
    pub fn titbit_dotted_start(&self) -> f32 {
        self.feedback.titbit_manager.dotted_start()
    }

    /// Global AI state (alert levels, seek points, …). Read-only.
    pub fn ai_global(&self) -> &AiGlobalState {
        &self.ai.global
    }

    /// Read-only access to the per-PC quick-action macro store.  Host
    /// renderers use this to iterate slots for the portrait strip.
    pub fn macro_store(&self) -> &crate::macro_store::MacroStore {
        &self.players.macro_store
    }

    /// Does `pc` have a recorded macro in `slot`?
    pub fn has_quick_action(&self, pc: EntityId, slot: u8) -> bool {
        self.players
            .macro_store
            .get(pc)
            .map(|s| s.has_macro(slot as usize))
            .unwrap_or(false)
    }

    pub(crate) fn macro_slot_lengths(&self) -> Vec<MacroSlotLengths> {
        self.world
            .pc_ids
            .iter()
            .filter_map(|&pc_id| {
                let state = self.players.macro_store.get(pc_id)?;
                let lengths = std::array::from_fn(|slot| {
                    state
                        .slot(slot)
                        .map(crate::macro_store::QuickActionSlot::len)
                        .unwrap_or(0)
                        .try_into()
                        .unwrap_or_else(|_| {
                            panic!("macro slot {slot} has more than u16::MAX steps")
                        })
                });
                Some(MacroSlotLengths { pc_id, lengths })
            })
            .collect()
    }

    /// Whether the `--goldeneye` cheat is active.  Used by the PC
    /// refresh path to render every PC sprite at 50% alpha.
    pub fn get_golden_eye_mode(&self) -> bool {
        self.ai.global.golden_eye_mode
    }

    /// Weather / ambiance state (night colour, rain, fog, …).
    pub fn weather(&self) -> &WeatherState {
        &self.world.weather
    }

    /// Shield protection state (for the "Immortality" cheat).
    pub fn shield(&self) -> &ShieldState {
        &self.world.shield
    }

    /// Spatial acceleration grid (sectors, masks, jump lines, doors).
    pub fn fast_grid(&self) -> &FastFindGrid {
        &self.world.fast_grid
    }

    /// Canonical door selected by the same click-polygon fallback used by
    /// group movement when no fast-grid sector polygon contains the point.
    pub fn group_move_door_at(&self, point: crate::coordinates::MapPoint) -> Option<u32> {
        movement::door_click_polygon_at(&self.script_domains.interactables.doors, point)
    }

    /// A* waypoint pathfinder.
    pub fn pathfinder(&self) -> &PathFinder {
        &self.world.pathfinder
    }

    /// Committed path waypoints for an actor's active movement, if any.
    ///
    /// Returns the `(target_x, target_y)` of each remaining (non-`done`)
    /// order on the actor's currently-executing sequence element, in
    /// execution order.  Used by the surface debug overlay to draw the
    /// path the character will follow.  Returns `None` when the actor
    /// has no active movement element.
    pub fn actor_path_waypoints(
        &self,
        actor: EntityId,
    ) -> Option<Vec<crate::coordinates::MapPoint>> {
        let entity = self.get_entity(actor)?;
        let actor_data = entity.actor_data()?;
        let seq_id = actor_data.active_movement.sequence_id?;
        let elem_idx = actor_data.active_movement.element_index;
        let elem = self.orders.sequence_manager.get_element(seq_id, elem_idx)?;
        Some(
            elem.orders
                .iter()
                .filter(|o| !o.done)
                .map(|o| crate::coordinates::MapPoint::new(o.target_x, o.target_y))
                .collect(),
        )
    }

    /// Destination markers drawn on the ground.
    pub fn ground_mark(&self) -> &GroundMark {
        &self.feedback.ground_mark
    }

    /// Short mission briefing entries (read-only, drained by host UI).
    pub fn short_briefings(&self) -> &ShortBriefings {
        &self.mission_domain.short_briefings
    }

    /// Read the accumulated mission statistics (money, score, kills,
    /// recruitment, …).  Written by script natives during the tick and
    /// rolled up at mission end by [`EngineInner::apply_quit_mission_updates`].
    pub fn mission_stat(&self) -> &MissionStat {
        &self.mission_domain.mission_stat
    }

    /// Live deterministic achievement evidence for debriefing/tracker UI.
    pub fn mission_achievement_state(&self) -> &crate::achievement::MissionAchievementState {
        &self.mission_domain.achievements
    }

    /// Frozen successful-run results, if the mission crossed its successful
    /// terminal update boundary.
    pub fn mission_achievement_results(
        &self,
    ) -> Option<&crate::achievement::MissionAchievementResults> {
        self.mission_domain.achievements.finalized_results()
    }

    /// Whether the camera is locked to follow an entity.
    pub fn locker_active(&self) -> bool {
        self.players.seats[0].locker_active
    }

    /// Original messenger view-lock, distinct from camera-follow locker mode.
    pub fn view_locked(&self) -> bool {
        self.players.view_locked
    }

    /// Whether the player has the engine "user-locked" (alt-lock UI).
    pub fn user_locked(&self) -> bool {
        self.players.user_locked
    }

    /// Whether `pc` is part of the currently-armed recording set.
    pub fn is_qa_recording_for(&self, pc: EntityId) -> bool {
        self.players.qa_recording_for.contains(&pc)
    }
}
