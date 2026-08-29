//! Profile data types and manager for the Robin Hood game.
//!
//! Profiles are static game data describing characters, soldiers, civilians,
//! weapons, and missions — loaded from CSV at startup. The manager owns
//! vectors of profile structs and handles CSV loading and binary
//! serialization.

use num_enum::TryFromPrimitive;
use serde::{Deserialize, Serialize};

use crate::coordinates::{MoveBox, SpriteAnchor};
use crate::geo2d;
use crate::legacy_io::{LegacyIoError, LegacyReader, LegacyResult};
use crate::sbfile::SbFile;

// ─── Constants ───────────────────────────────────────────────────

pub const MAX_NUMBER_OF_PC: usize = 5;
pub const NUMBER_OF_PC_ACTIONS: usize = 3;
pub const NUMBER_OF_PC_CONTEXTUAL_ACTIONS: usize = 4;
pub const INVALID_PROFILE_ID: u32 = 0xFFFFFFFF;

// ─── Profile index newtypes ──────────────────────────────────────

/// Index into [`ProfileManager::characters`] (PC character profiles).
///
/// Plain `u32` wrapper (not `NonMaxU32`); the sentinel
/// [`INVALID_PROFILE_ID`] lives at the serialization boundary only.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct CharacterProfileIdx(pub u32);

impl From<CharacterProfileIdx> for u32 {
    #[inline]
    fn from(i: CharacterProfileIdx) -> u32 {
        i.0
    }
}
impl From<CharacterProfileIdx> for usize {
    #[inline]
    fn from(i: CharacterProfileIdx) -> usize {
        i.0 as usize
    }
}
impl From<u32> for CharacterProfileIdx {
    #[inline]
    fn from(v: u32) -> Self {
        Self(v)
    }
}
impl std::fmt::Display for CharacterProfileIdx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Index into [`ProfileManager::soldiers`].
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SoldierProfileIdx(pub u32);

impl From<SoldierProfileIdx> for u32 {
    #[inline]
    fn from(i: SoldierProfileIdx) -> u32 {
        i.0
    }
}
impl From<SoldierProfileIdx> for usize {
    #[inline]
    fn from(i: SoldierProfileIdx) -> usize {
        i.0 as usize
    }
}
impl From<u32> for SoldierProfileIdx {
    #[inline]
    fn from(v: u32) -> Self {
        Self(v)
    }
}
impl std::fmt::Display for SoldierProfileIdx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Index into [`ProfileManager::civilians`].
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct CivilianProfileIdx(pub u32);

impl From<CivilianProfileIdx> for u32 {
    #[inline]
    fn from(i: CivilianProfileIdx) -> u32 {
        i.0
    }
}
impl From<CivilianProfileIdx> for usize {
    #[inline]
    fn from(i: CivilianProfileIdx) -> usize {
        i.0 as usize
    }
}
impl From<u32> for CivilianProfileIdx {
    #[inline]
    fn from(v: u32) -> Self {
        Self(v)
    }
}
impl std::fmt::Display for CivilianProfileIdx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// ─── Enums ───────────────────────────────────────────────────────

/// Player character actions. Single source of truth — both static
/// profile data (CSV-loaded) and runtime `PcData::current_action` use
/// this enum. `#[repr(u32)]` matches the integer representation used
/// for script natives and profile serialization.
#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Default,
    Serialize,
    Deserialize,
    TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum Action {
    #[default]
    NoAction = 0,
    Bow,
    Hit,
    HitHard,
    Purse,
    Stone,
    Shield,
    BigShield,
    Strangle,
    Lever,
    HelpToClimb,
    Apple,
    Ale,
    Eat,
    Guzzle,
    Listen,
    Heal,
    Net,
    Beggar,
    WaspNest,
    Whistle,
    // Contextual actions
    Climb,
    Jump,
    Search,
    Resuscitate,
    LittleJohnCarry,
    FarmerCarry,
    Tie,
    Lockpick,
    Execute,
    Test,
}

impl Action {
    pub fn from_u32(v: u32) -> Self {
        Self::try_from(v).unwrap_or_else(|_| {
            tracing::warn!("invalid Action value {v}, clamping to NoAction");
            Action::NoAction
        })
    }
}

/// Script-level action codes used by native functions like `HasAnyPCAction`.
#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[allow(missing_docs)]
pub enum ScriptAction {
    Bow = 0,
    Hit = 1,
    HitHard = 2,
    Purse = 3,
    Stone = 4,
    Shield = 5,
    BigShield = 6,
    Strangle = 7,
    Lever = 8,
    HelpToClimb = 9,
    Apple = 10,
    Ale = 11,
    Eat = 12,
    Guzzle = 13,
    Listen = 14,
    Heal = 15,
    Net = 16,
    Beggar = 17,
    WaspNest = 18,
    Whistle = 19,
    Climb = 20,
    Jump = 21,
    // 22 = unused
    Search = 23,
    Resuscitate = 24,
    LittleJohnCarry = 25,
    FarmerCarry = 26,
    Tie = 27,
    Lockpick = 28,
    Execute = 29,
}

impl ScriptAction {
    /// Convert to the runtime `Action` enum.
    pub fn to_action(self) -> Action {
        match self {
            Self::Bow => Action::Bow,
            Self::Hit => Action::Hit,
            Self::HitHard => Action::HitHard,
            Self::Purse => Action::Purse,
            Self::Stone => Action::Stone,
            Self::Shield => Action::Shield,
            Self::BigShield => Action::BigShield,
            Self::Strangle => Action::Strangle,
            Self::Lever => Action::Lever,
            Self::HelpToClimb => Action::HelpToClimb,
            Self::Apple => Action::Apple,
            Self::Ale => Action::Ale,
            Self::Eat => Action::Eat,
            Self::Guzzle => Action::Guzzle,
            Self::Listen => Action::Listen,
            Self::Heal => Action::Heal,
            Self::Net => Action::Net,
            Self::Beggar => Action::Beggar,
            Self::WaspNest => Action::WaspNest,
            Self::Whistle => Action::Whistle,
            Self::Climb => Action::Climb,
            Self::Jump => Action::Jump,
            Self::Search => Action::Search,
            Self::Resuscitate => Action::Resuscitate,
            Self::LittleJohnCarry => Action::LittleJohnCarry,
            Self::FarmerCarry => Action::FarmerCarry,
            Self::Tie => Action::Tie,
            Self::Lockpick => Action::Lockpick,
            Self::Execute => Action::Execute,
        }
    }
}

#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum ProfileRank {
    #[default]
    Soldier = 0,
    Officer,
    Knight,
    None,
}

#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum WeaponMaterial {
    #[default]
    Wood = 0,
    Steel,
    CastIron,
    SteelAndWood,
}

#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum ArmorMaterial {
    #[default]
    Leather = 0,
    ChainMail,
    Plate,
}

#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum CivilianType {
    #[default]
    Man = 0,
    Woman,
    OldMan,
    Child,
    Beggar,
    Vip,
}

#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum Attitude {
    #[default]
    Hostile = 0,
    Friendly,
}

#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum WeaponTarget {
    #[default]
    Head = 0,
    Front,
    Left,
    Back,
    Right,
    None,
}

#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum WeaponThrustKind {
    #[default]
    Straight = 0,
    Lateral,
    PushAside,
    TrueHalfCircle,
    TrueCircle,
    FalseHalfCircle,
    FalseCircle,
    Assault,
}

#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum WeaponThrustDirection {
    #[default]
    LeftToRight = 0,
    RightToLeft,
    NonApplicable,
}

#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum MissionType {
    Historical = 0,
    Attack,
    Rescue,
    Ambush,
    Hq,
    Pseudo,
    Tactical,
    #[default]
    End,
}

impl MissionType {
    pub fn from_u32(v: u32) -> Self {
        Self::try_from(v).unwrap_or_else(|_| {
            tracing::warn!("invalid MissionType value {v}, clamping to End");
            MissionType::End
        })
    }
}

#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum MissionLocation {
    #[default]
    Nowhere = 0,
    Cross1,
    Cross2,
    Cross3,
    Derby,
    Leicester,
    Lincoln,
    Nottingham,
    Sherwood,
    York,
}

// ─── Profile Structs ─────────────────────────────────────────────

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ThrustProfile {
    pub target: WeaponTarget,
    pub kind: WeaponThrustKind,
    pub direction: WeaponThrustDirection,
    pub stunning: u16,
    pub cutting: u16,
    pub minimal_distance: u16,
    pub maximal_distance: u16,
    pub initial_angle: u16,
    pub final_angle: u16,
    pub rotation_angle: u16,
    pub repulsion: u16,
    pub stumble_probability: u16,
    pub energy: u16,
}

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct HtHWeaponProfile {
    pub distance: [u16; 4],
    pub protection_by_localization: [u16; 5],
    pub bludgeon_protection: u16,
    pub piercing_protection: u16,
    pub charge: bool,
    pub shield: bool,
    pub shield_width: u16,
    pub shield_height: u16,
    pub thrusts: [ThrustProfile; 10],
}

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct CharacterProfile {
    pub index: u32,
    pub filename: String,
    pub profile_name: String,
    /// Host-facing label for mod-added characters. Retail profiles leave this
    /// empty and continue to use their localized `CharacterKind` name.
    #[serde(default)]
    pub display_name: String,
    pub alternative_profile_name: String,
    pub valid_alternative_profile: bool,
    pub vip: bool,
    pub shooting: u16,
    pub fighting: u16,
    pub endurance: u16,
    pub exclamation_id: u32,
    pub hth_weapon_id: u32,
    pub shooting_weapon_id: u32,
    pub actions: [Action; NUMBER_OF_PC_ACTIONS],
    pub action_max_ammo: [u16; NUMBER_OF_PC_ACTIONS],
    pub contextual_actions: [Action; NUMBER_OF_PC_CONTEXTUAL_ACTIONS],
    pub pathfinder_index: u8,
    pub box_move: MoveBox,
    pub center: SpriteAnchor,
    pub priority: u16,
    pub wake_up: u16,
    pub detection_speed_in_city: u16,
    pub detection_speed_in_forest: u16,
    pub weapon_material: WeaponMaterial,
    pub armor_material: ArmorMaterial,
}

impl CharacterProfile {
    /// Returns true if this PC profile has the given contextual action.
    pub fn has_contextual_action(&self, action: Action) -> bool {
        self.contextual_actions.contains(&action)
    }

    /// Returns true if this PC profile has the given action in its main
    /// action slots.
    pub fn has_action(&self, action: Action) -> bool {
        self.actions.contains(&action)
    }

    /// Returns true if this PC can carry bodies (LittleJohn or Farmer carry).
    pub fn can_carry(&self) -> bool {
        self.has_action(Action::LittleJohnCarry)
            || self.has_action(Action::FarmerCarry)
            || self.has_contextual_action(Action::LittleJohnCarry)
            || self.has_contextual_action(Action::FarmerCarry)
    }
}

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SoldierProfile {
    pub filename: String,
    pub profile_name: String,
    pub display_name: String,
    pub life_point: u16,
    pub intelligence: u16,
    pub courage: u16,
    pub initiative: u16,
    pub pride: u16,
    pub formation: bool,
    pub shooting: u16,
    pub fighting: u16,
    pub endurance: u16,
    pub bee_time: u16,
    pub exclamation_id: u32,
    pub hth_weapon_id: u32,
    pub shooting_weapon_id: u32,
    pub beer: u16,
    pub apple: u16,
    pub money: u16,
    pub whistle: u16,
    pub rank: ProfileRank,
    pub hostile: bool,
    pub rider: bool,
    pub heavy: bool,
    pub vip: bool,
    pub duty: bool,
    pub strangle: bool,
    pub pathfinder_index: u8,
    pub box_move: MoveBox,
    pub center: SpriteAnchor,
    pub wake_up: u16,
    pub weapon_material: WeaponMaterial,
    pub armor_material: ArmorMaterial,
}

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct CivilianProfile {
    pub filename: String,
    pub profile_name: String,
    pub display_name: String,
    pub civilian_type: CivilianType,
    pub attitude: Attitude,
    pub exclamation_id: u32,
}

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MissionProfile {
    pub id: u32,
    pub proto_level_filename: String,
    pub mission_filename: String,
    pub mission_name: String,
    pub mission_type: MissionType,
    pub pass_through_hq: bool,
    pub location: MissionLocation,
    pub min_ransom: u32,
    pub max_ransom: u32,
    pub min_gang_size: u16,
    pub max_gang_size: u16,
    pub life_time: u16,
    pub access_probability: u16,
    pub priority: u16,
    pub length: u16,
    pub ares_sensible: bool,
    pub available_in_ares_state: [bool; 10],
    pub obligatory: bool,
    pub ares_state_succeeded: i8,
    pub ares_state_lost: i8,
    pub ares_state_refused: i8,
    pub min_new_team_members: u16,
    pub max_new_team_members: u16,
    pub number_of_blazons_to_win: u16,
    pub number_of_blazons_to_be_collected: u16,
    pub blazon_price: u16,
    pub blazon_inflation: u16,
    pub peasant_to_blazon_quotation: u16,
    pub number_of_beam_mes: u16,
    /// Indices into the character profile vector.
    pub required_character_indices: Vec<u32>,
    pub required_actions: Vec<Action>,
    pub missions_required_to_be_done: Vec<u32>,
    pub missions_required_not_to_be_done: Vec<u32>,
    pub map_resource_ids: Vec<u32>,
    pub green_music: String,
    pub yellow_music: String,
    pub red_music: String,
}

// ─── Bow Profile ─────────────────────────────────────────────────

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct BowHitChance {
    pub hit_chance: [u16; 6],
}

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct BowShootMode {
    pub range: u16,
    pub hit_chances: [BowHitChance; 3], // Beginner, Normal, Elite
    pub damage: u16,
}

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct BowProfile {
    pub normal_shoot: BowShootMode,
    pub has_long_shoot: bool,
    pub long_shoot: BowShootMode,
}

impl BowShootMode {
    fn read_legacy_cpf(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        let range = reader.read_u16("range")?;
        let mut hit_chances = std::array::from_fn(|_| BowHitChance::default());
        for i in 0..6 {
            for (skill_index, skill) in hit_chances.iter_mut().enumerate() {
                skill.hit_chance[i] =
                    reader.read_u16(format_args!("hit_chances[{skill_index}][{i}]"))?;
            }
        }
        let damage = reader.read_u16("damage")?;
        Ok(Self {
            range,
            hit_chances,
            damage,
        })
    }
}

impl BowProfile {
    fn read_legacy_cpf(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        Ok(Self {
            normal_shoot: reader.scope("normal_shoot", BowShootMode::read_legacy_cpf)?,
            has_long_shoot: reader.read_bool("has_long_shoot")?,
            long_shoot: reader.scope("long_shoot", BowShootMode::read_legacy_cpf)?,
        })
    }
}

// ─── Profile Manager ─────────────────────────────────────────────

/// Manages all profile data (characters, soldiers, civilians, weapons,
/// missions). Loaded from CSV files at startup, optionally cached as
/// compiled `.cpf` binary files.
#[derive(
    Debug,
    Default,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ProfileManager {
    pub characters: Vec<CharacterProfile>,
    pub soldiers: Vec<SoldierProfile>,
    pub hth_weapons: Vec<HtHWeaponProfile>,
    pub bows: Vec<BowProfile>,
    pub missions: Vec<MissionProfile>,
    pub civilians: Vec<CivilianProfile>,
}

impl ProfileManager {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Profile accessors ────────────────────────────────────────

    pub fn get_character(&self, id: impl Into<CharacterProfileIdx>) -> Option<&CharacterProfile> {
        self.characters.get(usize::from(id.into()))
    }

    /// Index variant of [`character_by_name`] returning a typed
    /// [`CharacterProfileIdx`]. Same exact-match `profile_name` compare.
    pub fn character_idx_by_name(&self, name: &str) -> Option<CharacterProfileIdx> {
        self.characters
            .iter()
            .position(|cp| cp.profile_name == name)
            .map(|p| CharacterProfileIdx(p as u32))
    }

    pub fn get_soldier(&self, id: impl Into<SoldierProfileIdx>) -> Option<&SoldierProfile> {
        self.soldiers.get(usize::from(id.into()))
    }

    /// Resolve the stable hackable-level identifier derived from a soldier's
    /// CPF filename (for example `Guard A01` becomes `guard_a01`).
    ///
    /// Original level data identifies soldier profiles by their numeric CPF
    /// index, so repeated filenames are valid. Hackable JSON keeps the short
    /// name for unique filenames and disambiguates repeats by appending their
    /// original index, for example `archer05__47`.
    pub fn soldier_idx_by_identifier(&self, identifier: &str) -> Result<SoldierProfileIdx, String> {
        if let Some((base, raw_index)) = identifier.rsplit_once("__") {
            let index = raw_index.parse::<usize>().map_err(|_| {
                format!(
                    "invalid soldier profile identifier {identifier:?}; expected <name>__<cpf-index>"
                )
            })?;
            let profile = self.soldiers.get(index).ok_or_else(|| {
                format!("soldier profile identifier {identifier:?} uses missing CPF index {index}")
            })?;
            let actual_base = soldier_profile_identifier(profile);
            if actual_base != base {
                return Err(format!(
                    "soldier profile identifier {identifier:?} names CPF index {index}, whose identifier is {actual_base:?}"
                ));
            }
            let duplicate_count = self
                .soldiers
                .iter()
                .filter(|candidate| soldier_profile_identifier(candidate) == base)
                .count();
            if duplicate_count < 2 {
                return Err(format!(
                    "soldier profile identifier {identifier:?} unnecessarily uses a CPF index; use {base:?}"
                ));
            }
            return Ok(SoldierProfileIdx(index as u32));
        }

        let mut matches = self
            .soldiers
            .iter()
            .enumerate()
            .filter(|(_, profile)| soldier_profile_identifier(profile) == identifier);
        let Some((index, _)) = matches.next() else {
            return Err(format!("unknown soldier profile identifier {identifier:?}"));
        };
        if matches.next().is_some() {
            let alternatives = self
                .soldiers
                .iter()
                .enumerate()
                .filter(|(_, profile)| soldier_profile_identifier(profile) == identifier)
                .map(|(index, _)| format!("{identifier}__{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "ambiguous soldier profile identifier {identifier:?}; use one of: {alternatives}"
            ));
        }
        Ok(SoldierProfileIdx(index as u32))
    }

    /// Lookup a HtH weapon profile by the character/soldier profile's
    /// hand-to-hand weapon id. The stored id is 1-based; this function
    /// subtracts 1 to index into `self.hth_weapons`.
    ///
    /// Returns `None` for `id == 0` (no weapon) or when the index is out
    /// of range.
    pub fn get_hth_weapon(&self, id: u32) -> Option<&HtHWeaponProfile> {
        let idx = id.checked_sub(1)? as usize;
        self.hth_weapons.get(idx)
    }

    /// Lookup a bow profile by the character/soldier profile's shooting
    /// weapon id. Same 1-based convention as [`get_hth_weapon`].
    pub fn get_bow(&self, id: u32) -> Option<&BowProfile> {
        let idx = id.checked_sub(1)? as usize;
        self.bows.get(idx)
    }

    pub fn get_mission(&self, id: u32) -> Option<&MissionProfile> {
        self.missions.get(id as usize)
    }

    /// Append a synthetic mission profile for a `.rhm` that isn't in the
    /// campaign descriptor. Constructs a default profile with
    /// `mission_filename` / `proto_level_filename` / `mission_name` set,
    /// and applies the special-case ARES overrides for the SherwoodOutro
    /// mission. Returns the new profile's index.
    pub fn add_forced_mission(
        &mut self,
        proto_level_filename: String,
        mission_filename: String,
        mission_name: String,
    ) -> u32 {
        tracing::warn!("Adding a forced mission profile: {mission_filename}");
        let mut p = MissionProfile {
            id: self.missions.len() as u32,
            proto_level_filename,
            mission_filename,
            mission_name,
            ..MissionProfile::default()
        };
        // SherwoodOutro is special-cased: ARES success/loss states pinned to 11.
        if p.mission_filename == "SherwoodOutro" {
            p.ares_state_succeeded = 11;
            p.ares_state_lost = 11;
        }
        let idx = self.missions.len() as u32;
        self.missions.push(p);
        idx
    }

    pub fn get_civilian(&self, id: impl Into<CivilianProfileIdx>) -> Option<&CivilianProfile> {
        self.civilians.get(usize::from(id.into()))
    }

    // ── Index lookup (for serialization) ─────────────────────────

    // ── Profile pointer serialization ────────────────────────────
}

/// Canonical readable identifier used by hackable mission JSON.
pub fn soldier_profile_identifier(profile: &SoldierProfile) -> String {
    let mut identifier = String::new();
    let mut separator = false;
    for ch in profile.filename.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            if separator && !identifier.is_empty() {
                identifier.push('_');
            }
            identifier.push(ch);
            separator = false;
        } else {
            separator = true;
        }
    }
    identifier
}

// ─── Legacy authored profile.cpf loading ─────────────────────────
//
// Format: u32 count + per-profile fields.

fn read_count(reader: &mut LegacyReader<'_>, field: &str) -> LegacyResult<usize> {
    Ok(reader.read_u32(field)? as usize)
}

fn reserve_legacy<T>(
    reader: &mut LegacyReader<'_>,
    field: &str,
    count: usize,
) -> LegacyResult<Vec<T>> {
    let offset = reader.offset();
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| reader.allocation_error(offset, field, count))?;
    Ok(values)
}

/// Read a Vec<u32> as a u32 count prefix + N u32 values.
fn read_u32_vec(reader: &mut LegacyReader<'_>, field: &str) -> LegacyResult<Vec<u32>> {
    reader.scope(field, |reader| {
        let count = read_count(reader, "count")?;
        let mut values = reserve_legacy(reader, "items", count)?;
        for index in 0..count {
            values.push(reader.read_u32(format_args!("items[{index}]"))?);
        }
        Ok(values)
    })
}

fn read_profiles<T>(
    reader: &mut LegacyReader<'_>,
    field: &str,
    mut read_one: impl FnMut(&mut LegacyReader<'_>, usize) -> LegacyResult<T>,
) -> LegacyResult<Vec<T>> {
    reader.scope(field, |reader| {
        let count = read_count(reader, "count")?;
        let mut values = reserve_legacy(reader, "items", count)?;
        for index in 0..count {
            values.push(reader.scope(format!("items[{index}]"), |reader| read_one(reader, index))?);
        }
        Ok(values)
    })
}

fn normalized_action(value: u32) -> Action {
    Action::from_u32(value)
}

fn normalized_mission_type(value: u32) -> MissionType {
    MissionType::from_u32(value)
}

fn validate_character_index(
    reader: &mut LegacyReader<'_>,
    offset: u64,
    field: impl std::fmt::Display,
    index: u32,
    character_count: usize,
) -> LegacyResult<u32> {
    if (index as usize) < character_count {
        Ok(index)
    } else {
        Err(reader.invalid_value(
            offset,
            field,
            index,
            "an index into the loaded character profile table",
        ))
    }
}

impl MissionProfile {
    fn read_legacy_cpf(
        reader: &mut LegacyReader<'_>,
        character_count: usize,
    ) -> LegacyResult<Self> {
        let id = reader.read_u32("id")?;
        let proto_level_filename = reader.read_string("proto_level_filename")?;
        let mission_filename = reader.read_string("mission_filename")?;
        let mission_name = reader.read_string("mission_name")?;

        // The previous loader and shipped-data path normalize unknown enum
        // discriminants instead of rejecting the entire authored cache.
        let mission_type = normalized_mission_type(reader.read_u32("mission_type")?);
        let location_value = reader.read_u32("location")?;
        let location = MissionLocation::try_from(location_value).unwrap_or_else(|_| {
            tracing::warn!("invalid MissionLocation value {location_value}, clamping to York");
            MissionLocation::York
        });

        let pass_through_hq = reader.read_bool("pass_through_hq")?;
        let life_time = reader.read_u16("life_time")?;
        let obligatory = reader.read_bool("obligatory")?;
        let length = reader.read_u16("length")?;
        let min_ransom = reader.read_u32("min_ransom")?;
        let max_ransom = reader.read_u32("max_ransom")?;
        let missions_required_to_be_done = read_u32_vec(reader, "missions_required_to_be_done")?;
        let missions_required_not_to_be_done =
            read_u32_vec(reader, "missions_required_not_to_be_done")?;
        let min_gang_size = reader.read_u16("min_gang_size")?;
        let max_gang_size = reader.read_u16("max_gang_size")?;
        let access_probability = reader.read_u16("access_probability")?;
        let priority = reader.read_u16("priority")?;

        let required_count = read_count(reader, "required_character_indices.count")?;
        let mut required_character_indices =
            reserve_legacy(reader, "required_character_indices", required_count)?;
        for index in 0..required_count {
            let field = format!("required_character_indices[{index}]");
            let offset = reader.offset();
            let value = reader.read_u32(&field)?;
            required_character_indices.push(validate_character_index(
                reader,
                offset,
                &field,
                value,
                character_count,
            )?);
        }

        let ares_sensible = reader.read_bool("ares_sensible")?;
        let mut available_in_ares_state = [false; 10];
        // The authored CPF format contains nine entries although the runtime
        // structure has ten. This matches RHProfileManager::SerializeMissions.
        for (index, available) in available_in_ares_state[..9].iter_mut().enumerate() {
            *available = reader.read_bool(format_args!("available_in_ares_state[{index}]"))?;
        }

        Ok(Self {
            id,
            proto_level_filename,
            mission_filename,
            mission_name,
            mission_type,
            location,
            pass_through_hq,
            life_time,
            obligatory,
            length,
            min_ransom,
            max_ransom,
            missions_required_to_be_done,
            missions_required_not_to_be_done,
            min_gang_size,
            max_gang_size,
            access_probability,
            priority,
            required_character_indices,
            ares_sensible,
            available_in_ares_state,
            ares_state_succeeded: reader.read_i8("ares_state_succeeded")?,
            ares_state_lost: reader.read_i8("ares_state_lost")?,
            ares_state_refused: reader.read_i8("ares_state_refused")?,
            min_new_team_members: reader.read_u16("min_new_team_members")?,
            max_new_team_members: reader.read_u16("max_new_team_members")?,
            number_of_blazons_to_win: reader.read_u16("number_of_blazons_to_win")?,
            number_of_blazons_to_be_collected: reader
                .read_u16("number_of_blazons_to_be_collected")?,
            blazon_price: reader.read_u16("blazon_price")?,
            blazon_inflation: reader.read_u16("blazon_inflation")?,
            peasant_to_blazon_quotation: reader.read_u16("peasant_to_blazon_quotation")?,
            green_music: reader.read_string("green_music")?,
            yellow_music: reader.read_string("yellow_music")?,
            red_music: reader.read_string("red_music")?,
            ..Self::default()
        })
    }
}

impl CivilianProfile {
    fn read_legacy_cpf(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        let filename = reader.read_string("filename")?;
        let profile_name = reader.read_string("profile_name")?;
        let display_name = reader.read_string("display_name")?;
        let civilian_type_value = reader.read_u32("civilian_type")?;
        let attitude_value = reader.read_u32("attitude")?;
        Ok(Self {
            filename,
            profile_name,
            display_name,
            civilian_type: CivilianType::try_from(civilian_type_value).unwrap_or_else(|_| {
                tracing::warn!("invalid CivilianType value {civilian_type_value}, clamping to Vip");
                CivilianType::Vip
            }),
            attitude: Attitude::try_from(attitude_value).unwrap_or_else(|_| {
                tracing::warn!("invalid Attitude value {attitude_value}, clamping to Friendly");
                Attitude::Friendly
            }),
            exclamation_id: reader.read_u32("exclamation_id")?,
        })
    }
}

impl ThrustProfile {
    fn read_legacy_cpf(
        reader: &mut LegacyReader<'_>,
        clamp_counts: &mut (u32, u32),
    ) -> LegacyResult<Self> {
        // The on-disk CPF layout interleaves target/stunts/distances with
        // kind/direction in the middle (not the natural struct order):
        //   target (u32), stunning, cutting, min, max (u16×4),
        //   kind (u32), direction (u32),
        //   initAngle, finalAngle, rotAngle, repulsion, stumble, energy (u16×6)
        // This matches the layout of the shipped `profile.cpf`.
        let target_value = reader.read_u32("target")?;
        let target = WeaponTarget::try_from(target_value).unwrap_or_else(|_| {
            tracing::warn!("invalid WeaponTarget value {target_value}, clamping to None");
            WeaponTarget::None
        });
        let stunning = reader.read_u16("stunning")?;
        let cutting = reader.read_u16("cutting")?;
        let minimal_distance = reader.read_u16("minimal_distance")?;
        let maximal_distance = reader.read_u16("maximal_distance")?;
        let kind_value = reader.read_u32("kind")?;
        let kind = WeaponThrustKind::try_from(kind_value).unwrap_or_else(|_| {
            tracing::debug!("invalid WeaponThrustKind value {kind_value}, clamping to Assault");
            clamp_counts.0 += 1;
            WeaponThrustKind::Assault
        });
        let direction_value = reader.read_u32("direction")?;
        let direction = WeaponThrustDirection::try_from(direction_value).unwrap_or_else(|_| {
            tracing::debug!(
                "invalid WeaponThrustDirection value {direction_value}, clamping to NonApplicable"
            );
            clamp_counts.1 += 1;
            WeaponThrustDirection::NonApplicable
        });
        Ok(Self {
            target,
            kind,
            direction,
            stunning,
            cutting,
            minimal_distance,
            maximal_distance,
            initial_angle: reader.read_u16("initial_angle")?,
            final_angle: reader.read_u16("final_angle")?,
            rotation_angle: reader.read_u16("rotation_angle")?,
            repulsion: reader.read_u16("repulsion")?,
            stumble_probability: reader.read_u16("stumble_probability")?,
            energy: reader.read_u16("energy")?,
        })
    }
}

impl HtHWeaponProfile {
    fn read_legacy_cpf(
        reader: &mut LegacyReader<'_>,
        clamps: &mut (u32, u32),
    ) -> LegacyResult<Self> {
        let mut distance = [0; 4];
        for (index, value) in distance.iter_mut().enumerate() {
            *value = reader.read_u16(format_args!("distance[{index}]"))?;
        }
        let mut protection_by_localization = [0; 5];
        for (index, value) in protection_by_localization.iter_mut().enumerate() {
            *value = reader.read_u16(format_args!("protection_by_localization[{index}]"))?;
        }
        let bludgeon_protection = reader.read_u16("bludgeon_protection")?;
        let piercing_protection = reader.read_u16("piercing_protection")?;
        let charge = reader.read_bool("charge")?;
        let shield = reader.read_bool("shield")?;
        let shield_width = reader.read_u16("shield_width")?;
        let shield_height = reader.read_u16("shield_height")?;
        let mut thrusts = std::array::from_fn(|_| ThrustProfile::default());
        for (index, thrust) in thrusts.iter_mut().enumerate() {
            *thrust = reader.scope(format!("thrusts[{index}]"), |reader| {
                ThrustProfile::read_legacy_cpf(reader, clamps)
            })?;
        }
        Ok(Self {
            distance,
            protection_by_localization,
            bludgeon_protection,
            piercing_protection,
            charge,
            shield,
            shield_width,
            shield_height,
            thrusts,
        })
    }
}

impl CharacterProfile {
    fn read_legacy_cpf(reader: &mut LegacyReader<'_>, index: u32) -> LegacyResult<Self> {
        // `index` and `priority` are derived from the loop counter, not
        // serialized. Note: the legacy implementation had an off-by-one
        // (the first iteration's index underflowed to 0xFFFFFFFF, giving
        // priority = 11 instead of 10), but every consumer of `priority`
        // uses relative comparisons so the bug was invisible. We use the
        // natural 0-based loop index (`(0, 10), (1, 9), …`).
        let priority = 10u32.checked_sub(index).ok_or_else(|| {
            let offset = reader.offset();
            reader.invalid_value(
                offset,
                "index",
                index,
                "a character profile index from 0 to 10",
            )
        })? as u16;

        let filename = reader.read_string("filename")?;
        let profile_name = reader.read_string("profile_name")?;
        let alternative_profile_name = reader.read_string("alternative_profile_name")?;
        let valid_alternative_profile = reader.read_bool("valid_alternative_profile")?;
        let vip = reader.read_bool("vip")?;
        let shooting = reader.read_u16("shooting")?;
        let fighting = reader.read_u16("fighting")?;
        let endurance = reader.read_u16("endurance")?;
        let exclamation_id = reader.read_u32("exclamation_id")?;
        let hth_weapon_id = reader.read_u32("hth_weapon_id")?;
        let shooting_weapon_id = reader.read_u32("shooting_weapon_id")?;

        // 3 action + ammo pairs
        let mut actions = [Action::default(); NUMBER_OF_PC_ACTIONS];
        let mut action_max_ammo = [0; NUMBER_OF_PC_ACTIONS];
        for i in 0..NUMBER_OF_PC_ACTIONS {
            actions[i] = normalized_action(reader.read_u32(format_args!("actions[{i}]"))?);
            action_max_ammo[i] = reader.read_u16(format_args!("action_max_ammo[{i}]"))?;
        }
        // 4 contextual actions
        let mut contextual_actions = [Action::default(); NUMBER_OF_PC_CONTEXTUAL_ACTIONS];
        for (i, action) in contextual_actions.iter_mut().enumerate() {
            *action = normalized_action(reader.read_u32(format_args!("contextual_actions[{i}]"))?);
        }

        let pathfinder_index = reader.read_u8("pathfinder_index")?;
        let box_move = MoveBox::read_legacy(reader, "box_move")?;
        let center = geo2d::read_legacy_geo_point(reader, "center")?;
        let wake_up = reader.read_u16("wake_up")?;

        let weapon_material_value = reader.read_u32("weapon_material")?;
        let weapon_material =
            WeaponMaterial::try_from(weapon_material_value).unwrap_or_else(|_| {
                tracing::warn!(
                    "invalid WeaponMaterial value {weapon_material_value}, clamping to SteelAndWood"
                );
                WeaponMaterial::SteelAndWood
            });
        let armor_material_value = reader.read_u32("armor_material")?;
        let armor_material = ArmorMaterial::try_from(armor_material_value).unwrap_or_else(|_| {
            tracing::warn!("invalid ArmorMaterial value {armor_material_value}, clamping to Plate");
            ArmorMaterial::Plate
        });

        Ok(Self {
            index,
            priority,
            filename,
            profile_name,
            display_name: String::new(),
            alternative_profile_name,
            valid_alternative_profile,
            vip,
            shooting,
            fighting,
            endurance,
            exclamation_id,
            hth_weapon_id,
            shooting_weapon_id,
            actions,
            action_max_ammo,
            contextual_actions,
            pathfinder_index,
            box_move,
            center: SpriteAnchor::new(center.x, center.y),
            wake_up,
            weapon_material,
            armor_material,
            detection_speed_in_forest: reader.read_u16("detection_speed_in_forest")?,
            detection_speed_in_city: reader.read_u16("detection_speed_in_city")?,
        })
    }
}

impl SoldierProfile {
    fn read_legacy_cpf(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        let filename = reader.read_string("filename")?;
        let profile_name = reader.read_string("profile_name")?;
        let display_name = reader.read_string("display_name")?;
        let life_point = reader.read_u16("life_point")?;
        let intelligence = reader.read_u16("intelligence")?;
        let courage = reader.read_u16("courage")?;
        let initiative = reader.read_u16("initiative")?;
        let pride = reader.read_u16("pride")?;
        let formation = reader.read_bool("formation")?;
        let shooting = reader.read_u16("shooting")?;
        let fighting = reader.read_u16("fighting")?;
        let endurance = reader.read_u16("endurance")?;

        let rank_value = reader.read_u32("rank")?;
        let rank = ProfileRank::try_from(rank_value).unwrap_or_else(|_| {
            tracing::warn!("invalid ProfileRank value {rank_value}, clamping to None");
            ProfileRank::None
        });
        let exclamation_id = reader.read_u32("exclamation_id")?;
        let bee_time = reader.read_u16("bee_time")?;

        // Flags packed as single byte (bitfield)
        let flags = reader.read_u8("flags")?;
        let money = reader.read_u16("money")?;
        let apple = reader.read_u16("apple")?;
        let beer = reader.read_u16("beer")?;
        let whistle = reader.read_u16("whistle")?;
        let hth_weapon_id = reader.read_u32("hth_weapon_id")?;
        let shooting_weapon_id = reader.read_u32("shooting_weapon_id")?;
        let pathfinder_index = reader.read_u8("pathfinder_index")?;
        let box_move = MoveBox::read_legacy(reader, "box_move")?;
        let center = geo2d::read_legacy_geo_point(reader, "center")?;
        let wake_up = reader.read_u16("wake_up")?;

        let weapon_material_value = reader.read_u32("weapon_material")?;
        let weapon_material =
            WeaponMaterial::try_from(weapon_material_value).unwrap_or_else(|_| {
                tracing::warn!(
                    "invalid WeaponMaterial value {weapon_material_value}, clamping to SteelAndWood"
                );
                WeaponMaterial::SteelAndWood
            });
        let armor_material_value = reader.read_u32("armor_material")?;
        let armor_material = ArmorMaterial::try_from(armor_material_value).unwrap_or_else(|_| {
            tracing::warn!("invalid ArmorMaterial value {armor_material_value}, clamping to Plate");
            ArmorMaterial::Plate
        });

        Ok(Self {
            filename,
            profile_name,
            display_name,
            life_point,
            intelligence,
            courage,
            initiative,
            pride,
            formation,
            shooting,
            fighting,
            endurance,
            rank,
            exclamation_id,
            bee_time,
            hostile: flags & 1 != 0,
            rider: flags & 2 != 0,
            heavy: flags & 4 != 0,
            vip: flags & 8 != 0,
            duty: flags & 16 != 0,
            strangle: flags & 32 != 0,
            money,
            apple,
            beer,
            whistle,
            hth_weapon_id,
            shooting_weapon_id,
            pathfinder_index,
            box_move,
            center: SpriteAnchor::new(center.x, center.y),
            wake_up,
            weapon_material,
            armor_material,
        })
    }
}

impl ProfileManager {
    fn read_legacy_cpf_hth_weapons(
        reader: &mut LegacyReader<'_>,
    ) -> LegacyResult<Vec<HtHWeaponProfile>> {
        let mut clamps = (0u32, 0u32);
        let profiles = read_profiles(reader, "hth_weapons", |reader, _| {
            HtHWeaponProfile::read_legacy_cpf(reader, &mut clamps)
        })?;
        if clamps.0 > 0 || clamps.1 > 0 {
            // Known quirk: shipped CPF data has garbage `kind`/`direction`
            // bytes that the original loader silently accepted. The clamp
            // is behaviorally identical for every caller of the strike
            // kind / direction getters.
            tracing::warn!(
                "HtH weapons: clamped {} invalid thrust kind(s) and {} invalid thrust direction(s) across {} weapon(s) (shipped data quirk, benign)",
                clamps.0,
                clamps.1,
                profiles.len()
            );
        }
        Ok(profiles)
    }

    fn read_all_legacy_cpf(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        // RHProfileManager::Serialize writes this exact order. This is the
        // authored profile.cpf cache format, not save/snapshot serialization.
        let hth_weapons = Self::read_legacy_cpf_hth_weapons(reader)?;
        let bows = read_profiles(reader, "bows", |reader, _| {
            BowProfile::read_legacy_cpf(reader)
        })?;
        let characters = read_profiles(reader, "characters", |reader, index| {
            CharacterProfile::read_legacy_cpf(reader, index as u32)
        })?;
        let soldiers = read_profiles(reader, "soldiers", |reader, _| {
            SoldierProfile::read_legacy_cpf(reader)
        })?;
        let character_count = characters.len();
        let missions = read_profiles(reader, "missions", |reader, _| {
            MissionProfile::read_legacy_cpf(reader, character_count)
        })?;
        let civilians = read_profiles(reader, "civilians", |reader, _| {
            CivilianProfile::read_legacy_cpf(reader)
        })?;

        Ok(Self {
            characters,
            soldiers,
            hth_weapons,
            bows,
            missions,
            civilians,
        })
    }

    /// Walk every mission's `.rhm` file and populate
    /// `MissionProfile::number_of_beam_mes` /
    /// `MissionProfile::required_actions`.
    ///
    /// Skips `Pseudo` missions and the placeholder `"Impossible_mission"`
    /// entry, then for every beam-me in every other mission, pushes the
    /// matching `Action::*` per `true` flag (duplicates allowed). On a
    /// missing/corrupt mission file, falls back to the "bad version"
    /// default of `number_of_beam_mes = 5` and an empty action list for
    /// that mission.
    ///
    /// Downstream consumers (`widget_state::requirements`,
    /// `campaign::CreateGang`, native `GetNumberOfBeamMes`) silently
    /// hide required-action requirements if these fields are zero, so
    /// this must run before the briefing UI or auto-gang-selection
    /// reaches a freshly loaded profile.
    pub fn import_beam_mes(&mut self, level_directory: &str) {
        for profile in self.missions.iter_mut() {
            if profile.mission_type == MissionType::Pseudo
                || profile.mission_filename == "Impossible_mission"
            {
                continue;
            }
            let path = format!("{}/{}.rhm", level_directory, profile.mission_filename);
            match crate::level_data::scan_mission_for_beam_mes(&path) {
                Ok(scan) => {
                    profile.number_of_beam_mes = scan.number_of_beam_mes;
                    for flags in scan.action_flags {
                        // One Action push per `true` flag per beam-me, using
                        // the flag→Action mapping below.
                        if flags.climb {
                            profile.required_actions.push(Action::Climb);
                        }
                        if flags.jump {
                            profile.required_actions.push(Action::Jump);
                        }
                        if flags.lockpick {
                            profile.required_actions.push(Action::Lockpick);
                        }
                        if flags.archery {
                            profile.required_actions.push(Action::Bow);
                        }
                        if flags.carry {
                            profile.required_actions.push(Action::LittleJohnCarry);
                        }
                        if flags.tie {
                            profile.required_actions.push(Action::Tie);
                        }
                        if flags.stun {
                            profile.required_actions.push(Action::Hit);
                        }
                        if flags.lever {
                            profile.required_actions.push(Action::Lever);
                        }
                        if flags.eat {
                            profile.required_actions.push(Action::Eat);
                        }
                        if flags.search {
                            profile.required_actions.push(Action::Search);
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        "ImportBeamMes: {} — falling back to number_of_beam_mes=5 (bad-version default)",
                        err
                    );
                    profile.number_of_beam_mes = 5;
                }
            }
        }
    }

    /// Load all profiles. Order is fixed by the authored CPF format:
    /// weapons, bows, characters, soldiers, missions, civilians.
    pub fn load_all_legacy_cpf(&mut self, file: &mut SbFile) -> Result<(), LegacyIoError> {
        let mut reader = LegacyReader::new(file);
        let loaded = reader.scope("profiles", Self::read_all_legacy_cpf)?;
        *self = loaded;
        Ok(())
    }
}

impl ProfileManager {
    /// Create a minimal ProfileManager for tests.
    #[cfg(test)]
    pub fn test_profiles() -> Self {
        let mut mgr = Self::new();
        // One hostile soldier profile at index 0
        mgr.soldiers.push(SoldierProfile {
            hostile: true,
            life_point: 80,
            ..SoldierProfile::default()
        });
        // One hostile civilian profile at index 0
        mgr.civilians.push(CivilianProfile {
            attitude: Attitude::Hostile,
            ..CivilianProfile::default()
        });
        mgr
    }
}

// ─── JSON loading ────────────────────────────────────────────────

impl ProfileManager {
    /// Load profiles from a JSON file (produced by cpf_to_json).
    pub fn load_json(path: &str) -> Result<Self, String> {
        let mut file = crate::sbfile::SbFile::open(path, crate::sbfile::SB_FILE_READ)
            .map_err(|e| format!("Failed to open {}: error {}", path, e))?;
        let mut bytes = vec![0u8; file.get_size() as usize];
        file.serialize_bytes(&mut bytes)
            .map_err(|e| format!("Failed to read {}: error {}", path, e))?;
        let data = String::from_utf8(bytes)
            .map_err(|e| format!("Failed to decode {} as UTF-8: {}", path, e))?;
        serde_json::from_str(&data).map_err(|e| format!("Failed to parse {}: {}", path, e))
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hackable_soldier_identifier_is_readable_and_uniquely_resolved() {
        let mut profiles = ProfileManager::new();
        profiles.soldiers.push(SoldierProfile {
            filename: "Guard A01".to_owned(),
            ..Default::default()
        });
        assert_eq!(
            soldier_profile_identifier(&profiles.soldiers[0]),
            "guard_a01"
        );
        assert_eq!(
            profiles.soldier_idx_by_identifier("guard_a01").unwrap(),
            SoldierProfileIdx(0)
        );

        profiles.soldiers.push(SoldierProfile {
            filename: "Guard A01".to_owned(),
            hostile: true,
            ..Default::default()
        });
        let error = profiles.soldier_idx_by_identifier("guard_a01").unwrap_err();
        assert!(error.contains("ambiguous"));
        assert!(error.contains("guard_a01__0"));
        assert!(error.contains("guard_a01__1"));
        assert_eq!(
            profiles.soldier_idx_by_identifier("guard_a01__0").unwrap(),
            SoldierProfileIdx(0)
        );
        assert_eq!(
            profiles.soldier_idx_by_identifier("guard_a01__1").unwrap(),
            SoldierProfileIdx(1)
        );
        assert!(
            profiles
                .soldier_idx_by_identifier("archer01__1")
                .unwrap_err()
                .contains("whose identifier is")
        );
        assert!(
            profiles
                .soldier_idx_by_identifier("guard_a01__99")
                .unwrap_err()
                .contains("missing CPF index")
        );

        let unique = ProfileManager {
            soldiers: vec![SoldierProfile {
                filename: "Archer01".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(
            unique
                .soldier_idx_by_identifier("archer01__0")
                .unwrap_err()
                .contains("unnecessarily")
        );
    }
    #[test]
    fn action_round_trip() {
        assert_eq!(Action::from_u32(0), Action::NoAction);
        assert_eq!(Action::from_u32(1), Action::Bow);
        assert_eq!(Action::from_u32(Action::Test as u32), Action::Test);
        assert_eq!(Action::from_u32(999), Action::NoAction);
    }

    #[test]
    fn mission_type_round_trip() {
        assert_eq!(MissionType::from_u32(0), MissionType::Historical);
        assert_eq!(MissionType::from_u32(5), MissionType::Pseudo);
        assert_eq!(MissionType::from_u32(7), MissionType::End);
        assert_eq!(MissionType::from_u32(99), MissionType::End);
    }

    #[test]
    fn profile_manager_accessors() {
        let mut mgr = ProfileManager::new();
        mgr.missions.push(MissionProfile {
            id: 42,
            mission_name: "Test Mission".into(),
            ..Default::default()
        });
        let m = mgr.get_mission(0).unwrap();
        assert_eq!(m.id, 42);
        assert_eq!(m.mission_name, "Test Mission");
        assert!(mgr.get_mission(1).is_none());
    }

    #[test]
    fn serde_json_round_trip() {
        let mut mgr = ProfileManager::new();
        mgr.missions.push(MissionProfile {
            id: 42,
            mission_name: "Test".into(),
            mission_type: MissionType::Attack,
            blazon_price: 10,
            ..Default::default()
        });
        mgr.characters.push(CharacterProfile {
            index: 0,
            profile_name: "Robin".into(),
            shooting: 100,
            ..Default::default()
        });

        let json = serde_json::to_string(&mgr).unwrap();
        let mgr2: ProfileManager = serde_json::from_str(&json).unwrap();

        assert_eq!(mgr2.missions.len(), 1);
        assert_eq!(mgr2.missions[0].id, 42);
        assert_eq!(mgr2.missions[0].blazon_price, 10);
        assert_eq!(mgr2.characters[0].profile_name, "Robin");
        assert_eq!(mgr2.characters[0].shooting, 100);
    }

    #[test]
    fn default_mission_profile() {
        let p = MissionProfile::default();
        assert_eq!(p.id, 0);
        assert_eq!(p.mission_type, MissionType::End);
        assert!(p.required_actions.is_empty());
    }

    // ── Integration tests against real profile.json files ───────

    fn demo_profile_path() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../datadirs/demo/Data/Configuration/profile.json")
    }

    fn fullgame_profile_path() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../datadirs/fullgame/Data/Configuration/profile.json")
    }

    #[test]
    fn load_demo_profile_json() {
        let path = demo_profile_path();
        if !path.exists() {
            eprintln!("skipping: demo profile.json not present");
            return;
        }

        let mgr = ProfileManager::load_json(path.to_str().unwrap())
            .expect("failed to load demo profile.json");

        // Collection counts
        assert_eq!(mgr.hth_weapons.len(), 20, "expected 20 HtH weapons");
        assert_eq!(mgr.bows.len(), 4, "expected 4 bows");
        assert_eq!(mgr.characters.len(), 10, "expected 10 characters");
        assert_eq!(mgr.soldiers.len(), 68, "expected 68 soldiers");
        assert_eq!(mgr.missions.len(), 2, "expected 2 missions");
        assert_eq!(mgr.civilians.len(), 24, "expected 24 civilians");

        // Robin Hood is character[0]
        let robin = &mgr.characters[0];
        assert!(
            robin.profile_name.contains("Robin"),
            "expected Robin Hood, got: {}",
            robin.profile_name
        );
        assert_eq!(robin.shooting, 100);
        assert_eq!(robin.fighting, 20);
        assert!(robin.vip, "Robin Hood should be a VIP");

        // Mission names
        assert_eq!(mgr.missions[0].mission_name, "Sherwood Forest");
        assert_eq!(mgr.missions[1].mission_name, "Save Scarlett");
    }

    #[test]
    fn load_fullgame_profile_json() {
        let path = fullgame_profile_path();
        if !path.exists() {
            eprintln!("skipping: fullgame profile.json not present");
            return;
        }

        let mgr = ProfileManager::load_json(path.to_str().unwrap())
            .expect("failed to load fullgame profile.json");

        // Collection counts
        assert_eq!(mgr.missions.len(), 63, "expected 63 missions");
        assert_eq!(mgr.characters.len(), 10, "expected 10 characters");
        assert_eq!(mgr.hth_weapons.len(), 27, "expected 27 HtH weapons");

        // Verify mission type distribution — at least one of each expected type
        let count_type = |mt: MissionType| -> usize {
            mgr.missions.iter().filter(|m| m.mission_type == mt).count()
        };
        assert!(
            count_type(MissionType::Historical) >= 1,
            "expected at least one Historical mission"
        );
        assert!(
            count_type(MissionType::Attack) >= 1,
            "expected at least one Attack mission"
        );
        assert!(
            count_type(MissionType::Ambush) >= 1,
            "expected at least one Ambush mission"
        );
        assert!(
            count_type(MissionType::Tactical) >= 1,
            "expected at least one Tactical mission"
        );
        assert!(
            count_type(MissionType::Hq) >= 1,
            "expected at least one Hq mission"
        );
        assert!(
            count_type(MissionType::Pseudo) >= 1,
            "expected at least one Pseudo mission"
        );
    }

    #[test]
    fn demo_profile_serde_round_trip() {
        let path = demo_profile_path();
        if !path.exists() {
            eprintln!("skipping: demo profile.json not present");
            return;
        }

        let mgr = ProfileManager::load_json(path.to_str().unwrap())
            .expect("failed to load demo profile.json");

        // Serialize to JSON string, then deserialize back
        let json = serde_json::to_string(&mgr).expect("failed to serialize ProfileManager to JSON");
        let mgr2: ProfileManager =
            serde_json::from_str(&json).expect("failed to deserialize ProfileManager from JSON");

        // Verify all collection counts survive the round trip
        assert_eq!(mgr2.hth_weapons.len(), mgr.hth_weapons.len());
        assert_eq!(mgr2.bows.len(), mgr.bows.len());
        assert_eq!(mgr2.characters.len(), mgr.characters.len());
        assert_eq!(mgr2.soldiers.len(), mgr.soldiers.len());
        assert_eq!(mgr2.missions.len(), mgr.missions.len());
        assert_eq!(mgr2.civilians.len(), mgr.civilians.len());

        // Spot-check that field values survived
        assert_eq!(mgr2.characters[0].shooting, 100);
        assert_eq!(mgr2.characters[0].fighting, 20);
        assert!(mgr2.characters[0].vip);
        assert_eq!(mgr2.missions[0].mission_name, "Sherwood Forest");
        assert_eq!(mgr2.missions[1].mission_name, "Save Scarlett");
    }
}
