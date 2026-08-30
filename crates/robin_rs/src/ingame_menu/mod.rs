//! In-game menus: pause, dialogue, debriefing, mission state, options,
//! graphics, sounds, shortcuts.
//!
//! Used during gameplay (as opposed to the main menu, which lives in
//! [`crate::main_menu`]).  Contains:
//!
//! - [`resources`] — per-mission resource cache: DEFAULT.RES button sprites,
//!   window backgrounds, parchments, native bitmap fonts and the menu text
//!   string table.
//! - [`layout`] — virtual 640x480 coordinate helpers, button/widget layout,
//!   text rendering and wrapping / pagination helpers.
//! - [`pause`] — the six-button in-game pause menu.
//! - [`briefings`] — renders the primary/secondary objective list in the
//!   left half of the pause window.
//! - [`yesno`] — the 400x200 modal confirmation.
//! - [`mission_state`] — the mission start/won/lost transient popup with
//!   zoom transition and Yes/No confirm.
//! - [`debriefing`] — the post-mission parchment with justified text,
//!   pagination and Restart / Load / OK buttons.
//! - [`dialogue`] — character dialogues with per-sentence audio, mouth
//!   animation and auto-advance timer.
//! - [`options`] — the in-mission options hub.
//! - [`graphics`] — resolution + visual toggles.
//! - [`sounds`] — stereo/EAX radios and volume sliders.
//! - [`shortcuts`] — keyboard binding editor with click-to-rebind mode.

pub mod blazon_set;
pub mod briefings;
pub mod buy_blazons;
pub mod debriefing;
pub mod dialogue;
pub mod gameplay;
pub mod graphics;
pub mod language;
pub mod layout;
pub mod mission_description;
pub mod mission_state;
pub mod modal_net;
pub mod options;
pub mod pause;
pub mod popup_scroll;
pub mod resources;
pub mod save_load;
pub mod shortcuts;
pub mod sounds;
pub mod trading;
pub mod widget_bridge;
pub mod yesno;

pub use buy_blazons::{BuyBlazonsOutcome, show_buy_blazons};
pub use debriefing::{DebriefingModalState, DebriefingOutcome, show_debriefing};
pub(crate) use dialogue::show_dialogue_batch;
pub use dialogue::{
    BatchDialogue, DIALOGUE_PORTRAIT_IDS, DialogueModalState, DialogueSentence, show_dialogue,
};
pub use layout::{MENU_H, MENU_W, MenuButton, MenuTransform};
pub use mission_state::{MissionStatePopupState, show_mission_state_popup};
pub use modal_net::ModalNet;
pub use options::{OptionsOutcome, show_options};
pub use pause::{PauseMenu, PauseMenuOutcome};
pub(crate) use popup_scroll::show_popup_scroll;
pub use popup_scroll::{PopupScrollItem, PopupScrollModalState};
pub use resources::{IngameMenuResources, MenuSurface};
pub use save_load::{LoadPickerModalState, SaveLoadMode, SaveLoadOutcome, show_save_load};
pub use trading::{TradingModalState, TradingOutcome};
pub use yesno::{YesNoModalState, show_file_not_found, show_yesno};

/// Terminal result from a one-frame menu state.
///
/// `ExitRequested` is deliberately distinct from cancellation: closing the
/// native window must propagate out of nested pause-side screens instead of
/// quietly returning to gameplay.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModalScreenOutcome<T> {
    Accepted(T),
    Cancelled,
    ExitRequested,
}

use robin_engine::game_operation::GameCode;

/// Translate a `GameCode` reported by the mission into a human-readable
/// title / body pair.  Placeholder text used until the real level
/// debriefing table lookup is wired up.
///
/// Note: for proper localisation, the caller should look the title up via
/// [`resources::MenuText`] using [`resources::MT_TTL_MISSION_WON`] /
/// [`resources::MT_TTL_MISSION_LOST`] / [`resources::MT_TTL_MISSION_ABORTED`].
pub fn mission_state_text(code: GameCode) -> Option<(&'static str, &'static str)> {
    match code {
        GameCode::LevelSucceeded => Some((
            "Mission Won",
            "The Sheriff's men have been defeated and the mission is complete.",
        )),
        GameCode::LevelFailed => Some((
            "Mission Lost",
            "Robin's band has been defeated.  You may reload a save or try again.",
        )),
        GameCode::LevelInterrupted => Some((
            "Mission Abandoned",
            "The mission was interrupted before it could be completed.",
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mission_state_text_handles_all_end_codes() {
        assert!(mission_state_text(GameCode::LevelSucceeded).is_some());
        assert!(mission_state_text(GameCode::LevelFailed).is_some());
        assert!(mission_state_text(GameCode::LevelInterrupted).is_some());
        assert!(mission_state_text(GameCode::LevelInProgress).is_none());
    }
}
