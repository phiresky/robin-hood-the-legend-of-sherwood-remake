//! Exact shared-manifest validation for operator-mounted official content.
//!
//! The offer pins the canonical digest of
//! [`robin_run_protocol::ContentManifestV1`]. The verifier validates that
//! exact shared document and every file it names; there is deliberately no
//! second verifier-specific content identity which could be substituted for
//! the signed one.

use robin_run_protocol::{
    CanonicalDocument as _, ContentManifestV1, Digest32, OfficialContentEditionV1,
    OfficialContentSubjectV1, SimulationContentComponentDocumentV1,
    SimulationContentComponentKindV1, SimulationContentComponentV1, Validate as _,
    simulation_component_filename_v1, simulation_content_component_relative_path_v1,
};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

const HASH_BUFFER_BYTES: usize = 128 * 1024;
const MAX_COMPONENT_DOCUMENT_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("shared content manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("content manifest digest mismatch; expected {expected}, got {actual}")]
    ManifestIdentityMismatch {
        expected: Digest32,
        actual: Digest32,
    },
    #[error("content manifest edition mismatch; expected {expected:?}, got {actual:?}")]
    EditionMismatch {
        expected: OfficialContentEditionV1,
        actual: OfficialContentEditionV1,
    },
    #[error("content manifest subject mismatch; expected {expected:?}, got {actual:?}")]
    SubjectMismatch {
        expected: OfficialContentSubjectV1,
        actual: OfficialContentSubjectV1,
    },
    #[error("content mount `{0}` is not an existing directory")]
    MountMissing(PathBuf),
    #[error("content mount `{0}` is not mounted read-only")]
    MountNotReadOnly(PathBuf),
    #[error("checking read-only content mount `{path}` failed: {message}")]
    MountCheck { path: PathBuf, message: String },
    #[error("content manifest contains an unsafe path `{0}`")]
    UnsafePath(String),
    #[error("content tree contains non-UTF-8 path `{0}`")]
    NonUtf8Path(PathBuf),
    #[error("content tree contains unsupported symlink or special file `{0}`")]
    SpecialFile(PathBuf),
    #[error("content tree contains unexpected file `{0}`")]
    UnexpectedFile(String),
    #[error("content tree is missing manifest file `{0}`")]
    MissingFile(String),
    #[error("content file `{path}` has {actual} bytes; expected {expected}")]
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    #[error("content file `{path}` SHA-256 mismatch; expected {expected}, got {actual}")]
    FileHashMismatch {
        path: String,
        expected: Digest32,
        actual: Digest32,
    },
    #[error("content component `{path}` is too large ({actual} bytes)")]
    ComponentTooLarge { path: String, actual: u64 },
    #[error("content component `{path}` is not the exact canonical typed document: {message}")]
    InvalidComponent { path: String, message: String },
    #[error("content I/O failed for `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("walking content mount `{path}` failed: {message}")]
    Walk { path: PathBuf, message: String },
}

/// Capability returned only after the signed shared manifest, whole mounted
/// tree, and read-only-mount invariant all pass.
///
/// This capability remains valid only inside the verifier's private mount
/// namespace. Production also makes the mount inaccessible to the untrusted
/// job uid for mutation; the read-only filesystem check is repeated after
/// hashing to detect an operator/deployment mistake.
#[derive(Debug)]
pub struct ValidatedContentMount {
    root: PathBuf,
    edition: OfficialContentEditionV1,
    subject: OfficialContentSubjectV1,
    manifest_sha256: Digest32,
    manifest: ContentManifestV1,
    documents: BTreeMap<SimulationContentComponentKindV1, SimulationContentComponentDocumentV1>,
}

impl ValidatedContentMount {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub const fn edition(&self) -> OfficialContentEditionV1 {
        self.edition
    }

    pub const fn subject(&self) -> &OfficialContentSubjectV1 {
        &self.subject
    }

    pub const fn manifest_sha256(&self) -> Digest32 {
        self.manifest_sha256
    }

    pub const fn manifest(&self) -> &ContentManifestV1 {
        &self.manifest
    }

    pub fn component(
        &self,
        kind: SimulationContentComponentKindV1,
    ) -> &SimulationContentComponentDocumentV1 {
        self.documents
            .get(&kind)
            .expect("validated content mount contains every required component")
    }

    pub fn ordered_documents(&self) -> Vec<SimulationContentComponentDocumentV1> {
        [
            SimulationContentComponentKindV1::Profiles,
            SimulationContentComponentKindV1::LoadedLevel,
            SimulationContentComponentKindV1::MissionScripts,
            SimulationContentComponentKindV1::SpriteSimulationMetadata,
            SimulationContentComponentKindV1::MapGeometryMetadata,
            SimulationContentComponentKindV1::LocalizedDeterministicText,
            SimulationContentComponentKindV1::SoundDurationTables,
            SimulationContentComponentKindV1::InterfaceSimulationMetadata,
        ]
        .into_iter()
        .map(|kind| self.component(kind).clone())
        .collect()
    }
}

/// Validate the exact canonical manifest pinned by a submission against its
/// sanitized, no-extra subject directory inside an operator catalog. The
/// subject directory is resolved exclusively through the shared protocol path
/// helper; raw mission IDs are never joined into a filesystem path.
pub fn validate_content_mount(
    catalog_root: &Path,
    expected_edition: OfficialContentEditionV1,
    expected_subject: &OfficialContentSubjectV1,
    manifest: &ContentManifestV1,
    expected_manifest_sha256: Digest32,
) -> Result<ValidatedContentMount, ManifestError> {
    validate_content_mount_inner(
        catalog_root,
        expected_edition,
        expected_subject,
        manifest,
        expected_manifest_sha256,
        true,
    )
}

fn validate_content_mount_inner(
    catalog_root: &Path,
    expected_edition: OfficialContentEditionV1,
    expected_subject: &OfficialContentSubjectV1,
    manifest: &ContentManifestV1,
    expected_manifest_sha256: Digest32,
    require_read_only: bool,
) -> Result<ValidatedContentMount, ManifestError> {
    manifest
        .validate()
        .map_err(|error| ManifestError::InvalidManifest(error.to_string()))?;
    let manifest_sha256 = manifest
        .canonical_digest()
        .map_err(|error| ManifestError::InvalidManifest(error.to_string()))?;
    if manifest_sha256 != expected_manifest_sha256 {
        return Err(ManifestError::ManifestIdentityMismatch {
            expected: expected_manifest_sha256,
            actual: manifest_sha256,
        });
    }
    if manifest.edition != expected_edition {
        return Err(ManifestError::EditionMismatch {
            expected: expected_edition,
            actual: manifest.edition,
        });
    }
    if &manifest.subject != expected_subject {
        return Err(ManifestError::SubjectMismatch {
            expected: expected_subject.clone(),
            actual: manifest.subject.clone(),
        });
    }

    let root = subject_root(catalog_root, expected_subject)?;
    let root_metadata = std::fs::symlink_metadata(&root).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            ManifestError::MountMissing(root.clone())
        } else {
            ManifestError::Io {
                path: root.clone(),
                source,
            }
        }
    })?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(ManifestError::MountMissing(root));
    }
    if require_read_only && !mount_is_read_only(&root)? {
        return Err(ManifestError::MountNotReadOnly(root));
    }

    let expected = validate_manifest_inventory(manifest)?;
    let expected_paths = expected.keys().cloned().collect::<BTreeSet<_>>();
    let actual_paths = enumerate_regular_files(&root)?;
    if let Some(path) = actual_paths.difference(&expected_paths).next() {
        return Err(ManifestError::UnexpectedFile(path.clone()));
    }
    if let Some(path) = expected_paths.difference(&actual_paths).next() {
        return Err(ManifestError::MissingFile(path.clone()));
    }

    let mut documents = BTreeMap::new();
    for (relative, entry) in expected {
        let absolute = root.join(path_from_manifest(&relative)?);
        let metadata =
            std::fs::symlink_metadata(&absolute).map_err(|source| ManifestError::Io {
                path: absolute.clone(),
                source,
            })?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(ManifestError::SpecialFile(absolute));
        }
        if metadata.len() != entry.artifact.byte_length {
            return Err(ManifestError::SizeMismatch {
                path: relative,
                expected: entry.artifact.byte_length,
                actual: metadata.len(),
            });
        }
        let actual = hash_file(&absolute)?;
        if actual != entry.artifact.sha256 {
            return Err(ManifestError::FileHashMismatch {
                path: relative,
                expected: entry.artifact.sha256,
                actual,
            });
        }
        if entry.artifact.byte_length > MAX_COMPONENT_DOCUMENT_BYTES {
            return Err(ManifestError::ComponentTooLarge {
                path: relative,
                actual: entry.artifact.byte_length,
            });
        }
        let bytes = std::fs::read(&absolute).map_err(|source| ManifestError::Io {
            path: absolute.clone(),
            source,
        })?;
        let document =
            SimulationContentComponentDocumentV1::from_bitcode(&bytes).map_err(|error| {
                ManifestError::InvalidComponent {
                    path: relative.clone(),
                    message: error.to_string(),
                }
            })?;
        document
            .validate()
            .map_err(|error| ManifestError::InvalidComponent {
                path: relative.clone(),
                message: error.to_string(),
            })?;
        if document.kind != entry.kind
            || document.component_schema_version != entry.component_schema_version
        {
            return Err(ManifestError::InvalidComponent {
                path: relative,
                message: "component kind/schema does not match manifest".into(),
            });
        }
        let canonical =
            document
                .bitcode_bytes()
                .map_err(|error| ManifestError::InvalidComponent {
                    path: relative.clone(),
                    message: error.to_string(),
                })?;
        if canonical != bytes {
            return Err(ManifestError::InvalidComponent {
                path: relative,
                message: "component bytes are not canonical bitcode".into(),
            });
        }
        documents.insert(entry.kind, document);
    }
    if require_read_only && !mount_is_read_only(&root)? {
        return Err(ManifestError::MountNotReadOnly(root));
    }

    Ok(ValidatedContentMount {
        root,
        edition: manifest.edition,
        subject: manifest.subject.clone(),
        manifest_sha256,
        manifest: manifest.clone(),
        documents,
    })
}

fn validate_manifest_inventory(
    manifest: &ContentManifestV1,
) -> Result<BTreeMap<String, SimulationContentComponentV1>, ManifestError> {
    let mut expected = BTreeMap::new();
    for entry in &manifest.components {
        let normalized = simulation_component_filename_v1(entry.kind).to_owned();
        if expected.insert(normalized.clone(), entry.clone()).is_some() {
            return Err(ManifestError::InvalidManifest(format!(
                "duplicate content path {normalized}"
            )));
        }
    }
    Ok(expected)
}

fn subject_root(
    catalog_root: &Path,
    subject: &OfficialContentSubjectV1,
) -> Result<PathBuf, ManifestError> {
    let first = simulation_content_component_relative_path_v1(
        subject,
        SimulationContentComponentKindV1::Profiles,
    )
    .map_err(|error| ManifestError::InvalidManifest(error.to_string()))?;
    let first = path_from_manifest(&first)?;
    let relative_root = first.parent().ok_or_else(|| {
        ManifestError::InvalidManifest("component path has no subject directory".into())
    })?;
    for kind in [
        SimulationContentComponentKindV1::LoadedLevel,
        SimulationContentComponentKindV1::MissionScripts,
        SimulationContentComponentKindV1::SpriteSimulationMetadata,
        SimulationContentComponentKindV1::MapGeometryMetadata,
        SimulationContentComponentKindV1::LocalizedDeterministicText,
        SimulationContentComponentKindV1::SoundDurationTables,
        SimulationContentComponentKindV1::InterfaceSimulationMetadata,
    ] {
        let relative = simulation_content_component_relative_path_v1(subject, kind)
            .map_err(|error| ManifestError::InvalidManifest(error.to_string()))?;
        let relative = path_from_manifest(&relative)?;
        if relative.parent() != Some(relative_root) {
            return Err(ManifestError::InvalidManifest(
                "component paths disagree on subject directory".into(),
            ));
        }
    }
    Ok(catalog_root.join(relative_root))
}

fn enumerate_regular_files(root: &Path) -> Result<BTreeSet<String>, ManifestError> {
    let mut paths = BTreeSet::new();
    for item in WalkDir::new(root).follow_links(false).sort_by_file_name() {
        let item = item.map_err(|error| ManifestError::Walk {
            path: error.path().unwrap_or(root).to_path_buf(),
            message: error.to_string(),
        })?;
        if item.path() == root {
            continue;
        }
        let file_type = item.file_type();
        if file_type.is_dir() {
            continue;
        }
        if !file_type.is_file() {
            return Err(ManifestError::SpecialFile(item.path().to_path_buf()));
        }
        let relative = item
            .path()
            .strip_prefix(root)
            .expect("WalkDir entry must remain below its root");
        let relative = relative
            .to_str()
            .ok_or_else(|| ManifestError::NonUtf8Path(relative.to_path_buf()))?;
        let normalized =
            normalize_manifest_path(&relative.replace(std::path::MAIN_SEPARATOR, "/"))?;
        if !paths.insert(normalized.clone()) {
            return Err(ManifestError::InvalidManifest(format!(
                "duplicate filesystem path {normalized}"
            )));
        }
    }
    Ok(paths)
}

fn normalize_manifest_path(path: &str) -> Result<String, ManifestError> {
    if path.is_empty() || path.contains('\0') || path.contains('\\') {
        return Err(ManifestError::UnsafePath(path.to_owned()));
    }
    let parsed = Path::new(path);
    if parsed.is_absolute() {
        return Err(ManifestError::UnsafePath(path.to_owned()));
    }
    let mut components = Vec::new();
    for component in parsed.components() {
        match component {
            Component::Normal(part) if part != OsStr::new("") => {
                let part = part
                    .to_str()
                    .ok_or_else(|| ManifestError::UnsafePath(path.to_owned()))?;
                if part == "." || part == ".." {
                    return Err(ManifestError::UnsafePath(path.to_owned()));
                }
                components.push(part);
            }
            _ => return Err(ManifestError::UnsafePath(path.to_owned())),
        }
    }
    if components.is_empty() {
        return Err(ManifestError::UnsafePath(path.to_owned()));
    }
    Ok(components.join("/"))
}

fn path_from_manifest(path: &str) -> Result<PathBuf, ManifestError> {
    let normalized = normalize_manifest_path(path)?;
    Ok(normalized.split('/').collect())
}

fn hash_file(path: &Path) -> Result<Digest32, ManifestError> {
    let file = File::open(path).map_err(|source| ManifestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Digest32::digest_reader(BufReader::with_capacity(HASH_BUFFER_BYTES, file)).map_err(|source| {
        ManifestError::Io {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(unix)]
pub(crate) fn mount_is_read_only(path: &Path) -> Result<bool, ManifestError> {
    use nix::sys::statvfs::{FsFlags, statvfs};

    let stats = statvfs(path).map_err(|error| ManifestError::MountCheck {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    Ok(stats.flags().contains(FsFlags::ST_RDONLY))
}

#[cfg(not(unix))]
pub(crate) fn mount_is_read_only(path: &Path) -> Result<bool, ManifestError> {
    Err(ManifestError::MountCheck {
        path: path.to_path_buf(),
        message: "read-only mount verification is not implemented on this platform".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_run_protocol::{
        ArtifactRefV1, CanonicalValue, ContentClosureKindV1, SCHEMA_VERSION_V1,
        SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1, SimulationSpeechTimingSourceV1,
    };

    fn artifact(bytes: &[u8]) -> ArtifactRefV1 {
        ArtifactRefV1 {
            sha256: Digest32::digest_bytes(bytes),
            byte_length: bytes.len() as u64,
            media_type: SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1.into(),
        }
    }

    fn document(kind: SimulationContentComponentKindV1) -> SimulationContentComponentDocumentV1 {
        SimulationContentComponentDocumentV1 {
            schema_version: SCHEMA_VERSION_V1,
            kind,
            component_schema_version: 1,
            payload: CanonicalValue::String(format!("fixture-{kind:?}")),
        }
    }

    fn component(kind: SimulationContentComponentKindV1) -> SimulationContentComponentV1 {
        let bytes = document(kind).bitcode_bytes().unwrap();
        SimulationContentComponentV1 {
            kind,
            component_schema_version: 1,
            artifact: artifact(&bytes),
        }
    }

    fn subject() -> OfficialContentSubjectV1 {
        OfficialContentSubjectV1::FieldMission {
            mission_id: "Mission1".into(),
        }
    }

    fn manifest() -> ContentManifestV1 {
        let kinds = [
            SimulationContentComponentKindV1::Profiles,
            SimulationContentComponentKindV1::LoadedLevel,
            SimulationContentComponentKindV1::MissionScripts,
            SimulationContentComponentKindV1::SpriteSimulationMetadata,
            SimulationContentComponentKindV1::MapGeometryMetadata,
            SimulationContentComponentKindV1::LocalizedDeterministicText,
            SimulationContentComponentKindV1::SoundDurationTables,
            SimulationContentComponentKindV1::InterfaceSimulationMetadata,
        ];
        ContentManifestV1 {
            schema_version: SCHEMA_VERSION_V1,
            name: "official-demo-fixture".into(),
            edition: OfficialContentEditionV1::Demo,
            subject: subject(),
            closure: ContentClosureKindV1::StaticPreparedMissionContentProjection,
            projection_schema_version: 2,
            resource_locale_root: robin_run_protocol::ResourceLocaleRootV1::new("1033").unwrap(),
            speech_timing: SimulationSpeechTimingSourceV1::BaseInstallation,
            components: kinds.into_iter().map(component).collect(),
        }
    }

    fn write_tree(root: &Path, manifest: &ContentManifestV1) {
        let root = subject_root(root, &manifest.subject).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        for component in &manifest.components {
            std::fs::write(
                root.join(simulation_component_filename_v1(component.kind)),
                document(component.kind).bitcode_bytes().unwrap(),
            )
            .unwrap();
        }
    }

    fn validate_test_tree(
        root: &Path,
        manifest: &ContentManifestV1,
    ) -> Result<ValidatedContentMount, ManifestError> {
        validate_content_mount_inner(
            root,
            OfficialContentEditionV1::Demo,
            &subject(),
            manifest,
            manifest.canonical_digest().unwrap(),
            false,
        )
    }

    #[test]
    fn exact_shared_manifest_tree_validates() {
        let dir = tempfile::tempdir().unwrap();
        let approved = manifest();
        write_tree(dir.path(), &approved);

        let validated = validate_test_tree(dir.path(), &approved).unwrap();
        assert_eq!(validated.edition(), OfficialContentEditionV1::Demo);
        assert_eq!(validated.subject(), &subject());
        assert_eq!(
            validated.manifest_sha256(),
            approved.canonical_digest().unwrap()
        );
        assert_eq!(validated.manifest(), &approved);
        assert_eq!(
            validated.root(),
            subject_root(dir.path(), &subject()).unwrap()
        );
        assert_eq!(
            validated.component(SimulationContentComponentKindV1::LoadedLevel),
            &document(SimulationContentComponentKindV1::LoadedLevel)
        );
    }

    #[test]
    fn signed_manifest_identity_cannot_be_substituted() {
        let dir = tempfile::tempdir().unwrap();
        let approved = manifest();
        write_tree(dir.path(), &approved);
        let wrong = Digest32::digest_bytes(b"different shared manifest");
        assert!(matches!(
            validate_content_mount_inner(
                dir.path(),
                OfficialContentEditionV1::Demo,
                &subject(),
                &approved,
                wrong,
                false,
            ),
            Err(ManifestError::ManifestIdentityMismatch { .. })
        ));
    }

    #[test]
    fn changed_missing_and_extra_files_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let approved = manifest();
        write_tree(dir.path(), &approved);
        let path = subject_root(dir.path(), &subject())
            .unwrap()
            .join("profiles.bitcode");

        std::fs::write(&path, b"changed").unwrap();
        assert!(matches!(
            validate_test_tree(dir.path(), &approved),
            Err(ManifestError::SizeMismatch { .. }) | Err(ManifestError::FileHashMismatch { .. })
        ));
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(
            validate_test_tree(dir.path(), &approved),
            Err(ManifestError::MissingFile(path)) if path == "profiles.bitcode"
        ));
        std::fs::write(
            &path,
            document(SimulationContentComponentKindV1::Profiles)
                .bitcode_bytes()
                .unwrap(),
        )
        .unwrap();
        std::fs::write(
            subject_root(dir.path(), &subject())
                .unwrap()
                .join("extra.bin"),
            b"extra",
        )
        .unwrap();
        assert!(matches!(
            validate_test_tree(dir.path(), &approved),
            Err(ManifestError::UnexpectedFile(path)) if path == "extra.bin"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_never_followed() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let mut outside = tempfile::NamedTempFile::new().unwrap();
        let approved = manifest();
        write_tree(dir.path(), &approved);
        let path = subject_root(dir.path(), &subject())
            .unwrap()
            .join("profiles.bitcode");
        std::fs::remove_file(&path).unwrap();
        std::io::Write::write_all(
            &mut outside,
            &document(SimulationContentComponentKindV1::Profiles)
                .bitcode_bytes()
                .unwrap(),
        )
        .unwrap();
        symlink(outside.path(), path).unwrap();
        assert!(matches!(
            validate_test_tree(dir.path(), &approved),
            Err(ManifestError::SpecialFile(_))
        ));
    }

    #[test]
    fn traversal_noncanonical_order_and_unknown_fields_are_rejected() {
        for path in [
            "",
            "/absolute",
            "../escape",
            "Data/../escape",
            "Data\\asset",
        ] {
            assert!(matches!(
                normalize_manifest_path(path),
                Err(ManifestError::UnsafePath(_))
            ));
        }

        let extra = r#"{"schema_version":1,"name":"demo","files":[],"kind":"demo"}"#;
        assert!(serde_json::from_str::<ContentManifestV1>(extra).is_err());
    }

    #[test]
    fn edition_subject_and_noncanonical_component_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let approved = manifest();
        write_tree(dir.path(), &approved);
        assert!(matches!(
            validate_content_mount_inner(
                dir.path(),
                OfficialContentEditionV1::Full,
                &subject(),
                &approved,
                approved.canonical_digest().unwrap(),
                false,
            ),
            Err(ManifestError::EditionMismatch { .. })
        ));
        let wrong_subject = OfficialContentSubjectV1::Headquarters {
            mission_id: "Headquarters".into(),
        };
        assert!(matches!(
            validate_content_mount_inner(
                dir.path(),
                OfficialContentEditionV1::Demo,
                &wrong_subject,
                &approved,
                approved.canonical_digest().unwrap(),
                false,
            ),
            Err(ManifestError::SubjectMismatch { .. })
        ));

        let path = subject_root(dir.path(), &subject())
            .unwrap()
            .join("profiles.bitcode");
        let canonical = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!(" {canonical}")).unwrap();
        let mut noncanonical_manifest = approved.clone();
        noncanonical_manifest.components[0].artifact = artifact(&std::fs::read(&path).unwrap());
        assert!(matches!(
            validate_test_tree(dir.path(), &noncanonical_manifest),
            Err(ManifestError::InvalidComponent { .. })
        ));
    }
}
