//! Menu system — state management and logic for menu screens.
//!
//! This module covers:
//! - Campaign map location/ARES mapping

use robin_engine::profiles as engine_profiles;
use robin_engine::sherwood_stat as engine_sherwood_stat;

use robin_engine::campaign::{Campaign, CampaignValue};
use robin_engine::profiles::{MissionLocation, MissionType};

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
