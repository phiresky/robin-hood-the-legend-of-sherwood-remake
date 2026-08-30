//! Game input translator.
//!
//! Translates raw keyboard state and mouse position/wheel into high-level
//! [`GameAction`]s, returning the actions for the caller to dispatch.

use robin_engine::coordinates::ScreenPoint;
use std::collections::BTreeSet;

use bitflags::bitflags;
use enum_map::{Enum, EnumMap};
use geo::Rect;
use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use crate::key_config::{KeyConfig, REAL_KEY_COUNT};

// ---------------------------------------------------------------------------
// GameKey — bindable action slots
// ---------------------------------------------------------------------------

/// Bindable game key slots.  Each slot maps to a physical key via the key
/// configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[repr(u16)]
pub enum GameKey {
    ZoomIn = 0,
    ZoomOut,

    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,

    DisplayMap,

    SelectCharacter1,
    SelectCharacter2,
    SelectCharacter3,
    SelectCharacter4,
    SelectCharacter5,
    SelectAll,
    SelectNone,

    CrouchDown,
    StandUp,

    ShowDoors,
    SwitchHiddenDisplay,

    Action1,
    Action2,
    Action3,

    MoveDuringAction,

    RecordQa,
    StartQa,
    DeleteQa,

    ShowViewCone,

    QuickSave1,
    QuickLoad1,
    ToggleCloak,

    // --- Non-rebindable / debug ---
    StartMission,
    DisplayMenu,

    RecordMovie,

    PrintScreen,
    DisplayConsole,

    SlowMotion,
    RequestInfo,
    Teleport,
    AiInfo,
    /// Built-in Sherwood-only shortcut for the post-port trading panel.
    SherwoodTrading,
}

impl GameKey {
    pub const COUNT: usize = 39;

    /// All variants in enum order.
    pub const ALL: [GameKey; Self::COUNT] = [
        Self::ZoomIn,
        Self::ZoomOut,
        Self::ScrollUp,
        Self::ScrollDown,
        Self::ScrollLeft,
        Self::ScrollRight,
        Self::DisplayMap,
        Self::SelectCharacter1,
        Self::SelectCharacter2,
        Self::SelectCharacter3,
        Self::SelectCharacter4,
        Self::SelectCharacter5,
        Self::SelectAll,
        Self::SelectNone,
        Self::CrouchDown,
        Self::StandUp,
        Self::ShowDoors,
        Self::SwitchHiddenDisplay,
        Self::Action1,
        Self::Action2,
        Self::Action3,
        Self::MoveDuringAction,
        Self::RecordQa,
        Self::StartQa,
        Self::DeleteQa,
        Self::ShowViewCone,
        Self::QuickSave1,
        Self::QuickLoad1,
        Self::ToggleCloak,
        Self::StartMission,
        Self::DisplayMenu,
        Self::RecordMovie,
        Self::PrintScreen,
        Self::DisplayConsole,
        Self::SlowMotion,
        Self::RequestInfo,
        Self::Teleport,
        Self::AiInfo,
        Self::SherwoodTrading,
    ];

    /// The action name string used in [`KeyConfig`] bindings.
    pub fn action_name(self) -> &'static str {
        match self {
            Self::ZoomIn => "ZoomIn",
            Self::ZoomOut => "ZoomOut",
            Self::ScrollUp => "ScrollUp",
            Self::ScrollDown => "ScrollDown",
            Self::ScrollLeft => "ScrollLeft",
            Self::ScrollRight => "ScrollRight",
            Self::DisplayMap => "DisplayMap",
            Self::SelectCharacter1 => "SelectCharacter1",
            Self::SelectCharacter2 => "SelectCharacter2",
            Self::SelectCharacter3 => "SelectCharacter3",
            Self::SelectCharacter4 => "SelectCharacter4",
            Self::SelectCharacter5 => "SelectCharacter5",
            Self::SelectAll => "SelectAll",
            Self::SelectNone => "SelectNone",
            Self::CrouchDown => "CrouchDown",
            Self::StandUp => "StandUp",
            Self::ShowDoors => "ShowDoors",
            Self::SwitchHiddenDisplay => "SwitchHiddenDisplay",
            Self::Action1 => "Action1",
            Self::Action2 => "Action2",
            Self::Action3 => "Action3",
            Self::MoveDuringAction => "MoveDuringAction",
            Self::RecordQa => "RecordQa",
            Self::StartQa => "StartQa",
            Self::DeleteQa => "DeleteQa",
            Self::ShowViewCone => "ShowViewCone",
            Self::QuickSave1 => "QuickSave1",
            Self::QuickLoad1 => "QuickLoad1",
            Self::ToggleCloak => "ToggleCloak",
            Self::StartMission => "StartMission",
            Self::DisplayMenu => "DisplayMenu",
            Self::RecordMovie => "RecordMovie",
            Self::PrintScreen => "PrintScreen",
            Self::DisplayConsole => "DisplayConsole",
            Self::SlowMotion => "SlowMotion",
            Self::RequestInfo => "RequestInfo",
            Self::Teleport => "Teleport",
            Self::AiInfo => "AiInfo",
            Self::SherwoodTrading => "SherwoodTrading",
        }
    }
}

// ---------------------------------------------------------------------------
// GameAction — output actions produced by translation
// ---------------------------------------------------------------------------

/// High-level game action produced by input translation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GameAction {
    // Camera
    ScrollLeft,
    ScrollRight,
    ScrollUp,
    ScrollDown,
    ZoomIn,
    ZoomOut,

    // Character selection
    SelectCharacter {
        portrait_index: u8,
    },
    SelectAll,
    UnselectAll,

    // Action slots
    SelectAction {
        index: u8,
    },

    // Modifier key press/release (ShowDoors, ShowViewCone, MoveDuringAction)
    KeyShift,
    KeyReleaseShift,
    KeyAlt,
    KeyReleaseAlt,
    KeyControl,
    KeyReleaseControl,

    // UI / Display
    DisplayMenu,
    DisplayConsole,
    DisplayInfo,
    DisplayAiInfo,
    /// Ask the host presentation to open the Sherwood trading panel.
    OpenSherwoodTrading,
    SwitchMaskedDisplay,
    PrintScreen,

    // Game control
    SlowMotion,
    Teleport,
    RecordMovie,
    QuickSave,
    QuickLoad,

    // Macros (quick-action recording)
    StartMacro,
    DeleteAllMacros,
    /// RECORD_QA keybind (default F5).  The accelerator that the
    /// clock widget binds; the consumer in `game_session.rs` replays
    /// the corner-clock left-click path — record / cycle the
    /// currently selected PC's macro slot.
    RecordQa,

    // Stance
    CrouchDown,
    StandUp,
    ToggleCloak,

    // System
    SwitchTask,
}

// ---------------------------------------------------------------------------
// TranslationFlags — controls which input categories are active
// ---------------------------------------------------------------------------

bitflags! {
    /// Controls which input categories are translated.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct TranslationFlags: u32 {
        const QUICK_LOAD = 0x01;
        const QUICK_SAVE = 0x02;
        const INGAME_MENU = 0x04;
        const MISSION = 0x08;

        /// All categories enabled (default).
        const ALL = 0x0F;
    }
}

// ---------------------------------------------------------------------------
// Key state edge-detection helpers
// ---------------------------------------------------------------------------

fn key_hit(cur: &BTreeSet<KeyCode>, prev: &BTreeSet<KeyCode>, key: Option<KeyCode>) -> bool {
    key.is_some_and(|key| cur.contains(&key) && !prev.contains(&key))
}

fn key_released(cur: &BTreeSet<KeyCode>, prev: &BTreeSet<KeyCode>, key: Option<KeyCode>) -> bool {
    key.is_some_and(|key| !cur.contains(&key) && prev.contains(&key))
}

fn key_held(cur: &BTreeSet<KeyCode>, key: Option<KeyCode>) -> bool {
    key.is_some_and(|key| cur.contains(&key))
}

// ---------------------------------------------------------------------------
// Dead zone helpers
// ---------------------------------------------------------------------------

/// Check if a point falls inside any dead zone rectangle (boundary-inclusive).
///
/// We use `Intersects` rather than `Contains` because `geo::Contains` for
/// `Rect` uses strict inequality and excludes boundary points — but dead zones
/// need to cover the exact screen edges where scrolling triggers.
fn is_in_dead_zone(dead_zones: &[Rect<f32>], point: ScreenPoint) -> bool {
    use geo::Intersects;
    let p = geo::Point::new(point.x, point.y);
    dead_zones.iter().any(|dz| dz.intersects(&p))
}

// ---------------------------------------------------------------------------
// InputTranslator
// ---------------------------------------------------------------------------

/// Translates raw input events into [`GameAction`]s.
///
/// Maintains previous keyboard state for edge-detection and dead-zone
/// rectangles for mouse edge-scroll suppression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputTranslator {
    /// Physical key bound to each [`GameKey`] slot.
    bindings: EnumMap<GameKey, Option<KeyCode>>,
    /// Previous frame's keyboard state for edge detection.
    prev_keys: BTreeSet<KeyCode>,
    /// Rectangular screen regions where mouse edge-scrolling is suppressed
    /// (e.g. UI panels along screen borders).
    dead_zones: Vec<Rect<f32>>,
    pub screen_width: f32,
    pub screen_height: f32,
    /// Whether the user is "locked" (UI modal, cutscene, etc.).
    user_locked: bool,
}

impl Default for InputTranslator {
    fn default() -> Self {
        Self {
            bindings: EnumMap::default(),
            prev_keys: BTreeSet::new(),
            dead_zones: Vec::new(),
            screen_width: 1024.0,
            screen_height: 768.0,
            user_locked: false,
        }
    }
}

impl InputTranslator {
    pub fn new(screen_width: f32, screen_height: f32) -> Self {
        let mut t = Self {
            screen_width,
            screen_height,
            ..Self::default()
        };
        t.set_default_bindings();
        t.set_reserved_bindings();
        t
    }

    /// Set the default rebindable key bindings.
    ///
    /// Matches the runtime Default1 preset used for new profiles.
    fn set_default_bindings(&mut self) {
        use KeyCode::*;
        // Camera
        self.bindings[GameKey::ZoomIn] = Some(PageUp);
        self.bindings[GameKey::ZoomOut] = Some(PageDown);
        self.bindings[GameKey::ScrollUp] = Some(ArrowUp);
        self.bindings[GameKey::ScrollDown] = Some(ArrowDown);
        self.bindings[GameKey::ScrollLeft] = Some(ArrowLeft);
        self.bindings[GameKey::ScrollRight] = Some(ArrowRight);

        // Map
        self.bindings[GameKey::DisplayMap] = Some(KeyM);

        // Character selection
        self.bindings[GameKey::SelectCharacter1] = Some(Digit1);
        self.bindings[GameKey::SelectCharacter2] = Some(Digit2);
        self.bindings[GameKey::SelectCharacter3] = Some(Digit3);
        self.bindings[GameKey::SelectCharacter4] = Some(Digit4);
        self.bindings[GameKey::SelectCharacter5] = Some(Digit5);
        self.bindings[GameKey::SelectAll] = Some(F2);
        self.bindings[GameKey::SelectNone] = Some(F3);

        // Stance
        self.bindings[GameKey::CrouchDown] = Some(KeyC);
        self.bindings[GameKey::StandUp] = Some(KeyX);

        // Vision modifiers
        self.bindings[GameKey::ShowDoors] = Some(ShiftLeft);
        self.bindings[GameKey::SwitchHiddenDisplay] = Some(KeyH);
        self.bindings[GameKey::ShowViewCone] = Some(AltLeft);

        // Action slots
        self.bindings[GameKey::Action1] = Some(KeyA);
        self.bindings[GameKey::Action2] = Some(KeyS);
        self.bindings[GameKey::Action3] = Some(KeyD);
        self.bindings[GameKey::MoveDuringAction] = Some(ControlLeft);

        // Quick actions (macro recording)
        self.bindings[GameKey::RecordQa] = Some(F5);
        self.bindings[GameKey::StartQa] = Some(F6);
        self.bindings[GameKey::DeleteQa] = Some(F8);

        // Save / Load
        self.bindings[GameKey::QuickSave1] = Some(F9);
        self.bindings[GameKey::QuickLoad1] = Some(F12);
        self.bindings[GameKey::ToggleCloak] = Some(KeyV);
    }

    /// Set the non-rebindable key bindings (console, print screen, menu,
    /// and debug keys).
    fn set_reserved_bindings(&mut self) {
        use KeyCode::*;
        self.bindings[GameKey::DisplayConsole] = Some(Backquote);
        self.bindings[GameKey::PrintScreen] = Some(PrintScreen);
        self.bindings[GameKey::DisplayMenu] = Some(Escape);
        self.bindings[GameKey::SherwoodTrading] = Some(KeyT);

        // Debug keys (only in non-shipping builds)
        self.bindings[GameKey::SlowMotion] = Some(Pause);
        self.bindings[GameKey::Teleport] = Some(F7);
        self.bindings[GameKey::RecordMovie] = Some(NumpadEnter);
        self.bindings[GameKey::RequestInfo] = Some(Home);
    }

    // --- Binding management ---

    pub fn set_binding(&mut self, key: GameKey, physical_key: Option<KeyCode>) {
        self.bindings[key] = physical_key;
    }

    /// Apply the shipping-build deity easter egg rebind. The original
    /// Triggered from `EngineInner::run_console_command` via
    /// `ConsoleResponse::DeityInvoked`, drained by the host game loop.
    pub fn deity_call(&mut self) {
        use KeyCode::*;
        self.bindings[GameKey::SlowMotion] = Some(ScrollLock);
        self.bindings[GameKey::Teleport] = Some(Home);
        self.bindings[GameKey::RecordMovie] = Some(NumpadEnter);
        self.bindings[GameKey::RequestInfo] = Some(End);
        // The reference duplicates the SLOW_MOTION assignment;
        // preserved here as a redundant write for literal parity.
        self.bindings[GameKey::SlowMotion] = Some(ScrollLock);
    }

    pub fn get_binding(&self, key: GameKey) -> Option<KeyCode> {
        self.bindings[key]
    }

    /// Load rebindable keys from a [`KeyConfig`].
    ///
    /// Uses index-based loading — a raw copy from the key config's
    /// flat array.
    pub fn load_bindings_from_keyconfig(&mut self, cfg: &KeyConfig) {
        for i in 0..REAL_KEY_COUNT as usize {
            if let Some(game_key) = GameKey::ALL.get(i).copied() {
                self.bindings[game_key] = cfg.get_key_by_index(i as u16);
            }
        }
        // Re-apply reserved bindings so they can't be overwritten by config.
        self.set_reserved_bindings();
    }

    /// Edge detection helper for physical keys that aren't routed through the
    /// standard [`GameAction`] translation path. Returns `true` on the
    /// frame the key transitions from up -> down. Must be called
    /// before [`Self::translate_keyboard`] advances `prev_keys`.
    pub fn was_key_pressed(&self, key: KeyCode, current: &BTreeSet<KeyCode>) -> bool {
        key_hit(current, &self.prev_keys, Some(key))
    }

    /// Edge detection helper for physical keys that aren't routed through the
    /// standard [`GameAction`] translation path — e.g. the minimap
    /// accelerator, which is stored on the widget rather than bound to
    /// a [`GameAction`] variant.  Returns `true` on the frame the
    /// key transitions from down to up.  Must be called before
    /// [`Self::translate_keyboard`] (which advances `prev_keys`).
    pub fn was_key_released(&self, key: KeyCode, current: &BTreeSet<KeyCode>) -> bool {
        key_released(current, &self.prev_keys, Some(key))
    }

    /// Look up which [`GameKey`] a physical key is bound to.
    pub fn translate_key(&self, key: KeyCode) -> Option<GameKey> {
        GameKey::ALL
            .iter()
            .copied()
            .find(|&gk| self.bindings[gk] == Some(key))
    }

    // --- Dead zones ---

    pub fn clear_dead_zones(&mut self) {
        self.dead_zones.clear();
    }

    /// Add a rectangular dead zone defined by two corner points.
    pub fn add_dead_zone(&mut self, a: ScreenPoint, b: ScreenPoint) {
        let min_x = a.x.min(b.x);
        let max_x = a.x.max(b.x);
        let min_y = a.y.min(b.y);
        let max_y = a.y.max(b.y);
        self.dead_zones.push(Rect::new(
            geo::coord! { x: min_x, y: min_y },
            geo::coord! { x: max_x, y: max_y },
        ));
    }

    /// Install the four HUD-adjacent edge-scroll dead-zone strips that
    /// keep the mouse from scrolling the viewport when the cursor is
    /// parked on or beside the bottom HUD panels. Called from
    /// post-initialize and on resolution change. `PANNEL_DEADZONE = 60`.
    pub fn install_hud_dead_zones(&mut self) {
        const PANNEL_DEADZONE: f32 = 60.0;
        let w = self.screen_width;
        let h = self.screen_height;

        self.clear_dead_zones();

        // Bottom-left vertical strip:
        //   ptA=(0, h-PANNEL_DEADZONE) .. ptB=(0, h-3)
        self.add_dead_zone(
            ScreenPoint::new(0.0, h - PANNEL_DEADZONE),
            ScreenPoint::new(0.0, h - 3.0),
        );
        // Bottom-left horizontal strip:
        //   ptA=(2, h-1) .. ptB=(PANNEL_DEADZONE, h-1)
        self.add_dead_zone(
            ScreenPoint::new(2.0, h - 1.0),
            ScreenPoint::new(PANNEL_DEADZONE, h - 1.0),
        );
        // Bottom-right horizontal strip:
        //   ptA=(w-PANNEL_DEADZONE, h-1) .. ptB=(w-3, h-1)
        self.add_dead_zone(
            ScreenPoint::new(w - PANNEL_DEADZONE, h - 1.0),
            ScreenPoint::new(w - 3.0, h - 1.0),
        );
        // Bottom-right vertical strip:
        //   ptA=(w-1, h-3) .. ptB=(w-1, h-PANNEL_DEADZONE)
        self.add_dead_zone(
            ScreenPoint::new(w - 1.0, h - 3.0),
            ScreenPoint::new(w - 1.0, h - PANNEL_DEADZONE),
        );
    }

    // --- User lock ---

    pub fn set_user_locked(&mut self, locked: bool) {
        self.user_locked = locked;
    }

    // --- State reset ---

    /// Reset stored keyboard state.  Called when re-entering gameplay.
    pub fn reset_state(&mut self) {
        self.prev_keys.clear();
    }

    // --- Mouse translation ---

    /// Translate mouse position and wheel into game actions.
    ///
    /// Edge-scrolling triggers when the cursor is within 1–2 pixels of
    /// a screen edge and not in a dead zone.
    pub fn translate_mouse(&self, x: f32, y: f32, wheel_delta: i16) -> Vec<GameAction> {
        let mut actions = Vec::new();

        if self.user_locked {
            return actions;
        }

        let point = ScreenPoint::new(x, y);

        if !is_in_dead_zone(&self.dead_zones, point) {
            if x <= 1.0 {
                tracing::trace!(x, y, "edge-scroll: Left");
                actions.push(GameAction::ScrollLeft);
            }
            if x >= self.screen_width - 2.0 {
                tracing::trace!(x, y, sw = self.screen_width, "edge-scroll: Right");
                actions.push(GameAction::ScrollRight);
            }
            if y <= 1.0 {
                tracing::trace!(x, y, "edge-scroll: Up");
                actions.push(GameAction::ScrollUp);
            }
            if y >= self.screen_height - 2.0 {
                tracing::trace!(x, y, sh = self.screen_height, "edge-scroll: Down");
                actions.push(GameAction::ScrollDown);
            }
        }

        if wheel_delta > 0 {
            actions.push(GameAction::ZoomIn);
        }
        if wheel_delta < 0 {
            actions.push(GameAction::ZoomOut);
        }

        actions
    }

    // --- Keyboard translation ---

    /// Shorthand to get the physical key for a game key slot.
    fn key(&self, gk: GameKey) -> Option<KeyCode> {
        self.bindings[gk]
    }

    /// Translate a full keyboard state array into game actions.
    ///
    /// Call once per frame with the current key state. Updates internal
    /// previous-state for next-frame edge detection.
    pub fn translate_keyboard(
        &mut self,
        keys: &BTreeSet<KeyCode>,
        flags: TranslationFlags,
    ) -> Vec<GameAction> {
        let mut actions = Vec::new();
        let prev = &self.prev_keys;

        // --- Always-active keys ---
        if key_released(keys, prev, self.key(GameKey::SlowMotion)) {
            actions.push(GameAction::SlowMotion);
        }
        if key_released(keys, prev, self.key(GameKey::PrintScreen)) {
            actions.push(GameAction::PrintScreen);
        }

        // --- Ingame menu ---
        if flags.contains(TranslationFlags::INGAME_MENU)
            && key_released(keys, prev, self.key(GameKey::DisplayMenu))
        {
            actions.push(GameAction::DisplayMenu);
        }

        // --- Quick load/save ---
        if flags.contains(TranslationFlags::QUICK_LOAD)
            && key_released(keys, prev, self.key(GameKey::QuickLoad1))
        {
            actions.push(GameAction::QuickLoad);
        }
        if flags.contains(TranslationFlags::QUICK_SAVE)
            && key_released(keys, prev, self.key(GameKey::QuickSave1))
        {
            actions.push(GameAction::QuickSave);
        }

        // --- Mission keys ---
        if flags.contains(TranslationFlags::MISSION) {
            if key_released(keys, prev, self.key(GameKey::DisplayConsole)) {
                actions.push(GameAction::DisplayConsole);
            }

            // Alt+Tab or Ctrl+Esc -> SwitchTask (physical keys, not bindings).
            if key_held(keys, Some(KeyCode::Tab)) && key_held(keys, Some(KeyCode::AltLeft)) {
                actions.push(GameAction::SwitchTask);
            }
            if key_held(keys, Some(KeyCode::Escape))
                && (key_held(keys, Some(KeyCode::ControlLeft))
                    || key_held(keys, Some(KeyCode::ControlRight)))
            {
                actions.push(GameAction::SwitchTask);
            }

            if key_released(keys, prev, self.key(GameKey::RecordMovie)) {
                actions.push(GameAction::RecordMovie);
            }

            // --- User-unlocked mission keys ---
            if !self.user_locked {
                // Modifier key hit (down-edge) and release (up-edge) produce
                // separate actions. Order: all three hits first, then all
                // three releases (group-by-edge).
                let show_doors = self.key(GameKey::ShowDoors);
                let view_cone = self.key(GameKey::ShowViewCone);
                let move_action = self.key(GameKey::MoveDuringAction);

                if key_hit(keys, prev, show_doors) {
                    actions.push(GameAction::KeyShift);
                }
                if key_hit(keys, prev, view_cone) {
                    actions.push(GameAction::KeyAlt);
                }
                if key_hit(keys, prev, move_action) {
                    actions.push(GameAction::KeyControl);
                }

                if key_released(keys, prev, show_doors) {
                    actions.push(GameAction::KeyReleaseShift);
                }
                if key_released(keys, prev, view_cone) {
                    actions.push(GameAction::KeyReleaseAlt);
                }
                if key_released(keys, prev, move_action) {
                    actions.push(GameAction::KeyReleaseControl);
                }

                if key_released(keys, prev, self.key(GameKey::Teleport)) {
                    actions.push(GameAction::Teleport);
                }
                if key_released(keys, prev, self.key(GameKey::SwitchHiddenDisplay)) {
                    actions.push(GameAction::SwitchMaskedDisplay);
                }
                if key_released(keys, prev, self.key(GameKey::AiInfo)) {
                    actions.push(GameAction::DisplayAiInfo);
                }
                let trading_key = self.key(GameKey::SherwoodTrading);
                if key_released(keys, prev, trading_key)
                    && trading_key.is_some_and(|key| {
                        self.translate_key(key) == Some(GameKey::SherwoodTrading)
                    })
                {
                    actions.push(GameAction::OpenSherwoodTrading);
                }

                // Macro keys
                if key_released(keys, prev, self.key(GameKey::StartQa)) {
                    actions.push(GameAction::StartMacro);
                }
                if key_released(keys, prev, self.key(GameKey::DeleteQa)) {
                    actions.push(GameAction::DeleteAllMacros);
                }
                // RECORD_QA keybind — clock-widget accelerator.
                if key_released(keys, prev, self.key(GameKey::RecordQa)) {
                    actions.push(GameAction::RecordQa);
                }

                // Selection
                if key_released(keys, prev, self.key(GameKey::SelectNone)) {
                    actions.push(GameAction::UnselectAll);
                }
                if key_released(keys, prev, self.key(GameKey::SelectAll)) {
                    actions.push(GameAction::SelectAll);
                }

                // Stance
                if key_released(keys, prev, self.key(GameKey::CrouchDown)) {
                    actions.push(GameAction::CrouchDown);
                }
                if key_released(keys, prev, self.key(GameKey::StandUp)) {
                    actions.push(GameAction::StandUp);
                }
                if key_released(keys, prev, self.key(GameKey::ToggleCloak)) {
                    actions.push(GameAction::ToggleCloak);
                }

                if key_released(keys, prev, self.key(GameKey::RequestInfo)) {
                    actions.push(GameAction::DisplayInfo);
                }

                // Scroll keys use held (continuous while pressed)
                if key_held(keys, self.key(GameKey::ScrollLeft)) {
                    actions.push(GameAction::ScrollLeft);
                }
                if key_held(keys, self.key(GameKey::ScrollRight)) {
                    actions.push(GameAction::ScrollRight);
                }
                if key_held(keys, self.key(GameKey::ScrollUp)) {
                    actions.push(GameAction::ScrollUp);
                }
                if key_held(keys, self.key(GameKey::ScrollDown)) {
                    actions.push(GameAction::ScrollDown);
                }

                // Zoom uses released (single trigger)
                if key_released(keys, prev, self.key(GameKey::ZoomOut)) {
                    actions.push(GameAction::ZoomOut);
                }
                if key_released(keys, prev, self.key(GameKey::ZoomIn)) {
                    actions.push(GameAction::ZoomIn);
                }

                // Action slots
                if key_released(keys, prev, self.key(GameKey::Action1)) {
                    actions.push(GameAction::SelectAction { index: 0 });
                }
                if key_released(keys, prev, self.key(GameKey::Action2)) {
                    actions.push(GameAction::SelectAction { index: 1 });
                }
                if key_released(keys, prev, self.key(GameKey::Action3)) {
                    actions.push(GameAction::SelectAction { index: 2 });
                }

                // Character selection (portrait index 0–4).
                // We just emit the index — the caller resolves the entity.
                if key_released(keys, prev, self.key(GameKey::SelectCharacter1)) {
                    actions.push(GameAction::SelectCharacter { portrait_index: 0 });
                }
                if key_released(keys, prev, self.key(GameKey::SelectCharacter2)) {
                    actions.push(GameAction::SelectCharacter { portrait_index: 1 });
                }
                if key_released(keys, prev, self.key(GameKey::SelectCharacter3)) {
                    actions.push(GameAction::SelectCharacter { portrait_index: 2 });
                }
                if key_released(keys, prev, self.key(GameKey::SelectCharacter4)) {
                    actions.push(GameAction::SelectCharacter { portrait_index: 3 });
                }
                if key_released(keys, prev, self.key(GameKey::SelectCharacter5)) {
                    actions.push(GameAction::SelectCharacter { portrait_index: 4 });
                }
            }
        }

        // Save current state as previous for next frame.
        self.prev_keys.clone_from(keys);

        actions
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use winit::keyboard::KeyCode;

    fn make_translator() -> InputTranslator {
        let mut t = InputTranslator::new(1024.0, 768.0);
        // Bind some keys for testing
        t.set_binding(GameKey::ZoomIn, Some(KeyCode::Equal));
        t.set_binding(GameKey::ZoomOut, Some(KeyCode::Minus));
        t.set_binding(GameKey::ScrollLeft, Some(KeyCode::ArrowLeft));
        t.set_binding(GameKey::ScrollRight, Some(KeyCode::ArrowRight));
        t.set_binding(GameKey::ScrollUp, Some(KeyCode::ArrowUp));
        t.set_binding(GameKey::ScrollDown, Some(KeyCode::ArrowDown));
        t.set_binding(GameKey::SelectCharacter1, Some(KeyCode::Digit1));
        t.set_binding(GameKey::SelectAll, Some(KeyCode::KeyQ));
        t.set_binding(GameKey::Action1, Some(KeyCode::KeyG));
        t.set_binding(GameKey::ShowDoors, Some(KeyCode::ShiftLeft));
        t.set_binding(GameKey::QuickSave1, Some(KeyCode::F1));
        t.set_binding(GameKey::QuickLoad1, Some(KeyCode::F5));
        t.set_binding(GameKey::ToggleCloak, Some(KeyCode::KeyV));
        t
    }

    fn keys_down(keys: &[KeyCode]) -> BTreeSet<KeyCode> {
        keys.iter().copied().collect()
    }

    #[test]
    fn translate_key_returns_bound_game_key() {
        let t = make_translator();
        assert_eq!(t.translate_key(KeyCode::Equal), Some(GameKey::ZoomIn));
        assert_eq!(t.translate_key(KeyCode::Minus), Some(GameKey::ZoomOut));
        assert_eq!(
            t.translate_key(KeyCode::Digit1),
            Some(GameKey::SelectCharacter1)
        );
    }

    #[test]
    fn translate_key_returns_none_for_unbound() {
        let t = make_translator();
        assert_eq!(t.translate_key(KeyCode::F24), None);
    }

    #[test]
    fn mouse_edge_scroll_left() {
        let t = make_translator();
        let actions = t.translate_mouse(0.0, 400.0, 0);
        assert!(actions.contains(&GameAction::ScrollLeft));
    }

    #[test]
    fn mouse_edge_scroll_right() {
        let t = make_translator();
        let actions = t.translate_mouse(1023.0, 400.0, 0);
        assert!(actions.contains(&GameAction::ScrollRight));
    }

    #[test]
    fn mouse_edge_scroll_up() {
        let t = make_translator();
        let actions = t.translate_mouse(500.0, 0.0, 0);
        assert!(actions.contains(&GameAction::ScrollUp));
    }

    #[test]
    fn mouse_edge_scroll_down() {
        let t = make_translator();
        let actions = t.translate_mouse(500.0, 767.0, 0);
        assert!(actions.contains(&GameAction::ScrollDown));
    }

    #[test]
    fn mouse_center_no_scroll() {
        let t = make_translator();
        let actions = t.translate_mouse(500.0, 400.0, 0);
        assert!(actions.is_empty());
    }

    #[test]
    fn mouse_wheel_zoom() {
        let t = make_translator();
        assert!(
            t.translate_mouse(500.0, 400.0, 1)
                .contains(&GameAction::ZoomIn)
        );
        assert!(
            t.translate_mouse(500.0, 400.0, -1)
                .contains(&GameAction::ZoomOut)
        );
    }

    #[test]
    fn mouse_locked_suppresses_all() {
        let mut t = make_translator();
        t.set_user_locked(true);
        let actions = t.translate_mouse(0.0, 0.0, 5);
        assert!(actions.is_empty());
    }

    #[test]
    fn mouse_dead_zone_suppresses_scroll() {
        let mut t = make_translator();
        t.add_dead_zone(ScreenPoint::new(0.0, 350.0), ScreenPoint::new(50.0, 450.0));
        // Point (0, 400) is in the dead zone → no scroll
        let actions = t.translate_mouse(0.0, 400.0, 0);
        assert!(!actions.contains(&GameAction::ScrollLeft));
    }

    #[test]
    fn mouse_dead_zone_does_not_suppress_wheel() {
        let mut t = make_translator();
        t.add_dead_zone(ScreenPoint::new(0.0, 0.0), ScreenPoint::new(1024.0, 768.0));
        // Wheel still works even inside dead zone
        let actions = t.translate_mouse(500.0, 400.0, 3);
        assert!(actions.contains(&GameAction::ZoomIn));
    }

    #[test]
    fn keyboard_released_triggers_action() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::Equal]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);

        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::ZoomIn));
    }

    #[test]
    fn cloak_key_is_rebindable_and_release_triggered() {
        let mut t = make_translator();
        let pressed = keys_down(&[KeyCode::KeyV]);
        assert!(
            !t.translate_keyboard(&pressed, TranslationFlags::ALL)
                .contains(&GameAction::ToggleCloak)
        );
        assert!(
            t.translate_keyboard(&BTreeSet::new(), TranslationFlags::ALL)
                .contains(&GameAction::ToggleCloak)
        );
    }

    #[test]
    fn raw_key_pressed_is_edge_triggered() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::Period]);
        assert!(t.was_key_pressed(KeyCode::Period, &frame1));

        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        assert!(!t.was_key_pressed(KeyCode::Period, &frame1));
    }

    #[test]
    fn keyboard_held_scroll() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::ArrowLeft]);
        let actions = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::ScrollLeft));

        // Frame 2: still holding → still scrolling
        let actions = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::ScrollLeft));
    }

    #[test]
    fn keyboard_show_doors_hit_and_release() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::ShiftLeft]);
        let actions = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::KeyShift));
        assert!(!actions.contains(&GameAction::KeyReleaseShift));

        // Frame 2: released → KeyReleaseShift
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::KeyReleaseShift));
        assert!(!actions.contains(&GameAction::KeyShift));
    }

    #[test]
    fn keyboard_flags_filter_categories() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::F1]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);

        // Release with QUICK_SAVE disabled → no action
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::MISSION);
        assert!(!actions.contains(&GameAction::QuickSave));
    }

    #[test]
    fn keyboard_user_locked_blocks_mission_keys() {
        let mut t = make_translator();
        t.set_user_locked(true);
        let frame1 = keys_down(&[KeyCode::KeyQ]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(!actions.contains(&GameAction::SelectAll));
    }

    #[test]
    fn keyboard_select_character() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::Digit1]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::SelectCharacter { portrait_index: 0 }));
    }

    #[test]
    fn keyboard_select_action_index() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::KeyG]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::SelectAction { index: 0 }));
    }

    #[test]
    fn reset_state_clears_previous_keys() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::Equal]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);

        t.reset_state();

        // After reset, releasing key 10 should NOT trigger (prev is cleared)
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(!actions.contains(&GameAction::ZoomIn));
    }

    #[test]
    fn clear_dead_zones() {
        let mut t = make_translator();
        t.add_dead_zone(ScreenPoint::new(0.0, 0.0), ScreenPoint::new(100.0, 100.0));
        assert!(!t.dead_zones.is_empty());
        t.clear_dead_zones();
        assert!(t.dead_zones.is_empty());
    }

    #[test]
    fn load_bindings_from_keyconfig() {
        let mut t = InputTranslator::new(1024.0, 768.0);
        let mut cfg = KeyConfig::default();
        cfg.set_binding("ZoomIn", Some(KeyCode::PageUp), None);
        cfg.set_binding("ScrollUp", Some(KeyCode::ArrowUp), None);

        t.load_bindings_from_keyconfig(&cfg);

        assert_eq!(t.get_binding(GameKey::ZoomIn), Some(KeyCode::PageUp));
        assert_eq!(t.get_binding(GameKey::ScrollUp), Some(KeyCode::ArrowUp));
        // Reserved bindings survive
        assert_eq!(t.get_binding(GameKey::DisplayMenu), Some(KeyCode::Escape));
        assert_eq!(t.get_binding(GameKey::SherwoodTrading), Some(KeyCode::KeyT));
    }

    #[test]
    fn built_in_t_shortcut_emits_typed_trading_action_on_release() {
        let mut t = InputTranslator::new(1024.0, 768.0);

        let pressed = keys_down(&[KeyCode::KeyT]);
        assert!(
            !t.translate_keyboard(&pressed, TranslationFlags::ALL)
                .contains(&GameAction::OpenSherwoodTrading)
        );

        let released = keys_down(&[]);
        assert!(
            t.translate_keyboard(&released, TranslationFlags::ALL)
                .contains(&GameAction::OpenSherwoodTrading)
        );
    }

    #[test]
    fn custom_binding_on_t_takes_precedence_over_trading_shortcut() {
        let mut t = InputTranslator::new(1024.0, 768.0);
        t.set_binding(GameKey::Action1, Some(KeyCode::KeyT));

        let pressed = keys_down(&[KeyCode::KeyT]);
        let _ = t.translate_keyboard(&pressed, TranslationFlags::ALL);
        let actions = t.translate_keyboard(&keys_down(&[]), TranslationFlags::ALL);

        assert!(actions.contains(&GameAction::SelectAction { index: 0 }));
        assert!(!actions.contains(&GameAction::OpenSherwoodTrading));
    }

    #[test]
    fn game_key_count_matches_all() {
        assert_eq!(GameKey::ALL.len(), GameKey::COUNT);
        // Verify each variant appears exactly once via its discriminant
        for (i, key) in GameKey::ALL.iter().enumerate() {
            assert_eq!(*key as usize, i);
        }
    }

    #[test]
    fn translation_flags_combine() {
        let flags = TranslationFlags::QUICK_LOAD | TranslationFlags::MISSION;
        assert!(flags.contains(TranslationFlags::QUICK_LOAD));
        assert!(flags.contains(TranslationFlags::MISSION));
        assert!(!flags.contains(TranslationFlags::QUICK_SAVE));
        assert!(!flags.contains(TranslationFlags::INGAME_MENU));
    }

    #[test]
    fn serde_round_trip_game_action() {
        let action = GameAction::SelectCharacter { portrait_index: 3 };
        let json = serde_json::to_string(&action).unwrap();
        let back: GameAction = serde_json::from_str(&json).unwrap();
        assert_eq!(action, back);
    }

    #[test]
    fn serde_round_trip_translator() {
        let mut t = make_translator();
        t.add_dead_zone(ScreenPoint::new(10.0, 20.0), ScreenPoint::new(30.0, 40.0));
        let json = serde_json::to_string(&t).unwrap();
        let back: InputTranslator = serde_json::from_str(&json).unwrap();
        assert_eq!(back.screen_width, 1024.0);
        assert_eq!(back.get_binding(GameKey::ZoomIn), Some(KeyCode::Equal));
        assert_eq!(back.dead_zones.len(), 1);
    }

    #[test]
    fn game_key_action_names_unique() {
        let names: Vec<&str> = GameKey::ALL.iter().map(|k| k.action_name()).collect();
        let mut deduped = names.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(names.len(), deduped.len(), "duplicate action names found");
    }
}
