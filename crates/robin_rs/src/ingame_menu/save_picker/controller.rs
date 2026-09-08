//! Common picker input policy. This controller emits intentions only; storage,
//! confirmation scheduling and Save-only text editing remain in the adapters.

use super::{ListRow, PickerModel};
use crate::gfx_types::{GameEvent, Keycode};
use crate::ingame_menu::layout::{MenuRect, MenuTransform};
use crate::ingame_menu::widget_bridge::{self, ModalInputState};
use crate::savegame::SlotName;
use crate::ui::{MouseButtons, UiEvent};
use crate::widget::FrameWnd;
use serde::{Deserialize, Serialize};

pub(in crate::ingame_menu) const ID_LOAD_SAVE: u32 = 0;
pub(in crate::ingame_menu) const ID_DELETE: u32 = 1;
pub(in crate::ingame_menu) const ID_CANCEL: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::ingame_menu) enum PickerTarget {
    New,
    Existing(SlotName),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::ingame_menu) enum PickerAction {
    Accept(PickerTarget),
    ConfirmDelete(SlotName),
    Cancel,
}

/// Runtime capture/widget ownership is not reconstructed from serialized data.
pub(in crate::ingame_menu) struct PickerController {
    pub input: ModalInputState,
    frame: FrameWnd,
    pending: Option<PickerAction>,
}

impl PickerController {
    pub fn new(input: ModalInputState) -> Self {
        Self {
            input,
            frame: FrameWnd::default(),
            pending: None,
        }
    }

    pub fn begin_frame(
        &mut self,
        model: &PickerModel,
        buttons: &[(u32, &str, i32, i32); 3],
        width: i32,
        height: i32,
    ) {
        self.pending = None;
        if self.frame.widget_count() == 0 {
            self.frame.enabled = true;
            self.frame.input_enabled = true;
            for (id, label, x, y) in buttons {
                self.frame.add_widget_absolute(widget_bridge::make_button(
                    *id, label, *x, *y, width, height,
                ));
            }
        }
        self.sync_enabled(model);
    }

    fn sync_enabled(&mut self, model: &PickerModel) {
        for (id, enabled) in [
            (ID_LOAD_SAVE, model.selected_row().is_some()),
            (ID_DELETE, model.can_delete()),
            (ID_CANCEL, true),
        ] {
            if !enabled && self.input.capture() == Some(id) {
                self.input
                    .as_widget_input()
                    .capture
                    .expect("modal input owns capture")
                    .clear();
            }
            let base = self
                .frame
                .widget_mut(id)
                .expect("picker button exists")
                .base_mut();
            base.enabled = enabled;
            if !enabled {
                base.state = crate::ui::UiState::Default;
            }
        }
    }

    /// Returns whether the adapter needs to resynchronize its save-name field.
    /// Keyboard/IME editing events remain available to that adapter unchanged.
    pub fn handle_event(
        &mut self,
        model: &mut PickerModel,
        event: &GameEvent,
        transform: MenuTransform,
        list: MenuRect,
        row_height: i32,
    ) -> bool {
        self.input.update_from_event(event, transform);
        if matches!(
            event,
            GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                }
        ) {
            self.pending = Some(PickerAction::Cancel);
            return false;
        }
        if self.pending.is_some() {
            return false;
        }
        let before = model.selected_row();
        match event {
            GameEvent::KeyDown {
                keycode: Keycode::Up,
                ..
            } => {
                model.navigate(false);
            }
            GameEvent::KeyDown {
                keycode: Keycode::Down,
                ..
            } => {
                model.navigate(true);
            }
            GameEvent::KeyDown {
                keycode: Keycode::Return | Keycode::KpEnter,
                ..
            } => self.activate(model, ID_LOAD_SAVE),
            GameEvent::MouseWheel(dy) if *dy != 0 => model.scroll(*dy < 0),
            GameEvent::MouseUp(x, y, 1) if self.input.capture().is_none() => {
                let (x, y) = transform.from_screen(*x, *y);
                if list.contains_virt(x, y) {
                    let offset = ((y - list.y - 4) / row_height).max(0) as usize;
                    model.select(model.row_at(model.scroll_offset() + offset));
                    if self.input.buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK) {
                        self.activate(model, ID_LOAD_SAVE);
                    }
                }
            }
            _ => {}
        }
        before != model.selected_row()
    }

    fn activate(&mut self, model: &PickerModel, id: u32) {
        if self.pending == Some(PickerAction::Cancel) {
            return;
        }
        let action = match id {
            ID_CANCEL => Some(PickerAction::Cancel),
            ID_LOAD_SAVE => match model.selected_row() {
                Some(ListRow::New) => Some(PickerAction::Accept(PickerTarget::New)),
                Some(ListRow::Existing(_)) => Some(PickerAction::Accept(PickerTarget::Existing(
                    model
                        .selected_slot()
                        .expect("existing selection has an identity")
                        .clone(),
                ))),
                None => None,
            },
            ID_DELETE if model.can_delete() => Some(PickerAction::ConfirmDelete(
                model
                    .selected_slot()
                    .expect("deletable selection has an identity")
                    .clone(),
            )),
            ID_DELETE => None,
            _ => panic!("unknown picker button {id}"),
        };
        if action.is_some() {
            self.pending = action;
        }
    }

    /// Text remains buffered until the Save adapter consumes it and explicitly
    /// calls `input.end_frame`. The button capture owner survives every frame.
    pub fn process_widgets(&mut self, model: &PickerModel) -> Vec<UiEvent> {
        self.sync_enabled(model);
        let events = self.frame.process_input(&self.input.as_widget_input());
        if let Some(id) = widget_bridge::find_activated(&events) {
            self.activate(model, id);
        }
        events
    }

    pub fn take_action(&mut self) -> Option<PickerAction> {
        self.pending.take()
    }
    pub fn frame(&self) -> &FrameWnd {
        &self.frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingame_menu::save_load::SaveLoadMode;
    use crate::ingame_menu::save_picker::PickerSlot;

    const BUTTONS: [(u32, &str, i32, i32); 3] = [
        (ID_LOAD_SAVE, "Load", 460, 300),
        (ID_DELETE, "Delete", 460, 350),
        (ID_CANCEL, "Cancel", 460, 400),
    ];
    const LIST: MenuRect = MenuRect {
        x: 30,
        y: 10,
        w: 420,
        h: 240,
    };
    fn key(keycode: Keycode) -> GameEvent {
        GameEvent::KeyDown {
            keycode,
            physical_key: None,
        }
    }
    fn model() -> PickerModel {
        PickerModel::new(
            SaveLoadMode::Load,
            false,
            2,
            (0..4)
                .map(|index| PickerSlot {
                    name: SlotName::new(format!("Savegame_{index:03}")).unwrap(),
                    manager_index: index,
                    special: false,
                    hidden_from_load: false,
                    autosave: false,
                    multiplayer_diagnostic: false,
                })
                .collect(),
        )
    }
    fn frame(
        controller: &mut PickerController,
        model: &mut PickerModel,
        events: &[GameEvent],
    ) -> Option<PickerAction> {
        controller.begin_frame(model, &BUTTONS, 150, 40);
        for event in events {
            controller.handle_event(model, event, MenuTransform::centered(640, 480), LIST, 20);
        }
        controller.process_widgets(model);
        controller.input.end_frame();
        controller.take_action()
    }

    #[test]
    fn paired_adapter_traces_share_navigation_accept_delete_and_cancel() {
        let traces = [
            vec![key(Keycode::Down), key(Keycode::Down), key(Keycode::Return)],
            vec![GameEvent::MouseMove {
                x: 480,
                y: 360,
                xrel: 0,
                yrel: 0,
            }],
            vec![GameEvent::MouseDown(480, 360, 1, 1)],
            vec![GameEvent::MouseUp(480, 360, 1)],
            vec![key(Keycode::Escape)],
            vec![GameEvent::Quit, key(Keycode::Return)],
        ];
        let mut cooperative = model();
        let mut standalone = cooperative.clone();
        let mut one_frame = PickerController::new(ModalInputState::new());
        let mut modal_loop = PickerController::new(ModalInputState::new());
        let mut actions = Vec::new();
        for events in traces {
            // The cooperative host may refresh its snapshot between ticks;
            // the standalone loop owns the same manager throughout the modal.
            cooperative.refresh(standalone.slots.clone());
            let action = frame(&mut one_frame, &mut cooperative, &events);
            assert_eq!(action, frame(&mut modal_loop, &mut standalone, &events));
            assert_eq!(cooperative, standalone);
            if let Some(action) = action {
                actions.push(action);
            }
        }
        let name = SlotName::new("Savegame_001").unwrap();
        assert_eq!(
            actions,
            vec![
                PickerAction::Accept(PickerTarget::Existing(name.clone())),
                PickerAction::ConfirmDelete(name),
                PickerAction::Cancel,
                PickerAction::Cancel
            ]
        );
    }

    #[test]
    fn pointer_capture_survives_frames_and_stationary_repeated_clicks() {
        let mut model = model();
        model.navigate(true);
        let mut controller = PickerController::new(ModalInputState::new());
        assert_eq!(
            frame(
                &mut controller,
                &mut model,
                &[GameEvent::MouseMove {
                    x: 480,
                    y: 360,
                    xrel: 0,
                    yrel: 0
                }]
            ),
            None
        );
        for _ in 0..2 {
            assert_eq!(
                frame(
                    &mut controller,
                    &mut model,
                    &[GameEvent::MouseDown(480, 360, 1, 1)]
                ),
                None
            );
            assert!(matches!(
                frame(
                    &mut controller,
                    &mut model,
                    &[GameEvent::MouseUp(480, 360, 1)]
                ),
                Some(PickerAction::ConfirmDelete(_))
            ));
        }
        frame(
            &mut controller,
            &mut model,
            &[GameEvent::MouseDown(480, 360, 1, 1)],
        );
        frame(
            &mut controller,
            &mut model,
            &[GameEvent::MouseMove {
                x: 35,
                y: 55,
                xrel: 0,
                yrel: 0,
            }],
        );
        assert_eq!(
            frame(
                &mut controller,
                &mut model,
                &[GameEvent::MouseUp(35, 55, 1)]
            ),
            None
        );
        assert_eq!(
            model.selected_slot().unwrap().as_str(),
            "Savegame_000",
            "button drag-off must not select an underlying list row"
        );
    }

    #[test]
    fn accept_intent_keeps_exact_identity_after_reordering() {
        let mut model = model();
        let mut controller = PickerController::new(ModalInputState::new());
        let action = frame(
            &mut controller,
            &mut model,
            &[key(Keycode::Down), key(Keycode::Return), key(Keycode::Down)],
        )
        .unwrap();
        model.slots.reverse();
        assert_eq!(
            action,
            PickerAction::Accept(PickerTarget::Existing(
                SlotName::new("Savegame_000").unwrap()
            ))
        );
    }

    #[test]
    fn disappearance_cancels_disabled_button_capture() {
        let mut model = model();
        model.navigate(true);
        let mut controller = PickerController::new(ModalInputState::new());
        frame(
            &mut controller,
            &mut model,
            &[GameEvent::MouseMove {
                x: 480,
                y: 360,
                xrel: 0,
                yrel: 0,
            }],
        );
        frame(
            &mut controller,
            &mut model,
            &[GameEvent::MouseDown(480, 360, 1, 1)],
        );
        assert_eq!(controller.input.capture(), Some(ID_DELETE));
        model.refresh(vec![]);
        assert_eq!(
            frame(
                &mut controller,
                &mut model,
                &[GameEvent::MouseUp(480, 360, 1)]
            ),
            None
        );
        assert_eq!(controller.input.capture(), None);
    }
}
