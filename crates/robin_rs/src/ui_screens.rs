//! UI screen state machines for the menu system.
//!
//! These structs capture the logical state-machine behaviour (visibility,
//! timeouts, progress tracking, screen transitions, dialog choices) without
//! the rendering details, which are handled separately.
//!
//! Screens covered:
//! - transient overlay popup
//! - scrollable text popup with pagination
//! - character dialogue with portrait animation
//! - post-mission debriefing
//! - pre-mission description with blazon support
//! - hover tooltip for missions
//! - main menu screen
//! - in-game pause menu
//! - options hub screen
//! - graphics settings
//! - sound settings
//! - load/save screen
//! - new player creation
//! - player selection
//! - blazon purchase
//! - keyboard shortcuts configuration
//! - movie viewer
//! - mission started/quit popup with transition

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
use robin_engine::game_operation::GameCode;
use robin_engine::graphic_config::GraphicConfig;
use robin_engine::mission::Mission;
use robin_engine::profiles::MissionType;
use robin_engine::resource_ids;
use robin_engine::sherwood_stat::MenuTextLookup;
use robin_engine::sound_config::SoundConfig;

// ---------------------------------------------------------------------------
// InfoPopup
// ---------------------------------------------------------------------------

/// Transient information popup shown over the game view.
///
/// Models the visibility and timeout logic for sword/bow experience
/// indicators; rendering is handled elsewhere.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InfoPopup {
    /// Resource / string ID for the text to display.
    pub text_id: u32,
    /// Whether the popup is currently visible.
    pub visible: bool,
    /// Total number of frames the popup should remain visible.
    pub timeout_frames: u32,
    /// How many frames have elapsed since the popup was shown.
    pub current_frame: u32,
}

impl InfoPopup {
    /// Show the popup with the given text id and timeout (in frames).
    pub fn show(&mut self, text_id: u32, timeout_frames: u32) {
        self.text_id = text_id;
        self.timeout_frames = timeout_frames;
        self.current_frame = 0;
        self.visible = true;
    }

    /// Advance one frame. Returns `true` while the popup is still active,
    /// `false` once the timeout has expired (at which point it auto-hides).
    pub fn tick(&mut self) -> bool {
        if !self.visible {
            return false;
        }
        self.current_frame += 1;
        if self.current_frame >= self.timeout_frames {
            self.visible = false;
            return false;
        }
        true
    }

    /// Immediately hide the popup.
    pub fn hide(&mut self) {
        self.visible = false;
    }
}

// ---------------------------------------------------------------------------
// PopupScroll
// ---------------------------------------------------------------------------

/// State for a scrollable text popup with optional illustration.
///
/// Supports text pagination: when the rendered text overflows,
/// `text_remaining` holds what didn't fit, and the caller should display
/// another popup with that text.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PopupScroll {
    /// Full text to display (may be truncated by rendering).
    pub text: String,
    /// Text that didn't fit and needs a follow-up page.
    pub text_remaining: String,
    /// Optional picture resource ID (0 = no picture).
    pub picture_id: u32,
    /// Text alignment mode (0 = justified, 1 = centered, etc.).
    pub text_alignment: u32,
    /// Whether the dialog has been closed (OK pressed).
    pub closed: bool,
}

impl PopupScroll {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }

    pub fn with_picture(mut self, picture_id: u32) -> Self {
        self.picture_id = picture_id;
        self
    }

    pub fn with_alignment(mut self, alignment: u32) -> Self {
        self.text_alignment = alignment;
        self
    }

    /// Handle OK button press.  The renderer should have set
    /// `text_remaining` before this is called.
    pub fn on_ok(&mut self) {
        self.closed = true;
    }

    /// Whether more pages remain after this one.
    pub fn has_more_pages(&self) -> bool {
        !self.text_remaining.is_empty()
    }

    /// Advance to the next page, consuming remaining text.
    /// Returns `true` if there was a next page.
    pub fn advance_page(&mut self) -> bool {
        if self.text_remaining.is_empty() {
            return false;
        }
        self.text = std::mem::take(&mut self.text_remaining);
        self.closed = false;
        true
    }
}

// ---------------------------------------------------------------------------
// DialogueScreen
// ---------------------------------------------------------------------------

/// Number of valid character portrait indices.
pub const VALID_PORTRAIT_COUNT: usize = 15;
/// Total portrait slots (valid + 1 fallback for invalid IDs).
pub const TOTAL_PORTRAIT_COUNT: usize = 16;
/// Maximum times the same mouth frame is shown before a random blink.
const MAX_FACE_COUNT: u32 = 3;

/// A single sentence in a dialogue sequence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DialogueSentence {
    /// Text to display.
    pub text: String,
    /// Sound resource identifier (empty = no voice).
    pub sound_id: String,
    /// Character portrait index (0..VALID_PORTRAIT_COUNT-1).
    pub portrait_index: u8,
}

/// State for a character dialogue screen with portrait animation.
///
/// Manages sentence progression and portrait mouth-sync animation.  Sound
/// playback is delegated to the sound manager; rendering is handled
/// elsewhere.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogueScreen {
    /// The dialogue's resource ID.
    pub dialogue_id: u32,
    /// All sentences in this dialogue.
    pub sentences: Vec<DialogueSentence>,
    /// Index of the current sentence (-1 = not started).
    pub current_sentence: i32,
    /// Current portrait mouth frame (0..4).
    pub mouth_frame: u8,
    /// Counter for how many consecutive frames the same mouth state was shown.
    pub same_face_count: u32,
    /// Timer ID for sentence auto-advance (set by the menu screen timer system).
    pub update_timer_id: Option<u32>,
    /// Whether the dialogue has been completed or abandoned.
    pub finished: bool,
    /// Whether the dialogue was abandoned (Stop button) vs completed normally.
    pub abandoned: bool,
}

impl Default for DialogueScreen {
    fn default() -> Self {
        Self {
            dialogue_id: 0,
            sentences: Vec::new(),
            current_sentence: -1,
            mouth_frame: 0,
            same_face_count: 0,
            update_timer_id: None,
            finished: false,
            abandoned: false,
        }
    }
}

impl DialogueScreen {
    pub fn new(dialogue_id: u32, sentences: Vec<DialogueSentence>) -> Self {
        Self {
            dialogue_id,
            sentences,
            ..Default::default()
        }
    }

    /// Advance to the next sentence.  Returns the sentence if there is one,
    /// or `None` if the dialogue is complete.
    pub fn next_sentence(&mut self) -> Option<&DialogueSentence> {
        self.current_sentence += 1;
        let idx = self.current_sentence as usize;
        if idx < self.sentences.len() {
            self.mouth_frame = 0;
            self.same_face_count = 0;
            Some(&self.sentences[idx])
        } else {
            self.finished = true;
            None
        }
    }

    /// Update the portrait mouth animation based on sound volume.
    pub fn update_portrait(&mut self, sound_volume: f32) {
        let new_frame = if sound_volume < 0.01 {
            0
        } else if sound_volume < 0.02 {
            1
        } else if sound_volume < 0.15 {
            2
        } else if sound_volume < 0.30 {
            3
        } else {
            4
        };

        if new_frame == self.mouth_frame {
            self.same_face_count += 1;
            if self.same_face_count >= MAX_FACE_COUNT {
                // Random blink — alternate between 0 and 1
                self.mouth_frame = if self.mouth_frame == 0 { 1 } else { 0 };
                self.same_face_count = 0;
            }
        } else {
            self.mouth_frame = new_frame;
            self.same_face_count = 0;
        }
    }

    /// Handle the Stop/Abandon button — end the dialogue early.
    pub fn on_stop(&mut self) {
        self.abandoned = true;
        self.finished = true;
    }
}

// ---------------------------------------------------------------------------
// DebriefingScreen
// ---------------------------------------------------------------------------

/// Result of the debriefing screen — what the player wants to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DebriefingAction {
    /// Continue playing (default — no special action).
    #[default]
    Continue,
    /// Load a different save game.
    Load,
    /// Restart from the automatic checkpoint save.
    Restart,
}

/// State for the post-mission debriefing screen.
///
/// Supports text pagination, win/loss display, and restart/load actions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DebriefingScreen {
    /// Whether the mission was won.
    pub win: bool,
    /// Whether the restart option is available.
    pub restart_allowed: bool,
    /// Title text ("Mission Won" / "Mission Lost").
    pub title: String,
    /// Current page's body text.
    pub text: String,
    /// Text that didn't fit on the current page.
    pub text_remaining: String,
    /// What the player chose to do.
    pub action: DebriefingAction,
    /// Game operation code to return to the caller.
    pub game_code: GameCode,
    /// Whether a load was requested.
    pub load_requested: bool,
    /// Whether the dialog has been closed.
    pub closed: bool,
}

impl DebriefingScreen {
    pub fn new(win: bool, restart_allowed: bool, title: String, text: String) -> Self {
        Self {
            win,
            restart_allowed,
            title,
            text,
            game_code: GameCode::LevelInProgress,
            ..Default::default()
        }
    }

    /// Handle the OK button — check for text overflow, then close.
    ///
    /// The renderer should set `text_remaining` before calling this.
    pub fn on_ok(&mut self) {
        self.closed = true;
    }

    /// Whether more text pages remain.
    pub fn has_more_pages(&self) -> bool {
        !self.text_remaining.is_empty()
    }

    /// Advance to the next text page. Returns true if there was one.
    pub fn advance_page(&mut self) -> bool {
        if self.text_remaining.is_empty() {
            return false;
        }
        self.text = std::mem::take(&mut self.text_remaining);
        self.closed = false;
        true
    }

    /// Handle the Restart button.
    pub fn on_restart(&mut self) {
        self.action = DebriefingAction::Restart;
        self.game_code = GameCode::LevelLoad;
        self.load_requested = true;
        self.closed = true;
    }

    /// Handle the Load button — caller should present load screen, then
    /// call `set_load_result` with the outcome.
    pub fn on_load(&mut self) {
        self.action = DebriefingAction::Load;
    }

    /// Set the result of the load screen interaction.
    pub fn set_load_result(&mut self, loaded: bool) {
        if loaded {
            self.game_code = GameCode::LevelLoad;
            self.load_requested = true;
            self.closed = true;
        }
        // If not loaded, the debriefing remains open.
    }
}

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

// ---------------------------------------------------------------------------
// ShortMissionDescription
// ---------------------------------------------------------------------------

/// State for the compact mission info tooltip that follows the mouse.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShortMissionDescription {
    /// Index of the mission being described, or None.
    pub mission_index: Option<usize>,
    /// Description text.
    pub description: String,
    /// Remaining lifetime in turns (for the expiration indicator).
    pub remaining_lifetime: Option<u32>,
    /// Whether blazons are shown for this mission.
    pub show_blazons: bool,
    /// Window position (follows mouse with offset).
    pub position_x: f32,
    pub position_y: f32,
    /// Whether the tooltip is currently visible.
    pub visible: bool,
}

/// Offset from mouse cursor to tooltip window.
const TOOLTIP_OFFSET_X: f32 = 25.0;
const TOOLTIP_OFFSET_Y: f32 = 25.0;

impl ShortMissionDescription {
    /// Update the mission being described.
    pub fn set_mission(
        &mut self,
        index: usize,
        description: String,
        lifetime: Option<u32>,
        show_blazons: bool,
    ) {
        let changed = self.mission_index != Some(index);
        self.mission_index = Some(index);
        if changed {
            self.description = description;
            self.remaining_lifetime = lifetime;
            self.show_blazons = show_blazons;
        }
        self.visible = true;
    }

    /// Clear the tooltip (mouse left the location).
    pub fn clear(&mut self) {
        self.mission_index = None;
        self.visible = false;
    }

    /// Track mouse position with clamping to screen bounds.
    pub fn track_mouse(
        &mut self,
        mouse_x: f32,
        mouse_y: f32,
        screen_width: f32,
        screen_height: f32,
        tooltip_width: f32,
        tooltip_height: f32,
    ) {
        let mut x = mouse_x + TOOLTIP_OFFSET_X;
        let mut y = mouse_y + TOOLTIP_OFFSET_Y;

        // Clamp to screen bounds
        if x + tooltip_width > screen_width {
            x = screen_width - tooltip_width;
        }
        if y + tooltip_height > screen_height {
            y = screen_height - tooltip_height;
        }
        if x < 0.0 {
            x = 0.0;
        }
        if y < 0.0 {
            y = 0.0;
        }

        self.position_x = x;
        self.position_y = y;
    }

    /// Lifetime indicator index (0–4) for the expiration icon.
    pub fn lifetime_indicator(&self) -> u8 {
        match self.remaining_lifetime {
            Some(ttl) if ttl <= 3 => ttl as u8,
            Some(_) => 4,
            None => 4,
        }
    }
}

// ---------------------------------------------------------------------------
// IntroScreen
// ---------------------------------------------------------------------------

/// Operation result from the intro/main menu screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum IntroOperation {
    #[default]
    Unknown,
    /// Exit the game.
    Exit,
    /// Start a new game / continue campaign.
    Start,
    /// Load a saved game.
    Load,
    /// Re-display the menu (e.g. after resolution change).
    Redisplay,
}

/// State for the main menu (intro) screen.
///
/// Buttons: Start, Load, Select Player, Movies, Credits, Options, Exit.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntroScreen {
    /// The operation the player chose.
    pub operation: IntroOperation,
    /// Game code from a loaded save, if any.
    pub game_code: GameCode,
    /// Current player profile name.
    pub profile_name: String,
    /// Current player profile info text.
    pub profile_info: String,
    /// Whether the screen has been closed.
    pub closed: bool,
}

impl IntroScreen {
    pub fn new(profile_name: String, profile_info: String) -> Self {
        Self {
            profile_name,
            profile_info,
            ..Default::default()
        }
    }

    /// Handle Start Game button.
    pub fn on_start_game(&mut self) {
        self.operation = IntroOperation::Start;
        self.closed = true;
    }

    /// Handle Load button — caller should present the load/save screen
    /// and call `set_load_result` with the outcome.
    pub fn on_load(&mut self) {
        // Caller opens LoadSaveScreen in load mode
    }

    /// Set the result after the load screen closes.
    pub fn set_load_result(&mut self, loaded: bool, game_code: GameCode) {
        if loaded && game_code == GameCode::LevelLoad {
            self.operation = IntroOperation::Load;
            self.game_code = game_code;
            self.closed = true;
        }
    }

    /// Handle Options button — if resolution changed, redisplay.
    pub fn set_options_result(&mut self, resolution_changed: bool) {
        if resolution_changed {
            self.operation = IntroOperation::Redisplay;
            self.closed = true;
        }
    }

    /// Handle Exit button — presents confirmation dialog first.
    pub fn on_exit(&mut self, confirmed: bool) {
        if confirmed {
            self.operation = IntroOperation::Exit;
            self.closed = true;
        }
    }
}

// ---------------------------------------------------------------------------
// IngameScreen
// ---------------------------------------------------------------------------

/// State for the in-game (pause) menu.
///
/// Buttons: Continue, Load, Save, Options, Restart, Quit.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngameScreen {
    /// Game operation code indicating what to do next.
    pub game_code: GameCode,
    /// Whether game options were changed (requiring refresh).
    pub options_changed: bool,
    /// Whether the screen needs to be re-displayed (resolution change).
    pub redisplay: bool,
    /// Whether the screen has been closed.
    pub closed: bool,
}

impl IngameScreen {
    /// Handle Continue button (or Escape shortcut).
    pub fn on_continue(&mut self) {
        self.game_code = GameCode::LevelInProgress;
        self.closed = true;
    }

    /// Set the result after the load screen closes.
    pub fn set_load_result(&mut self, loaded: bool) {
        if loaded {
            self.game_code = GameCode::LevelLoad;
            self.closed = true;
        }
    }

    /// Set the result after the options screen closes.
    pub fn set_options_result(&mut self, changed: bool, resolution_changed: bool) {
        self.options_changed = changed;
        if resolution_changed {
            self.redisplay = true;
        }
    }

    /// Handle Restart button (after confirmation).
    pub fn on_restart(&mut self, confirmed: bool) {
        if confirmed {
            self.game_code = GameCode::LevelRestart;
            self.closed = true;
        }
    }

    /// Handle Quit Game button (after confirmation).
    pub fn on_quit(&mut self, confirmed: bool) {
        if confirmed {
            self.game_code = GameCode::Quit;
            self.closed = true;
        }
    }
}

// ---------------------------------------------------------------------------
// OptionsScreen
// ---------------------------------------------------------------------------

/// State for the options hub screen.
///
/// Delegates to Graphics, Sounds, and Shortcuts sub-screens.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OptionsScreen {
    /// Whether any options were changed across sub-screens.
    pub options_changed: bool,
    /// Whether the screen needs re-display (resolution changed).
    pub redisplay: bool,
    /// Whether the screen has been closed.
    pub closed: bool,
}

impl OptionsScreen {
    /// Set the result after the graphics sub-screen closes.
    pub fn set_graphics_result(&mut self, changed: bool, resolution_changed: bool) {
        if changed {
            self.options_changed = true;
        }
        if resolution_changed {
            self.redisplay = true;
        }
    }
}

// ---------------------------------------------------------------------------
// GraphicsScreen
// ---------------------------------------------------------------------------

/// Resolution presets available in the graphics settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum ResolutionPreset {
    Low = 0,
    #[default]
    Medium = 1,
    High = 2,
}

impl ResolutionPreset {
    /// Get the pixel dimensions for this preset.
    pub fn dimensions(self) -> (f32, f32) {
        match self {
            Self::Low => (640.0, 480.0),
            Self::Medium => (800.0, 600.0),
            Self::High => (1024.0, 768.0),
        }
    }

    /// Determine preset from dimensions, defaulting to Medium.
    pub fn from_dimensions(x: f32, y: f32) -> Self {
        if (x - 640.0).abs() < 1.0 && (y - 480.0).abs() < 1.0 {
            Self::Low
        } else if (x - 1024.0).abs() < 1.0 && (y - 768.0).abs() < 1.0 {
            Self::High
        } else {
            Self::Medium
        }
    }
}

/// Graphics option toggles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum GraphicsOption {
    AlphaVisionField = 0,
    TransparentShadows = 1,
    EffectAnimations = 2,
    BackgroundAnimations = 3,
    FogTintAllSprites = 4,
    AdaptiveWidescreen = 5,
    NativeRefreshPresentation = 6,
}

/// Number of graphics option toggles.
pub const GRAPHICS_OPTION_COUNT: usize = 7;

/// State for the graphics settings screen.
///
/// Edits a working copy of `GraphicConfig` and applies on OK.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphicsScreen {
    /// Working copy of graphics settings being edited.
    pub config: GraphicConfig,
    /// The original config (for cancel/revert).
    pub original_config: GraphicConfig,
    /// Currently selected resolution preset.
    pub resolution: ResolutionPreset,
    /// Toggle states for the graphics option buttons.
    pub option_toggles: [bool; GRAPHICS_OPTION_COUNT],
    /// Whether any setting was changed.
    pub changed: bool,
    /// Whether the screen has been closed.
    pub closed: bool,
    /// Whether OK was pressed (vs Cancel).
    pub accepted: bool,
}

impl GraphicsScreen {
    pub fn new(config: GraphicConfig) -> Self {
        let resolution =
            ResolutionPreset::from_dimensions(config.resolution_x, config.resolution_y);
        let toggles = [
            config.framed_view_cone,
            config.display_shadow,
            config.display_anim,
            config.display_titbits,
            config.apply_fog_to_all_sprites,
            config.adaptive_widescreen,
            config.native_refresh_presentation,
        ];
        Self {
            config: config.clone(),
            original_config: config,
            resolution,
            option_toggles: toggles,
            changed: false,
            closed: false,
            accepted: false,
        }
    }

    /// Handle a resolution radio button selection.
    pub fn on_resolution(&mut self, preset: ResolutionPreset) {
        if self.resolution != preset {
            self.resolution = preset;
            let (x, y) = preset.dimensions();
            self.config.set_resolution(x, y);
            self.changed = true;
        }
    }

    /// Handle a graphics option toggle.
    pub fn on_toggle(&mut self, option: GraphicsOption) {
        let idx = option as usize;
        self.option_toggles[idx] = !self.option_toggles[idx];
        self.changed = true;

        match option {
            GraphicsOption::AlphaVisionField => {
                self.config.framed_view_cone = self.option_toggles[idx];
            }
            GraphicsOption::TransparentShadows => {
                self.config.display_shadow = self.option_toggles[idx];
            }
            GraphicsOption::EffectAnimations => {
                self.config.display_anim = self.option_toggles[idx];
            }
            GraphicsOption::BackgroundAnimations => {
                self.config.display_titbits = self.option_toggles[idx];
            }
            GraphicsOption::FogTintAllSprites => {
                self.config.apply_fog_to_all_sprites = self.option_toggles[idx];
            }
            GraphicsOption::AdaptiveWidescreen => {
                self.config.adaptive_widescreen = self.option_toggles[idx];
            }
            GraphicsOption::NativeRefreshPresentation => {
                self.config.native_refresh_presentation = self.option_toggles[idx];
            }
        }
    }

    /// Handle OK — accept changes.
    pub fn on_ok(&mut self) {
        self.accepted = true;
        self.closed = true;
    }

    /// Handle Cancel — revert to original.
    pub fn on_cancel(&mut self) {
        self.config = self.original_config.clone();
        self.accepted = false;
        self.closed = true;
    }

    /// Whether the resolution changed from the original.
    pub fn resolution_changed(&self) -> bool {
        (self.config.resolution_x - self.original_config.resolution_x).abs() > 0.1
            || (self.config.resolution_y - self.original_config.resolution_y).abs() > 0.1
            || self.config.adaptive_widescreen != self.original_config.adaptive_widescreen
    }
}

// ---------------------------------------------------------------------------
// SoundsScreen
// ---------------------------------------------------------------------------

/// State for the sound settings screen.
///
/// Edits a working copy of `SoundConfig` and applies on OK.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoundsScreen {
    /// Working copy of sound settings being edited.
    pub config: SoundConfig,
    /// The original config (for cancel/revert).
    pub original_config: SoundConfig,
    /// Whether any setting was changed.
    pub changed: bool,
    /// Whether the screen has been closed.
    pub closed: bool,
    /// Whether OK was pressed (vs Cancel).
    pub accepted: bool,
}

impl SoundsScreen {
    pub fn new(config: SoundConfig) -> Self {
        Self {
            config,
            original_config: config,
            changed: false,
            closed: false,
            accepted: false,
        }
    }

    /// Handle any slider or toggle change.
    pub fn on_change(&mut self) {
        self.changed = true;
    }

    /// Handle OK — accept changes.
    pub fn on_ok(&mut self) {
        self.accepted = true;
        self.closed = true;
    }

    /// Handle Cancel — revert to original.
    pub fn on_cancel(&mut self) {
        self.config = self.original_config;
        self.accepted = false;
        self.closed = true;
    }
}

// ---------------------------------------------------------------------------
// LoadSaveScreen
// ---------------------------------------------------------------------------

/// Action taken on the load/save screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LoadSaveAction {
    /// A save game was loaded.
    Load,
    /// The game was saved.
    Save,
    /// No action taken (cancelled).
    #[default]
    None,
}

/// Information about a save game slot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SaveGameEntry {
    /// Display name of the save.
    pub name: String,
    /// Index / identifier for the save.
    pub index: u32,
    /// Thumbnail resource ID (0 = no thumbnail).
    pub thumbnail_id: u32,
}

/// State for the load/save screen.
///
/// Manages save game list, selection, and load/save/delete actions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoadSaveScreen {
    /// Whether this screen is in load mode (true) or save mode (false).
    pub load_mode: bool,
    /// Available save game entries.
    pub entries: Vec<SaveGameEntry>,
    /// Currently selected entry index, or None.
    pub selected_index: Option<usize>,
    /// Text in the save name input field (save mode only).
    pub input_text: String,
    /// The action taken by the player.
    pub action: LoadSaveAction,
    /// Whether the screen has been closed.
    pub closed: bool,
}

impl LoadSaveScreen {
    pub fn new(load_mode: bool, entries: Vec<SaveGameEntry>) -> Self {
        Self {
            load_mode,
            entries,
            ..Default::default()
        }
    }

    /// Handle Load/Save button click.
    pub fn on_load_save(&mut self) -> bool {
        if self.load_mode {
            if self.selected_index.is_some() {
                self.action = LoadSaveAction::Load;
                self.closed = true;
                return true;
            }
        } else {
            // Save mode — need a name
            let name = if self.input_text.is_empty() {
                // Use selected entry name if available
                self.selected_index
                    .and_then(|i| self.entries.get(i))
                    .map(|e| e.name.clone())
            } else {
                Some(self.input_text.clone())
            };
            if name.is_some() {
                self.action = LoadSaveAction::Save;
                self.closed = true;
                return true;
            }
        }
        false
    }

    /// Handle Delete button click (after confirmation).
    pub fn on_delete(&mut self, confirmed: bool) {
        if confirmed
            && let Some(idx) = self.selected_index
            && idx < self.entries.len()
        {
            self.entries.remove(idx);
            self.selected_index = None;
        }
    }

    /// Handle Cancel button.
    pub fn on_cancel(&mut self) {
        self.action = LoadSaveAction::None;
        self.closed = true;
    }

    /// Handle list selection change.
    pub fn on_selection_change(&mut self, index: Option<usize>) {
        self.selected_index = index;
    }

    /// Handle double-click on list item (performs load/save directly).
    pub fn on_double_click(&mut self) {
        self.on_load_save();
    }

    /// Handle text input change (save mode).
    pub fn on_text_change(&mut self, text: String) {
        self.input_text = text;
    }

    /// Whether the load/save button should be enabled.
    pub fn can_load_save(&self) -> bool {
        if self.load_mode {
            self.selected_index.is_some()
        } else {
            !self.input_text.is_empty() || self.selected_index.is_some()
        }
    }

    /// Whether the delete button should be enabled.
    pub fn can_delete(&self) -> bool {
        self.selected_index.is_some()
    }
}

// ---------------------------------------------------------------------------
// NewPlayerScreen
// ---------------------------------------------------------------------------

/// State for the new player creation screen.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewPlayerScreen {
    /// Player name input text.
    pub name: String,
    /// Selected difficulty level index.
    pub difficulty_index: u32,
    /// Whether OK was pressed (vs Cancel).
    pub confirmed: bool,
    /// Whether the screen has been closed.
    pub closed: bool,
}

/// Maximum player name length.
pub const MAX_PLAYER_NAME_LENGTH: usize = 30;

/// Number of selectable presets, including Legendary and Advanced/Custom.
pub const DIFFICULTY_LEVEL_COUNT: u32 = 5;

impl NewPlayerScreen {
    /// Create with default difficulty (Medium = index 1).
    pub fn new() -> Self {
        Self {
            difficulty_index: 1, // Medium
            ..Default::default()
        }
    }

    /// Handle OK button.
    pub fn on_ok(&mut self) {
        self.confirmed = true;
        self.closed = true;
    }

    /// Handle Cancel button.
    pub fn on_cancel(&mut self) {
        self.confirmed = false;
        self.closed = true;
    }

    /// Get the validated player name (defaults to "Anonymous" if empty).
    ///
    /// The raw text is only replaced when fully empty — a whitespace-only
    /// name is preserved verbatim.
    pub fn validated_name(&self) -> String {
        if self.name.is_empty() {
            "Anonymous".to_string()
        } else {
            self.name.clone()
        }
    }

    /// Set the name input text (clamped to max length).
    pub fn set_name(&mut self, name: String) {
        if name.len() > MAX_PLAYER_NAME_LENGTH {
            self.name = name[..MAX_PLAYER_NAME_LENGTH].to_string();
        } else {
            self.name = name;
        }
    }
}

// ---------------------------------------------------------------------------
// SelectPlayerScreen
// ---------------------------------------------------------------------------

/// Maximum number of player profile slots.
pub const PLAYER_PROFILE_COUNT: usize = 10;

/// State for the player selection screen.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SelectPlayerScreen {
    /// Profile names for each slot (empty string = unused slot).
    pub profile_names: Vec<String>,
    /// Currently focused/selected slot index.
    pub selected_index: Option<usize>,
    /// Whether a resolution change occurred (triggers redisplay).
    pub resolution_changed: bool,
    /// Whether the screen has been closed.
    pub closed: bool,
}

impl SelectPlayerScreen {
    pub fn new(profile_names: Vec<String>) -> Self {
        Self {
            profile_names,
            ..Default::default()
        }
    }

    /// Handle Select button — activates the selected profile.
    pub fn on_select(&mut self) -> Option<usize> {
        if self.selected_index.is_some() {
            self.closed = true;
        }
        self.selected_index
    }

    /// Handle New button — caller should open NewPlayerScreen, then call
    /// `add_profile` with the result.
    pub fn add_profile(&mut self, name: String) -> Option<usize> {
        if self.profile_names.len() < PLAYER_PROFILE_COUNT {
            let idx = self.profile_names.len();
            self.profile_names.push(name);
            self.selected_index = Some(idx);
            Some(idx)
        } else {
            None
        }
    }

    /// Handle Delete button (after confirmation).
    pub fn on_delete(&mut self, confirmed: bool) {
        if confirmed
            && let Some(idx) = self.selected_index
            && idx < self.profile_names.len()
        {
            self.profile_names.remove(idx);
            self.selected_index = None;
        }
    }

    /// Handle double-click on a profile (selects immediately).
    pub fn on_double_click(&mut self, index: usize) {
        self.selected_index = Some(index);
        self.closed = true;
    }

    /// Whether the Select button should be enabled.
    pub fn can_select(&self) -> bool {
        self.selected_index
            .map(|i| i < self.profile_names.len() && !self.profile_names[i].is_empty())
            .unwrap_or(false)
    }

    /// Whether the Delete button should be enabled.
    pub fn can_delete(&self) -> bool {
        self.selected_index.is_some()
    }
}

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

// ---------------------------------------------------------------------------
// ShortcutsScreen
// ---------------------------------------------------------------------------

/// Keyboard shortcut preset type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShortcutPreset {
    Default1,
    Default2,
    UserDefined,
}

/// State for the keyboard shortcuts configuration screen.
///
/// The actual key bindings are stored in [`crate::key_config::KeyConfig`]; this
/// screen manages the editing workflow.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShortcutsScreen {
    /// Which preset is currently active.
    pub active_preset: Option<ShortcutPreset>,
    /// Whether changes were made.
    pub changed: bool,
    /// Whether the screen has been closed.
    pub closed: bool,
    /// Whether OK was pressed (vs Cancel).
    pub accepted: bool,
}

impl ShortcutsScreen {
    /// Handle OK — accept changes.
    pub fn on_ok(&mut self) {
        self.accepted = true;
        self.closed = true;
    }

    /// Handle Cancel — discard changes.
    pub fn on_cancel(&mut self) {
        self.accepted = false;
        self.closed = true;
    }
}

// ---------------------------------------------------------------------------
// MoviesScreen
// ---------------------------------------------------------------------------

/// State for the movie viewer screen.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MoviesScreen {
    /// Whether the outro button is available (campaign >= 100%).
    pub outro_available: bool,
    /// Whether the screen has been closed.
    pub closed: bool,
}

impl MoviesScreen {
    pub fn new(campaign_complete: bool) -> Self {
        Self {
            outro_available: campaign_complete,
            ..Default::default()
        }
    }

    /// Handle Outro button — caller should play the outro video.
    /// Returns None if outro is not available.
    pub fn on_outro(&self) -> Option<&'static str> {
        if self.outro_available {
            Some("Data/Cinematics/Outro.ogg")
        } else {
            None
        }
    }

    /// Handle OK button.
    pub fn on_ok(&mut self) {
        self.closed = true;
    }
}

// ---------------------------------------------------------------------------
// MissionWonPopup
// ---------------------------------------------------------------------------

/// Speed of the open/close transition animation.
const TRANSITION_SPEED: f32 = 1.5;
/// Inverse of the transition speed (for exponential decay).
const INV_TRANSITION_SPEED: f32 = 1.0 / TRANSITION_SPEED;

/// Transition phase for the mission-won popup animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TransitionPhase {
    /// No transition in progress.
    #[default]
    Idle,
    /// Popup is expanding from the source button.
    Opening,
    /// Popup is collapsing back to the source button.
    Closing,
}

/// State for the mission-won/quit transient popup with transition animation.
///
/// Manages the open/close transition and confirmation dialog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionWonPopup {
    /// Text message to display.
    pub text: String,
    /// Current transition phase.
    pub phase: TransitionPhase,
    /// Animation progress counter.
    pub transition_counter: f32,
    /// Whether the user confirmed the action.
    pub confirmed: bool,
    /// Whether this popup is for the Start button (true) or Quit button (false).
    pub is_start: bool,
    /// Whether the popup is visible.
    pub visible: bool,
}

impl Default for MissionWonPopup {
    fn default() -> Self {
        Self {
            text: String::new(),
            phase: TransitionPhase::Idle,
            transition_counter: 0.0,
            confirmed: false,
            is_start: true,
            visible: false,
        }
    }
}

impl MissionWonPopup {
    pub fn new(text: String, is_start: bool) -> Self {
        Self {
            text,
            is_start,
            ..Default::default()
        }
    }

    /// Start the opening transition.
    pub fn open(&mut self) {
        self.phase = TransitionPhase::Opening;
        self.transition_counter = TRANSITION_SPEED;
        self.visible = true;
    }

    /// Start the closing transition.
    pub fn close(&mut self) {
        self.phase = TransitionPhase::Closing;
        self.transition_counter = 0.05;
    }

    /// Advance the transition animation by one frame.
    /// Returns `true` while the transition is still in progress.
    pub fn tick(&mut self) -> bool {
        match self.phase {
            TransitionPhase::Opening => {
                self.transition_counter *= INV_TRANSITION_SPEED;
                if self.transition_counter < 0.03 {
                    self.phase = TransitionPhase::Idle;
                    return false;
                }
                true
            }
            TransitionPhase::Closing => {
                self.transition_counter *= TRANSITION_SPEED;
                if self.transition_counter >= 0.8 {
                    self.phase = TransitionPhase::Idle;
                    self.visible = false;
                    return false;
                }
                true
            }
            TransitionPhase::Idle => false,
        }
    }

    /// Handle the confirmation dialog result.
    pub fn on_confirm(&mut self, yes: bool) {
        if yes {
            self.confirmed = true;
            self.visible = false;
            self.phase = TransitionPhase::Idle;
        } else {
            self.close();
        }
    }

    /// Whether the opening transition has completed (popup fully visible).
    pub fn is_fully_open(&self) -> bool {
        self.visible && self.phase == TransitionPhase::Idle
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "ui_screens_tests.rs"]
mod tests;
