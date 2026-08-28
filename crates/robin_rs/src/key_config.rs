//! Host-side keyboard binding configuration.
//!
//! Stores named action strings with primary and secondary key slots, and
//! provides hardcoded default presets. The original game owns the active and
//! custom bindings as part of each player profile and copies the active
//! profile's bindings into its input translator, so those live values are host
//! application state rather than deterministic engine state.
//!
//! The original preset definitions came from
//! `Data/Configuration/keyset1.cfg` and `keyset2.cfg`; the Rust port currently
//! preserves its existing hardcoded tables here.
//! TODO(architecture): load preset definitions through the asset layer while
//! keeping physical [`KeyCode`] values and per-profile selections host-owned.
//!
//! This unpublished workspace API previously lived at
//! `robin_assets::keyconfig`; consumers must now import
//! `robin_rs::key_config`.

use winit::keyboard::KeyCode;

/// A single action‐to‐key mapping.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct KeyBinding {
    pub action: String,
    pub primary_key: Option<KeyCode>,
    pub secondary_key: Option<KeyCode>,
}

/// The full set of key bindings.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct KeyConfig {
    pub bindings: Vec<KeyBinding>,
    /// Config type: `0` = Unknown, `1` = UserDefined, `2+` = PresetBase+index.
    pub key_type: u16,
}

// ── Index‐to‐action‐name mapping ──

/// Action names indexed 0..27.  Index 28 is `Dummy` (the sentinel — excluded
/// from [`REAL_KEY_COUNT`]).
const KEY_NAMES: &[&str] = &[
    "ZoomIn",
    "ZoomOut",
    "ScrollUp",
    "ScrollDown",
    "ScrollLeft",
    "ScrollRight",
    "Minimap",
    "Character1",
    "Character2",
    "Character3",
    "Character4",
    "Character5",
    "AllCharacters",
    "NoneCharacters",
    "Crouch",
    "StandUp",
    "GoBehindBuildings",
    "ToggleOutlineDisplay",
    "Action1",
    "Action2",
    "Action3",
    "MoveDuringAction",
    "RecordQuickAction",
    "StartQuickAction",
    "DeleteQuickAction",
    "ShowViewCone",
    "QuickSave1",
    "QuickLoad1",
    "ToggleCloak",
    "Dummy",
];

/// Number of real key bindings (excludes the Dummy sentinel).
pub const REAL_KEY_COUNT: u16 = (KEY_NAMES.len() - 1) as u16;
/// Total key name count including the Dummy sentinel.
pub const KEY_NAME_COUNT: u16 = KEY_NAMES.len() as u16;

impl KeyConfig {
    /// Insert or update a binding for `action`.
    pub fn set_binding(
        &mut self,
        action: &str,
        primary: Option<KeyCode>,
        secondary: Option<KeyCode>,
    ) {
        if let Some(b) = self.bindings.iter_mut().find(|b| b.action == action) {
            b.primary_key = primary;
            b.secondary_key = secondary;
        } else {
            self.bindings.push(KeyBinding {
                action: action.to_owned(),
                primary_key: primary,
                secondary_key: secondary,
            });
        }
    }

    /// Look up a binding by action name.
    pub fn get_binding(&self, action: &str) -> Option<&KeyBinding> {
        self.bindings.iter().find(|b| b.action == action)
    }

    /// Return the action name whose primary *or* secondary key matches `key`.
    pub fn get_action_for_key(&self, key: KeyCode) -> Option<&str> {
        self.bindings
            .iter()
            .find(|b| b.primary_key == Some(key) || b.secondary_key == Some(key))
            .map(|b| b.action.as_str())
    }

    // ── Index-based access ──

    /// Get the primary key for the binding at the given action index.
    /// Returns `None` if the index is out of range or the binding doesn't exist.
    pub fn get_key_by_index(&self, index: u16) -> Option<KeyCode> {
        KEY_NAMES
            .get(index as usize)
            .and_then(|name| self.get_binding(name))
            .and_then(|b| b.primary_key)
    }

    /// Set the primary key for the binding at the given action index.
    /// Preserves the existing secondary key if a binding already exists.
    pub fn set_key_by_index(&mut self, index: u16, key: Option<KeyCode>) {
        if let Some(&name) = KEY_NAMES.get(index as usize) {
            let secondary = self.get_binding(name).and_then(|b| b.secondary_key);
            self.set_binding(name, key, secondary);
        }
    }

    /// Reverse lookup: find the action index whose primary key matches `key`.
    /// Returns 0xFFFF if not found.
    pub fn get_index_for_key(&self, key: KeyCode) -> u16 {
        for (i, &name) in KEY_NAMES.iter().enumerate().take(REAL_KEY_COUNT as usize) {
            if let Some(b) = self.get_binding(name)
                && b.primary_key == Some(key)
            {
                return i as u16;
            }
        }
        0xFFFF
    }

    /// Copy all primary keys into a flat array, indexed by action.
    /// Fills up to `len` entries; missing bindings produce `None`.
    pub fn get_keys_array(&self, out: &mut [Option<KeyCode>]) {
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = KEY_NAMES
                .get(i)
                .and_then(|name| self.get_binding(name))
                .and_then(|b| b.primary_key);
        }
    }

    /// Load all primary keys from a flat array. Clears existing bindings and
    /// recreates them from the array.
    pub fn load_keys_array(&mut self, keys: &[Option<KeyCode>]) {
        self.bindings.clear();
        for (i, &key) in keys.iter().enumerate() {
            if let Some(&name) = KEY_NAMES.get(i) {
                self.bindings.push(KeyBinding {
                    action: name.to_owned(),
                    primary_key: key,
                    secondary_key: None,
                });
            }
        }
    }

    /// Returns the Default1 preset used as the seed for new profiles.
    pub fn default_preset() -> Self {
        use KeyCode::*;
        const DEFAULT_KEYS: [Option<KeyCode>; REAL_KEY_COUNT as usize] = [
            Some(NumpadAdd),      // ZoomIn
            Some(NumpadSubtract), // ZoomOut
            Some(ArrowUp),        // ScrollUp
            Some(ArrowDown),      // ScrollDown
            Some(ArrowLeft),      // ScrollLeft
            Some(ArrowRight),     // ScrollRight
            Some(Semicolon),      // Minimap
            Some(Digit1),         // Character1
            Some(Digit2),         // Character2
            Some(Digit3),         // Character3
            Some(Digit4),         // Character4
            Some(Digit5),         // Character5
            Some(KeyQ),           // AllCharacters
            Some(KeyD),           // NoneCharacters
            Some(KeyC),           // Crouch
            Some(KeyS),           // StandUp
            Some(ShiftLeft),      // GoBehindBuildings
            Some(CapsLock),       // ToggleOutlineDisplay
            Some(KeyG),           // Action1
            Some(KeyH),           // Action2
            Some(KeyJ),           // Action3
            Some(ControlLeft),    // MoveDuringAction
            Some(KeyA),           // RecordQuickAction
            Some(Space),          // StartQuickAction
            Some(Backspace),      // DeleteQuickAction
            Some(AltLeft),        // ShowViewCone
            Some(F1),             // QuickSave1
            Some(F5),             // QuickLoad1
            Some(KeyV),           // ToggleCloak
        ];

        let mut cfg = Self::default();
        cfg.load_keys_array(&DEFAULT_KEYS);
        cfg.key_type = 2; // PresetBase + 0
        cfg
    }

    /// Returns the "numpad-centric" Default2 preset.
    pub fn alternate_preset() -> Self {
        use KeyCode::*;
        const ALTERNATE_KEYS: [Option<KeyCode>; REAL_KEY_COUNT as usize] = [
            Some(NumpadAdd),      // ZoomIn
            Some(NumpadSubtract), // ZoomOut
            Some(ArrowUp),        // ScrollUp
            Some(ArrowDown),      // ScrollDown
            Some(ArrowLeft),      // ScrollLeft
            Some(ArrowRight),     // ScrollRight
            Some(NumpadMultiply), // Minimap
            Some(Numpad1),        // Character1
            Some(Numpad2),        // Character2
            Some(Numpad3),        // Character3
            Some(Numpad4),        // Character4
            Some(Numpad5),        // Character5
            Some(Numpad6),        // AllCharacters
            Some(Numpad0),        // NoneCharacters
            Some(PageDown),       // Crouch
            Some(PageUp),         // StandUp
            Some(ShiftRight),     // GoBehindBuildings
            Some(CapsLock),       // ToggleOutlineDisplay
            Some(Numpad7),        // Action1
            Some(Numpad8),        // Action2
            Some(Numpad9),        // Action3
            Some(ControlRight),   // MoveDuringAction
            Some(Enter),          // RecordQuickAction
            Some(Space),          // StartQuickAction
            Some(Backspace),      // DeleteQuickAction
            Some(AltRight),       // ShowViewCone
            Some(F1),             // QuickSave1
            Some(F5),             // QuickLoad1
            Some(KeyV),           // ToggleCloak
        ];

        let mut cfg = Self::default();
        cfg.load_keys_array(&ALTERNATE_KEYS);
        cfg.key_type = 3; // PresetBase + 1
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    #[test]
    fn set_and_get_binding() {
        let mut cfg = KeyConfig::default();
        cfg.set_binding("ZoomIn", Some(KeyCode::PageUp), None);
        let b = cfg.get_binding("ZoomIn").unwrap();
        assert_eq!(b.primary_key, Some(KeyCode::PageUp));
        assert_eq!(b.secondary_key, None);
    }

    #[test]
    fn update_existing_binding() {
        let mut cfg = KeyConfig::default();
        cfg.set_binding("ZoomIn", Some(KeyCode::PageUp), None);
        cfg.set_binding("ZoomIn", Some(KeyCode::PageDown), Some(KeyCode::Home));
        assert_eq!(cfg.bindings.len(), 1);
        let b = cfg.get_binding("ZoomIn").unwrap();
        assert_eq!(b.primary_key, Some(KeyCode::PageDown));
        assert_eq!(b.secondary_key, Some(KeyCode::Home));
    }

    #[test]
    fn get_binding_missing() {
        let cfg = KeyConfig::default();
        assert!(cfg.get_binding("NonExistent").is_none());
    }

    #[test]
    fn get_action_for_primary_key() {
        let mut cfg = KeyConfig::default();
        cfg.set_binding("ScrollUp", Some(KeyCode::ArrowUp), None);
        assert_eq!(cfg.get_action_for_key(KeyCode::ArrowUp), Some("ScrollUp"));
    }

    #[test]
    fn get_action_for_secondary_key() {
        let mut cfg = KeyConfig::default();
        cfg.set_binding("ScrollUp", Some(KeyCode::ArrowUp), Some(KeyCode::F11));
        assert_eq!(cfg.get_action_for_key(KeyCode::F11), Some("ScrollUp"));
    }

    #[test]
    fn get_action_for_key_missing() {
        let cfg = KeyConfig::default();
        assert!(cfg.get_action_for_key(KeyCode::F24).is_none());
    }

    #[test]
    fn default_and_alternate_presets_differ() {
        let default = KeyConfig::default_preset();
        let alternate = KeyConfig::alternate_preset();

        let mut default_keys = vec![None; REAL_KEY_COUNT as usize];
        let mut alt_keys = vec![None; REAL_KEY_COUNT as usize];
        default.get_keys_array(&mut default_keys);
        alternate.get_keys_array(&mut alt_keys);

        assert_ne!(
            default_keys, alt_keys,
            "Default1 and Default2 must produce different bindings"
        );
        assert_eq!(
            default.key_type, 2,
            "default_preset key_type = PresetBase+0"
        );
        assert_eq!(
            alternate.key_type, 3,
            "alternate_preset key_type = PresetBase+1"
        );
    }

    #[test]
    fn serde_round_trip() {
        let mut cfg = KeyConfig::default();
        cfg.set_binding(
            "Crouch",
            Some(KeyCode::ShiftLeft),
            Some(KeyCode::ShiftRight),
        );
        cfg.set_binding("Minimap", Some(KeyCode::KeyM), None);

        let json = serde_json::to_string(&cfg).unwrap();
        let restored: KeyConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.bindings.len(), 2);
        let b = restored.get_binding("Crouch").unwrap();
        assert_eq!(b.primary_key, Some(KeyCode::ShiftLeft));
        assert_eq!(b.secondary_key, Some(KeyCode::ShiftRight));
    }

    #[test]
    fn serialized_shape_remains_compatible_with_existing_keyconfigs() {
        let json = r#"{"bindings":[{"action":"Crouch","primary_key":"ShiftLeft","secondary_key":"ShiftRight"},{"action":"Minimap","primary_key":"KeyM","secondary_key":null}],"key_type":1}"#;

        let cfg: KeyConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.key_type, 1);
        assert_eq!(
            cfg.get_binding("Crouch").unwrap().primary_key,
            Some(KeyCode::ShiftLeft)
        );
        assert_eq!(serde_json::to_string(&cfg).unwrap(), json);
    }

    #[test]
    fn preset_bindings_remain_exact() {
        use KeyCode::*;

        let mut default_keys = vec![None; REAL_KEY_COUNT as usize];
        KeyConfig::default_preset().get_keys_array(&mut default_keys);
        assert_eq!(
            default_keys,
            vec![
                Some(NumpadAdd),
                Some(NumpadSubtract),
                Some(ArrowUp),
                Some(ArrowDown),
                Some(ArrowLeft),
                Some(ArrowRight),
                Some(Semicolon),
                Some(Digit1),
                Some(Digit2),
                Some(Digit3),
                Some(Digit4),
                Some(Digit5),
                Some(KeyQ),
                Some(KeyD),
                Some(KeyC),
                Some(KeyS),
                Some(ShiftLeft),
                Some(CapsLock),
                Some(KeyG),
                Some(KeyH),
                Some(KeyJ),
                Some(ControlLeft),
                Some(KeyA),
                Some(Space),
                Some(Backspace),
                Some(AltLeft),
                Some(F1),
                Some(F5),
                Some(KeyV),
            ]
        );

        let mut alternate_keys = vec![None; REAL_KEY_COUNT as usize];
        KeyConfig::alternate_preset().get_keys_array(&mut alternate_keys);
        assert_eq!(
            alternate_keys,
            vec![
                Some(NumpadAdd),
                Some(NumpadSubtract),
                Some(ArrowUp),
                Some(ArrowDown),
                Some(ArrowLeft),
                Some(ArrowRight),
                Some(NumpadMultiply),
                Some(Numpad1),
                Some(Numpad2),
                Some(Numpad3),
                Some(Numpad4),
                Some(Numpad5),
                Some(Numpad6),
                Some(Numpad0),
                Some(PageDown),
                Some(PageUp),
                Some(ShiftRight),
                Some(CapsLock),
                Some(Numpad7),
                Some(Numpad8),
                Some(Numpad9),
                Some(ControlRight),
                Some(Enter),
                Some(Space),
                Some(Backspace),
                Some(AltRight),
                Some(F1),
                Some(F5),
                Some(KeyV),
            ]
        );
    }
}
