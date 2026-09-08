//! Player-character discriminant.
//!
//! The game has exactly 10 concrete playable characters: the 7 hero
//! profiles (Robin Hood / Robin Town share every resource except the
//! localized-name string id) plus 3 merry-man templates.
//!
//! Caches (portraits, action buttons, fighting overlays, localized names)
//! key on this enum, and the engine pre-resolves the variant once at
//! level-load instead of re-matching strings.
//!
//! The peasants use the `MerryMan{A,B,C}` variant names — resource
//! identifiers spell them `RHID_PORTRAIT_MERRYMEN_{A,B,C}` etc.  (The
//! *internal* French profile-name is "Paysan A/B/C"; "MerryMan" matches
//! the external-facing resource convention.)
//!
//! Robin Hood's forest and town variants share every resource except
//! their localized-name string id (144 vs 145).  `RobinHood { is_town }`
//! carries that one bit of state so the localized-name lookup can
//! distinguish them while every other lookup treats them identically.

use serde::{Deserialize, Serialize};

use crate::resource_ids;

/// One of the 10 concrete playable characters.
///
/// `from_profile` parses the engine's stable profile filename plus
/// French profile-name strings; `profile_names` returns the canonical
/// French name for a variant.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum CharacterKind {
    /// Robin Hood — forest (`"Robin des bois"`, is_town=false) and town
    /// (`"Robin des villes"`, is_town=true) variants.  Share every
    /// resource except the localized-name string id.
    RobinHood { is_town: bool },
    /// Little John — `"Petit Jean"`.
    LittleJohn,
    /// Friar Tuck — `"Frere Tuck"`.
    FriarTuck,
    /// Stutely — `"Stutely"`.
    Stutely,
    /// Will Scarlet — `"Will Ecarlate"`.
    WillScarlet,
    /// Lady Marianne — `"Lady Marianne"`.
    LadyMarianne,
    /// Merry-man slot A — internal `"Paysan A"`, resources spelled `MERRYMEN_A`.
    MerryManA,
    /// Merry-man slot B — internal `"Paysan B"`, resources spelled `MERRYMEN_B`.
    MerryManB,
    /// Merry-man slot C — internal `"Paysan C"`, resources spelled `MERRYMEN_C`.
    MerryManC,
}

/// Resource identifier type, matching the `i32` constants in
/// [`crate::resource_ids`] and the host's `robin_assets::resource_manager::ResourceId`.
pub type ResourceId = i32;

impl CharacterKind {
    /// Number of distinct cache slots — includes both Robin forest and
    /// town as separate indices because their localized names differ.
    pub const COUNT: usize = 10;

    /// The canonical iteration order — one entry per cache slot.  Both
    /// `RobinHood { is_town: false }` and `RobinHood { is_town: true }`
    /// appear because their `as_index()` values differ.
    pub const VARIANTS: &'static [CharacterKind; Self::COUNT] = &[
        CharacterKind::RobinHood { is_town: false },
        CharacterKind::RobinHood { is_town: true },
        CharacterKind::LittleJohn,
        CharacterKind::FriarTuck,
        CharacterKind::Stutely,
        CharacterKind::WillScarlet,
        CharacterKind::LadyMarianne,
        CharacterKind::MerryManA,
        CharacterKind::MerryManB,
        CharacterKind::MerryManC,
    ];

    /// Parse a stable profile filename plus French profile-name string
    /// into a variant.  Some full-game profile tables give Robin Hood
    /// and Robin Town the same localized profile name; the filename is
    /// the reliable discriminator used by the original engine.
    pub fn from_profile(filename: &str, name: &str) -> Option<Self> {
        match filename {
            "RobinHood" => Some(CharacterKind::RobinHood { is_town: false }),
            "RobinTown" => Some(CharacterKind::RobinHood { is_town: true }),
            _ => Self::from_profile_name(name),
        }
    }

    /// Parse a French profile-name string into a variant.  Accepts both
    /// Robin Hood forms; returns `None` for anything else.
    pub fn from_profile_name(name: &str) -> Option<Self> {
        Some(match name {
            "Robin des bois" => CharacterKind::RobinHood { is_town: false },
            "Robin des villes" => CharacterKind::RobinHood { is_town: true },
            "Petit Jean" => CharacterKind::LittleJohn,
            "Frere Tuck" => CharacterKind::FriarTuck,
            "Stutely" => CharacterKind::Stutely,
            "Will Ecarlate" => CharacterKind::WillScarlet,
            "Lady Marianne" => CharacterKind::LadyMarianne,
            "Paysan A" => CharacterKind::MerryManA,
            "Paysan B" => CharacterKind::MerryManB,
            "Paysan C" => CharacterKind::MerryManC,
            _ => return None,
        })
    }

    /// Map an uppercase PC-initial character (`'R'` / `'J'` / …) used by
    /// the `stringPCs` / `LUKAS` cheat tables to a variant.
    ///
    /// `'R'` always resolves to the forest Robin — the cheat table makes
    /// no distinction.
    pub fn from_pc_initial(c: char) -> Option<Self> {
        Some(match c.to_ascii_uppercase() {
            'R' => CharacterKind::RobinHood { is_town: false },
            'J' => CharacterKind::LittleJohn,
            'T' => CharacterKind::FriarTuck,
            'S' => CharacterKind::Stutely,
            'W' => CharacterKind::WillScarlet,
            'M' => CharacterKind::LadyMarianne,
            'A' => CharacterKind::MerryManA,
            'B' => CharacterKind::MerryManB,
            'C' => CharacterKind::MerryManC,
            _ => return None,
        })
    }

    /// The canonical French profile-name for this variant — the exact
    /// string the level data / campaign profile tables use.
    pub fn profile_name(&self) -> &'static str {
        match self {
            CharacterKind::RobinHood { is_town: false } => "Robin des bois",
            CharacterKind::RobinHood { is_town: true } => "Robin des villes",
            CharacterKind::LittleJohn => "Petit Jean",
            CharacterKind::FriarTuck => "Frere Tuck",
            CharacterKind::Stutely => "Stutely",
            CharacterKind::WillScarlet => "Will Ecarlate",
            CharacterKind::LadyMarianne => "Lady Marianne",
            CharacterKind::MerryManA => "Paysan A",
            CharacterKind::MerryManB => "Paysan B",
            CharacterKind::MerryManC => "Paysan C",
        }
    }

    /// Dense cache index in `[0, COUNT)` for this variant.  Both Robin
    /// Hood forest and town have distinct indices so the localized-name
    /// cache can hold separate entries for them.
    pub fn as_index(&self) -> usize {
        match self {
            CharacterKind::RobinHood { is_town: false } => 0,
            CharacterKind::RobinHood { is_town: true } => 1,
            CharacterKind::LittleJohn => 2,
            CharacterKind::FriarTuck => 3,
            CharacterKind::Stutely => 4,
            CharacterKind::WillScarlet => 5,
            CharacterKind::LadyMarianne => 6,
            CharacterKind::MerryManA => 7,
            CharacterKind::MerryManB => 8,
            CharacterKind::MerryManC => 9,
        }
    }

    /// True for either Robin Hood variant.
    pub fn is_robin(&self) -> bool {
        matches!(self, CharacterKind::RobinHood { .. })
    }

    /// Portrait bitmap resource id.
    pub fn portrait_resource(&self) -> ResourceId {
        match self {
            CharacterKind::RobinHood { .. } => resource_ids::RHID_PORTRAIT_ROBIN_HOOD,
            CharacterKind::LittleJohn => resource_ids::RHID_PORTRAIT_LITTLE_JOHN,
            CharacterKind::FriarTuck => resource_ids::RHID_PORTRAIT_FRIAR_TUCK,
            CharacterKind::Stutely => resource_ids::RHID_PORTRAIT_STUTELEY,
            CharacterKind::WillScarlet => resource_ids::RHID_PORTRAIT_WILL_SCARLET,
            CharacterKind::LadyMarianne => resource_ids::RHID_PORTRAIT_LADY_MARIAN,
            CharacterKind::MerryManA => resource_ids::RHID_PORTRAIT_MERRYMEN_A,
            CharacterKind::MerryManB => resource_ids::RHID_PORTRAIT_MERRYMEN_B,
            CharacterKind::MerryManC => resource_ids::RHID_PORTRAIT_MERRYMEN_C,
        }
    }

    /// Fighting-overlay bitmap resource id.
    pub fn fighting_resource(&self) -> ResourceId {
        match self {
            CharacterKind::RobinHood { .. } => resource_ids::RHID_FIGHTING_ROBIN,
            CharacterKind::LittleJohn => resource_ids::RHID_FIGHTING_JOHN,
            CharacterKind::FriarTuck => resource_ids::RHID_FIGHTING_TUCK,
            CharacterKind::Stutely => resource_ids::RHID_FIGHTING_STUTELEY,
            CharacterKind::WillScarlet => resource_ids::RHID_FIGHTING_SCARLET,
            CharacterKind::LadyMarianne => resource_ids::RHID_FIGHTING_MARIAN,
            CharacterKind::MerryManA => resource_ids::RHID_FIGHTING_MERRYMEN_A,
            CharacterKind::MerryManB => resource_ids::RHID_FIGHTING_MERRYMEN_B,
            CharacterKind::MerryManC => resource_ids::RHID_FIGHTING_MERRYMEN_C,
        }
    }

    /// Action-button bitmap resource ids `[action1, action2, action3]`.
    /// Peasants have only 2 actions; their third slot is `None`.
    pub fn action_resources(&self) -> [Option<ResourceId>; 3] {
        match self {
            CharacterKind::RobinHood { .. } => [
                Some(resource_ids::RHID_RH_ACTION_1),
                Some(resource_ids::RHID_RH_ACTION_2),
                Some(resource_ids::RHID_RH_ACTION_3),
            ],
            CharacterKind::LittleJohn => [
                Some(resource_ids::RHID_LJ_ACTION_1),
                Some(resource_ids::RHID_LJ_ACTION_2),
                Some(resource_ids::RHID_LJ_ACTION_3),
            ],
            CharacterKind::FriarTuck => [
                Some(resource_ids::RHID_FT_ACTION_1),
                Some(resource_ids::RHID_FT_ACTION_2),
                Some(resource_ids::RHID_FT_ACTION_3),
            ],
            CharacterKind::LadyMarianne => [
                Some(resource_ids::RHID_LM_ACTION_1),
                Some(resource_ids::RHID_LM_ACTION_2),
                Some(resource_ids::RHID_LM_ACTION_3),
            ],
            CharacterKind::Stutely => [
                Some(resource_ids::RHID_ST_ACTION_1),
                Some(resource_ids::RHID_ST_ACTION_2),
                Some(resource_ids::RHID_ST_ACTION_3),
            ],
            CharacterKind::WillScarlet => [
                Some(resource_ids::RHID_WS_ACTION_1),
                Some(resource_ids::RHID_WS_ACTION_2),
                Some(resource_ids::RHID_WS_ACTION_3),
            ],
            CharacterKind::MerryManA => [
                Some(resource_ids::RHID_MA_ACTION_1),
                Some(resource_ids::RHID_MA_ACTION_2),
                None,
            ],
            CharacterKind::MerryManB => [
                Some(resource_ids::RHID_MB_ACTION_1),
                Some(resource_ids::RHID_MB_ACTION_2),
                None,
            ],
            CharacterKind::MerryManC => [
                Some(resource_ids::RHID_MC_ACTION_1),
                Some(resource_ids::RHID_MC_ACTION_2),
                None,
            ],
        }
    }

    /// Sub-id in the menu-text table for the localized hero name, or
    /// `None` for the three merry men (random peasant names are
    /// generated at runtime).
    ///
    /// Robin Hood forest and town have distinct sub-ids (144 vs 145).
    pub fn localized_name_string_id(&self) -> Option<usize> {
        Some(match self {
            CharacterKind::RobinHood { is_town: false } => 144, // ROBIN_HOOD + 0
            CharacterKind::RobinHood { is_town: true } => 145,  // ROBIN_TOWN (+1)
            CharacterKind::WillScarlet => 146,                  // WILL_SCARLET (+2)
            CharacterKind::LittleJohn => 147,                   // LITTLE_JOHN (+3)
            CharacterKind::FriarTuck => 148,                    // FRIAR_TUCK (+4)
            CharacterKind::LadyMarianne => 149,                 // LADY_MARIANNE (+5)
            CharacterKind::Stutely => 150,                      // STUTELY (+6)
            CharacterKind::MerryManA | CharacterKind::MerryManB | CharacterKind::MerryManC => {
                return None;
            }
        })
    }

    /// Sub-picture index within `RHID_REQUIRED_PC` for a required
    /// character slot on the mission requirements bar.  Peasants use
    /// the `UnknownPC=0` fallback — the widget only visualises heroes.
    ///
    /// Sub-id assignments:
    /// `UnknownPC=0, RobinHood=1, Stutley=2, Scarlet=3,
    /// LittleJohn=4, FriarTuck=5, Marian=6.`
    pub fn required_pc_sub_id(&self) -> usize {
        match self {
            CharacterKind::RobinHood { .. } => 1,
            CharacterKind::Stutely => 2,
            CharacterKind::WillScarlet => 3,
            CharacterKind::LittleJohn => 4,
            CharacterKind::FriarTuck => 5,
            CharacterKind::LadyMarianne => 6,
            CharacterKind::MerryManA | CharacterKind::MerryManB | CharacterKind::MerryManC => 0,
        }
    }

    /// Sub-picture index within `RHID_OPTIONAL_PC` for an optional team
    /// slot.  Unoccupied slots use `EmptyOption=0` via `Option::None`.
    ///
    /// Sub-id assignments:
    /// `EmptyOption=0, LittleJohnOption=1, LadyMarianOption=2,
    /// PeasantAOption=3, PeasantBOption=4, PeasantCOption=5,
    /// RobinHoodOption=6, WillScarletOption=7, StutleyOption=8,
    /// FriarTuckOption=9.`
    pub fn optional_pc_sub_id(slot: Option<Self>) -> usize {
        match slot {
            Some(CharacterKind::LittleJohn) => 1,
            Some(CharacterKind::LadyMarianne) => 2,
            Some(CharacterKind::MerryManA) => 3,
            Some(CharacterKind::MerryManB) => 4,
            Some(CharacterKind::MerryManC) => 5,
            Some(CharacterKind::RobinHood { .. }) => 6,
            Some(CharacterKind::WillScarlet) => 7,
            Some(CharacterKind::Stutely) => 8,
            Some(CharacterKind::FriarTuck) => 9,
            None => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_profile_name_round_trip() {
        for variant in CharacterKind::VARIANTS {
            let name = variant.profile_name();
            assert_eq!(CharacterKind::from_profile_name(name), Some(*variant));
        }
    }

    #[test]
    fn from_profile_name_handles_both_robin_forms() {
        assert_eq!(
            CharacterKind::from_profile_name("Robin des bois"),
            Some(CharacterKind::RobinHood { is_town: false }),
        );
        assert_eq!(
            CharacterKind::from_profile_name("Robin des villes"),
            Some(CharacterKind::RobinHood { is_town: true }),
        );
    }

    #[test]
    fn from_profile_uses_robin_filename_when_names_are_ambiguous() {
        assert_eq!(
            CharacterKind::from_profile("RobinHood", "Robin des bois"),
            Some(CharacterKind::RobinHood { is_town: false }),
        );
        assert_eq!(
            CharacterKind::from_profile("RobinTown", "Robin des bois"),
            Some(CharacterKind::RobinHood { is_town: true }),
        );
        assert_eq!(
            CharacterKind::from_profile("LittleJohn", "Petit Jean"),
            Some(CharacterKind::LittleJohn),
        );
    }

    #[test]
    fn from_profile_name_rejects_unknown() {
        assert_eq!(CharacterKind::from_profile_name("Nobody"), None);
        assert_eq!(CharacterKind::from_profile_name(""), None);
    }

    #[test]
    fn variants_have_dense_indices() {
        for (i, v) in CharacterKind::VARIANTS.iter().enumerate() {
            assert_eq!(v.as_index(), i);
        }
    }

    #[test]
    fn from_pc_initial_table() {
        assert_eq!(
            CharacterKind::from_pc_initial('R'),
            Some(CharacterKind::RobinHood { is_town: false }),
        );
        assert_eq!(
            CharacterKind::from_pc_initial('j'),
            Some(CharacterKind::LittleJohn),
        );
        assert_eq!(
            CharacterKind::from_pc_initial('C'),
            Some(CharacterKind::MerryManC),
        );
        assert_eq!(CharacterKind::from_pc_initial('X'), None);
    }

    #[test]
    fn portrait_resource_ids() {
        use CharacterKind::*;
        assert_eq!(RobinHood { is_town: false }.portrait_resource(), 71);
        assert_eq!(RobinHood { is_town: true }.portrait_resource(), 71);
        assert_eq!(LittleJohn.portrait_resource(), 42);
        assert_eq!(FriarTuck.portrait_resource(), 72);
        assert_eq!(LadyMarianne.portrait_resource(), 73);
        assert_eq!(Stutely.portrait_resource(), 74);
        assert_eq!(WillScarlet.portrait_resource(), 75);
        assert_eq!(MerryManA.portrait_resource(), 113);
        assert_eq!(MerryManB.portrait_resource(), 114);
        assert_eq!(MerryManC.portrait_resource(), 115);
    }

    #[test]
    fn action_resources_for_peasants_have_two_slots() {
        let a = CharacterKind::MerryManA.action_resources();
        assert_eq!(a, [Some(192), Some(193), None]);
        let robin = CharacterKind::RobinHood { is_town: false }.action_resources();
        assert_eq!(robin, [Some(76), Some(77), Some(78)]);
    }

    #[test]
    fn localized_name_string_ids() {
        use CharacterKind::*;
        assert_eq!(
            RobinHood { is_town: false }.localized_name_string_id(),
            Some(144)
        );
        assert_eq!(
            RobinHood { is_town: true }.localized_name_string_id(),
            Some(145)
        );
        assert_eq!(WillScarlet.localized_name_string_id(), Some(146));
        assert_eq!(LittleJohn.localized_name_string_id(), Some(147));
        assert_eq!(FriarTuck.localized_name_string_id(), Some(148));
        assert_eq!(LadyMarianne.localized_name_string_id(), Some(149));
        assert_eq!(Stutely.localized_name_string_id(), Some(150));
        assert_eq!(MerryManA.localized_name_string_id(), None);
    }

    #[test]
    fn required_pc_sub_ids() {
        use CharacterKind::*;
        assert_eq!(RobinHood { is_town: false }.required_pc_sub_id(), 1);
        assert_eq!(RobinHood { is_town: true }.required_pc_sub_id(), 1);
        assert_eq!(Stutely.required_pc_sub_id(), 2);
        assert_eq!(WillScarlet.required_pc_sub_id(), 3);
        assert_eq!(LittleJohn.required_pc_sub_id(), 4);
        assert_eq!(FriarTuck.required_pc_sub_id(), 5);
        assert_eq!(LadyMarianne.required_pc_sub_id(), 6);
        assert_eq!(MerryManA.required_pc_sub_id(), 0);
    }

    #[test]
    fn optional_pc_sub_ids() {
        use CharacterKind::*;
        assert_eq!(CharacterKind::optional_pc_sub_id(None), 0);
        assert_eq!(CharacterKind::optional_pc_sub_id(Some(LittleJohn)), 1);
        assert_eq!(CharacterKind::optional_pc_sub_id(Some(LadyMarianne)), 2);
        assert_eq!(CharacterKind::optional_pc_sub_id(Some(MerryManA)), 3);
        assert_eq!(CharacterKind::optional_pc_sub_id(Some(MerryManB)), 4);
        assert_eq!(CharacterKind::optional_pc_sub_id(Some(MerryManC)), 5);
        assert_eq!(
            CharacterKind::optional_pc_sub_id(Some(RobinHood { is_town: false })),
            6
        );
        assert_eq!(
            CharacterKind::optional_pc_sub_id(Some(RobinHood { is_town: true })),
            6
        );
        assert_eq!(CharacterKind::optional_pc_sub_id(Some(WillScarlet)), 7);
        assert_eq!(CharacterKind::optional_pc_sub_id(Some(Stutely)), 8);
        assert_eq!(CharacterKind::optional_pc_sub_id(Some(FriarTuck)), 9);
    }

    #[test]
    fn is_robin_covers_both_subvariants() {
        assert!(CharacterKind::RobinHood { is_town: false }.is_robin());
        assert!(CharacterKind::RobinHood { is_town: true }.is_robin());
        assert!(!CharacterKind::LittleJohn.is_robin());
        assert!(!CharacterKind::MerryManA.is_robin());
    }
}
