//! Profile data types and manager for the Robin Hood game.
//!
//! Profiles are static game data describing characters, soldiers, civilians,
//! weapons, and missions — loaded from CSV at startup. The manager owns
//! vectors of profile structs and handles CSV loading and binary
//! serialization.

use num_enum::TryFromPrimitive;
use serde::{Deserialize, Serialize};

use crate::geo2d::{self, BBox2D, GeoPoint2D};
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
    /// True when this is one of the contextual (non-toolbar) actions.

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct CharacterProfile {
    pub index: u32,
    pub filename: String,
    pub profile_name: String,
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
    pub box_move: BBox2D,
    pub center: GeoPoint2D,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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
    pub box_move: BBox2D,
    pub center: GeoPoint2D,
    pub wake_up: u16,
    pub weapon_material: WeaponMaterial,
    pub armor_material: ArmorMaterial,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct CivilianProfile {
    pub filename: String,
    pub profile_name: String,
    pub display_name: String,
    pub civilian_type: CivilianType,
    pub attitude: Attitude,
    pub exclamation_id: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct BowHitChance {
    pub hit_chance: [u16; 6],
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct BowShootMode {
    pub range: u16,
    pub hit_chances: [BowHitChance; 3], // Beginner, Normal, Elite
    pub damage: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct BowProfile {
    pub normal_shoot: BowShootMode,
    pub has_long_shoot: bool,
    pub long_shoot: BowShootMode,
}

impl BowShootMode {
    fn load_legacy_cpf(&mut self, file: &mut SbFile) -> Result<(), i32> {
        file.serialize_u16(&mut self.range)?;
        for i in 0..6 {
            for skill in self.hit_chances.iter_mut() {
                file.serialize_u16(&mut skill.hit_chance[i])?;
            }
        }
        file.serialize_u16(&mut self.damage)?;
        Ok(())
    }
}

impl BowProfile {
    pub fn load_legacy_cpf(&mut self, file: &mut SbFile) -> Result<(), i32> {
        self.normal_shoot.load_legacy_cpf(file)?;
        file.serialize_bool(&mut self.has_long_shoot)?;
        self.long_shoot.load_legacy_cpf(file)?;
        Ok(())
    }
}

// ─── Profile Manager ─────────────────────────────────────────────

/// Manages all profile data (characters, soldiers, civilians, weapons,
/// missions). Loaded from CSV files at startup, optionally cached as
/// compiled `.cpf` binary files.
#[derive(Debug, Default, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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

// ─── Binary Serialization ────────────────────────────────────────
//
// Format: u32 count + per-profile fields.

/// Serialize a Vec<u32> as a u32 count prefix + N u32 values.
fn serialize_u32_vec(file: &mut SbFile, vec: &mut Vec<u32>) -> Result<(), i32> {
    if file.is_write_mode() {
        let mut count = vec.len() as u32;
        file.serialize_u32(&mut count)?;
        for val in vec.iter_mut() {
            file.serialize_u32(val)?;
        }
    } else {
        let mut count = 0u32;
        file.serialize_u32(&mut count)?;
        vec.clear();
        for _ in 0..count {
            let mut val = 0u32;
            file.serialize_u32(&mut val)?;
            vec.push(val);
        }
    }
    Ok(())
}

impl MissionProfile {
    /// Binary serialize the per-mission profile block.
    pub fn load_legacy_cpf(
        &mut self,
        file: &mut SbFile,
        _characters: &[CharacterProfile],
    ) -> Result<(), i32> {
        file.serialize_u32(&mut self.id)?;

        file.serialize_string(&mut self.proto_level_filename)?;
        file.serialize_string(&mut self.mission_filename)?;
        file.serialize_string(&mut self.mission_name)?;

        // Enums serialized as u32 with a clamping fallback on read.
        let mut mtype = self.mission_type as u32;
        file.serialize_u32(&mut mtype)?;
        if file.is_read_mode() {
            self.mission_type = MissionType::from_u32(mtype);
        }

        let mut loc = self.location as u32;
        file.serialize_u32(&mut loc)?;
        if file.is_read_mode() {
            self.location = MissionLocation::try_from(loc).unwrap_or_else(|_| {
                tracing::warn!("invalid MissionLocation value {loc}, clamping to York");
                MissionLocation::York
            });
        }

        file.serialize_bool(&mut self.pass_through_hq)?;
        file.serialize_u16(&mut self.life_time)?;
        file.serialize_bool(&mut self.obligatory)?;
        file.serialize_u16(&mut self.length)?;
        file.serialize_u32(&mut self.min_ransom)?;
        file.serialize_u32(&mut self.max_ransom)?;

        serialize_u32_vec(file, &mut self.missions_required_to_be_done)?;
        serialize_u32_vec(file, &mut self.missions_required_not_to_be_done)?;

        file.serialize_u16(&mut self.min_gang_size)?;
        file.serialize_u16(&mut self.max_gang_size)?;
        file.serialize_u16(&mut self.access_probability)?;
        file.serialize_u16(&mut self.priority)?;

        // Required characters: stored as indices into character vector
        if file.is_write_mode() {
            let mut count = self.required_character_indices.len() as u32;
            file.serialize_u32(&mut count)?;
            for idx in self.required_character_indices.iter_mut() {
                file.serialize_u32(idx)?;
            }
        } else {
            let mut count = 0u32;
            file.serialize_u32(&mut count)?;
            self.required_character_indices.clear();
            for _ in 0..count {
                let mut idx = 0u32;
                file.serialize_u32(&mut idx)?;
                self.required_character_indices.push(idx);
            }
        }

        file.serialize_bool(&mut self.ares_sensible)?;

        // Only 9 ARES states are serialized (not 10) — the format uses a
        // 9-slot table even though the in-memory array has room for 10.
        for i in 0..9 {
            file.serialize_bool(&mut self.available_in_ares_state[i])?;
        }

        file.serialize_i8(&mut self.ares_state_succeeded)?;
        file.serialize_i8(&mut self.ares_state_lost)?;
        file.serialize_i8(&mut self.ares_state_refused)?;

        file.serialize_u16(&mut self.min_new_team_members)?;
        file.serialize_u16(&mut self.max_new_team_members)?;

        file.serialize_u16(&mut self.number_of_blazons_to_win)?;
        file.serialize_u16(&mut self.number_of_blazons_to_be_collected)?;
        file.serialize_u16(&mut self.blazon_price)?;
        file.serialize_u16(&mut self.blazon_inflation)?;
        file.serialize_u16(&mut self.peasant_to_blazon_quotation)?;

        file.serialize_string(&mut self.green_music)?;
        file.serialize_string(&mut self.yellow_music)?;
        file.serialize_string(&mut self.red_music)?;

        Ok(())
    }
}

impl CivilianProfile {
    pub fn load_legacy_cpf(&mut self, file: &mut SbFile) -> Result<(), i32> {
        file.serialize_string(&mut self.filename)?;
        file.serialize_string(&mut self.profile_name)?;
        file.serialize_string(&mut self.display_name)?;
        let mut ct = self.civilian_type as u32;
        file.serialize_u32(&mut ct)?;
        if file.is_read_mode() {
            self.civilian_type = CivilianType::try_from(ct).unwrap_or_else(|_| {
                tracing::warn!("invalid CivilianType value {ct}, clamping to Vip");
                CivilianType::Vip
            });
        }
        let mut att = self.attitude as u32;
        file.serialize_u32(&mut att)?;
        if file.is_read_mode() {
            self.attitude = Attitude::try_from(att).unwrap_or_else(|_| {
                tracing::warn!("invalid Attitude value {att}, clamping to Friendly");
                Attitude::Friendly
            });
        }
        file.serialize_u32(&mut self.exclamation_id)?;
        Ok(())
    }
}

impl ThrustProfile {
    pub fn load_legacy_cpf(
        &mut self,
        file: &mut SbFile,
        clamp_counts: &mut (u32, u32),
    ) -> Result<(), i32> {
        // The on-disk CPF layout interleaves target/stunts/distances with
        // kind/direction in the middle (not the natural struct order):
        //   target (u32), stunning, cutting, min, max (u16×4),
        //   kind (u32), direction (u32),
        //   initAngle, finalAngle, rotAngle, repulsion, stumble, energy (u16×6)
        // This matches the layout of the shipped `profile.cpf`.
        let mut target = self.target as u32;
        file.serialize_u32(&mut target)?;
        if file.is_read_mode() {
            self.target = WeaponTarget::try_from(target).unwrap_or_else(|_| {
                tracing::warn!("invalid WeaponTarget value {target}, clamping to None");
                WeaponTarget::None
            });
        }
        file.serialize_u16(&mut self.stunning)?;
        file.serialize_u16(&mut self.cutting)?;
        file.serialize_u16(&mut self.minimal_distance)?;
        file.serialize_u16(&mut self.maximal_distance)?;
        let mut kind = self.kind as u32;
        file.serialize_u32(&mut kind)?;
        if file.is_read_mode() {
            self.kind = WeaponThrustKind::try_from(kind).unwrap_or_else(|_| {
                tracing::debug!("invalid WeaponThrustKind value {kind}, clamping to Assault");
                clamp_counts.0 += 1;
                WeaponThrustKind::Assault
            });
        }
        let mut dir = self.direction as u32;
        file.serialize_u32(&mut dir)?;
        if file.is_read_mode() {
            self.direction = WeaponThrustDirection::try_from(dir).unwrap_or_else(|_| {
                tracing::debug!(
                    "invalid WeaponThrustDirection value {dir}, clamping to NonApplicable"
                );
                clamp_counts.1 += 1;
                WeaponThrustDirection::NonApplicable
            });
        }
        file.serialize_u16(&mut self.initial_angle)?;
        file.serialize_u16(&mut self.final_angle)?;
        file.serialize_u16(&mut self.rotation_angle)?;
        file.serialize_u16(&mut self.repulsion)?;
        file.serialize_u16(&mut self.stumble_probability)?;
        file.serialize_u16(&mut self.energy)?;
        Ok(())
    }
}

impl HtHWeaponProfile {
    pub fn load_legacy_cpf(
        &mut self,
        file: &mut SbFile,
        clamps: &mut (u32, u32),
    ) -> Result<(), i32> {
        for d in self.distance.iter_mut() {
            file.serialize_u16(d)?;
        }
        for p in self.protection_by_localization.iter_mut() {
            file.serialize_u16(p)?;
        }
        file.serialize_u16(&mut self.bludgeon_protection)?;
        file.serialize_u16(&mut self.piercing_protection)?;
        file.serialize_bool(&mut self.charge)?;
        file.serialize_bool(&mut self.shield)?;
        file.serialize_u16(&mut self.shield_width)?;
        file.serialize_u16(&mut self.shield_height)?;
        for t in self.thrusts.iter_mut() {
            t.load_legacy_cpf(file, clamps)?;
        }
        Ok(())
    }
}

impl CharacterProfile {
    pub fn load_legacy_cpf(&mut self, file: &mut SbFile, index: u32) -> Result<(), i32> {
        // `index` and `priority` are derived from the loop counter, not
        // serialized. Note: the legacy implementation had an off-by-one
        // (the first iteration's index underflowed to 0xFFFFFFFF, giving
        // priority = 11 instead of 10), but every consumer of `priority`
        // uses relative comparisons so the bug was invisible. We use the
        // natural 0-based loop index (`(0, 10), (1, 9), …`).
        self.index = index;
        self.priority = (10 - index) as u16;

        file.serialize_string(&mut self.filename)?;
        file.serialize_string(&mut self.profile_name)?;
        file.serialize_string(&mut self.alternative_profile_name)?;
        file.serialize_bool(&mut self.valid_alternative_profile)?;
        file.serialize_bool(&mut self.vip)?;
        file.serialize_u16(&mut self.shooting)?;
        file.serialize_u16(&mut self.fighting)?;
        file.serialize_u16(&mut self.endurance)?;
        file.serialize_u32(&mut self.exclamation_id)?;
        file.serialize_u32(&mut self.hth_weapon_id)?;
        file.serialize_u32(&mut self.shooting_weapon_id)?;

        // 3 action + ammo pairs
        for i in 0..NUMBER_OF_PC_ACTIONS {
            let mut act = self.actions[i] as u32;
            file.serialize_u32(&mut act)?;
            if file.is_read_mode() {
                self.actions[i] = Action::from_u32(act);
            }
            file.serialize_u16(&mut self.action_max_ammo[i])?;
        }
        // 4 contextual actions
        for i in 0..NUMBER_OF_PC_CONTEXTUAL_ACTIONS {
            let mut act = self.contextual_actions[i] as u32;
            file.serialize_u32(&mut act)?;
            if file.is_read_mode() {
                self.contextual_actions[i] = Action::from_u32(act);
            }
        }

        file.serialize_u8(&mut self.pathfinder_index)?;
        self.box_move.binary_rw(file)?;
        geo2d::serialize_geo_point(file, &mut self.center)?;
        file.serialize_u16(&mut self.wake_up)?;

        let mut wm = self.weapon_material as u32;
        file.serialize_u32(&mut wm)?;
        if file.is_read_mode() {
            self.weapon_material = WeaponMaterial::try_from(wm).unwrap_or_else(|_| {
                tracing::warn!("invalid WeaponMaterial value {wm}, clamping to SteelAndWood");
                WeaponMaterial::SteelAndWood
            });
        }
        let mut am = self.armor_material as u32;
        file.serialize_u32(&mut am)?;
        if file.is_read_mode() {
            self.armor_material = ArmorMaterial::try_from(am).unwrap_or_else(|_| {
                tracing::warn!("invalid ArmorMaterial value {am}, clamping to Plate");
                ArmorMaterial::Plate
            });
        }

        file.serialize_u16(&mut self.detection_speed_in_forest)?;
        file.serialize_u16(&mut self.detection_speed_in_city)?;

        Ok(())
    }
}

impl SoldierProfile {
    pub fn load_legacy_cpf(&mut self, file: &mut SbFile) -> Result<(), i32> {
        file.serialize_string(&mut self.filename)?;
        file.serialize_string(&mut self.profile_name)?;
        file.serialize_string(&mut self.display_name)?;
        file.serialize_u16(&mut self.life_point)?;
        file.serialize_u16(&mut self.intelligence)?;
        file.serialize_u16(&mut self.courage)?;
        file.serialize_u16(&mut self.initiative)?;
        file.serialize_u16(&mut self.pride)?;
        file.serialize_bool(&mut self.formation)?;
        file.serialize_u16(&mut self.shooting)?;
        file.serialize_u16(&mut self.fighting)?;
        file.serialize_u16(&mut self.endurance)?;

        let mut rank = self.rank as u32;
        file.serialize_u32(&mut rank)?;
        if file.is_read_mode() {
            self.rank = ProfileRank::try_from(rank).unwrap_or_else(|_| {
                tracing::warn!("invalid ProfileRank value {rank}, clamping to None");
                ProfileRank::None
            });
        }

        file.serialize_u32(&mut self.exclamation_id)?;
        file.serialize_u16(&mut self.bee_time)?;

        // Flags packed as single byte (bitfield)
        let mut flags_byte: u8 = 0;
        if file.is_write_mode() {
            if self.hostile {
                flags_byte |= 1;
            }
            if self.rider {
                flags_byte |= 2;
            }
            if self.heavy {
                flags_byte |= 4;
            }
            if self.vip {
                flags_byte |= 8;
            }
            if self.duty {
                flags_byte |= 16;
            }
            if self.strangle {
                flags_byte |= 32;
            }
        }
        file.serialize_u8(&mut flags_byte)?;
        if file.is_read_mode() {
            self.hostile = flags_byte & 1 != 0;
            self.rider = flags_byte & 2 != 0;
            self.heavy = flags_byte & 4 != 0;
            self.vip = flags_byte & 8 != 0;
            self.duty = flags_byte & 16 != 0;
            self.strangle = flags_byte & 32 != 0;
        }

        file.serialize_u16(&mut self.money)?;
        file.serialize_u16(&mut self.apple)?;
        file.serialize_u16(&mut self.beer)?;
        file.serialize_u16(&mut self.whistle)?;
        file.serialize_u32(&mut self.hth_weapon_id)?;
        file.serialize_u32(&mut self.shooting_weapon_id)?;
        file.serialize_u8(&mut self.pathfinder_index)?;
        self.box_move.binary_rw(file)?;
        geo2d::serialize_geo_point(file, &mut self.center)?;
        file.serialize_u16(&mut self.wake_up)?;

        let mut wm = self.weapon_material as u32;
        file.serialize_u32(&mut wm)?;
        if file.is_read_mode() {
            self.weapon_material = WeaponMaterial::try_from(wm).unwrap_or_else(|_| {
                tracing::warn!("invalid WeaponMaterial value {wm}, clamping to SteelAndWood");
                WeaponMaterial::SteelAndWood
            });
        }
        let mut am = self.armor_material as u32;
        file.serialize_u32(&mut am)?;
        if file.is_read_mode() {
            self.armor_material = ArmorMaterial::try_from(am).unwrap_or_else(|_| {
                tracing::warn!("invalid ArmorMaterial value {am}, clamping to Plate");
                ArmorMaterial::Plate
            });
        }

        Ok(())
    }
}

impl ProfileManager {
    /// Serialize all mission profiles.
    pub fn load_legacy_cpf_missions(&mut self, file: &mut SbFile) -> Result<(), i32> {
        if file.is_write_mode() {
            let mut count = self.missions.len() as u32;
            file.serialize_u32(&mut count)?;
        } else {
            let mut count = 0u32;
            file.serialize_u32(&mut count)?;
            self.missions.clear();
            for _ in 0..count {
                self.missions.push(MissionProfile::default());
            }
        }

        // Need to serialize each mission; the character list is needed
        // for resolving character indices in required_character_indices.
        let n = self.missions.len();
        for i in 0..n {
            // Split borrow: take mission out, serialize, put back
            let mut mission = std::mem::take(&mut self.missions[i]);
            mission.load_legacy_cpf(file, &self.characters)?;
            self.missions[i] = mission;
        }

        Ok(())
    }

    /// Serialize all civilian profiles.
    pub fn load_legacy_cpf_civilians(&mut self, file: &mut SbFile) -> Result<(), i32> {
        if file.is_write_mode() {
            let mut count = self.civilians.len() as u32;
            file.serialize_u32(&mut count)?;
        } else {
            let mut count = 0u32;
            file.serialize_u32(&mut count)?;
            self.civilians.clear();
            for _ in 0..count {
                self.civilians.push(CivilianProfile::default());
            }
        }
        for p in self.civilians.iter_mut() {
            p.load_legacy_cpf(file)?;
        }
        Ok(())
    }

    /// Serialize all character profiles.
    pub fn load_legacy_cpf_characters(&mut self, file: &mut SbFile) -> Result<(), i32> {
        if file.is_write_mode() {
            let mut count = self.characters.len() as u32;
            file.serialize_u32(&mut count)?;
        } else {
            let mut count = 0u32;
            file.serialize_u32(&mut count)?;
            self.characters.clear();
            for _ in 0..count {
                self.characters.push(CharacterProfile::default());
            }
        }
        let n = self.characters.len();
        for i in 0..n {
            let mut ch = std::mem::take(&mut self.characters[i]);
            ch.load_legacy_cpf(file, i as u32)?;
            self.characters[i] = ch;
        }
        Ok(())
    }

    /// Serialize all soldier profiles.
    pub fn load_legacy_cpf_soldiers(&mut self, file: &mut SbFile) -> Result<(), i32> {
        if file.is_write_mode() {
            let mut count = self.soldiers.len() as u32;
            file.serialize_u32(&mut count)?;
        } else {
            let mut count = 0u32;
            file.serialize_u32(&mut count)?;
            self.soldiers.clear();
            for _ in 0..count {
                self.soldiers.push(SoldierProfile::default());
            }
        }
        for p in self.soldiers.iter_mut() {
            p.load_legacy_cpf(file)?;
        }
        Ok(())
    }

    /// Serialize all hand-to-hand weapon profiles.
    pub fn load_legacy_cpf_hth_weapons(&mut self, file: &mut SbFile) -> Result<(), i32> {
        if file.is_write_mode() {
            let mut count = self.hth_weapons.len() as u32;
            file.serialize_u32(&mut count)?;
        } else {
            let mut count = 0u32;
            file.serialize_u32(&mut count)?;
            self.hth_weapons.clear();
            for _ in 0..count {
                self.hth_weapons.push(HtHWeaponProfile::default());
            }
        }
        let mut clamps = (0u32, 0u32);
        for p in self.hth_weapons.iter_mut() {
            p.load_legacy_cpf(file, &mut clamps)?;
        }
        if file.is_read_mode() && (clamps.0 > 0 || clamps.1 > 0) {
            // Known quirk: shipped CPF data has garbage `kind`/`direction`
            // bytes that the original loader silently accepted. The clamp
            // is behaviorally identical for every caller of the strike
            // kind / direction getters.
            tracing::warn!(
                "HtH weapons: clamped {} invalid thrust kind(s) and {} invalid thrust direction(s) across {} weapon(s) (shipped data quirk, benign)",
                clamps.0,
                clamps.1,
                self.hth_weapons.len()
            );
        }
        Ok(())
    }

    /// Serialize all bow/shooting weapon profiles.
    pub fn load_legacy_cpf_bows(&mut self, file: &mut SbFile) -> Result<(), i32> {
        if file.is_write_mode() {
            let mut count = self.bows.len() as u32;
            file.serialize_u32(&mut count)?;
        } else {
            let mut count = 0u32;
            file.serialize_u32(&mut count)?;
            self.bows.clear();
            for _ in 0..count {
                self.bows.push(BowProfile::default());
            }
        }
        for p in self.bows.iter_mut() {
            p.load_legacy_cpf(file)?;
        }
        Ok(())
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

    /// Serialize all profiles. Order is fixed by the on-disk format:
    /// weapons, bows, characters, soldiers, missions, civilians.
    pub fn load_all_legacy_cpf(&mut self, file: &mut SbFile) -> Result<(), i32> {
        self.load_legacy_cpf_hth_weapons(file)?;
        self.load_legacy_cpf_bows(file)?;
        self.load_legacy_cpf_characters(file)?;
        self.load_legacy_cpf_soldiers(file)?;
        self.load_legacy_cpf_missions(file)?;
        self.load_legacy_cpf_civilians(file)?;
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
