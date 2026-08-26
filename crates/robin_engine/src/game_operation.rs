//! Tracks the current game operation/mode (level in progress, failed, quit, etc.).

use std::ffi::c_void;

/// All possible game operation codes. The discriminant values match
/// the original on-disk enum exactly.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(i32)]
pub enum GameCode {
    #[default]
    LevelInProgress = 0,
    LevelFailed = 1,
    LevelSucceeded = 2,
    LevelInterrupted = 3,
    LevelNext = 4,
    LevelRestart = 5,
    Quit = 6,
    LevelLoad = 7,
    LevelSave = 8,
}

/// FFI-compatible struct: `{ GameCode code; SaveGame* save_game; }`.
///
/// The `save_game` pointer is opaque (`void*`) on the Rust side.
#[repr(C)]
pub struct GameOperationFfi {
    pub code: GameCode,
    pub save_game: *mut c_void,
}

/// Tracks the current and previous game operation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GameOperationState {
    current: GameCode,
    previous: GameCode,
}

impl GameOperationState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the current operation, saving the old one as previous.
    pub fn set(&mut self, op: GameCode) {
        self.previous = self.current;
        self.current = op;
    }

    pub fn get_current(&self) -> GameCode {
        self.current
    }

    pub fn get_previous(&self) -> GameCode {
        self.previous
    }

    /// Check if the current operation matches `op`.
    pub fn is(&self, op: GameCode) -> bool {
        self.current == op
    }
}

// ===================== C ABI =====================

/// Return a default-initialised game operation (code = LevelInProgress,
/// save_game = null).
pub fn game_operation_ffi_default() -> GameOperationFfi {
    GameOperationFfi {
        code: GameCode::LevelInProgress,
        save_game: std::ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_level_in_progress() {
        let state = GameOperationState::new();
        assert_eq!(state.get_current(), GameCode::LevelInProgress);
        assert_eq!(state.get_previous(), GameCode::LevelInProgress);
    }

    #[test]
    fn set_updates_current_and_previous() {
        let mut state = GameOperationState::new();
        state.set(GameCode::LevelFailed);
        assert_eq!(state.get_current(), GameCode::LevelFailed);
        assert_eq!(state.get_previous(), GameCode::LevelInProgress);

        state.set(GameCode::LevelRestart);
        assert_eq!(state.get_current(), GameCode::LevelRestart);
        assert_eq!(state.get_previous(), GameCode::LevelFailed);
    }

    #[test]
    fn is_checks_current() {
        let mut state = GameOperationState::new();
        assert!(state.is(GameCode::LevelInProgress));
        assert!(!state.is(GameCode::Quit));

        state.set(GameCode::Quit);
        assert!(state.is(GameCode::Quit));
        assert!(!state.is(GameCode::LevelInProgress));
    }

    #[test]
    fn all_variants_roundtrip_serde() {
        let variants = [
            GameCode::LevelInProgress,
            GameCode::LevelFailed,
            GameCode::LevelSucceeded,
            GameCode::LevelInterrupted,
            GameCode::LevelNext,
            GameCode::LevelRestart,
            GameCode::Quit,
            GameCode::LevelLoad,
            GameCode::LevelSave,
        ];
        for op in &variants {
            let json = serde_json::to_string(op).unwrap();
            let back: GameCode = serde_json::from_str(&json).unwrap();
            assert_eq!(*op, back);
        }
    }

    #[test]
    fn discriminant_values_match_original() {
        assert_eq!(GameCode::LevelInProgress as i32, 0);
        assert_eq!(GameCode::LevelFailed as i32, 1);
        assert_eq!(GameCode::LevelSucceeded as i32, 2);
        assert_eq!(GameCode::LevelInterrupted as i32, 3);
        assert_eq!(GameCode::LevelNext as i32, 4);
        assert_eq!(GameCode::LevelRestart as i32, 5);
        assert_eq!(GameCode::Quit as i32, 6);
        assert_eq!(GameCode::LevelLoad as i32, 7);
        assert_eq!(GameCode::LevelSave as i32, 8);
    }

    #[test]
    fn ffi_default() {
        let op = game_operation_ffi_default();
        assert_eq!(op.code, GameCode::LevelInProgress);
        assert!(op.save_game.is_null());
    }
}
