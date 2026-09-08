//! Canonical Spellforge archive admission.
//!
//! Native, browser, author tooling, save loading, and network adoption must
//! all construct packages through this byte-oriented path.  It deliberately
//! does not extract archives to a filesystem.

use robin_engine::spellforge::{
    SPELLFORGE_CONTRACT_VERSION, SpellforgePackage, SpellforgeScriptMode,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

/// Maximum compressed bytes accepted for one mission or library archive.
pub const ARCHIVE_BYTE_LIMIT: usize = 64 * 1024 * 1024;
/// Maximum number of entries advertised by one ZIP central directory.
pub const ARCHIVE_ENTRY_LIMIT: usize = 2_048;
/// Maximum encoded ZIP central-directory bytes accepted before entry scans.
pub const ARCHIVE_DIRECTORY_LIMIT: usize = 4 * 1024 * 1024;
/// Maximum combined uncompressed Lua source retained by a package.
pub const PACKAGE_SOURCE_LIMIT: usize = robin_engine::spellforge::SPELLFORGE_PACKAGE_SOURCE_LIMIT;
pub(crate) const PACKAGE_FILE_LIMIT: usize =
    robin_engine::spellforge::SPELLFORGE_PACKAGE_FILE_LIMIT;
pub(crate) const PACKAGE_PATH_LIMIT: usize =
    robin_engine::spellforge::SPELLFORGE_PACKAGE_PATH_LIMIT;
pub(crate) const PACKAGE_METADATA_LIMIT: usize =
    robin_engine::spellforge::SPELLFORGE_PACKAGE_METADATA_LIMIT;
const CONTRACT_MANIFEST_LIMIT: usize = 64 * 1024;

/// Stable package-admission categories suitable for UI and author tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpellforgePackageErrorKind {
    ArchiveLimit,
    InvalidArchive,
    UnsafePath,
    DuplicatePath,
    MissingMission,
    MissingCompanion,
    SourceLimit,
    InvalidContract,
}

/// A typed failure from the package trust boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct SpellforgePackageError {
    pub kind: SpellforgePackageErrorKind,
    pub message: String,
}

impl SpellforgePackageError {
    fn new(kind: SpellforgePackageErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// Build the exact executable package selected by an `.rhm` archive entry.
///
/// `mission_rhm_entry` is the full ZIP path chosen by the mission picker, not
/// merely its basename.  That distinction is required for multilingual
/// distributions which contain several same-name missions and companions.
pub fn build_package_from_archives(
    mission_archive: &[u8],
    mission_rhm_entry: &str,
    mission_basename: &str,
    shared_library_archive: Option<&[u8]>,
) -> Result<SpellforgePackage, SpellforgePackageError> {
    let selected_rhm = canonical_archive_path(mission_rhm_entry)?;
    let expected_companion = selected_rhm
        .strip_suffix(".rhm")
        .map(|path| format!("{path}.lua"))
        .ok_or_else(|| {
            SpellforgePackageError::new(
                SpellforgePackageErrorKind::MissingCompanion,
                format!("selected mission archive entry `{mission_rhm_entry}` is not an .rhm file"),
            )
        })?;
    let expected_basename = format!("{}.lua", mission_basename.to_ascii_lowercase());
    if expected_companion
        .rsplit('/')
        .next()
        .is_none_or(|leaf| leaf != expected_basename)
    {
        return Err(SpellforgePackageError::new(
            SpellforgePackageErrorKind::MissingCompanion,
            format!(
                "selected mission `{mission_rhm_entry}` does not match basename `{mission_basename}`"
            ),
        ));
    }

    let mission = read_mission_archive(mission_archive, &selected_rhm, &expected_companion)?;
    let mut files = BTreeMap::new();
    let entrypoint = expected_basename;
    files.insert(entrypoint.clone(), mission.companion);
    files.extend(mission.modules);
    if mission.libraries.is_empty() {
        if let Some(shared) = shared_library_archive {
            files.extend(read_shared_libraries(shared)?);
        }
    } else {
        files.extend(mission.libraries);
    }
    let mut package = SpellforgePackage {
        contract_version: SPELLFORGE_CONTRACT_VERSION,
        vm_abi: super::spellforge_vm_abi().to_owned(),
        script_mode: mission.script_mode,
        entrypoint,
        files,
        sha256: [0; 32],
    };
    package.sha256 = super::compute_package_sha256(&package);
    Ok(package)
}

struct MissionSources {
    companion: Vec<u8>,
    modules: BTreeMap<String, Vec<u8>>,
    libraries: BTreeMap<String, Vec<u8>>,
    script_mode: SpellforgeScriptMode,
}

#[derive(Deserialize)]
struct ContractManifest {
    #[serde(default = "contract_version")]
    contract_version: u32,
    #[serde(default)]
    script_mode: Option<String>,
}

const fn contract_version() -> u32 {
    SPELLFORGE_CONTRACT_VERSION
}

fn read_mission_archive(
    bytes: &[u8],
    selected_rhm: &str,
    expected_companion: &str,
) -> Result<MissionSources, SpellforgePackageError> {
    let mut archive = open_archive(bytes)?;
    let companion_parent = expected_companion
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent);
    let library_prefix = if companion_parent.is_empty() {
        "lib/".to_owned()
    } else {
        format!("{companion_parent}/lib/")
    };
    let mut companion = None;
    let mut sibling_modules = BTreeMap::<String, Vec<u8>>::new();
    let mut sibling_missions = BTreeSet::<String>::new();
    let mut libraries = BTreeMap::new();
    let mut manifest = None;
    let mut selected_mission_exists = false;
    let mut seen = BTreeMap::<String, String>::new();
    let mut source_bytes = 0usize;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| invalid_archive(format!("cannot read ZIP entry {index}: {error}")))?;
        let raw_name = std::str::from_utf8(entry.name_raw()).map_err(|_| {
            SpellforgePackageError::new(
                SpellforgePackageErrorKind::UnsafePath,
                format!("ZIP entry {index} has a non-UTF-8 path"),
            )
        })?;
        let canonical = canonical_archive_path(raw_name)?;
        reject_duplicate(&mut seen, &canonical, raw_name)?;
        reject_symlink(&entry, &canonical)?;
        if entry.is_dir() {
            continue;
        }

        if canonical == selected_rhm {
            selected_mission_exists = true;
        }
        if let Some(relative) = relative_to_parent(&canonical, companion_parent)
            && !relative.contains('/')
            && let Some(stem) = relative.strip_suffix(".rhm")
        {
            sibling_missions.insert(stem.to_owned());
        }

        if canonical == expected_companion {
            if companion.is_some() {
                return Err(SpellforgePackageError::new(
                    SpellforgePackageErrorKind::DuplicatePath,
                    format!("mission contains multiple `{expected_companion}` companions"),
                ));
            }
            let source = read_bounded(
                &mut entry,
                PACKAGE_SOURCE_LIMIT.saturating_sub(source_bytes),
                &canonical,
            )?;
            source_bytes = checked_source_total(source_bytes, source.len(), &canonical)?;
            companion = Some(source);
            continue;
        }
        if let Some(relative) = relative_to_parent(&canonical, companion_parent)
            && !relative.contains('/')
            && relative.ends_with(".lua")
        {
            let source = read_bounded(
                &mut entry,
                PACKAGE_SOURCE_LIMIT.saturating_sub(source_bytes),
                &canonical,
            )?;
            source_bytes = checked_source_total(source_bytes, source.len(), &canonical)?;
            if sibling_modules
                .insert(relative.to_owned(), source)
                .is_some()
            {
                return Err(SpellforgePackageError::new(
                    SpellforgePackageErrorKind::DuplicatePath,
                    format!("duplicate case-insensitive Spellforge module `{relative}`"),
                ));
            }
            continue;
        }
        if let Some(relative) = canonical.strip_prefix(&library_prefix) {
            if !relative.ends_with(".lua") {
                continue;
            }
            validate_library_relative(relative, &canonical)?;
            let package_path = format!("lib/{relative}");
            let source = read_bounded(
                &mut entry,
                PACKAGE_SOURCE_LIMIT.saturating_sub(source_bytes),
                &canonical,
            )?;
            source_bytes = checked_source_total(source_bytes, source.len(), &canonical)?;
            if libraries.insert(package_path.clone(), source).is_some() {
                return Err(SpellforgePackageError::new(
                    SpellforgePackageErrorKind::DuplicatePath,
                    format!("duplicate case-insensitive Spellforge library `{package_path}`"),
                ));
            }
            continue;
        }
        if canonical
            .rsplit('/')
            .next()
            .is_some_and(|leaf| leaf == "spellforge.contract.json")
        {
            if manifest.is_some() {
                return Err(SpellforgePackageError::new(
                    SpellforgePackageErrorKind::InvalidContract,
                    "mission contains multiple spellforge.contract.json files",
                ));
            }
            manifest = Some((
                canonical.clone(),
                read_bounded(&mut entry, CONTRACT_MANIFEST_LIMIT, &canonical)?,
            ));
        }
    }

    if !selected_mission_exists {
        return Err(SpellforgePackageError::new(
            SpellforgePackageErrorKind::MissingMission,
            format!("no exact selected mission `{selected_rhm}` in mission archive"),
        ));
    }
    let companion = companion.ok_or_else(|| {
        SpellforgePackageError::new(
            SpellforgePackageErrorKind::MissingCompanion,
            format!("no exact `{expected_companion}` companion in mission archive"),
        )
    })?;
    let script_mode = manifest.map_or(Ok(SpellforgeScriptMode::Replace), |(path, bytes)| {
        parse_contract(&path, &bytes)
    })?;
    sibling_modules.retain(|path, _| {
        path.strip_suffix(".lua")
            .is_none_or(|stem| !sibling_missions.contains(stem))
    });
    Ok(MissionSources {
        companion,
        modules: sibling_modules,
        libraries,
        script_mode,
    })
}

fn relative_to_parent<'a>(path: &'a str, parent: &str) -> Option<&'a str> {
    if parent.is_empty() {
        Some(path)
    } else {
        path.strip_prefix(parent)?.strip_prefix('/')
    }
}

fn read_shared_libraries(
    bytes: &[u8],
) -> Result<BTreeMap<String, Vec<u8>>, SpellforgePackageError> {
    let mut archive = open_archive(bytes)?;
    let mut libraries = BTreeMap::new();
    let mut seen = BTreeMap::<String, String>::new();
    let mut source_bytes = 0usize;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| invalid_archive(format!("cannot read ZIP entry {index}: {error}")))?;
        let raw_name = std::str::from_utf8(entry.name_raw()).map_err(|_| {
            SpellforgePackageError::new(
                SpellforgePackageErrorKind::UnsafePath,
                format!("ZIP entry {index} has a non-UTF-8 path"),
            )
        })?;
        let canonical = canonical_archive_path(raw_name)?;
        reject_duplicate(&mut seen, &canonical, raw_name)?;
        reject_symlink(&entry, &canonical)?;
        if entry.is_dir() || !canonical.ends_with(".lua") {
            continue;
        }
        let relative = if let Some(relative) = canonical.strip_prefix("lib/") {
            relative
        } else if let Some((_, relative)) = canonical.rsplit_once("/lib/") {
            relative
        } else {
            continue;
        };
        validate_library_relative(relative, &canonical)?;
        let package_path = format!("lib/{relative}");
        let source = read_bounded(
            &mut entry,
            PACKAGE_SOURCE_LIMIT.saturating_sub(source_bytes),
            &canonical,
        )?;
        source_bytes = checked_source_total(source_bytes, source.len(), &canonical)?;
        if libraries.insert(package_path.clone(), source).is_some() {
            return Err(SpellforgePackageError::new(
                SpellforgePackageErrorKind::DuplicatePath,
                format!("duplicate case-insensitive Spellforge library `{package_path}`"),
            ));
        }
    }
    Ok(libraries)
}

fn open_archive(bytes: &[u8]) -> Result<zip::ZipArchive<Cursor<&[u8]>>, SpellforgePackageError> {
    preflight_zip(bytes)?;
    zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| invalid_archive(format!("invalid ZIP archive: {error}")))
}

/// Bound archive bytes, entry count, and central-directory bytes before the
/// ZIP crate scans or allocates its entry table.
fn preflight_zip(bytes: &[u8]) -> Result<(), SpellforgePackageError> {
    if bytes.len() > ARCHIVE_BYTE_LIMIT {
        return Err(SpellforgePackageError::new(
            SpellforgePackageErrorKind::ArchiveLimit,
            format!(
                "ZIP archive is {} bytes; limit is {ARCHIVE_BYTE_LIMIT}",
                bytes.len()
            ),
        ));
    }
    const EOCD_LEN: usize = 22;
    const MAX_COMMENT: usize = u16::MAX as usize;
    if bytes.len() < EOCD_LEN {
        return Err(invalid_archive(
            "ZIP archive has no end-of-central-directory record",
        ));
    }
    let search_start = bytes.len().saturating_sub(EOCD_LEN + MAX_COMMENT);
    let eocd = (search_start..=bytes.len() - EOCD_LEN)
        .rev()
        .find(|&offset| {
            bytes[offset..].starts_with(b"PK\x05\x06")
                && offset + EOCD_LEN + le_u16(bytes, offset + 20) as usize == bytes.len()
        })
        .ok_or_else(|| invalid_archive("ZIP archive has no valid terminal directory record"))?;
    let disk = le_u16(bytes, eocd + 4);
    let directory_disk = le_u16(bytes, eocd + 6);
    let disk_entries = le_u16(bytes, eocd + 8);
    let total_entries = le_u16(bytes, eocd + 10);
    let directory_bytes = le_u32(bytes, eocd + 12) as usize;
    let directory_offset = le_u32(bytes, eocd + 16) as usize;
    if disk != 0 || directory_disk != 0 || disk_entries != total_entries {
        return Err(invalid_archive("multi-disk ZIP archives are not supported"));
    }
    if total_entries == u16::MAX
        || directory_bytes == u32::MAX as usize
        || directory_offset == u32::MAX as usize
    {
        return Err(SpellforgePackageError::new(
            SpellforgePackageErrorKind::ArchiveLimit,
            "ZIP64 archives are not supported by the bounded package format",
        ));
    }
    if total_entries as usize > ARCHIVE_ENTRY_LIMIT {
        return Err(SpellforgePackageError::new(
            SpellforgePackageErrorKind::ArchiveLimit,
            format!(
                "ZIP archive advertises {total_entries} entries; limit is {ARCHIVE_ENTRY_LIMIT}"
            ),
        ));
    }
    if directory_bytes > ARCHIVE_DIRECTORY_LIMIT {
        return Err(SpellforgePackageError::new(
            SpellforgePackageErrorKind::ArchiveLimit,
            format!(
                "ZIP central directory is {directory_bytes} bytes; limit is {ARCHIVE_DIRECTORY_LIMIT}"
            ),
        ));
    }
    if directory_offset
        .checked_add(directory_bytes)
        .is_none_or(|end| end > eocd)
    {
        return Err(invalid_archive(
            "ZIP central directory lies outside the archive",
        ));
    }
    Ok(())
}

fn canonical_archive_path(raw: &str) -> Result<String, SpellforgePackageError> {
    if raw.is_empty()
        || raw.starts_with('/')
        || raw.contains('\\')
        || raw.contains('\0')
        || raw.len() > PACKAGE_PATH_LIMIT
    {
        return Err(SpellforgePackageError::new(
            SpellforgePackageErrorKind::UnsafePath,
            format!("unsafe ZIP entry path `{raw}`"),
        ));
    }
    let without_directory_slash = raw.strip_suffix('/').unwrap_or(raw);
    if without_directory_slash
        .split('/')
        .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(SpellforgePackageError::new(
            SpellforgePackageErrorKind::UnsafePath,
            format!("unsafe ZIP entry path `{raw}`"),
        ));
    }
    Ok(without_directory_slash.to_ascii_lowercase())
}

fn reject_duplicate(
    seen: &mut BTreeMap<String, String>,
    canonical: &str,
    raw: &str,
) -> Result<(), SpellforgePackageError> {
    if let Some(previous) = seen.insert(canonical.to_owned(), raw.to_owned()) {
        return Err(SpellforgePackageError::new(
            SpellforgePackageErrorKind::DuplicatePath,
            format!("duplicate case-insensitive ZIP path `{previous}` and `{raw}`"),
        ));
    }
    Ok(())
}

fn reject_symlink<R: Read>(
    entry: &zip::read::ZipFile<'_, R>,
    path: &str,
) -> Result<(), SpellforgePackageError> {
    if entry
        .unix_mode()
        .is_some_and(|mode| mode & 0o170000 == 0o120000)
    {
        return Err(SpellforgePackageError::new(
            SpellforgePackageErrorKind::UnsafePath,
            format!("symlink ZIP entry `{path}` is not allowed"),
        ));
    }
    Ok(())
}

fn validate_library_relative(
    relative: &str,
    archive_path: &str,
) -> Result<(), SpellforgePackageError> {
    if relative.is_empty()
        || relative
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(SpellforgePackageError::new(
            SpellforgePackageErrorKind::UnsafePath,
            format!("unsafe Spellforge library archive path `{archive_path}`"),
        ));
    }
    Ok(())
}

fn read_bounded(
    reader: &mut impl Read,
    limit: usize,
    path: &str,
) -> Result<Vec<u8>, SpellforgePackageError> {
    let mut bytes = Vec::new();
    reader
        .take((limit as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| invalid_archive(format!("cannot read `{path}`: {error}")))?;
    if bytes.len() > limit {
        return Err(SpellforgePackageError::new(
            SpellforgePackageErrorKind::SourceLimit,
            format!(
                "Spellforge sources exceed the {} MiB limit while reading `{path}`",
                PACKAGE_SOURCE_LIMIT / (1024 * 1024)
            ),
        ));
    }
    Ok(bytes)
}

fn checked_source_total(
    current: usize,
    added: usize,
    path: &str,
) -> Result<usize, SpellforgePackageError> {
    current
        .checked_add(added)
        .filter(|&total| total <= PACKAGE_SOURCE_LIMIT)
        .ok_or_else(|| {
            SpellforgePackageError::new(
                SpellforgePackageErrorKind::SourceLimit,
                format!(
                    "Spellforge sources exceed the {} MiB limit while reading `{path}`",
                    PACKAGE_SOURCE_LIMIT / (1024 * 1024)
                ),
            )
        })
}

fn parse_contract(
    path: &str,
    bytes: &[u8],
) -> Result<SpellforgeScriptMode, SpellforgePackageError> {
    let manifest: ContractManifest = serde_json::from_slice(bytes).map_err(|error| {
        SpellforgePackageError::new(
            SpellforgePackageErrorKind::InvalidContract,
            format!("invalid `{path}`: {error}"),
        )
    })?;
    if manifest.contract_version != SPELLFORGE_CONTRACT_VERSION {
        return Err(SpellforgePackageError::new(
            SpellforgePackageErrorKind::InvalidContract,
            format!(
                "unsupported contract_version {}; expected {SPELLFORGE_CONTRACT_VERSION}",
                manifest.contract_version
            ),
        ));
    }
    match manifest.script_mode.as_deref().unwrap_or("replace") {
        "replace" => Ok(SpellforgeScriptMode::Replace),
        "augment_before" => Ok(SpellforgeScriptMode::AugmentBefore),
        "augment_after" => Ok(SpellforgeScriptMode::AugmentAfter),
        mode => Err(SpellforgePackageError::new(
            SpellforgePackageErrorKind::InvalidContract,
            format!("unsupported Spellforge script_mode `{mode}`"),
        )),
    }
}

fn invalid_archive(message: impl Into<String>) -> SpellforgePackageError {
    SpellforgePackageError::new(SpellforgePackageErrorKind::InvalidArchive, message)
}

fn le_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn le_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut output);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (path, bytes) in entries {
                writer.start_file(*path, options).unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        }
        output.into_inner()
    }

    #[test]
    fn exact_selected_language_controls_companion_and_libraries() {
        let mission = archive(&[
            ("English/DATA/Levels/H06_Lin_VL.rhm", b"rhm"),
            ("English/DATA/Levels/H06_Lin_VL.lua", b"return 'en'"),
            ("English/DATA/Levels/lib/common.lua", b"return 'en-lib'"),
            ("German/DATA/Levels/H06_Lin_VL.rhm", b"rhm"),
            ("German/DATA/Levels/H06_Lin_VL.lua", b"return 'de'"),
            ("German/DATA/Levels/lib/common.lua", b"return 'de-lib'"),
        ]);
        let package = build_package_from_archives(
            &mission,
            "German/DATA/Levels/H06_Lin_VL.rhm",
            "H06_Lin_VL",
            None,
        )
        .unwrap();
        assert_eq!(package.files["h06_lin_vl.lua"], b"return 'de'");
        assert_eq!(package.files["lib/common.lua"], b"return 'de-lib'");
    }

    #[test]
    fn mission_libraries_override_shared_archive_as_one_atomic_set() {
        let mission = archive(&[
            ("Mission.rhm", b"rhm"),
            ("Mission.lua", b"return 1"),
            ("lib/local.lua", b"return 2"),
        ]);
        let shared = archive(&[("lib/shared.lua", b"return 3")]);
        let package =
            build_package_from_archives(&mission, "Mission.rhm", "Mission", Some(&shared)).unwrap();
        assert!(package.files.contains_key("lib/local.lua"));
        assert!(!package.files.contains_key("lib/shared.lua"));
    }

    #[test]
    fn mission_sibling_modules_are_retained_but_other_companions_are_not() {
        let mission = archive(&[
            ("Wrapped/Mission.rhm", b"rhm"),
            (
                "Wrapped/Mission.lua",
                b"require('helpers'); require('lib/common')",
            ),
            ("Wrapped/helpers.lua", b"helper_value=17"),
            ("Wrapped/Other.rhm", b"other-rhm"),
            ("Wrapped/Other.lua", b"other_mission=true"),
        ]);
        let shared = archive(&[("lib/common.lua", b"common_value=23")]);
        let package =
            build_package_from_archives(&mission, "Wrapped/Mission.rhm", "Mission", Some(&shared))
                .unwrap();

        assert_eq!(package.files["helpers.lua"], b"helper_value=17");
        assert!(!package.files.contains_key("other.lua"));
        assert_eq!(package.files["lib/common.lua"], b"common_value=23");
    }

    #[test]
    fn selected_rhm_must_exist_even_when_its_companion_exists() {
        let mission = archive(&[("Mission.lua", b"return 1")]);
        let error = build_package_from_archives(&mission, "Mission.rhm", "Mission", None)
            .expect_err("a companion alone must not impersonate a selected mission");
        assert_eq!(error.kind, SpellforgePackageErrorKind::MissingMission);
    }

    #[test]
    fn duplicate_case_folded_paths_are_rejected() {
        let mission = archive(&[
            ("Mission.rhm", b"rhm"),
            ("Mission.lua", b"return 1"),
            ("MISSION.LUA", b"return 2"),
        ]);
        let error = build_package_from_archives(&mission, "Mission.rhm", "Mission", None)
            .expect_err("ambiguous paths must fail");
        assert_eq!(error.kind, SpellforgePackageErrorKind::DuplicatePath);
    }

    #[test]
    fn traversal_and_trailing_polyglot_bytes_are_rejected() {
        let unsafe_archive = archive(&[
            ("Mission.rhm", b"rhm"),
            ("Mission.lua", b"return 1"),
            ("lib/../escape.lua", b"return 2"),
        ]);
        let error = build_package_from_archives(&unsafe_archive, "Mission.rhm", "Mission", None)
            .expect_err("traversal must fail");
        assert_eq!(error.kind, SpellforgePackageErrorKind::UnsafePath);

        let mut trailing = archive(&[("Mission.rhm", b"rhm"), ("Mission.lua", b"return 1")]);
        trailing.extend_from_slice(b"polyglot");
        let error = build_package_from_archives(&trailing, "Mission.rhm", "Mission", None)
            .expect_err("trailing bytes must fail preflight");
        assert_eq!(error.kind, SpellforgePackageErrorKind::InvalidArchive);
    }
}
