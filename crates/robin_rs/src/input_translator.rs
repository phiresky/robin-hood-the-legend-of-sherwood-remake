//! Game input translator.
//!
//! Translates raw keyboard state and mouse position/wheel into high-level
//! [`GameAction`]s, returning the actions for the caller to dispatch.

use bitflags::bitflags;
use geo::Rect;
use serde::{Deserialize, Serialize};

use robin_engine::coordinates::ScreenPoint;

// ---------------------------------------------------------------------------
// SDL scancodes used for hardcoded key checks.
// These match the SDL_Scancode enum values (SDL_scancode.h).
// ---------------------------------------------------------------------------

// -- Existing key constants used by translation logic --
const SDL_SCANCODE_TAB: u16 = 43;
const SDL_SCANCODE_ESCAPE: u16 = 41;
const SDL_SCANCODE_PRINTSCREEN: u16 = 70;
const SDL_SCANCODE_PAUSE: u16 = 72;
const SDL_SCANCODE_HOME: u16 = 74;
const SDL_SCANCODE_F7: u16 = 64;
const SDL_SCANCODE_KP_ENTER: u16 = 88;
const SDL_SCANCODE_GRAVE: u16 = 53;
const SDL_SCANCODE_LCTRL: u16 = 224;
const SDL_SCANCODE_LALT: u16 = 226;
const SDL_SCANCODE_RCTRL: u16 = 228;

// -- Additional scancode constants for default bindings --
const SDL_SCANCODE_A: u16 = 4;
const SDL_SCANCODE_C: u16 = 6;
const SDL_SCANCODE_D: u16 = 7;
const SDL_SCANCODE_H: u16 = 11;
const SDL_SCANCODE_M: u16 = 16;
const SDL_SCANCODE_S: u16 = 22;
const SDL_SCANCODE_X: u16 = 27;
const SDL_SCANCODE_1: u16 = 30;
const SDL_SCANCODE_2: u16 = 31;
const SDL_SCANCODE_3: u16 = 32;
const SDL_SCANCODE_4: u16 = 33;
const SDL_SCANCODE_5: u16 = 34;
const SDL_SCANCODE_LSHIFT: u16 = 225;
const SDL_SCANCODE_PAGEUP: u16 = 75;
const SDL_SCANCODE_PAGEDOWN: u16 = 78;
const SDL_SCANCODE_LEFT: u16 = 80;
const SDL_SCANCODE_RIGHT: u16 = 79;
const SDL_SCANCODE_DOWN: u16 = 81;
const SDL_SCANCODE_UP: u16 = 82;
const SDL_SCANCODE_F2: u16 = 59;
const SDL_SCANCODE_F3: u16 = 60;
const SDL_SCANCODE_F5: u16 = 62;
const SDL_SCANCODE_F6: u16 = 63;
const SDL_SCANCODE_F8: u16 = 65;
const SDL_SCANCODE_F9: u16 = 66;
const SDL_SCANCODE_F12: u16 = 69;

// ---------------------------------------------------------------------------
// GameKey — bindable action slots
// ---------------------------------------------------------------------------

/// Bindable game key slots.  Each slot maps to a scancode via the key
/// configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
}

impl GameKey {
    pub const COUNT: usize = 37;

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
        Self::StartMission,
        Self::DisplayMenu,
        Self::RecordMovie,
        Self::PrintScreen,
        Self::DisplayConsole,
        Self::SlowMotion,
        Self::RequestInfo,
        Self::Teleport,
        Self::AiInfo,
    ];

    /// The action name string used in [`KeyConfig`](robin_assets::keyconfig::KeyConfig)
    /// bindings.
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
            Self::StartMission => "StartMission",
            Self::DisplayMenu => "DisplayMenu",
            Self::RecordMovie => "RecordMovie",
            Self::PrintScreen => "PrintScreen",
            Self::DisplayConsole => "DisplayConsole",
            Self::SlowMotion => "SlowMotion",
            Self::RequestInfo => "RequestInfo",
            Self::Teleport => "Teleport",
            Self::AiInfo => "AiInfo",
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

fn key_hit(cur: &[u8], prev: &[u8], sc: u16) -> bool {
    let i = sc as usize;
    i < cur.len() && i < prev.len() && cur[i] != 0 && prev[i] == 0
}

fn key_released(cur: &[u8], prev: &[u8], sc: u16) -> bool {
    let i = sc as usize;
    i < cur.len() && i < prev.len() && cur[i] == 0 && prev[i] != 0
}

fn key_held(cur: &[u8], sc: u16) -> bool {
    let i = sc as usize;
    i < cur.len() && cur[i] != 0
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

/// Maximum number of scancodes tracked (covers full SDL scancode range).
const MAX_SCANCODES: usize = 512;

/// Translates raw input events into [`GameAction`]s.
///
/// Maintains previous keyboard state for edge-detection and dead-zone
/// rectangles for mouse edge-scroll suppression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputTranslator {
    /// Scancode bound to each [`GameKey`] slot.
    bindings: Vec<u16>,
    /// Previous frame's keyboard state for edge detection.
    prev_keys: Vec<u8>,
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
            bindings: vec![0u16; GameKey::COUNT],
            prev_keys: vec![0u8; MAX_SCANCODES],
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
    /// Matches the original game's keyset1.cfg preset (converted from
    /// DirectInput DIK codes to SDL scancodes).
    fn set_default_bindings(&mut self) {
        // Camera
        self.bindings[GameKey::ZoomIn as usize] = SDL_SCANCODE_PAGEUP;
        self.bindings[GameKey::ZoomOut as usize] = SDL_SCANCODE_PAGEDOWN;
        self.bindings[GameKey::ScrollUp as usize] = SDL_SCANCODE_UP;
        self.bindings[GameKey::ScrollDown as usize] = SDL_SCANCODE_DOWN;
        self.bindings[GameKey::ScrollLeft as usize] = SDL_SCANCODE_LEFT;
        self.bindings[GameKey::ScrollRight as usize] = SDL_SCANCODE_RIGHT;

        // Map
        self.bindings[GameKey::DisplayMap as usize] = SDL_SCANCODE_M;

        // Character selection
        self.bindings[GameKey::SelectCharacter1 as usize] = SDL_SCANCODE_1;
        self.bindings[GameKey::SelectCharacter2 as usize] = SDL_SCANCODE_2;
        self.bindings[GameKey::SelectCharacter3 as usize] = SDL_SCANCODE_3;
        self.bindings[GameKey::SelectCharacter4 as usize] = SDL_SCANCODE_4;
        self.bindings[GameKey::SelectCharacter5 as usize] = SDL_SCANCODE_5;
        self.bindings[GameKey::SelectAll as usize] = SDL_SCANCODE_F2;
        self.bindings[GameKey::SelectNone as usize] = SDL_SCANCODE_F3;

        // Stance
        self.bindings[GameKey::CrouchDown as usize] = SDL_SCANCODE_C;
        self.bindings[GameKey::StandUp as usize] = SDL_SCANCODE_X;

        // Vision modifiers
        self.bindings[GameKey::ShowDoors as usize] = SDL_SCANCODE_LSHIFT;
        self.bindings[GameKey::SwitchHiddenDisplay as usize] = SDL_SCANCODE_H;
        self.bindings[GameKey::ShowViewCone as usize] = SDL_SCANCODE_LALT;

        // Action slots
        self.bindings[GameKey::Action1 as usize] = SDL_SCANCODE_A;
        self.bindings[GameKey::Action2 as usize] = SDL_SCANCODE_S;
        self.bindings[GameKey::Action3 as usize] = SDL_SCANCODE_D;
        self.bindings[GameKey::MoveDuringAction as usize] = SDL_SCANCODE_LCTRL;

        // Quick actions (macro recording)
        self.bindings[GameKey::RecordQa as usize] = SDL_SCANCODE_F5;
        self.bindings[GameKey::StartQa as usize] = SDL_SCANCODE_F6;
        self.bindings[GameKey::DeleteQa as usize] = SDL_SCANCODE_F8;

        // Save / Load
        self.bindings[GameKey::QuickSave1 as usize] = SDL_SCANCODE_F9;
        self.bindings[GameKey::QuickLoad1 as usize] = SDL_SCANCODE_F12;
    }

    /// Set the non-rebindable key bindings (console, print screen, menu,
    /// and debug keys).
    fn set_reserved_bindings(&mut self) {
        self.bindings[GameKey::DisplayConsole as usize] = SDL_SCANCODE_GRAVE;
        self.bindings[GameKey::PrintScreen as usize] = SDL_SCANCODE_PRINTSCREEN;
        self.bindings[GameKey::DisplayMenu as usize] = SDL_SCANCODE_ESCAPE;

        // Debug keys (only in non-shipping builds)
        self.bindings[GameKey::SlowMotion as usize] = SDL_SCANCODE_PAUSE;
        self.bindings[GameKey::Teleport as usize] = SDL_SCANCODE_F7;
        self.bindings[GameKey::RecordMovie as usize] = SDL_SCANCODE_KP_ENTER;
        self.bindings[GameKey::RequestInfo as usize] = SDL_SCANCODE_HOME;
    }

    // --- Binding management ---

    pub fn set_binding(&mut self, key: GameKey, scancode: u16) {
        self.bindings[key as usize] = scancode;
    }

    /// Apply the shipping-build deity easter egg rebind. The original
    /// uses raw DIK scancodes (0x46/0xc7/0x9c/0xcf); this port
    /// consistently uses SDL scancodes for the bindings table, so we
    /// translate to the SDL equivalents here (the rest of
    /// `set_default_bindings` / `set_reserved_bindings` already does
    /// the same mapping).
    ///
    /// Triggered from `EngineInner::run_console_command` via
    /// `ConsoleResponse::DeityInvoked`, drained by the host game loop.
    pub fn deity_call(&mut self) {
        // 0x46 DIK_SCROLL  → SDL_SCANCODE_SCROLLLOCK (71)
        self.bindings[GameKey::SlowMotion as usize] = 71;
        // 0xc7 DIK_HOME    → SDL_SCANCODE_HOME (74)
        self.bindings[GameKey::Teleport as usize] = SDL_SCANCODE_HOME;
        // 0x9c DIK_NUMPADENTER → SDL_SCANCODE_KP_ENTER (88)
        self.bindings[GameKey::RecordMovie as usize] = SDL_SCANCODE_KP_ENTER;
        // 0xcf DIK_END     → SDL_SCANCODE_END (77)
        self.bindings[GameKey::RequestInfo as usize] = 77;
        // The reference duplicates the SLOW_MOTION assignment;
        // preserved here as a redundant write for literal parity.
        self.bindings[GameKey::SlowMotion as usize] = 71;
    }

    pub fn get_binding(&self, key: GameKey) -> u16 {
        self.bindings[key as usize]
    }

    /// Load rebindable keys from a [`KeyConfig`](robin_assets::keyconfig::KeyConfig).
    ///
    /// Uses index-based loading — a raw copy from the key config's
    /// flat array.
    pub fn load_bindings_from_keyconfig(&mut self, cfg: &robin_assets::keyconfig::KeyConfig) {
        for i in 0..robin_assets::keyconfig::REAL_KEY_COUNT as usize {
            if i < self.bindings.len() {
                self.bindings[i] = cfg.get_key_by_index(i as u16);
            }
        }
        // Re-apply reserved bindings so they can't be overwritten by config.
        self.set_reserved_bindings();
    }

    /// Edge detection helper for scancodes that aren't routed through the
    /// standard [`GameAction`] translation path. Returns `true` on the
    /// frame the scancode transitions from up -> down. Must be called
    /// before [`Self::translate_keyboard`] advances `prev_keys`.
    pub fn was_scancode_pressed(&self, scancode: u16, current: &[u8]) -> bool {
        key_hit(current, &self.prev_keys, scancode)
    }

    /// Edge detection helper for scancodes that aren't routed through the
    /// standard [`GameAction`] translation path — e.g. the minimap
    /// accelerator, which is stored on the widget rather than bound to
    /// a [`GameAction`] variant.  Returns `true` on the frame the
    /// scancode transitions from down → up.  Must be called before
    /// [`Self::translate_keyboard`] (which advances `prev_keys`).
    pub fn was_scancode_released(&self, scancode: u16, current: &[u8]) -> bool {
        key_released(current, &self.prev_keys, scancode)
    }

    /// Look up which [`GameKey`] a scancode is bound to.
    pub fn translate_key(&self, scancode: u16) -> Option<GameKey> {
        if scancode == 0 {
            return None;
        }
        GameKey::ALL
            .iter()
            .copied()
            .find(|&gk| self.bindings[gk as usize] == scancode)
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

    pub fn is_user_locked(&self) -> bool {
        self.user_locked
    }

    // --- State reset ---

    /// Reset stored keyboard state.  Called when re-entering gameplay.
    pub fn reset_state(&mut self) {
        self.prev_keys.fill(0);
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

    /// Shorthand to get the scancode for a game key slot.
    fn sc(&self, gk: GameKey) -> u16 {
        self.bindings[gk as usize]
    }

    /// Translate a full keyboard state array into game actions.
    ///
    /// Call once per frame with the current key state (indexed by SDL
    /// scancode; non-zero = pressed). Updates internal previous-state
    /// for next-frame edge detection.
    pub fn translate_keyboard(&mut self, keys: &[u8], flags: TranslationFlags) -> Vec<GameAction> {
        let mut actions = Vec::new();
        let prev = &self.prev_keys;

        // --- Always-active keys ---
        if key_released(keys, prev, self.sc(GameKey::SlowMotion)) {
            actions.push(GameAction::SlowMotion);
        }
        if key_released(keys, prev, self.sc(GameKey::PrintScreen)) {
            actions.push(GameAction::PrintScreen);
        }

        // --- Ingame menu ---
        if flags.contains(TranslationFlags::INGAME_MENU)
            && key_released(keys, prev, self.sc(GameKey::DisplayMenu))
        {
            actions.push(GameAction::DisplayMenu);
        }

        // --- Quick load/save ---
        if flags.contains(TranslationFlags::QUICK_LOAD)
            && key_released(keys, prev, self.sc(GameKey::QuickLoad1))
        {
            actions.push(GameAction::QuickLoad);
        }
        if flags.contains(TranslationFlags::QUICK_SAVE)
            && key_released(keys, prev, self.sc(GameKey::QuickSave1))
        {
            actions.push(GameAction::QuickSave);
        }

        // --- Mission keys ---
        if flags.contains(TranslationFlags::MISSION) {
            if key_released(keys, prev, self.sc(GameKey::DisplayConsole)) {
                actions.push(GameAction::DisplayConsole);
            }

            // Alt+Tab or Ctrl+Esc → SwitchTask (raw scancodes, not bindings).
            if key_held(keys, SDL_SCANCODE_TAB) && key_held(keys, SDL_SCANCODE_LALT) {
                actions.push(GameAction::SwitchTask);
            }
            if key_held(keys, SDL_SCANCODE_ESCAPE)
                && (key_held(keys, SDL_SCANCODE_LCTRL) || key_held(keys, SDL_SCANCODE_RCTRL))
            {
                actions.push(GameAction::SwitchTask);
            }

            if key_released(keys, prev, self.sc(GameKey::RecordMovie)) {
                actions.push(GameAction::RecordMovie);
            }

            // --- User-unlocked mission keys ---
            if !self.user_locked {
                // Modifier key hit (down-edge) and release (up-edge) produce
                // separate actions. Order: all three hits first, then all
                // three releases (group-by-edge).
                let show_doors = self.sc(GameKey::ShowDoors);
                let view_cone = self.sc(GameKey::ShowViewCone);
                let move_action = self.sc(GameKey::MoveDuringAction);

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

                if key_released(keys, prev, self.sc(GameKey::Teleport)) {
                    actions.push(GameAction::Teleport);
                }
                if key_released(keys, prev, self.sc(GameKey::SwitchHiddenDisplay)) {
                    actions.push(GameAction::SwitchMaskedDisplay);
                }
                if key_released(keys, prev, self.sc(GameKey::AiInfo)) {
                    actions.push(GameAction::DisplayAiInfo);
                }

                // Macro keys
                if key_released(keys, prev, self.sc(GameKey::StartQa)) {
                    actions.push(GameAction::StartMacro);
                }
                if key_released(keys, prev, self.sc(GameKey::DeleteQa)) {
                    actions.push(GameAction::DeleteAllMacros);
                }
                // RECORD_QA keybind — clock-widget accelerator.
                if key_released(keys, prev, self.sc(GameKey::RecordQa)) {
                    actions.push(GameAction::RecordQa);
                }

                // Selection
                if key_released(keys, prev, self.sc(GameKey::SelectNone)) {
                    actions.push(GameAction::UnselectAll);
                }
                if key_released(keys, prev, self.sc(GameKey::SelectAll)) {
                    actions.push(GameAction::SelectAll);
                }

                // Stance
                if key_released(keys, prev, self.sc(GameKey::CrouchDown)) {
                    actions.push(GameAction::CrouchDown);
                }
                if key_released(keys, prev, self.sc(GameKey::StandUp)) {
                    actions.push(GameAction::StandUp);
                }

                if key_released(keys, prev, self.sc(GameKey::RequestInfo)) {
                    actions.push(GameAction::DisplayInfo);
                }

                // Scroll keys use held (continuous while pressed)
                if key_held(keys, self.sc(GameKey::ScrollLeft)) {
                    actions.push(GameAction::ScrollLeft);
                }
                if key_held(keys, self.sc(GameKey::ScrollRight)) {
                    actions.push(GameAction::ScrollRight);
                }
                if key_held(keys, self.sc(GameKey::ScrollUp)) {
                    actions.push(GameAction::ScrollUp);
                }
                if key_held(keys, self.sc(GameKey::ScrollDown)) {
                    actions.push(GameAction::ScrollDown);
                }

                // Zoom uses released (single trigger)
                if key_released(keys, prev, self.sc(GameKey::ZoomOut)) {
                    actions.push(GameAction::ZoomOut);
                }
                if key_released(keys, prev, self.sc(GameKey::ZoomIn)) {
                    actions.push(GameAction::ZoomIn);
                }

                // Action slots
                if key_released(keys, prev, self.sc(GameKey::Action1)) {
                    actions.push(GameAction::SelectAction { index: 0 });
                }
                if key_released(keys, prev, self.sc(GameKey::Action2)) {
                    actions.push(GameAction::SelectAction { index: 1 });
                }
                if key_released(keys, prev, self.sc(GameKey::Action3)) {
                    actions.push(GameAction::SelectAction { index: 2 });
                }

                // Character selection (portrait index 0–4).
                // We just emit the index — the caller resolves the entity.
                if key_released(keys, prev, self.sc(GameKey::SelectCharacter1)) {
                    actions.push(GameAction::SelectCharacter { portrait_index: 0 });
                }
                if key_released(keys, prev, self.sc(GameKey::SelectCharacter2)) {
                    actions.push(GameAction::SelectCharacter { portrait_index: 1 });
                }
                if key_released(keys, prev, self.sc(GameKey::SelectCharacter3)) {
                    actions.push(GameAction::SelectCharacter { portrait_index: 2 });
                }
                if key_released(keys, prev, self.sc(GameKey::SelectCharacter4)) {
                    actions.push(GameAction::SelectCharacter { portrait_index: 3 });
                }
                if key_released(keys, prev, self.sc(GameKey::SelectCharacter5)) {
                    actions.push(GameAction::SelectCharacter { portrait_index: 4 });
                }
            }
        }

        // Save current state as previous for next frame.
        self.prev_keys.resize(MAX_SCANCODES, 0);
        let copy_len = keys.len().min(MAX_SCANCODES);
        self.prev_keys[..copy_len].copy_from_slice(&keys[..copy_len]);
        if copy_len < MAX_SCANCODES {
            self.prev_keys[copy_len..].fill(0);
        }

        actions
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_translator() -> InputTranslator {
        let mut t = InputTranslator::new(1024.0, 768.0);
        // Bind some keys for testing
        t.set_binding(GameKey::ZoomIn, 10);
        t.set_binding(GameKey::ZoomOut, 11);
        t.set_binding(GameKey::ScrollLeft, 20);
        t.set_binding(GameKey::ScrollRight, 21);
        t.set_binding(GameKey::ScrollUp, 22);
        t.set_binding(GameKey::ScrollDown, 23);
        t.set_binding(GameKey::SelectCharacter1, 30);
        t.set_binding(GameKey::SelectAll, 35);
        t.set_binding(GameKey::Action1, 40);
        t.set_binding(GameKey::ShowDoors, 50);
        t.set_binding(GameKey::QuickSave1, 60);
        t.set_binding(GameKey::QuickLoad1, 61);
        t
    }

    /// Build a key state array with specific scancodes pressed.
    fn keys_down(scancodes: &[u16]) -> Vec<u8> {
        let mut keys = vec![0u8; MAX_SCANCODES];
        for &sc in scancodes {
            keys[sc as usize] = 1;
        }
        keys
    }

    #[test]
    fn translate_key_returns_bound_game_key() {
        let t = make_translator();
        assert_eq!(t.translate_key(10), Some(GameKey::ZoomIn));
        assert_eq!(t.translate_key(11), Some(GameKey::ZoomOut));
        assert_eq!(t.translate_key(30), Some(GameKey::SelectCharacter1));
    }

    #[test]
    fn translate_key_returns_none_for_unbound() {
        let t = make_translator();
        assert_eq!(t.translate_key(99), None);
    }

    #[test]
    fn translate_key_returns_none_for_zero_scancode() {
        let t = make_translator();
        assert_eq!(t.translate_key(0), None);
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
        // Frame 1: key 10 (ZoomIn) is pressed
        let frame1 = keys_down(&[10]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);

        // Frame 2: key 10 released → should produce ZoomIn action
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::ZoomIn));
    }

    #[test]
    fn raw_scancode_pressed_is_edge_triggered() {
        let mut t = make_translator();
        let frame1 = keys_down(&[55]);
        assert!(t.was_scancode_pressed(55, &frame1));

        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        assert!(!t.was_scancode_pressed(55, &frame1));
    }

    #[test]
    fn keyboard_held_scroll() {
        let mut t = make_translator();
        // Frame 1: start holding scroll left
        let frame1 = keys_down(&[20]);
        let actions = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::ScrollLeft));

        // Frame 2: still holding → still scrolling
        let actions = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::ScrollLeft));
    }

    #[test]
    fn keyboard_show_doors_hit_and_release() {
        let mut t = make_translator();
        // Frame 1: ShowDoors (50) just pressed → KeyShift
        let frame1 = keys_down(&[50]);
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
        // Press QuickSave key
        let frame1 = keys_down(&[60]);
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
        // Press and release SelectAll
        let frame1 = keys_down(&[35]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(!actions.contains(&GameAction::SelectAll));
    }

    #[test]
    fn keyboard_select_character() {
        let mut t = make_translator();
        let frame1 = keys_down(&[30]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::SelectCharacter { portrait_index: 0 }));
    }

    #[test]
    fn keyboard_select_action_index() {
        let mut t = make_translator();
        let frame1 = keys_down(&[40]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::SelectAction { index: 0 }));
    }

    #[test]
    fn reset_state_clears_previous_keys() {
        let mut t = make_translator();
        let frame1 = keys_down(&[10]);
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
        let mut cfg = robin_assets::keyconfig::KeyConfig::default();
        cfg.set_binding("ZoomIn", 0x49, 0x00);
        cfg.set_binding("ScrollUp", 0xC8, 0x00);

        t.load_bindings_from_keyconfig(&cfg);

        assert_eq!(t.get_binding(GameKey::ZoomIn), 0x49);
        assert_eq!(t.get_binding(GameKey::ScrollUp), 0xC8);
        // Reserved bindings survive
        assert_eq!(t.get_binding(GameKey::DisplayMenu), SDL_SCANCODE_ESCAPE);
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
        assert_eq!(back.get_binding(GameKey::ZoomIn), 10);
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
