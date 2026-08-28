//! Application-owned runtime language selection.
//!
//! The retail game chose the first locale directory it found during startup.
//! The Rust port keeps that compatibility data format, but makes the chosen
//! locale explicit and replaceable.  Only validated packs are exposed to the
//! options UI; changing language is a host-side presentation operation and is
//! deliberately absent from simulation saves, hashes, replays, and network
//! commands.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use robin_assets::resource_manager::ResourceManager;
use robin_assets::shipping_datadir::{ShippingDatadir, ShippingLocale};
use robin_engine::sbfile::SbFile;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(any(not(target_arch = "wasm32"), test))]
const PREFERENCES_FILE: &str = "language.json";
const BROWSER_PREFERENCES_KEY: &str = "robin_hood.language.v1";
const MENU_TEXT_TABLES: [i32; 3] = [1_000_507, 1_000_040, 1_000_034];
const MINIMUM_CORE_MENU_STRINGS: usize = 32;
static ACTIVE_PROCESS_LOCALE: RwLock<Option<String>> = RwLock::new(None);

/// Stable, application-global language choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageSelection {
    /// Follow the operating-system/browser language when it is installed.
    Auto,
    /// A canonical BCP-47 tag (`de-DE`, `pt-BR`, ...), or `und` for the
    /// international/neutral LCID 2047 data set.
    Locale(String),
}

impl Default for LanguageSelection {
    fn default() -> Self {
        Self::Auto
    }
}

/// Non-simulation language preferences.  `show_in_options` is intentionally
/// persisted so packagers and accessibility-focused builds can disable the
/// visual selector without maintaining a source fork.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalizationPreferences {
    pub selection: LanguageSelection,
    pub show_in_options: bool,
}

impl Default for LocalizationPreferences {
    fn default() -> Self {
        Self {
            selection: LanguageSelection::Auto,
            show_in_options: true,
        }
    }
}

/// One installed and validated language pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguagePack {
    pub locale: String,
    pub native_name: String,
    /// Loose-datadir directory name.  Shipping packs do not need a native
    /// root, but retaining the canonical identity keeps both backends uniform.
    pub data_root: String,
    pub has_voice: bool,
    pub has_cinematics: bool,
    pub voice_uses_english_fallback: bool,
    pub cinematics_use_english_fallback: bool,
    /// Presentation-only mission titles keyed by stable authored mission id.
    /// Simulation profiles remain untouched during a mid-mission switch.
    #[serde(default)]
    pub mission_names: std::collections::BTreeMap<u32, String>,
}

/// Result of committing a different host language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageChange {
    pub previous_locale: Option<String>,
    pub active_locale: Option<String>,
    pub generation: u64,
}

#[derive(Debug, Error)]
pub enum LocalizationError {
    #[error("language pack {0} is not installed or did not pass validation")]
    Unavailable(String),
    #[error("language pack {locale} has no usable core menu text: {reason}")]
    InvalidCoreText { locale: String, reason: String },
    #[error("failed to read language preferences from {path}: {source}")]
    ReadPreferences {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to decode language preferences from {path}: {source}")]
    DecodePreferences {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to persist language preferences to {path}: {source}")]
    PersistPreferences {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to install language resource lookup (SBFile status {0})")]
    FileLookup(i32),
    #[error("failed to install shipping language resources: {0:#}")]
    Shipping(anyhow::Error),
    #[error(
        "language change failed ({change}); restoring the previous locale also failed ({rollback})"
    )]
    Rollback {
        change: Box<LocalizationError>,
        rollback: Box<LocalizationError>,
    },
    #[cfg(target_arch = "wasm32")]
    #[error("browser language-preference storage is unavailable: {0}")]
    BrowserStorage(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum PreferenceStore {
    Native(PathBuf),
    Browser,
    Memory,
}

/// Mutable application service shared by menus and the active mission host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalizationService {
    preferences: LocalizationPreferences,
    installed: Vec<LanguagePack>,
    active_locale: Option<String>,
    active_data_root: Option<String>,
    generation: u64,
    store: PreferenceStore,
}

#[derive(Debug, Clone, Copy)]
struct LanguageDefinition {
    locale: &'static str,
    lcid: &'static str,
    native_name: &'static str,
    aliases: &'static [&'static str],
}

const LANGUAGE_DEFINITIONS: &[LanguageDefinition] = &[
    LanguageDefinition {
        locale: "en-US",
        lcid: "1033",
        native_name: "English (US)",
        aliases: &["en-US", "en_US", "english"],
    },
    LanguageDefinition {
        locale: "de-DE",
        lcid: "1031",
        native_name: "Deutsch",
        aliases: &["de-DE", "de_DE", "german"],
    },
    LanguageDefinition {
        locale: "und",
        lcid: "2047",
        native_name: "International / Neutral",
        aliases: &["neutral", "international"],
    },
    LanguageDefinition {
        locale: "fr-FR",
        lcid: "1036",
        native_name: "Français",
        aliases: &["fr-FR", "fr_FR", "french"],
    },
    LanguageDefinition {
        locale: "it-IT",
        lcid: "1040",
        native_name: "Italiano",
        aliases: &["it-IT", "it_IT", "italian"],
    },
    LanguageDefinition {
        locale: "pt-PT",
        lcid: "2070",
        native_name: "Português (Portugal)",
        aliases: &["pt-PT", "pt_PT"],
    },
    LanguageDefinition {
        locale: "es-ES",
        lcid: "3082",
        native_name: "Español",
        aliases: &["es-ES", "es_ES", "spanish"],
    },
    LanguageDefinition {
        locale: "ru-RU",
        lcid: "1049",
        native_name: "Русский",
        aliases: &["ru-RU", "ru_RU", "russian"],
    },
    LanguageDefinition {
        locale: "ja-JP",
        lcid: "1041",
        native_name: "日本語",
        aliases: &["ja-JP", "ja_JP", "japanese"],
    },
    LanguageDefinition {
        locale: "cs-CZ",
        lcid: "1029",
        native_name: "Čeština",
        aliases: &["cs-CZ", "cs_CZ", "czech"],
    },
    LanguageDefinition {
        locale: "pl-PL",
        lcid: "1045",
        native_name: "Polski",
        aliases: &["pl-PL", "pl_PL", "polish"],
    },
    LanguageDefinition {
        locale: "pt-BR",
        lcid: "1046",
        native_name: "Português (Brasil)",
        aliases: &["pt-BR", "pt_BR"],
    },
    LanguageDefinition {
        locale: "zh-TW",
        lcid: "1028",
        native_name: "繁體中文",
        aliases: &["zh-TW", "zh_TW"],
    },
    LanguageDefinition {
        locale: "ko-KR",
        lcid: "1042",
        native_name: "한국어",
        aliases: &["ko-KR", "ko_KR", "korean"],
    },
    LanguageDefinition {
        locale: "zh-CN",
        lcid: "2052",
        native_name: "简体中文",
        aliases: &["zh-CN", "zh_CN"],
    },
    LanguageDefinition {
        locale: "th-TH",
        lcid: "1054",
        native_name: "ไทย",
        aliases: &["th-TH", "th_TH", "thai"],
    },
];

impl LocalizationService {
    /// A context-local disabled service used by tests and bootstrap contexts.
    /// It never mutates process-wide file lookup state.
    pub fn disabled() -> Self {
        Self {
            preferences: LocalizationPreferences {
                show_in_options: false,
                ..LocalizationPreferences::default()
            },
            installed: Vec::new(),
            active_locale: None,
            active_data_root: None,
            generation: 0,
            store: PreferenceStore::Memory,
        }
    }

    /// Discover installed packs, load the application-global preference, and
    /// install the selected locale before any UI resources are constructed.
    pub fn initialize(shipping: Option<&ShippingDatadir>) -> Result<Self, LocalizationError> {
        let store = default_preference_store();
        Self::initialize_with_store(shipping, store)
    }

    fn initialize_with_store(
        shipping: Option<&ShippingDatadir>,
        store: PreferenceStore,
    ) -> Result<Self, LocalizationError> {
        let preferences = load_preferences(&store)?;
        let installed = discover_installed_languages(shipping);
        let active = resolve_selection(&preferences.selection, &installed);
        let active_data_root = active.as_ref().map(|pack| pack.data_root.clone());
        install_file_lookup(active, &installed, shipping)?;
        let active_locale = active.map(|pack| pack.locale.clone());

        if installed.is_empty() {
            tracing::warn!(
                "No validated locale packs found; language switching is unavailable for this data set"
            );
        } else {
            tracing::info!(
                locale = active_locale.as_deref().unwrap_or("base"),
                packs = installed.len(),
                "Initialized runtime localization"
            );
        }

        Ok(Self {
            preferences,
            installed,
            active_locale,
            active_data_root,
            generation: 1,
            store,
        })
    }

    pub fn preferences(&self) -> &LocalizationPreferences {
        &self.preferences
    }

    pub fn installed(&self) -> &[LanguagePack] {
        &self.installed
    }

    pub fn active_locale(&self) -> Option<&str> {
        self.active_locale.as_deref()
    }

    pub fn active_data_root(&self) -> Option<&str> {
        self.active_data_root.as_deref()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn selector_visible(&self) -> bool {
        self.preferences.show_in_options && self.installed.len() > 1
    }

    /// Locale whose recorded samples define deterministic logical speech
    /// duration. This is deliberately independent of the active presentation
    /// language: multiplayer hosts publish it in the mission handshake and
    /// every peer derives timing from this exact pack while playing its own
    /// selected voice track.
    pub fn canonical_speech_timing_locale(&self) -> Option<&str> {
        self.installed
            .iter()
            .filter(|pack| pack.has_voice)
            .min_by_key(|pack| (pack.locale != "en-US", pack.locale.as_str()))
            .map(|pack| pack.locale.as_str())
    }

    /// Persist and atomically commit a new locale lookup generation.  The
    /// caller rebuilds presentation caches only after this succeeds.
    pub fn set_selection(
        &mut self,
        selection: LanguageSelection,
        shipping: Option<&ShippingDatadir>,
    ) -> Result<LanguageChange, LocalizationError> {
        let active = match &selection {
            LanguageSelection::Auto => resolve_selection(&selection, &self.installed),
            LanguageSelection::Locale(locale) => Some(
                self.installed
                    .iter()
                    .find(|pack| locale_eq(&pack.locale, locale))
                    .ok_or_else(|| LocalizationError::Unavailable(locale.clone()))?,
            ),
        };

        // Re-validate loose content at the commit boundary. A removable or
        // user-edited data directory must not leave half of the UI switched.
        if let Some(pack) = active
            && !pack.data_root.is_empty()
        {
            validate_loose_pack(&pack.locale, &pack.data_root)?;
        } else if let (Some(pack), Some(shipping)) = (active, shipping) {
            let locale = shipping
                .locale(&pack.locale)
                .map_err(LocalizationError::Shipping)?
                .ok_or_else(|| LocalizationError::Unavailable(pack.locale.clone()))?;
            validate_shipping_pack(&pack.locale, locale)?;
        }

        let next_preferences = LocalizationPreferences {
            selection,
            ..self.preferences.clone()
        };
        let previous_locale = self.active_locale.clone();
        let previous = previous_locale.as_deref().and_then(|locale| {
            self.installed
                .iter()
                .find(|pack| locale_eq(&pack.locale, locale))
        });

        if let Err(change) = install_file_lookup(active, &self.installed, shipping) {
            if let Err(rollback) = install_file_lookup(previous, &self.installed, shipping) {
                return Err(LocalizationError::Rollback {
                    change: Box::new(change),
                    rollback: Box::new(rollback),
                });
            }
            return Err(change);
        }
        if let Err(change) = persist_preferences(&self.store, &next_preferences) {
            if let Err(rollback) = install_file_lookup(previous, &self.installed, shipping) {
                return Err(LocalizationError::Rollback {
                    change: Box::new(change),
                    rollback: Box::new(rollback),
                });
            }
            return Err(change);
        }

        self.preferences = next_preferences;
        self.active_locale = active.map(|pack| pack.locale.clone());
        self.active_data_root = active.map(|pack| pack.data_root.clone());
        self.generation = self.generation.wrapping_add(1).max(1);

        Ok(LanguageChange {
            previous_locale,
            active_locale: self.active_locale.clone(),
            generation: self.generation,
        })
    }

    pub fn set_selector_visible(&mut self, visible: bool) -> Result<(), LocalizationError> {
        let next = LocalizationPreferences {
            show_in_options: visible,
            ..self.preferences.clone()
        };
        persist_preferences(&self.store, &next)?;
        self.preferences = next;
        Ok(())
    }
}

fn discover_installed_languages(shipping: Option<&ShippingDatadir>) -> Vec<LanguagePack> {
    let english_root = find_loose_root(&LANGUAGE_DEFINITIONS[0]);
    let mut packs = Vec::new();

    if let Some(shipping) = shipping {
        for (locale, assets) in shipping.available_locales() {
            match validate_shipping_pack(locale, assets) {
                Ok(()) => {
                    let definition = definition_for_locale(locale);
                    let has_voice = assets
                        .raw
                        .keys()
                        .any(|path| path == "sounds/exclamations/actors.res");
                    let has_cinematics = assets
                        .raw
                        .keys()
                        .any(|path| path.starts_with("cinematics/"));
                    packs.push(LanguagePack {
                        locale: locale.to_owned(),
                        native_name: definition
                            .map(|definition| definition.native_name)
                            .unwrap_or(locale)
                            .to_owned(),
                        // An empty root denotes the already-decoded shipping
                        // manifest. Loose roots are never empty after discovery.
                        data_root: String::new(),
                        has_voice,
                        has_cinematics,
                        voice_uses_english_fallback: false,
                        cinematics_use_english_fallback: false,
                        mission_names: assets
                            .profiles
                            .as_ref()
                            .map(mission_names_from_profiles)
                            .unwrap_or_default(),
                    });
                }
                Err(error) => {
                    tracing::warn!(locale, "Ignoring invalid shipping language pack: {error}")
                }
            }
        }
    }

    let shipping_english =
        shipping.is_some_and(|shipping| shipping.locale("en-US").ok().flatten().is_some());
    for definition in LANGUAGE_DEFINITIONS {
        if packs
            .iter()
            .any(|pack| locale_eq(&pack.locale, definition.locale))
        {
            continue;
        }
        let Some(root) = find_loose_root(definition) else {
            continue;
        };
        match validate_loose_pack(definition.locale, &root) {
            Ok(()) => {
                let has_voice = loose_path_exists(&root, "Data/Sounds/Exclamations/actors.res");
                let has_cinematics = loose_path_exists(&root, "Data/Cinematics");
                let mission_names = load_loose_mission_names(&root);
                packs.push(LanguagePack {
                    locale: definition.locale.to_owned(),
                    native_name: definition.native_name.to_owned(),
                    data_root: root,
                    has_voice,
                    has_cinematics,
                    voice_uses_english_fallback: !has_voice
                        && (english_root.is_some() || shipping_english),
                    cinematics_use_english_fallback: !has_cinematics
                        && (english_root.is_some() || shipping_english),
                    mission_names,
                });
            }
            Err(error) => tracing::warn!(
                locale = definition.locale,
                "Ignoring invalid language pack: {error}"
            ),
        }
    }
    for pack in &mut packs {
        let same_backend_english = if pack.data_root.is_empty() {
            shipping_english
        } else {
            english_root.is_some()
        };
        pack.voice_uses_english_fallback = !pack.has_voice && same_backend_english;
        pack.cinematics_use_english_fallback = !pack.has_cinematics && same_backend_english;
    }
    packs.sort_by(|left, right| left.locale.cmp(&right.locale));
    packs
}

fn mission_names_from_profiles(
    profiles: &robin_engine::profiles::ProfileManager,
) -> std::collections::BTreeMap<u32, String> {
    profiles
        .missions
        .iter()
        .filter(|mission| !mission.mission_name.trim().is_empty())
        .map(|mission| (mission.id, mission.mission_name.clone()))
        .collect()
}

fn load_loose_mission_names(root: &str) -> std::collections::BTreeMap<u32, String> {
    let path = format!("{root}/Data/Configuration/profile.cpf");
    let Ok(mut file) = SbFile::open(&path, robin_engine::sbfile::SB_FILE_READ) else {
        tracing::debug!(
            root,
            "Language pack has no localized profile.cpf mission titles"
        );
        return std::collections::BTreeMap::new();
    };
    let mut profiles = robin_engine::profiles::ProfileManager::new();
    match profiles.load_all_legacy_cpf(&mut file) {
        Ok(()) => mission_names_from_profiles(&profiles),
        Err(error) => {
            tracing::warn!(
                root,
                "Ignoring invalid localized profile.cpf titles: {error}"
            );
            std::collections::BTreeMap::new()
        }
    }
}

fn definition_for_locale(locale: &str) -> Option<&'static LanguageDefinition> {
    LANGUAGE_DEFINITIONS
        .iter()
        .find(|definition| locale_eq(definition.locale, locale))
}

fn find_loose_root(definition: &LanguageDefinition) -> Option<String> {
    std::iter::once(definition.lcid)
        .chain(std::iter::once(definition.locale))
        .chain(definition.aliases.iter().copied())
        .find(|root| {
            loose_path_exists(root, "Data/Text/Level.res")
                || loose_path_exists(root, "Data/Interface/Start.sxt")
        })
        .map(str::to_owned)
}

fn loose_path_exists(root: &str, relative: &str) -> bool {
    SbFile::exists(&format!("{root}/{relative}"))
}

fn validate_loose_pack(locale: &str, root: &str) -> Result<(), LocalizationError> {
    let mut resources = ResourceManager::new();
    let mut attached = 0usize;
    for relative in ["Data/Text/Level.res", "Data/Interface/Start.sxt"] {
        let path = format!("{root}/{relative}");
        if SbFile::exists(&path) {
            resources.attach_resource_file(&path).map_err(|error| {
                LocalizationError::InvalidCoreText {
                    locale: locale.to_owned(),
                    reason: format!("cannot parse {relative}: {error:#}"),
                }
            })?;
            attached += 1;
        }
    }
    if attached == 0 {
        return Err(LocalizationError::InvalidCoreText {
            locale: locale.to_owned(),
            reason: "neither Level.res nor Start.sxt exists".to_owned(),
        });
    }

    let usable = MENU_TEXT_TABLES.iter().any(|table| {
        resources
            .get_string_count(*table)
            .is_ok_and(|count| count >= MINIMUM_CORE_MENU_STRINGS)
    });
    if !usable {
        return Err(LocalizationError::InvalidCoreText {
            locale: locale.to_owned(),
            reason: format!(
                "none of menu tables {MENU_TEXT_TABLES:?} contains at least {MINIMUM_CORE_MENU_STRINGS} strings"
            ),
        });
    }
    Ok(())
}

fn validate_shipping_pack(locale: &str, assets: &ShippingLocale) -> Result<(), LocalizationError> {
    let usable = ["text/level.res", "interface/start.sxt"]
        .iter()
        .filter_map(|path| assets.res_files.get(*path))
        .any(|resources| {
            MENU_TEXT_TABLES.iter().any(|table| {
                resources
                    .resident_string_count(*table)
                    .is_some_and(|count| count >= MINIMUM_CORE_MENU_STRINGS)
            })
        });
    if !usable {
        return Err(LocalizationError::InvalidCoreText {
            locale: locale.to_owned(),
            reason: format!(
                "shipping pack has no menu table {MENU_TEXT_TABLES:?} with at least {MINIMUM_CORE_MENU_STRINGS} strings"
            ),
        });
    }
    Ok(())
}

fn install_file_lookup(
    active: Option<&LanguagePack>,
    installed: &[LanguagePack],
    shipping: Option<&ShippingDatadir>,
) -> Result<(), LocalizationError> {
    let selected = active
        .filter(|pack| !pack.data_root.is_empty())
        .map(|pack| pack.data_root.as_str());
    let fallback = selected.and_then(|selected| {
        installed
            .iter()
            .find(|pack| pack.locale == "en-US" && !pack.data_root.is_empty())
            .map(|pack| pack.data_root.as_str())
            .filter(|root| *root != selected)
    });
    let status = SbFile::set_locale_paths(selected, fallback);
    if status != robin_engine::sbfile::SBFILE_NO_ERROR {
        return Err(LocalizationError::FileLookup(status));
    }
    if let Some(shipping) = shipping {
        let shipping_locale = active
            .filter(|pack| pack.data_root.is_empty())
            .map(|pack| pack.locale.as_str());
        shipping
            .set_active_locale(shipping_locale)
            .map_err(LocalizationError::Shipping)?;
    } else {
        robin_util::asset_fs::install_locale_bundle(None)
            .map_err(|error| LocalizationError::Shipping(error.into()))?;
    }
    *ACTIVE_PROCESS_LOCALE
        .write()
        .expect("active process locale lock poisoned") = active.map(|pack| pack.locale.clone());
    Ok(())
}

/// The original font manager selected its TrueType family for international
/// builds whose bitmap fonts cannot cover the locale's script. Keep that
/// decision host-global alongside SbFile's active locale generation.
pub fn active_locale_prefers_truetype() -> bool {
    let locale = ACTIVE_PROCESS_LOCALE
        .read()
        .expect("active process locale lock poisoned");
    matches!(
        locale.as_deref().map(locale_primary).as_deref(),
        Some("ja" | "zh" | "ko" | "th" | "ru" | "pl" | "cs")
    )
}

fn resolve_selection<'a>(
    selection: &LanguageSelection,
    installed: &'a [LanguagePack],
) -> Option<&'a LanguagePack> {
    if installed.is_empty() {
        return None;
    }
    match selection {
        LanguageSelection::Locale(locale) => installed
            .iter()
            .find(|pack| locale_eq(&pack.locale, locale))
            .or_else(|| auto_language(installed)),
        LanguageSelection::Auto => auto_language(installed),
    }
}

fn auto_language(installed: &[LanguagePack]) -> Option<&LanguagePack> {
    let system = sys_locale::get_locale().unwrap_or_default();
    installed
        .iter()
        .find(|pack| locale_eq(&pack.locale, &system))
        .or_else(|| {
            let primary = locale_primary(&system);
            (!primary.is_empty())
                .then(|| {
                    installed
                        .iter()
                        .find(|pack| locale_primary(&pack.locale) == primary)
                })
                .flatten()
        })
        .or_else(|| installed.iter().find(|pack| pack.locale == "en-US"))
        .or_else(|| installed.first())
}

fn locale_eq(a: &str, b: &str) -> bool {
    normalize_locale(a) == normalize_locale(b)
}

fn locale_primary(locale: &str) -> String {
    normalize_locale(locale)
        .split('-')
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn normalize_locale(locale: &str) -> String {
    locale
        .split(['.', '@'])
        .next()
        .unwrap_or(locale)
        .replace('_', "-")
        .to_ascii_lowercase()
}

fn default_preference_store() -> PreferenceStore {
    #[cfg(target_arch = "wasm32")]
    {
        PreferenceStore::Browser
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        PreferenceStore::Native(crate::save_file::default_save_directory().join(PREFERENCES_FILE))
    }
}

fn load_preferences(store: &PreferenceStore) -> Result<LocalizationPreferences, LocalizationError> {
    let Some(encoded) = read_store(store)? else {
        return Ok(LocalizationPreferences::default());
    };
    serde_json::from_str(&encoded).map_err(|source| LocalizationError::DecodePreferences {
        path: store_display_path(store),
        source,
    })
}

fn persist_preferences(
    store: &PreferenceStore,
    preferences: &LocalizationPreferences,
) -> Result<(), LocalizationError> {
    let encoded = serde_json::to_string_pretty(preferences)
        .expect("LocalizationPreferences serialization cannot fail");
    match store {
        PreferenceStore::Native(path) => persist_native(path, encoded.as_bytes()),
        #[cfg(target_arch = "wasm32")]
        PreferenceStore::Browser => {
            let storage = web_sys::window()
                .ok_or_else(|| LocalizationError::BrowserStorage("window is absent".to_owned()))?
                .local_storage()
                .map_err(|error| LocalizationError::BrowserStorage(format!("{error:?}")))?
                .ok_or_else(|| {
                    LocalizationError::BrowserStorage("localStorage is disabled".to_owned())
                })?;
            storage
                .set_item(BROWSER_PREFERENCES_KEY, &encoded)
                .map_err(|error| LocalizationError::BrowserStorage(format!("{error:?}")))
        }
        #[cfg(not(target_arch = "wasm32"))]
        PreferenceStore::Browser => Ok(()),
        PreferenceStore::Memory => Ok(()),
    }
}

fn read_store(store: &PreferenceStore) -> Result<Option<String>, LocalizationError> {
    match store {
        PreferenceStore::Native(path) => match std::fs::read_to_string(path) {
            Ok(encoded) => Ok(Some(encoded)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(LocalizationError::ReadPreferences {
                path: path.clone(),
                source,
            }),
        },
        #[cfg(target_arch = "wasm32")]
        PreferenceStore::Browser => {
            let storage = web_sys::window()
                .ok_or_else(|| LocalizationError::BrowserStorage("window is absent".to_owned()))?
                .local_storage()
                .map_err(|error| LocalizationError::BrowserStorage(format!("{error:?}")))?
                .ok_or_else(|| {
                    LocalizationError::BrowserStorage("localStorage is disabled".to_owned())
                })?;
            storage
                .get_item(BROWSER_PREFERENCES_KEY)
                .map_err(|error| LocalizationError::BrowserStorage(format!("{error:?}")))
        }
        #[cfg(not(target_arch = "wasm32"))]
        PreferenceStore::Browser => Ok(None),
        PreferenceStore::Memory => Ok(None),
    }
}

fn persist_native(path: &Path, bytes: &[u8]) -> Result<(), LocalizationError> {
    use std::io::Write as _;

    let parent = path
        .parent()
        .ok_or_else(|| LocalizationError::PersistPreferences {
            path: path.to_owned(),
            source: std::io::Error::other("language preference path has no parent directory"),
        })?;
    std::fs::create_dir_all(parent).map_err(|source| LocalizationError::PersistPreferences {
        path: path.to_owned(),
        source,
    })?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".language-")
        .suffix(".json.tmp")
        .tempfile_in(parent)
        .map_err(|source| LocalizationError::PersistPreferences {
            path: path.to_owned(),
            source,
        })?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|source| LocalizationError::PersistPreferences {
            path: path.to_owned(),
            source,
        })?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| LocalizationError::PersistPreferences {
            path: path.to_owned(),
            source: error.error,
        })
}

fn store_display_path(store: &PreferenceStore) -> PathBuf {
    match store {
        PreferenceStore::Native(path) => path.clone(),
        PreferenceStore::Browser => PathBuf::from(BROWSER_PREFERENCES_KEY),
        PreferenceStore::Memory => PathBuf::from("<memory>"),
    }
}

/// Strings introduced by the port cannot rely on unused numeric slots in the
/// retail resource tables. Keep the small language-selector vocabulary in a
/// stable keyed catalogue alongside the locale service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortTextKey {
    Language,
    Automatic,
    Apply,
    InstalledLanguages,
    OptionalEnglishFallback,
}

pub fn port_text(locale: Option<&str>, key: PortTextKey) -> &'static str {
    let primary = locale
        .map(locale_primary)
        .unwrap_or_else(|| "en".to_owned());
    match (primary.as_str(), key) {
        ("de", PortTextKey::Language) => "Sprache",
        ("de", PortTextKey::Automatic) => "Automatisch",
        ("de", PortTextKey::Apply) => "Anwenden",
        ("fr", PortTextKey::Language) => "Langue",
        ("fr", PortTextKey::Automatic) => "Automatique",
        ("fr", PortTextKey::Apply) => "Appliquer",
        ("it", PortTextKey::Language) => "Lingua",
        ("it", PortTextKey::Automatic) => "Automatico",
        ("it", PortTextKey::Apply) => "Applica",
        ("pt" | "es", PortTextKey::Language) => "Idioma",
        ("pt" | "es", PortTextKey::Automatic) => "Automático",
        ("pt" | "es", PortTextKey::Apply) => "Aplicar",
        ("ru", PortTextKey::Language) => "Язык",
        ("ru", PortTextKey::Automatic) => "Автоматически",
        ("ru", PortTextKey::Apply) => "Применить",
        ("ja", PortTextKey::Language) => "言語",
        ("ja", PortTextKey::Automatic) => "自動",
        ("ja", PortTextKey::Apply) => "適用",
        ("cs", PortTextKey::Language) => "Jazyk",
        ("cs", PortTextKey::Automatic) => "Automaticky",
        ("cs", PortTextKey::Apply) => "Použít",
        ("pl", PortTextKey::Language) => "Język",
        ("pl", PortTextKey::Automatic) => "Automatycznie",
        ("pl", PortTextKey::Apply) => "Zastosuj",
        ("zh", PortTextKey::Language) => "語言",
        ("zh", PortTextKey::Automatic) => "自動",
        ("zh", PortTextKey::Apply) => "套用",
        ("ko", PortTextKey::Language) => "언어",
        ("ko", PortTextKey::Automatic) => "자동",
        ("ko", PortTextKey::Apply) => "적용",
        ("th", PortTextKey::Language) => "ภาษา",
        ("th", PortTextKey::Automatic) => "อัตโนมัติ",
        ("th", PortTextKey::Apply) => "ใช้",
        (_, PortTextKey::Language) => "Language",
        (_, PortTextKey::Automatic) => "Automatic",
        (_, PortTextKey::Apply) => "Apply",
        (_, PortTextKey::InstalledLanguages) => "Installed languages",
        (_, PortTextKey::OptionalEnglishFallback) => {
            "Missing optional voice or cinematics use the installed English pack"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_preferences_without_visibility_migrate() {
        let decoded: LocalizationPreferences =
            serde_json::from_str(r#"{"selection":"auto"}"#).unwrap();
        assert_eq!(decoded.selection, LanguageSelection::Auto);
        assert!(decoded.show_in_options);
    }

    #[test]
    fn locale_matching_ignores_encoding_case_and_separator() {
        assert!(locale_eq("pt-BR", "pt_BR.UTF-8"));
        assert_eq!(locale_primary("ZH_tw.UTF-8"), "zh");
    }

    #[test]
    fn preference_store_round_trips_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PREFERENCES_FILE);
        let expected = LocalizationPreferences {
            selection: LanguageSelection::Locale("de-DE".to_owned()),
            show_in_options: false,
        };
        persist_preferences(&PreferenceStore::Native(path.clone()), &expected).unwrap();
        let loaded = load_preferences(&PreferenceStore::Native(path)).unwrap();
        assert_eq!(loaded, expected);
    }

    #[test]
    fn disabled_service_never_advertises_a_selector() {
        let service = LocalizationService::disabled();
        assert!(!service.selector_visible());
        assert_eq!(service.generation(), 0);
    }

    #[test]
    fn port_owned_strings_follow_the_active_language() {
        assert_eq!(port_text(Some("de-DE"), PortTextKey::Language), "Sprache");
        assert_eq!(port_text(Some("ja-JP"), PortTextKey::Apply), "適用");
        assert_eq!(port_text(None, PortTextKey::Automatic), "Automatic");
    }

    #[test]
    fn missing_preference_file_uses_auto_migration_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.json");
        let preferences = load_preferences(&PreferenceStore::Native(path)).unwrap();
        assert_eq!(preferences, LocalizationPreferences::default());
    }
}
