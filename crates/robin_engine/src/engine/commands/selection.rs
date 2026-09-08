//! Selection execution after the parent resolves live versus recorded adjacency.
//! Keep that batch interpretation outside these individual command handlers.

use crate::element::EntityId;
use crate::engine::{EngineInner, LevelAssets};

impl EngineInner {
    pub(super) fn dispatch_pc_selection(
        &mut self,
        assets: &LevelAssets,
        seat: usize,
        pc_id: &EntityId,
        append: &bool,
        recorded_nested_selection_action: bool,
    ) {
        if !append {
            self.players.tactical.ensure_seat(seat).selection.clear();
        }
        if recorded_nested_selection_action {
            assert!(
                self.get_entity(*pc_id)
                    .and_then(crate::element::Entity::pc_data)
                    .is_some(),
                "recorded nested selection action targets missing or non-PC {pc_id:?}"
            );
        }
        self.select_pc_with_action_fanout(
            assets,
            seat,
            *pc_id,
            *append,
            true,
            !recorded_nested_selection_action,
        );
        self.update_recording_after_selection_change();
    }

    pub(super) fn dispatch_portrait_selection(
        &mut self,
        assets: &LevelAssets,
        seat: usize,
        portrait_index: &u32,
        append: &bool,
    ) {
        if !append {
            self.players.tactical.ensure_seat(seat).selection.clear();
        }
        // Portrait click → `select_by_portrait_index` fires
        // `select_pc` with `speak=true` directly.
        self.select_by_portrait_index(assets, seat, *portrait_index as u8, *append);
        self.update_recording_after_selection_change();
    }

    pub(super) fn apply_box_select(
        &mut self,
        assets: &LevelAssets,
        seat: usize,
        pt1: crate::coordinates::MapPoint,
        pt2: crate::coordinates::MapPoint,
        shift: bool,
    ) {
        self.perform_box_selection(assets, seat, pt1, pt2, shift);
        self.feedback.pending_side_effects.host_events.push(
            crate::engine::HostEvent::CancelMultiSelection {
                suppress_next_double: true,
            },
        );
    }

    pub(super) fn apply_box_unselect(
        &mut self,
        seat: usize,
        pt1: crate::coordinates::MapPoint,
        pt2: crate::coordinates::MapPoint,
    ) {
        self.perform_box_unselection(seat, pt1, pt2);
        self.feedback.pending_side_effects.host_events.push(
            crate::engine::HostEvent::CancelMultiSelection {
                suppress_next_double: false,
            },
        );
    }
}
