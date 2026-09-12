//! Production mission-description and blazon-purchase models.
//!
//! Other menus own their state in their concrete controllers; this module
//! does not maintain parallel reference implementations of those screens.

use robin_engine::profiles as engine_profiles;
#[cfg(test)]
use robin_engine::sherwood_stat as engine_sherwood_stat;
use serde::{Deserialize, Serialize};

use crate::ingame_menu::resources::{
    MT_INFOBULLE_BUTTON_CANCEL, MT_INFOBULLE_BUTTON_FARMERS_TO_BLAZON,
    MT_INFOBULLE_BUTTON_MISSION_TO_BLAZON, MT_INFOBULLE_BUTTON_MONEY_TO_BLAZON,
    MT_INFOBULLE_BUTTON_PLAY_MISSION,
};
use robin_assets::res_descr::LevelDescriptors;
use robin_assets::resource_manager::ResourceManager;
use robin_engine::campaign::Campaign;
use robin_engine::mission::Mission;
use robin_engine::profiles::MissionType;
use robin_engine::resource_ids;
use robin_engine::sherwood_stat::MenuTextLookup;

// ---------------------------------------------------------------------------
// MissionDescriptionScreen
// ---------------------------------------------------------------------------

/// Widget-tree geometry for the mission description dialog.
///
/// Constants are grouped so a future renderer can lay the widgets out
/// without re-deriving the geometry from comments.
pub mod mission_description_layout {
    /// Window bounds: `(0, 0, 496, 463)`.
    pub const WINDOW_WIDTH: i32 = 496;
    pub const WINDOW_HEIGHT: i32 = 463;

    // ── Picture frame ──
    //
    // The frame is created with a zero-sized box starting at (50, 40); the
    // widget self-sizes to its picture and is then re-anchored so its
    // right edge sits at x = 450.
    pub const PICTURE_FRAME_INITIAL_X: i32 = 50;
    pub const PICTURE_FRAME_Y: i32 = 40;
    pub const PICTURE_FRAME_RIGHT_EDGE: i32 = 450;

    // ── Title ──
    //
    // `(50, 50)..(picture_left - 10, 125)`.
    pub const TITLE_X: i32 = 50;
    pub const TITLE_Y: i32 = 50;
    pub const TITLE_BOTTOM: i32 = 125;
    /// Gap between the title's right edge and the picture frame.
    pub const TITLE_PICTURE_GAP: i32 = 10;

    // ── Description ──
    //
    // Two variants:
    // - Blazon-requiring missions:
    //     `(50, picture_bottom + 5)..(450, 385)`
    // - Non-blazon missions:
    //     `(50, 125)..(450, 385)`
    pub const DESCRIPTION_X: i32 = 50;
    pub const DESCRIPTION_RIGHT: i32 = 450;
    pub const DESCRIPTION_BOTTOM: i32 = 385;
    /// Description top when the mission does *not* require blazons.
    pub const DESCRIPTION_TOP_NO_BLAZONS: i32 = 125;
    /// Gap between the picture's bottom edge and the description box
    /// top when the mission *does* require blazons.
    pub const DESCRIPTION_PICTURE_GAP: i32 = 5;

    // ── Blazon set (blazon-requiring missions only) ──
    //
    // `(50, 125)..(picture_left - 20, 463)`.
    pub const BLAZON_BOX_X: i32 = 50;
    pub const BLAZON_BOX_Y: i32 = 125;
    pub const BLAZON_BOX_BOTTOM: i32 = 463;
    /// Gap between the blazon set's right edge and the picture frame.
    pub const BLAZON_BOX_PICTURE_GAP: i32 = 20;

    // ── Choice buttons ──
    //
    // Convert / start / cancel buttons all sit at y=384 and are centered
    // horizontally across the window with an 8 px gap between neighbours.
    pub const BUTTON_ROW_Y: i32 = 384;
    pub const BUTTON_GAP: i32 = 8;
}

/// The player's choice on the mission description screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MissionChoice {
    /// Start the selected mission.
    StartMission,
    /// Go back to view other pending missions.
    ShowPendingMissions,
    /// No choice made / cancelled.
    #[default]
    None,
}

/// Buttons the mission description dialog can show.
///
/// The three convert buttons only appear in the blazon-requiring layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionDescriptionButton {
    /// Closes the dialog without committing.  Shortcut: Escape.
    Cancel,
    /// Commits the mission.  In the blazon layout this only exists when
    /// the mission is not pseudo; in the non-blazon layout it is the
    /// generic OK button.  Shortcut: Return / Numpad-Enter.
    StartMission,
    /// Opens the buy-blazons child modal.  Blazon layout only.
    ConvertMoney,
    /// Enters the men-to-blazon conversion mode and starts the mission.
    /// Blazon layout only.
    ConvertPeasants,
    /// Swaps the pending mission list into the accessible list.  Blazon
    /// layout only.
    ConvertMission,
}

/// Horizontal placement for a row of buttons.
///
/// Given a list of button widths, returns the left-edge x of each button
/// so the whole row is centered within the window (width `window_w`) with
/// `gap` pixels between neighbours.
pub fn center_horizontally_x(widths: &[i32], window_w: i32, gap: i32) -> Vec<i32> {
    if widths.is_empty() {
        return Vec::new();
    }
    let total: i32 = widths.iter().copied().sum::<i32>() + gap * (widths.len() as i32 - 1).max(0);
    let mut x = (window_w - total) / 2;
    let mut xs = Vec::with_capacity(widths.len());
    for &w in widths {
        xs.push(x);
        x += w + gap;
    }
    xs
}

/// State for the pre-mission description screen.
///
/// Handles mission info display and blazon conversion button logic.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MissionDescriptionScreen {
    /// Index of the mission being described.
    pub mission_index: usize,
    /// Title text for the mission.
    pub title: String,
    /// Description text for the mission.
    pub description: String,
    /// Picture resource ID for the mission.
    pub picture_id: i32,
    /// Whether this mission requires blazons (shows conversion buttons).
    pub requires_blazons: bool,
    /// Whether the "convert peasants" button is enabled.
    pub can_convert_peasants: bool,
    /// Whether the "convert money" button is enabled.
    pub can_convert_money: bool,
    /// Whether the "convert mission" button is enabled.
    pub can_convert_mission: bool,
    /// Whether the "start mission" button should be shown in the
    /// blazon-requiring layout.  Gated on the mission's type being
    /// non-`Pseudo`.
    pub show_start_mission: bool,
    /// Whether men-to-blazon conversion mode was chosen.
    pub men_to_blazon_mode: bool,
    /// The player's choice.
    pub user_choice: MissionChoice,
    /// Whether the screen has been closed.
    pub closed: bool,
}

impl MissionDescriptionScreen {
    /// Resolve the picture resource ID for a mission.
    ///
    /// Returns the `.red` descriptor's mission-description picture ID, or
    /// `RHID_DEFAULT_POPUP_SCROLL_PICTURE` if the level descriptor is
    /// missing.
    pub fn get_mission_picture(level_descriptors: Option<&LevelDescriptors>) -> i32 {
        match level_descriptors {
            Some(d) => d.mission_description.picture_id,
            None => resource_ids::RHID_DEFAULT_POPUP_SCROLL_PICTURE,
        }
    }

    /// Resolve a mission narrative text entry.
    ///
    /// `text_index` 0 is the title and 2 is the description body; 1 is
    /// used by the short mission description tooltip blurb.
    pub fn get_mission_text(
        level_descriptors: Option<&LevelDescriptors>,
        text_resources: &mut ResourceManager,
        text_index: usize,
    ) -> String {
        let Some(desc) = level_descriptors else {
            return "Unable to find the mission resource...".to_string();
        };
        match text_resources.get_string(desc.mission_description.text_table_id, text_index) {
            Ok(s) => s.to_string(),
            Err(e) => {
                tracing::warn!(
                    "MissionDescription text {}.{}: {e}",
                    desc.mission_description.text_table_id,
                    text_index
                );
                "Invalid resource ID...".to_string()
            }
        }
    }

    /// Build the mission description dialog state for a specific
    /// mission.  Resolves title / description / picture resources
    /// internally and latches the blazon-conversion button enable flags
    /// from the campaign.
    ///
    /// Decides which widgets to show and what their initial enable state
    /// is.  The actual widget rendering is done by a future renderer
    /// using [`mission_description_layout`] constants.
    pub fn create(
        mission_index: usize,
        mission: &Mission,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
        level_descriptors: Option<&LevelDescriptors>,
        text_resources: &mut ResourceManager,
    ) -> Self {
        let profile = mission.profile(profiles);
        let requires_blazons = mission.requires_blazons(profiles);
        let is_pseudo = profile.mission_type == MissionType::Pseudo;

        let picture_id = Self::get_mission_picture(level_descriptors);
        let title = Self::get_mission_text(level_descriptors, text_resources, 0);
        let description = Self::get_mission_text(level_descriptors, text_resources, 2);

        // Initial enable flags come straight from the campaign.  When
        // the mission requires blazons AND is a pseudo-mission with zero
        // peasant quotation, `convert_peasants` and `convert_mission`
        // are further forced off.
        let mut can_convert_peasants =
            campaign.can_convert_merry_men_to_blazons(mission_index, profiles);
        let can_convert_money = campaign.can_convert_money_to_blazons(mission_index, profiles);
        let mut can_convert_mission =
            campaign.can_convert_mission_to_blazons(mission_index, profiles);

        if requires_blazons && is_pseudo && profile.peasant_to_blazon_quotation == 0 {
            can_convert_peasants = false;
            can_convert_mission = false;
        }

        // The start-mission button only exists in the blazon branch
        // when the mission is *not* pseudo.  In the non-blazon branch
        // it's always created as the generic OK button.  We store a
        // single flag so renderer code can pick the right button to draw.
        let show_start_mission = !requires_blazons || !is_pseudo;

        Self {
            mission_index,
            title,
            description,
            picture_id,
            requires_blazons,
            can_convert_peasants,
            can_convert_money,
            can_convert_mission,
            show_start_mission,
            men_to_blazon_mode: false,
            user_choice: MissionChoice::None,
            closed: false,
        }
    }

    /// Handle the Start Mission button.
    pub fn on_start_mission(&mut self) {
        self.men_to_blazon_mode = false;
        self.user_choice = MissionChoice::StartMission;
        self.closed = true;
    }

    /// Handle the Cancel button.
    pub fn on_cancel(&mut self) {
        self.user_choice = MissionChoice::None;
        self.closed = true;
    }

    /// Handle the Convert Peasants button.
    pub fn on_convert_peasants(&mut self) {
        self.men_to_blazon_mode = true;
        self.user_choice = MissionChoice::StartMission;
        self.closed = true;
    }

    /// Handle the Convert Money button — caller should open the buy
    /// blazons screen, then call `update_conversion_state` with new values.
    pub fn on_convert_money(&mut self) {
        // The buy blazons screen is shown as a child window.
        // State is updated after it closes via update_conversion_state.
    }

    /// Handle the Convert Mission button.
    pub fn on_convert_mission(&mut self) {
        self.user_choice = MissionChoice::ShowPendingMissions;
        self.closed = true;
    }

    /// Update conversion button availability (called after buy-blazons closes).
    pub fn update_conversion_state(
        &mut self,
        can_peasants: bool,
        can_money: bool,
        can_mission: bool,
    ) {
        self.can_convert_peasants = can_peasants;
        self.can_convert_money = can_money;
        self.can_convert_mission = can_mission;
    }

    /// List of buttons the dialog should show, in dialog-creation order.
    /// Drives both the centered button-row layout and the focus-manager
    /// groupable order.
    pub fn buttons(&self) -> Vec<MissionDescriptionButton> {
        let mut buttons = Vec::new();
        if self.requires_blazons {
            // The three convert buttons go first in this order; then
            // start-mission is appended when the mission is not pseudo.
            buttons.push(MissionDescriptionButton::ConvertPeasants);
            buttons.push(MissionDescriptionButton::ConvertMoney);
            buttons.push(MissionDescriptionButton::ConvertMission);
            if self.show_start_mission {
                buttons.push(MissionDescriptionButton::StartMission);
            }
        } else {
            // The generic OK / start-mission button.
            buttons.push(MissionDescriptionButton::StartMission);
        }
        // Cancel is always appended last.
        buttons.push(MissionDescriptionButton::Cancel);
        buttons
    }

    /// Whether a given button is interactive for the current state.
    pub fn is_enabled(&self, button: MissionDescriptionButton) -> bool {
        match button {
            MissionDescriptionButton::Cancel | MissionDescriptionButton::StartMission => true,
            MissionDescriptionButton::ConvertPeasants => self.can_convert_peasants,
            MissionDescriptionButton::ConvertMoney => self.can_convert_money,
            MissionDescriptionButton::ConvertMission => self.can_convert_mission,
        }
    }

    /// Tooltip string for a button.
    pub fn tooltip(button: MissionDescriptionButton, menu_text: &dyn MenuTextLookup) -> String {
        let id = match button {
            MissionDescriptionButton::Cancel => MT_INFOBULLE_BUTTON_CANCEL,
            MissionDescriptionButton::StartMission => MT_INFOBULLE_BUTTON_PLAY_MISSION,
            MissionDescriptionButton::ConvertMoney => MT_INFOBULLE_BUTTON_MONEY_TO_BLAZON,
            MissionDescriptionButton::ConvertPeasants => MT_INFOBULLE_BUTTON_FARMERS_TO_BLAZON,
            MissionDescriptionButton::ConvertMission => MT_INFOBULLE_BUTTON_MISSION_TO_BLAZON,
        };
        menu_text.get(id)
    }

    /// Dispatch a button activation.  Disabled buttons are no-ops.
    pub fn activate(&mut self, button: MissionDescriptionButton) {
        if !self.is_enabled(button) {
            return;
        }
        match button {
            MissionDescriptionButton::Cancel => self.on_cancel(),
            MissionDescriptionButton::StartMission => self.on_start_mission(),
            MissionDescriptionButton::ConvertPeasants => self.on_convert_peasants(),
            MissionDescriptionButton::ConvertMission => self.on_convert_mission(),
            MissionDescriptionButton::ConvertMoney => self.on_convert_money(),
        }
    }

    /// Dropped-initial carveout dimensions for the description text box.
    ///
    /// Returns `(width, height)` of the picture-shaped hole to reserve
    /// in the top-right of the description text box so the narrative
    /// wraps around the picture.  Only applies to the non-blazon layout;
    /// in the blazon layout the description sits *below* the picture so
    /// no carveout is used.
    pub fn description_drop_cap(
        &self,
        picture_width: i32,
        picture_height: i32,
    ) -> Option<(i32, i32)> {
        if self.requires_blazons {
            return None;
        }
        let w = picture_width + 10;
        let h = picture_height + mission_description_layout::PICTURE_FRAME_Y
            - mission_description_layout::DESCRIPTION_TOP_NO_BLAZONS
            + 5;
        Some((w, h))
    }
}

/// Maximum player name length, including the input widget's terminator slot.
pub const MAX_PLAYER_NAME_LENGTH: usize = 30;

// ---------------------------------------------------------------------------
// BuyBlazonsScreen
// ---------------------------------------------------------------------------

/// State for the blazon purchase screen.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuyBlazonsScreen {
    /// Mission index this purchase is for.
    pub mission_index: usize,
    /// Cost of the blazon set.
    pub cost: u32,
    /// Available ransom funds.
    pub available_funds: u32,
    /// Status/price display message.
    pub message: String,
    /// Whether a purchase was made.
    pub purchased: bool,
    /// Whether the screen has been closed.
    pub closed: bool,
}

impl BuyBlazonsScreen {
    pub fn new(mission_index: usize, cost: u32, available_funds: u32) -> Self {
        let can_afford = available_funds >= cost;
        let message = if can_afford {
            format!("Cost: {}", cost)
        } else {
            format!("Not enough funds (need {}, have {})", cost, available_funds)
        };
        Self {
            mission_index,
            cost,
            available_funds,
            message,
            ..Default::default()
        }
    }

    /// Whether the Buy button should be enabled.
    pub fn can_buy(&self) -> bool {
        self.available_funds >= self.cost
    }

    /// Handle Buy button.
    pub fn on_buy(&mut self) {
        if self.can_buy() {
            self.available_funds -= self.cost;
            self.purchased = true;
            self.closed = true;
        }
    }

    /// Handle Quit button.
    pub fn on_quit(&mut self) {
        self.closed = true;
    }
}

#[cfg(test)]
mod tests;
