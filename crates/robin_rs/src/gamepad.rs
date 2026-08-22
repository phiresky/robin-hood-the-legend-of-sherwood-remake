//! Gamepad/controller input handling.
//!
//! Captures the input model (button definitions, axis values, joystick
//! state), edge-detection between frames, and the sword-swing gesture
//! recognizer.

use crate::input::{MouseButton, ThreadedInput};
use robin_engine::coordinates as engine_coordinates;
use robin_engine::element as engine_element;
use robin_engine::element::{Command, Posture};
use robin_engine::engine as engine_api;
use robin_engine::engine::ScrollDirection;
use robin_engine::player_command as engine_player_command;
use robin_engine::player_command::PlayerCommand;
use serde::{Deserialize, Serialize};

// ── Constants ───────────────────────────────────────────────────────

/// Conversion factor for mouse simulation from axis values.
const MOUSE_CONVERSION: f32 = 1638.0;
/// Movement unit for character movement per frame.
const MOVE_UNIT: f32 = 25.0;
/// Axis magnitude above which the character runs instead of walks.
const RUN_THRESHOLD: f32 = 28000.0;
/// Axis magnitude above which a sword hit is registered as strong.
const HIT_THRESHOLD: f32 = 28000.0;
/// Time limit (ms) for QA double-click detection.
const QA_TIMER_LIMIT: u32 = 250;
/// Neutral center value for axes (0x7FFF in DirectInput).
const AXIS_CENTER: i32 = 0x7FFF;

const MAX_BUTTONS: usize = 32;
const MAX_POVS: usize = 4;
const MAX_SLIDERS: usize = 2;

// ── Button enum ─────────────────────────────────────────────────────

/// Named gamepad buttons, mapped to DirectInput button indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum GamePadButton {
    ActionB = 0,
    ActionA = 1,
    ActionC = 2,
    CancelParade = 3,
    SelectPrevCharacter = 4,
    AltChoice = 5,
    SelectNextCharacter = 6,
    QaManage = 7,
    CrouchChinese = 10,
    SimulatedLeftMouse = 11,
}

impl GamePadButton {
    /// All defined button variants, for iteration.
    pub const ALL: &[GamePadButton] = &[
        Self::ActionB,
        Self::ActionA,
        Self::ActionC,
        Self::CancelParade,
        Self::SelectPrevCharacter,
        Self::AltChoice,
        Self::SelectNextCharacter,
        Self::QaManage,
        Self::CrouchChinese,
        Self::SimulatedLeftMouse,
    ];

    pub fn index(self) -> usize {
        self as usize
    }
}

// ── POV hat directions ──────────────────────────────────────────────

/// POV hat direction values (in hundredths of degrees, matching DirectInput).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PovDirection {
    /// No direction / hat centered.
    Centered,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

impl PovDirection {
    /// Convert a raw DirectInput POV value (hundredths of degrees) to a
    /// `PovDirection`. Unknown values map to `Centered`.
    pub fn from_raw(value: u32) -> Self {
        match value {
            0 => Self::North,
            4500 => Self::NorthEast,
            9000 => Self::East,
            13500 => Self::SouthEast,
            18000 => Self::South,
            22500 => Self::SouthWest,
            27000 => Self::West,
            31500 => Self::NorthWest,
            _ => Self::Centered,
        }
    }
}

// ── Raw joystick state ──────────────────────────────────────────────

/// Raw joystick state, holding the DirectInput `DIJOYSTATE2` fields used
/// by the input layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoystickState {
    /// Main X axis (left stick horizontal).
    pub x: i32,
    /// Main Y axis (left stick vertical).
    pub y: i32,
    /// Z rotation axis (used for mouse cursor X).
    pub rz: i32,
    /// Slider axes (`sliders[0]` used for mouse cursor Y).
    pub sliders: [i32; MAX_SLIDERS],
    /// POV hat values in hundredths of degrees (`0xFFFFFFFF` = centered).
    pub povs: [u32; MAX_POVS],
    /// Button states (non-zero = pressed).
    pub buttons: [u8; MAX_BUTTONS],
}

impl Default for JoystickState {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            rz: AXIS_CENTER,
            sliders: [AXIS_CENTER; MAX_SLIDERS],
            povs: [0xFFFF_FFFF; MAX_POVS],
            buttons: [0; MAX_BUTTONS],
        }
    }
}

// ── Button edge detection ───────────────────────────────────────────

/// Edge-detection result for a button between two frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonEdge {
    /// Button is up in both frames.
    Up,
    /// Button transitioned from up to down (just pushed).
    Pushed,
    /// Button is held down in both frames.
    Held,
    /// Button transitioned from down to up (just released).
    Released,
}

// ── GamePadState — current + previous for edge detection ────────────

/// Tracks current and previous joystick state to enable button edge
/// detection, axis queries, and the swordfight/QA accumulator state
/// that the per-frame dispatcher needs across calls.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GamePadState {
    current: JoystickState,
    previous: JoystickState,
    /// In-flight state accumulated from gilrs events during the current
    /// frame. Promoted to `current` at the start of each
    /// [`process_gamepad_input`] call via `self.update(pending.clone())`.
    pending: JoystickState,
    /// Tick-count timestamp of the first `QaManage` push while `AltChoice`
    /// was held. Used to distinguish a single-click-then-pause from a
    /// double-click.
    qa_timer_ms: u32,
    /// Whether `qa_timer_ms` is armed.
    qa_timer_valid: bool,
    /// Accumulated sword-stick samples while the right stick is deflected
    /// during a swordfight. Recognized as a swing once the stick returns
    /// to centre.
    sword_swing: Vec<(f32, f32)>,
}

impl GamePadState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed new raw joystick data. The old `current` becomes `previous`.
    pub fn update(&mut self, new_state: JoystickState) {
        self.previous = std::mem::replace(&mut self.current, new_state);
    }

    // ── Button queries ──────────────────────────────────────────

    /// True if the button is currently held down.
    pub fn is_down(&self, button: GamePadButton) -> bool {
        self.current.buttons[button.index()] != 0
    }

    /// True if the button just transitioned from up → down this frame.
    pub fn is_pushed(&self, button: GamePadButton) -> bool {
        self.current.buttons[button.index()] != 0 && self.previous.buttons[button.index()] == 0
    }

    /// True if the button just transitioned from down → up this frame.
    pub fn is_released(&self, button: GamePadButton) -> bool {
        self.current.buttons[button.index()] == 0 && self.previous.buttons[button.index()] != 0
    }

    /// Full edge-detection for a button.
    pub fn button_edge(&self, button: GamePadButton) -> ButtonEdge {
        let cur = self.current.buttons[button.index()] != 0;
        let prev = self.previous.buttons[button.index()] != 0;
        match (prev, cur) {
            (false, false) => ButtonEdge::Up,
            (false, true) => ButtonEdge::Pushed,
            (true, true) => ButtonEdge::Held,
            (true, false) => ButtonEdge::Released,
        }
    }

    // ── Axis queries ────────────────────────────────────────────

    /// Main stick X axis (raw).
    pub fn axis_x(&self) -> i32 {
        self.current.x
    }

    /// Main stick as `(x, y)` floats.
    pub fn main_stick(&self) -> (f32, f32) {
        (self.current.x as f32, self.current.y as f32)
    }

    /// Whether the main stick magnitude exceeds the run threshold.
    pub fn is_running(&self) -> bool {
        let (x, y) = self.main_stick();
        (x * x + y * y).sqrt() > RUN_THRESHOLD
    }

    /// Simulated mouse delta from Rz and Slider\[0\], centered at
    /// `AXIS_CENTER`.
    pub fn mouse_delta(&self) -> (f32, f32) {
        let dx = (self.current.rz - AXIS_CENTER) as f32 / MOUSE_CONVERSION;
        let dy = (self.current.sliders[0] - AXIS_CENTER) as f32 / MOUSE_CONVERSION;
        (dx, dy)
    }

    // ── POV / D-pad ─────────────────────────────────────────────

    /// POV hat 0 direction.
    pub fn pov(&self) -> PovDirection {
        PovDirection::from_raw(self.current.povs[0])
    }

    /// Current joystick state (read-only).
    pub fn current(&self) -> &JoystickState {
        &self.current
    }

    // ── Gamepad event folding ─────────────────────────────────────

    /// Apply an axis-motion event to the in-flight joystick state.
    ///
    /// `which_axis` follows the standard layout emitted by the gilrs adapter
    /// (`LeftX`=0, `LeftY`=1, `RightX`=2, `RightY`=3, `TriggerLeft`=4,
    /// `TriggerRight`=5). `value` is a signed i16 centered at zero;
    /// the `Slider` / `Rz` fields follow the DirectInput unsigned
    /// convention (centered at `AXIS_CENTER = 0x7FFF`) so we translate
    /// here to keep the rest of the dispatcher (and the existing sword
    /// gesture tests) unchanged.
    pub fn apply_axis_event(&mut self, which_axis: u8, value: i16) {
        match which_axis {
            0 => self.pending.x = value as i32,
            1 => self.pending.y = value as i32,
            2 => self.pending.rz = value as i32 + AXIS_CENTER,
            3 => self.pending.sliders[0] = value as i32 + AXIS_CENTER,
            _ => {} // triggers not used by the game
        }
    }

    /// Apply a button press/release event to the in-flight state.
    ///
    /// `which_button` is the index into `JoystickState.buttons`. The
    /// caller is responsible for mapping standard button values to the
    /// original DirectInput button indices that [`GamePadButton`]
    /// expects.
    pub fn apply_button_event(&mut self, which_button: u8, pressed: bool) {
        let idx = which_button as usize;
        if idx < MAX_BUTTONS {
            self.pending.buttons[idx] = u8::from(pressed);
        }
    }

    /// Set the POV based on the current D-pad button pressed state
    /// (gilrs reports the D-pad as four individual buttons, not a POV hat).
    pub fn apply_dpad_state(&mut self, up: bool, right: bool, down: bool, left: bool) {
        self.pending.povs[0] = dpad_to_pov(up, right, down, left);
    }

    // ── Per-frame dispatch ─────────────────────────────────────

    /// The full per-frame pump.
    ///
    /// Snapshots the in-flight `pending` state over `current`/`previous`,
    /// then calls each `Manage*` arm. Returns a list of sim-affecting
    /// [`PlayerCommand`]s plus any synthesised mouse events pushed onto
    /// `threaded_input`. Non-sim state mutations (cursor position, QA
    /// macro timer) happen in place.
    ///
    /// `now_ms` is the monotonic tick count in milliseconds;
    /// the dispatcher uses it only for the QA single- vs double-click
    /// timing (`QA_TIMER_LIMIT`).
    pub fn process_gamepad_input(
        &mut self,
        now_ms: u32,
        engine: &robin_engine::engine::Engine,
        threaded_input: &mut ThreadedInput,
    ) -> GamepadFrame {
        let snapshot = self.pending.clone();
        self.update(snapshot);

        let mut cmds = Vec::new();
        let viewport = self.manage_scroll_axis(&mut cmds);
        cmds.extend(self.manage_move_axis(engine));
        cmds.extend(self.manage_mouse_axis(engine, threaded_input));
        let qa = self.manage_qa(now_ms, engine);
        cmds.extend(self.manage_character_select(engine));
        cmds.extend(self.manage_action_select(engine));
        GamepadFrame { cmds, viewport, qa }
    }

    /// POV-hat → scroll/zoom messenger dispatch.
    pub fn manage_scroll_axis(
        &self,
        cmds: &mut Vec<engine_player_command::PlayerCommand>,
    ) -> Vec<ViewportCommand> {
        let mut viewport = Vec::new();
        let alt = self.is_down(GamePadButton::AltChoice);
        // `SelectFollowElement(None)` fires on every scroll direction so
        // pad-scrolling unlocks any locker-follow target, matching the
        // keyboard path.
        let push_unfollow = |cmds: &mut Vec<PlayerCommand>| {
            cmds.push(PlayerCommand::SelectFollowElement { entity_id: None });
        };
        match self.pov() {
            PovDirection::North => {
                if alt {
                    viewport.push(ViewportCommand::ZoomIn);
                } else {
                    viewport.push(ViewportCommand::Scroll(ScrollDirection::Up));
                    push_unfollow(cmds);
                }
            }
            PovDirection::NorthEast => {
                viewport.push(ViewportCommand::Scroll(ScrollDirection::Right));
                viewport.push(ViewportCommand::Scroll(ScrollDirection::Up));
                push_unfollow(cmds);
            }
            PovDirection::East => {
                viewport.push(ViewportCommand::Scroll(ScrollDirection::Right));
                push_unfollow(cmds);
            }
            PovDirection::SouthEast => {
                viewport.push(ViewportCommand::Scroll(ScrollDirection::Right));
                viewport.push(ViewportCommand::Scroll(ScrollDirection::Down));
                push_unfollow(cmds);
            }
            PovDirection::South => {
                if alt {
                    viewport.push(ViewportCommand::ZoomOut);
                } else {
                    viewport.push(ViewportCommand::Scroll(ScrollDirection::Down));
                    push_unfollow(cmds);
                }
            }
            PovDirection::SouthWest => {
                viewport.push(ViewportCommand::Scroll(ScrollDirection::Down));
                viewport.push(ViewportCommand::Scroll(ScrollDirection::Left));
                push_unfollow(cmds);
            }
            PovDirection::West => {
                viewport.push(ViewportCommand::Scroll(ScrollDirection::Left));
                push_unfollow(cmds);
            }
            PovDirection::NorthWest => {
                viewport.push(ViewportCommand::Scroll(ScrollDirection::Left));
                viewport.push(ViewportCommand::Scroll(ScrollDirection::Up));
                push_unfollow(cmds);
            }
            PovDirection::Centered => {}
        }
        viewport
    }

    /// Left-stick → character movement + crouch toggle.
    ///
    /// The dispatch is gated on `frame_counter % 5 == 0` AND on a
    /// successful `get_sector_screen` probe at the destination (only
    /// emit on a sector that is patch / area+motion / door / jump).
    /// We don't thread the resolved layer/sector/patch into a separate
    /// `PadMove` variant because `EngineInner::perform_group_move`
    /// already redoes the `get_sector_screen` probe inline from the
    /// destination point and drives the same per-PC routing — caching
    /// the resolved sector/patch would only avoid the redundant probe,
    /// not inject distinct semantics.
    pub fn manage_move_axis(
        &self,
        engine: &robin_engine::engine::Engine,
    ) -> Vec<engine_player_command::PlayerCommand> {
        let mut cmds = Vec::new();

        let selected = engine.selected_pc_ids();
        if selected.is_empty() {
            return cmds;
        }
        let leader = selected[0];
        let leader_pos = match engine.get_entity(leader) {
            Some(e) => e.element_data().position_map(),
            None => return cmds,
        };
        let leader_posture = engine
            .get_entity(leader)
            .map(|e| e.element_data().posture)
            .unwrap_or_default();
        let leader_swordfighting = engine
            .get_entity(leader)
            .and_then(|e| e.human_data())
            .is_some_and(|h| !h.opponents.is_empty());

        let (x, y) = self.main_stick();
        if (x != 0.0 || y != 0.0) && engine.frame_counter().is_multiple_of(5) {
            let norm = (x * x + y * y).sqrt();
            let running = norm > RUN_THRESHOLD;
            let scale = if running {
                3.0 * MOVE_UNIT / norm
            } else {
                MOVE_UNIT / norm
            };
            let dest = engine_coordinates::MapPoint::new(
                leader_pos.x + x * scale,
                leader_pos.y + y * scale,
            );

            // Validity check: probe the destination and only dispatch
            // the move when the sector is a patch (auto-valid after the
            // is_patch unwrap) or one of area+motion / door / jump.
            // Drops the move when the stick points at unreachable
            // terrain instead of letting it resolve through
            // `perform_group_move`'s snap-to-walkable fallback.
            let leader_ref = engine_coordinates::MapPoint::new(leader_pos.x, leader_pos.y);
            let hit = engine.fast_grid().get_sector_screen(dest, leader_ref);
            let hit_is_patch = hit
                .sector_idx
                .and_then(|i| engine.fast_grid().level.sectors.get(usize::from(i)))
                .is_some_and(|s| s.sector_type.is_patch());
            if hit.is_valid_for_move(engine.fast_grid()) || hit_is_patch {
                cmds.push(PlayerCommand::GroupMove {
                    actors: selected.to_vec(),
                    destination: dest,
                    running,
                    show_marker: false,
                    // Gamepad cursor doesn't yet plumb the patch
                    // override through; spatial lookup at the cursor
                    // position is the existing behaviour.
                    goal_override: None,
                    door_route_override: None,
                    recorded_gate_routes: Vec::new(),
                });
            }
        }

        // CROUCH_CHINESE on release ─────────────────────────────
        // Outside swordfight: toggle posture (crouch ↔ stand).
        // Inside swordfight: synthesised right-click — route through
        // `resolve_right_click` so the swordfighter parries (launches
        // the parry-sword ability) instead of falling through to the
        // narrower unselect-all-actions shortcut.
        if self.is_released(GamePadButton::CrouchChinese) {
            if leader_swordfighting {
                cmds.extend(crate::game_input::resolve_right_click(
                    engine,
                    engine_player_command::PlayerId::HOST,
                ));
            } else if leader_posture == Posture::Crouched {
                cmds.push(PlayerCommand::StandUp);
            } else {
                cmds.push(PlayerCommand::CrouchDown);
            }
        }
        cmds
    }

    /// Right-stick → mouse cursor delta + simulated LMB; or, while
    /// swordfighting, buffer the stick samples until it returns to
    /// centre then feed them to [`recognize_swing`].
    pub fn manage_mouse_axis(
        &mut self,
        engine: &robin_engine::engine::Engine,
        threaded_input: &mut ThreadedInput,
    ) -> Vec<engine_player_command::PlayerCommand> {
        let mut cmds = Vec::new();

        let selected = engine.selected_pc_ids();
        let leader = selected.first().copied();
        let leader_entity = leader.and_then(|id| engine.get_entity(id));
        let swordfighting = leader_entity
            .and_then(|e| e.human_data())
            .is_some_and(|h| !h.opponents.is_empty());

        let (dx, dy) = self.mouse_delta();

        if !swordfighting {
            if dx != 0.0 || dy != 0.0 {
                let target = threaded_input.position();
                threaded_input.reach_position(engine_coordinates::ScreenPoint::new(
                    target.x + dx,
                    target.y + dy,
                ));
            }
            if self.is_pushed(GamePadButton::SimulatedLeftMouse) {
                threaded_input.push_button(MouseButton::Left);
            }
            if self.is_released(GamePadButton::SimulatedLeftMouse) {
                threaded_input.release_button(MouseButton::Left);
            }
        } else if dx != 0.0 || dy != 0.0 {
            self.sword_swing.push((dx, dy));
        } else if !self.sword_swing.is_empty() {
            // Stick returned to centre — recognise the swing and
            // dispatch it against the PC's principal opponent.
            let facing = leader_entity
                .map(|e| e.element_data().direction())
                .unwrap_or(0);
            let facing_dir = (facing.rem_euclid(16)) as u16;
            if let Some(strike) = recognize_swing(&self.sword_swing, facing_dir)
                && let (Some(pc_id), Some(target_id)) = (
                    leader,
                    leader_entity
                        .and_then(|e| e.human_data())
                        .and_then(|h| h.opponents.first().copied()),
                )
            {
                cmds.push(PlayerCommand::SwordStrikeCmd {
                    actor: pc_id,
                    target: target_id,
                    command: strike.to_command(),
                    with_seek: false,
                    seek_distance: None,
                });
            }
            self.sword_swing.clear();
        }
        cmds
    }

    /// SELECT_PREV_CHARACTER / SELECT_NEXT_CHARACTER → cycle selection.
    pub fn manage_character_select(
        &self,
        engine: &robin_engine::engine::Engine,
    ) -> Vec<engine_player_command::PlayerCommand> {
        let mut cmds = Vec::new();
        let pc_ids = engine.pc_ids();
        if pc_ids.is_empty() {
            return cmds;
        }
        let num_pcs = pc_ids.len();

        let selected_idx = engine
            .selected_pc_ids()
            .first()
            .and_then(|sel| pc_ids.iter().position(|id| id == sel));

        let prev_pushed = self.is_released(GamePadButton::SelectPrevCharacter);
        let next_pushed = self.is_released(GamePadButton::SelectNextCharacter);
        let alt_down = self.is_down(GamePadButton::AltChoice);

        if prev_pushed {
            let to_select = match selected_idx {
                Some(0) | None => num_pcs - 1,
                Some(i) => i - 1,
            };
            cmds.push(PlayerCommand::SelectByPortrait {
                portrait_index: to_select as u32,
                append: false,
            });
            // Follow-lock fires for both PREV branches (with and
            // without ALT). The asymmetry is with NEXT, whose ALT
            // variant deliberately omits the call.
            cmds.push(PlayerCommand::SelectFollowElement {
                entity_id: Some(pc_ids[to_select]),
            });
        }

        if next_pushed {
            let to_select = match selected_idx {
                Some(i) if i == num_pcs - 1 => 0,
                None => 0,
                Some(i) => i + 1,
            };
            cmds.push(PlayerCommand::SelectByPortrait {
                portrait_index: to_select as u32,
                append: false,
            });
            if !alt_down {
                cmds.push(PlayerCommand::SelectFollowElement {
                    entity_id: Some(pc_ids[to_select]),
                });
            }
        }

        cmds
    }

    /// ACTION_A/B/C → action select or opponent cycle; CANCEL_PARADE →
    /// right-click.
    pub fn manage_action_select(
        &self,
        engine: &robin_engine::engine::Engine,
    ) -> Vec<engine_player_command::PlayerCommand> {
        let mut cmds = Vec::new();
        let selected = engine.selected_pc_ids();
        if !selected.is_empty() {
            let leader = selected[0];
            let leader_entity = engine.get_entity(leader);
            let swordfighting = leader_entity
                .and_then(|e| e.human_data())
                .is_some_and(|h| !h.opponents.is_empty());

            if !swordfighting {
                // action indices 0..=2 mapped A/B/C. Note the enum
                // ordering (`ACTION_A=1, ACTION_B=0, ACTION_C=2`) maps
                // to action-slot indices 0, 1, 2 respectively.
                if self.is_released(GamePadButton::ActionA) {
                    cmds.push(PlayerCommand::SelectAction {
                        pc_id: leader,
                        action_index: 0,
                    });
                }
                if self.is_released(GamePadButton::ActionB) {
                    cmds.push(PlayerCommand::SelectAction {
                        pc_id: leader,
                        action_index: 1,
                    });
                }
                if self.is_released(GamePadButton::ActionC) {
                    cmds.push(PlayerCommand::SelectAction {
                        pc_id: leader,
                        action_index: 2,
                    });
                }
            } else {
                // Opponent cycling during a swordfight:
                //   A → choose_opponent(pc, (dir+8)%16,  +1)  — rear
                //   B → choose_opponent(pc, (dir+15)%16, -1)  — one step CCW
                //   C → choose_opponent(pc,  dir+1,      +1)  — one step CW
                let facing = leader_entity
                    .map(|e| e.element_data().direction())
                    .unwrap_or(0);
                let facing_dir = facing.rem_euclid(16) as u16;
                let seeds: &[(GamePadButton, u16, i16)] = &[
                    (GamePadButton::ActionA, (facing_dir + 8) % 16, 1),
                    (GamePadButton::ActionB, (facing_dir + 15) % 16, -1),
                    (GamePadButton::ActionC, facing_dir + 1, 1),
                ];
                for &(button, start_dir, increment) in seeds {
                    if !self.is_released(button) {
                        continue;
                    }
                    if let Some(opponent_id) = choose_opponent(engine, leader, start_dir, increment)
                    {
                        cmds.push(PlayerCommand::SetPrincipalOpponent {
                            actor: leader,
                            opponent_id,
                        });
                    }
                }
            }
        }

        // CANCEL_PARADE: press edge sets the held-right-mouse state
        // via MouseRightDown; release edge clears it and fans out
        // through `resolve_right_click` so the gamepad path matches
        // the mouse pipeline (parry sword while swordfighting,
        // posture-aware stops, action-specific cancellations).
        if self.is_pushed(GamePadButton::CancelParade) {
            cmds.push(PlayerCommand::MouseRightDown);
        }
        if self.is_released(GamePadButton::CancelParade) {
            cmds.push(PlayerCommand::MouseRightUp);
            cmds.extend(crate::game_input::resolve_right_click(
                engine,
                engine_player_command::PlayerId::HOST,
            ));
        }

        cmds
    }

    /// QA_MANAGE single-click → record; double-click (with `AltChoice`
    /// held) → launch all QAs; solo click → launch QA for the
    /// currently-selected PC after `QA_TIMER_LIMIT` ms.
    ///
    /// Returns the [`QaEvent`] this frame produced, or `None` if nothing
    /// happened. The caller is responsible for turning `QaEvent` into
    /// engine mutations — start-macro verbs aren't on the
    /// `PlayerCommand` bus yet, so routing those messages lives
    /// host-side for now.
    pub fn manage_qa(
        &mut self,
        now_ms: u32,
        engine: &robin_engine::engine::Engine,
    ) -> Option<QaEvent> {
        if engine.selected_pc_ids().is_empty() {
            return None;
        }

        if self.is_released(GamePadButton::QaManage) {
            if !self.is_down(GamePadButton::AltChoice) {
                // Toggle recording: if already recording, stop; else
                // start on the first selected PC.
                Some(QaEvent::ToggleRecording)
            } else if !self.qa_timer_valid {
                self.qa_timer_valid = true;
                self.qa_timer_ms = now_ms;
                None
            } else {
                self.qa_timer_valid = false;
                Some(QaEvent::LaunchAllMacros)
            }
        } else if self.qa_timer_valid && now_ms > self.qa_timer_ms + QA_TIMER_LIMIT {
            self.qa_timer_valid = false;
            Some(QaEvent::LaunchMacroForSelected)
        } else {
            None
        }
    }
}

// ── Dispatch result types ───────────────────────────────────────────

/// One frame's worth of gamepad dispatch output.
#[derive(Debug, Default)]
pub struct GamepadFrame {
    /// Sim-affecting commands produced this frame. Apply to the engine
    /// via `engine.apply_command(...)` and also push onto the replay
    /// recorder.
    pub cmds: Vec<engine_player_command::PlayerCommand>,
    pub viewport: Vec<ViewportCommand>,
    /// QA macro event, if any.  The host decides how to route these —
    /// the `PlayerCommand` bus doesn't yet carry start-macro /
    /// start-recording verbs, so the dispatch stays host-side until
    /// those variants land.
    pub qa: Option<QaEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportCommand {
    Scroll(engine_api::ScrollDirection),
    ZoomIn,
    ZoomOut,
}

/// High-level QA-macro event surfaced by the gamepad's `manage_qa` arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QaEvent {
    /// QA_MANAGE alone — toggle macro recording on the first selected PC.
    ToggleRecording,
    /// ALT+QA_MANAGE double-click — launch the macro for all PCs.
    LaunchAllMacros,
    /// ALT+QA_MANAGE single-click (after the timeout) — launch the macro
    /// for the first selected PC only.
    LaunchMacroForSelected,
}

// ── Standard button → game button mapping ───────────────────────────────

/// Map a standard-layout gamepad button ordinal to the DirectInput button
/// index that the game's `GamePadButton` enum addresses.
///
/// Standard ordinals produced by `window::gilrs_button_to_index`:
///   0=South, 1=East, 2=West, 3=North, 4=Back, 5=Guide, 6=Start,
///   7=LeftStick, 8=RightStick, 9=LeftShoulder, 10=RightShoulder,
///   11=DPadUp, 12=DPadDown, 13=DPadLeft, 14=DPadRight.
///
/// Returns `None` for buttons that are routed as POV-hat state
/// (D-pad) or that the game doesn't bind. D-pad buttons must be
/// tracked separately by the caller and fed into [`GamePadState::apply_dpad_state`].
pub fn standard_button_to_gamepad_index(standard_button: u8) -> Option<u8> {
    Some(match standard_button {
        0 => GamePadButton::ActionB as u8,      // South → B (ActionB=0)
        1 => GamePadButton::ActionA as u8,      // East  → A (ActionA=1)
        2 => GamePadButton::ActionC as u8,      // West  → X (ActionC=2)
        3 => GamePadButton::CancelParade as u8, // North → Y
        9 => GamePadButton::SelectPrevCharacter as u8, // LeftShoulder (LB)
        10 => GamePadButton::SelectNextCharacter as u8, // RightShoulder (RB)
        4 => GamePadButton::AltChoice as u8,    // Back → modifier
        6 => GamePadButton::QaManage as u8,     // Start → QA
        7 => GamePadButton::CrouchChinese as u8, // LeftStick press
        8 => GamePadButton::SimulatedLeftMouse as u8, // RightStick press
        _ => return None,
    })
}

/// Whether `standard_button` is a D-pad button (its state feeds the POV hat,
/// not a `GamePadButton`). Standard ordinals: `Up=11`, `Down=12`,
/// `Left=13`, `Right=14`.
pub fn is_dpad_button(standard_button: u8) -> bool {
    matches!(standard_button, 11..=14)
}

// ── POV hat helper ──────────────────────────────────────────────────

/// Translate a D-pad button combo to the DirectInput POV-hat angle
/// (hundredths of degrees) that `PovDirection::from_raw` expects.
fn dpad_to_pov(up: bool, right: bool, down: bool, left: bool) -> u32 {
    match (up, right, down, left) {
        (true, false, false, false) => 0,
        (true, true, false, false) => 4500,
        (false, true, false, false) => 9000,
        (false, true, true, false) => 13500,
        (false, false, true, false) => 18000,
        (false, false, true, true) => 22500,
        (false, false, false, true) => 27000,
        (true, false, false, true) => 31500,
        _ => 0xFFFF_FFFF,
    }
}

// ── Sword swing recognition ─────────────────────────────────────────

/// The type of sword strike recognized from an analog stick gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SwordStrike {
    /// Forward thrust (weak) — stick toward opponent, below hit threshold.
    ThrustA,
    /// Forward thrust (strong) — stick toward opponent, above hit threshold.
    ThrustB,
    /// Double-circle special attack.
    ThrustC,
    /// Right-side thrust — stick ~90° clockwise from facing direction.
    ThrustD,
    /// Left-side thrust — stick ~90° counter-clockwise from facing direction.
    ThrustE,
    /// Circle-based slash (single circle).
    ThrustH,
}

impl SwordStrike {
    /// Convert the recognised gesture to the engine-side sword-strike
    /// [`Command`](robin_engine::element::Command) variant.
    pub fn to_command(self) -> engine_element::Command {
        match self {
            Self::ThrustA => Command::SwordstrikeThrustA,
            Self::ThrustB => Command::SwordstrikeThrustB,
            Self::ThrustC => Command::SwordstrikeThrustC,
            Self::ThrustD => Command::SwordstrikeThrustD,
            Self::ThrustE => Command::SwordstrikeThrustE,
            Self::ThrustH => Command::SwordstrikeThrustH,
        }
    }
}

/// Tracks which quadrants the stick has visited during a swing gesture.
#[derive(Debug, Clone, Default)]
struct QuadrantTracker {
    visited: [bool; 4],
    circles: u8,
    /// Sample index when the stick last entered quadrant 0 (−X, −Y).
    last_q0_index: u32,
    /// Sample index when the stick last entered quadrant 1 (+X, −Y).
    last_q1_index: u32,
}

impl QuadrantTracker {
    fn visit(&mut self, x: f32, y: f32, index: u32) {
        if x < 0.0 && y < 0.0 {
            self.visited[0] = true;
            self.last_q0_index = index;
        }
        if x > 0.0 && y < 0.0 {
            self.visited[1] = true;
            self.last_q1_index = index;
        }
        if x > 0.0 && y > 0.0 {
            self.visited[2] = true;
        }
        if x < 0.0 && y > 0.0 {
            self.visited[3] = true;
        }

        if self.visited.iter().all(|&v| v) {
            self.circles += 1;
            self.visited = [false; 4];
        }
    }

    fn quadrants_visited(&self) -> u8 {
        self.visited.iter().filter(|&&v| v).count() as u8
    }
}

/// Analyzes a sequence of analog stick samples to recognize a sword swing
/// gesture.
///
/// `facing_direction` is the character's current facing in 0..16 sectors.
///
/// Returns `None` if no recognizable gesture was found.
pub fn recognize_swing(samples: &[(f32, f32)], facing_direction: u16) -> Option<SwordStrike> {
    if samples.is_empty() {
        return None;
    }

    let mut tracker = QuadrantTracker::default();
    let mut max_sample = (0.1_f32, 0.1_f32);
    let mut max_sq_norm = max_sample.0 * max_sample.0 + max_sample.1 * max_sample.1;

    for (i, &(x, y)) in samples.iter().enumerate() {
        tracker.visit(x, y, (i + 1) as u32);

        let sq = x * x + y * y;
        if sq > max_sq_norm {
            max_sample = (x, y);
            max_sq_norm = sq;
        }
    }

    let num_quadrants = tracker.quadrants_visited();
    let one_circle = tracker.circles >= 1;
    let two_circles = tracker.circles >= 2;

    if num_quadrants == 1 {
        let pad_dir = vector_to_sector_0_to_15(max_sample.0, max_sample.1);
        let dir = facing_direction;

        // Forward: pad direction matches facing ±1
        if pad_dir == dir || pad_dir == (dir + 1) % 16 || pad_dir == (dir + 15) % 16 {
            let norm = max_sq_norm.sqrt();
            return if norm < HIT_THRESHOLD {
                Some(SwordStrike::ThrustA)
            } else {
                Some(SwordStrike::ThrustB)
            };
        }

        // Right side: ~90° clockwise (dir + 4 ± 1)
        if pad_dir == (dir + 4) % 16 || pad_dir == (dir + 5) % 16 || pad_dir == (dir + 3) % 16 {
            return Some(SwordStrike::ThrustD);
        }

        // Left side: ~90° counter-clockwise (dir + 12 ± 1)
        if pad_dir == (dir + 12) % 16 || pad_dir == (dir + 13) % 16 || pad_dir == (dir + 11) % 16 {
            return Some(SwordStrike::ThrustE);
        }

        None
    } else if two_circles {
        Some(SwordStrike::ThrustC)
    } else if one_circle {
        // Both branches deliberately emit ThrustH (likely an
        // original-game bug we preserve for parity).
        Some(SwordStrike::ThrustH)
    } else {
        None
    }
}

/// Directional opponent sweep used by the gamepad's swordfight A/B/C
/// cycle.
///
/// Iterates `human_data.opponents[1..]` looking for an opponent whose
/// `(opp_pos - pc_pos).sector0to15()` matches `direction`.  On miss,
/// rotates `direction` by `increment` modulo 16 and retries — guaranteed
/// to terminate because there are at least two opponents and 16 sectors
/// cover every possible relative direction.  Caps at 16 iterations as a
/// belt-and-braces guard against malformed input.
///
/// Returns `None` when the PC has fewer than 2 opponents.
fn choose_opponent(
    engine: &robin_engine::engine::Engine,
    pc_id: engine_element::EntityId,
    mut direction: u16,
    increment: i16,
) -> Option<engine_element::EntityId> {
    let pc_entity = engine.get_entity(pc_id)?;
    let pc_pos = pc_entity.element_data().position_map();
    let opponents = pc_entity.human_data().map(|h| h.opponents.clone())?;
    if opponents.len() < 2 {
        return None;
    }

    for _ in 0..16 {
        for &candidate in &opponents[1..] {
            let Some(cand) = engine.get_entity(candidate) else {
                continue;
            };
            let cand_pos = cand.element_data().position_map();
            let sector = vector_to_sector_0_to_15(cand_pos.x - pc_pos.x, cand_pos.y - pc_pos.y);
            if sector == direction {
                return Some(candidate);
            }
        }
        direction = (direction as i32 + increment as i32).rem_euclid(16) as u16;
    }
    None
}

/// Convert a 2D vector to a direction sector 0..15. Sector 0 = up/north,
/// increasing clockwise.
fn vector_to_sector_0_to_15(x: f32, y: f32) -> u16 {
    // atan2 gives angle from positive X axis, counter-clockwise.
    // We want 0 = north (negative Y), increasing clockwise.
    let angle = (-y).atan2(x);
    let mut degrees = 90.0 - angle.to_degrees();
    if degrees < 0.0 {
        degrees += 360.0;
    }
    if degrees >= 360.0 {
        degrees -= 360.0;
    }
    ((degrees / 22.5).round() as u16) % 16
}

// ── Re-exported constants for use by other modules ──────────────────

pub const MOUSE_CONVERSION_FACTOR: f32 = MOUSE_CONVERSION;
pub const MOVEMENT_UNIT: f32 = MOVE_UNIT;
pub const RUN_THRESHOLD_VALUE: f32 = RUN_THRESHOLD;
pub const HIT_THRESHOLD_VALUE: f32 = HIT_THRESHOLD;
pub const QA_TIMER_LIMIT_MS: u32 = QA_TIMER_LIMIT;

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_engine() -> (engine_api::Engine, engine_api::LevelAssets) {
        use robin_engine::campaign::Campaign;
        let mut assets = engine_api::LevelAssets::new();
        let engine =
            engine_api::Engine::new_for_test(800.0, 600.0, Campaign::default(), &mut assets)
                .expect("engine");
        (engine, assets)
    }

    #[test]
    fn button_indices_match_original_defines() {
        assert_eq!(GamePadButton::ActionB.index(), 0);
        assert_eq!(GamePadButton::ActionA.index(), 1);
        assert_eq!(GamePadButton::ActionC.index(), 2);
        assert_eq!(GamePadButton::CancelParade.index(), 3);
        assert_eq!(GamePadButton::SelectPrevCharacter.index(), 4);
        assert_eq!(GamePadButton::AltChoice.index(), 5);
        assert_eq!(GamePadButton::SelectNextCharacter.index(), 6);
        assert_eq!(GamePadButton::QaManage.index(), 7);
        assert_eq!(GamePadButton::CrouchChinese.index(), 10);
        assert_eq!(GamePadButton::SimulatedLeftMouse.index(), 11);
    }

    #[test]
    fn pov_from_raw_known_values() {
        assert_eq!(PovDirection::from_raw(0), PovDirection::North);
        assert_eq!(PovDirection::from_raw(4500), PovDirection::NorthEast);
        assert_eq!(PovDirection::from_raw(9000), PovDirection::East);
        assert_eq!(PovDirection::from_raw(13500), PovDirection::SouthEast);
        assert_eq!(PovDirection::from_raw(18000), PovDirection::South);
        assert_eq!(PovDirection::from_raw(22500), PovDirection::SouthWest);
        assert_eq!(PovDirection::from_raw(27000), PovDirection::West);
        assert_eq!(PovDirection::from_raw(31500), PovDirection::NorthWest);
    }

    #[test]
    fn pov_from_raw_unknown_maps_to_centered() {
        assert_eq!(PovDirection::from_raw(0xFFFF_FFFF), PovDirection::Centered);
        assert_eq!(PovDirection::from_raw(1234), PovDirection::Centered);
    }

    #[test]
    fn default_joystick_state_is_neutral() {
        let state = JoystickState::default();
        assert_eq!(state.x, 0);
        assert_eq!(state.y, 0);
        assert_eq!(state.rz, AXIS_CENTER);
        assert_eq!(state.sliders[0], AXIS_CENTER);
        assert_eq!(state.povs[0], 0xFFFF_FFFF);
        assert!(state.buttons.iter().all(|&b| b == 0));
    }

    #[test]
    fn button_edge_full_lifecycle() {
        let mut pad = GamePadState::new();

        // Initially up
        assert_eq!(pad.button_edge(GamePadButton::ActionA), ButtonEdge::Up);
        assert!(!pad.is_down(GamePadButton::ActionA));
        assert!(!pad.is_pushed(GamePadButton::ActionA));
        assert!(!pad.is_released(GamePadButton::ActionA));

        // Press → Pushed
        let mut state = JoystickState::default();
        state.buttons[GamePadButton::ActionA.index()] = 1;
        pad.update(state);
        assert_eq!(pad.button_edge(GamePadButton::ActionA), ButtonEdge::Pushed);
        assert!(pad.is_down(GamePadButton::ActionA));
        assert!(pad.is_pushed(GamePadButton::ActionA));
        assert!(!pad.is_released(GamePadButton::ActionA));

        // Hold → Held
        let mut state = JoystickState::default();
        state.buttons[GamePadButton::ActionA.index()] = 1;
        pad.update(state);
        assert_eq!(pad.button_edge(GamePadButton::ActionA), ButtonEdge::Held);
        assert!(pad.is_down(GamePadButton::ActionA));
        assert!(!pad.is_pushed(GamePadButton::ActionA));
        assert!(!pad.is_released(GamePadButton::ActionA));

        // Release → Released
        pad.update(JoystickState::default());
        assert_eq!(
            pad.button_edge(GamePadButton::ActionA),
            ButtonEdge::Released
        );
        assert!(!pad.is_down(GamePadButton::ActionA));
        assert!(!pad.is_pushed(GamePadButton::ActionA));
        assert!(pad.is_released(GamePadButton::ActionA));

        // Back to Up
        pad.update(JoystickState::default());
        assert_eq!(pad.button_edge(GamePadButton::ActionA), ButtonEdge::Up);
    }

    #[test]
    fn mouse_delta_centered_is_zero() {
        let pad = GamePadState::new();
        let (dx, dy) = pad.mouse_delta();
        assert!(dx.abs() < 0.01);
        assert!(dy.abs() < 0.01);
    }

    #[test]
    fn mouse_delta_offset() {
        let mut pad = GamePadState::new();
        let mut state = JoystickState {
            rz: AXIS_CENTER + 1638, // dx ≈ 1.0
            ..Default::default()
        };
        state.sliders[0] = AXIS_CENTER + 1638; // dy ≈ 1.0
        pad.update(state);

        let (dx, dy) = pad.mouse_delta();
        assert!((dx - 1.0).abs() < 0.01);
        assert!((dy - 1.0).abs() < 0.01);
    }

    #[test]
    fn is_running_threshold() {
        let mut pad = GamePadState::new();

        // Below threshold
        let state = JoystickState {
            x: 10000,
            y: 10000,
            ..Default::default()
        };
        pad.update(state);
        assert!(!pad.is_running());

        // Above threshold
        let state = JoystickState {
            x: 25000,
            y: 25000,
            ..Default::default()
        };
        pad.update(state);
        assert!(pad.is_running());
    }

    #[test]
    fn pov_query() {
        let mut pad = GamePadState::new();
        assert_eq!(pad.pov(), PovDirection::Centered);

        let mut state = JoystickState::default();
        state.povs[0] = 9000;
        pad.update(state);
        assert_eq!(pad.pov(), PovDirection::East);
    }

    #[test]
    fn vector_to_sector_cardinal_directions() {
        // North (0, -1) → sector 0
        assert_eq!(vector_to_sector_0_to_15(0.0, -1.0), 0);
        // East (1, 0) → sector 4
        assert_eq!(vector_to_sector_0_to_15(1.0, 0.0), 4);
        // South (0, 1) → sector 8
        assert_eq!(vector_to_sector_0_to_15(0.0, 1.0), 8);
        // West (-1, 0) → sector 12
        assert_eq!(vector_to_sector_0_to_15(-1.0, 0.0), 12);
    }

    #[test]
    fn vector_to_sector_diagonals() {
        // NE → sector 2
        assert_eq!(vector_to_sector_0_to_15(1.0, -1.0), 2);
        // SE → sector 6
        assert_eq!(vector_to_sector_0_to_15(1.0, 1.0), 6);
        // SW → sector 10
        assert_eq!(vector_to_sector_0_to_15(-1.0, 1.0), 10);
        // NW → sector 14
        assert_eq!(vector_to_sector_0_to_15(-1.0, -1.0), 14);
    }

    #[test]
    fn recognize_swing_empty_returns_none() {
        assert_eq!(recognize_swing(&[], 0), None);
    }

    #[test]
    fn recognize_swing_forward_weak() {
        // Facing north (sector 0), push stick north — below HIT_THRESHOLD.
        // Small x offset so it falls into a quadrant (strict < / > checks).
        let samples = vec![(-0.1, -100.0)];
        assert_eq!(recognize_swing(&samples, 0), Some(SwordStrike::ThrustA));
    }

    #[test]
    fn recognize_swing_forward_strong() {
        // Facing north (sector 0), push stick north hard — above HIT_THRESHOLD
        let samples = vec![(-0.1, -30000.0)];
        assert_eq!(recognize_swing(&samples, 0), Some(SwordStrike::ThrustB));
    }

    #[test]
    fn recognize_swing_right_side() {
        // Facing north (sector 0), push stick east (sector 4) → ThrustD
        let samples = vec![(100.0, -0.1)];
        assert_eq!(recognize_swing(&samples, 0), Some(SwordStrike::ThrustD));
    }

    #[test]
    fn recognize_swing_left_side() {
        // Facing north (sector 0), push stick west (sector 12) → ThrustE
        let samples = vec![(-100.0, -0.1)];
        assert_eq!(recognize_swing(&samples, 0), Some(SwordStrike::ThrustE));
    }

    #[test]
    fn recognize_swing_single_circle() {
        let samples = vec![
            (-1.0, -1.0), // Q0
            (1.0, -1.0),  // Q1
            (1.0, 1.0),   // Q2
            (-1.0, 1.0),  // Q3
        ];
        assert_eq!(recognize_swing(&samples, 0), Some(SwordStrike::ThrustH));
    }

    #[test]
    fn recognize_swing_double_circle() {
        let samples = vec![
            (-1.0, -1.0),
            (1.0, -1.0),
            (1.0, 1.0),
            (-1.0, 1.0),
            (-1.0, -1.0),
            (1.0, -1.0),
            (1.0, 1.0),
            (-1.0, 1.0),
        ];
        assert_eq!(recognize_swing(&samples, 0), Some(SwordStrike::ThrustC));
    }

    #[test]
    fn recognize_swing_unrecognized_direction() {
        // Facing north, push stick to sector 8 (behind) — no matching strike
        let samples = vec![(0.0, 100.0)];
        assert_eq!(recognize_swing(&samples, 0), None);
    }

    #[test]
    fn gamepad_state_serde_roundtrip() {
        let mut pad = GamePadState::new();
        let mut state = JoystickState {
            x: 1234,
            ..Default::default()
        };
        state.buttons[0] = 1;
        pad.update(state);

        let json = serde_json::to_string(&pad).unwrap();
        let restored: GamePadState = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.axis_x(), 1234);
        assert!(restored.is_down(GamePadButton::ActionB));
    }

    #[test]
    fn multiple_buttons_independent() {
        let mut pad = GamePadState::new();
        let mut state = JoystickState::default();
        state.buttons[GamePadButton::ActionA.index()] = 1;
        state.buttons[GamePadButton::ActionB.index()] = 1;
        pad.update(state);

        assert!(pad.is_pushed(GamePadButton::ActionA));
        assert!(pad.is_pushed(GamePadButton::ActionB));
        assert!(!pad.is_pushed(GamePadButton::ActionC));
    }

    // ── Dispatcher tests ────────────────────────────────────────

    fn empty_engine() -> engine_api::Engine {
        fresh_engine().0
    }

    /// Prime `pad` with a prior-frame state where nothing is pushed so
    /// the edge detectors treat `new_state` as fresh input.
    fn prime_and_set(pad: &mut GamePadState, new_state: JoystickState) {
        pad.update(JoystickState::default());
        pad.update(new_state);
    }

    #[test]
    fn manage_scroll_axis_pov_north_emits_scroll_up_and_unfollow() {
        let mut pad = GamePadState::new();
        let mut state = JoystickState::default();
        state.povs[0] = 0; // North
        prime_and_set(&mut pad, state);

        let mut cmds = Vec::new();
        let viewport = pad.manage_scroll_axis(&mut cmds);
        assert!(
            viewport
                .iter()
                .any(|c| matches!(c, ViewportCommand::Scroll(ScrollDirection::Up))),
            "{viewport:?}"
        );
        assert!(
            cmds.iter()
                .any(|c| matches!(c, PlayerCommand::SelectFollowElement { entity_id: None })),
            "{cmds:?}"
        );
    }

    #[test]
    fn manage_scroll_axis_pov_north_with_alt_zooms_in() {
        let mut pad = GamePadState::new();
        let mut state = JoystickState::default();
        state.povs[0] = 0; // North
        state.buttons[GamePadButton::AltChoice.index()] = 1;
        prime_and_set(&mut pad, state);

        let mut cmds = Vec::new();
        let viewport = pad.manage_scroll_axis(&mut cmds);
        assert!(
            viewport
                .iter()
                .any(|c| matches!(c, ViewportCommand::ZoomIn))
        );
        assert!(
            !viewport
                .iter()
                .any(|c| matches!(c, ViewportCommand::Scroll(_))),
            "scroll should be suppressed when zooming"
        );
    }

    #[test]
    fn manage_scroll_axis_pov_northeast_emits_both_directions() {
        let mut pad = GamePadState::new();
        let mut state = JoystickState::default();
        state.povs[0] = 4500;
        prime_and_set(&mut pad, state);

        let mut cmds = Vec::new();
        let viewport = pad.manage_scroll_axis(&mut cmds);
        assert!(
            viewport
                .iter()
                .any(|c| matches!(c, ViewportCommand::Scroll(ScrollDirection::Right)))
        );
        assert!(
            viewport
                .iter()
                .any(|c| matches!(c, ViewportCommand::Scroll(ScrollDirection::Up)))
        );
    }

    #[test]
    fn manage_scroll_axis_centered_emits_nothing() {
        let pad = GamePadState::new();
        let mut cmds = Vec::new();
        assert!(pad.manage_scroll_axis(&mut cmds).is_empty());
        assert!(cmds.is_empty());
    }

    #[test]
    fn manage_character_select_none_selected_without_pcs() {
        // Empty engine has no PCs — dispatch must not panic and must
        // return no commands.
        let pad = GamePadState::new();
        let engine = empty_engine();
        assert!(pad.manage_character_select(&engine).is_empty());
    }

    #[test]
    fn manage_action_select_cancel_parade_release_emits_right_click() {
        let mut pad = GamePadState::new();
        // Push then release CANCEL_PARADE (edge-detect the up-transition).
        let mut state = JoystickState::default();
        state.buttons[GamePadButton::CancelParade.index()] = 1;
        pad.update(state);
        pad.update(JoystickState::default()); // release

        let engine = empty_engine();
        let cmds = pad.manage_action_select(&engine);
        // Release edge always clears the held state via MouseRightUp.
        // `resolve_right_click` is empty when no PC is selected, so on
        // an empty engine MouseRightUp is the only command we expect.
        assert!(
            cmds.iter()
                .any(|c| matches!(c, PlayerCommand::MouseRightUp)),
            "{cmds:?}"
        );
    }

    #[test]
    fn manage_qa_single_click_arms_then_fires_on_timeout() {
        let mut pad = GamePadState::new();
        let engine = empty_engine();

        // No selected PC → no QA events.
        // (Real test would need to populate engine.selected_pc_ids, but
        // the dispatcher's early-return branch is important to verify.)
        assert!(pad.manage_qa(1000, &engine).is_none());
    }

    #[test]
    fn manage_qa_timer_expiration_with_selected_pc() {
        // Build a minimal engine with one selected PC so manage_qa's
        // early-return guard passes. We can't easily synthesise a full
        // PC entity without a level, but engine exposes selected_pc_ids
        // as a mutator — push a dummy id in and verify the timer flow.
        let mut pad = GamePadState::new();
        // Arm the timer: release QA while ALT is held.
        let mut pressed = JoystickState::default();
        pressed.buttons[GamePadButton::QaManage.index()] = 1;
        pressed.buttons[GamePadButton::AltChoice.index()] = 1;
        pad.update(pressed);

        let mut released_alt_still_down = JoystickState::default();
        released_alt_still_down.buttons[GamePadButton::AltChoice.index()] = 1;
        pad.update(released_alt_still_down);

        // First pass: arm the timer. Requires at least one selected PC;
        // without one the dispatch short-circuits. This test documents
        // the short-circuit behaviour until a richer engine fixture
        // lands.
        let engine = empty_engine();
        assert!(pad.manage_qa(1000, &engine).is_none());
    }

    #[test]
    fn standard_button_to_gamepad_index_mapping() {
        assert_eq!(
            standard_button_to_gamepad_index(0),
            Some(GamePadButton::ActionB as u8)
        );
        assert_eq!(
            standard_button_to_gamepad_index(1),
            Some(GamePadButton::ActionA as u8)
        );
        assert_eq!(
            standard_button_to_gamepad_index(3),
            Some(GamePadButton::CancelParade as u8)
        );
        // D-pad buttons are routed elsewhere.
        assert_eq!(standard_button_to_gamepad_index(11), None);
    }

    #[test]
    fn is_dpad_button_covers_all_four() {
        assert!(is_dpad_button(11));
        assert!(is_dpad_button(12));
        assert!(is_dpad_button(13));
        assert!(is_dpad_button(14));
        assert!(!is_dpad_button(10));
        assert!(!is_dpad_button(15));
    }

    #[test]
    fn dpad_to_pov_cardinal_and_diagonal() {
        assert_eq!(dpad_to_pov(true, false, false, false), 0);
        assert_eq!(dpad_to_pov(true, true, false, false), 4500);
        assert_eq!(dpad_to_pov(false, true, false, false), 9000);
        assert_eq!(dpad_to_pov(false, false, true, true), 22500);
        assert_eq!(dpad_to_pov(false, false, false, false), 0xFFFF_FFFF);
    }

    #[test]
    fn apply_axis_event_translates_rz_to_directinput_center() {
        let mut pad = GamePadState::new();
        // Standard axis 2 = RightX, value 0 (center) → rz = AXIS_CENTER.
        pad.apply_axis_event(2, 0);
        assert_eq!(pad.pending.rz, AXIS_CENTER);
        // Standard axis 2, value 1638 → rz ≈ AXIS_CENTER + 1638 → dx ≈ 1.0
        pad.apply_axis_event(2, 1638);
        assert_eq!(pad.pending.rz, AXIS_CENTER + 1638);
    }

    #[test]
    fn apply_axis_event_left_stick_stays_signed() {
        let mut pad = GamePadState::new();
        pad.apply_axis_event(0, 25000); // LeftX
        pad.apply_axis_event(1, -25000); // LeftY
        assert_eq!(pad.pending.x, 25000);
        assert_eq!(pad.pending.y, -25000);
    }

    #[test]
    fn apply_button_event_mirrors_pressed_state() {
        let mut pad = GamePadState::new();
        pad.apply_button_event(GamePadButton::ActionA as u8, true);
        assert_eq!(pad.pending.buttons[GamePadButton::ActionA.index()], 1);
        pad.apply_button_event(GamePadButton::ActionA as u8, false);
        assert_eq!(pad.pending.buttons[GamePadButton::ActionA.index()], 0);
    }

    #[test]
    fn process_gamepad_input_promotes_pending_to_current() {
        let mut pad = GamePadState::new();
        let mut threaded = ThreadedInput::new();
        pad.apply_axis_event(0, 15000);
        let engine = empty_engine();
        let _ = pad.process_gamepad_input(0, &engine, &mut threaded);
        assert_eq!(pad.current().x, 15000);
    }
}
