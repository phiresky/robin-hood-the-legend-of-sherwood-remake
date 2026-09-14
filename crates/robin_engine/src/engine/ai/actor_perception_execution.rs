use super::*;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_focus(
        &mut self,
        owner: EntityId,
        target: Option<crate::ai::AiEntityHandle>,
    ) {
        if let Some(target) = target {
            let target = self.expect_entity_id_for_index(target.get(), "AI focus target");
            let npc = self
                .world
                .entities
                .expect_ai_actor_data_mut(owner, format_args!("focus owner"));
            crate::ai_vision::focus_entity(npc, target);
        } else {
            self.execute_ai_unfocus(owner);
        }
    }

    pub(in crate::engine) fn execute_ai_unfocus(&mut self, owner: EntityId) {
        let npc = self
            .world
            .entities
            .expect_ai_actor_data_mut(owner, format_args!("unfocus owner"));
        crate::ai_vision::unfocus(npc);
    }

    pub(in crate::engine) fn execute_ai_focus_point(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        point: crate::ai::Position,
    ) {
        let point = self.position_to_point_3d(assets, point.sector, point.level, point.x, point.y);
        let npc = self
            .world
            .entities
            .expect_ai_actor_data_mut(owner, format_args!("focus-point owner"));
        crate::ai_vision::focus_point(npc, crate::coordinates::GroundPoint::new(point.x, point.y));
    }

    pub(in crate::engine) fn execute_ai_add_detectable(
        &mut self,
        owner: EntityId,
        target: EntityId,
        kind: DetectableType,
    ) {
        let target_entity = self.expect_entity(target, "detectable target");
        match kind {
            DetectableType::Enemy | DetectableType::Body | DetectableType::Beggar => {
                assert!(target_entity.is_human(), "detectable kind requires a human")
            }
            DetectableType::Friend | DetectableType::MissedFriend => assert!(
                target_entity.npc_data().is_some(),
                "friend detectable requires an NPC"
            ),
            DetectableType::Object => assert!(
                target_entity.is_object(),
                "object detectable requires an object"
            ),
            DetectableType::None => panic!("a detectable requires a concrete kind"),
        }
        let already_body = kind == DetectableType::Body
            && target_entity
                .human_data()
                .expect("body target is human")
                .already_detectable_body;
        if kind == DetectableType::Enemy {
            let target = self.expect_entity(target, "enemy detectable target");
            let owner = self.expect_entity(owner, "enemy detectable owner");
            if !target.is_human()
                || !crate::ai_detectable_filter::should_add_enemy_detectable_with(
                    &self.mission_domain.diplomacy,
                    owner.camp(),
                    owner.enemy_ai().is_some(),
                    target.is_pc(),
                    target.is_soldier(),
                    target.camp(),
                )
            {
                return;
            }
        }
        let npc = self
            .world
            .entities
            .expect_ai_actor_data_mut(owner, format_args!("detectable owner"));
        let list = &mut npc.detectable_lists[kind as usize];
        let present = list.iter().any(|entry| entry.element == Some(target));
        if already_body && present {
            return;
        }
        assert!(!present, "detectable must not already be registered");
        append_detectable(list, target, kind, true);
    }

    pub(in crate::engine) fn execute_ai_append_detectable(
        &mut self,
        owner: EntityId,
        target: EntityId,
        kind: DetectableType,
    ) {
        let npc = self
            .world
            .entities
            .expect_ai_actor_data_mut(owner, format_args!("detectable owner"));
        append_detectable(&mut npc.detectable_lists[kind as usize], target, kind, true);
    }

    pub(in crate::engine) fn execute_ai_delete_detectable_type(
        &mut self,
        owner: EntityId,
        kind: DetectableType,
    ) {
        let npc = self
            .world
            .entities
            .expect_ai_actor_data_mut(owner, format_args!("detectable owner"));
        npc.detectable_lists[kind as usize].clear();
    }

    pub(in crate::engine) fn execute_ai_delete_detectable_entity(
        &mut self,
        owner: EntityId,
        target: EntityId,
        kind: DetectableType,
    ) {
        self.world
            .entities
            .expect_ai_actor_data_mut(owner, format_args!("detectable owner"))
            .delete_detectable(target, kind);
    }

    pub(in crate::engine) fn execute_ai_blink_all_enemies(&mut self, owner: EntityId) {
        let npc = self
            .world
            .entities
            .expect_ai_actor_data_mut(owner, format_args!("blink owner"));
        for detectable in &mut npc.detectable_lists[DetectableType::Enemy as usize] {
            detectable.seen_now = false;
            detectable.seen_last_frame = false;
        }
    }

    pub(in crate::engine) fn execute_ai_set_checkpoint_charly(
        &mut self,
        owner: EntityId,
        target: Option<crate::ai::AiEntityHandle>,
    ) {
        self.execute_ai_delete_detectable_type(owner, DetectableType::MissedFriend);
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("checkpoint owner"))
            .checkpoint_charly = target;
        if let Some(target) = target {
            let target = self.expect_entity_id_for_index(target.get(), "checkpoint Charly");
            self.execute_ai_add_detectable(owner, target, DetectableType::MissedFriend);
        } else {
            self.world
                .entities
                .expect_ai_controller_mut(owner, format_args!("checkpoint owner"))
                .sorrow_level = 0;
            self.execute_ai_delete_detectable_type(owner, DetectableType::MissedFriend);
        }
    }

    pub(in crate::engine) fn execute_ai_break_macro(&mut self, owner: EntityId) {
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("macro owner"));
        ai.macro_in_progress = false;
        ai.macro_timer_is_running = false;
        self.execute_ai_set_checkpoint_charly(owner, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::AiEntityHandle;
    use crate::engine::test_support::actors::make_test_ai_soldier;

    #[test]
    fn checkpoint_replacement_and_macro_break_update_live_lists_immediately() {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let first = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let second = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        engine.execute_ai_set_checkpoint_charly(owner, Some(AiEntityHandle::new(first.index())));
        let list = &engine
            .world
            .entities
            .expect_ai_actor_data(owner, format_args!("checkpoint test"))
            .detectable_lists[DetectableType::MissedFriend as usize];
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].element, Some(first));
        engine.execute_ai_set_checkpoint_charly(owner, Some(AiEntityHandle::new(second.index())));
        let list = &engine
            .world
            .entities
            .expect_ai_actor_data(owner, format_args!("checkpoint test"))
            .detectable_lists[DetectableType::MissedFriend as usize];
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].element, Some(second));
        let ai = engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("checkpoint test"));
        ai.sorrow_level = 15;
        ai.macro_command = vec![10, 20, 30, 40, 50];
        ai.macro_command_offset = 3;
        ai.number_of_remaining_macro_bytes = 2;
        ai.macro_in_progress = true;
        ai.macro_timer_is_running = true;
        engine.execute_ai_break_macro(owner);
        let ai = engine
            .world
            .entities
            .expect_ai_controller(owner, format_args!("checkpoint test"));
        assert_eq!(ai.sorrow_level, 0);
        assert_eq!(ai.checkpoint_charly, None);
        assert!(!ai.macro_in_progress);
        assert!(!ai.macro_timer_is_running);
        assert_eq!(ai.macro_command, [10, 20, 30, 40, 50]);
        assert_eq!(ai.macro_command_offset, 3);
        assert_eq!(ai.number_of_remaining_macro_bytes, 2);
        assert!(
            engine
                .world
                .entities
                .expect_ai_actor_data(owner, format_args!("checkpoint test"))
                .detectable_lists[DetectableType::MissedFriend as usize]
                .is_empty()
        );
    }

    #[test]
    fn focus_operations_keep_mobile_target_without_following_after_unfocus() {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let target = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        engine.execute_ai_focus(owner, Some(AiEntityHandle::new(target.index())));
        engine.execute_ai_unfocus(owner);
        let npc = engine
            .world
            .entities
            .expect_ai_actor_data(owner, format_args!("focus test"));
        assert_eq!(npc.follow_target, Some(target));
        assert_eq!(npc.eye_status, crate::element::EyeStatus::LookForward);
    }
}
