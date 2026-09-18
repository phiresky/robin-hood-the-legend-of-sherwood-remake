//! Port-owned Feature 40 catalogue.
//!
//! Retail resource tables cannot provide strings for a feature added by this
//! port. Keep one complete catalogue for every locale the shipping-datadir
//! importer accepts, as one embedded `data/feature40/<locale>.toml` file per
//! locale keyed by `PortTextKey` variant name. Parsing rejects missing and
//! unknown keys; tests in `localization.rs` enforce full coverage and
//! identical named-placeholder contracts in every language.

use std::collections::HashMap;
use std::sync::LazyLock;

use super::{FEATURE40_PORT_TEXT_KEYS, PortTextKey, normalize_locale};

pub(super) fn text(locale: &str, key: PortTextKey) -> Option<&'static str> {
    let index = FEATURE40_PORT_TEXT_KEYS
        .iter()
        .position(|candidate| *candidate == key)?;
    let normalized = normalize_locale(locale);
    let table = match normalized.as_str() {
        "en" | "en-us" | "und" => "en",
        "de" | "de-de" => "de",
        "fr" | "fr-fr" => "fr",
        "it" | "it-it" => "it",
        "pt" | "pt-pt" => "pt",
        "pt-br" => "pt-br",
        "es" | "es-es" => "es",
        "ru" | "ru-ru" => "ru",
        "ja" | "ja-jp" => "ja",
        "cs" | "cs-cz" => "cs",
        "pl" | "pl-pl" => "pl",
        "zh" | "zh-tw" => "zh-tw",
        "zh-cn" => "zh-cn",
        "ko" | "ko-kr" => "ko",
        "th" | "th-th" => "th",
        _ => return None,
    };
    let (_, texts) = TABLES
        .iter()
        .find(|(name, _)| *name == table)
        .expect("every matched locale has an embedded Feature 40 table");
    Some(&texts[index])
}

macro_rules! sources {
    ($($locale:literal),* $(,)?) => {
        [$(($locale, include_str!(concat!("data/feature40/", $locale, ".toml")))),*]
    };
}

/// Embedded `(table name, TOML source)` pairs.
const SOURCES: [(&str, &str); 15] = sources![
    "en", "de", "fr", "it", "pt", "pt-br", "es", "ru", "ja", "cs", "pl", "zh-tw", "zh-cn", "ko",
    "th",
];

/// Per-locale texts in `FEATURE40_PORT_TEXT_KEYS` order.
static TABLES: LazyLock<Vec<(&'static str, Vec<String>)>> = LazyLock::new(|| {
    SOURCES
        .iter()
        .map(|(locale, source)| (*locale, parse_table(locale, source)))
        .collect()
});

fn parse_table(locale: &str, source: &str) -> Vec<String> {
    let mut rows: HashMap<String, String> = toml::from_str(source)
        .unwrap_or_else(|error| panic!("malformed Feature 40 table `{locale}`: {error}"));
    let texts = FEATURE40_PORT_TEXT_KEYS
        .iter()
        .map(|key| {
            rows.remove(&format!("{key:?}"))
                .unwrap_or_else(|| panic!("Feature 40 table `{locale}` is missing {key:?}"))
        })
        .collect();
    assert!(
        rows.is_empty(),
        "Feature 40 table `{locale}` has unknown keys: {:?}",
        rows.keys().collect::<Vec<_>>()
    );
    texts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_table_is_complete() {
        assert_eq!(TABLES.len(), SOURCES.len());
        for (locale, texts) in TABLES.iter() {
            assert_eq!(texts.len(), FEATURE40_PORT_TEXT_KEYS.len(), "{locale}");
            for &key in FEATURE40_PORT_TEXT_KEYS {
                assert!(!text(locale, key).expect("complete table").is_empty());
            }
        }
    }
}
