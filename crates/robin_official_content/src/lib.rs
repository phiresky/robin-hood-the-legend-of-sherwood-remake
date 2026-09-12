//! Fail-closed filesystem closures for official projection sources.
//!
//! A retail install root also contains player saves, logs, executables, and
//! developer diagnostics. Hashing the complete tree therefore makes an
//! official-content receipt depend on mutable state which the engine cannot
//! consume while preparing static simulation inputs. This module selects the
//! inverse: the narrow loader-derived namespace that can affect those inputs,
//! rejects ambiguous filesystem shapes, and can copy it into an exact
//! read-only mount for the exporter.
//!
//! This filesystem proof is necessary but not sufficient: official export
//! initialization must mount only the sanitized root and must disable process
//! overlays, user profile/localization state, and the persistent sound-duration
//! cache. Those inputs are deliberately not laundered into this content proof.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use robin_assets::shipping_datadir::ShippingDatadir;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;

/// Stable identifier for the loader-derived loose/native closure.
pub const LOOSE_NATIVE_SOURCE_CLOSURE_V2: &str = "loose_native_simulation_inputs_v2";
/// Stable identifier for a decoded RHDDNA10 manifest and every external split
/// payload it authoritatively references.
pub const SHIPPING_DATADIR_SOURCE_CLOSURE_V2: &str = "shipping_datadir_v10_referenced_payloads_v2";

/// Compute the external split-file closure consumed by the current shipping
/// datadir schema. The protocol's `V10` source-format name is frozen wire
/// vocabulary; this selector deliberately reads current typed fields instead
/// of hard-coding a historical archive schema number.
pub fn shipping_projection_external_file_paths_v2(
    datadir: &ShippingDatadir,
) -> Result<Vec<String>> {
    fn insert_path(output: &mut BTreeMap<String, String>, candidate: &str) -> Result<()> {
        if candidate.is_empty()
            || candidate.starts_with('/')
            || candidate.ends_with('/')
            || candidate.contains('\\')
            || candidate.contains('\0')
            || candidate
                .split('/')
                .any(|component| component.is_empty() || matches!(component, "." | ".."))
        {
            bail!("shipping projection closure contains unsafe relative path {candidate:?}");
        }
        let folded = candidate.to_ascii_lowercase();
        if folded == "datadir.bin" {
            bail!("shipping projection split closure must not name datadir.bin");
        }
        if let Some(previous) = output.get(&folded) {
            if previous != candidate {
                bail!(
                    "shipping projection closure has case-colliding paths {previous:?} and {candidate:?}"
                );
            }
        } else {
            output.insert(folded, candidate.to_owned());
        }
        Ok(())
    }

    let mut closure = BTreeMap::new();
    for path in datadir
        .missions
        .values()
        .flat_map(|mission| mission.files.iter())
        .chain(datadir.character_rhs_files.values().flatten())
        .chain(datadir.character_audio_files.values().flatten())
        .chain(datadir.saved_world_rhs_files.iter())
    {
        insert_path(&mut closure, path)?;
    }
    let mut paths = closure.into_values().collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

/// Prove that a physical shipping source inventory contains exactly the
/// current typed archive plus every external payload it references.
pub fn validate_shipping_projection_source_relative_paths_v2(
    datadir: &ShippingDatadir,
    actual_paths: &[String],
) -> Result<()> {
    fn folded_unique<'a>(paths: &'a [String], label: &str) -> Result<BTreeMap<String, &'a str>> {
        let mut output = BTreeMap::new();
        for path in paths {
            let folded = path.to_ascii_lowercase();
            if let Some(previous) = output.insert(folded, path.as_str()) {
                bail!("{label} has case-colliding or duplicate paths {previous:?} and {path:?}");
            }
        }
        Ok(output)
    }

    let mut expected = shipping_projection_external_file_paths_v2(datadir)?
        .into_iter()
        .map(|path| format!("Data/{path}"))
        .collect::<Vec<_>>();
    expected.push("Data/datadir.bin".to_owned());
    expected.sort();
    let expected = folded_unique(&expected, "decoded shipping closure")?;
    let actual = folded_unique(actual_paths, "shipping source inventory")?;
    if actual.keys().ne(expected.keys()) {
        let missing = expected
            .keys()
            .filter(|path| !actual.contains_key(*path))
            .collect::<Vec<_>>();
        let unexpected = actual
            .keys()
            .filter(|path| !expected.contains_key(*path))
            .collect::<Vec<_>>();
        bail!(
            "shipping source inventory differs from decoded closure (missing: {missing:?}; unexpected: {unexpected:?})"
        );
    }
    Ok(())
}

/// One exact file in the selected private source inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceClosureFile {
    pub path: String,
    pub sha256: [u8; 32],
    pub byte_length: u64,
}

/// Sorted exact inventory selected from a potentially polluted retail root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LooseSourceClosureInventory {
    pub policy: String,
    pub resource_locale_root: String,
    pub files: Vec<SourceClosureFile>,
}

/// Sorted exact inventory selected from a v10 shipping export root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShippingSourceClosureInventory {
    pub policy: String,
    pub files: Vec<SourceClosureFile>,
}

impl ShippingSourceClosureInventory {
    pub fn sha256(&self) -> Result<[u8; 32]> {
        Ok(sha2::Sha256::digest(serde_json::to_vec(self)?).into())
    }
}

impl LooseSourceClosureInventory {
    /// Deterministic internal fingerprint used by hostile selector tests and
    /// by callers before adapting this inventory into a protocol document.
    pub fn sha256(&self) -> Result<[u8; 32]> {
        Ok(sha2::Sha256::digest(serde_json::to_vec(self)?).into())
    }
}

/// Inventory the exact loose/native namespace that can influence the static
/// prepared-mission projection.
///
/// `resource_locale_root` is the authenticated edition LCID (`1033` for the
/// demo and `2047` for the full game), not a host language preference.
pub fn inventory_loose_source_closure(
    source_root: &Path,
    resource_locale_root: &str,
) -> Result<LooseSourceClosureInventory> {
    validate_locale_component(resource_locale_root)?;
    validate_directory(source_root, "source root")?;

    let mut selected = BTreeMap::<String, PathBuf>::new();
    let data = resolve_unique_child(source_root, "Data", true)?
        .context("official loose source has no Data directory")?;
    if resolve_unique_child(&data, "datadir.bin", false)?.is_some() {
        bail!("loose/native source closure refuses Data/datadir.bin shipping metadata");
    }

    // These complete roots are loader-authoritative. In particular, loading
    // profiles scans every RHM for beam-me requirements, runtime dependency
    // closure can reach any shipped RHS/ambience fallback, and deterministic
    // sound timing reads the source samples rather than presentation caches.
    for directory in ["Levels", "Characters", "Animations", "Sounds", "Text"] {
        let path = resolve_unique_child(&data, directory, true)?
            .with_context(|| format!("official loose source has no Data/{directory}"))?;
        collect_tree(source_root, &path, &mut selected)?;
    }

    let configuration = resolve_unique_child(&data, "Configuration", true)?
        .context("official loose source has no Data/Configuration")?;
    let binary_profile = resolve_unique_child(&configuration, "profile.cpf", false)?;
    let json_profile = resolve_unique_child(&configuration, "profile.cpf.json", false)?;
    let profile = match (binary_profile, json_profile) {
        (Some(_), Some(_)) => bail!(
            "official loose source contains both profile.cpf and profile.cpf.json; loader precedence is not an acceptable provenance ambiguity"
        ),
        (Some(path), None) | (None, Some(path)) => path,
        (None, None) => bail!(
            "official loose source has neither Data/Configuration/profile.cpf nor profile.cpf.json"
        ),
    };
    collect_file(source_root, &profile, &mut selected)?;
    if resolve_unique_child(&configuration, "soldier-profiles.patch.json", false)?.is_some() {
        bail!("soldier-profiles.patch.json is no longer supported; migrate to profiles.patch.json");
    }
    if let Some(patch) = resolve_unique_child(&configuration, "profiles.patch.json", false)? {
        collect_file(source_root, &patch, &mut selected)?;
    }

    let interface = resolve_unique_child(&data, "Interface", true)?
        .context("official loose source has no Data/Interface")?;
    let default_res = resolve_unique_child(&interface, "DEFAULT.RES", false)?
        .context("official loose source has no Data/Interface/DEFAULT.RES")?;
    collect_file(source_root, &default_res, &mut selected)?;
    for bank in ["robinhood.bks", "robinhood.dic"] {
        let path = resolve_unique_child(&data, bank, false)?
            .with_context(|| format!("official loose source has no Data/{bank}"))?;
        collect_file(source_root, &path, &mut selected)?;
    }

    // SbFile only permits locales to overlay Text, Interface, recorded
    // exclamations, and cinematics. Static preparation consumes the first
    // three; cinematics are presentation-only and remain outside the closure.
    // Keep the complete selected subtrees so adding/removing an optional
    // fallback asset changes provenance instead of changing behavior silently.
    let locale = resolve_unique_child(source_root, resource_locale_root, true)?
        .with_context(|| format!("official loose source has no {resource_locale_root} locale"))?;
    let locale_data = resolve_unique_child(&locale, "Data", true)?.with_context(|| {
        format!("official loose source has no {resource_locale_root}/Data directory")
    })?;
    for directory in ["Text", "Interface"] {
        let path = resolve_unique_child(&locale_data, directory, true)?.with_context(|| {
            format!("official loose source has no {resource_locale_root}/Data/{directory}")
        })?;
        collect_tree(source_root, &path, &mut selected)?;
    }
    let locale_sounds = resolve_unique_child(&locale_data, "Sounds", true)?.with_context(|| {
        format!("official loose source has no {resource_locale_root}/Data/Sounds")
    })?;
    let exclamations =
        resolve_unique_child(&locale_sounds, "Exclamations", true)?.with_context(|| {
            format!("official loose source has no {resource_locale_root}/Data/Sounds/Exclamations")
        })?;
    collect_tree(source_root, &exclamations, &mut selected)?;

    let files = selected
        .into_iter()
        .map(|(path, native)| {
            let (sha256, byte_length) = hash_regular_file(&native)?;
            Ok(SourceClosureFile {
                path,
                sha256,
                byte_length,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if files.is_empty() {
        bail!("official loose source closure is empty");
    }
    Ok(LooseSourceClosureInventory {
        policy: LOOSE_NATIVE_SOURCE_CLOSURE_V2.to_owned(),
        resource_locale_root: resource_locale_root.to_owned(),
        files,
    })
}

/// Inventory `Data/datadir.bin` plus the exact external split payload union
/// decoded from that v10 manifest by the caller.
///
/// Dependency interpretation deliberately stays with the shipping loader. The
/// caller supplies the duplicate-free union of mission references, character
/// RHS references, character-audio references, and the saved-world RHS set;
/// this function only proves their filesystem identity. The caller must derive
/// that union from the same `datadir.bin` bytes whose returned file fact it
/// accepts, and must repeat the decode/union equality check from the sanitized
/// copy before mounting it. An arbitrary caller-provided subset is not proof of
/// shipping closure completeness.
pub fn inventory_shipping_source_closure(
    source_root: &Path,
    referenced_paths: &[String],
) -> Result<ShippingSourceClosureInventory> {
    validate_directory(source_root, "source root")?;
    let data = resolve_unique_child(source_root, "Data", true)?
        .context("shipping source has no Data directory")?;
    let datadir = resolve_unique_child(&data, "datadir.bin", false)?
        .context("shipping source has no Data/datadir.bin")?;

    let mut selected = BTreeMap::<String, PathBuf>::new();
    collect_file(source_root, &datadir, &mut selected)?;
    let mut logical_paths = BTreeMap::<String, String>::new();
    for referenced in referenced_paths {
        let normalized = normalize_shipping_reference(referenced)?;
        let folded = normalized.to_ascii_lowercase();
        if let Some(first) = logical_paths.insert(folded, normalized.clone()) {
            bail!(
                "shipping manifest contains duplicate or case-colliding references {first:?} and {normalized:?}"
            );
        }
        let path = resolve_exact_relative(&data, &normalized)?;
        collect_file(source_root, &path, &mut selected)?;
    }
    if referenced_paths.is_empty() {
        bail!("shipping v10 source closure has no referenced split payloads");
    }

    let files = selected
        .into_iter()
        .map(|(path, native)| {
            let (sha256, byte_length) = hash_regular_file(&native)?;
            Ok(SourceClosureFile {
                path,
                sha256,
                byte_length,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ShippingSourceClosureInventory {
        policy: SHIPPING_DATADIR_SOURCE_CLOSURE_V2.to_owned(),
        files,
    })
}

/// Copy an already inventoried closure into a fresh, exact, recursively
/// read-only mount. The exporter must mount this path, never the polluted
/// retail root from which it was selected.
pub fn materialize_loose_source_closure(
    source_root: &Path,
    destination_root: &Path,
    expected: &LooseSourceClosureInventory,
) -> Result<()> {
    if expected.policy != LOOSE_NATIVE_SOURCE_CLOSURE_V2 || expected.files.is_empty() {
        bail!("refusing to materialize an empty or unknown loose source closure policy");
    }
    materialize_files(source_root, destination_root, &expected.files)?;
    let actual = inventory_loose_source_closure(destination_root, &expected.resource_locale_root)?;
    if &actual != expected {
        let _ = make_tree_writable(destination_root);
        bail!("sanitized source closure differs from its selected inventory");
    }
    Ok(())
}

/// Materialize a decoded shipping closure into an exact recursively read-only
/// root. Unreferenced sibling payloads are intentionally not copied.
pub fn materialize_shipping_source_closure(
    source_root: &Path,
    destination_root: &Path,
    referenced_paths: &[String],
    expected: &ShippingSourceClosureInventory,
) -> Result<()> {
    if expected.policy != SHIPPING_DATADIR_SOURCE_CLOSURE_V2 || expected.files.is_empty() {
        bail!("refusing to materialize an empty or unknown shipping source closure policy");
    }
    materialize_files(source_root, destination_root, &expected.files)?;
    let actual = inventory_shipping_source_closure(destination_root, referenced_paths)?;
    if &actual != expected {
        let _ = make_tree_writable(destination_root);
        bail!("sanitized shipping source closure differs from its selected inventory");
    }
    Ok(())
}

fn materialize_files(
    source_root: &Path,
    destination_root: &Path,
    files: &[SourceClosureFile],
) -> Result<()> {
    validate_directory(source_root, "source root")?;
    if files.is_empty() {
        bail!("refusing to materialize an empty source closure");
    }
    if destination_root.exists() || fs::symlink_metadata(destination_root).is_ok() {
        bail!(
            "sanitized source closure destination already exists: {}",
            destination_root.display()
        );
    }
    let destination_parent = destination_root
        .parent()
        .context("sanitized source closure destination has no parent")?;
    validate_directory(destination_parent, "sanitized destination parent")?;
    let canonical_source = fs::canonicalize(source_root)?;
    let canonical_destination = fs::canonicalize(destination_parent)?.join(
        destination_root
            .file_name()
            .context("sanitized source closure destination has no final component")?,
    );
    if canonical_destination.starts_with(&canonical_source)
        || canonical_source.starts_with(&canonical_destination)
    {
        bail!("source and sanitized closure destination must not overlap");
    }

    let mut previous = None::<String>;
    let mut folded_paths = BTreeSet::new();
    for file in files {
        validate_relative_path(&file.path)?;
        if previous
            .as_deref()
            .is_some_and(|path| path >= file.path.as_str())
        {
            bail!("source closure inventory paths must be strictly sorted");
        }
        if !folded_paths.insert(file.path.to_ascii_lowercase()) {
            bail!("source closure inventory contains case-fold duplicate paths");
        }
        previous = Some(file.path.clone());
    }

    fs::create_dir(destination_root).with_context(|| {
        format!(
            "create sanitized source closure {}",
            destination_root.display()
        )
    })?;
    let result = (|| {
        let mut directories = BTreeSet::<PathBuf>::from([destination_root.to_path_buf()]);
        for expected_file in files {
            let source = source_root.join(&expected_file.path);
            validate_regular_file(&source)?;
            let destination = destination_root.join(&expected_file.path);
            let parent = destination
                .parent()
                .context("closure file destination has no parent")?;
            fs::create_dir_all(parent)
                .with_context(|| format!("create closure directory {}", parent.display()))?;
            let mut ancestor = Some(parent);
            while let Some(directory) = ancestor {
                if !directory.starts_with(destination_root) {
                    break;
                }
                directories.insert(directory.to_path_buf());
                if directory == destination_root {
                    break;
                }
                ancestor = directory.parent();
            }
            copy_verified(&source, &destination, expected_file)?;
            set_read_only(&destination)?;
        }

        // Make children read-only before their parents so no later creation is
        // needed below a sealed directory.
        let mut directories = directories.into_iter().collect::<Vec<_>>();
        directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for directory in directories {
            set_read_only(&directory)?;
        }
        validate_exact_materialized_tree(destination_root, files)?;
        Ok(())
    })();
    if result.is_err() {
        // Best effort only: the caller supplied a fresh dedicated destination.
        // TODO(source-closure): expose a recoverable trash handoff on desktop
        // instead of leaving a partial diagnostic tree for manual inspection.
        let _ = make_tree_writable(destination_root);
    }
    result
}

fn validate_exact_materialized_tree(root: &Path, expected: &[SourceClosureFile]) -> Result<()> {
    fn visit(root: &Path, directory: &Path, actual: &mut BTreeSet<String>) -> Result<()> {
        validate_directory(directory, "sanitized source directory")?;
        for entry in fs::read_dir(directory)
            .with_context(|| format!("read sanitized source directory {}", directory.display()))?
        {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                bail!(
                    "sanitized source tree contains a symlink: {}",
                    path.display()
                );
            }
            if metadata.is_dir() {
                visit(root, &path, actual)?;
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)?
                    .to_str()
                    .with_context(|| {
                        format!("sanitized source path is not UTF-8: {}", path.display())
                    })?
                    .replace('\\', "/");
                if !actual.insert(relative.clone()) {
                    bail!("sanitized source tree repeats file path {relative:?}");
                }
            } else {
                bail!(
                    "sanitized source tree contains a non-regular node: {}",
                    path.display()
                );
            }
        }
        Ok(())
    }

    let mut actual = BTreeSet::new();
    visit(root, root, &mut actual)?;
    let expected = expected
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    if actual != expected {
        bail!("sanitized source tree does not contain exactly the inventoried files");
    }
    Ok(())
}

fn validate_locale_component(locale: &str) -> Result<()> {
    if locale.is_empty() || !locale.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("resource locale root must be one non-empty numeric path component");
    }
    validate_relative_path(locale)
}

fn validate_relative_path(path: &str) -> Result<()> {
    robin_util::asset_fs::validate_canonical_relative_path(path)
        .with_context(|| format!("source closure path is not a canonical relative path: {path}"))
}

fn normalize_shipping_reference(path: &str) -> Result<String> {
    if path.is_empty()
        || path.trim() != path
        || path
            .chars()
            .any(|character| character.is_control() || matches!(character, ':' | '?' | '#' | '%'))
    {
        bail!(
            "shipping reference is empty, untrimmed, or contains a forbidden character: {path:?}"
        );
    }
    let normalized = path.replace('\\', "/");
    if normalized.starts_with('/')
        || normalized
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        bail!("shipping reference contains an invalid component: {path:?}");
    }
    validate_relative_path(&normalized)?;
    Ok(normalized)
}

fn resolve_exact_relative(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_relative_path(relative)?;
    let mut resolved = root.to_path_buf();
    let components = Path::new(relative).components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(wanted) = component else {
            unreachable!("validate_relative_path admitted a non-normal component")
        };
        let wanted = wanted
            .to_str()
            .with_context(|| format!("shipping manifest path is not UTF-8: {relative:?}"))?;
        validate_directory(&resolved, "shipping reference parent")?;
        let mut case_matches = Vec::new();
        let mut exact = None;
        for entry in fs::read_dir(&resolved)
            .with_context(|| format!("read shipping directory {}", resolved.display()))?
        {
            let entry = entry?;
            let name = entry.file_name();
            if let Some(name_utf8) = name.to_str()
                && name_utf8.eq_ignore_ascii_case(wanted)
            {
                if name_utf8 == wanted {
                    exact = Some(entry.path());
                }
                case_matches.push(name);
            }
        }
        if case_matches.len() > 1 {
            bail!(
                "shipping reference {relative:?} is case-ambiguous below {}: {:?}",
                resolved.display(),
                case_matches
            );
        }
        let path = exact.with_context(|| {
            if case_matches.is_empty() {
                format!("shipping reference is missing: {relative}")
            } else {
                format!("shipping reference casing differs from its manifest spelling: {relative}")
            }
        })?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("shipping reference traverses a symlink: {}", path.display());
        }
        let is_last = index + 1 == components.len();
        if (is_last && !metadata.is_file()) || (!is_last && !metadata.is_dir()) {
            bail!(
                "shipping reference component has the wrong node type: {}",
                path.display()
            );
        }
        resolved = path;
    }
    Ok(resolved)
}

fn validate_directory(path: &Path, description: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect {description} {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "{description} must be a non-symlink directory: {}",
            path.display()
        );
    }
    Ok(())
}

fn validate_regular_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect selected source file {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "selected source path is not a regular non-symlink file: {}",
            path.display()
        );
    }
    Ok(())
}

fn resolve_unique_child(parent: &Path, wanted: &str, directory: bool) -> Result<Option<PathBuf>> {
    validate_directory(parent, "selected source directory")?;
    let mut matches = Vec::new();
    for entry in fs::read_dir(parent)
        .with_context(|| format!("read selected source directory {}", parent.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name_utf8) = name.to_str() else {
            // An out-of-closure sibling cannot affect lookup unless its byte
            // name case-folds to the selected component, which ASCII paths do
            // not. Selected subtrees reject non-UTF-8 names in collect_tree.
            continue;
        };
        if name_utf8.eq_ignore_ascii_case(wanted) {
            matches.push((name, entry.path()));
        }
    }
    if matches.len() > 1 {
        let names = matches
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        bail!(
            "case-insensitive source lookup for {}/{} is ambiguous: {:?}",
            parent.display(),
            wanted,
            names
        );
    }
    let Some((_, path)) = matches.pop() else {
        return Ok(None);
    };
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        bail!(
            "selected source {} is not a non-symlink {}",
            path.display(),
            if directory {
                "directory"
            } else {
                "regular file"
            }
        );
    }
    Ok(Some(path))
}

fn collect_tree(
    source_root: &Path,
    directory: &Path,
    selected: &mut BTreeMap<String, PathBuf>,
) -> Result<()> {
    validate_directory(directory, "selected source directory")?;
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("read selected source directory {}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);

    let mut folded = BTreeMap::<String, OsString>::new();
    for entry in &entries {
        let name = entry.file_name();
        let utf8 = name
            .to_str()
            .with_context(|| format!("selected source name is not UTF-8: {:?}", entry.path()))?;
        let key = utf8.to_ascii_lowercase();
        if let Some(first) = folded.insert(key, name.clone()) {
            bail!(
                "selected source directory {} contains case-fold duplicate names {:?} and {:?}",
                directory.display(),
                first,
                name
            );
        }
    }

    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("selected source tree contains symlink: {}", path.display());
        }
        if metadata.is_dir() {
            collect_tree(source_root, &path, selected)?;
        } else if metadata.is_file() {
            collect_file(source_root, &path, selected)?;
        } else {
            bail!(
                "selected source tree contains non-regular node: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn collect_file(
    source_root: &Path,
    path: &Path,
    selected: &mut BTreeMap<String, PathBuf>,
) -> Result<()> {
    validate_regular_file(path)?;
    let relative = path
        .strip_prefix(source_root)
        .with_context(|| format!("selected path escaped source root: {}", path.display()))?
        .to_str()
        .with_context(|| format!("selected source path is not UTF-8: {}", path.display()))?
        .replace('\\', "/");
    validate_relative_path(&relative)?;
    let folded = relative.to_ascii_lowercase();
    if let Some(existing) = selected
        .keys()
        .find(|candidate| candidate.to_ascii_lowercase() == folded)
    {
        bail!("selected source paths are case-fold duplicates: {existing:?} and {relative:?}");
    }
    selected.insert(relative, path.to_path_buf());
    Ok(())
}

fn hash_regular_file(path: &Path) -> Result<([u8; 32], u64)> {
    validate_regular_file(path)?;
    let mut file =
        File::open(path).with_context(|| format!("open source file {}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        length = length
            .checked_add(u64::try_from(count)?)
            .context("selected source file exceeds u64 length")?;
    }
    Ok((hasher.finalize().into(), length))
}

fn copy_verified(source: &Path, destination: &Path, expected: &SourceClosureFile) -> Result<()> {
    let mut input = File::open(source)
        .with_context(|| format!("open selected source file {}", source.display()))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .with_context(|| format!("create sanitized source file {}", destination.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
        length = length
            .checked_add(u64::try_from(count)?)
            .context("selected source file exceeds u64 length")?;
    }
    output.sync_all()?;
    let sha256: [u8; 32] = hasher.finalize().into();
    if length != expected.byte_length || sha256 != expected.sha256 {
        bail!(
            "selected source file changed while materializing closure: {}",
            source.display()
        );
    }
    Ok(())
}

fn set_read_only(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("make sanitized closure path read-only: {}", path.display()))
}

fn make_tree_writable(root: &Path) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let mut paths = vec![root.to_path_buf()];
    let mut cursor = 0;
    while cursor < paths.len() {
        let path = paths[cursor].clone();
        cursor += 1;
        if fs::symlink_metadata(&path)?.is_dir() {
            for entry in fs::read_dir(&path)? {
                paths.push(entry?.path());
            }
        }
    }
    paths.sort_by_key(|path| path.components().count());
    for path in paths {
        let mut permissions = fs::metadata(&path)?.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(permissions.mode() | 0o200);
        }
        #[cfg(not(unix))]
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipping_selector_uses_current_typed_reference_fields() {
        let mut datadir = ShippingDatadir::default();
        datadir.missions.insert(
            "mission".into(),
            robin_assets::shipping_datadir::ShippingMissionRef {
                forest_level: false,
                files: vec!["missions/mission.bin".into(), "terrain/shared.bin".into()],
            },
        );
        datadir
            .character_rhs_files
            .insert(1, vec!["rhs/robin.bin".into()]);
        datadir
            .character_audio_files
            .insert(1, vec!["audio/robin.bin".into()]);
        datadir
            .saved_world_rhs_files
            .push("rhs/saved-world.bin".into());

        let references = shipping_projection_external_file_paths_v2(&datadir).unwrap();
        assert_eq!(
            references,
            [
                "audio/robin.bin",
                "missions/mission.bin",
                "rhs/robin.bin",
                "rhs/saved-world.bin",
                "terrain/shared.bin",
            ]
        );
        let mut inventory = references
            .iter()
            .map(|path| format!("Data/{path}"))
            .collect::<Vec<_>>();
        inventory.push("Data/datadir.bin".into());
        inventory.sort();
        validate_shipping_projection_source_relative_paths_v2(&datadir, &inventory).unwrap();

        inventory.push("Data/unexpected.bin".into());
        assert!(
            validate_shipping_projection_source_relative_paths_v2(&datadir, &inventory).is_err()
        );
    }

    #[test]
    fn shipping_selector_rejects_unsafe_and_case_colliding_references() {
        let mut datadir = ShippingDatadir::default();
        datadir
            .character_rhs_files
            .insert(1, vec!["rhs/Robin.bin".into(), "rhs/robin.bin".into()]);
        assert!(shipping_projection_external_file_paths_v2(&datadir).is_err());

        datadir
            .character_rhs_files
            .insert(1, vec!["../escape".into()]);
        assert!(shipping_projection_external_file_paths_v2(&datadir).is_err());
    }

    fn write(root: &Path, relative: &str, bytes: &[u8]) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for (path, bytes) in [
            ("Data/Levels/M01.rhm", b"mission".as_slice()),
            ("Data/Characters/Robin.rhs", b"character"),
            ("Data/Animations/Day/Robin.rhs", b"animation"),
            ("Data/Sounds/robin hood.fxg", b"fx"),
            ("Data/Text/RHLevelAA.red", b"red"),
            ("Data/Configuration/profile.cpf", b"profile"),
            ("Data/Interface/DEFAULT.RES", b"interface"),
            ("Data/robinhood.bks", b"bank"),
            ("Data/robinhood.dic", b"dictionary"),
            ("1033/Data/Text/Level.res", b"localized text"),
            ("1033/Data/Interface/Start.sxt", b"localized interface"),
            (
                "1033/Data/Sounds/Exclamations/actors.res",
                b"localized speech",
            ),
        ] {
            write(root.path(), path, bytes);
        }
        root
    }

    fn shipping_fixture() -> (tempfile::TempDir, Vec<String>) {
        let root = tempfile::tempdir().unwrap();
        for (path, bytes) in [
            ("Data/datadir.bin", b"RHDDNA10".as_slice()),
            ("Data/Missions/001/mission.bin", b"mission"),
            ("Data/Characters/Robin.rhs", b"character"),
            ("Data/Audio/Robin/attack.wav", b"audio"),
            ("Data/SavedWorld/guards.rhs", b"saved world"),
            ("Data/Unreferenced/debug.json", b"out of closure"),
        ] {
            write(root.path(), path, bytes);
        }
        let references = [
            "Missions/001/mission.bin",
            "Characters/Robin.rhs",
            "Audio/Robin/attack.wav",
            "SavedWorld/guards.rhs",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        (root, references)
    }

    #[test]
    fn every_selected_file_mutation_changes_inventory() {
        let root = fixture();
        let original = inventory_loose_source_closure(root.path(), "1033").unwrap();
        let original_digest = original.sha256().unwrap();
        for selected in &original.files {
            let path = root.path().join(&selected.path);
            let bytes = fs::read(&path).unwrap();
            let mut mutated = bytes.clone();
            mutated.push(0xa5);
            fs::write(&path, mutated).unwrap();
            let changed = inventory_loose_source_closure(root.path(), "1033").unwrap();
            assert_ne!(
                changed.sha256().unwrap(),
                original_digest,
                "{}",
                selected.path
            );
            fs::write(path, bytes).unwrap();
        }
        assert_eq!(
            inventory_loose_source_closure(root.path(), "1033").unwrap(),
            original
        );

        write(
            root.path(),
            "Data/Levels/Optional.rhm",
            b"new selected input",
        );
        assert_ne!(
            inventory_loose_source_closure(root.path(), "1033")
                .unwrap()
                .sha256()
                .unwrap(),
            original_digest,
            "adding a file inside a loader-authoritative root must change provenance"
        );
    }

    #[test]
    fn generic_profile_patch_is_bound_by_official_inventory() {
        let root = fixture();
        let original = inventory_loose_source_closure(root.path(), "1033")
            .unwrap()
            .sha256()
            .unwrap();
        let path = "Data/Configuration/profiles.patch.json";
        write(root.path(), path, b"[]");
        let patched = inventory_loose_source_closure(root.path(), "1033").unwrap();
        assert!(patched.files.iter().any(|file| file.path == path));
        assert_ne!(patched.sha256().unwrap(), original);
    }

    #[test]
    fn mutable_and_presentation_only_siblings_are_explicitly_out_of_closure() {
        let root = fixture();
        let original = inventory_loose_source_closure(root.path(), "1033").unwrap();
        for relative in [
            "Campaign.bck",
            "Savegame/Profile_001/Continue",
            ".codex-tmp/parity-dumps/m01.json",
            "debug.jsonl",
            "sblibng.log",
            "Manual/index.html",
            "lib32/libogg.so",
            "Data/Savegame/profiles.json",
            "Data/sblibng.log",
            "Data/Musics/Menu.wav",
            "Data/Cinematics/Intro.ogg",
            "Data/Configuration/keyset1.cfg",
            "Data/Configuration/profile.json",
            "Data/Interface/Loading.pak",
            "1033/Data/Cinematics/Intro.ogg",
            "2047/Data/Text/Level.res",
        ] {
            write(root.path(), relative, b"first");
            assert_eq!(
                inventory_loose_source_closure(root.path(), "1033").unwrap(),
                original,
                "adding {relative} must not alter the selected closure"
            );
            write(root.path(), relative, b"second");
            assert_eq!(
                inventory_loose_source_closure(root.path(), "1033").unwrap(),
                original,
                "mutating {relative} must not alter the selected closure"
            );
        }
    }

    #[test]
    fn profile_representation_is_exactly_one() {
        let root = fixture();
        write(root.path(), "Data/Configuration/profile.cpf.json", b"{}");
        let error = inventory_loose_source_closure(root.path(), "1033").unwrap_err();
        assert!(error.to_string().contains("both profile.cpf"));

        fs::remove_file(root.path().join("Data/Configuration/profile.cpf")).unwrap();
        assert!(inventory_loose_source_closure(root.path(), "1033").is_ok());
    }

    #[test]
    fn loose_lane_refuses_shipping_metadata() {
        let root = fixture();
        write(root.path(), "Data/datadir.bin", b"RHDDNA10");
        assert!(
            inventory_loose_source_closure(root.path(), "1033")
                .unwrap_err()
                .to_string()
                .contains("refuses Data/datadir.bin")
        );
    }

    #[cfg(unix)]
    #[test]
    fn selected_casefold_duplicates_and_ambiguous_roots_fail() {
        let root = fixture();
        write(root.path(), "Data/Levels/m01.RHM", b"duplicate");
        assert!(
            inventory_loose_source_closure(root.path(), "1033")
                .unwrap_err()
                .to_string()
                .contains("case-fold duplicate")
        );

        fs::remove_file(root.path().join("Data/Levels/m01.RHM")).unwrap();
        fs::create_dir(root.path().join("data")).unwrap();
        assert!(
            inventory_loose_source_closure(root.path(), "1033")
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_nonregular_nodes_inside_selected_roots_fail() {
        use std::os::unix::fs::symlink;
        use std::os::unix::net::UnixListener;

        let root = fixture();
        symlink("M01.rhm", root.path().join("Data/Levels/alias.rhm")).unwrap();
        assert!(
            inventory_loose_source_closure(root.path(), "1033")
                .unwrap_err()
                .to_string()
                .contains("symlink")
        );

        fs::remove_file(root.path().join("Data/Levels/alias.rhm")).unwrap();
        let socket = root.path().join("Data/Levels/not-a-file.sock");
        let _listener = UnixListener::bind(&socket).unwrap();
        assert!(
            inventory_loose_source_closure(root.path(), "1033")
                .unwrap_err()
                .to_string()
                .contains("non-regular")
        );
    }

    #[cfg(unix)]
    #[test]
    fn writable_cleanup_restores_only_owner_write_permission() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("content");
        fs::write(&file, b"content").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o440)).unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o550)).unwrap();

        make_tree_writable(root.path()).unwrap();

        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(
            fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
            0o750
        );
    }

    #[test]
    fn materialized_mount_contains_exactly_selected_read_only_files() {
        let root = fixture();
        write(root.path(), "Data/Savegame/profiles.json", b"mutable");
        let expected = inventory_loose_source_closure(root.path(), "1033").unwrap();
        let destination_parent = tempfile::tempdir().unwrap();
        let destination = destination_parent.path().join("closure");
        materialize_loose_source_closure(root.path(), &destination, &expected).unwrap();

        assert_eq!(
            inventory_loose_source_closure(&destination, "1033").unwrap(),
            expected
        );
        assert!(!destination.join("Data/Savegame").exists());
        for selected in &expected.files {
            assert!(destination.join(&selected.path).is_file());
            assert!(
                fs::metadata(destination.join(&selected.path))
                    .unwrap()
                    .permissions()
                    .readonly()
            );
        }

        // Restore permissions so TempDir can remove the diagnostic fixture on
        // platforms that enforce directory read-only bits during deletion.
        make_tree_writable(&destination).unwrap();
    }

    #[test]
    fn shipping_inventory_binds_metadata_and_every_referenced_payload_only() {
        let (root, references) = shipping_fixture();
        let original = inventory_shipping_source_closure(root.path(), &references).unwrap();
        let original_digest = original.sha256().unwrap();
        assert_eq!(original.files.len(), references.len() + 1);

        for selected in &original.files {
            let path = root.path().join(&selected.path);
            let bytes = fs::read(&path).unwrap();
            let mut mutated = bytes.clone();
            mutated.push(0xa5);
            fs::write(&path, mutated).unwrap();
            assert_ne!(
                inventory_shipping_source_closure(root.path(), &references)
                    .unwrap()
                    .sha256()
                    .unwrap(),
                original_digest,
                "{}",
                selected.path
            );
            fs::write(path, bytes).unwrap();
        }

        write(
            root.path(),
            "Data/Unreferenced/debug.json",
            b"still out of closure",
        );
        write(root.path(), "Data/Unreferenced/new.bin", b"also ignored");
        assert_eq!(
            inventory_shipping_source_closure(root.path(), &references).unwrap(),
            original
        );
    }

    #[test]
    fn shipping_manifest_references_fail_closed() {
        let (root, references) = shipping_fixture();
        for invalid in [
            "",
            "../escape.bin",
            "/absolute.bin",
            "Missions//001/mission.bin",
            "Missions/001/mission.bin ",
            "Missions/001/missing.bin",
            "missions/001/mission.bin",
        ] {
            assert!(
                inventory_shipping_source_closure(root.path(), &[invalid.to_owned()]).is_err(),
                "reference {invalid:?} must fail"
            );
        }

        let mut duplicate = references.clone();
        duplicate.push(references[0].clone());
        assert!(inventory_shipping_source_closure(root.path(), &duplicate).is_err());

        let mut case_collision = references;
        case_collision.push("missions/001/mission.bin".to_owned());
        assert!(inventory_shipping_source_closure(root.path(), &case_collision).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn shipping_references_reject_ambiguous_symlink_and_nonregular_nodes() {
        use std::os::unix::fs::symlink;
        use std::os::unix::net::UnixListener;

        let (root, references) = shipping_fixture();
        write(
            root.path(),
            "Data/Missions/001/MISSION.BIN",
            b"case collision",
        );
        assert!(
            inventory_shipping_source_closure(root.path(), &references)
                .unwrap_err()
                .to_string()
                .contains("case-ambiguous")
        );
        fs::remove_file(root.path().join("Data/Missions/001/MISSION.BIN")).unwrap();

        fs::remove_file(root.path().join("Data/Missions/001/mission.bin")).unwrap();
        symlink(
            "../../../Characters/Robin.rhs",
            root.path().join("Data/Missions/001/mission.bin"),
        )
        .unwrap();
        assert!(
            inventory_shipping_source_closure(root.path(), &references)
                .unwrap_err()
                .to_string()
                .contains("symlink")
        );
        fs::remove_file(root.path().join("Data/Missions/001/mission.bin")).unwrap();

        let socket_path = root.path().join("Data/Missions/001/mission.bin");
        let _listener = UnixListener::bind(&socket_path).unwrap();
        assert!(
            inventory_shipping_source_closure(root.path(), &references)
                .unwrap_err()
                .to_string()
                .contains("wrong node type")
        );
    }

    #[test]
    fn shipping_materialized_mount_is_exact_and_read_only() {
        let (root, references) = shipping_fixture();
        let expected = inventory_shipping_source_closure(root.path(), &references).unwrap();
        let destination_parent = tempfile::tempdir().unwrap();
        let destination = destination_parent.path().join("shipping-closure");
        materialize_shipping_source_closure(root.path(), &destination, &references, &expected)
            .unwrap();

        assert_eq!(
            inventory_shipping_source_closure(&destination, &references).unwrap(),
            expected
        );
        assert!(!destination.join("Data/Unreferenced").exists());
        for selected in &expected.files {
            let metadata = fs::metadata(destination.join(&selected.path)).unwrap();
            assert!(metadata.is_file());
            assert!(metadata.permissions().readonly());
        }
        make_tree_writable(&destination).unwrap();
    }

    #[test]
    #[ignore = "requires a locally licensed official install"]
    fn authentic_loose_source_shape_is_accepted_without_embedding_content() {
        let source = std::env::var_os("ROBINHOOD_DATA_DIR")
            .expect("set ROBINHOOD_DATA_DIR to the official install root");
        let locale = std::env::var("ROBINHOOD_OFFICIAL_LOCALE_ROOT")
            .expect("set ROBINHOOD_OFFICIAL_LOCALE_ROOT to 1033 or 2047");
        let inventory = inventory_loose_source_closure(Path::new(&source), &locale).unwrap();
        assert!(!inventory.files.is_empty());
        assert!(
            inventory
                .files
                .iter()
                .any(|file| file.path.eq_ignore_ascii_case("Data/robinhood.bks"))
        );
        assert!(inventory.files.iter().any(|file| {
            file.path
                .eq_ignore_ascii_case(&format!("{locale}/Data/Text/Level.res"))
        }));
    }
}
