//! Menu system — state management and logic for menu screens.
//!
//! This module covers:
//! - Menu-screen timer management
//! - Window layer stack (modal/non-modal window management)
//! - Widget alignment helpers
//! - Campaign map location/ARES mapping

use robin_engine::profiles as engine_profiles;
use robin_engine::sherwood_stat as engine_sherwood_stat;
use serde::{Deserialize, Serialize};

use crate::campaign::{Campaign, CampaignValue};
use crate::profiles::{MissionLocation, MissionType};

// ═══════════════════════════════════════════════════════════════════
// Menu Screen — timer and window layer management
// ═══════════════════════════════════════════════════════════════════

/// Timer event message ID.
pub const RHMS_TIMER: u32 = 0x1000;

/// A timer managed by a menu screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuTimer {
    pub id: u32,
    pub delay_ms: u32,
    pub last_trigger_tick: u32,
}

/// Identifies a window in the layer stack. Windows are identified by a
/// unique ID assigned at registration time.
pub type WindowId = u32;

/// A layer in the window stack. A modal window creates a new layer;
/// non-modal windows are added to the current top layer.
#[derive(Debug, Clone, Default)]
pub struct WindowLayer {
    pub window_ids: Vec<WindowId>,
}

/// State management for a menu screen. Handles timers and the window
/// layer stack.
#[derive(Debug, Clone, Default)]
pub struct MenuScreenState {
    /// Stack of window layers. The top layer receives events.
    window_layers: Vec<WindowLayer>,
    /// Active timers.
    timers: Vec<MenuTimer>,
    /// Next timer ID to assign.
    next_timer_id: u32,
    /// Refresh counter for UI repainting.
    pub refresh_counter: u32,
}

impl MenuScreenState {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Timer management ────────────────────────────────────────────

    /// Create a new timer with the given delay in milliseconds.
    /// Returns the timer ID.
    pub fn create_timer(&mut self, delay_ms: u32, current_tick: u32) -> u32 {
        let id = self.next_timer_id;
        self.timers.push(MenuTimer {
            id,
            delay_ms,
            last_trigger_tick: current_tick,
        });
        self.next_timer_id += 1;
        id
    }

    /// Delete a timer by ID.
    pub fn delete_timer(&mut self, timer_id: u32) {
        self.timers.retain(|t| t.id != timer_id);
    }

    /// Check which timers should fire at the given tick count.
    /// Returns a list of timer IDs that triggered.
    pub fn refresh_timers(&mut self, current_tick: u32) -> Vec<u32> {
        let mut triggered = Vec::new();

        for timer in &mut self.timers {
            if current_tick.wrapping_sub(timer.last_trigger_tick) >= timer.delay_ms {
                timer.last_trigger_tick = current_tick;
                triggered.push(timer.id);
            }
        }

        triggered
    }

    // ── Window layer stack ──────────────────────────────────────────

    /// Register a window. If `modal`, creates a new layer and disables
    /// the previous top layer. Otherwise adds to the current top layer.
    pub fn register_window(&mut self, window_id: WindowId, modal: bool) {
        if modal {
            let mut new_layer = WindowLayer::default();
            new_layer.window_ids.push(window_id);
            self.window_layers.push(new_layer);
        } else if let Some(top) = self.window_layers.last_mut()
            && !top.window_ids.contains(&window_id)
        {
            top.window_ids.push(window_id);
        }
    }

    /// Close a window, removing it from the layer stack.
    ///
    /// If the window is the modal root of the top layer, the entire layer
    /// is popped. Otherwise the window is removed from the current layer.
    ///
    /// Returns the list of window IDs that were removed from the UI
    /// (the caller must actually remove them from the rendering system).
    pub fn close_window(&mut self, window_id: WindowId) -> Vec<WindowId> {
        let mut removed = Vec::new();

        if let Some(top) = self.window_layers.last() {
            if top.window_ids.first() == Some(&window_id) {
                // This is the modal root — pop the entire layer.
                let layer = self.window_layers.pop().unwrap();
                removed = layer.window_ids;
            } else {
                // Non-modal — just remove from the current layer.
                if let Some(top) = self.window_layers.last_mut() {
                    top.window_ids.retain(|&id| id != window_id);
                }
                removed.push(window_id);
            }
        }

        removed
    }

    /// Get the IDs of all windows in the current top layer.
    pub fn top_layer_windows(&self) -> &[WindowId] {
        self.window_layers
            .last()
            .map(|l| l.window_ids.as_slice())
            .unwrap_or(&[])
    }

    /// Number of window layers (for detecting modal pop in RefreshLoop).
    pub fn layer_count(&self) -> usize {
        self.window_layers.len()
    }

    // The legacy emergency-exit / input-translator-flag plumbing is
    // not modeled on `MenuScreenState` — we open one `show_*` modal at
    // a time rather than maintaining a modal-window layer stack, so
    // the pop-all-layers + set-flag behaviour has no in-flight state
    // to act on. The observable behaviours are still covered:
    //   * Emergency-exit on external events → every `show_*` treats
    //     `GameEvent::Quit` as dismiss; debriefing uniquely escalates
    //     it to `DebriefingOutcome::EmergencyEnd` so the caller can
    //     propagate `GameCode::Quit` (see `game_session.rs`).
    //   * Game-input-translator enable (only quick-load needs it) →
    //     `show_debriefing` takes a physical quick-load key supplied
    //     by the caller.
    //     supplied by the caller.
    //   * Translator-flag save/disable/restore around a nested modal:
    //     the equivalent call site (`game_session.rs` →
    //     `show_mission_description`) doesn't consult an input
    //     translator inside the modal at all, so the save/restore
    //     wrapper is observably a no-op.
}

// ═══════════════════════════════════════════════════════════════════
// Menu Window — widget alignment helpers
// ═══════════════════════════════════════════════════════════════════

/// A rectangle for widget positioning.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            x,
            y,
            width: w,
            height: h,
        }
    }

    pub fn bottom_right(&self) -> (f32, f32) {
        (self.x + self.width, self.y + self.height)
    }
}

/// Align a set of widget rects to the bottom-right corner of a container.
pub fn align_bottom_right(container: &Rect, widgets: &mut [Rect], spacing: f32) {
    if widgets.is_empty() {
        return;
    }

    let total_height: f32 =
        widgets.iter().map(|w| w.height).sum::<f32>() + (widgets.len() as f32 - 1.0) * spacing;
    let max_width: f32 = widgets.iter().map(|w| w.width).fold(0.0f32, f32::max);

    let (br_x, br_y) = container.bottom_right();
    let mut cur_y = br_y - total_height;
    let start_x = br_x - max_width;

    for widget in widgets.iter_mut() {
        widget.x = start_x;
        // The original alignment routine was labelled "center the widget
        // horizontally" but actually writes the offset to the Y
        // component, so a narrower widget is shifted up by half its
        // width deficit. Replicated literally for parity; a no-op when
        // all widgets share max_width.
        widget.y = cur_y + (widget.width - max_width) / 2.0;
        cur_y += widget.height + spacing;
    }
}

/// Center widgets horizontally within a container.
pub fn center_horizontally(container: &Rect, widgets: &mut [Rect], spacing: f32) {
    if widgets.is_empty() {
        return;
    }

    let line_width: f32 =
        widgets.iter().map(|w| w.width).sum::<f32>() + spacing * (widgets.len() as f32 - 1.0);

    let start_x = (container.width - line_width) / 2.0;
    let y = widgets[0].y;
    let mut cur_x = start_x;

    for widget in widgets.iter_mut() {
        widget.x = cur_x;
        widget.y = y;
        cur_x += widget.width + spacing;
    }
}

// ═══════════════════════════════════════════════════════════════════
// Campaign Map — location management and ARES mapping
// ═══════════════════════════════════════════════════════════════════

/// Number of attack arrow sprites on the campaign map.
pub const ATTACK_ARROW_COUNT: usize = 10;

/// Number of castle locations on the campaign map (the 5 cities with flags).
pub const CASTLE_COUNT: usize = 5;

/// The order of castle locations for flag display.
pub const CASTLE_LOCATIONS: [MissionLocation; CASTLE_COUNT] = [
    MissionLocation::Leicester,
    MissionLocation::Lincoln,
    MissionLocation::Derby,
    MissionLocation::York,
    MissionLocation::Nottingham,
];

/// Pixel positions for location buttons on the campaign map.
/// Order matches `MissionLocation` enum (Nowhere=0 excluded).
pub const LOCATION_POSITIONS: [(u16, u16); 10] = [
    (0, 0),     // Nowhere (unused)
    (214, 145), // Cross1
    (240, 298), // Cross2
    (349, 137), // Cross3
    (70, 198),  // Derby
    (413, 339), // Leicester
    (427, 57),  // Lincoln
    (307, 238), // Nottingham
    (0, 0),     // Sherwood (not shown on the campaign map)
    (126, 48),  // York
];

/// A location on the campaign map with its associated mission.
#[derive(Debug, Clone, Default)]
pub struct CampaignMapLocation {
    /// Index into the campaign's mission list, if a mission is assigned.
    pub mission_idx: Option<usize>,
    /// Whether the location button is enabled (has an available mission).
    pub enabled: bool,
    /// Whether the location button is blinking (mission about to expire).
    pub blinking: bool,
    /// Whether the blazon icon is shown at this location.
    pub show_blazon: bool,
    /// Whether a friendly flag is shown at this location.
    pub show_flag: bool,
}

/// State for the campaign map screen.
#[derive(Debug, Clone)]
pub struct CampaignMapState {
    /// One entry per map location (indexed by `MissionLocation` discriminant,
    /// skipping `Nowhere` and `Sherwood`).
    pub locations: Vec<CampaignMapLocation>,

    /// Which attack arrows are visible (indexed 0..ATTACK_ARROW_COUNT).
    pub attack_arrows_visible: [bool; ATTACK_ARROW_COUNT],

    /// Timer ID for the announcement text, or None.
    pub announcement_timer: Option<u32>,

    /// Timer ID for delayed debriefing display, or None.
    pub debriefing_timer: Option<u32>,

    /// Current war-crime / score / ransom display text.
    pub status_text: String,
}

impl Default for CampaignMapState {
    fn default() -> Self {
        // Create one entry per MissionLocation variant (0..=York=9).
        let locations = (0..10).map(|_| CampaignMapLocation::default()).collect();

        Self {
            locations,
            attack_arrows_visible: [false; ATTACK_ARROW_COUNT],
            announcement_timer: None,
            debriefing_timer: None,
            status_text: String::new(),
        }
    }
}

impl CampaignMapState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset all locations to disabled with no missions.
    pub fn init_locations_and_arrows(&mut self) {
        for loc in &mut self.locations {
            loc.enabled = false;
            loc.blinking = false;
            loc.show_blazon = false;
            loc.show_flag = false;
            loc.mission_idx = None;
        }
        self.attack_arrows_visible = [false; ATTACK_ARROW_COUNT];
    }

    /// Assign accessible missions to their map locations.
    pub fn assign_missions(
        &mut self,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
        mission_indices: &[usize],
    ) {
        for &idx in mission_indices {
            if let Some(mission) = campaign.missions.get(idx) {
                let profile = mission.profile(profiles);
                let loc_idx = profile.location as usize;

                if loc_idx < self.locations.len() {
                    let loc = &mut self.locations[loc_idx];
                    loc.mission_idx = Some(idx);
                    loc.enabled = true;
                    loc.blinking = mission.age == profile.life_time.saturating_sub(1);
                    loc.show_blazon = mission.produces_blazons(profiles);
                }
            }
        }
    }

    /// Convert an ARES state number to the corresponding map location.
    pub fn ares_to_location(ares_state: u32) -> MissionLocation {
        match ares_state {
            1 => MissionLocation::Leicester,
            2 | 3 => MissionLocation::Lincoln,
            4 | 5 => MissionLocation::Derby,
            6 | 7 => MissionLocation::York,
            8 => MissionLocation::Nottingham,
            _ => MissionLocation::Nowhere,
        }
    }

    /// Update attack arrow visibility based on the ARES state.
    ///
    /// An attack arrow is shown at an ARES state index if:
    /// - The ARES state matches that index
    /// - The mission at the corresponding location is PSEUDO or ATTACK type
    pub fn assign_ares_to_arrows(
        &mut self,
        ares: i8,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
    ) {
        // ARES = -1 is a no-op that preserves prior arrow state, so
        // the reset below must come *after* this gate.
        if ares < 0 {
            return;
        }

        self.attack_arrows_visible = [false; ATTACK_ARROW_COUNT];

        let ares_idx = ares as usize;
        for i in 0..ATTACK_ARROW_COUNT {
            if i == ares_idx {
                let location = Self::ares_to_location(i as u32);
                let loc_idx = location as usize;
                if loc_idx < self.locations.len()
                    && let Some(mission_idx) = self.locations[loc_idx].mission_idx
                    && let Some(mission) = campaign.missions.get(mission_idx)
                {
                    let mtype = mission.profile(profiles).mission_type;
                    if mtype == MissionType::Pseudo || mtype == MissionType::Attack {
                        self.attack_arrows_visible[i] = true;
                    }
                }
            }
        }
    }

    /// Update flag visibility at castle locations based on ARES state.
    ///
    /// As ARES progresses (1..9), castles are liberated in order:
    /// Leicester, Lincoln, Derby, York, Nottingham.
    pub fn assign_ares_to_flags(&mut self, ares: i8) {
        // Determine which castles are allied based on ARES state.
        let allied: [bool; CASTLE_COUNT] = match ares {
            1 | 2 => [true, false, false, false, false],
            3 | 4 => [true, true, false, false, false],
            5 | 6 => [true, true, true, false, false],
            7 | 8 => [true, true, true, true, false],
            9 => [true, true, true, true, true],
            _ => [false; CASTLE_COUNT],
        };

        for (i, &castle_loc) in CASTLE_LOCATIONS.iter().enumerate() {
            let loc_idx = castle_loc as usize;
            if loc_idx < self.locations.len() {
                self.locations[loc_idx].show_flag = allied[i];
            }
        }
    }

    /// Full update: reset, assign missions, arrows, and flags.
    pub fn update_all(&mut self, campaign: &Campaign, profiles: &engine_profiles::ProfileManager) {
        self.init_locations_and_arrows();
        self.assign_missions(campaign, profiles, &campaign.accessible_mission_indices);
        self.assign_ares_to_arrows(campaign.get_ares(), campaign, profiles);
        self.assign_ares_to_flags(campaign.get_ares());
    }

    /// Build the status text showing ransom, score, and preserved lives.
    ///
    /// `menu_text` supplies the localized ransom / score / preserved-lives
    /// strings; the ransom string is a `%d` format template and gets
    /// its number substituted in.
    pub fn update_war_crime_text(
        &mut self,
        campaign: &Campaign,
        menu_text: &dyn engine_sherwood_stat::MenuTextLookup,
    ) {
        use crate::ingame_menu::resources::{MT_STR_PRESERVED_LIFES, MT_STR_RANSOM, MT_STR_SCORE};

        let living = campaign.get_value(CampaignValue::LivingSoldiers) as u32;
        let dead = campaign.get_value(CampaignValue::DeadSoldiers) as u32;

        let preserved = if living > 0 || dead > 0 {
            100 * living / (living + dead)
        } else {
            0
        };

        let ransom = campaign.get_value(CampaignValue::Ransom);
        let score = campaign.get_value(CampaignValue::Score);

        let ransom_str = menu_text
            .get(MT_STR_RANSOM)
            .replacen("%d", &ransom.to_string(), 1);
        let score_label = menu_text.get(MT_STR_SCORE);
        let preserved_label = menu_text.get(MT_STR_PRESERVED_LIFES);

        self.status_text =
            format!("{ransom_str} -  {score_label} : {score}  -  {preserved_label} : {preserved}%");
    }

    /// Set an announcement text and start the announcement timer.
    pub fn set_announcement(
        &mut self,
        text: String,
        screen: &mut MenuScreenState,
        current_tick: u32,
    ) {
        if let Some(old_timer) = self.announcement_timer.take() {
            screen.delete_timer(old_timer);
        }
        self.status_text = text;
        self.announcement_timer = Some(screen.create_timer(3000, current_tick));
    }

    /// Handle the announcement timer firing — restore war crime text.
    pub fn on_announcement_timer(
        &mut self,
        screen: &mut MenuScreenState,
        campaign: &Campaign,
        menu_text: &dyn engine_sherwood_stat::MenuTextLookup,
    ) {
        if let Some(timer_id) = self.announcement_timer.take() {
            screen.delete_timer(timer_id);
        }
        self.update_war_crime_text(campaign, menu_text);
    }

    /// Get the mission index at a given location, if any.

    // ── Campaign interaction ───────────────────────────────────────

    /// Handle the player clicking on a map location.
    ///
    /// Validates that the location has an enabled mission and returns
    /// the mission index if valid.
    pub fn on_location_clicked(&self, location: MissionLocation) -> Option<usize> {
        let loc_idx = location as usize;
        let loc = self.locations.get(loc_idx)?;
        if loc.enabled { loc.mission_idx } else { None }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Blazon status for mission description
// ═══════════════════════════════════════════════════════════════════

/// Blazon requirements and conversion options for a mission.
///
/// Used by the mission description dialog to show whether the player
/// has enough blazons and what conversion options are available.
#[derive(Debug, Clone, Copy)]
pub struct BlazonStatus {
    /// Total blazons required to win the mission.
    pub required: u16,
    /// Blazons that can be collected during the mission itself.
    pub collectable: u16,
    /// Blazons the player currently has.
    pub current: u16,
    /// Whether the player can convert merry men (peasants) to blazons.
    pub can_convert_men: bool,
    /// Whether the player can play another mission to earn blazons.
    pub can_convert_mission: bool,
    /// Whether the player can buy blazons with money.
    pub can_convert_money: bool,
}

impl BlazonStatus {
    /// Whether the player has enough blazons to start the mission.
    pub fn has_enough(&self) -> bool {
        self.current >= self.required.saturating_sub(self.collectable)
    }

    /// How many more blazons the player needs.
    pub fn deficit(&self) -> u16 {
        self.required
            .saturating_sub(self.collectable)
            .saturating_sub(self.current)
    }
}

/// Convert a numeric index to `MissionLocation`.
pub fn mission_location_from_index(idx: usize) -> Option<MissionLocation> {
    match idx {
        0 => Some(MissionLocation::Nowhere),
        1 => Some(MissionLocation::Cross1),
        2 => Some(MissionLocation::Cross2),
        3 => Some(MissionLocation::Cross3),
        4 => Some(MissionLocation::Derby),
        5 => Some(MissionLocation::Leicester),
        6 => Some(MissionLocation::Lincoln),
        7 => Some(MissionLocation::Nottingham),
        8 => Some(MissionLocation::Sherwood),
        9 => Some(MissionLocation::York),
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════════════
// User choice from mission description menu
// ═══════════════════════════════════════════════════════════════════

/// The user's choice from the mission description dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissionDescriptionChoice {
    /// User cancelled / closed the dialog.
    Cancel,
    /// User chose to start the mission.
    StartMission,
    /// User chose to show pending missions.
    ShowPendingMissions,
}

// ═══════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use engine_sherwood_stat::MenuTextLookup;

    /// Test-only stub returning the menu-text id inlined into the
    /// string. That keeps assertions simple — we only care that the
    /// string substitution happens, not what it looks like.
    struct StubMenuText;
    impl MenuTextLookup for StubMenuText {
        fn get(&self, id: usize) -> String {
            use crate::ingame_menu::resources::{
                MT_STR_PRESERVED_LIFES, MT_STR_RANSOM, MT_STR_SCORE,
            };
            match id {
                MT_STR_RANSOM => "Ransom: %d".to_string(),
                MT_STR_SCORE => "Score".to_string(),
                MT_STR_PRESERVED_LIFES => "Preserved lives".to_string(),
                _ => String::new(),
            }
        }
    }

    // ── MenuScreenState tests ───────────────────────────────────────

    #[test]
    fn timer_create_and_delete() {
        let mut screen = MenuScreenState::new();
        let id1 = screen.create_timer(100, 0);
        let id2 = screen.create_timer(200, 0);
        assert_eq!(screen.timers.len(), 2);

        screen.delete_timer(id1);
        assert_eq!(screen.timers.len(), 1);
        assert_eq!(screen.timers[0].id, id2);
    }

    #[test]
    fn timer_refresh_triggers() {
        let mut screen = MenuScreenState::new();
        let id = screen.create_timer(100, 1000);

        // Not enough time passed.
        let triggered = screen.refresh_timers(1050);
        assert!(triggered.is_empty());

        // Enough time passed.
        let triggered = screen.refresh_timers(1100);
        assert_eq!(triggered, vec![id]);

        // Timer was re-triggered, so it resets.
        let triggered = screen.refresh_timers(1150);
        assert!(triggered.is_empty());
    }

    #[test]
    fn window_layer_modal() {
        let mut screen = MenuScreenState::new();

        // Register a modal window — creates layer 1.
        screen.register_window(1, true);
        assert_eq!(screen.layer_count(), 1);
        assert_eq!(screen.top_layer_windows(), &[1]);

        // Register a non-modal window in the same layer.
        screen.register_window(2, false);
        assert_eq!(screen.layer_count(), 1);
        assert_eq!(screen.top_layer_windows(), &[1, 2]);

        // Register another modal window — creates layer 2.
        screen.register_window(3, true);
        assert_eq!(screen.layer_count(), 2);
        assert_eq!(screen.top_layer_windows(), &[3]);

        // Close the modal — pops layer 2, restores layer 1.
        let removed = screen.close_window(3);
        assert_eq!(removed, vec![3]);
        assert_eq!(screen.layer_count(), 1);
        assert_eq!(screen.top_layer_windows(), &[1, 2]);
    }

    #[test]
    fn window_layer_close_non_modal() {
        let mut screen = MenuScreenState::new();
        screen.register_window(1, true);
        screen.register_window(2, false);

        let removed = screen.close_window(2);
        assert_eq!(removed, vec![2]);
        assert_eq!(screen.top_layer_windows(), &[1]);
    }

    // ── Widget alignment tests ──────────────────────────────────────

    #[test]
    fn align_bottom_right_basic() {
        let container = Rect::new(0.0, 0.0, 640.0, 480.0);
        let mut widgets = vec![
            Rect::new(0.0, 0.0, 100.0, 30.0),
            Rect::new(0.0, 0.0, 80.0, 30.0),
        ];

        align_bottom_right(&container, &mut widgets, 10.0);

        // Both should be right-aligned.
        assert_eq!(widgets[0].x, 540.0); // 640 - 100
        assert_eq!(widgets[1].x, 540.0);
        // Vertically stacked from the bottom.
        let total_h = 30.0 + 10.0 + 30.0;
        assert_eq!(widgets[0].y, 480.0 - total_h);
    }

    #[test]
    fn center_horizontally_basic() {
        let container = Rect::new(0.0, 0.0, 400.0, 300.0);
        let mut widgets = vec![
            Rect::new(0.0, 50.0, 100.0, 30.0),
            Rect::new(0.0, 50.0, 100.0, 30.0),
        ];

        center_horizontally(&container, &mut widgets, 20.0);

        // Total width: 100 + 20 + 100 = 220, centered in 400 => start at 90.
        assert_eq!(widgets[0].x, 90.0);
        assert_eq!(widgets[1].x, 210.0);
    }

    // ── CampaignMapState tests ──────────────────────────────────────

    #[test]
    fn ares_to_location_mapping() {
        assert_eq!(
            CampaignMapState::ares_to_location(1),
            MissionLocation::Leicester
        );
        assert_eq!(
            CampaignMapState::ares_to_location(2),
            MissionLocation::Lincoln
        );
        assert_eq!(
            CampaignMapState::ares_to_location(3),
            MissionLocation::Lincoln
        );
        assert_eq!(
            CampaignMapState::ares_to_location(4),
            MissionLocation::Derby
        );
        assert_eq!(
            CampaignMapState::ares_to_location(5),
            MissionLocation::Derby
        );
        assert_eq!(CampaignMapState::ares_to_location(6), MissionLocation::York);
        assert_eq!(
            CampaignMapState::ares_to_location(8),
            MissionLocation::Nottingham
        );
        assert_eq!(
            CampaignMapState::ares_to_location(0),
            MissionLocation::Nowhere
        );
        assert_eq!(
            CampaignMapState::ares_to_location(99),
            MissionLocation::Nowhere
        );
    }

    #[test]
    fn flags_follow_ares_progression() {
        let mut map = CampaignMapState::new();

        // ARES 0: no flags.
        map.assign_ares_to_flags(0);
        assert!(!map.locations[MissionLocation::Leicester as usize].show_flag);

        // ARES 1: Leicester flag.
        map.assign_ares_to_flags(1);
        assert!(map.locations[MissionLocation::Leicester as usize].show_flag);
        assert!(!map.locations[MissionLocation::Lincoln as usize].show_flag);

        // ARES 5: Leicester + Lincoln + Derby.
        map.assign_ares_to_flags(5);
        assert!(map.locations[MissionLocation::Leicester as usize].show_flag);
        assert!(map.locations[MissionLocation::Lincoln as usize].show_flag);
        assert!(map.locations[MissionLocation::Derby as usize].show_flag);
        assert!(!map.locations[MissionLocation::York as usize].show_flag);

        // ARES 9: all flags.
        map.assign_ares_to_flags(9);
        for &loc in &CASTLE_LOCATIONS {
            assert!(
                map.locations[loc as usize].show_flag,
                "Expected flag at {:?} for ARES 9",
                loc
            );
        }
    }

    #[test]
    fn init_locations_resets() {
        let mut map = CampaignMapState::new();
        map.locations[1].enabled = true;
        map.locations[1].mission_idx = Some(5);
        map.attack_arrows_visible[3] = true;

        map.init_locations_and_arrows();

        assert!(!map.locations[1].enabled);
        assert!(map.locations[1].mission_idx.is_none());
        assert!(!map.attack_arrows_visible[3]);
    }

    #[test]
    fn assign_ares_to_arrows_negative_is_noop() {
        let mut map = CampaignMapState::new();
        map.attack_arrows_visible[3] = true;

        let profiles = engine_profiles::ProfileManager::new();
        map.assign_ares_to_arrows(-1, &Campaign::default(), &profiles);

        assert!(map.attack_arrows_visible[3]);
    }

    #[test]
    fn war_crime_text_format() {
        let mut map = CampaignMapState::new();
        let mut campaign = Campaign::default();
        campaign.set_value(CampaignValue::Ransom, 500);
        campaign.set_value(CampaignValue::Score, 1200);
        campaign.set_value(CampaignValue::LivingSoldiers, 80);
        campaign.set_value(CampaignValue::DeadSoldiers, 20);

        map.update_war_crime_text(&campaign, &StubMenuText);

        assert!(map.status_text.contains("500"));
        assert!(map.status_text.contains("1200"));
        assert!(map.status_text.contains("80%"));
    }

    #[test]
    fn announcement_timer_lifecycle() {
        let mut map = CampaignMapState::new();
        let mut screen = MenuScreenState::new();
        let campaign = Campaign::default();

        map.set_announcement("Test!".into(), &mut screen, 1000);
        assert!(map.announcement_timer.is_some());
        assert_eq!(map.status_text, "Test!");
        assert_eq!(screen.timers.len(), 1);

        map.on_announcement_timer(&mut screen, &campaign, &StubMenuText);
        assert!(map.announcement_timer.is_none());
        assert_eq!(screen.timers.len(), 0);
        // Status text is now war crime text (defaults to zeros).
        assert!(map.status_text.contains("0"));
    }

    #[test]
    fn mission_location_from_index_roundtrip() {
        let locations = [
            MissionLocation::Nowhere,
            MissionLocation::Cross1,
            MissionLocation::Cross2,
            MissionLocation::Cross3,
            MissionLocation::Derby,
            MissionLocation::Leicester,
            MissionLocation::Lincoln,
            MissionLocation::Nottingham,
            MissionLocation::Sherwood,
            MissionLocation::York,
        ];
        for (i, &loc) in locations.iter().enumerate() {
            assert_eq!(mission_location_from_index(i), Some(loc));
        }
        assert_eq!(mission_location_from_index(99), None);
    }

    // ── Campaign interaction tests ─────────────────────────────────

    #[test]
    fn on_location_clicked_enabled() {
        let mut map = CampaignMapState::new();
        let loc_idx = MissionLocation::Derby as usize;
        map.locations[loc_idx].enabled = true;
        map.locations[loc_idx].mission_idx = Some(3);

        assert_eq!(map.on_location_clicked(MissionLocation::Derby), Some(3));
    }

    #[test]
    fn on_location_clicked_disabled() {
        let mut map = CampaignMapState::new();
        let loc_idx = MissionLocation::Derby as usize;
        map.locations[loc_idx].enabled = false;
        map.locations[loc_idx].mission_idx = Some(3);

        assert_eq!(map.on_location_clicked(MissionLocation::Derby), None);
    }

    #[test]
    fn on_location_clicked_no_mission() {
        let map = CampaignMapState::new();
        assert_eq!(map.on_location_clicked(MissionLocation::Derby), None);
    }

    #[test]
    fn blazon_status_has_enough() {
        let status = BlazonStatus {
            required: 10,
            collectable: 3,
            current: 7,
            can_convert_men: false,
            can_convert_mission: false,
            can_convert_money: false,
        };
        assert!(status.has_enough()); // 7 >= 10 - 3

        let status2 = BlazonStatus {
            required: 10,
            collectable: 3,
            current: 5,
            can_convert_men: false,
            can_convert_mission: false,
            can_convert_money: false,
        };
        assert!(!status2.has_enough()); // 5 < 10 - 3

        assert_eq!(status.deficit(), 0);
        assert_eq!(status2.deficit(), 2);
    }

    #[test]
    fn blazon_status_zero_required() {
        let status = BlazonStatus {
            required: 0,
            collectable: 0,
            current: 0,
            can_convert_men: false,
            can_convert_mission: false,
            can_convert_money: false,
        };
        assert!(status.has_enough());
        assert_eq!(status.deficit(), 0);
    }
}
