//! Sherwood Forest campaign statistics report.
//!
//! A stateless presenter that assembles debriefing text from
//! production-sector state and player-profile data.
//!
//! Text is resolved through [`MenuTextLookup`], which the host implements
//! on top of the loaded campaign menu-text table.  Every user-visible
//! phrase the report emits has a menu-text id — including the
//! "Sherwood report:" header, the "with" / "led by" connectives, the
//! training / healing verbs, the "max reached" suffix, and the score /
//! preserved-lives / play-time `printf`-style format strings.  The host's
//! English fallback table provides equivalent templates when the
//! localized `.sxt` is missing.

use crate::campaign::PcDescription;
use crate::profiles::ProfileManager;
use crate::sector_production::{SectorProduction, Type};
use serde::{Deserialize, Serialize};

// ─── Menu-text ids consumed here ───────────────────────────────────

pub const MT_STR_SCORE: usize = 64;
/// Hand-to-hand training format string (`%u` count).
pub const MT_STR_DB_S01: usize = 68;
/// Bow training format string (`%u` count).
pub const MT_STR_DB_S02: usize = 69;
/// Healing format string (`%u` count).
pub const MT_STR_DB_S03: usize = 70;
/// "(max reached)" suffix (no args).
pub const MT_STR_DB_S04: usize = 71;
/// "Sherwood report:" header (no args).
pub const MT_STR_DB_S05: usize = 72;
/// Score format string (`%u` score).
pub const MT_STR_DB_S12: usize = 79;
/// Preserved-lives format string (`%u` count).
pub const MT_STR_DB_S14: usize = 81;
/// Play-time format string (`%s` "HH:MM:SS").
pub const MT_STR_DB_S16: usize = 83;
/// "with" connective word (no args).
pub const MT_STR_DB_C01: usize = 86;
/// "led by %s" specialist suffix (`%s` name).
pub const MT_STR_DB_C02: usize = 87;
pub const MT_STR_DB_BONUS_ARROW: usize = 89;
pub const MT_STR_DB_BONUS_APPLE: usize = 90;
pub const MT_STR_DB_BONUS_WASP_NEST: usize = 91;
pub const MT_STR_DB_BONUS_LAMB_LEGG: usize = 92;
pub const MT_STR_DB_BONUS_PLANTS: usize = 93;
pub const MT_STR_DB_BONUS_STONE: usize = 94;
pub const MT_STR_DB_BONUS_ALE: usize = 95;
pub const MT_STR_DB_BONUS_NET: usize = 96;
pub const MT_STR_DB_BONUS_PURSE: usize = 97;
pub const MT_STR_DB_MERRYMAN: usize = 98;
pub const MT_STR_DB_MERRYMEN: usize = 99;
pub const MT_STR_PRESERVED_LIFES: usize = 243;
pub const MT_STR_PLAYING_TIME: usize = 258;
pub const MT_WORD_NOTHING: usize = 329;

// Rust-port extension strings. They intentionally live outside the original
// Start.sxt range: hosts can supply localized overrides while legacy tables
// fall through to the registered English strings.
pub const MT_STR_PRODUCTION_FORECAST_SELECTED: usize = 1_000;
pub const MT_STR_PRODUCTION_FORECAST_RATE: usize = 1_001;
pub const MT_STR_PRODUCTION_FORECAST_LINE: usize = 1_002;
pub const MT_STR_PRODUCTION_FORECAST_SPECIALIST: usize = 1_003;

// ─── Host-supplied menu-text lookup ────────────────────────────────

/// Abstract lookup over the campaign menu-text table.
///
/// Takes an id and returns the localized string. The host implements
/// this on top of the loaded `.sxt` resource table; the engine stays
/// free of I/O.
pub trait MenuTextLookup {
    /// Look up the string for the given menu text id. Must return the
    /// English fallback (or the id's canonical meaning) when the
    /// localized table is absent — callers do not handle `None`.
    fn get(&self, id: usize) -> String;
}

// ─── Sector types the report iterates over ────────────────────────

const PRODUCTION_TYPES: &[Type] = &[
    Type::MakeArrow,
    Type::MakePurse,
    Type::MakeStone,
    Type::MakeApple,
    Type::MakeAle,
    Type::MakeLamblegg,
    Type::MakePlant,
    Type::MakeNet,
    Type::MakeWaspNest,
];

const TRAIN_HEAL_TYPES: &[Type] = &[Type::TrainBow, Type::TrainHandToHand, Type::Heal];

// ─── Specialist mapping ────────────────────────────────────────────

/// Returns the expected specialist's profile name for a sector type.
/// The names are the French internal profile names; comparison is
/// case-insensitive.
pub fn specialist_name_for_type(t: Type) -> Option<&'static str> {
    match t {
        Type::MakeArrow | Type::MakePurse | Type::TrainBow => Some("Robin des bois"),
        Type::MakeStone => Some("Will Ecarlate"),
        Type::MakeAle | Type::MakeLamblegg | Type::Heal | Type::MakeWaspNest => Some("Frere Tuck"),
        Type::MakePlant => Some("Lady Marianne"),
        Type::MakeNet | Type::MakeApple => Some("Stutely"),
        Type::TrainHandToHand => Some("Petit Jean"),
        Type::Relic | Type::Unknown => None,
    }
}

/// Look up the specialist among a sector's occupants.
/// Returns the specialist's name if the expected character is present.
///
/// The match must follow the `PcDescription → CharacterProfile`
/// indirection and compare against `profile_name` case-insensitively —
/// `PcStatus::name` (the display field) is left empty by
/// `PcStatus::from_profile` and would never match.
fn find_specialist<'a>(
    sector: &SectorProduction,
    characters: &[PcDescription],
    profiles: &'a ProfileManager,
) -> Option<&'a str> {
    let expected = specialist_name_for_type(sector.prod_type)?;
    for occ in &sector.occupants {
        let ch = characters.get(occ.pc_description_idx)?;
        let profile_idx = ch.character_profile_idx?;
        let profile = profiles.get_character(profile_idx)?;
        if profile.profile_name.eq_ignore_ascii_case(expected) {
            return Some(&profile.profile_name);
        }
    }
    None
}

/// Menu-text id for the item name of a production sector type.
pub fn bonus_text_id(t: Type) -> usize {
    match t {
        Type::MakeArrow => MT_STR_DB_BONUS_ARROW,
        Type::MakePurse => MT_STR_DB_BONUS_PURSE,
        Type::MakeStone => MT_STR_DB_BONUS_STONE,
        Type::MakeApple => MT_STR_DB_BONUS_APPLE,
        Type::MakeAle => MT_STR_DB_BONUS_ALE,
        Type::MakeLamblegg => MT_STR_DB_BONUS_LAMB_LEGG,
        Type::MakePlant => MT_STR_DB_BONUS_PLANTS,
        Type::MakeNet => MT_STR_DB_BONUS_NET,
        Type::MakeWaspNest => MT_STR_DB_BONUS_WASP_NEST,
        other => panic!("bonus_text_id called with non-production type {other:?}"),
    }
}

// ─── Time formatting ───────────────────────────────────────────────

/// Convert seconds to "HH:MM". Drops the seconds component.
fn seconds_to_time(total: u32) -> String {
    let h = total / 3600;
    let m = (total % 3600) / 60;
    format!("{h:02}:{m:02}")
}

// ─── Printf-style substitution ─────────────────────────────────────

/// Substitute successive `values` for the printf placeholders in
/// `template`.  Handles `%u` / `%lu` / `%i` / `%d` (integer) and
/// `%s` / `%ls` (string) — the specs that appear in the campaign
/// text table templates we consume.
///
/// Extra values are dropped; missing placeholders leave the literal
/// `%spec` in place.
fn substitute_printf(template: &str, values: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut idx = 0;
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut spec = String::new();
        if let Some(&'l') = chars.peek() {
            spec.push('l');
            chars.next();
        }
        match chars.peek() {
            Some(&'u') | Some(&'i') | Some(&'d') | Some(&'s') => {
                spec.push(*chars.peek().unwrap());
                chars.next();
                if idx < values.len() {
                    out.push_str(values[idx]);
                    idx += 1;
                } else {
                    out.push('%');
                    out.push_str(&spec);
                }
            }
            _ => {
                out.push('%');
                out.push_str(&spec);
            }
        }
    }
    out
}

// ─── SherwoodStat ──────────────────────────────────────────────────

/// Sherwood Forest debriefing statistics.
///
/// A pure presenter with no data members — required data is passed
/// explicitly to [`SherwoodStat::get_text`].
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
pub struct SherwoodStat;

/// Player-profile data needed by the score section of the report.
/// Separates the data dependency so callers don't need a full profile type.
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
pub struct ScoreInfo {
    pub score: i32,
    pub preserved_lives: i32,
    pub play_time_seconds: u32,
}

/// Selected production cycle displayed by the forecast panel.
///
/// `None` duration means no mission has been committed yet; the presenter
/// then uses an exact one-hour rate instead of guessing which mission the
/// player intends to choose.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ProductionForecastCycle {
    pub mission_name: Option<String>,
    pub duration_seconds: Option<u16>,
}

impl SherwoodStat {
    /// Assemble the full Sherwood debriefing text.
    ///
    /// * `sectors` — all production sectors (iterated in order).
    /// * `characters` — campaign character pool (for occupant/specialist lookup).
    /// * `score_info` — score, preserved lives, play time.
    /// * `menu_text` — host-side text table for localized item names etc.
    pub fn get_text(
        &self,
        sectors: &[SectorProduction],
        characters: &[PcDescription],
        profiles: &ProfileManager,
        score_info: &ScoreInfo,
        menu_text: &dyn MenuTextLookup,
    ) -> String {
        self.get_text_with_forecast(sectors, characters, profiles, score_info, menu_text, None)
    }

    /// Assemble the Sherwood report and, when requested, a compact live item
    /// production panel. Forecast arithmetic remains owned by
    /// [`SectorProduction::forecast`].
    pub fn get_text_with_forecast(
        &self,
        sectors: &[SectorProduction],
        characters: &[PcDescription],
        profiles: &ProfileManager,
        score_info: &ScoreInfo,
        menu_text: &dyn MenuTextLookup,
        forecast_cycle: Option<&ProductionForecastCycle>,
    ) -> String {
        let mut text = String::new();

        text.push_str(&menu_text.get(MT_STR_DB_S05));

        let mut production = String::new();
        for sector in sectors {
            if PRODUCTION_TYPES.contains(&sector.prod_type) {
                Self::append_production_text(
                    sector,
                    characters,
                    profiles,
                    menu_text,
                    &mut production,
                );
            } else if TRAIN_HEAL_TYPES.contains(&sector.prod_type) {
                Self::append_train_heal_text(
                    sector,
                    characters,
                    profiles,
                    menu_text,
                    &mut production,
                );
            }
        }

        if production.is_empty() {
            text.push(' ');
            text.push_str(&menu_text.get(MT_WORD_NOTHING));
            text.push('\n');
        } else {
            text.push('\n');
            text.push_str(&production);
        }
        if let Some(cycle) = forecast_cycle {
            text.push('\n');
            Self::append_forecast_text(sectors, characters, profiles, cycle, menu_text, &mut text);
        }
        text.push('\n');

        Self::append_score_text(score_info, menu_text, &mut text);
        text
    }

    /// Append the item-only production projection. The original campaign has
    /// no raw-material inventory, which is reported explicitly rather than
    /// inventing hidden recipe requirements.
    fn append_forecast_text(
        sectors: &[SectorProduction],
        characters: &[PcDescription],
        profiles: &ProfileManager,
        cycle: &ProductionForecastCycle,
        menu_text: &dyn MenuTextLookup,
        out: &mut String,
    ) {
        let (cycle_seconds, heading) = match cycle.duration_seconds {
            Some(seconds) => {
                let mission_name = cycle
                    .mission_name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .unwrap_or("?");
                let template = menu_text.get(MT_STR_PRODUCTION_FORECAST_SELECTED);
                (
                    seconds,
                    substitute_printf(&template, &[mission_name, &seconds_to_time(seconds.into())]),
                )
            }
            None => {
                let seconds = 3_600;
                let template = menu_text.get(MT_STR_PRODUCTION_FORECAST_RATE);
                (
                    seconds,
                    substitute_printf(&template, &[&seconds_to_time(seconds.into())]),
                )
            }
        };
        out.push_str(&heading);
        out.push('\n');

        for sector in sectors
            .iter()
            .filter(|sector| PRODUCTION_TYPES.contains(&sector.prod_type))
        {
            let specialist = find_specialist(sector, characters, profiles).is_some();
            let forecast = sector.forecast(cycle_seconds, specialist);
            let item = menu_text.get(bonus_text_id(sector.prod_type));
            let specialist_suffix = if specialist {
                menu_text.get(MT_STR_PRODUCTION_FORECAST_SPECIALIST)
            } else {
                String::new()
            };
            let template = menu_text.get(MT_STR_PRODUCTION_FORECAST_LINE);
            let values = [
                item,
                forecast.current_stock.to_string(),
                forecast.storage_capacity.to_string(),
                forecast.produced.to_string(),
                forecast.overflow.to_string(),
                forecast.worker_count.to_string(),
                specialist_suffix,
                forecast.speed.to_string(),
                forecast.raw_materials_required.to_string(),
            ];
            let refs: Vec<&str> = values.iter().map(String::as_str).collect();
            out.push_str(&substitute_printf(&template, &refs));
            out.push('\n');
        }
    }

    /// Format the item-production line for a single sector.
    /// Returns the item name and quantity string.
    fn get_ammo_text(sector: &SectorProduction, menu_text: &dyn MenuTextLookup) -> String {
        let item = menu_text.get(bonus_text_id(sector.prod_type));
        format!("{} {item}", sector.get_produced_amount())
    }

    /// Append production text for one sector if it produced anything.
    fn append_production_text(
        sector: &SectorProduction,
        characters: &[PcDescription],
        profiles: &ProfileManager,
        menu_text: &dyn MenuTextLookup,
        out: &mut String,
    ) {
        if sector.get_produced_amount() == 0 {
            return;
        }

        let occupant_count = sector.get_occupant_count();
        let specialist = find_specialist(sector, characters, profiles);

        // Ammo part.
        out.push_str(&Self::get_ammo_text(sector, menu_text));

        // Merrym(a|e)n part.  Format is " {with} {count} {merryman|merrymen}":
        // the leading space and the count number are literal; only
        // "with" and the noun come from the text table.
        if occupant_count >= 1 {
            let noun_id = if occupant_count == 1 {
                MT_STR_DB_MERRYMAN
            } else {
                MT_STR_DB_MERRYMEN
            };
            out.push_str(&format!(
                " {} {} {}",
                menu_text.get(MT_STR_DB_C01),
                occupant_count,
                menu_text.get(noun_id),
            ));
        }

        // Specialist part (only shown if specialist present and more than one occupant).
        // The C02 menu text is a printf format string with the
        // specialist name as `%s` — preserves localized punctuation.
        if let Some(name) = specialist
            && occupant_count > 1
        {
            let template = menu_text.get(MT_STR_DB_C02);
            out.push_str(&substitute_printf(&template, &[name]));
        }

        // Max amount reached.
        if sector.is_max_reached() {
            out.push(' ');
            out.push_str(&menu_text.get(MT_STR_DB_S04));
        }

        out.push_str(".\n");
    }

    /// Append training / healing text for one sector if it has occupants.
    fn append_train_heal_text(
        sector: &SectorProduction,
        characters: &[PcDescription],
        profiles: &ProfileManager,
        menu_text: &dyn MenuTextLookup,
        out: &mut String,
    ) {
        let occupant_count = sector.get_occupant_count();
        if occupant_count == 0 {
            return;
        }

        let specialist = find_specialist(sector, characters, profiles);

        // The specialist doesn't count toward the trained/healed total.
        let trained_count = if specialist.is_some() {
            occupant_count.saturating_sub(1)
        } else {
            occupant_count
        };

        // S01/S02/S03 menu texts are printf format strings taking the
        // trained/healed count as `%u`.
        let verb_id = match sector.prod_type {
            Type::TrainBow => MT_STR_DB_S02,
            Type::TrainHandToHand => MT_STR_DB_S01,
            Type::Heal => MT_STR_DB_S03,
            other => panic!("append_train_heal_text called with wrong type {other:?}"),
        };
        let template = menu_text.get(verb_id);
        out.push_str(&substitute_printf(&template, &[&trained_count.to_string()]));

        // Specialist part — C02 menu text is a printf format string
        // with the specialist name as `%s`, prefixed by a space.
        if let Some(name) = specialist {
            out.push(' ');
            let c02 = menu_text.get(MT_STR_DB_C02);
            out.push_str(&substitute_printf(&c02, &[name]));
        }

        out.push_str(".\n");
    }

    /// Append score, preserved lives, and play time.
    ///
    /// S12/S14/S16 menu texts are printf format strings taking the
    /// score / preserved-lives count / play-time string as the sole
    /// substitution argument.
    fn append_score_text(info: &ScoreInfo, menu_text: &dyn MenuTextLookup, out: &mut String) {
        let score_tpl = menu_text.get(MT_STR_DB_S12);
        out.push_str(&substitute_printf(&score_tpl, &[&info.score.to_string()]));
        out.push('\n');

        let lives_tpl = menu_text.get(MT_STR_DB_S14);
        out.push_str(&substitute_printf(
            &lives_tpl,
            &[&info.preserved_lives.to_string()],
        ));
        out.push('\n');

        let time_tpl = menu_text.get(MT_STR_DB_S16);
        out.push_str(&substitute_printf(
            &time_tpl,
            &[&seconds_to_time(info.play_time_seconds)],
        ));
        out.push('\n');
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pc_status::PcStatus;
    use crate::profiles::{CharacterProfile, CharacterProfileIdx, ProfileManager};
    use std::collections::HashMap;

    /// Test stub for [`MenuTextLookup`]. Returns the English fallbacks
    /// that would normally come from the campaign `.sxt` table.
    struct FakeMenuText {
        map: HashMap<usize, &'static str>,
    }

    impl FakeMenuText {
        fn english() -> Self {
            let mut map = HashMap::new();
            map.insert(MT_STR_SCORE, "Score");
            map.insert(MT_STR_DB_S01, "%u trained in hand-to-hand");
            map.insert(MT_STR_DB_S02, "%u trained in archery");
            map.insert(MT_STR_DB_S03, "%u healed");
            map.insert(MT_STR_DB_S04, "(max reached)");
            map.insert(MT_STR_DB_S05, "Sherwood report:");
            map.insert(MT_STR_DB_S12, "Score: %u");
            map.insert(MT_STR_DB_S14, "Preserved lives: %u");
            map.insert(MT_STR_DB_S16, "Play time: %s");
            map.insert(MT_STR_DB_C01, "with");
            map.insert(MT_STR_DB_C02, "led by %s");
            map.insert(
                MT_STR_PRODUCTION_FORECAST_SELECTED,
                "Next production — %s (%s cycle):",
            );
            map.insert(
                MT_STR_PRODUCTION_FORECAST_RATE,
                "Production rate (%s; select mission):",
            );
            map.insert(
                MT_STR_PRODUCTION_FORECAST_LINE,
                "%s: stock %u/%u; +%u; overflow %u; input %u workers%s at speed %u; raw materials %u.",
            );
            map.insert(MT_STR_PRODUCTION_FORECAST_SPECIALIST, " + specialist");
            map.insert(MT_STR_DB_BONUS_ARROW, "arrows");
            map.insert(MT_STR_DB_BONUS_APPLE, "apples");
            map.insert(MT_STR_DB_BONUS_WASP_NEST, "wasp nests");
            map.insert(MT_STR_DB_BONUS_LAMB_LEGG, "lamb legs");
            map.insert(MT_STR_DB_BONUS_PLANTS, "plants");
            map.insert(MT_STR_DB_BONUS_STONE, "stones");
            map.insert(MT_STR_DB_BONUS_ALE, "ales");
            map.insert(MT_STR_DB_BONUS_NET, "nets");
            map.insert(MT_STR_DB_BONUS_PURSE, "purses");
            map.insert(MT_STR_DB_MERRYMAN, "merryman");
            map.insert(MT_STR_DB_MERRYMEN, "merrymen");
            map.insert(MT_STR_PRESERVED_LIFES, "Preserved lives");
            map.insert(MT_STR_PLAYING_TIME, "Play time");
            map.insert(MT_WORD_NOTHING, "nothing");
            Self { map }
        }
    }

    impl MenuTextLookup for FakeMenuText {
        fn get(&self, id: usize) -> String {
            self.map.get(&id).copied().unwrap_or("").to_string()
        }
    }

    fn make_sector(prod_type: Type, produced: u16, occupants: Vec<usize>) -> SectorProduction {
        use crate::sector_production::Occupant;
        SectorProduction {
            prod_type,
            produced_amount: produced,
            occupants: occupants
                .into_iter()
                .map(|idx| Occupant {
                    pc_description_idx: idx,
                    x: 0.0,
                    y: 0.0,
                    obstacle: None,
                })
                .collect(),
            ..Default::default()
        }
    }

    /// Build a `(characters, profiles)` pair where each `PcDescription`
    /// is wired to a `CharacterProfile` whose `profile_name` matches the
    /// supplied name.  `find_specialist` consults `profile_name`, so
    /// tests must thread both halves rather than just `PcStatus::name`.
    fn make_party(names: &[&str]) -> (Vec<PcDescription>, ProfileManager) {
        let mut profiles = ProfileManager::default();
        let mut characters = Vec::with_capacity(names.len());
        for (i, name) in names.iter().enumerate() {
            profiles.characters.push(CharacterProfile {
                index: i as u32,
                profile_name: (*name).to_string(),
                ..Default::default()
            });
            characters.push(PcDescription {
                character_profile_idx: Some(CharacterProfileIdx(i as u32)),
                status: PcStatus {
                    name: (*name).to_string(),
                    ..Default::default()
                },
                ..Default::default()
            });
        }
        (characters, profiles)
    }

    #[test]
    fn test_seconds_to_time() {
        assert_eq!(seconds_to_time(0), "00:00");
        assert_eq!(seconds_to_time(61), "00:01");
        assert_eq!(seconds_to_time(3661), "01:01");
        assert_eq!(seconds_to_time(86399), "23:59");
    }

    #[test]
    fn test_get_ammo_text() {
        let text = FakeMenuText::english();
        let s = make_sector(Type::MakeArrow, 42, vec![]);
        assert_eq!(SherwoodStat::get_ammo_text(&s, &text), "42 arrows");

        let s = make_sector(Type::MakeWaspNest, 5, vec![]);
        assert_eq!(SherwoodStat::get_ammo_text(&s, &text), "5 wasp nests");
    }

    #[test]
    fn selected_cycle_forecast_reports_live_inputs_stock_capacity_and_overflow() {
        let text = FakeMenuText::english();
        let (characters, profiles) = make_party(&["Peasant", "Robin des bois"]);
        let mut sector = make_sector(Type::MakeArrow, 0, vec![0, 1]);
        sector.speed = 100;
        sector.amount = 9;
        sector.production_points = vec![
            crate::sector_production::Point::default(),
            crate::sector_production::Point::default(),
        ];
        let mut out = String::new();
        SherwoodStat::append_forecast_text(
            &[sector],
            &characters,
            &profiles,
            &ProductionForecastCycle {
                mission_name: Some("Nottingham".to_string()),
                duration_seconds: Some(60),
            },
            &text,
            &mut out,
        );

        assert!(out.contains("Next production — Nottingham (00:01 cycle):"));
        assert!(out.contains("arrows: stock 9/10; +18; overflow 17"));
        assert!(out.contains("input 2 workers + specialist at speed 100"));
        assert!(out.contains("raw materials 0"));
    }

    #[test]
    fn unselected_cycle_reports_an_exact_hourly_rate_without_guessing_a_mission() {
        let text = FakeMenuText::english();
        let (characters, profiles) = make_party(&["Peasant"]);
        let mut sector = make_sector(Type::MakeStone, 0, vec![0]);
        sector.speed = 100;
        sector.production_points = vec![crate::sector_production::Point::default(); 100];
        let mut out = String::new();
        SherwoodStat::append_forecast_text(
            &[sector],
            &characters,
            &profiles,
            &ProductionForecastCycle::default(),
            &text,
            &mut out,
        );

        assert!(out.contains("Production rate (01:00; select mission):"));
        assert!(out.contains("stones: stock 0/500; +360; overflow 0"));
    }

    #[test]
    #[should_panic(expected = "non-production type")]
    fn test_get_ammo_text_panics_on_train_type() {
        let text = FakeMenuText::english();
        let s = make_sector(Type::TrainBow, 0, vec![]);
        SherwoodStat::get_ammo_text(&s, &text);
    }

    #[test]
    fn test_specialist_mapping() {
        assert_eq!(
            specialist_name_for_type(Type::MakeArrow),
            Some("Robin des bois")
        );
        assert_eq!(
            specialist_name_for_type(Type::MakeStone),
            Some("Will Ecarlate")
        );
        assert_eq!(specialist_name_for_type(Type::Heal), Some("Frere Tuck"));
        assert_eq!(
            specialist_name_for_type(Type::MakePlant),
            Some("Lady Marianne")
        );
        assert_eq!(specialist_name_for_type(Type::MakeNet), Some("Stutely"));
        assert_eq!(
            specialist_name_for_type(Type::TrainHandToHand),
            Some("Petit Jean")
        );
        assert_eq!(specialist_name_for_type(Type::Unknown), None);
    }

    #[test]
    fn test_find_specialist_present() {
        let (chars, profiles) = make_party(&["Nobody", "Robin des bois"]);
        let s = make_sector(Type::MakeArrow, 10, vec![0, 1]);
        assert_eq!(
            find_specialist(&s, &chars, &profiles),
            Some("Robin des bois")
        );
    }

    #[test]
    fn test_find_specialist_absent() {
        let (chars, profiles) = make_party(&["Nobody"]);
        let s = make_sector(Type::MakeArrow, 10, vec![0]);
        assert_eq!(find_specialist(&s, &chars, &profiles), None);
    }

    #[test]
    fn test_production_text_with_specialist() {
        let text = FakeMenuText::english();
        let (chars, profiles) = make_party(&["Peasant", "Robin des bois"]);
        let s = make_sector(Type::MakeArrow, 15, vec![0, 1]);
        let mut out = String::new();
        SherwoodStat::append_production_text(&s, &chars, &profiles, &text, &mut out);
        assert!(out.contains("15 arrows"));
        assert!(out.contains("2 merrymen"));
        assert!(out.contains("led by Robin des bois"));
        assert!(out.ends_with(".\n"));
    }

    #[test]
    fn test_production_text_zero_produced() {
        let text = FakeMenuText::english();
        let s = make_sector(Type::MakeArrow, 0, vec![0]);
        let (chars, profiles) = make_party(&["Robin des bois"]);
        let mut out = String::new();
        SherwoodStat::append_production_text(&s, &chars, &profiles, &text, &mut out);
        assert!(
            out.is_empty(),
            "Should produce no text when produced_amount is 0"
        );
    }

    #[test]
    fn test_production_text_max_reached() {
        let text = FakeMenuText::english();
        let mut s = make_sector(Type::MakeStone, 99, vec![0]);
        s.max_amount_reached = true;
        let (chars, profiles) = make_party(&["Peasant"]);
        let mut out = String::new();
        SherwoodStat::append_production_text(&s, &chars, &profiles, &text, &mut out);
        assert!(out.contains("(max reached)"));
    }

    #[test]
    fn test_production_text_single_merryman() {
        let text = FakeMenuText::english();
        let s = make_sector(Type::MakeAle, 3, vec![0]);
        let (chars, profiles) = make_party(&["Peasant"]);
        let mut out = String::new();
        SherwoodStat::append_production_text(&s, &chars, &profiles, &text, &mut out);
        assert!(out.contains("1 merryman"));
        assert!(!out.contains("merrymen"));
    }

    #[test]
    fn test_production_text_specialist_hidden_with_single_occupant() {
        // Specialist is only shown when occupant_count > 1.
        let text = FakeMenuText::english();
        let (chars, profiles) = make_party(&["Robin des bois"]);
        let s = make_sector(Type::MakeArrow, 10, vec![0]);
        let mut out = String::new();
        SherwoodStat::append_production_text(&s, &chars, &profiles, &text, &mut out);
        assert!(
            !out.contains("led by"),
            "Specialist hidden when only 1 occupant"
        );
    }

    #[test]
    fn test_train_heal_text_with_specialist() {
        let text = FakeMenuText::english();
        let (chars, profiles) = make_party(&["Peasant A", "Peasant B", "Robin des bois"]);
        let s = make_sector(Type::TrainBow, 0, vec![0, 1, 2]);
        let mut out = String::new();
        SherwoodStat::append_train_heal_text(&s, &chars, &profiles, &text, &mut out);
        // 3 occupants, specialist present → trained_count = 2
        assert!(out.contains("2 trained in archery"));
        assert!(out.contains("led by Robin des bois"));
    }

    #[test]
    fn test_train_heal_text_no_specialist() {
        let text = FakeMenuText::english();
        let (chars, profiles) = make_party(&["Peasant A", "Peasant B"]);
        let s = make_sector(Type::Heal, 0, vec![0, 1]);
        let mut out = String::new();
        SherwoodStat::append_train_heal_text(&s, &chars, &profiles, &text, &mut out);
        assert!(out.contains("2 healed"));
        assert!(!out.contains("led by"));
    }

    #[test]
    fn test_train_heal_text_empty_sector() {
        let text = FakeMenuText::english();
        let s = make_sector(Type::TrainHandToHand, 0, vec![]);
        let profiles = ProfileManager::default();
        let mut out = String::new();
        SherwoodStat::append_train_heal_text(&s, &[], &profiles, &text, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn test_score_text() {
        let text = FakeMenuText::english();
        let info = ScoreInfo {
            score: 1500,
            preserved_lives: 7,
            play_time_seconds: 7261,
        };
        let mut out = String::new();
        SherwoodStat::append_score_text(&info, &text, &mut out);
        assert!(out.contains("Score: 1500"));
        assert!(out.contains("Preserved lives: 7"));
        assert!(out.contains("Play time: 02:01"));
    }

    #[test]
    fn test_get_text_no_production() {
        let text = FakeMenuText::english();
        let stat = SherwoodStat;
        let sectors = vec![
            make_sector(Type::MakeArrow, 0, vec![]),
            make_sector(Type::TrainBow, 0, vec![]),
        ];
        let info = ScoreInfo {
            score: 100,
            preserved_lives: 3,
            play_time_seconds: 60,
        };
        let profiles = ProfileManager::default();
        let assembled = stat.get_text(&sectors, &[], &profiles, &info, &text);
        assert!(assembled.contains("nothing"));
        assert!(assembled.contains("Score: 100"));
    }

    #[test]
    fn test_get_text_with_production() {
        let text = FakeMenuText::english();
        let (chars, profiles) = make_party(&["Peasant", "Robin des bois"]);
        let sectors = vec![
            make_sector(Type::MakeArrow, 20, vec![0, 1]),
            make_sector(Type::TrainBow, 0, vec![0, 1]),
        ];
        let info = ScoreInfo {
            score: 500,
            preserved_lives: 5,
            play_time_seconds: 3600,
        };
        let stat = SherwoodStat;
        let assembled = stat.get_text(&sectors, &chars, &profiles, &info, &text);
        assert!(assembled.starts_with("Sherwood report:"));
        assert!(assembled.contains("20 arrows"));
        assert!(assembled.contains("1 trained in archery"));
        assert!(assembled.contains("Score: 500"));
        assert!(!assembled.contains("nothing"));
    }

    #[test]
    fn test_serde_roundtrip() {
        let stat = SherwoodStat;
        let json = serde_json::to_string(&stat).unwrap();
        let _: SherwoodStat = serde_json::from_str(&json).unwrap();

        let info = ScoreInfo {
            score: 42,
            preserved_lives: 3,
            play_time_seconds: 999,
        };
        let json = serde_json::to_string(&info).unwrap();
        let info2: ScoreInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info2.score, 42);
    }
}
