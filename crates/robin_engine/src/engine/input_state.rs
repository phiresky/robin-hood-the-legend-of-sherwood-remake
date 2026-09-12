//! Host-local pointer gestures, sampled controls, and input feedback.
use super::*;

// ─── Input state (transient per-frame) ───────────────────────────────

/// A held pointer may remain physically down after an action disarms its drag.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
enum LeftPointerPhase {
    #[default]
    Released,
    Dragging,
    HeldWithoutDrag,
}

/// Both buttons may be held at once. Starting the second gesture replaces the
/// shared rectangle. Cancelling one mode can leave the other mode armed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
enum SelectionGesture {
    #[default]
    Idle,
    Selecting,
    Unselecting,
    SelectingAndUnselecting,
}

/// Physical controls sampled by the frontend, independent of cursor feedback.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SampledControls {
    /// The engine currently has OS focus.
    pub has_focus: bool,
    /// Right mouse button is currently held down.
    pub right_mouse_down: bool,
    /// Modifier snapshot consumed by gesture and view-cone updates.
    pub is_alt: bool,
}

/// Persistent pointer sequence and click targets. Gesture flags are changed
/// only through `InputState`'s named transitions.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PointerGestures {
    // Pointer lifecycle is private: callers dispatch transitions, never toggle flags.
    left_pointer: LeftPointerPhase,
    left_mouse_start_screen: ScreenPoint,
    left_double_click_pending: bool,
    selection_gesture: SelectionGesture,
    draw_multi_selection: bool,
    multi_selection_pt1: MapPoint,
    multi_selection_pt2: MapPoint,
    ignore_next_drag: bool,
    ignore_next_left_click: bool,
    next_left_double_is_simple: bool,

    /// Previous successful click target, retained for double-click dispatch.
    pub element_old_click: Option<crate::element::EntityId>,
    /// Current click-and-drag action target, retired on pointer release.
    pub target_drag: Option<crate::element::EntityId>,
    /// Portrait right-click arms dropping ammo on the next action click.
    pub portrait_drop_ammo_armed: bool,
    /// Double-click acceleration window for the last portrait action.
    pub portrait_action_countdown: u16,
    pub portrait_action_pc: Option<crate::element::EntityId>,
}

impl<'de> Deserialize<'de> for PointerGestures {
    fn deserialize<D: serde::Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "pointer gestures are live input state; initialize and dispatch pointer transitions",
        ))
    }
}

/// One spatial query result. Build locally, then publish the entire result;
/// consumers cannot mutate individual fields through `InputState`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpatialHit {
    pub selected_map_point: MapPoint,
    pub selected_layer: u16,
    /// Index into `FastFindGrid::sectors` for the sector under the mouse.
    /// Set each frame in `update_mouse`. Used for door/jump alpha overlays.
    pub selected_sector_idx: Option<crate::fast_find_grid::SectorIndex>,
    /// Index into the canonical interactable patch table for the patch whose overlay sector
    /// the mouse is hovering, if any.  Persisted on InputState so the
    /// cursor / render hooks don't re-scan `self.script_domains.interactables.patches` each
    /// frame.
    pub selected_patch_idx: Option<u32>,
    /// Index into the canonical interactable door table for a door whose click polygon is
    /// under the mouse. Some building doors are wider than their grid
    /// door sector, so hover/click handling must not depend only on
    /// `selected_sector_idx`.
    pub hovered_door_idx: Option<u32>,
    /// True when the hovered sector is a motion-area / door / jump
    /// sector or a patch overlay sits over the mouse — i.e. a move
    /// command dispatched here would have somewhere to land. Updated
    /// alongside `selected_sector_idx` each frame.
    pub valid_position_for_move: bool,
}

/// Presentation feedback computed from the spatial hit and active action.
/// Focus here is an action target, not necessarily the entity at the map point.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CursorFeedback {
    /// Entity currently under the mouse cursor.  Reset each frame.
    pub focused_entity_id: Option<crate::element::EntityId>,

    /// Entity whose double status bar should be shown this frame.
    pub double_status_bar_entity_id: Option<crate::element::EntityId>,

    /// PCs that should render at full-alpha outline this frame in
    /// response to a requirements-bar action hover. Iterates the PC
    /// list and marks each PC whose profile has the action. Populated
    /// by the host-side requirements-bar hit test before the outline
    /// pass; cleared at the start of each frame.
    pub marked_pc_ids: Vec<crate::element::EntityId>,

    /// Mouse cursor shadow intensity (0 = fully transparent, 50 = normal).
    /// Default = 40.  Set by bow/projectile branches.
    pub mouse_opacity: u16,

    /// Mouse cursor shadow color (16-bit packed, 0 = no shadow tint).
    /// Set by bow branch for no-target / civilian / VIP coloring.
    pub mouse_shadow_color: u16,

    /// Whether to advance cursor animation this frame.
    /// Set false for door cursors and some other cases where cursor
    /// animation should freeze.
    /// Default is `true` (via Default impl reset each frame).
    pub increment_cursor_animation: bool,

    /// Whether to render door hover UI this frame.
    /// Set true at start of `choose_mouse_pointer_for_no_action`,
    /// cleared when an entity is focused.
    pub display_door: bool,

    /// Debug "draw hidden" toggle, flipped by the masked-display
    /// switch message. When on, titbits attached to entities the
    /// player can't currently see (inside buildings, blipped) are
    /// still rendered so the debug view can inspect AI state.
    pub draw_hidden: bool,
}

/// Host-local input domains. The engine's serialized authoritative state does
/// not contain this state; snapshot restoration resets every domain to its
/// unfocused default. Fresh host construction explicitly uses `focused()`.
#[derive(Debug, Clone, Default)]
pub struct InputState {
    pub controls: SampledControls,
    pub gestures: PointerGestures,
    pub feedback: CursorFeedback,
    spatial_hit: SpatialHit,
}

impl InputState {
    pub fn focused() -> Self {
        Self {
            controls: SampledControls {
                has_focus: true,
                ..SampledControls::default()
            },
            ..Self::default()
        }
    }

    /// Read one coherent hit-test publication, never an in-progress query.
    ///
    /// ```compile_fail
    /// let mut input = robin_engine::engine::InputState::default();
    /// input.spatial_hit().selected_layer = 7;
    /// ```
    ///
    /// ```compile_fail
    /// let mut input = robin_engine::engine::InputState::default();
    /// input.spatial_hit = robin_engine::engine::SpatialHit::default();
    /// ```
    pub fn spatial_hit(&self) -> &SpatialHit {
        &self.spatial_hit
    }

    pub fn publish_spatial_hit(&mut self, hit: SpatialHit) {
        self.spatial_hit = hit;
    }

    /// Door cursor resolution historically selects the visible door layer while
    /// retaining the underlying patch sector and its movement eligibility.
    pub fn select_door_cursor_layer(&mut self, layer: u16) {
        self.spatial_hit.selected_layer = layer;
    }

    /// A rejected jump cursor retries the underlying motion sector. This is a
    /// cursor fallback, not a new mouse sample: point/layer/patch stay unchanged.
    pub fn select_jump_fallback_sector(
        &mut self,
        sector: Option<crate::fast_find_grid::SectorIndex>,
    ) {
        self.spatial_hit.selected_sector_idx = sector;
    }

    /// Clear only per-frame action feedback. Requirements-bar marks and the
    /// debug toggle have separate lifetimes and deliberately survive this reset.
    pub fn begin_cursor_feedback(&mut self, opacity: u16) {
        self.feedback.focused_entity_id = None;
        self.feedback.double_status_bar_entity_id = None;
        self.feedback.mouse_opacity = opacity;
        self.feedback.mouse_shadow_color = 0;
        self.feedback.increment_cursor_animation = true;
        self.feedback.display_door = false;
    }
    pub fn left_mouse_down(&self) -> bool {
        self.gestures.left_pointer != LeftPointerPhase::Released
    }

    pub fn is_dragging(&self) -> bool {
        self.gestures.left_pointer == LeftPointerPhase::Dragging
    }

    pub fn multi_selection_active(&self) -> bool {
        matches!(
            self.gestures.selection_gesture,
            SelectionGesture::Selecting | SelectionGesture::SelectingAndUnselecting
        )
    }
    pub fn multi_unselection_active(&self) -> bool {
        matches!(
            self.gestures.selection_gesture,
            SelectionGesture::Unselecting | SelectionGesture::SelectingAndUnselecting
        )
    }
    pub fn draw_multi_selection(&self) -> bool {
        self.gestures.draw_multi_selection
    }
    pub fn multi_selection_pt1(&self) -> MapPoint {
        self.gestures.multi_selection_pt1
    }
    pub fn multi_selection_pt2(&self) -> MapPoint {
        self.gestures.multi_selection_pt2
    }
    pub fn ignore_next_drag(&self) -> bool {
        self.gestures.ignore_next_drag
    }
    pub fn ignore_next_left_click(&self) -> bool {
        self.gestures.ignore_next_left_click
    }

    /// A platform press consumes double-click demotion even for a single click.
    pub fn press_left_pointer(&mut self, screen: ScreenPoint, clicks: u8) {
        self.gestures.left_pointer = LeftPointerPhase::Dragging;
        self.gestures.left_mouse_start_screen = screen;
        self.gestures.left_double_click_pending =
            !self.gestures.next_left_double_is_simple && clicks >= 2;
        self.gestures.next_left_double_is_simple = false;
    }

    /// Release retires drag authority but leaves selection/click suppression for dispatch.
    pub fn release_left_pointer(&mut self) -> bool {
        self.gestures.left_pointer = LeftPointerPhase::Released;
        self.gestures.target_drag = None;
        self.gestures.ignore_next_drag = false;
        std::mem::take(&mut self.gestures.left_double_click_pending)
    }

    /// Action re-arm/modal input reset disarms drawing without inventing a release.
    pub fn disarm_left_drag(&mut self) {
        if self.left_mouse_down() {
            self.gestures.left_pointer = LeftPointerPhase::HeldWithoutDrag;
        }
    }

    /// Frontend reset preserves pending suppression, matching existing modal behavior.
    pub fn reset_pointer_sequence(&mut self) {
        self.gestures.left_pointer = LeftPointerPhase::Released;
        self.controls.right_mouse_down = false;
        self.cancel_selection_gestures();
    }

    /// Touch takeover must not dispatch the abandoned pointer's release action.
    pub fn cancel_left_pointer(&mut self) {
        self.release_left_pointer();
        self.accept_mouse_event(true, true);
        self.finish_click_dispatch();
        self.cancel_selection_gestures();
    }

    pub fn reset_modal_input(&mut self) {
        self.cancel_selection_gestures();
        self.accept_mouse_event(true, true);
        self.disarm_left_drag();
        self.controls.is_alt = false;
    }

    pub fn cancel_selection_gestures(&mut self) {
        self.gestures.selection_gesture = SelectionGesture::Idle;
        self.gestures.draw_multi_selection = false;
    }

    /// Swordfight cancellation preserves the historical draw latch until reset.
    pub fn cancel_selection_for_swordfight(&mut self) {
        self.gestures.selection_gesture = SelectionGesture::Idle;
    }

    pub fn latch_selection_outline(&mut self) {
        assert!(self.multi_selection_active() || self.multi_unselection_active());
        self.gestures.draw_multi_selection = true;
    }

    pub fn demote_next_double_click(&mut self) {
        self.gestures.next_left_double_is_simple = true;
    }

    pub fn finish_click_dispatch(&mut self) {
        self.gestures.next_left_double_is_simple = false;
    }

    /// A drag action already fired; prevent release from repeating it. Macro
    /// recording also suppresses later motion so it records exactly one step.
    pub fn drag_action_dispatched(&mut self, recording_macro: bool) {
        self.gestures.ignore_next_left_click = true;
        self.gestures.ignore_next_drag |= recording_macro;
    }

    pub fn consume_suppressed_click(&mut self) -> bool {
        std::mem::take(&mut self.gestures.ignore_next_left_click)
    }

    /// Start a drag-box multi-selection at the given map-space point.
    pub fn start_multi_selection(&mut self, map_pt: MapPoint) {
        self.gestures.selection_gesture = if self.multi_unselection_active() {
            SelectionGesture::SelectingAndUnselecting
        } else {
            SelectionGesture::Selecting
        };
        self.gestures.draw_multi_selection = false;
        self.gestures.multi_selection_pt1 = map_pt;
        self.gestures.multi_selection_pt2 = map_pt;
    }

    /// Update the drag-box endpoint during a multi-selection drag.
    pub fn update_multi_selection(&mut self, map_pt: MapPoint) {
        self.gestures.multi_selection_pt2 = map_pt;
    }

    /// Cancel an in-progress multi-selection.
    pub fn cancel_multi_selection(&mut self) {
        self.gestures.selection_gesture = if self.multi_unselection_active() {
            SelectionGesture::Unselecting
        } else {
            SelectionGesture::Idle
        };
        self.gestures.draw_multi_selection = false;
    }

    /// Start a drag-box multi-UNselection at the given map-space point.
    pub fn start_multi_unselection(&mut self, map_pt: MapPoint) {
        self.gestures.selection_gesture = if self.multi_selection_active() {
            SelectionGesture::SelectingAndUnselecting
        } else {
            SelectionGesture::Unselecting
        };
        self.gestures.draw_multi_selection = false;
        self.gestures.multi_selection_pt1 = map_pt;
        self.gestures.multi_selection_pt2 = map_pt;
    }

    /// Cancel an in-progress multi-unselection.
    pub fn cancel_multi_unselection(&mut self) {
        self.gestures.selection_gesture = if self.multi_selection_active() {
            SelectionGesture::Selecting
        } else {
            SelectionGesture::Idle
        };
        self.gestures.draw_multi_selection = false;
    }

    /// Sets the three suppression flags the host reads at the next
    /// mouse event:
    ///
    /// - `click` → suppresses the next LMB-up.
    /// - `drag` → suppresses the next LMB drag motion.
    /// - `next_left_double_is_simple` → demotes the next platform double-
    ///   click to a single click; the event loop consumes this at
    ///   MouseDown to clear `left_double_click_pending`.
    ///
    /// The press transition consumes double-click demotion before classifying
    /// the release; suppression remains independent of the physical button.
    pub fn ignore_mouse_event(
        &mut self,
        click: bool,
        drag: bool,
        next_left_double_is_simple: bool,
    ) {
        if click {
            self.gestures.ignore_next_left_click = true;
        }
        if drag {
            self.gestures.ignore_next_drag = true;
        }
        self.gestures.next_left_double_is_simple = next_left_double_is_simple;
    }

    /// Clears the matching suppression flags.  Used by
    /// `perform_mouse_left_click` after it consumes the ignore-click,
    /// and by `perform_mouse_right_click` at the end of its body to
    /// drop any pending ignore state.
    pub fn accept_mouse_event(&mut self, click: bool, drag: bool) {
        if click {
            self.gestures.ignore_next_left_click = false;
        }
        if drag {
            self.gestures.ignore_next_drag = false;
        }
    }
}

#[cfg(test)]
mod gesture_lifecycle_tests {
    use super::*;

    fn press(input: &mut InputState, clicks: u8) {
        input.press_left_pointer(ScreenPoint::new(12.0, 34.0), clicks);
    }

    #[test]
    fn hit_publication_replaces_all_geometry_without_resetting_gestures() {
        let mut input = InputState::focused();
        press(&mut input, 2);
        input.controls.is_alt = true;
        input.feedback.draw_hidden = true;
        input.publish_spatial_hit(SpatialHit {
            selected_map_point: MapPoint::new(10.0, 20.0),
            selected_layer: 3,
            selected_sector_idx: Some(crate::fast_find_grid::SectorIndex::new(7).unwrap()),
            selected_patch_idx: Some(2),
            hovered_door_idx: Some(8),
            valid_position_for_move: true,
        });
        input.publish_spatial_hit(SpatialHit::default());
        let hit = input.spatial_hit();
        assert_eq!(hit.selected_map_point, MapPoint::default());
        assert_eq!(hit.selected_layer, 0);
        assert!(hit.selected_sector_idx.is_none());
        assert!(hit.selected_patch_idx.is_none());
        assert!(hit.hovered_door_idx.is_none());
        assert!(!hit.valid_position_for_move);
        assert!(input.controls.has_focus && input.controls.is_alt);
        assert!(input.feedback.draw_hidden);
        assert!(input.is_dragging());
        assert!(input.release_left_pointer());
    }

    #[test]
    fn cursor_overrides_preserve_sample_and_movement_eligibility() {
        let mut input = InputState::default();
        input.publish_spatial_hit(SpatialHit {
            selected_map_point: MapPoint::new(10.0, 20.0),
            selected_layer: 3,
            selected_patch_idx: Some(2),
            hovered_door_idx: Some(8),
            valid_position_for_move: true,
            ..SpatialHit::default()
        });
        let sector = crate::fast_find_grid::SectorIndex::new(7).unwrap();
        input.select_door_cursor_layer(9);
        input.select_jump_fallback_sector(Some(sector));
        assert_eq!(input.spatial_hit().selected_layer, 9);
        assert_eq!(input.spatial_hit().selected_sector_idx, Some(sector));
        assert_eq!(
            input.spatial_hit().selected_map_point,
            MapPoint::new(10.0, 20.0)
        );
        assert_eq!(input.spatial_hit().selected_patch_idx, Some(2));
        assert_eq!(input.spatial_hit().hovered_door_idx, Some(8));
        assert!(input.spatial_hit().valid_position_for_move);
    }

    #[test]
    fn cursor_frame_reset_does_not_erase_persistent_input_domains() {
        let mut input = InputState::focused();
        press(&mut input, 1);
        input.gestures.portrait_action_countdown = 5;
        input.feedback.draw_hidden = true;
        input.feedback.mouse_shadow_color = 42;
        input.feedback.display_door = true;
        input.publish_spatial_hit(SpatialHit {
            selected_layer: 4,
            ..SpatialHit::default()
        });
        input.begin_cursor_feedback(40);
        assert_eq!(input.feedback.mouse_opacity, 40);
        assert_eq!(input.feedback.mouse_shadow_color, 0);
        assert!(input.feedback.increment_cursor_animation);
        assert!(!input.feedback.display_door);
        assert!(input.feedback.draw_hidden);
        assert!(input.controls.has_focus && input.is_dragging());
        assert_eq!(input.gestures.portrait_action_countdown, 5);
        assert_eq!(input.spatial_hit().selected_layer, 4);
    }

    #[test]
    fn serialized_gestures_cannot_restore_live_pointer_authority() {
        let mut input = InputState::focused();
        press(&mut input, 2);
        let diagnostic = serde_json::to_value(&input.gestures).unwrap();
        assert!(serde_json::from_value::<PointerGestures>(diagnostic).is_err());
    }

    #[test]
    fn release_consumes_double_click_and_drag_suppression_once() {
        let mut input = InputState::default();
        press(&mut input, 2);
        input.drag_action_dispatched(true);
        assert!(input.left_mouse_down() && input.is_dragging());
        assert!(input.release_left_pointer());
        assert!(!input.left_mouse_down() && !input.is_dragging());
        assert!(!input.ignore_next_drag());
        assert!(input.consume_suppressed_click());
        assert!(!input.consume_suppressed_click());
        assert!(!input.release_left_pointer());
    }

    #[test]
    fn double_click_demotion_is_consumed_by_next_press_even_if_single() {
        for clicks in [1, 2] {
            let mut input = InputState::default();
            input.demote_next_double_click();
            press(&mut input, clicks);
            assert!(!input.release_left_pointer());
            press(&mut input, 2);
            assert!(input.release_left_pointer());
        }
    }

    #[test]
    fn action_rearm_disarms_drag_without_releasing_physical_button() {
        let mut input = InputState::default();
        press(&mut input, 2);
        input.disarm_left_drag();
        assert!(input.left_mouse_down());
        assert!(!input.is_dragging());
        assert!(input.release_left_pointer());
        press(&mut input, 1);
        assert!(input.is_dragging());
    }

    #[test]
    fn simultaneous_selection_buttons_preserve_the_other_mode() {
        let mut input = InputState::default();
        input.start_multi_selection(MapPoint::new(1.0, 2.0));
        input.start_multi_unselection(MapPoint::new(3.0, 4.0));
        assert!(input.multi_selection_active() && input.multi_unselection_active());
        assert_eq!(input.multi_selection_pt1(), MapPoint::new(3.0, 4.0));
        input.latch_selection_outline();
        input.cancel_multi_selection();
        assert!(!input.multi_selection_active() && input.multi_unselection_active());
        assert!(!input.draw_multi_selection());
        input.start_multi_selection(MapPoint::default());
        input.cancel_multi_unselection();
        assert!(input.multi_selection_active() && !input.multi_unselection_active());
        input.cancel_selection_gestures();
        assert!(!input.multi_selection_active() && !input.multi_unselection_active());
    }

    #[test]
    fn selection_outline_latches_until_cancellation_even_if_rectangle_shrinks() {
        let mut input = InputState::default();
        input.start_multi_selection(MapPoint::default());
        input.latch_selection_outline();
        input.update_multi_selection(MapPoint::default());
        assert!(input.draw_multi_selection());
        input.cancel_selection_for_swordfight();
        assert!(!input.multi_selection_active());
        assert!(
            input.draw_multi_selection(),
            "legacy swordfight cancellation preserves latch"
        );
        input.cancel_selection_gestures();
        assert!(!input.draw_multi_selection());
    }

    #[test]
    fn modal_reset_preserves_button_and_pending_double_but_clears_drag() {
        let mut input = InputState::default();
        press(&mut input, 2);
        input.ignore_mouse_event(true, true, true);
        input.start_multi_selection(MapPoint::default());
        input.controls.is_alt = true;
        input.reset_modal_input();
        assert!(input.left_mouse_down() && !input.is_dragging());
        assert!(!input.ignore_next_drag() && !input.ignore_next_left_click());
        assert!(!input.multi_selection_active() && !input.controls.is_alt);
        assert!(input.release_left_pointer());
        press(&mut input, 2);
        assert!(
            !input.release_left_pointer(),
            "modal reset preserves demotion"
        );
    }

    #[test]
    fn frontend_reset_preserves_suppression_but_touch_takeover_discards_it() {
        let mut input = InputState::default();
        press(&mut input, 2);
        input.ignore_mouse_event(true, true, true);
        input.reset_pointer_sequence();
        assert!(!input.left_mouse_down() && !input.is_dragging());
        assert!(input.ignore_next_drag() && input.ignore_next_left_click());
        assert!(
            input.release_left_pointer(),
            "frontend reset retains pending release classification"
        );
        input.cancel_left_pointer();
        assert!(!input.ignore_next_drag() && !input.ignore_next_left_click());
        assert!(!input.release_left_pointer());
        press(&mut input, 2);
        assert!(input.release_left_pointer());
    }

    #[test]
    fn suppression_accumulates_but_demotion_is_replaced() {
        let mut input = InputState::default();
        input.ignore_mouse_event(true, true, true);
        input.ignore_mouse_event(false, false, false);
        assert!(input.ignore_next_drag() && input.ignore_next_left_click());
        press(&mut input, 2);
        assert!(input.release_left_pointer());
        assert!(input.consume_suppressed_click());
    }
}
