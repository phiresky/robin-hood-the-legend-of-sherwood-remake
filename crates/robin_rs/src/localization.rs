//! Application-owned runtime language selection.
//!
//! The retail game chose the first locale directory it found during startup.
//! The Rust port keeps that compatibility data format, but makes the chosen
//! locale explicit and replaceable.  Only validated packs are exposed to the
//! options UI; changing language is a host-side presentation operation and is
//! deliberately absent from simulation saves, hashes, replays, and network
//! commands.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use robin_assets::resource_manager::ResourceManager;
use robin_assets::shipping_datadir::{ShippingDatadir, ShippingLocale};
use robin_engine::sbfile::SbFileSystem;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod catalog;
mod feature40_text;

#[cfg(any(not(target_arch = "wasm32"), test))]
const PREFERENCES_FILE: &str = "language.json";
#[cfg(target_arch = "wasm32")]
const BROWSER_PREFERENCES_KEY: &str = "robin_hood.language.v1";
const MENU_TEXT_TABLES: [i32; 3] = [1_000_507, 1_000_040, 1_000_034];
const MINIMUM_CORE_MENU_STRINGS: usize = 32;

/// Stable, application-global language choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum LanguageSelection {
    /// Follow the operating-system/browser language when it is installed.
    #[default]
    Auto,
    /// A canonical BCP-47 tag (`de-DE`, `pt-BR`, ...), or `und` for the
    /// international/neutral LCID 2047 data set.
    Locale(String),
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
    #[error("localization file authority is unavailable; initialize the application service first")]
    MissingFileAuthority,
    #[error("shipping locale resources and localization files belong to different applications")]
    MismatchedFileAuthority,
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
    #[error("failed to install language resource lookup (file status {0})")]
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
    #[cfg(target_arch = "wasm32")]
    Browser,
    Memory,
}

/// Mutable application service shared by menus and the active mission host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalizationService {
    /// Live application lookup configuration, never restored from persisted values.
    /// Mission preparation snapshots this reader after selecting its resources.
    #[serde(skip)]
    files: Option<Arc<SbFileSystem>>,
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
            files: None,
            store: PreferenceStore::Memory,
        }
    }

    /// Discover installed packs, load the application-global preference, and
    /// install the selected locale before any UI resources are constructed.
    /// The supplied reader remains live for language changes; mission readers
    /// are separately snapshotted and do not observe those changes.
    pub fn initialize_with_files(
        shipping: Option<&ShippingDatadir>,
        files: Arc<SbFileSystem>,
    ) -> Result<Self, LocalizationError> {
        let store = default_preference_store();
        Self::initialize_with_store(shipping, store, files)
    }

    #[cfg(test)]
    pub(crate) fn initialize_in_memory_for_test(
        files: Arc<SbFileSystem>,
    ) -> Result<Self, LocalizationError> {
        Self::initialize_with_store(None, PreferenceStore::Memory, files)
    }

    fn initialize_with_store(
        shipping: Option<&ShippingDatadir>,
        store: PreferenceStore,
        files: Arc<SbFileSystem>,
    ) -> Result<Self, LocalizationError> {
        let preferences = load_preferences(&store)?;
        let installed = discover_installed_languages(shipping, &files);
        let active = resolve_selection(&preferences.selection, &installed)?;
        let active_data_root = active.as_ref().map(|pack| pack.data_root.clone());
        install_file_lookup(active, &installed, shipping, &files)?;
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
            files: Some(files),
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

    /// No installed voice pack is required: all sessions use the core audio timing table.
    /// The legacy optional handshake locale stays None for this engine-owned authority.
    pub fn canonical_speech_timing_locale(&self) -> Option<&str> {
        None
    }

    /// Persist and atomically commit a new locale lookup generation.  The
    /// caller rebuilds presentation caches only after this succeeds.
    pub fn set_selection(
        &mut self,
        selection: LanguageSelection,
        shipping: Option<&ShippingDatadir>,
    ) -> Result<LanguageChange, LocalizationError> {
        let files = self
            .files
            .as_ref()
            .ok_or(LocalizationError::MissingFileAuthority)?;
        let active = resolve_selection(&selection, &self.installed)?;

        // Re-validate loose content at the commit boundary. A removable or
        // user-edited data directory must not leave half of the UI switched.
        if let Some(pack) = active
            && !pack.data_root.is_empty()
        {
            validate_loose_pack(&pack.locale, &pack.data_root, files)?;
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

        let change = install_file_lookup(active, &self.installed, shipping, files)
            .and_then(|()| persist_preferences(&self.store, &next_preferences));
        if let Err(change) = change {
            if let Err(rollback) = install_file_lookup(previous, &self.installed, shipping, files) {
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

fn discover_installed_languages(
    shipping: Option<&ShippingDatadir>,
    files: &Arc<SbFileSystem>,
) -> Vec<LanguagePack> {
    let english_root = find_loose_root(&LANGUAGE_DEFINITIONS[0], files);
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
        let Some(root) = find_loose_root(definition, files) else {
            continue;
        };
        match validate_loose_pack(definition.locale, &root, files) {
            Ok(()) => {
                let has_voice =
                    loose_path_exists(&root, "Data/Sounds/Exclamations/actors.res", files);
                let has_cinematics = loose_path_exists(&root, "Data/Cinematics", files);
                let mission_names = load_loose_mission_names(&root, files);
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

fn load_loose_mission_names(
    root: &str,
    files: &SbFileSystem,
) -> std::collections::BTreeMap<u32, String> {
    let path = format!("{root}/Data/Configuration/profile.cpf");
    let Ok(mut file) = files.open(&path) else {
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

fn find_loose_root(definition: &LanguageDefinition, files: &SbFileSystem) -> Option<String> {
    std::iter::once(definition.lcid)
        .chain(std::iter::once(definition.locale))
        .chain(definition.aliases.iter().copied())
        .find(|root| {
            loose_path_exists(root, "Data/Text/Level.res", files)
                || loose_path_exists(root, "Data/Interface/Start.sxt", files)
        })
        .map(str::to_owned)
}

fn loose_path_exists(root: &str, relative: &str, files: &SbFileSystem) -> bool {
    files
        .try_exists(&format!("{root}/{relative}"))
        .unwrap_or_else(|status| {
            tracing::warn!(
                root,
                relative,
                status,
                "Cannot inspect optional language pack content"
            );
            false
        })
}

fn validate_loose_pack(
    locale: &str,
    root: &str,
    files: &Arc<SbFileSystem>,
) -> Result<(), LocalizationError> {
    let mut resources = ResourceManager::with_files(files.clone());
    let mut attached = 0usize;
    for relative in ["Data/Text/Level.res", "Data/Interface/Start.sxt"] {
        let path = format!("{root}/{relative}");
        if files
            .try_exists(&path)
            .map_err(LocalizationError::FileLookup)?
        {
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
    files: &SbFileSystem,
) -> Result<(), LocalizationError> {
    if shipping.is_some_and(|shipping| !Arc::ptr_eq(shipping.asset_vfs(), files.asset_vfs())) {
        return Err(LocalizationError::MismatchedFileAuthority);
    }
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
    let status =
        files.set_presentation_locale(selected, fallback, active.map(|pack| pack.locale.as_str()));
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
        files
            .asset_vfs()
            .select_locale(None, None)
            .map_err(|error| LocalizationError::Shipping(error.into()))?;
    }
    Ok(())
}

/// The original font manager selected its TrueType family for international
/// builds whose bitmap fonts cannot cover the locale's script. Keep that
/// decision tied to the same prepared reader as font resource lookup.
pub fn locale_prefers_truetype(files: &SbFileSystem) -> bool {
    let locale = files.presentation_locale();
    matches!(
        locale.as_deref().map(locale_primary).as_deref(),
        Some("ja" | "zh" | "ko" | "th" | "ru" | "pl" | "cs")
    )
}

fn resolve_selection<'a>(
    selection: &LanguageSelection,
    installed: &'a [LanguagePack],
) -> Result<Option<&'a LanguagePack>, LocalizationError> {
    match selection {
        LanguageSelection::Locale(locale) => installed
            .iter()
            .find(|pack| locale_eq(&pack.locale, locale))
            .map(Some)
            .ok_or_else(|| LocalizationError::Unavailable(locale.clone())),
        LanguageSelection::Auto => Ok(auto_language(installed)),
    }
}

fn auto_language(installed: &[LanguagePack]) -> Option<&LanguagePack> {
    if installed.is_empty() {
        return None;
    }
    let system = sys_locale::get_locale().unwrap_or_default();
    auto_language_for_locale(installed, &system)
}

fn auto_language_for_locale<'a>(
    installed: &'a [LanguagePack],
    system: &str,
) -> Option<&'a LanguagePack> {
    installed
        .iter()
        .find(|pack| locale_eq(&pack.locale, system))
        .or_else(|| {
            let primary = locale_primary(system);
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
    normalized_locale_bytes(a).eq(normalized_locale_bytes(b))
}

fn locale_primary(locale: &str) -> Cow<'_, str> {
    let primary = locale
        .split(['.', '@', '-', '_'])
        .next()
        .unwrap_or_default();
    if primary.bytes().any(|byte| byte.is_ascii_uppercase()) {
        Cow::Owned(primary.to_ascii_lowercase())
    } else {
        Cow::Borrowed(primary)
    }
}

fn normalized_locale_bytes(locale: &str) -> impl Iterator<Item = u8> + '_ {
    locale
        .split(['.', '@'])
        .next()
        .unwrap_or(locale)
        .bytes()
        .map(|byte| {
            if byte == b'_' {
                b'-'
            } else {
                byte.to_ascii_lowercase()
            }
        })
}

fn normalize_locale(locale: &str) -> String {
    let mut normalized = locale
        .split(['.', '@'])
        .next()
        .unwrap_or(locale)
        .replace('_', "-");
    normalized.make_ascii_lowercase();
    normalized
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
            let storage = crate::browser_storage::local_storage()
                .map_err(LocalizationError::BrowserStorage)?;
            storage
                .set_item(BROWSER_PREFERENCES_KEY, &encoded)
                .map_err(|error| LocalizationError::BrowserStorage(format!("{error:?}")))
        }
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
            let storage = crate::browser_storage::local_storage()
                .map_err(LocalizationError::BrowserStorage)?;
            storage
                .get_item(BROWSER_PREFERENCES_KEY)
                .map_err(|error| LocalizationError::BrowserStorage(format!("{error:?}")))
        }
        PreferenceStore::Memory => Ok(None),
    }
}

fn persist_native(path: &Path, bytes: &[u8]) -> Result<(), LocalizationError> {
    crate::desktop_persistence::write_bytes(path, bytes).map_err(|source| {
        LocalizationError::PersistPreferences {
            path: path.to_owned(),
            source,
        }
    })
}

fn store_display_path(store: &PreferenceStore) -> PathBuf {
    match store {
        PreferenceStore::Native(path) => path.clone(),
        #[cfg(target_arch = "wasm32")]
        PreferenceStore::Browser => PathBuf::from(BROWSER_PREFERENCES_KEY),
        PreferenceStore::Memory => PathBuf::from("<memory>"),
    }
}

/// Units displayed in save metadata; templates own their complete inflection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RelativeTimeUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

/// Strings introduced by the port cannot rely on unused numeric slots in the
/// retail resource tables. Keep the small language-selector vocabulary in a
/// stable keyed catalogue alongside the locale service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortTextKey {
    SaveNewSaveLabel,
    SaveNewSaveHint,
    SaveMission,
    SavePlayer,
    SaveSaved,
    SaveExactDate,
    SaveCampaignProgress,
    SaveMissions,
    SaveGangSize,
    SaveRansom,
    SaveBlazons,
    SaveAmulets,
    SaveLegacyValueUnavailable,
    SaveInvalidTimestamp,
    SaveRelativeTimeUnavailable,
    SaveLocalTimeUnavailable,
    SaveJustNow,
    SaveCompactSaved,
    SaveCompactCampaignProgress,
    SaveCompactMissions,
    SaveCompactGangSize,
    SaveCompactRansom,
    SaveCompactBlazons,
    SaveCompactAmulets,
    /// English fallback currently owns the complete relative-time grammar.
    SaveRelativeTime {
        unit: RelativeTimeUnit,
        future: bool,
        singular: bool,
    },

    GameplayLabel(crate::gameplay_settings::GameplaySetting),
    GameplayTooltip(crate::gameplay_settings::GameplaySetting),
    GameAutosaved,
    CampaignClassicMap,
    CampaignProgressTree,
    CampaignSherwoodMuseum,
    AutosaveFailed,
    SaveFailed,
    Language,
    Automatic,
    Apply,
    InstalledLanguages,
    OptionalEnglishFallback,
    SpellforgeGameplayAllowLabel,
    SpellforgeGameplayAllowTooltip,
    SpellforgeManageContent,
    SpellforgeHostAndAttest,
    SpellforgeDistributeCompleteMod,
    SpellforgeMissingLicenseWarning,
    SpellforgeHostAttestation,
    SpellforgeMissionField,
    SpellforgeClaimedAuthorField,
    SpellforgeVersionField,
    SpellforgeSourceField,
    SpellforgeLicenseField,
    SpellforgeAuthenticatedHostKeyField,
    SpellforgeCompleteModDeliveryWarning,
    SpellforgeRedistributionWarning,
    SpellforgeTrustExactMod,
    SpellforgeTrustHostContent,
    SpellforgeVanillaPackage,
    SpellforgeLicensePermissionField,
    SpellforgeDistributorKeyField,
    SpellforgeDownloadBytesField,
    SpellforgeFullModHash,
    SpellforgeLuaPackageHash,
    SpellforgeExecutableWarning,
    SpellforgeNonExecutableWarning,
    SpellforgeDisabled,
    SpellforgeApprovalNotPersisted,
    SpellforgePrevious,
    SpellforgeNext,
    SpellforgeRevokeAll,
    SpellforgeClearCache,
    SpellforgeResetTrust,
    SpellforgeRevokeGrant,
    SpellforgeRevokedApprovals,
    SpellforgeRevocationFailed,
    SpellforgeRemovedCachedMods,
    SpellforgeCacheClearFailed,
    SpellforgeTrustResetSuccess,
    SpellforgeTrustResetFailed,
    SpellforgeRevokedApproval,
    SpellforgeApprovalAlreadyAbsent,
    SpellforgeContentTitle,
    SpellforgeApprovalsPage,
    SpellforgeNoApprovals,
    SpellforgeMpPreparedContentLost,
    SpellforgeMpCannotVerifyPreparedContent,
    SpellforgeMpPreparedContentChanged,
    SpellforgeMpRejectedChangedStart,
    SpellforgeMpHostingCancelled,
    SpellforgeMpCannotReviewMetadata,
    SpellforgeMpCannotHostMission,
    SpellforgeMpCannotAdvertiseMission,
    SpellforgeMpCreatingCustomGame,
    SpellforgeMpBrowserCannotHost,
    SpellforgeMpConnectDirectInvite,
    SpellforgeMpDirectTransportClosed,
    SpellforgeMpDirectResolveTimeout,
    SpellforgeMpDirectJoinCancelled,
    SpellforgeMpDirectInviteNoMission,
    SpellforgeMpVanillaMissionMissing,
    SpellforgeMpDirectInvitesBrowserOnly,
    SpellforgeMpNoSeatPreflightTimeout,
    SpellforgeMpPreflightTransportClosed,
    SpellforgeMpConnectForReview,
    SpellforgeMpOrdinaryWelcome,
    SpellforgeMpOfferTimeout,
    SpellforgeMpOfferMismatch,
    SpellforgeMpJoinCancelled,
    SpellforgeMpPreflightTimeout,
    SpellforgeMpModMissionLabel,
}

pub const FEATURE40_PORT_TEXT_KEYS: &[PortTextKey] = &[
    PortTextKey::SpellforgeGameplayAllowLabel,
    PortTextKey::SpellforgeGameplayAllowTooltip,
    PortTextKey::SpellforgeManageContent,
    PortTextKey::SpellforgeHostAndAttest,
    PortTextKey::SpellforgeDistributeCompleteMod,
    PortTextKey::SpellforgeMissingLicenseWarning,
    PortTextKey::SpellforgeHostAttestation,
    PortTextKey::SpellforgeMissionField,
    PortTextKey::SpellforgeClaimedAuthorField,
    PortTextKey::SpellforgeVersionField,
    PortTextKey::SpellforgeSourceField,
    PortTextKey::SpellforgeLicenseField,
    PortTextKey::SpellforgeAuthenticatedHostKeyField,
    PortTextKey::SpellforgeCompleteModDeliveryWarning,
    PortTextKey::SpellforgeRedistributionWarning,
    PortTextKey::SpellforgeTrustExactMod,
    PortTextKey::SpellforgeTrustHostContent,
    PortTextKey::SpellforgeVanillaPackage,
    PortTextKey::SpellforgeLicensePermissionField,
    PortTextKey::SpellforgeDistributorKeyField,
    PortTextKey::SpellforgeDownloadBytesField,
    PortTextKey::SpellforgeFullModHash,
    PortTextKey::SpellforgeLuaPackageHash,
    PortTextKey::SpellforgeExecutableWarning,
    PortTextKey::SpellforgeNonExecutableWarning,
    PortTextKey::SpellforgeDisabled,
    PortTextKey::SpellforgeApprovalNotPersisted,
    PortTextKey::SpellforgePrevious,
    PortTextKey::SpellforgeNext,
    PortTextKey::SpellforgeRevokeAll,
    PortTextKey::SpellforgeClearCache,
    PortTextKey::SpellforgeResetTrust,
    PortTextKey::SpellforgeRevokeGrant,
    PortTextKey::SpellforgeRevokedApprovals,
    PortTextKey::SpellforgeRevocationFailed,
    PortTextKey::SpellforgeRemovedCachedMods,
    PortTextKey::SpellforgeCacheClearFailed,
    PortTextKey::SpellforgeTrustResetSuccess,
    PortTextKey::SpellforgeTrustResetFailed,
    PortTextKey::SpellforgeRevokedApproval,
    PortTextKey::SpellforgeApprovalAlreadyAbsent,
    PortTextKey::SpellforgeContentTitle,
    PortTextKey::SpellforgeApprovalsPage,
    PortTextKey::SpellforgeNoApprovals,
    PortTextKey::SpellforgeMpPreparedContentLost,
    PortTextKey::SpellforgeMpCannotVerifyPreparedContent,
    PortTextKey::SpellforgeMpPreparedContentChanged,
    PortTextKey::SpellforgeMpRejectedChangedStart,
    PortTextKey::SpellforgeMpHostingCancelled,
    PortTextKey::SpellforgeMpCannotReviewMetadata,
    PortTextKey::SpellforgeMpCannotHostMission,
    PortTextKey::SpellforgeMpCannotAdvertiseMission,
    PortTextKey::SpellforgeMpCreatingCustomGame,
    PortTextKey::SpellforgeMpBrowserCannotHost,
    PortTextKey::SpellforgeMpConnectDirectInvite,
    PortTextKey::SpellforgeMpDirectTransportClosed,
    PortTextKey::SpellforgeMpDirectResolveTimeout,
    PortTextKey::SpellforgeMpDirectJoinCancelled,
    PortTextKey::SpellforgeMpDirectInviteNoMission,
    PortTextKey::SpellforgeMpVanillaMissionMissing,
    PortTextKey::SpellforgeMpDirectInvitesBrowserOnly,
    PortTextKey::SpellforgeMpNoSeatPreflightTimeout,
    PortTextKey::SpellforgeMpPreflightTransportClosed,
    PortTextKey::SpellforgeMpConnectForReview,
    PortTextKey::SpellforgeMpOrdinaryWelcome,
    PortTextKey::SpellforgeMpOfferTimeout,
    PortTextKey::SpellforgeMpOfferMismatch,
    PortTextKey::SpellforgeMpJoinCancelled,
    PortTextKey::SpellforgeMpPreflightTimeout,
    PortTextKey::SpellforgeMpModMissionLabel,
];

pub fn port_text(locale: Option<&str>, key: PortTextKey) -> &'static str {
    catalog::text(locale, key)
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PortTextFormatError {
    #[error("localized text argument `{argument}` was supplied more than once")]
    DuplicateArgument { argument: String },
    #[error("localized text template requires missing argument `{argument}`")]
    MissingArgument { argument: String },
    #[error("localized text argument `{argument}` is not used by the template")]
    UnexpectedArgument { argument: String },
    #[error("localized text template has an invalid placeholder near byte {offset}")]
    InvalidTemplate { offset: usize },
}

/// Format one port-owned localized template with strict named placeholders.
/// Inserted values are appended atomically and are never parsed as template
/// syntax, so braces in untrusted metadata cannot alter another field.
pub fn format_port_text(
    locale: Option<&str>,
    key: PortTextKey,
    arguments: &[(&str, &str)],
) -> Result<String, PortTextFormatError> {
    let mut values = std::collections::BTreeMap::new();
    for &(name, value) in arguments {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(PortTextFormatError::InvalidTemplate { offset: 0 });
        }
        if values.insert(name, value).is_some() {
            return Err(PortTextFormatError::DuplicateArgument {
                argument: name.to_owned(),
            });
        }
    }

    let template = port_text(locale, key);
    let mut output = String::with_capacity(template.len());
    let mut used = std::collections::BTreeSet::new();
    let mut offset = 0usize;
    while offset < template.len() {
        let remaining = &template[offset..];
        if remaining.starts_with("{{") {
            output.push('{');
            offset += 2;
            continue;
        }
        if remaining.starts_with("}}") {
            output.push('}');
            offset += 2;
            continue;
        }
        let character = remaining
            .chars()
            .next()
            .expect("non-empty localized template remainder");
        if character == '{' {
            let Some(close) = remaining[1..].find('}') else {
                return Err(PortTextFormatError::InvalidTemplate { offset });
            };
            let name = &remaining[1..1 + close];
            if name.is_empty()
                || name.contains('{')
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err(PortTextFormatError::InvalidTemplate { offset });
            }
            let value = values
                .get(name)
                .ok_or_else(|| PortTextFormatError::MissingArgument {
                    argument: name.to_owned(),
                })?;
            output.push_str(value);
            used.insert(name);
            offset += close + 2;
        } else if character == '}' {
            return Err(PortTextFormatError::InvalidTemplate { offset });
        } else {
            output.push(character);
            offset += character.len_utf8();
        }
    }
    if let Some(name) = values.keys().find(|name| !used.contains(**name)) {
        return Err(PortTextFormatError::UnexpectedArgument {
            argument: (*name).to_owned(),
        });
    }
    Ok(output)
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
    fn borrowed_locale_comparisons_preserve_normalization_rules() {
        let normalize = |locale: &str| {
            locale
                .split(['.', '@'])
                .next()
                .unwrap_or(locale)
                .replace('_', "-")
                .to_ascii_lowercase()
        };
        let locales = [
            "",
            "en-US",
            "EN_us.UTF-8",
            "en",
            "en@latin",
            "en_US@latin.UTF-8",
            "pt_BR",
            "pt-PT",
            "ZH_tw.UTF-8",
            "zh-Hant-TW",
            "@latin",
            ".UTF-8",
            "_en",
            "-en",
            "é_FR",
            "É_fr",
            " de-DE ",
            "en__US",
            "en--us",
        ];
        for left in locales {
            let normalized = normalize(left);
            assert_eq!(normalize_locale(left), normalized);
            assert_eq!(locale_primary(left), normalized.split('-').next().unwrap());
            for right in locales {
                assert_eq!(
                    locale_eq(left, right),
                    normalized == normalize(right),
                    "{left:?} vs {right:?}"
                );
            }
        }
    }

    #[test]
    fn automatic_language_precedence_is_exact_then_primary_then_english_then_first() {
        let installed = ["de-DE", "pt-PT", "pt-BR", "en-US"].map(|locale| LanguagePack {
            locale: locale.into(),
            native_name: locale.into(),
            data_root: String::new(),
            has_voice: false,
            has_cinematics: false,
            voice_uses_english_fallback: false,
            cinematics_use_english_fallback: false,
            mission_names: Default::default(),
        });
        for (system, expected) in [
            ("PT_br.UTF-8", "pt-BR"),
            ("pt-AO", "pt-PT"),
            ("de_AT@euro", "de-DE"),
            ("ja-JP", "en-US"),
            ("", "en-US"),
            (".UTF-8", "en-US"),
        ] {
            assert_eq!(
                auto_language_for_locale(&installed, system).unwrap().locale,
                expected
            );
        }
        assert_eq!(
            auto_language_for_locale(&installed[..3], "ja-JP")
                .unwrap()
                .locale,
            "de-DE"
        );
        assert!(auto_language_for_locale(&[], "en-US").is_none());
        let reordered = [installed[2].clone(), installed[1].clone()];
        assert_eq!(
            auto_language_for_locale(&reordered, "pt-AO")
                .unwrap()
                .locale,
            "pt-BR"
        );
        assert_eq!(
            auto_language_for_locale(&reordered, "pt-PT")
                .unwrap()
                .locale,
            "pt-PT"
        );
    }

    #[test]
    fn selection_resolution_never_falls_back_for_an_explicit_locale() {
        let installed = [LanguagePack {
            locale: "en-US".into(),
            native_name: "English".into(),
            data_root: "1033".into(),
            has_voice: false,
            has_cinematics: false,
            voice_uses_english_fallback: false,
            cinematics_use_english_fallback: false,
            mission_names: Default::default(),
        }];
        let selected =
            resolve_selection(&LanguageSelection::Locale("EN_us.UTF-8".into()), &installed)
                .unwrap()
                .unwrap();
        assert!(std::ptr::eq(selected, &installed[0]));
        assert!(
            resolve_selection(&LanguageSelection::Auto, &[])
                .unwrap()
                .is_none()
        );
        assert_eq!(
            resolve_selection(&LanguageSelection::Auto, &installed).unwrap(),
            Some(&installed[0])
        );
        for packs in [&installed[..], &[][..]] {
            assert!(matches!(
                resolve_selection(&LanguageSelection::Locale("fr-FR".into()), packs),
                Err(LocalizationError::Unavailable(locale)) if locale == "fr-FR"
            ));
        }
    }

    #[test]
    fn lowercase_primary_tags_borrow_only_the_original_prefix() {
        for (locale, expected) in [
            ("en_US.UTF-8", "en"),
            ("zh-Hant-TW", "zh"),
            ("é_FR", "é"),
            ("", ""),
        ] {
            let primary = locale_primary(locale);
            assert!(matches!(primary, Cow::Borrowed(_)));
            assert_eq!(primary, expected);
            assert_eq!(primary.as_ptr(), locale.as_ptr());
        }
        let primary = locale_primary("ZH_tw.UTF-8");
        assert!(matches!(primary, Cow::Owned(_)));
        assert_eq!(primary, "zh");
    }

    #[test]
    fn feature_catalogue_leaves_other_keys_to_their_own_text_policy() {
        for locale in ["en-US", "de-DE", "PT_br.UTF-8", "unknown", ""] {
            for key in [
                PortTextKey::Language,
                PortTextKey::Automatic,
                PortTextKey::Apply,
                PortTextKey::SaveFailed,
            ] {
                assert_eq!(feature40_text::text(locale, key), None);
            }
        }
        assert_eq!(port_text(Some("de-DE"), PortTextKey::Language), "Sprache");
        assert_eq!(
            port_text(Some("unknown"), PortTextKey::Language),
            "Language"
        );
    }

    #[test]
    fn locale_matching_ignores_encoding_case_and_separator() {
        assert!(locale_eq("pt-BR", "pt_BR.UTF-8"));
        assert_eq!(locale_primary("ZH_tw.UTF-8"), "zh");
        assert_eq!(
            port_text(Some("DE_de.UTF-8@latin"), PortTextKey::SpellforgeRevokeAll),
            feature40_text::text("de-DE", PortTextKey::SpellforgeRevokeAll)
                .expect("complete German catalogue")
        );
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
    fn named_port_text_formatting_is_strict_and_does_not_reparse_values() {
        let rendered = format_port_text(
            Some("de-DE"),
            PortTextKey::SpellforgeHostAttestation,
            &[
                ("host", "key-{literal}"),
                ("title", "A {source} title"),
                ("source", "https://example.invalid/{title}"),
            ],
        )
        .unwrap();
        assert_eq!(
            rendered,
            "Host key-{literal} bestätigt die Erlaubnis, `A {source} title` aus https://example.invalid/{title} für diese Mehrspielersitzung weiterzuverteilen"
        );
        assert!(matches!(
            format_port_text(
                None,
                PortTextKey::SpellforgeMissionField,
                &[("value", "one"), ("value", "two")]
            ),
            Err(PortTextFormatError::DuplicateArgument { .. })
        ));
        assert!(matches!(
            format_port_text(None, PortTextKey::SpellforgeMissionField, &[]),
            Err(PortTextFormatError::MissingArgument { .. })
        ));
        assert!(matches!(
            format_port_text(
                None,
                PortTextKey::SpellforgeMissionField,
                &[("value", "mission"), ("extra", "unused")]
            ),
            Err(PortTextFormatError::UnexpectedArgument { .. })
        ));
    }

    #[test]
    fn every_feature40_port_text_key_has_an_english_fallback() {
        for &key in FEATURE40_PORT_TEXT_KEYS {
            assert!(!port_text(Some("zz-ZZ"), key).is_empty());
        }
    }

    #[test]
    fn every_supported_shipping_locale_has_a_complete_feature40_catalogue() {
        for definition in LANGUAGE_DEFINITIONS {
            for &key in FEATURE40_PORT_TEXT_KEYS {
                let translated =
                    feature40_text::text(definition.locale, key).unwrap_or_else(|| {
                        panic!(
                            "missing Feature 40 translation for {} {key:?}",
                            definition.locale
                        )
                    });
                assert!(!translated.trim().is_empty());
                assert_eq!(port_text(Some(definition.locale), key), translated);
            }
        }
        assert_ne!(
            port_text(Some("zh-CN"), PortTextKey::SpellforgeExecutableWarning),
            port_text(Some("zh-TW"), PortTextKey::SpellforgeExecutableWarning),
            "Simplified and Traditional Chinese must retain regional catalogues"
        );
    }

    #[test]
    fn safety_critical_feature40_text_is_really_translated() {
        let safety_surface = [
            PortTextKey::SpellforgeGameplayAllowLabel,
            PortTextKey::SpellforgeGameplayAllowTooltip,
            PortTextKey::SpellforgeManageContent,
            PortTextKey::SpellforgeHostAndAttest,
            PortTextKey::SpellforgeDistributeCompleteMod,
            PortTextKey::SpellforgeMissingLicenseWarning,
            PortTextKey::SpellforgeHostAttestation,
            PortTextKey::SpellforgeAuthenticatedHostKeyField,
            PortTextKey::SpellforgeCompleteModDeliveryWarning,
            PortTextKey::SpellforgeRedistributionWarning,
            PortTextKey::SpellforgeTrustExactMod,
            PortTextKey::SpellforgeTrustHostContent,
            PortTextKey::SpellforgeLicensePermissionField,
            PortTextKey::SpellforgeDistributorKeyField,
            PortTextKey::SpellforgeExecutableWarning,
            PortTextKey::SpellforgeNonExecutableWarning,
            PortTextKey::SpellforgeDisabled,
            PortTextKey::SpellforgeApprovalNotPersisted,
            PortTextKey::SpellforgeRevokeAll,
            PortTextKey::SpellforgeClearCache,
            PortTextKey::SpellforgeResetTrust,
            PortTextKey::SpellforgeRevokeGrant,
            PortTextKey::SpellforgeRevokedApprovals,
            PortTextKey::SpellforgeRevocationFailed,
            PortTextKey::SpellforgeRemovedCachedMods,
            PortTextKey::SpellforgeCacheClearFailed,
            PortTextKey::SpellforgeTrustResetSuccess,
            PortTextKey::SpellforgeTrustResetFailed,
            PortTextKey::SpellforgeRevokedApproval,
            PortTextKey::SpellforgeApprovalAlreadyAbsent,
            PortTextKey::SpellforgeApprovalsPage,
            PortTextKey::SpellforgeNoApprovals,
            PortTextKey::SpellforgeMpPreparedContentLost,
            PortTextKey::SpellforgeMpCannotVerifyPreparedContent,
            PortTextKey::SpellforgeMpPreparedContentChanged,
            PortTextKey::SpellforgeMpRejectedChangedStart,
            PortTextKey::SpellforgeMpHostingCancelled,
            PortTextKey::SpellforgeMpCannotReviewMetadata,
            PortTextKey::SpellforgeMpCannotHostMission,
            PortTextKey::SpellforgeMpCannotAdvertiseMission,
            PortTextKey::SpellforgeMpCreatingCustomGame,
            PortTextKey::SpellforgeMpBrowserCannotHost,
            PortTextKey::SpellforgeMpConnectDirectInvite,
            PortTextKey::SpellforgeMpDirectTransportClosed,
            PortTextKey::SpellforgeMpDirectResolveTimeout,
            PortTextKey::SpellforgeMpDirectJoinCancelled,
            PortTextKey::SpellforgeMpDirectInviteNoMission,
            PortTextKey::SpellforgeMpVanillaMissionMissing,
            PortTextKey::SpellforgeMpDirectInvitesBrowserOnly,
            PortTextKey::SpellforgeMpNoSeatPreflightTimeout,
            PortTextKey::SpellforgeMpPreflightTransportClosed,
            PortTextKey::SpellforgeMpConnectForReview,
            PortTextKey::SpellforgeMpOrdinaryWelcome,
            PortTextKey::SpellforgeMpOfferTimeout,
            PortTextKey::SpellforgeMpOfferMismatch,
            PortTextKey::SpellforgeMpJoinCancelled,
            PortTextKey::SpellforgeMpPreflightTimeout,
        ];
        for definition in LANGUAGE_DEFINITIONS
            .iter()
            .filter(|definition| !matches!(definition.locale, "en-US" | "und"))
        {
            for key in safety_surface {
                assert_ne!(
                    port_text(Some(definition.locale), key),
                    port_text(Some("en-US"), key),
                    "{} silently fell back to English for {key:?}",
                    definition.locale
                );
            }
        }
    }

    fn template_arguments(template: &str) -> std::collections::BTreeSet<String> {
        let mut arguments = std::collections::BTreeSet::new();
        let mut offset = 0usize;
        while offset < template.len() {
            let remaining = &template[offset..];
            if remaining.starts_with("{{") || remaining.starts_with("}}") {
                offset += 2;
                continue;
            }
            let character = remaining.chars().next().unwrap();
            if character == '{' {
                let close = remaining[1..]
                    .find('}')
                    .expect("catalogue templates must close every placeholder");
                arguments.insert(remaining[1..1 + close].to_owned());
                offset += close + 2;
            } else {
                assert_ne!(character, '}', "unescaped closing brace in {template:?}");
                offset += character.len_utf8();
            }
        }
        arguments
    }

    #[test]
    fn every_feature40_template_has_the_exact_named_argument_contract() {
        let templated: &[(PortTextKey, &[&str])] = &[
            (
                PortTextKey::SpellforgeHostAttestation,
                &["host", "source", "title"],
            ),
            (PortTextKey::SpellforgeMissionField, &["value"]),
            (PortTextKey::SpellforgeClaimedAuthorField, &["value"]),
            (PortTextKey::SpellforgeVersionField, &["value"]),
            (PortTextKey::SpellforgeSourceField, &["value"]),
            (PortTextKey::SpellforgeLicenseField, &["value"]),
            (PortTextKey::SpellforgeAuthenticatedHostKeyField, &["value"]),
            (PortTextKey::SpellforgeLicensePermissionField, &["value"]),
            (PortTextKey::SpellforgeDistributorKeyField, &["value"]),
            (PortTextKey::SpellforgeDownloadBytesField, &["bytes"]),
            (PortTextKey::SpellforgeApprovalNotPersisted, &["error"]),
            (PortTextKey::SpellforgeRevokeGrant, &["hash", "title"]),
            (PortTextKey::SpellforgeRevokedApprovals, &["count"]),
            (PortTextKey::SpellforgeRevocationFailed, &["error"]),
            (PortTextKey::SpellforgeRemovedCachedMods, &["count"]),
            (PortTextKey::SpellforgeCacheClearFailed, &["error"]),
            (PortTextKey::SpellforgeTrustResetFailed, &["error"]),
            (PortTextKey::SpellforgeRevokedApproval, &["title"]),
            (PortTextKey::SpellforgeApprovalsPage, &["page", "pages"]),
            (
                PortTextKey::SpellforgeMpCannotVerifyPreparedContent,
                &["error"],
            ),
            (PortTextKey::SpellforgeMpRejectedChangedStart, &["error"]),
            (PortTextKey::SpellforgeMpCannotReviewMetadata, &["error"]),
            (PortTextKey::SpellforgeMpCannotHostMission, &["error"]),
            (PortTextKey::SpellforgeMpCannotAdvertiseMission, &["error"]),
            (PortTextKey::SpellforgeMpConnectDirectInvite, &["error"]),
            (PortTextKey::SpellforgeMpVanillaMissionMissing, &["mission"]),
            (PortTextKey::SpellforgeMpConnectForReview, &["error"]),
            (
                PortTextKey::SpellforgeMpOfferMismatch,
                &["actual", "advertised"],
            ),
            (
                PortTextKey::SpellforgeMpModMissionLabel,
                &["title", "version"],
            ),
        ];

        for definition in LANGUAGE_DEFINITIONS {
            for &(key, expected) in templated {
                let actual = template_arguments(port_text(Some(definition.locale), key));
                let expected = expected
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect::<std::collections::BTreeSet<_>>();
                assert_eq!(
                    actual, expected,
                    "placeholder drift for {} {key:?}",
                    definition.locale
                );
                let values = expected
                    .iter()
                    .map(|name| (name.as_str(), "value"))
                    .collect::<Vec<_>>();
                format_port_text(Some(definition.locale), key, &values).unwrap();
            }
        }

        for definition in LANGUAGE_DEFINITIONS {
            for &key in FEATURE40_PORT_TEXT_KEYS {
                if !templated
                    .iter()
                    .any(|(templated_key, _)| *templated_key == key)
                {
                    assert!(
                        template_arguments(port_text(Some(definition.locale), key)).is_empty(),
                        "unregistered template arguments for {} {key:?}",
                        definition.locale
                    );
                }
            }
        }
    }

    #[test]
    fn every_feature40_catalogue_key_is_used_by_its_presentation_surface() {
        let multiplayer = include_str!("main_menu/multiplayer_menu.rs");
        let consent = include_str!("ingame_menu/spellforge_content.rs");
        let gameplay = include_str!("ingame_menu/gameplay.rs");
        for &key in FEATURE40_PORT_TEXT_KEYS {
            let name = format!("{key:?}");
            let needle = format!("PortTextKey::{name}");
            let source = if name.starts_with("SpellforgeMp") {
                multiplayer
            } else {
                // Gameplay owns three keys; the consent/trust surface owns the
                // rest. Search both without treating catalogue/test references
                // as production use.
                if gameplay.contains(&needle) {
                    gameplay
                } else {
                    consent
                }
            };
            assert!(source.contains(&needle), "unused Feature40 key {name}");
        }
    }

    #[test]
    fn missing_preference_file_uses_auto_migration_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.json");
        let preferences = load_preferences(&PreferenceStore::Native(path)).unwrap();
        assert_eq!(preferences, LocalizationPreferences::default());
    }

    #[test]
    fn installing_locale_lookup_publishes_font_policy_only_to_its_reader() {
        let files = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        let other = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        let pack = LanguagePack {
            locale: "ja-JP".to_owned(),
            native_name: "日本語".to_owned(),
            data_root: "1041".to_owned(),
            has_voice: false,
            has_cinematics: false,
            voice_uses_english_fallback: false,
            cinematics_use_english_fallback: false,
            mission_names: Default::default(),
        };
        install_file_lookup(Some(&pack), std::slice::from_ref(&pack), None, &files).unwrap();
        assert_eq!(files.locale_paths(), (Some("1041".to_owned()), None));
        assert!(locale_prefers_truetype(&files));
        let prepared = files.snapshot();
        install_file_lookup(None, &[], None, &other).unwrap();
        assert!(locale_prefers_truetype(&files));
        install_file_lookup(None, &[], None, &files).unwrap();
        assert!(!locale_prefers_truetype(&files));
        assert!(locale_prefers_truetype(&prepared));
    }

    #[test]
    fn font_policy_uses_each_readers_selected_language_not_its_root_spelling() {
        let files = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        for language in [
            "ja-JP", "zh-CN", "ko-KR", "th-TH", "ru-RU", "pl-PL", "cs-CZ",
        ] {
            assert_eq!(
                files.set_presentation_locale(Some("1033"), None, Some(language)),
                0
            );
            assert!(locale_prefers_truetype(&files), "{language}");
        }
        for language in [None, Some("en-US"), Some("de-DE"), Some("und")] {
            assert_eq!(
                files.set_presentation_locale(Some("1041"), None, language),
                0
            );
            assert!(!locale_prefers_truetype(&files), "{language:?}");
        }
    }

    #[test]
    fn failed_preference_publication_restores_reader_and_keeps_service_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences-is-a-directory");
        std::fs::create_dir(&path).unwrap();
        let files = Arc::new(SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        let mut service = LocalizationService::disabled();
        service.files = Some(files.clone());
        service.store = PreferenceStore::Native(path.clone());
        service.installed = ["en-US", "de-DE"]
            .map(|locale| LanguagePack {
                locale: locale.into(),
                native_name: locale.into(),
                data_root: String::new(),
                has_voice: false,
                has_cinematics: false,
                voice_uses_english_fallback: false,
                cinematics_use_english_fallback: false,
                mission_names: Default::default(),
            })
            .to_vec();
        service.active_locale = Some("en-US".into());
        service.active_data_root = Some(String::new());
        install_file_lookup(
            Some(&service.installed[0]),
            &service.installed,
            None,
            &files,
        )
        .unwrap();
        let before = serde_json::to_value(&service).unwrap();
        assert!(matches!(
            service.set_selection(LanguageSelection::Locale("de-DE".into()), None),
            Err(LocalizationError::PersistPreferences { .. })
        ));
        assert_eq!(serde_json::to_value(&service).unwrap(), before);
        assert_eq!(files.presentation_locale().as_deref(), Some("en-US"));
        assert!(path.is_dir());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn localization_retains_live_application_reader_without_touching_other_readers() {
        let files = Arc::new(SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        let other = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            other.set_presentation_locale(Some("other-locale"), None, Some("en-US")),
            0
        );
        let mut service = LocalizationService::initialize_with_store(
            None,
            PreferenceStore::Memory,
            files.clone(),
        )
        .unwrap();
        assert!(Arc::ptr_eq(service.files.as_ref().unwrap(), &files));
        assert_eq!(
            files.set_presentation_locale(Some("application-locale"), None, Some("ja-JP")),
            0
        );
        let mission = files.snapshot();
        service
            .set_selection(LanguageSelection::Auto, None)
            .unwrap();
        assert_eq!(files.locale_paths(), (None, None));
        assert_eq!(files.presentation_locale(), None);
        assert_eq!(mission.presentation_locale().as_deref(), Some("ja-JP"));
        assert_eq!(other.presentation_locale().as_deref(), Some("en-US"));
        assert_eq!(
            mission.locale_paths(),
            (Some("application-locale".to_owned()), None)
        );
        assert_eq!(
            other.locale_paths(),
            (Some("other-locale".to_owned()), None)
        );
    }

    #[test]
    fn disabled_and_deserialized_localization_cannot_acquire_file_authority() {
        let files = Arc::new(SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        let initialized =
            LocalizationService::initialize_with_store(None, PreferenceStore::Memory, files)
                .unwrap();
        let serialized = serde_json::to_value(&initialized).unwrap();
        assert!(serialized.get("files").is_none());
        let restored = serde_json::from_value::<LocalizationService>(serialized).unwrap();
        for mut service in [LocalizationService::disabled(), restored] {
            assert!(matches!(
                service.set_selection(LanguageSelection::Auto, None),
                Err(LocalizationError::MissingFileAuthority)
            ));
        }
    }

    #[test]
    fn explicit_unavailable_saved_locale_fails_initialization() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PREFERENCES_FILE);
        persist_preferences(
            &PreferenceStore::Native(path.clone()),
            &LocalizationPreferences {
                selection: LanguageSelection::Locale("zz-ZZ".to_owned()),
                show_in_options: true,
            },
        )
        .unwrap();

        let files = Arc::new(SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        let error =
            LocalizationService::initialize_with_store(None, PreferenceStore::Native(path), files)
                .unwrap_err();
        assert!(matches!(error, LocalizationError::Unavailable(locale) if locale == "zz-ZZ"));
    }
}
