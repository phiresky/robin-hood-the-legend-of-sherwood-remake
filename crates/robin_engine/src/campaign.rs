//! Campaign state singleton.
//!
//! Manages mission progression, gang membership, currency/values,
//! mission team selection, ARES state, and peasant names.
//!
//! This module covers pure state management; methods that depend on
//! engine callbacks (sound, UI, character spawning) live elsewhere.

use enum_map::{Enum, EnumMap, enum_map};
use serde::{Deserialize, Serialize};

use crate::mission::{Mission, MissionStatus};
use crate::pc_status::{LIFEPOINTS_PC, PcStatus, SkillName};
use crate::player_profile::DifficultyLevel;
use crate::player_profile::difficulty_params;
use crate::profiles::{CharacterProfileIdx, MissionType, ProfileManager};

// ─── Reservist reintegration coefficients ────────────────────────

/// Experience-scale coefficient applied to a reservist's hand-to-hand
/// skill when they return to the gang.
pub const COEFFICIENT_RESERVIST_HAND_TO_HAND: f32 = 1.5;

/// Experience-scale coefficient applied to a reservist's bow skill when
/// they return to the gang.
pub const COEFFICIENT_RESERVIST_BOW: f32 = 1.5;

/// Life-point multiplier applied to a reservist's life when they
/// return to the gang.  Note the original engine casts this `1.5f` to a
/// signed integer **before** the multiplication, which truncates to
/// `1` — the scaling is a no-op and is followed by a clamp to
/// `LIFEPOINTS_PC`.  [`Campaign::move_to_gang`] reproduces that
/// truncation so we match the original (buggy) behaviour exactly
/// rather than "fixing" it silently.
pub const COEFFICIENT_RESERVIST_LIFE: f32 = 1.5;

// ─── Enums ───────────────────────────────────────────────────────

#[repr(u32)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum LevelResult {
    Completed = 0,
    Lost = 1,
    Retreat = 2,
}

#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Enum,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum CampaignValue {
    Amulets = 0,
    Ransom = 1,
    Score = 2,
    Blazon = 3,
    LivingSoldiers = 4,
    DeadSoldiers = 5,
    MissionLength = 6,
    Custom1 = 7,
    Custom2 = 8,
    Custom3 = 9,
    Custom4 = 10,
    Custom5 = 11,
    Custom6 = 12,
    Custom7 = 13,
    Custom8 = 14,
    Custom9 = 15,
    Custom10 = 16,
    Custom11 = 17,
    Custom12 = 18,
    Custom13 = 19,
    Custom14 = 20,
    Custom15 = 21,
    Custom16 = 22,
    Custom17 = 23,
    Custom18 = 24,
    Custom19 = 25,
    Custom20 = 26,
}

impl CampaignValue {
    /// Map the script-facing custom-value ordinal (`0..20`) onto the
    /// campaign's canonical custom slots.
    pub fn custom(index: i32) -> Option<Self> {
        use CampaignValue::*;
        Some(match index {
            0 => Custom1,
            1 => Custom2,
            2 => Custom3,
            3 => Custom4,
            4 => Custom5,
            5 => Custom6,
            6 => Custom7,
            7 => Custom8,
            8 => Custom9,
            9 => Custom10,
            10 => Custom11,
            11 => Custom12,
            12 => Custom13,
            13 => Custom14,
            14 => Custom15,
            15 => Custom16,
            16 => Custom17,
            17 => Custom18,
            18 => Custom19,
            19 => Custom20,
            _ => return None,
        })
    }
}

pub const INITIAL_RANSOM: i32 = 100;

/// Maximum number of amulets a campaign can accumulate.  Once the
/// campaign's `CampaignValue::Amulets` counter reaches this, additional
/// amulet pickups are refused (see `engine::commands::is_pc_takable`).
pub const MAXIMUM_AMULETS_NUMBER: i32 = 10;

// ─── PC Description (simplified) ────────────────────────────────

/// A player character's dynamic state. The static profile data is
/// referenced by index into the ProfileManager.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PcDescription {
    /// Index into ProfileManager.characters (None = unlinked).
    pub character_profile_idx: Option<CharacterProfileIdx>,
    pub instanced: bool,
    /// Dynamic status: HP, skills, inventory.
    pub status: PcStatus,
}

// ─── Campaign ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct Campaign {
    // ── Values / currency ──
    pub values: EnumMap<CampaignValue, i32>,
    pub ares: i8,

    // ── Missions ──
    pub missions: Vec<Mission>,
    /// Indices into `missions` for accessible missions.
    pub accessible_mission_indices: Vec<usize>,
    pub pending_accessible_mission_indices: Vec<usize>,

    pub last_mission_idx: Option<usize>,
    pub current_mission_idx: Option<usize>,
    pub next_mission_idx: Option<usize>,
    pub blazon_mission_idx: Option<usize>,
    pub last_played_mission_indices: Vec<usize>,

    pub last_pseudo_mission_status: MissionStatus,
    pub last_pseudo_mission_id: u32,

    // ── Characters / gang ──
    pub characters: Vec<PcDescription>,
    /// Indices into `characters` for active gang members.
    pub gang_indices: Vec<usize>,
    /// Indices into `characters` for reservists.
    pub reservist_indices: Vec<usize>,
    /// Indices into `characters` for the upcoming mission team.
    pub mission_team_indices: Vec<usize>,

    pub peasant_names: Vec<String>,
    pub reservists_are_back: bool,

    // ── Collected relics (enum values stored as u32) ──
    pub collected_relics: Vec<u32>,

    /// Per-type production sectors.  Indexed by `sector_production::Type`
    /// (0..TYPE_NUMBER).  Created by `create_production_sectors` during
    /// campaign init; populated at runtime via `register_as_production_sector`
    /// / `add_production_point`.
    pub production_sectors: Vec<crate::sector_production::SectorProduction>,

    /// Pre-mission snapshot captured by [`Campaign::snapshot`] at mission
    /// start and consumed by [`Campaign::restore_snapshot`] when the
    /// player abandons / restarts.  Stored with campaign state so
    /// abandon/restart behavior survives save/load and rollback
    /// snapshots. Boxed to avoid recursive sizing.
    pub pre_mission_snapshot: Option<Box<Campaign>>,
    /// Authoritative simulation state immediately before mission selection.
    /// Kept beside the existing campaign restart snapshot so save-loaded
    /// `LevelRestart` can replay selection from the identical RNG position.
    pub pre_mission_rng_seed: Option<u64>,
    pub pre_mission_sim_config: Option<crate::engine::SimConfig>,
    /// Whether the restart snapshot already names the mission that should be
    /// constructed. Direct launches do not run campaign selection, so their
    /// restart path must rebuild that exact mission without selecting again.
    pub pre_mission_was_preselected: bool,
}

impl Default for Campaign {
    fn default() -> Self {
        // ARES starts at -1 and ransom is seeded to INITIAL_RANSOM so a
        // bare `Campaign::new()` already matches the post-constructor
        // state without requiring callers to invoke `reset()`.
        // `mission_team_indices` stays empty until `from_profiles` /
        // `reset` runs (it's seeded from the gang, which doesn't exist
        // on a default-constructed campaign).
        let mut values = enum_map! { _ => 0 };
        values[CampaignValue::Ransom] = INITIAL_RANSOM;
        Campaign {
            values,
            ares: -1,
            missions: Vec::new(),
            accessible_mission_indices: Vec::new(),
            pending_accessible_mission_indices: Vec::new(),
            last_mission_idx: None,
            current_mission_idx: None,
            next_mission_idx: None,
            blazon_mission_idx: None,
            last_played_mission_indices: Vec::new(),
            last_pseudo_mission_status: MissionStatus::Available,
            last_pseudo_mission_id: 0,
            characters: Vec::new(),
            gang_indices: Vec::new(),
            reservist_indices: Vec::new(),
            mission_team_indices: Vec::new(),
            peasant_names: Vec::new(),
            reservists_are_back: false,
            collected_relics: Vec::new(),
            production_sectors: default_production_sectors(),
            pre_mission_snapshot: None,
            pre_mission_rng_seed: None,
            pre_mission_sim_config: None,
            pre_mission_was_preselected: false,
        }
    }
}

/// Build the `TYPE_NUMBER`-long production-sector array with each slot tagged
/// by its `Type`.
fn default_production_sectors() -> Vec<crate::sector_production::SectorProduction> {
    use crate::sector_production::{SectorProduction, Type};
    [
        Type::MakeArrow,
        Type::MakePurse,
        Type::MakeStone,
        Type::MakeApple,
        Type::MakeAle,
        Type::MakeLamblegg,
        Type::MakePlant,
        Type::MakeNet,
        Type::MakeWaspNest,
        Type::TrainBow,
        Type::TrainHandToHand,
        Type::Heal,
        Type::Relic,
    ]
    .into_iter()
    .map(SectorProduction::new)
    .collect()
}

/// Calculate post-mission peasant recruitment count from the "warcrime" ratio.
///
/// The ratio measures how many enemy soldiers the player left alive (mercy).
/// Difficulty modifies the penalty:
/// - Easy: halved penalty (`1.0 - 0.5 * (1.0 - ratio)`)
/// - Medium: unmodified
/// - Hard: doubled penalty (`1.0 - 2.0 * (1.0 - ratio)`), clamped to 0
///
/// Result is `min_team + (u16)warcrime * (max_team - min_team)`.
///
/// NOTE: warcrime is cast to `u16` *before* multiplying, which truncates
/// any fractional value to 0. This means bonus peasants beyond
/// `min_team` are only awarded when warcrime is exactly 1.0 (all
/// soldiers left alive). This matches the original behavior.
pub fn calculate_warcrime_recruitment(
    living_soldiers: u32,
    dead_soldiers: u32,
    difficulty: DifficultyLevel,
    min_new_team_members: u16,
    max_new_team_members: u16,
) -> u32 {
    let total = dead_soldiers + living_soldiers;
    let mut warcrime = if total != 0 {
        living_soldiers as f32 / total as f32
    } else {
        0.0
    };

    match difficulty {
        DifficultyLevel::Easy => {
            warcrime = 1.0 - difficulty_params::EASY_CARNAGE * (1.0 - warcrime);
        }
        DifficultyLevel::Hard => {
            warcrime = 1.0 - difficulty_params::HARD_CARNAGE * (1.0 - warcrime);
            if warcrime < 0.0 {
                warcrime = 0.0;
            }
        }
        DifficultyLevel::Medium => {}
    }

    // Truncate to u16 before multiplying (likely a bug, but we match
    // the original behavior).
    let warcrime_int = warcrime as u16;
    let range = max_new_team_members.saturating_sub(min_new_team_members);
    (min_new_team_members + warcrime_int * range) as u32
}

impl Campaign {
    /// Create a new campaign from loaded profiles.
    pub fn from_profiles(profiles: &ProfileManager, difficulty: DifficultyLevel) -> Campaign {
        let mut campaign = Campaign {
            ..Default::default()
        };

        // Create missions from profiles
        for (i, mp) in profiles.missions.iter().enumerate() {
            let mut m = Mission::new();
            m.profile_idx = Some(i as u32);
            m.blazon_price = mp.blazon_price;
            campaign.missions.push(m);
        }

        // Create character-pool entries for every character profile.
        // Each entry is constructed with full pockets — ammo is seeded
        // from the profile's max-ammo values, then scaled by the active
        // player profile's difficulty tier.
        //
        // The original game starts the campaign with **only** "Robin
        // des bois" in the gang; recruitment of the rest happens later
        // through sector production, story events, and scripts.
        //
        // We still materialise a `PcDescription` for every character
        // profile so `get_character_by_profile` and index-based
        // recruitment (`add_new_peasant_to_gang`,
        // `rescue_pc_by_profile_name`) stay trivial — but
        // `gang_indices` is seeded with Robin alone so a fresh
        // campaign matches the "Robin alone in Sherwood" start.
        assert!(
            profiles.characters.len() > 1,
            "campaign init: need at least two character profiles (Robin Town + Robin Hood)"
        );
        let mut robin_char_idx: Option<usize> = None;
        for (i, cp) in profiles.characters.iter().enumerate() {
            campaign.characters.push(PcDescription {
                character_profile_idx: Some(CharacterProfileIdx(i as u32)),
                instanced: false,
                status: PcStatus::from_profile(cp, true, difficulty),
            });
            if robin_char_idx.is_none() && cp.profile_name == "Robin des bois" {
                robin_char_idx = Some(i);
            }
        }
        // Locate Robin by profile name to tolerate reordering of the
        // character-profile list (the loader may or may not include
        // "Robin des villes"), falling back on fixed index 1 when the
        // name lookup fails — the assertion above guarantees that
        // index is populated.
        campaign.seed_initial_gang(robin_char_idx.unwrap_or(1));

        campaign
    }

    pub fn new() -> Self {
        Self::default()
    }

    fn seed_initial_gang(&mut self, robin_char_idx: usize) {
        self.gang_indices.clear();
        self.gang_indices.push(robin_char_idx);
    }

    // ── Pre-mission snapshot ──────────────────────────────────────

    /// Capture the current campaign state as the pre-mission snapshot.
    ///
    /// Called at mission start.  The snapshot lives in-memory on the
    /// campaign itself. Any previous snapshot is overwritten; the nested
    /// snapshot field is cleared so the stored state doesn't grow
    /// unboundedly across repeated mission starts.
    pub fn snapshot(&mut self) {
        let mut snap = self.clone();
        snap.pre_mission_snapshot = None;
        self.pre_mission_snapshot = Some(Box::new(snap));
    }

    /// Capture the campaign and the authoritative pre-selection simulation
    /// checkpoint as one restart boundary.
    pub fn snapshot_with_simulation(
        &mut self,
        rng_seed: u64,
        sim_config: crate::engine::SimConfig,
    ) {
        self.snapshot_with_simulation_mode(rng_seed, sim_config, false);
    }

    fn snapshot_with_simulation_mode(
        &mut self,
        rng_seed: u64,
        sim_config: crate::engine::SimConfig,
        preselected: bool,
    ) {
        self.pre_mission_rng_seed = Some(rng_seed);
        self.pre_mission_sim_config = Some(sim_config);
        self.pre_mission_was_preselected = preselected;
        self.snapshot();
    }

    /// Capture a direct/preselected mission boundary. Unlike the ordinary
    /// session boundary, restart must not run campaign selection again.
    pub fn snapshot_preselected_with_simulation(
        &mut self,
        rng_seed: u64,
        sim_config: crate::engine::SimConfig,
    ) {
        self.snapshot_with_simulation_mode(rng_seed, sim_config, true);
    }

    pub fn has_restart_simulation_checkpoint(&self) -> bool {
        self.pre_mission_snapshot.is_some()
            && self.pre_mission_rng_seed.is_some()
            && self.pre_mission_sim_config.is_some()
    }

    /// Required pre-selection checkpoint paired with the current snapshot.
    pub fn restart_simulation_checkpoint(&self) -> (u64, crate::engine::SimConfig) {
        (
            self.pre_mission_rng_seed
                .expect("campaign restart snapshot is missing its RNG checkpoint"),
            self.pre_mission_sim_config
                .expect("campaign restart snapshot is missing its SimConfig checkpoint"),
        )
    }

    /// Restore the pre-mission snapshot captured by [`Campaign::snapshot`].
    /// Called when the player abandons / restarts the mission.
    ///
    /// Returns `true` if a snapshot was available and applied.
    /// `profiles` (the `Arc<ProfileManager>`) is preserved across the
    /// restore so engine-side references remain valid.
    pub fn restore_snapshot(&mut self) -> bool {
        let Some(snap) = self.pre_mission_snapshot.take() else {
            return false;
        };
        *self = *snap;
        true
    }

    // ── Values / currency ──

    pub fn get_value(&self, name: CampaignValue) -> i32 {
        self.values[name]
    }

    pub fn set_value(&mut self, name: CampaignValue, val: i32) {
        self.values[name] = val;
    }

    pub fn add_value(&mut self, name: CampaignValue, amount: i32) {
        self.values[name] += amount;
    }

    pub fn subtract_value(&mut self, name: CampaignValue, amount: i32) {
        self.values[name] -= amount;
    }

    /// Award experience to a PC's skill and pay the campaign-score
    /// bonus whenever the call crosses a 100-XP boundary (capacity
    /// increase).
    ///
    /// Strict inequality is used (capacity changed at all), matching
    /// the original behaviour.
    pub fn add_pc_experience(
        &mut self,
        profile_idx: usize,
        skill: crate::pc_status::SkillName,
        xp: u32,
    ) {
        let Some(desc) = self.characters.get_mut(profile_idx) else {
            return;
        };
        let prev_capacity = desc.status.human_status.skill(skill).capacity;
        desc.status.human_status.add_experience(skill, xp);
        let new_capacity = desc.status.human_status.skill(skill).capacity;
        if new_capacity != prev_capacity {
            self.add_value(
                CampaignValue::Score,
                crate::pc_status::PC_ADDITIONAL_CAPACITY_POINTS,
            );
        }
    }

    // ── ARES ──

    pub fn get_ares(&self) -> i8 {
        self.ares
    }

    pub fn set_ares(&mut self, state: i8) {
        self.ares = state;
    }

    /// Set ARES state. A value of -1 means "no change" (keep old state).
    pub fn set_ares_conditional(&mut self, new_state: i8) {
        if new_state != -1 {
            self.ares = new_state;
        }
    }

    // ── Mission access ──

    /// Find a mission by its profile ID.
    pub fn get_mission(&self, profile_id: u32, profiles: &ProfileManager) -> Option<&Mission> {
        self.missions.iter().find(|m| {
            m.profile_idx
                .and_then(|idx| profiles.missions.get(idx as usize))
                .is_some_and(|p| p.id == profile_id)
        })
    }

    /// Count how many missions have been completed (won or lost).
    pub fn get_number_of_missions_done(&self) -> usize {
        self.missions.iter().filter(|m| m.is_done()).count()
    }

    /// Campaign progression percentage.
    /// Excludes Ambush/Tactical missions. Special-cases mission IDs
    /// 'DD' (D04) and 'IH' (H12) for 95%/100% overrides.
    pub fn get_progression(&self, profiles: &ProfileManager) -> u32 {
        let mut accomplished = 0u32;
        let mut total = 0u32;
        let mut d04_done = false;
        let mut h12_done = false;

        for m in &self.missions {
            let p = m.profile(profiles);
            if p.mission_type == MissionType::Ambush || p.mission_type == MissionType::Tactical {
                continue;
            }
            total += 1;
            if m.status == MissionStatus::Won {
                accomplished += 1;
                // Two-byte ID constants: 'DD' = 0x4444, 'IH' = 0x4948.
                match p.id {
                    0x4444 => d04_done = true, // 'DD'
                    0x4948 => h12_done = true, // 'IH'
                    _ => {}
                }
            }
        }

        if h12_done {
            100
        } else if d04_done {
            95
        } else {
            (100 * accomplished).checked_div(total).unwrap_or(0)
        }
    }

    // ── Gang management ──

    pub fn get_size_of_gang(&self) -> usize {
        self.gang_indices.len()
    }

    pub fn is_in_gang(&self, character_profile_idx: CharacterProfileIdx) -> bool {
        self.gang_indices.iter().any(|&gi| {
            self.characters
                .get(gi)
                .and_then(|c| c.character_profile_idx)
                == Some(character_profile_idx)
        })
    }

    /// Pick a random peasant from the gang that isn't yet instantiated
    /// in the current mission.  Prefers candidates whose
    /// `character_profile_idx` matches `preferred_profile_idx`; falls
    /// back to any non-VIP uninstanced gang member if none match.
    ///
    /// Returns the index into `self.characters` (the `PcDescription`
    /// slot), or `None` when the gang has no eligible peasant.
    pub fn get_random_peasant_from_gang(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        preferred_profile_idx: Option<CharacterProfileIdx>,
        profiles: &ProfileManager,
    ) -> Option<usize> {
        let mut preferred: Vec<usize> = Vec::new();
        let mut others: Vec<usize> = Vec::new();
        for &gi in &self.gang_indices {
            let desc = match self.characters.get(gi) {
                Some(d) => d,
                None => continue,
            };
            if desc.instanced {
                continue;
            }
            let Some(cpi) = desc.character_profile_idx else {
                continue;
            };
            let is_vip = profiles
                .get_character(cpi)
                .map(|cp| cp.vip)
                .unwrap_or(false);
            if is_vip {
                continue;
            }
            if preferred_profile_idx == Some(cpi) {
                preferred.push(gi);
            } else {
                others.push(gi);
            }
        }
        let pool = if !preferred.is_empty() {
            &preferred
        } else if !others.is_empty() {
            &others
        } else {
            return None;
        };
        let pick = crate::sim_rng::usize(
            sim,
            crate::sim_rng::RngSite::CampaignReinforcementPeasant,
            0..pool.len(),
        );
        Some(pool[pick])
    }

    /// Check if a character profile is in the gang and NOT instanced (still in Sherwood).
    pub fn is_in_sherwood(&self, character_profile_idx: CharacterProfileIdx) -> bool {
        self.gang_indices.iter().any(|&gi| {
            self.characters.get(gi).is_some_and(|desc| {
                desc.character_profile_idx == Some(character_profile_idx) && !desc.instanced
            })
        })
    }

    /// Add a character to the gang.
    pub fn add_to_gang(&mut self, char_idx: usize, profiles: &ProfileManager) {
        // VIP characters must not already be in the gang.
        if let Some(desc) = self.characters.get(char_idx)
            && let Some(profile_idx) = desc.character_profile_idx
            && let Some(cp) = profiles.get_character(profile_idx)
        {
            debug_assert!(
                !cp.vip || !self.is_in_gang(profile_idx),
                "VIP character already in gang"
            );
        }
        if !self.gang_indices.contains(&char_idx) {
            self.gang_indices.push(char_idx);
        }
    }

    pub fn remove_from_gang(&mut self, char_idx: usize) {
        self.gang_indices.retain(|&i| i != char_idx);
    }

    pub fn move_to_reservists(&mut self, char_idx: usize) {
        self.remove_from_gang(char_idx);
        if !self.reservist_indices.contains(&char_idx) {
            self.reservist_indices.push(char_idx);
        }
    }

    /// Move a reservist back into the gang, applying the reintegration
    /// cost: experience is scaled by the reservist coefficients, life
    /// points are multiplied and clamped, inventory is cleared, and the
    /// Sherwood beam-me slot is released.
    ///
    /// If the character index is not actually a known reservist we
    /// still apply the list move (via [`Campaign::add_to_gang`]) but
    /// skip the stat mutations and emit a warning — stat side
    /// effects on a non-reservist would be wrong.
    pub fn move_to_gang(&mut self, char_idx: usize, profiles: &ProfileManager) {
        let was_reservist = self.reservist_indices.contains(&char_idx);
        self.reservist_indices.retain(|&i| i != char_idx);

        if was_reservist {
            if let Some(desc) = self.characters.get_mut(char_idx) {
                // Update fighting ability.
                desc.status
                    .human_status
                    .scale_experience(SkillName::HandToHand, COEFFICIENT_RESERVIST_HAND_TO_HAND);
                desc.status
                    .human_status
                    .scale_experience(SkillName::Bow, COEFFICIENT_RESERVIST_BOW);

                // Update life points.  The original casts the coefficient
                // to a signed integer before multiplying, which truncates
                // 1.5 to 1 — the scaling is a no-op and only the clamp
                // to `LIFEPOINTS_PC` has effect.  Mirror that truncation
                // exactly.
                let coeff = COEFFICIENT_RESERVIST_LIFE as i16;
                desc.status.life_points = desc.status.life_points.saturating_mul(coeff);
                if desc.status.life_points > LIFEPOINTS_PC {
                    desc.status.life_points = LIFEPOINTS_PC;
                }

                // Clear inventory.
                desc.status.reset_ammo();

                // Release Sherwood beam-me slot.
                desc.status.beam_me_index_in_sherwood = -1;
            } else {
                tracing::warn!(
                    "move_to_gang: reservist char_idx {char_idx} has no character description",
                );
            }
        }

        self.add_to_gang(char_idx, profiles);
    }

    // ── Mission team ──

    pub fn get_size_of_mission_team(&self) -> usize {
        self.mission_team_indices.len()
    }

    /// Character profile indices of all members of the upcoming mission team.
    pub fn mission_team_profile_indices(&self) -> Vec<CharacterProfileIdx> {
        self.mission_team_indices
            .iter()
            .filter_map(|&idx| self.characters.get(idx))
            .filter_map(|d| d.character_profile_idx)
            .collect()
    }

    pub fn add_to_mission_team(&mut self, char_idx: usize) {
        if !self.mission_team_indices.contains(&char_idx) {
            self.mission_team_indices.push(char_idx);
        }
    }

    pub fn remove_from_mission_team(&mut self, char_idx: usize) {
        self.mission_team_indices.retain(|&i| i != char_idx);
    }

    pub fn reset_mission_team(&mut self) {
        self.mission_team_indices.clear();
    }

    pub fn is_in_mission_team(&self, char_idx: usize) -> bool {
        self.mission_team_indices.contains(&char_idx)
    }

    /// Add all gang members to the mission team.
    pub fn add_all_to_mission_team(&mut self) {
        self.mission_team_indices.clear();
        self.mission_team_indices
            .extend_from_slice(&self.gang_indices);
    }

    // ── Character pool ──

    /// True if any `PcDescription` in the character pool already
    /// references the given character profile.  Used by
    /// [`Campaign::add_to_characters`] to enforce the VIP-uniqueness
    /// invariant.
    ///
    /// We deliberately query at the profile level only: descriptions
    /// are stored by profile index and there's no separate
    /// "this-exact-description" identity, so a per-profile check is
    /// the natural granularity for VIPs (where uniqueness is
    /// required) and is harmless for non-VIPs (where two peasants
    /// could legitimately share a profile).
    pub fn has_character_for_profile(&self, profile_idx: CharacterProfileIdx) -> bool {
        self.characters
            .iter()
            .any(|c| c.character_profile_idx == Some(profile_idx))
    }

    /// Find the character index (position in `self.characters`) for a given
    /// character profile index. Returns `None` if not found.
    pub fn get_character_by_profile(&self, profile_idx: CharacterProfileIdx) -> Option<usize> {
        self.characters
            .iter()
            .position(|c| c.character_profile_idx == Some(profile_idx))
    }

    /// Rescue a PC into the gang by profile name.
    ///
    /// Walks the character pool (which holds every potential recruit's
    /// description) looking for a matching profile name and adds them
    /// to the gang through `add_to_gang`.
    ///
    /// Returns `true` if the named profile was found and added (or was
    /// already in the gang — still counts as success).
    pub fn rescue_pc_by_profile_name(
        &mut self,
        name: &str,
        profiles: &ProfileManager,
        difficulty: DifficultyLevel,
    ) -> bool {
        // Find the character profile id matching the name (case-sensitive).
        let profile_idx = profiles.character_idx_by_name(name);
        let Some(profile_idx) = profile_idx else {
            tracing::warn!("rescue_pc_by_profile_name: no character profile named {name:?}");
            return false;
        };

        // Skip VIPs already in the gang.
        let is_vip = profiles
            .get_character(profile_idx)
            .map(|cp| cp.vip)
            .unwrap_or(false);
        if is_vip && self.is_in_gang(profile_idx) {
            return true;
        }

        let char_idx = match self.get_character_by_profile(profile_idx) {
            Some(idx) => idx,
            None => {
                // Not yet in the character pool — materialise a
                // PcDescription with full, difficulty-scaled pockets.
                // The PRIS rescue-PC spawn path doesn't push to
                // `campaign.characters` up front, so we do it here.
                let cp = profiles
                    .get_character(profile_idx)
                    .expect("rescue_pc_by_profile_name: profile_idx just resolved from name");
                let desc = PcDescription {
                    character_profile_idx: Some(profile_idx),
                    instanced: false,
                    status: PcStatus::from_profile(cp, true, difficulty),
                };
                self.add_to_characters(desc, profiles)
            }
        };
        self.add_to_gang(char_idx, profiles);
        true
    }

    /// Add a `PcDescription` to the character pool. Returns the new index.
    /// VIP characters must not already exist in the pool.
    pub fn add_to_characters(&mut self, desc: PcDescription, profiles: &ProfileManager) -> usize {
        if let Some(profile_idx) = desc.character_profile_idx
            && let Some(cp) = profiles.get_character(profile_idx)
        {
            assert!(
                !cp.vip || !self.has_character_for_profile(profile_idx),
                "VIP character already in character pool"
            );
        }
        let idx = self.characters.len();
        self.characters.push(desc);
        idx
    }

    // ── Peasant names ──

    pub fn is_peasant_name_registered(&self, name: &str) -> bool {
        self.peasant_names.iter().any(|n| n == name)
    }

    pub fn register_peasant_name(&mut self, name: String) {
        self.peasant_names.push(name);
    }

    // ── Reservists ──

    pub fn are_reservists_back(&self) -> bool {
        self.reservists_are_back
    }

    pub fn set_reservists_back(&mut self, value: bool) {
        self.reservists_are_back = value;
    }

    // ── Relics ──

    pub fn add_relic(&mut self, relic_type: u32) {
        self.collected_relics.push(relic_type);
    }

    // ── Peasant recruitment ──

    /// Add a new peasant to the gang, or bring back a reservist (50% chance).
    /// `peasant_type`: index into non-VIP profiles (0-based). If `None` or
    /// out of range, picks randomly. Returns the character index in `self.characters`.
    pub fn add_new_peasant_to_gang(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        peasant_type: Option<u16>,
        profiles: &ProfileManager,
    ) -> usize {
        // 50% chance to bring back a reservist if any exist
        if !self.reservist_indices.is_empty()
            && crate::sim_rng::bool(sim, crate::sim_rng::RngSite::CampaignReservistReturn)
        {
            let reservist_pos = crate::sim_rng::usize(
                sim,
                crate::sim_rng::RngSite::CampaignReservistReturn,
                ..self.reservist_indices.len(),
            );
            let char_idx = self.reservist_indices[reservist_pos];
            self.move_to_gang(char_idx, profiles);
            self.reservists_are_back = true;
            return char_idx;
        }

        // Collect non-VIP character profile indices as candidates
        let candidates: Vec<CharacterProfileIdx> = profiles
            .characters
            .iter()
            .filter(|cp| !cp.vip)
            .map(|cp| CharacterProfileIdx(cp.index))
            .collect();

        assert!(!candidates.is_empty(), "No peasant profiles found");

        let chosen = match peasant_type {
            Some(t) if (t as usize) < candidates.len() => t as usize,
            _ => crate::sim_rng::usize(
                sim,
                crate::sim_rng::RngSite::CampaignNewPeasantType,
                ..candidates.len(),
            ),
        };

        let profile_idx = candidates[chosen];
        // Peasants join empty-handed (no full pockets), but the human
        // status seeds fighting/shooting from the profile so the new
        // recruit carries their base combat skills.
        let cp = profiles
            .get_character(profile_idx)
            .expect("add_new_peasant_to_gang: candidate profile missing");
        let desc = PcDescription {
            character_profile_idx: Some(profile_idx),
            instanced: false,
            status: PcStatus::from_profile(cp, false, sim.config().difficulty),
        };
        let char_idx = self.add_to_characters(desc, profiles);
        self.add_to_gang(char_idx, profiles);
        char_idx
    }

    /// Post-mission peasant recruitment based on the "warcrime" ratio.
    ///
    /// Counts how many enemies the player left alive, applies a
    /// difficulty modifier, and recruits new peasants accordingly.
    ///
    /// Returns the number of new peasants added.
    pub fn recruit_post_mission_peasants(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        living_soldiers: u32,
        dead_soldiers: u32,
        difficulty: DifficultyLevel,
        profiles: &ProfileManager,
    ) -> u32 {
        let idx = self
            .current_mission_idx
            .expect("recruit_post_mission_peasants: no current mission");
        let profile = self.missions[idx].profile(profiles);
        let min_team = profile.min_new_team_members;
        let max_team = profile.max_new_team_members;

        let count = calculate_warcrime_recruitment(
            living_soldiers,
            dead_soldiers,
            difficulty,
            min_team,
            max_team,
        );

        self.set_reservists_back(false);
        for _ in 0..count {
            self.add_new_peasant_to_gang(sim, None, profiles);
        }
        // The caller is responsible for writing the new-peasant count
        // into the mission stats — we keep mission-stat mutation out
        // of this function.

        count
    }

    /// Consume blazons after a won mission.
    ///
    /// When the current mission is an ATTACK or TACTICAL mission and
    /// enough blazons have been collected, zeros the blazon campaign
    /// value. For TACTICAL missions, also marks the associated blazon
    /// (PSEUDO) mission as won.
    pub fn consume_blazons_post_mission(&mut self, profiles: &ProfileManager) {
        let blazon_idx = match self.blazon_mission_idx {
            Some(idx) => idx,
            None => return,
        };
        let current_idx = match self.current_mission_idx {
            Some(idx) => idx,
            None => return,
        };

        let current_type = self.missions[current_idx].profile(profiles).mission_type;
        let required = self.missions[blazon_idx]
            .profile(profiles)
            .number_of_blazons_to_win;
        let current_blazons = self.get_value(CampaignValue::Blazon);

        if current_blazons < required as i32 {
            return;
        }
        match current_type {
            MissionType::Attack => {
                self.set_value(CampaignValue::Blazon, 0);
            }
            MissionType::Tactical => {
                // Auto-complete the associated blazon (PSEUDO/ATTACK)
                // mission: zero blazons, mark the blazon mission done,
                // and drop it from the accessible list so the won
                // PSEUDO doesn't linger as a live candidate.
                self.set_value(CampaignValue::Blazon, 0);
                self.set_mission_done(true, Some(blazon_idx), profiles);
                self.remove_accessible_mission(blazon_idx);
            }
            _ => {}
        }
    }

    /// Auto-complete the PSEUDO blazon mission when enough blazons
    /// have been collected in Sherwood:
    ///
    /// 1. if `BLAZON_VALUE >= blazon_mission.to_win`, subtract the
    ///    win-requirement from the campaign value;
    /// 2. mark the blazon mission done (won);
    /// 3. remove it from the accessible list;
    /// 4. clear `next_mission_idx`;
    /// 5. re-derive accessible missions.
    ///
    /// Returns `true` when the cascade actually fired (caller should
    /// close any buy-blazon / mission-description windows and refresh
    /// the campaign map).
    pub fn try_consume_blazons_for_pseudo_in_sherwood(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        profiles: &ProfileManager,
    ) -> bool {
        let blazon_idx = match self.blazon_mission_idx {
            Some(idx) => idx,
            None => return false,
        };
        let profile = self.missions[blazon_idx].profile(profiles);
        if profile.mission_type != crate::profiles::MissionType::Pseudo {
            return false;
        }
        let collectable = profile.number_of_blazons_to_win as i32;
        let current = self.get_value(CampaignValue::Blazon);
        if current < collectable {
            return false;
        }
        self.add_value(CampaignValue::Blazon, -collectable);
        self.set_mission_done(true, Some(blazon_idx), profiles);
        self.remove_accessible_mission(blazon_idx);
        self.next_mission_idx = None;
        self.determine_accessible_missions(sim, profiles);
        true
    }

    /// Commit a blazon purchase: deduct ransom, bump blazon count,
    /// inflate the mission's next price, then run the Sherwood
    /// pseudo-mission consume cascade.  Runs on the campaign-map menu
    /// (no mission loaded), so engine-level side effects of
    /// `add_value` (CashWon jingle, mission_stat updates) are moot:
    /// Ransom's delta is negative and Blazon has no arm.
    ///
    /// Returns `true` when the cascade closed the buy screen — the
    /// caller uses that to short-circuit its post-buy update.
    pub fn buy_blazon(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        mission_index: usize,
        profiles: &ProfileManager,
    ) -> bool {
        let price = self.missions[mission_index].get_blazon_price() as i32;
        self.add_value(CampaignValue::Ransom, -price);
        self.add_value(CampaignValue::Blazon, 1);
        self.missions[mission_index].increase_blazon_price(profiles);
        self.try_consume_blazons_for_pseudo_in_sherwood(sim, profiles)
    }

    // ── Mission team / requirements ──

    /// Whether the currently-selected next mission's requirements
    /// are fulfilled by the current `mission_team_indices`.
    ///
    /// The team must be non-empty, every `required_character` must
    /// appear in the team, and every `required_action` must be
    /// available on *some* team member.  Returns `true` when all
    /// gates are satisfied (so the Sherwood `StartMission` widget
    /// should be enabled).
    pub fn mission_requirements_met(&self, profiles: &ProfileManager) -> bool {
        let Some(next_idx) = self.next_mission_idx else {
            return false;
        };
        let Some(mission) = self.missions.get(next_idx) else {
            return false;
        };
        if self.mission_team_indices.is_empty() {
            return false;
        }

        let profile = mission.profile(profiles);

        // Resolve the team's character profile indices once.
        let team_profile_ids: Vec<CharacterProfileIdx> = self
            .mission_team_indices
            .iter()
            .filter_map(|&char_idx| self.characters.get(char_idx))
            .filter_map(|desc| desc.character_profile_idx)
            .collect();

        // Required characters: every entry must appear in the team's
        // profile-id list.  Duplicate peasants are allowed, so we
        // don't de-dup here.
        for &required in &profile.required_character_indices {
            if !team_profile_ids.contains(&CharacterProfileIdx(required)) {
                return false;
            }
        }

        // Required actions: at least one team member must have the
        // action available.
        for &action in &profile.required_actions {
            let fulfilled = team_profile_ids.iter().any(|&pid| {
                profiles
                    .get_character(pid)
                    .map(|cp| cp.has_action(action))
                    .unwrap_or(false)
            });
            if !fulfilled {
                return false;
            }
        }

        true
    }

    // ── Rescue PCs (Win-mission recruitment) ──

    /// Per-mission rescue-PC table applied at mission Win.
    ///
    /// Each rescued PC is added via `rescue_pc_by_profile_name`.
    ///
    /// Returns the number of PCs actually added to the gang.
    pub fn rescue_pcs_for_current_mission_win(
        &mut self,
        profiles: &ProfileManager,
        difficulty: DifficultyLevel,
    ) -> usize {
        let idx = match self.current_mission_idx {
            Some(i) => i,
            None => return 0,
        };
        let filename = self.missions[idx]
            .profile(profiles)
            .mission_filename
            .clone();
        let names: &[&str] = match filename.as_str() {
            "S01_Not_VL" => &["Stutely", "Paysan A", "Paysan B", "Paysan C"],
            "S02_Lei_MP" => &["Will Ecarlate"],
            "S03_FoB_MP" => &["Petit Jean"],
            "S04_Der_EC" => &["Frere Tuck"],
            "S05_Yrk_EC" => &["Lady Marianne"],
            "H07_Not_MK" => &["Paysan A", "Paysan B", "Paysan C"],
            _ => &[],
        };
        let mut added = 0;
        for name in names {
            if self.rescue_pc_by_profile_name(name, profiles, difficulty) {
                added += 1;
            }
        }
        added
    }

    // ── Sherwood helper ──

    /// The Sherwood mission is always the first mission (index 0).
    pub fn get_sherwood_mission_idx(&self) -> usize {
        0
    }

    // ── Last pseudo mission status ───────────────────────────────────

    pub fn get_last_pseudo_mission_status(&self) -> MissionStatus {
        self.last_pseudo_mission_status
    }

    pub fn reset_last_pseudo_mission_status(&mut self) {
        self.last_pseudo_mission_status = MissionStatus::Available;
    }

    // ═══════════════════════════════════════════════════════════════
    // Pure-state mission-management methods
    // ═══════════════════════════════════════════════════════════════

    // ── set_mission_done ─────────────────────────────────────────────

    /// Mark a mission as won or lost, update ARES state accordingly.
    /// If `mission_idx` is `None`, operates on `current_mission_idx`.
    pub fn set_mission_done(
        &mut self,
        won: bool,
        mission_idx: Option<usize>,
        profiles: &ProfileManager,
    ) {
        let idx = mission_idx
            .or(self.current_mission_idx)
            .expect("set_mission_done: no mission to update");

        let ares_state;
        if won {
            ares_state = self.missions[idx]
                .ares_state_override
                .unwrap_or_else(|| self.missions[idx].profile(profiles).ares_state_succeeded);
            self.missions[idx].win();
        } else {
            ares_state = self.missions[idx].profile(profiles).ares_state_lost;
            self.missions[idx].lose();

            // Lose all blazons if this was the blazon mission
            if self.blazon_mission_idx == Some(idx) {
                self.set_value(CampaignValue::Blazon, 0);
            }
        }
        self.set_ares_conditional(ares_state);

        // Special pseudo mission tracking
        if self.blazon_mission_idx == Some(idx)
            && self.missions[idx].profile(profiles).mission_type == MissionType::Pseudo
        {
            self.last_pseudo_mission_id = self.missions[idx].profile(profiles).id;
            self.last_pseudo_mission_status = self.missions[idx].status;
        }
    }

    // ── determine_next_mission ───────────────────────────────────────

    /// Pick the next mission to play. Returns the mission index.
    /// If `next_mission_idx` is already set (chosen on the Sherwood map),
    /// uses that; otherwise runs `determine_accessible_missions`.
    pub fn determine_next_mission(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        profiles: &ProfileManager,
    ) -> usize {
        let sherwood_idx = self.get_sherwood_mission_idx();

        if let Some(next_idx) = self.next_mission_idx {
            // Player already chose a mission on the Sherwood map
            self.last_mission_idx = self.current_mission_idx;
            self.current_mission_idx = Some(next_idx);
            self.next_mission_idx = None;
        } else {
            // Determine missions that can be played
            self.determine_accessible_missions(sim, profiles);

            // Win pseudo missions that require zero blazons
            let empty_pseudo_indices: Vec<usize> = self
                .accessible_mission_indices
                .iter()
                .filter(|&&idx| {
                    let p = self.missions[idx].profile(profiles);
                    p.mission_type == MissionType::Pseudo && p.number_of_blazons_to_win == 0
                })
                .copied()
                .collect();
            for idx in empty_pseudo_indices {
                self.set_mission_done(true, Some(idx), profiles);
            }

            if self.accessible_mission_indices.len() == 1
                && !self.missions[self.accessible_mission_indices[0]]
                    .profile(profiles)
                    .pass_through_hq
            {
                // Only one mission accessible & it doesn't pass through Sherwood
                self.add_all_to_mission_team();
                self.last_mission_idx = self.current_mission_idx;
                self.current_mission_idx = Some(self.accessible_mission_indices[0]);
            } else {
                // Go to Sherwood and choose a mission
                self.add_all_to_mission_team();
                self.last_mission_idx = self.current_mission_idx;
                self.current_mission_idx = Some(sherwood_idx);
            }
        }

        let current = self.current_mission_idx.unwrap();

        // Remove the chosen mission from the accessible list
        if current != sherwood_idx {
            self.remove_accessible_mission(current);
        }

        // Log the launched mission
        {
            let cur_name = &self.missions[current].profile(profiles).mission_name;
            let cur_file = &self.missions[current].profile(profiles).mission_filename;
            let blazon_info = match self.blazon_mission_idx {
                Some(bi) => format!(
                    "{} ({})",
                    self.missions[bi].profile(profiles).mission_name,
                    self.missions[bi].profile(profiles).mission_filename,
                ),
                None => "NONE".to_string(),
            };
            tracing::info!(
                "Mission \"{}\" ({}) launched! (blazons: {})",
                cur_name,
                cur_file,
                blazon_info
            );
        }

        // Memorize last 3 played missions
        if current != sherwood_idx {
            self.last_played_mission_indices.push(current);
            if self.last_played_mission_indices.len() > 3 {
                self.last_played_mission_indices.remove(0);
            }
        }

        current
    }

    // ── select_next_mission ──────────────────────────────────────────

    /// Select a mission from the accessible list as the next to play.
    /// Non-selected missions get their age increased; if they exceed
    /// lifetime, ARES refused state is set.
    pub fn select_next_mission(&mut self, mission_idx: Option<usize>, profiles: &ProfileManager) {
        let target_idx = mission_idx
            .or(self.next_mission_idx)
            .expect("select_next_mission: no mission specified");

        debug_assert!(
            self.accessible_mission_indices.contains(&target_idx),
            "select_next_mission: mission not in accessible list"
        );

        let indices = self.accessible_mission_indices.clone();
        let mut removal_pos = None;
        for (i, &idx) in indices.iter().enumerate() {
            if idx != target_idx {
                self.missions[idx].increase_age(profiles);

                // NOTE: The original checks the *selected* mission's age
                // against its lifetime, which appears to be a bug — it
                // should likely check the current iteration's mission.
                // We replicate the original behavior exactly.
                let selected_age = self.missions[target_idx].get_age();
                let selected_lifetime = self.missions[target_idx].profile(profiles).life_time;
                if selected_age > selected_lifetime {
                    let ares_refused = self.missions[target_idx]
                        .profile(profiles)
                        .ares_state_refused;
                    self.set_ares_conditional(ares_refused);
                }
            } else {
                removal_pos = Some(i);
            }
        }

        // Store as next mission and remove from accessible list
        self.next_mission_idx = Some(target_idx);
        self.missions[target_idx].reset_age();
        if let Some(pos) = removal_pos {
            self.accessible_mission_indices.remove(pos);
        }
    }

    // ── force_next_mission ───────────────────────────────────────────

    /// Force a specific mission (by index) as the next mission.
    pub fn force_next_mission(&mut self, mission_idx: usize) {
        self.next_mission_idx = Some(mission_idx);
        self.add_all_to_mission_team();
    }

    /// Force a mission by filename.
    ///
    /// When `add_forced` is true and no existing mission matches, a new
    /// `MissionProfile` is appended via
    /// [`ProfileManager::add_forced_mission`] and a fresh [`Mission`]
    /// pointing at it is pushed onto `self.missions`. This is the CLI
    /// path for playing an arbitrary `.rhm` that isn't in the campaign
    /// descriptor.
    ///
    /// When `add_forced` is false and the mission isn't found,
    /// `next_mission_idx` is left unchanged.
    /// Returns the mission index that was selected, if any.
    pub fn force_next_mission_by_name(
        &mut self,
        profiles: &mut ProfileManager,
        mission_filename: &str,
        proto_level_filename: &str,
        add_forced: bool,
    ) -> Option<usize> {
        let found = self.missions.iter().enumerate().find_map(|(i, m)| {
            let pi = m.profile_idx?;
            let p = &profiles.missions[pi as usize];
            (p.proto_level_filename
                .eq_ignore_ascii_case(proto_level_filename)
                && p.mission_filename.eq_ignore_ascii_case(mission_filename))
            .then_some(i)
        });

        if let Some(i) = found {
            self.next_mission_idx = Some(i);
            self.add_all_to_mission_team();
            return Some(i);
        }

        if add_forced {
            // Construct a new Mission from a freshly-allocated profile
            // in the profile manager.  NOTE: callers must hold a
            // `&mut ProfileManager` to mutate the manager — currently
            // zero callers; kept for parity with the CLI flow.
            let profile_idx = profiles.add_forced_mission(
                proto_level_filename.to_string(),
                mission_filename.to_string(),
                mission_filename.to_string(),
            );
            let mut m = Mission::new();
            m.profile_idx = Some(profile_idx);
            m.blazon_price = profiles.missions[profile_idx as usize].blazon_price;
            let i = self.missions.len();
            self.missions.push(m);
            self.next_mission_idx = Some(i);
            self.add_all_to_mission_team();
            return Some(i);
        }

        None
    }

    // ── remove_accessible_mission ────────────────────────────────────

    /// Remove a mission from the accessible list and reset its age.
    pub fn remove_accessible_mission(&mut self, mission_idx: usize) {
        if let Some(pos) = self
            .accessible_mission_indices
            .iter()
            .position(|&i| i == mission_idx)
        {
            self.missions[mission_idx].reset_age();
            self.accessible_mission_indices.remove(pos);
        }
    }

    // ── swap_pending_to_accessible_missions ──────────────────────────

    /// Move all pending accessible missions into the main accessible list.
    pub fn swap_pending_to_accessible_missions(&mut self) {
        self.accessible_mission_indices
            .extend_from_slice(&self.pending_accessible_mission_indices);
        // Reset age for all pending missions before clearing
        for &idx in &self.pending_accessible_mission_indices {
            self.missions[idx].reset_age();
        }
        self.pending_accessible_mission_indices.clear();
    }

    // ── clear_accessible_missions ────────────────────────────────────

    /// Reset age of all missions in the accessible list and clear it.
    pub fn clear_accessible_missions(&mut self) {
        for &idx in &self.accessible_mission_indices {
            self.missions[idx].reset_age();
        }
        self.accessible_mission_indices.clear();
    }

    // ── is_mission_team_valid ────────────────────────────────────────

    /// Check that the mission team meets the next mission's requirements
    /// (required characters and required actions).
    pub fn is_mission_team_valid(&self, profiles: &ProfileManager) -> bool {
        let next_idx = self
            .next_mission_idx
            .expect("is_mission_team_valid: no next mission");
        let p = self.missions[next_idx].profile(profiles);

        // Check required characters
        for &char_profile_idx in &p.required_character_indices {
            if !self.find_character(
                CharacterProfileIdx(char_profile_idx),
                &self.mission_team_indices,
            ) {
                return false;
            }
        }

        // Check required actions
        for &action in &p.required_actions {
            if !self.find_action(action, &self.mission_team_indices, profiles) {
                return false;
            }
        }

        true
    }

    // ── get_number_of_peasants_in_gang ───────────────────────────────

    /// Count non-VIP (peasant) characters in the gang.
    pub fn get_number_of_peasants_in_gang(&self, profiles: &ProfileManager) -> usize {
        self.gang_indices
            .iter()
            .filter(|&&gi| {
                if let Some(desc) = self.characters.get(gi)
                    && let Some(cpi) = desc.character_profile_idx
                    && let Some(cp) = profiles.get_character(cpi)
                {
                    return !cp.vip;
                }
                false
            })
            .count()
    }

    // ── get_number_of_peasants_to_convert_to_blazons ─────────────────

    /// How many peasants can be converted to blazons for the next mission.
    /// Returns the number of peasants (not blazons) to convert.
    pub fn get_number_of_peasants_to_convert_to_blazons(&self, profiles: &ProfileManager) -> u16 {
        let next_idx = self
            .next_mission_idx
            .expect("get_number_of_peasants_to_convert_to_blazons: no next mission");
        let p = self.missions[next_idx].profile(profiles);

        if p.peasant_to_blazon_quotation == 0 {
            tracing::error!("Peasant to blazon quotation is zero -- would cause division by zero!");
            return 0;
        }

        // Number of blazons that can be exchanged against the peasants
        let possible_blazons =
            (self.get_size_of_mission_team() as u16) / p.peasant_to_blazon_quotation;

        // Number of blazons still required.  The original computes
        //   `required = blazons_to_win - blazons_to_be_collected
        //               - current_blazons`
        // as naked u16 subtraction — when the player already holds
        // more blazons than the net requirement the value underflows
        // to a huge number, forcing the `possible < required` branch
        // and returning `possible * quotation` ("convert as many
        // peasants as the team allows").  Saturating arithmetic would
        // floor to 0 and silently return 0 ("convert nothing"), which
        // would flip the selected branch.  We reproduce the original
        // wrap by doing the subtractions in signed i32 and treating a
        // negative `required` as "larger than any possible", so the
        // "convert all" branch is selected exactly when the original
        // would.
        let current_blazons = self.get_value(CampaignValue::Blazon);
        let required_signed = (p.number_of_blazons_to_win as i32)
            - (p.number_of_blazons_to_be_collected as i32)
            - current_blazons;
        let possible_signed = possible_blazons as i32;

        // `required_signed < 0` ≡ u16 underflow ≡
        // "required > u16::MAX/2 ≫ possible"; branch selection
        // there always picks `possible`.
        if required_signed < 0 || possible_signed < required_signed {
            possible_blazons * p.peasant_to_blazon_quotation
        } else {
            (required_signed as u16) * p.peasant_to_blazon_quotation
        }
    }

    // ── get_max_number_of_blazons ────────────────────────────────────

    /// Maximum blazons the player is allowed to possess for the current
    /// mission context.
    pub fn get_max_number_of_blazons(&self, profiles: &ProfileManager) -> u16 {
        let current_idx = self.current_mission_idx;

        if let Some(bi) = self
            .blazon_mission_idx
            .filter(|&bi| Some(bi) != current_idx)
        {
            // Preparing a blazon mission: allowed = required - collectable inside
            let p = self.missions[bi].profile(profiles);
            p.number_of_blazons_to_win
                .saturating_sub(p.number_of_blazons_to_be_collected)
        } else if let Some(ci) = current_idx {
            // Inside a blazon mission
            self.missions[ci].profile(profiles).number_of_blazons_to_win
        } else {
            0
        }
    }

    // ── can_convert_merry_men_to_blazons ─────────────────────────────

    /// Can we convert merry men (peasants) into blazons for the given mission?
    pub fn can_convert_merry_men_to_blazons(
        &self,
        mission_idx: usize,
        profiles: &ProfileManager,
    ) -> bool {
        let p = self.missions[mission_idx].profile(profiles);
        let current_blazons = self.get_value(CampaignValue::Blazon);
        let needed =
            (p.number_of_blazons_to_win as i32) - (p.number_of_blazons_to_be_collected as i32);

        current_blazons < needed
            && self.get_number_of_peasants_in_gang(profiles)
                >= p.peasant_to_blazon_quotation as usize
    }

    // ── can_convert_mission_to_blazons ───────────────────────────────

    /// Can the player play another mission to earn blazons for this one?
    pub fn can_convert_mission_to_blazons(
        &self,
        mission_idx: usize,
        profiles: &ProfileManager,
    ) -> bool {
        let p = self.missions[mission_idx].profile(profiles);
        let m = &self.missions[mission_idx];
        let current_blazons = self.get_value(CampaignValue::Blazon);
        let needed =
            (p.number_of_blazons_to_win as i32) - (p.number_of_blazons_to_be_collected as i32);

        !self.pending_accessible_mission_indices.is_empty()
            && (u32::from(m.get_age()) + 1 < u32::from(p.life_time)
                || p.mission_type == MissionType::Pseudo)
            && current_blazons < needed
    }

    // ── can_convert_money_to_blazons ─────────────────────────────────

    /// Can the player buy blazons with money for the given mission?
    pub fn can_convert_money_to_blazons(
        &self,
        mission_idx: usize,
        profiles: &ProfileManager,
    ) -> bool {
        let p = self.missions[mission_idx].profile(profiles);
        let m = &self.missions[mission_idx];
        let current_blazons = self.get_value(CampaignValue::Blazon);
        let ransom = self.get_value(CampaignValue::Ransom);
        let needed =
            (p.number_of_blazons_to_win as i32) - (p.number_of_blazons_to_be_collected as i32);

        (m.get_blazon_price() as i32) <= ransom && current_blazons < needed
    }

    // ── find_character ───────────────────────────────────────────────

    /// Check if a character (by profile index) is present in a team.
    pub fn find_character(
        &self,
        character_profile_idx: CharacterProfileIdx,
        team: &[usize],
    ) -> bool {
        team.iter().any(|&ti| {
            self.characters
                .get(ti)
                .and_then(|c| c.character_profile_idx)
                == Some(character_profile_idx)
        })
    }

    // ── find_action ──────────────────────────────────────────────────

    /// Check if any character in the team has the specified action.
    /// Handles special equivalences: Hit <-> HitHard, Eat <-> Guzzle,
    /// LittleJohnCarry <-> FarmerCarry.
    pub fn find_action(
        &self,
        action: crate::profiles::Action,
        team: &[usize],
        profiles: &ProfileManager,
    ) -> bool {
        use crate::profiles::Action;

        for &ti in team {
            let desc = match self.characters.get(ti) {
                Some(d) => d,
                None => continue,
            };
            let cpi = match desc.character_profile_idx {
                Some(i) => i,
                None => continue,
            };
            let cp = match profiles.get_character(cpi) {
                Some(p) => p,
                None => continue,
            };

            // Check normal actions
            for &a in &cp.actions {
                if a == action {
                    return true;
                }
            }

            // Hit <-> HitHard equivalence
            if action == Action::Hit {
                for &a in &cp.actions {
                    if a == Action::HitHard {
                        return true;
                    }
                }
            } else if action == Action::Eat {
                for &a in &cp.actions {
                    if a == Action::Guzzle {
                        return true;
                    }
                }
            }

            // Check contextual actions
            for &a in &cp.contextual_actions {
                if a == action {
                    return true;
                }
            }

            // LittleJohnCarry <-> FarmerCarry equivalence
            if action == Action::LittleJohnCarry {
                for &a in &cp.contextual_actions {
                    if a == Action::FarmerCarry {
                        return true;
                    }
                }
            }
        }

        false
    }

    // ── log_report ───────────────────────────────────────────────────

    /// Log campaign state using tracing::info!
    pub fn log_report(&self, profiles: &ProfileManager) {
        tracing::info!("---------- Campaign ----------");
        tracing::info!("  Ransom: {}", self.get_value(CampaignValue::Ransom));
        tracing::info!("  Gang size: {}", self.get_size_of_gang());
        tracing::info!("  ARES: {}", self.get_ares());

        tracing::info!("---------- Missions ----------");
        for (i, m) in self.missions.iter().enumerate() {
            let p = m.profile(profiles);
            let state = match m.status {
                MissionStatus::Won => "WON",
                MissionStatus::Lost => "LOST",
                MissionStatus::Available => "AVAILABLE",
            };
            let (accessible, reason) = match m.is_accessible_why(self, profiles) {
                Ok(()) => ("ACCESSIBLE", ""),
                Err(why) => ("NOT ACCESSIBLE", why),
            };
            tracing::info!(
                "  Mission[{}] \"{}\" ({},{}): {} - {} ({})",
                i,
                p.mission_name,
                p.mission_filename,
                p.proto_level_filename,
                state,
                accessible,
                reason
            );
        }

        tracing::info!("---------- Robin's gang ----------");
        for (i, &gi) in self.gang_indices.iter().enumerate() {
            if let Some(desc) = self.characters.get(gi) {
                let profile_name = desc
                    .character_profile_idx
                    .and_then(|cpi| profiles.get_character(cpi))
                    .map(|cp| cp.profile_name.as_str())
                    .unwrap_or("unknown");
                // This is a developer trace; we deliberately log the raw
                // `name` rather than `display_name(menu_text)` since the
                // campaign struct doesn't carry a `MenuTextLookup` and
                // plumbing one through purely for a debug print isn't
                // worth the surgery.  A `name_override` from PROP_NAME
                // will show as the underlying `name` / empty string
                // here — debriefing/render paths still resolve
                // correctly.
                tracing::info!(
                    "  Gang member[{}]: \"{}\" ({})",
                    i,
                    desc.status.name,
                    profile_name
                );
            }
        }
    }

    // ── reset ────────────────────────────────────────────────────────

    /// Reset the campaign to its initial state, recreating missions and
    /// gang from the loaded profiles.
    pub fn reset(&mut self, profiles: &ProfileManager, difficulty: DifficultyLevel) {
        self.last_mission_idx = None;
        self.current_mission_idx = None;
        self.next_mission_idx = None;
        self.blazon_mission_idx = None;
        self.ares = -1;
        self.last_pseudo_mission_status = MissionStatus::Available;
        self.pre_mission_snapshot = None;
        self.pre_mission_rng_seed = None;
        self.pre_mission_sim_config = None;
        self.pre_mission_was_preselected = false;

        // Reset values, set initial ransom
        self.values = enum_map! { _ => 0 };
        self.values[CampaignValue::Ransom] = INITIAL_RANSOM;

        // Recreate missions and gang from profiles

        self.missions.clear();
        for (i, mp) in profiles.missions.iter().enumerate() {
            let mut m = Mission::new();
            m.profile_idx = Some(i as u32);
            m.blazon_price = mp.blazon_price;
            self.missions.push(m);
        }

        self.characters.clear();
        self.gang_indices.clear();
        self.reservist_indices.clear();
        // Each character-pool entry is initialised with full pockets —
        // ammo is seeded from the profile's max-ammo values, then
        // difficulty-scaled. Only forest Robin is put into the initial
        // gang below, matching `RHCampaign::CreateGang`.
        let mut robin_char_idx: Option<usize> = None;
        for (i, cp) in profiles.characters.iter().enumerate() {
            self.characters.push(PcDescription {
                character_profile_idx: Some(CharacterProfileIdx(i as u32)),
                instanced: false,
                status: PcStatus::from_profile(cp, true, difficulty),
            });
            if robin_char_idx.is_none() && cp.profile_name == "Robin des bois" {
                robin_char_idx = Some(i);
            }
        }

        assert!(
            profiles.characters.len() > 1,
            "campaign reset: need at least two character profiles (Robin Town + Robin Hood)"
        );
        self.seed_initial_gang(robin_char_idx.unwrap_or(1));
        self.mission_team_indices.clear();
        self.mission_team_indices
            .extend_from_slice(&self.gang_indices);

        // Recreate the per-type production sector slots.
        self.production_sectors = default_production_sectors();

        self.accessible_mission_indices.clear();
        self.pending_accessible_mission_indices.clear();
        self.reservist_indices.clear();
        self.collected_relics.clear();
        self.peasant_names.clear();
        self.last_played_mission_indices.clear();
    }

    /// Rebuild the gang from a PC specification string (e.g. "RJMT").
    /// Used when the gang-string global option is set.
    /// Each character in the string maps to a profile name:
    ///   R=Robin, J=Petit Jean, T=Frere Tuck, S=Stutely,
    ///   W=Will Ecarlate, M=Lady Marianne, A/B/C=Paysan A/B/C,
    ///   F=Ferris.
    pub fn create_gang_from_pcs(
        &mut self,
        pcs: &str,
        profiles: &ProfileManager,
        difficulty: DifficultyLevel,
    ) {
        let char_names: Vec<&str> = pcs
            .chars()
            .filter_map(|c| match c.to_ascii_uppercase() {
                'R' => Some("Robin des bois"),
                'J' => Some("Petit Jean"),
                'T' => Some("Frere Tuck"),
                'S' => Some("Stutely"),
                'W' => Some("Will Ecarlate"),
                'M' => Some("Lady Marianne"),
                'A' => Some("Paysan A"),
                'B' => Some("Paysan B"),
                'C' => Some("Paysan C"),
                'F' => Some("Ferris"),
                _ => {
                    tracing::error!("Unknown PC code '{c}' in gang string");
                    None
                }
            })
            .collect();

        self.gang_indices.clear();
        self.characters.clear();

        // Each named gang member is initialised with full pockets;
        // ammo is difficulty-scaled.
        for name in &char_names {
            // Find the character profile index by name (case-sensitive).
            // If multiple profiles share the same name (e.g. forest/town Robin),
            // prefer the one whose RHS file actually exists on disk.
            let candidates: Vec<(usize, &crate::profiles::CharacterProfile)> = profiles
                .characters
                .iter()
                .enumerate()
                .filter(|(_, cp)| cp.profile_name == *name)
                .collect();
            let found = candidates
                .iter()
                .find(|(_, cp)| {
                    let path = format!("Data/Characters/{}.rhs", cp.filename);
                    crate::sbfile::SbFile::exists(&path)
                })
                .or_else(|| candidates.first())
                .map(|&(idx, cp)| (idx, cp));
            if let Some((idx, cp)) = found {
                self.characters.push(PcDescription {
                    character_profile_idx: Some(CharacterProfileIdx(idx as u32)),
                    instanced: false,
                    status: PcStatus::from_profile(cp, true, difficulty),
                });
                self.gang_indices.push(self.characters.len() - 1);
            } else {
                tracing::error!("Character profile '{}' not found for PC code", name);
            }
        }

        tracing::info!(
            "Created gang from PCs '{}': {} members",
            pcs,
            self.gang_indices.len()
        );
    }

    // ═══════════════════════════════════════════════════════════════
    // Mission selection algorithm
    // ═══════════════════════════════════════════════════════════════

    /// Determine which missions are accessible and select candidates.
    pub fn determine_accessible_missions(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        profiles: &ProfileManager,
    ) {
        // First mission is always Sherwood -- skip it (index 0)
        for i in 1..self.missions.len() {
            if self.missions[i].is_accessible(self, profiles)
                && !self.accessible_mission_indices.contains(&i)
            {
                self.accessible_mission_indices.push(i);
                self.missions[i].status = MissionStatus::Available;

                if let Some(idx) = self.missions[i].profile_idx
                    && let Some(p) = profiles.missions.get(idx as usize)
                {
                    tracing::info!(
                        "New accessible mission: {} ({})",
                        p.mission_name,
                        p.mission_filename
                    );
                }
            }
        }

        self.select_missions(sim, false, profiles);
        self.select_missions(sim, true, profiles);
    }

    /// Run the multi-pass mission selection pipeline.
    fn select_missions(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        pending: bool,
        profiles: &ProfileManager,
    ) {
        let mut attempts_left = 10u32;

        while attempts_left > 0 {
            let mut candidates: Vec<usize> = if pending {
                if self.pending_accessible_mission_indices.is_empty() {
                    return;
                }
                self.pending_accessible_mission_indices.clone()
            } else {
                self.accessible_mission_indices.clone()
            };

            if candidates.len() > 1 {
                self.filter_by_age(&mut candidates, profiles);
            }
            if candidates.len() > 1 {
                self.filter_by_obligation(&mut candidates, profiles);
            }

            if !pending {
                if candidates.len() > 1 {
                    self.filter_blazon_by_chance(sim, &mut candidates, profiles);
                }
                self.filter_by_blazons(&mut candidates, profiles);
            }

            if candidates.len() > 1 {
                self.filter_by_repetition(&mut candidates);
            }
            if candidates.len() > 1 {
                self.filter_non_blazon_by_chance(sim, &mut candidates, profiles);
            }
            if candidates.len() > 1 {
                self.filter_by_location(&mut candidates, profiles);
            }

            if !candidates.is_empty() {
                if pending {
                    self.pending_accessible_mission_indices = candidates;
                } else {
                    self.accessible_mission_indices = candidates;
                }
                return;
            }

            attempts_left -= 1;
        }

        // Failed after 10 attempts -- force a random pick.
        tracing::warn!("Campaign: unable to determine accessible mission, forcing");

        // Re-seed fallback from the current accessible/pending list
        // and, for the non-pending path, run the blazon filter on it
        // so the random pick can't land on a mission whose blazon
        // requirement isn't satisfied.
        let mut fallback: Vec<usize> = if pending {
            self.pending_accessible_mission_indices.clone()
        } else {
            self.accessible_mission_indices.clone()
        };
        if !pending {
            self.filter_by_blazons(&mut fallback, profiles);
        }

        if fallback.len() > 1 {
            let pick = fallback[crate::sim_rng::usize(
                sim,
                crate::sim_rng::RngSite::CampaignForcedMission,
                0..fallback.len(),
            )];
            if pending {
                self.pending_accessible_mission_indices = vec![pick];
            } else {
                // Reset age on every non-picked accessible mission so
                // their age-based filters restart cleanly next tick.
                let others: Vec<usize> = self.accessible_mission_indices.clone();
                for idx in others {
                    if idx != pick {
                        self.missions[idx].reset_age();
                    }
                }
                self.accessible_mission_indices = vec![pick];
            }
            return;
        }

        // Even when the forced pick degrades to size ≤ 1, always
        // write the (possibly-empty or single-element) fallback back
        // to the accessible / pending list.
        if pending {
            self.pending_accessible_mission_indices = fallback;
        } else {
            self.accessible_mission_indices = fallback;
        }
    }

    /// Remove missions that have exceeded their lifetime.
    /// Historical missions at lifetime-1 become the only option.
    fn filter_by_age(&mut self, candidates: &mut Vec<usize>, profiles: &ProfileManager) {
        // Check for historical mission at last chance.
        // The original uses `age == life_time - 1` on unsigned 16-bit
        // values, which underflows to u16::MAX when `life_time == 0`
        // — meaning the predicate essentially never fires for a
        // 0-lifetime profile. `saturating_sub` would incorrectly fire
        // on `age == 0` for `life_time == 0`, so guard on
        // `life_time > 0` to preserve the original behaviour.
        for &idx in candidates.iter() {
            let m = &self.missions[idx];
            let p = m.profile(profiles);
            if p.mission_type == MissionType::Historical
                && p.life_time > 0
                && m.age == p.life_time - 1
            {
                tracing::info!("Last chance for historical mission: {}", p.mission_name);
                for &other in candidates.iter() {
                    if other != idx {
                        self.missions[other].age = 0;
                    }
                }
                *candidates = vec![idx];
                return;
            }
        }

        // Collect expired indices so we can mutate both `candidates` and
        // `self.missions` (via `set_mission_done`) without aliasing.
        let expired: Vec<usize> = candidates
            .iter()
            .copied()
            .filter(|&idx| {
                let m = &self.missions[idx];
                m.age >= m.profile(profiles).life_time
            })
            .collect();

        for idx in expired {
            let (should_set_done, mission_name) = {
                let m = &self.missions[idx];
                let p = m.profile(profiles);
                let should = !m.is_done()
                    && matches!(p.mission_type, MissionType::Pseudo | MissionType::Attack);
                (should, p.mission_name.clone())
            };
            tracing::info!("Mission expired (age): {}", mission_name);
            // Reset age on the expired mission before removing it from
            // candidates: the mission still lives in `self.missions`
            // and could be re-added later via
            // `determine_accessible_missions`, so clearing the
            // accrued age keeps future eligibility correct.
            self.missions[idx].reset_age();
            candidates.retain(|&i| i != idx);
            // Expired PSEUDO/ATTACK missions that weren't done get
            // marked lost — advances ARES to the lost state and wipes
            // the blazon value if this was the blazon mission.
            if should_set_done {
                self.set_mission_done(false, Some(idx), profiles);
            }
        }
    }

    /// Keep only obligatory mission if one exists.
    fn filter_by_obligation(&mut self, candidates: &mut Vec<usize>, profiles: &ProfileManager) {
        // Sort by obligation first.  Sorting doesn't change the
        // keep-set in the single-obligatory case, but it matters if
        // two obligatory entries exist because the diagnostic message
        // names `missions[0]` / `missions[1]` of the sorted list.
        candidates
            .sort_by(|&a, &b| self.missions[a].cmp_by_obligation(&self.missions[b], profiles));

        if candidates.len() < 2 {
            return;
        }
        let first = candidates[0];
        let second = candidates[1];
        if !self.missions[first].profile(profiles).obligatory {
            return;
        }

        // Two obligatory missions coexisting is a profile-data bug;
        // respect the no-fake-data rule and panic rather than
        // silently picking one.
        if self.missions[second].profile(profiles).obligatory {
            panic!(
                "Campaign: more than one obligatory mission encountered ({} and {})",
                self.missions[first].profile(profiles).mission_name,
                self.missions[second].profile(profiles).mission_name,
            );
        }

        // Reset age on every non-obligatory mission before dropping it.
        for &idx in candidates.iter().skip(1) {
            tracing::info!(
                "Mission removed (obligatory): {}",
                self.missions[idx].profile(profiles).mission_name
            );
            self.missions[idx].reset_age();
        }
        candidates.truncate(1);
    }

    /// Random chance filter for blazon (attack/pseudo) missions.
    fn filter_blazon_by_chance(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        candidates: &mut Vec<usize>,
        profiles: &ProfileManager,
    ) {
        let backup = candidates.clone();
        candidates.retain(|&idx| {
            let m = &self.missions[idx];
            let p = m.profile(profiles);
            // Don't filter non-blazon missions or already-accessible ones
            if !m.requires_blazons(profiles) || m.age != 0 {
                return true;
            }
            let chance =
                crate::sim_rng::usize(sim, crate::sim_rng::RngSite::CampaignAccessChance, 0..101);
            if p.access_probability < chance as u16 {
                self.missions[idx].age = 0;
                false
            } else {
                true
            }
        });
        if candidates.is_empty() {
            *candidates = backup;
        }
    }

    /// Random chance filter for non-blazon missions.
    fn filter_non_blazon_by_chance(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        candidates: &mut Vec<usize>,
        profiles: &ProfileManager,
    ) {
        let backup = candidates.clone();
        candidates.retain(|&idx| {
            let m = &self.missions[idx];
            let p = m.profile(profiles);
            if m.requires_blazons(profiles) || m.age != 0 {
                return true;
            }
            let chance =
                crate::sim_rng::usize(sim, crate::sim_rng::RngSite::CampaignAccessChance, 0..101);
            if p.access_probability < chance as u16 {
                self.missions[idx].age = 0;
                false
            } else {
                true
            }
        });
        if candidates.is_empty() {
            *candidates = backup;
        }
    }

    /// Remove recently played missions to avoid repetition.
    fn filter_by_repetition(&mut self, candidates: &mut Vec<usize>) {
        for &last_idx in self.last_played_mission_indices.iter().rev() {
            if candidates.len() <= 1 {
                break;
            }
            if let Some(pos) = candidates.iter().position(|&c| c == last_idx) {
                self.missions[last_idx].age = 0;
                candidates.remove(pos);
            }
        }
    }

    /// For same-location missions, keep highest priority only.
    fn filter_by_location(&mut self, candidates: &mut Vec<usize>, profiles: &ProfileManager) {
        // Sort by location then priority
        candidates.sort_by(|&a, &b| {
            self.missions[a].cmp_by_location_and_priority(&self.missions[b], profiles)
        });

        let mut last_location: Option<u32> = None;
        let mut last_priority: u16 = 0xFFFF;
        let mut to_remove = Vec::new();

        for (i, &idx) in candidates.iter().enumerate() {
            let p = self.missions[idx].profile(profiles);
            let loc = p.location as u32;
            if last_location == Some(loc) {
                // Two candidates sharing both location and priority
                // is malformed profile data — panic rather than
                // silently dropping one.
                if p.priority == last_priority {
                    panic!(
                        "Campaign: two candidate missions share location and priority ({})",
                        p.mission_name,
                    );
                }
                // Same location as previous -- lower priority, remove
                to_remove.push(i);
            } else {
                last_location = Some(loc);
                last_priority = p.priority;
            }
        }

        for i in to_remove.into_iter().rev() {
            let idx = candidates[i];
            tracing::info!(
                "Mission removed (location): {}",
                self.missions[idx].profile(profiles).mission_name
            );
            // Reset age before erase — a dropped location loser keeps
            // accessibility for a future tick instead of aging out.
            self.missions[idx].reset_age();
            candidates.remove(i);
        }
    }

    /// Blazon mission selection -- at most one blazon mission at a time,
    /// others moved to pending.
    fn filter_by_blazons(&mut self, candidates: &mut Vec<usize>, profiles: &ProfileManager) {
        // The size==1 and size==0 arms still update
        // `blazon_mission_idx` — to the singleton if it requires
        // blazons, to None otherwise. We must keep it in sync on
        // every select_missions pass, otherwise callers
        // (consume_blazons_post_mission, can_convert_*) read a stale
        // value from the previous tick.
        if candidates.len() < 2 {
            self.blazon_mission_idx = match candidates.first() {
                Some(&idx) if self.missions[idx].requires_blazons(profiles) => Some(idx),
                _ => None,
            };
            return;
        }

        // Find first mission requiring blazons
        let blazon_idx = candidates
            .iter()
            .find(|&&idx| self.missions[idx].requires_blazons(profiles))
            .copied();
        self.blazon_mission_idx = blazon_idx;

        self.pending_accessible_mission_indices.clear();

        let mut keep = Vec::new();
        for &idx in candidates.iter() {
            let m = &self.missions[idx];
            if m.requires_blazons(profiles) && Some(idx) != blazon_idx {
                // Extra blazon mission -- remove
                self.missions[idx].age = 0;
            } else if m.produces_blazons(profiles) {
                if blazon_idx.is_none() {
                    // Produces blazons but nobody needs them -- remove
                    self.missions[idx].age = 0;
                } else if Some(idx) == blazon_idx {
                    keep.push(idx);
                } else {
                    // Move to pending
                    self.pending_accessible_mission_indices.push(idx);
                }
            } else {
                keep.push(idx);
            }
        }
        *candidates = keep;
    }

    /// Resolve whether the sector has its named specialist among occupants.
    /// Lives here because specialist name resolution needs the
    /// Campaign-owned `PcDescription` → `CharacterProfile` lookup.
    pub fn sector_has_specialist(
        &self,
        sector: &crate::sector_production::SectorProduction,
        profiles: &ProfileManager,
    ) -> bool {
        let expected = match crate::sherwood_stat::specialist_name_for_type(sector.prod_type) {
            Some(n) => n,
            None => return false,
        };
        for occ in &sector.occupants {
            let Some(desc) = self.characters.get(occ.pc_description_idx) else {
                // "No fake data": occupants recorded on a sector must point
                // to live PcDescriptions — a missing one means the save is
                // inconsistent.
                panic!(
                    "production sector occupant index {} out of range (campaign has {} characters)",
                    occ.pc_description_idx,
                    self.characters.len()
                );
            };
            let Some(profile_idx) = desc.character_profile_idx else {
                continue;
            };
            let Some(profile) = profiles.get_character(profile_idx) else {
                continue;
            };
            // Case-insensitive comparison against the profile name.
            if profile.profile_name.eq_ignore_ascii_case(expected) {
                return true;
            }
        }
        false
    }
}

// ─── Construction ────────────────────────────────────────────────

impl Campaign {
    /// Create a campaign from profiles loaded from disk.
    pub fn create(profiles: &ProfileManager, difficulty: DifficultyLevel) -> Campaign {
        let campaign = Campaign::from_profiles(profiles, difficulty);
        tracing::info!(
            "Rust campaign: {} missions, {} characters, {} in gang",
            campaign.missions.len(),
            campaign.characters.len(),
            campaign.gang_indices.len()
        );
        campaign
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_manager_with_robin_party() -> ProfileManager {
        let mut profiles = ProfileManager::new();
        for (idx, name) in [
            "Robin des villes",
            "Robin des bois",
            "Petit Jean",
            "Frere Tuck",
        ]
        .into_iter()
        .enumerate()
        {
            profiles.characters.push(crate::profiles::CharacterProfile {
                index: idx as u32,
                profile_name: name.to_string(),
                ..Default::default()
            });
        }
        profiles
    }

    #[test]
    fn campaign_values() {
        let mut c = Campaign::new();
        assert_eq!(c.get_value(CampaignValue::Amulets), 0);
        c.set_value(CampaignValue::Amulets, 100);
        assert_eq!(c.get_value(CampaignValue::Amulets), 100);
        c.add_value(CampaignValue::Amulets, 50);
        assert_eq!(c.get_value(CampaignValue::Amulets), 150);
        c.subtract_value(CampaignValue::Amulets, 30);
        assert_eq!(c.get_value(CampaignValue::Amulets), 120);
    }

    #[test]
    fn simulation_snapshot_records_explicit_preselection_mode() {
        let config = crate::engine::SimConfig::default();
        let mut campaign = Campaign::default();
        campaign.pre_mission_was_preselected = true;

        campaign.snapshot_with_simulation(11, config);
        assert!(!campaign.pre_mission_was_preselected);
        assert!(
            !campaign
                .pre_mission_snapshot
                .as_ref()
                .unwrap()
                .pre_mission_was_preselected
        );

        campaign.snapshot_preselected_with_simulation(22, config);
        assert!(campaign.pre_mission_was_preselected);
        assert!(
            campaign
                .pre_mission_snapshot
                .as_ref()
                .unwrap()
                .pre_mission_was_preselected
        );
    }

    #[test]
    fn campaign_gang() {
        let mut c = Campaign::new();
        c.characters.push(PcDescription {
            character_profile_idx: Some(CharacterProfileIdx(0)),
            instanced: false,
            ..Default::default()
        });
        c.characters.push(PcDescription {
            character_profile_idx: Some(CharacterProfileIdx(1)),
            instanced: false,
            ..Default::default()
        });
        let profiles = crate::profiles::ProfileManager::new();
        c.add_to_gang(0, &profiles);
        c.add_to_gang(1, &profiles);
        assert_eq!(c.get_size_of_gang(), 2);
        assert!(c.is_in_gang(CharacterProfileIdx(0)));
        c.move_to_reservists(0);
        assert_eq!(c.get_size_of_gang(), 1);
        assert!(!c.is_in_gang(CharacterProfileIdx(0)));
        c.move_to_gang(0, &profiles);
        assert_eq!(c.get_size_of_gang(), 2);
    }

    #[test]
    fn campaign_from_profiles_seeds_only_forest_robin() {
        let profiles = profile_manager_with_robin_party();
        let c = Campaign::from_profiles(&profiles, DifficultyLevel::Medium);

        assert_eq!(c.gang_indices, vec![1]);
        assert_eq!(
            profiles.characters[c.gang_indices[0]].profile_name,
            "Robin des bois"
        );
        assert!(c.mission_team_indices.is_empty());
    }

    #[test]
    fn campaign_reset_seeds_only_forest_robin_team() {
        let profiles = profile_manager_with_robin_party();
        let mut c = Campaign::from_profiles(&profiles, DifficultyLevel::Medium);
        c.gang_indices = vec![0, 1, 2, 3];
        c.mission_team_indices = vec![0, 1, 2, 3];

        c.reset(&profiles, DifficultyLevel::Medium);

        assert_eq!(c.gang_indices, vec![1]);
        assert_eq!(c.mission_team_indices, vec![1]);
        assert_eq!(
            profiles.characters[c.gang_indices[0]].profile_name,
            "Robin des bois"
        );
    }

    #[test]
    fn campaign_mission_team() {
        let mut c = Campaign::new();
        c.add_to_mission_team(0);
        c.add_to_mission_team(1);
        assert_eq!(c.get_size_of_mission_team(), 2);
        assert!(c.is_in_mission_team(0));
        c.remove_from_mission_team(0);
        assert_eq!(c.get_size_of_mission_team(), 1);
        c.reset_mission_team();
        assert_eq!(c.get_size_of_mission_team(), 0);
    }

    #[test]
    fn campaign_serde_round_trip() {
        let mut c = Campaign::new();
        c.set_value(CampaignValue::Amulets, 42);
        c.ares = 3;
        c.peasant_names.push("Bob".into());
        c.collected_relics.push(7);

        let json = serde_json::to_string(&c).unwrap();
        let c2: Campaign = serde_json::from_str(&json).unwrap();

        assert_eq!(c2.get_value(CampaignValue::Amulets), 42);
        assert_eq!(c2.ares, 3);
        assert_eq!(c2.peasant_names, vec!["Bob"]);
        assert_eq!(c2.collected_relics, vec![7]);
    }

    #[test]
    fn campaign_set_ares_conditional() {
        let mut c = Campaign::new();
        c.set_ares(2);
        assert_eq!(c.get_ares(), 2);
        // -1 means "no change"
        c.set_ares_conditional(-1);
        assert_eq!(c.get_ares(), 2);
        // Any other value updates
        c.set_ares_conditional(5);
        assert_eq!(c.get_ares(), 5);
    }

    #[test]
    fn campaign_add_all_to_mission_team() {
        let mut c = Campaign::new();
        c.characters.push(PcDescription::default());
        c.characters.push(PcDescription::default());
        c.gang_indices = vec![0, 1];
        c.add_all_to_mission_team();
        assert_eq!(c.mission_team_indices, vec![0, 1]);
        // Adding a third gang member and re-calling replaces the team
        c.gang_indices.push(2);
        c.add_all_to_mission_team();
        assert_eq!(c.mission_team_indices, vec![0, 1, 2]);
    }

    #[test]
    fn campaign_find_character() {
        let mut c = Campaign::new();
        c.characters.push(PcDescription {
            character_profile_idx: Some(CharacterProfileIdx(10)),
            ..Default::default()
        });
        c.characters.push(PcDescription {
            character_profile_idx: Some(CharacterProfileIdx(20)),
            ..Default::default()
        });
        let team = vec![0, 1];
        assert!(c.find_character(CharacterProfileIdx(10), &team));
        assert!(c.find_character(CharacterProfileIdx(20), &team));
        assert!(!c.find_character(CharacterProfileIdx(30), &team));
        assert!(!c.find_character(CharacterProfileIdx(10), &[]));
    }

    #[test]
    fn campaign_last_pseudo_mission_status() {
        let mut c = Campaign::new();
        assert_eq!(c.get_last_pseudo_mission_status(), MissionStatus::Available);
        c.last_pseudo_mission_status = MissionStatus::Won;
        assert_eq!(c.get_last_pseudo_mission_status(), MissionStatus::Won);
        c.reset_last_pseudo_mission_status();
        assert_eq!(c.get_last_pseudo_mission_status(), MissionStatus::Available);
    }

    #[test]
    fn campaign_sherwood_idx() {
        let c = Campaign::new();
        assert_eq!(c.get_sherwood_mission_idx(), 0);
    }

    #[test]
    fn campaign_accessible_mission_management() {
        let mut c = Campaign::new();
        // Create 3 dummy missions
        for _ in 0..3 {
            c.missions.push(Mission::new());
        }
        c.accessible_mission_indices = vec![0, 1, 2];
        c.missions[1].age = 5;

        // Remove mission 1
        c.remove_accessible_mission(1);
        assert_eq!(c.accessible_mission_indices, vec![0, 2]);
        assert_eq!(c.missions[1].age, 0); // age reset

        // Clear all
        c.missions[0].age = 3;
        c.missions[2].age = 7;
        c.clear_accessible_missions();
        assert!(c.accessible_mission_indices.is_empty());
        assert_eq!(c.missions[0].age, 0);
        assert_eq!(c.missions[2].age, 0);
    }

    #[test]
    fn campaign_swap_pending_to_accessible() {
        let mut c = Campaign::new();
        for _ in 0..4 {
            c.missions.push(Mission::new());
        }
        c.accessible_mission_indices = vec![0, 1];
        c.pending_accessible_mission_indices = vec![2, 3];
        c.missions[2].age = 2;
        c.missions[3].age = 4;

        c.swap_pending_to_accessible_missions();
        assert_eq!(c.accessible_mission_indices, vec![0, 1, 2, 3]);
        assert!(c.pending_accessible_mission_indices.is_empty());
        assert_eq!(c.missions[2].age, 0);
        assert_eq!(c.missions[3].age, 0);
    }

    #[test]
    fn campaign_force_next_mission() {
        let mut c = Campaign::new();
        c.characters.push(PcDescription::default());
        c.gang_indices = vec![0];

        for _ in 0..3 {
            c.missions.push(Mission::new());
        }

        c.force_next_mission(2);
        assert_eq!(c.next_mission_idx, Some(2));
        assert_eq!(c.mission_team_indices, vec![0]); // gang added to team
    }

    // ── Warcrime / carnage recruitment tests ──

    #[test]
    fn warcrime_no_soldiers() {
        // No soldiers at all → warcrime = 0 → min peasants only
        let count = calculate_warcrime_recruitment(0, 0, DifficultyLevel::Medium, 2, 5);
        assert_eq!(count, 2);
    }

    #[test]
    fn warcrime_all_alive_medium() {
        // All soldiers alive → warcrime = 1.0 → (u16)1.0 = 1 → min + 1*(max-min) = max
        let count = calculate_warcrime_recruitment(10, 0, DifficultyLevel::Medium, 2, 5);
        assert_eq!(count, 5);
    }

    #[test]
    fn warcrime_all_dead_medium() {
        // All soldiers dead → warcrime = 0.0 → (u16)0.0 = 0 → min
        let count = calculate_warcrime_recruitment(0, 10, DifficultyLevel::Medium, 2, 5);
        assert_eq!(count, 2);
    }

    #[test]
    fn warcrime_half_alive_medium() {
        // Half alive → warcrime = 0.5 → (u16)0.5 = 0 → min
        // (truncation means fractional warcrime gives no bonus)
        let count = calculate_warcrime_recruitment(5, 5, DifficultyLevel::Medium, 2, 5);
        assert_eq!(count, 2);
    }

    #[test]
    fn warcrime_all_alive_easy() {
        // Easy: warcrime = 1.0 - 0.5*(1.0-1.0) = 1.0 → max
        let count = calculate_warcrime_recruitment(10, 0, DifficultyLevel::Easy, 2, 5);
        assert_eq!(count, 5);
    }

    #[test]
    fn warcrime_all_dead_easy() {
        // Easy: warcrime = 1.0 - 0.5*(1.0-0.0) = 0.5 → (u16)0.5 = 0 → min
        let count = calculate_warcrime_recruitment(0, 10, DifficultyLevel::Easy, 2, 5);
        assert_eq!(count, 2);
    }

    #[test]
    fn warcrime_all_alive_hard() {
        // Hard: warcrime = 1.0 - 2.0*(1.0-1.0) = 1.0 → max
        let count = calculate_warcrime_recruitment(10, 0, DifficultyLevel::Hard, 2, 5);
        assert_eq!(count, 5);
    }

    #[test]
    fn warcrime_all_dead_hard() {
        // Hard: warcrime = 1.0 - 2.0*(1.0-0.0) = -1.0 → clamped to 0 → min
        let count = calculate_warcrime_recruitment(0, 10, DifficultyLevel::Hard, 2, 5);
        assert_eq!(count, 2);
    }

    #[test]
    fn warcrime_half_alive_hard() {
        // Hard: warcrime = 1.0 - 2.0*(1.0-0.5) = 0.0 → min
        let count = calculate_warcrime_recruitment(5, 5, DifficultyLevel::Hard, 2, 5);
        assert_eq!(count, 2);
    }

    #[test]
    fn warcrime_min_equals_max() {
        // When min == max, always returns that value regardless of ratio
        let count = calculate_warcrime_recruitment(10, 0, DifficultyLevel::Medium, 3, 3);
        assert_eq!(count, 3);
    }
}
