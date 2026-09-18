use super::*;
use crate::ai::{AiState, AlertLevel, LogLineType, Substate};
use crate::engine::TickCtx;

impl EngineInner {
    pub(super) fn begin_live_enemy_state(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        state: AiState,
        substate: Substate,
    ) -> bool {
        debug_assert_eq!(substate.ai_state_family(), Some(state));
        let ai = self.seek_enemy_mut(owner);
        let forced = ai.forced_attentive;
        ai.base
            .register_log_line(LogLineType::ChangeState, substate as u16);
        ai.base.timer_is_running = false;
        if ai.base.current_state == AiState::Menacing && state != AiState::Menacing {
            if let Some(pc) = ai.guarded_pc.take() {
                let entity = self
                    .entities_mut()
                    .expect_entity_mut(EntityId::Pc(pc), format_args!("released guarded PC"));
                let Entity::Pc(pc) = entity else {
                    unreachable!("typed PC guard")
                };
                pc.pc.guard = None;
            }
        }
        let ai = self.seek_enemy_mut(owner);
        if state != AiState::Default
            && !ai.changed_to_alert_path
            && let Some(path) = ai.base.alert_path_id
        {
            ai.changed_to_alert_path = true;
            ai.base.path_id = Some(path);
            let (last_waypoint_index, history) = if let Some(previous) = ai.base.patrol_path.take()
            {
                (previous.last_waypoint_index, previous.history)
            } else {
                (
                    ai.base.detached_patrol_path_status.last_waypoint_index,
                    std::mem::take(&mut ai.base.detached_patrol_path_status.history),
                )
            };
            ai.base.patrol_path = crate::ai::PatrolPath::new(path, &assets.navigation.hiking_paths)
                .map(|mut path| {
                    path.last_waypoint_index = last_waypoint_index;
                    path.history = history;
                    path
                });
            ai.base.has_patrol_path = true;
        }
        forced
    }

    pub(super) fn finish_live_enemy_state(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        state: AiState,
        substate: Substate,
        forced: bool,
    ) {
        if self.seek_enemy(owner).base.current_state == AiState::Sleeping
            && state != AiState::Sleeping
        {
            let npc = self.ai_actor_mut(owner, "awakening state owner");
            crate::ai_vision::set_view_status(npc, crate::element::EyeStatus::LookForward);
        }
        if !matches!(
            substate,
            Substate::AttackingProtectingWithShield
                | Substate::AttackingPhalanx
                | Substate::AttackingRunningToPhalanx
        ) && let Some(archer) = self.seek_enemy(owner).archer_behind_me
            && self.live_ai_is_shield_bearer(tcx.assets, owner)
        {
            let target = self.expect_human_id_for_ai_handle(archer.get(), "released paired archer");
            self.seek_enemy_mut(target).shield_bearer_before_me = None;
            self.seek_enemy_mut(owner).archer_behind_me = None;
        }
        if !matches!(
            substate,
            Substate::AttackingBowShooting
                | Substate::AttackingBowLoading
                | Substate::AttackingBowAiming
                | Substate::AttackingBowObservingLoading
                | Substate::AttackingBowObserving
                | Substate::AttackingBowRunningBehindShieldBearer
                | Substate::AttackingBowCorrectingPosition
        ) && self.seek_enemy(owner).is_archer()
            && let Some(bearer) = self.seek_enemy(owner).shield_bearer_before_me
        {
            let target = self.expect_human_id_for_ai_handle(bearer.get(), "released shield bearer");
            self.seek_enemy_mut(target).archer_behind_me = None;
            self.seek_enemy_mut(owner).shield_bearer_before_me = None;
        }
        // Both line-mode selectors inspect the incoming substate.
        if !matches!(
            substate,
            Substate::AttackingPhalanx
                | Substate::AttackingRunningToPhalanx
                | Substate::AttackingProtectingWithShield
        ) && !substate.is_real_swordfight()
        {
            let left = self.seek_enemy(owner).left_combat_neighbour;
            self.apply_update_left_combat_neighbour(owner.index(), left, None);
            let right = self.seek_enemy(owner).right_combat_neighbour;
            self.apply_update_right_combat_neighbour(owner.index(), right, None);
        }
        if !matches!(
            substate,
            Substate::AttackingArcherWaitOnArcheryPath
                | Substate::AttackingArcherWaitOnArcheryPathBending
                | Substate::AttackingArcherRunOnShootingPath
                | Substate::AttackingArcherRunOnShootingPathFinalSprint
                | Substate::AttackingArcherRunOnShootingPathTurn
                | Substate::AttackingOverviewLookLeft
                | Substate::AttackingOverviewLookRight
                | Substate::AttackingBowShooting
                | Substate::AttackingBowLoading
                | Substate::AttackingBowAiming
                | Substate::AttackingBowObservingLoading
                | Substate::AttackingBowObserving
        ) {
            if let Some((sector, point)) = self.seek_enemy_mut(owner).my_shooting_point.take() {
                self.ai
                    .global
                    .archery_sectors
                    .get_mut(usize::from(sector))
                    .expect("reserved shooting sector")
                    .points
                    .get_mut(usize::from(point))
                    .expect("reserved shooting point")
                    .owner = None;
            }
            if let Some(sector) = self.seek_enemy_mut(owner).my_archery_sector.take() {
                self.ai
                    .global
                    .archery_sectors
                    .get_mut(usize::from(sector))
                    .expect("reserved archery sector")
                    .decrement_owner_counter();
            }
        }
        if self.seek_enemy(owner).base.current_state == AiState::Seeking
            && state != AiState::Seeking
        {
            self.ai_actor_mut(owner, "departing search owner")
                .detectable_lists[crate::element::DetectableType::Beggar as usize]
                .clear();
            self.seek_enemy_mut(owner).beggar_to_examine = None;
        }
        let ai = self.seek_enemy_mut(owner);
        ai.base.set_ai_state(state);
        ai.base.current_substate = substate;
        let attentive = match state {
            AiState::Sleeping | AiState::Default => Some((forced, false)),
            AiState::Wondering => Some((
                (substate.is_take_money()
                    || substate.is_fight_for_money()
                    || matches!(
                        substate,
                        Substate::WonderingWatching | Substate::WonderingWatchingWhistling
                    ))
                    || forced,
                false,
            )),
            AiState::Seeking | AiState::Fleeing => match substate {
                Substate::SeekingSoldierCalledByOfficer
                | Substate::SeekingSoldierGoToOfficer
                | Substate::SeekingSoldierGetInstructedByOfficer
                | Substate::SeekingSoldierReturnToOfficer
                | Substate::SeekingSoldierGiveReportToOfficer
                | Substate::SeekingGroupGetInstructedByOfficer
                | Substate::SeekingCharlySentToOfficer
                | Substate::SeekingCharlyGoToOfficer
                | Substate::SeekingCharlyGoToOfficerSeen
                | Substate::SeekingCharlyGetLectureByOfficer
                | Substate::SeekingCharlyGetLectureByOfficer2 => Some((forced, true)),
                Substate::SeekingLookingResurrectedCharly
                | Substate::SeekingHeardstepsPreReactiontime => Some((forced, false)),
                Substate::SeekingGotStopEvent => None,
                _ => Some((true, false)),
            },
            AiState::Menacing => Some((true, false)),
            AiState::Attacking => Some((
                !matches!(
                    substate,
                    Substate::AttackingTooProudToAttack
                        | Substate::AttackingTooProudToAttackOverview
                        | Substate::AttackingTooProudToAttackApproach
                ),
                false,
            )),
        };
        if let Some((target, fast)) = attentive {
            self.set_soldier_attentive_mode_from(
                tcx,
                owner,
                target,
                fast,
                crate::engine::soldier_helpers::AttentiveModeCaller::AiOwnerEffect,
            );
        }
        let alert = match state {
            AiState::Sleeping | AiState::Default => AlertLevel::Green,
            AiState::Wondering if substate == Substate::WonderingUnderNet => AlertLevel::Green,
            AiState::Attacking => AlertLevel::Red,
            _ => AlertLevel::Yellow,
        };
        self.execute_ai_set_alert_status(tcx.assets, owner, alert, crate::ai::AlertFlags::empty());
    }
}
