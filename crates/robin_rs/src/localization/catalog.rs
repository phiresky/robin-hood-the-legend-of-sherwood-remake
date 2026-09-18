//! Single lookup boundary for port-owned text. Missing locales explicitly use English.
//!
//! The strings live in the embedded `data/catalog.toml`; tables there are
//! keyed by `PortTextKey`, `GameplaySetting` and `RelativeTimeUnit` variant
//! names. Parsing rejects incomplete or unknown rows.
use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

use super::{PortTextKey, RelativeTimeUnit, feature40_text, locale_primary};
use crate::gameplay_settings::GameplaySetting;

pub(super) fn text(locale: Option<&str>, key: PortTextKey) -> &'static str {
    let language = locale
        .map(locale_primary)
        .unwrap_or(std::borrow::Cow::Borrowed("en"));
    if let PortTextKey::SaveRelativeTime {
        unit,
        future,
        singular,
    } = key
    {
        // Relative-time translations must supply whole phrases and their own
        // plural selection. Until available, fall back as a complete English
        // message rather than combining localized affixes with English units.
        let row = &CATALOG
            .save_relative_time
            .iter()
            .find(|row| row.0 == unit)
            .expect("every relative-time unit has English templates")
            .1;
        return match (future, singular) {
            (false, true) => &row.past_singular,
            (false, false) => &row.past_plural,
            (true, true) => &row.future_singular,
            (true, false) => &row.future_plural,
        };
    }
    if let PortTextKey::GameplayLabel(setting) | PortTextKey::GameplayTooltip(setting) = key {
        let row = &CATALOG
            .gameplay
            .iter()
            .find(|row| row.0 == setting)
            .expect("every gameplay setting has catalogue text")
            .1;
        let (label, help) = if language == "de" {
            (&row.de_label, &row.de_tooltip)
        } else {
            (&row.en_label, &row.en_tooltip)
        };
        return if matches!(key, PortTextKey::GameplayLabel(_)) {
            label
        } else {
            help
        };
    }
    if let Some(text) = feature40_text::text(locale.unwrap_or("en-US"), key)
        .or_else(|| feature40_text::text("en-US", key))
    {
        return text;
    }
    let row = CATALOG
        .basic
        .get(&format!("{key:?}"))
        .expect("every port-owned key has English catalogue text");
    row.get(language.as_ref()).unwrap_or(&row["en"])
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GameplayRow {
    en_label: String,
    en_tooltip: String,
    de_label: String,
    de_tooltip: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelativeTimeRow {
    past_singular: String,
    past_plural: String,
    future_singular: String,
    future_plural: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogSource {
    /// `PortTextKey` variant name -> language -> text.
    basic: HashMap<String, HashMap<String, String>>,
    gameplay: HashMap<String, GameplayRow>,
    save_relative_time: HashMap<String, RelativeTimeRow>,
}

struct Catalog {
    basic: HashMap<String, HashMap<String, String>>,
    gameplay: Vec<(GameplaySetting, GameplayRow)>,
    save_relative_time: Vec<(RelativeTimeUnit, RelativeTimeRow)>,
}

const RELATIVE_TIME_UNITS: [RelativeTimeUnit; 7] = [
    RelativeTimeUnit::Second,
    RelativeTimeUnit::Minute,
    RelativeTimeUnit::Hour,
    RelativeTimeUnit::Day,
    RelativeTimeUnit::Week,
    RelativeTimeUnit::Month,
    RelativeTimeUnit::Year,
];

/// Moves one row per expected variant out of `rows`; missing and leftover
/// rows are malformed data.
fn take_rows<K: Copy + std::fmt::Debug, V>(
    table: &str,
    keys: &[K],
    mut rows: HashMap<String, V>,
) -> Vec<(K, V)> {
    let taken = keys
        .iter()
        .map(|key| {
            let row = rows
                .remove(&format!("{key:?}"))
                .unwrap_or_else(|| panic!("catalogue table `{table}` is missing {key:?}"));
            (*key, row)
        })
        .collect();
    assert!(
        rows.is_empty(),
        "catalogue table `{table}` has unknown rows: {:?}",
        rows.keys().collect::<Vec<_>>()
    );
    taken
}

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    let source: CatalogSource = toml::from_str(include_str!("data/catalog.toml"))
        .unwrap_or_else(|error| panic!("malformed port text catalogue: {error}"));
    // TODO: `PortTextKey` has no variant list, so a misspelt `basic` key is
    // only caught by `basic_keys_keep_every_language`, not at parse time.
    for (key, row) in &source.basic {
        assert!(
            row.contains_key("en"),
            "catalogue key {key} has no English text"
        );
        for (language, value) in row {
            assert!(
                !value.is_empty(),
                "catalogue key {key} has empty {language}"
            );
        }
    }
    Catalog {
        basic: source.basic,
        gameplay: take_rows("gameplay", &GameplaySetting::ALL, source.gameplay),
        save_relative_time: take_rows(
            "save_relative_time",
            &RELATIVE_TIME_UNITS,
            source.save_relative_time,
        ),
    }
});

#[test]
fn gameplay_keys_have_explicit_fallbacks() {
    for setting in GameplaySetting::ALL {
        for key in [
            PortTextKey::GameplayLabel(setting),
            PortTextKey::GameplayTooltip(setting),
        ] {
            assert!(!text(Some("de-DE"), key).is_empty());
            assert_eq!(text(Some("untranslated"), key), text(Some("en"), key));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Languages each basic key carried when the tables moved to data.
    const BASIC_LANGUAGES: &[(&str, &[&str])] = &[
        ("SaveNewSaveLabel", &["en"]),
        ("SaveNewSaveHint", &["en"]),
        ("SaveMission", &["en"]),
        ("SavePlayer", &["en"]),
        ("SaveSaved", &["en"]),
        ("SaveExactDate", &["en"]),
        ("SaveCampaignProgress", &["en"]),
        ("SaveMissions", &["en"]),
        ("SaveGangSize", &["en"]),
        ("SaveRansom", &["en"]),
        ("SaveBlazons", &["en"]),
        ("SaveAmulets", &["en"]),
        ("SaveLegacyValueUnavailable", &["en"]),
        ("SaveInvalidTimestamp", &["en"]),
        ("SaveRelativeTimeUnavailable", &["en"]),
        ("SaveLocalTimeUnavailable", &["en"]),
        ("SaveJustNow", &["en"]),
        ("SaveCompactSaved", &["en"]),
        ("SaveCompactCampaignProgress", &["en"]),
        ("SaveCompactMissions", &["en"]),
        ("SaveCompactGangSize", &["en"]),
        ("SaveCompactRansom", &["en"]),
        ("SaveCompactBlazons", &["en"]),
        ("SaveCompactAmulets", &["en"]),
        ("CampaignClassicMap", &["de", "en"]),
        ("CampaignProgressTree", &["de", "en"]),
        ("CampaignSherwoodMuseum", &["de", "en"]),
        ("GameAutosaved", &["de", "en"]),
        ("AutosaveFailed", &["de", "en"]),
        ("SaveFailed", &["de", "en"]),
        (
            "Language",
            &[
                "de", "fr", "it", "pt", "es", "ru", "ja", "cs", "pl", "zh", "ko", "th", "en",
            ],
        ),
        (
            "Automatic",
            &[
                "de", "fr", "it", "pt", "es", "ru", "ja", "cs", "pl", "zh", "ko", "th", "en",
            ],
        ),
        (
            "Apply",
            &[
                "de", "fr", "it", "pt", "es", "ru", "ja", "cs", "pl", "zh", "ko", "th", "en",
            ],
        ),
        ("InstalledLanguages", &["en"]),
        ("OptionalEnglishFallback", &["en"]),
    ];

    #[test]
    fn basic_keys_keep_every_language() {
        assert_eq!(CATALOG.basic.len(), BASIC_LANGUAGES.len());
        for (key, languages) in BASIC_LANGUAGES {
            let row = CATALOG
                .basic
                .get(*key)
                .unwrap_or_else(|| panic!("catalogue lost {key}"));
            assert_eq!(row.len(), languages.len(), "{key}");
            for language in *languages {
                assert!(row.contains_key(*language), "{key} lost {language}");
            }
        }
    }

    #[test]
    fn parsed_text_matches_former_constants() {
        use crate::gameplay_settings::GameplaySetting;
        let samples = [
            (Some("th"), PortTextKey::Automatic, "อัตโนมัติ"),
            (Some("ru"), PortTextKey::Automatic, "Автоматически"),
            (Some("zh"), PortTextKey::Automatic, "自動"),
            (Some("pt"), PortTextKey::Automatic, "Automático"),
            (
                Some("de-DE"),
                PortTextKey::SaveFailed,
                "Speichern fehlgeschlagen – vor erneutem Versuch das Protokoll prüfen.",
            ),
            (
                Some("fr"),
                PortTextKey::SaveCampaignProgress,
                "Campaign: {progress}%",
            ),
            (None, PortTextKey::SaveNewSaveLabel, "< New Save >"),
            (
                Some("de"),
                PortTextKey::GameplayTooltip(GameplaySetting::FixHardReactionTimes),
                "Den vorgesehenen Reaktionszeitfaktor für Schwer verwenden.",
            ),
            (
                Some("ja"),
                PortTextKey::GameplayLabel(GameplaySetting::FogOfWar),
                "Fog of War",
            ),
            (
                Some("de"),
                PortTextKey::SaveRelativeTime {
                    unit: RelativeTimeUnit::Week,
                    future: true,
                    singular: false,
                },
                "in {value} weeks",
            ),
            (
                Some("en-US"),
                PortTextKey::SpellforgeHostAttestation,
                "Host {host} attests permission to redistribute `{title}` from {source} for this multiplayer session",
            ),
            (
                Some("en-US"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[Mod] {title} - {version}",
            ),
            (
                Some("de-DE"),
                PortTextKey::SpellforgeHostAttestation,
                "Host {host} bestätigt die Erlaubnis, `{title}` aus {source} für diese Mehrspielersitzung weiterzuverteilen",
            ),
            (
                Some("de-DE"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[Mod] {title} - {version}",
            ),
            (
                Some("ja"),
                PortTextKey::SpellforgeHostAttestation,
                "ホスト {host} は、このマルチプレイセッションで {source} の `{title}` を再配布する権限があると表明します",
            ),
            (
                Some("ja"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[Mod] {title} - {version}",
            ),
            (
                Some("ru-RU"),
                PortTextKey::SpellforgeHostAttestation,
                "Хост {host} подтверждает право распространять `{title}` из {source} в этом сетевом сеансе",
            ),
            (
                Some("ru-RU"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[Мод] {title} - {version}",
            ),
            (
                Some("th"),
                PortTextKey::SpellforgeHostAttestation,
                "โฮสต์ {host} รับรองว่ามีสิทธิ์แจกจ่าย `{title}` จาก {source} ซ้ำสำหรับเซสชันผู้เล่นหลายคนนี้",
            ),
            (
                Some("th"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[ม็อด] {title} - {version}",
            ),
            (
                Some("zh"),
                PortTextKey::SpellforgeHostAttestation,
                "主機 {host} 聲明已獲准在此多人連線中，從 {source} 再散布 `{title}`",
            ),
            (
                Some("zh"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[模組] {title} - {version}",
            ),
            (
                Some("zh-CN"),
                PortTextKey::SpellforgeHostAttestation,
                "主机 {host} 声明已获准在此多人会话中从 {source} 再分发 `{title}`",
            ),
            (
                Some("zh-CN"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[模组] {title} - {version}",
            ),
            (
                Some("pt-BR"),
                PortTextKey::SpellforgeHostAttestation,
                "O host {host} declara ter permissão para redistribuir `{title}` de {source} nesta sessão multijogador",
            ),
            (
                Some("pt-BR"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[Mod] {title} - {version}",
            ),
            (
                Some("ko"),
                PortTextKey::SpellforgeHostAttestation,
                "호스트 {host}은(는) 이 멀티플레이 세션에서 {source}의 `{title}`을(를) 재배포할 권한이 있음을 확인합니다",
            ),
            (
                Some("ko"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[모드] {title} - {version}",
            ),
            (
                Some("cs"),
                PortTextKey::SpellforgeHostAttestation,
                "Hostitel {host} potvrzuje oprávnění redistribuovat `{title}` ze zdroje {source} pro tuto hru více hráčů",
            ),
            (
                Some("cs"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[Mod] {title} - {version}",
            ),
            (
                Some("pl"),
                PortTextKey::SpellforgeHostAttestation,
                "Host {host} potwierdza prawo do rozpowszechniania `{title}` ze źródła {source} w tej sesji wieloosobowej",
            ),
            (
                Some("pl"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[Mod] {title} - {version}",
            ),
            (
                Some("fr"),
                PortTextKey::SpellforgeHostAttestation,
                "L'hôte {host} atteste être autorisé à redistribuer `{title}` depuis {source} pour cette session multijoueur",
            ),
            (
                Some("fr"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[Mod] {title} - {version}",
            ),
            (
                Some("it"),
                PortTextKey::SpellforgeHostAttestation,
                "L'host {host} attesta il permesso di ridistribuire `{title}` da {source} per questa sessione multigiocatore",
            ),
            (
                Some("it"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[Mod] {title} - {version}",
            ),
            (
                Some("es"),
                PortTextKey::SpellforgeHostAttestation,
                "El anfitrión {host} certifica que tiene permiso para redistribuir `{title}` desde {source} en esta sesión multijugador",
            ),
            (
                Some("es"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[Mod] {title} - {version}",
            ),
            (
                Some("pt"),
                PortTextKey::SpellforgeHostAttestation,
                "O anfitrião {host} atesta ter permissão para redistribuir `{title}` de {source} nesta sessão multijogador",
            ),
            (
                Some("pt"),
                PortTextKey::SpellforgeMpModMissionLabel,
                "[Mod] {title} - {version}",
            ),
        ];
        for (locale, key, expected) in samples {
            assert_eq!(text(locale, key), expected, "{locale:?} {key:?}");
        }
    }
}
