//! Translation tables from winit keys and gilrs gamepad input to game events.

use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

#[cfg(feature = "gamepad")]
use super::GameWindow;
#[cfg(feature = "gamepad")]
use crate::gfx_types::GameEvent;
use crate::gfx_types::Keycode;

#[cfg(feature = "gamepad")]
impl GameWindow {
    pub(super) fn drain_gamepad_events(&mut self, events: &mut Vec<GameEvent>) {
        // Drain gilrs events to GameEvent::Gamepad{Added,Removed,Button,Axis}.
        if let Some(gilrs) = &mut self.gamepads {
            while let Some(gilrs::Event { id, event, .. }) = gilrs.next_event() {
                let which = usize::from(id) as u32;
                match event {
                    gilrs::EventType::Connected => {
                        events.push(GameEvent::GamepadAdded { which });
                    }
                    gilrs::EventType::Disconnected => {
                        events.push(GameEvent::GamepadRemoved { which });
                    }
                    gilrs::EventType::ButtonPressed(btn, _) => {
                        if let Some(b) = gilrs_button_to_index(btn) {
                            events.push(GameEvent::GamepadButton {
                                which,
                                button: b,
                                pressed: true,
                            });
                        }
                    }
                    gilrs::EventType::ButtonReleased(btn, _) => {
                        if let Some(b) = gilrs_button_to_index(btn) {
                            events.push(GameEvent::GamepadButton {
                                which,
                                button: b,
                                pressed: false,
                            });
                        }
                    }
                    gilrs::EventType::AxisChanged(axis, value, _) => {
                        if let Some(a) = gilrs_axis_to_index(axis) {
                            let v = (value * 32767.0).clamp(-32768.0, 32767.0) as i16;
                            events.push(GameEvent::GamepadAxis {
                                which,
                                axis: a,
                                value: v,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// Key mapping (unchanged from the pump-events implementation).
// ---------------------------------------------------------------------

pub(super) fn physical_key_to_key_code(key: PhysicalKey) -> Option<KeyCode> {
    match key {
        PhysicalKey::Code(c) => Some(c),
        PhysicalKey::Unidentified(_) => None,
    }
}

pub(super) fn is_android_back_key(logical_key: &Key, physical_key: PhysicalKey) -> bool {
    matches!(
        logical_key,
        Key::Named(NamedKey::BrowserBack | NamedKey::GoBack)
    ) || matches!(physical_key, PhysicalKey::Code(KeyCode::BrowserBack))
}

pub(super) fn physical_key_to_keycode(key: PhysicalKey) -> Keycode {
    use Keycode as K;
    let code = match key {
        PhysicalKey::Code(c) => c,
        PhysicalKey::Unidentified(_) => return K::Unknown,
    };
    match code {
        KeyCode::Escape => K::Escape,
        KeyCode::Enter => K::Return,
        KeyCode::NumpadEnter => K::KpEnter,
        KeyCode::Tab => K::Tab,
        KeyCode::Space => K::Space,
        KeyCode::Backspace => K::Backspace,
        KeyCode::Delete => K::Delete,
        KeyCode::Insert => K::Insert,
        KeyCode::ArrowUp => K::Up,
        KeyCode::ArrowDown => K::Down,
        KeyCode::ArrowLeft => K::Left,
        KeyCode::ArrowRight => K::Right,
        KeyCode::Home => K::Home,
        KeyCode::End => K::End,
        KeyCode::PageUp => K::PageUp,
        KeyCode::PageDown => K::PageDown,
        KeyCode::F1 => K::F1,
        KeyCode::F2 => K::F2,
        KeyCode::F3 => K::F3,
        KeyCode::F4 => K::F4,
        KeyCode::F5 => K::F5,
        KeyCode::F6 => K::F6,
        KeyCode::F7 => K::F7,
        KeyCode::F8 => K::F8,
        KeyCode::F9 => K::F9,
        KeyCode::F10 => K::F10,
        KeyCode::F11 => K::F11,
        KeyCode::F12 => K::F12,
        KeyCode::ShiftLeft => K::LShift,
        KeyCode::ShiftRight => K::RShift,
        KeyCode::ControlLeft => K::LCtrl,
        KeyCode::ControlRight => K::RCtrl,
        KeyCode::AltLeft => K::LAlt,
        KeyCode::AltRight => K::RAlt,
        KeyCode::KeyA => K::Char(b'a'),
        KeyCode::KeyB => K::Char(b'b'),
        KeyCode::KeyC => K::Char(b'c'),
        KeyCode::KeyD => K::Char(b'd'),
        KeyCode::KeyE => K::Char(b'e'),
        KeyCode::KeyF => K::Char(b'f'),
        KeyCode::KeyG => K::Char(b'g'),
        KeyCode::KeyH => K::Char(b'h'),
        KeyCode::KeyI => K::Char(b'i'),
        KeyCode::KeyJ => K::Char(b'j'),
        KeyCode::KeyK => K::Char(b'k'),
        KeyCode::KeyL => K::Char(b'l'),
        KeyCode::KeyM => K::Char(b'm'),
        KeyCode::KeyN => K::Char(b'n'),
        KeyCode::KeyO => K::Char(b'o'),
        KeyCode::KeyP => K::Char(b'p'),
        KeyCode::KeyQ => K::Char(b'q'),
        KeyCode::KeyR => K::Char(b'r'),
        KeyCode::KeyS => K::Char(b's'),
        KeyCode::KeyT => K::Char(b't'),
        KeyCode::KeyU => K::Char(b'u'),
        KeyCode::KeyV => K::Char(b'v'),
        KeyCode::KeyW => K::Char(b'w'),
        KeyCode::KeyX => K::Char(b'x'),
        KeyCode::KeyY => K::Char(b'y'),
        KeyCode::KeyZ => K::Char(b'z'),
        KeyCode::Digit0 => K::Char(b'0'),
        KeyCode::Digit1 => K::Char(b'1'),
        KeyCode::Digit2 => K::Char(b'2'),
        KeyCode::Digit3 => K::Char(b'3'),
        KeyCode::Digit4 => K::Char(b'4'),
        KeyCode::Digit5 => K::Char(b'5'),
        KeyCode::Digit6 => K::Char(b'6'),
        KeyCode::Digit7 => K::Char(b'7'),
        KeyCode::Digit8 => K::Char(b'8'),
        KeyCode::Digit9 => K::Char(b'9'),
        _ => K::Unknown,
    }
}

#[cfg(feature = "gamepad")]
fn gilrs_button_to_index(b: gilrs::Button) -> Option<u8> {
    use gilrs::Button as B;
    Some(match b {
        B::South => 0,
        B::East => 1,
        B::West => 2,
        B::North => 3,
        B::Select => 4,
        B::Mode => 5,
        B::Start => 6,
        B::LeftThumb => 7,
        B::RightThumb => 8,
        B::LeftTrigger => 9,
        B::RightTrigger => 10,
        B::DPadUp => 11,
        B::DPadDown => 12,
        B::DPadLeft => 13,
        B::DPadRight => 14,
        _ => return None,
    })
}

#[cfg(feature = "gamepad")]
fn gilrs_axis_to_index(a: gilrs::Axis) -> Option<u8> {
    use gilrs::Axis as A;
    Some(match a {
        A::LeftStickX => 0,
        A::LeftStickY => 1,
        A::RightStickX => 2,
        A::RightStickY => 3,
        A::LeftZ => 4,
        A::RightZ => 5,
        _ => return None,
    })
}
