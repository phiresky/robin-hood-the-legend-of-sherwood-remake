//! Shared immutable keyboard for mouse-only widget fixtures.

use super::{MouseButtons, WidgetInput};
use crate::ui::UiKeyboard;

pub(super) fn mouse_input(x: f32, y: f32, buttons: MouseButtons) -> WidgetInput<'static> {
    static KEYBOARD: std::sync::LazyLock<UiKeyboard> =
        std::sync::LazyLock::new(UiKeyboard::default);
    WidgetInput {
        mouse_position: robin_engine::coordinates::ScreenPoint::new(x, y),
        mouse_z: 0,
        mouse_button: buttons,
        keyboard: &KEYBOARD,
        text_input: "",
        capture: None,
    }
}
