//! cloudflare responsibilities of the admitted release pipeline.
use super::*;

#[cfg(target_os = "linux")]
pub(super) struct CloudflareMaterializationOutputBuilderV1 {
    pub(super) directories: BTreeMap<String, fs::File>,
    pub(super) files: BTreeMap<String, fs::File>,
}

#[cfg(target_os = "linux")]
impl CloudflareMaterializationOutputBuilderV1 {
    pub(super) fn new(staging: &PinnedPublicationStagingV3) -> Result<Self> {
        Ok(Self {
            directories: BTreeMap::from([(".".to_owned(), staging.root.try_clone()?)]),
            files: BTreeMap::new(),
        })
    }

    pub(super) fn create_directory(&mut self, path: &str) -> Result<()> {
        use rustix::fs::{Mode, mkdirat};
        use std::os::fd::AsFd as _;

        ensure!(
            path != "." && valid_publication_relative_path_v3(path),
            "invalid Cloudflare materialization directory {path}"
        );
        ensure!(
            !self.directories.contains_key(path),
            "duplicate Cloudflare materialization directory {path}"
        );
        let relative = Path::new(path);
        let parent = relative
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(path_to_manifest)
            .transpose()?
            .unwrap_or_else(|| ".".to_owned());
        let parent = self
            .directories
            .get(&parent)
            .with_context(|| format!("Cloudflare materialization omits parent of {path}"))?;
        let name = relative
            .file_name()
            .context("Cloudflare materialization directory has no basename")?;
        mkdirat(parent.as_fd(), name, Mode::RUSR | Mode::WUSR | Mode::XUSR)?;
        let directory = open_publication_child_v3(parent, Path::new(name))?;
        ensure!(
            directory.metadata()?.is_dir(),
            "Cloudflare materialization directory is not a directory"
        );
        self.directories.insert(path.to_owned(), directory);
        Ok(())
    }

    pub(super) fn create_file(&mut self, path: &str) -> Result<fs::File> {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::fd::AsFd as _;

        ensure!(
            valid_publication_relative_path_v3(path) && !self.files.contains_key(path),
            "invalid or duplicate Cloudflare materialization file {path}"
        );
        let relative = Path::new(path);
        let parent_key = relative
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(path_to_manifest)
            .transpose()?
            .unwrap_or_else(|| ".".to_owned());
        let parent = self
            .directories
            .get(&parent_key)
            .with_context(|| format!("Cloudflare materialization omits parent of {path}"))?;
        let name = relative
            .file_name()
            .context("Cloudflare materialization file has no basename")?;
        let descriptor = openat2(
            parent.as_fd(),
            Path::new(name),
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::CREATE | OFlags::EXCL,
            Mode::RUSR | Mode::WUSR,
            ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV,
        )?;
        let file = fs::File::from(descriptor);
        self.files.insert(path.to_owned(), file.try_clone()?);
        Ok(file)
    }

    pub(super) fn write_bytes(&mut self, path: &str, bytes: &[u8]) -> Result<ArtifactRefV1> {
        let mut file = self.create_file(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(ArtifactRefV1 {
            sha256: Digest32::digest_bytes(bytes),
            byte_length: u64::try_from(bytes.len())?,
            media_type: "application/json".into(),
        })
    }

    pub(super) fn ensure_exact_retained_nodes(
        &self,
        inventory: &PublicationTreeInventoryV3,
    ) -> Result<()> {
        ensure!(
            self.files.len() == inventory.files.len()
                && self.directories.len() == inventory.directories.len(),
            "Cloudflare materialization retained output topology is incomplete"
        );
        for file in &inventory.files {
            let retained = self.files.get(&file.path).with_context(|| {
                format!("Cloudflare materialization did not retain {}", file.path)
            })?;
            ensure!(
                publication_node_identity_v3(&retained.metadata()?) == file.identity,
                "Cloudflare materialization file inode was substituted at {}",
                file.path
            );
        }
        for ((path, retained), (identity_path, identity)) in
            self.directories.iter().zip(&inventory.directory_identities)
        {
            ensure!(
                path == identity_path
                    && publication_node_identity_v3(&retained.metadata()?) == *identity,
                "Cloudflare materialization directory inode was substituted at {path}"
            );
        }
        Ok(())
    }
}

pub(super) fn cloudflare_origin_inventory_v1(
    origin: CloudflareMaterializationOriginV1,
    files: Vec<(String, ArtifactRefV1)>,
) -> Result<CloudflareMaterializedOriginInventoryV1> {
    let mut admitted = BTreeMap::new();
    for (path, artifact) in files {
        ensure!(
            valid_publication_relative_path_v3(&path),
            "invalid Cloudflare materialization path {path}"
        );
        artifact.validate()?;
        ensure!(
            admitted.insert(path.clone(), artifact).is_none(),
            "Cloudflare materialization repeats typed path {path}"
        );
    }
    ensure!(
        !admitted.is_empty(),
        "Cloudflare materialization origin is empty"
    );
    let mut directories = BTreeSet::from([".".to_owned()]);
    for path in admitted.keys() {
        let mut parent = Path::new(path).parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            directories.insert(path_to_manifest(directory)?);
            parent = directory.parent();
        }
    }
    let inventory = CloudflareMaterializedOriginInventoryV1 {
        schema_version: CLOUDFLARE_MATERIALIZATION_SCHEMA_VERSION,
        origin,
        root: origin.root().into(),
        files: admitted
            .into_iter()
            .map(|(path, artifact)| CloudflareMaterializedFileV1 {
                path,
                artifact,
                unix_mode: 0o444,
            })
            .collect(),
        directories: directories
            .into_iter()
            .map(|path| PublicationDirectoryV3 {
                path,
                unix_mode: 0o555,
            })
            .collect(),
    };
    inventory.validate()?;
    Ok(inventory)
}

pub(super) fn derive_cloudflare_origin_inventories_v1(
    publication: &ValidatedPublicationV3,
    manifest: &PublicationManifestV3,
    build: &BuildManifestV2,
) -> Result<Vec<CloudflareMaterializedOriginInventoryV1>> {
    let build_path = format!("manifests/builds/{}.json", manifest.build_manifest_sha256);
    let mut public = vec![(
        build_path,
        ArtifactRefV1 {
            sha256: manifest.build_manifest_sha256,
            byte_length: u64::try_from(canonical_json_bytes(build)?.len())?,
            media_type: "application/json".into(),
        },
    )];
    public.extend(
        build
            .viewer
            .engine
            .artifacts
            .iter()
            .map(|named| {
                Ok((
                    build_artifact_object_path_v1(manifest.build_manifest_sha256, named)?,
                    named.artifact.clone(),
                ))
            })
            .collect::<Result<Vec<_>>>()?,
    );
    public.extend(
        manifest
            .public_static_files
            .iter()
            .map(|file| (file.published_path.clone(), file.artifact.clone())),
    );
    let signer = manifest
        .identity_signer_files
        .iter()
        .map(|file| (file.published_path.clone(), file.artifact.clone()))
        .collect();
    let deployment = [
        "exposure-v3.json",
        "datadir-authority.json",
        "datadir-deployment.json",
    ]
    .into_iter()
    .map(|relative| {
        Ok((
            relative.to_owned(),
            publication.artifact(&format!("deployment/{relative}"), "application/json")?,
        ))
    })
    .collect::<Result<Vec<_>>>()?;
    let inventories = vec![
        cloudflare_origin_inventory_v1(CloudflareMaterializationOriginV1::Public, public)?,
        cloudflare_origin_inventory_v1(CloudflareMaterializationOriginV1::IdentitySigner, signer)?,
        cloudflare_origin_inventory_v1(
            CloudflareMaterializationOriginV1::DeploymentAuthority,
            deployment,
        )?,
    ];
    for inventory in &inventories {
        let actual_files = publication
            .relative_files(&inventory.root)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let expected_files = inventory
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<BTreeSet<_>>();
        ensure!(
            actual_files == expected_files,
            "PublicationV3 {} origin differs from its typed materialization file closure",
            inventory.root
        );
        let actual_directories = publication
            .relative_directories(&inventory.root)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let expected_directories = inventory
            .directories
            .iter()
            .map(|directory| directory.path.clone())
            .collect::<BTreeSet<_>>();
        ensure!(
            actual_directories == expected_directories,
            "PublicationV3 {} origin differs from its typed materialization directory closure",
            inventory.root
        );
        for file in &inventory.files {
            ensure!(
                publication.artifact(
                    &format!("{}/{}", inventory.root, file.path),
                    &file.artifact.media_type,
                )? == file.artifact,
                "PublicationV3 typed materialization artifact differs at {}/{}",
                inventory.root,
                file.path
            );
        }
    }
    Ok(inventories)
}

pub(super) fn register_cloudflare_origin_v1(
    expected: &mut ExpectedPublicationTopologyV3,
    inventory: &CloudflareMaterializedOriginInventoryV1,
) -> Result<()> {
    for directory in &inventory.directories {
        let path = if directory.path == "." {
            inventory.root.clone()
        } else {
            format!("{}/{}", inventory.root, directory.path)
        };
        expected.register_directory(&path)?;
    }
    for file in &inventory.files {
        expected.register_file(
            format!("{}/{}", inventory.root, file.path),
            &file.artifact,
            false,
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(super) struct CloudflareMaterializationProvenanceV1 {
    pub(super) source_commit: String,
    pub(super) source_tree_sha1: String,
    pub(super) cargo_lock_sha256: Digest32,
    pub(super) publication_manifest_sha256: Digest32,
    pub(super) publication_lock_sha256: Digest32,
}

#[cfg(target_os = "linux")]
pub(super) fn resolve_cloudflare_materialization_git_authority_v1(
    repository: &fs::File,
) -> Result<(String, String)> {
    use std::os::fd::AsRawFd as _;

    let repository_fd = format!("/proc/self/fd/{}", repository.as_raw_fd());
    let resolve = |revision: &str| -> Result<String> {
        let git = std::process::Command::new("/usr/bin/git")
            .args([
                "--no-replace-objects",
                "rev-parse",
                "--verify",
                "--end-of-options",
                revision,
            ])
            .current_dir(&repository_fd)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .env_remove("GIT_NAMESPACE")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .with_context(|| {
                format!("resolve exact Cloudflare materialization Git revision {revision}")
            })?;
        ensure!(
            git.status.success(),
            "resolve Cloudflare materialization Git revision {revision} failed: {}",
            String::from_utf8_lossy(&git.stderr)
        );
        let resolved = std::str::from_utf8(&git.stdout)
            .context("Git authority output is not UTF-8")?
            .strip_suffix('\n')
            .context("Git authority output omits its one terminal newline")?;
        ensure!(
            valid_lower_hex(resolved, 40),
            "Cloudflare materialization Git revision {revision} is not one exact SHA-1"
        );
        Ok(resolved.to_owned())
    };
    let source_commit = resolve("HEAD^{commit}")?;
    let source_tree_sha1 = resolve(&format!("{source_commit}^{{tree}}"))?;
    Ok((source_commit, source_tree_sha1))
}

#[cfg(target_os = "linux")]
pub(super) fn load_cloudflare_materialization_provenance_v1(
    publication: &mut ValidatedPublicationV3,
    expected_publication_lock_sha256: Digest32,
    repo_root: &Path,
) -> Result<(
    PublicationManifestV3,
    BuildManifestV2,
    CloudflareMaterializationProvenanceV1,
)> {
    use std::os::unix::fs::MetadataExt as _;

    ensure!(
        publication.lock_sha256() == expected_publication_lock_sha256,
        "PublicationV3 lock differs from the independently approved digest"
    );
    let manifest: PublicationManifestV3 =
        publication.load_document("publication-manifest-v3.json")?;
    let publication_manifest_sha256 = manifest.canonical_digest()?;
    let build_path = format!(
        "backend/manifests/builds/{}.json",
        manifest.build_manifest_sha256
    );
    let build: BuildManifestV2 = publication.load_document(&build_path)?;
    ensure!(
        build.canonical_digest()? == manifest.build_manifest_sha256
            && valid_lower_hex(&build.source_commit, 40),
        "PublicationV3 BuildManifestV2 has invalid source authority"
    );

    ensure!(
        repo_root.is_absolute(),
        "Cloudflare materialization repository root must be absolute"
    );
    let repository = open_publication_root_v3(repo_root)?;
    let repository_identity = publication_node_identity_v3(&repository.metadata()?);
    ensure!(
        repository.metadata()?.is_dir()
            && repository.metadata()?.uid() == rustix::process::geteuid().as_raw(),
        "Cloudflare materialization repository is not an owned directory"
    );
    let mut cargo_lock = open_publication_child_v3(&repository, Path::new("Cargo.lock"))?;
    let cargo_identity = publication_node_identity_v3(&cargo_lock.metadata()?);
    ensure!(
        cargo_lock.metadata()?.is_file()
            && cargo_lock.metadata()?.uid() == rustix::process::geteuid().as_raw()
            && cargo_lock.metadata()?.dev() == repository.metadata()?.dev()
            && cargo_lock.metadata()?.nlink() == 1,
        "Cloudflare materialization Cargo.lock is not an owned same-device singleton"
    );
    let cargo_artifact =
        stable_publication_file_artifact_v3(&mut cargo_lock, &cargo_identity, "Cargo.lock")?;
    ensure!(
        cargo_artifact.sha256 == build.cargo_lock_sha256,
        "Cloudflare materialization Cargo.lock differs from BuildManifestV2"
    );

    let (source_commit, source_tree_sha1) =
        resolve_cloudflare_materialization_git_authority_v1(&repository)?;
    ensure!(
        source_commit == build.source_commit,
        "Cloudflare materialization checkout differs from BuildManifestV2"
    );

    let rebound_repository = open_publication_root_v3(repo_root)?;
    let rebound_cargo = open_publication_child_v3(&repository, Path::new("Cargo.lock"))?;
    ensure!(
        publication_same_stable_node_v3(
            &publication_node_identity_v3(&rebound_repository.metadata()?),
            &repository_identity,
        ) && publication_node_identity_v3(&rebound_cargo.metadata()?) == cargo_identity,
        "Cloudflare materialization repository authority changed during admission"
    );
    publication.ensure_live()?;
    Ok((
        manifest,
        build,
        CloudflareMaterializationProvenanceV1 {
            source_commit,
            source_tree_sha1,
            cargo_lock_sha256: cargo_artifact.sha256,
            publication_manifest_sha256,
            publication_lock_sha256: expected_publication_lock_sha256,
        },
    ))
}

pub(super) fn same_artifact_bytes_v1(left: &ArtifactRefV1, right: &ArtifactRefV1) -> bool {
    left.sha256 == right.sha256 && left.byte_length == right.byte_length
}

pub(super) fn expected_topology_from_materialized_inventory_v1(
    inventory: &CloudflareMaterializedTreeInventoryV1,
) -> Result<ExpectedPublicationTopologyV3> {
    inventory.validate()?;
    let mut expected = ExpectedPublicationTopologyV3::new_with_root_mode(0o555);
    for directory in &inventory.directories {
        if directory.path != "." {
            expected.register_directory(&directory.path)?;
        }
    }
    for file in &inventory.files {
        expected.register_file(file.path.clone(), &file.artifact, false)?;
    }
    ensure!(
        expected.directories.keys().eq(inventory
            .directories
            .iter()
            .map(|directory| &directory.path)),
        "Cloudflare materialization inventory has an untyped empty directory"
    );
    Ok(expected)
}

pub(super) fn register_prefixed_materialized_origin_v1(
    expected: &mut ExpectedPublicationTopologyV3,
    inventory: &CloudflareMaterializedOriginInventoryV1,
) -> Result<()> {
    inventory.validate()?;
    for directory in &inventory.directories {
        let path = if directory.path == "." {
            inventory.root.clone()
        } else {
            format!("{}/{}", inventory.root, directory.path)
        };
        expected.register_directory(&path)?;
    }
    for file in &inventory.files {
        expected.register_file(
            format!("{}/{}", inventory.root, file.path),
            &file.artifact,
            false,
        )?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(super) fn load_cloudflare_materialization_canonical_document_v1<T>(
    inventory: &mut PublicationTreeInventoryV3,
    path: &str,
) -> Result<T>
where
    T: DeserializeOwned + Serialize,
{
    let bytes = read_inventory_file_v3(inventory, path, MAX_DOCUMENT_BYTES)?;
    let document: T = strict_json_from_slice(&bytes)
        .with_context(|| format!("parse Cloudflare materialization document {path}"))?;
    ensure!(
        canonical_json_bytes(&document)? == bytes,
        "Cloudflare materialization document {path} is not canonical JSON"
    );
    Ok(document)
}

#[cfg(target_os = "linux")]
pub(super) fn derive_cloudflare_materialized_origin_authority_v1(
    inventory: &mut PublicationTreeInventoryV3,
    receipt: &CloudflarePublicationMaterializationV1,
    admitted: &BTreeMap<CloudflareMaterializationOriginV1, CloudflareMaterializedOriginInventoryV1>,
) -> Result<Vec<CloudflareMaterializedOriginInventoryV1>> {
    let public = admitted
        .get(&CloudflareMaterializationOriginV1::Public)
        .context("Cloudflare materialization omits its public origin inventory")?;
    let build_paths = public
        .files
        .iter()
        .filter_map(|file| {
            let name = file.path.strip_prefix("manifests/builds/")?;
            let digest = name.strip_suffix(".json")?;
            (valid_lower_hex(digest, 64) && file.artifact.media_type == "application/json")
                .then_some((file.path.clone(), digest.to_owned(), file.artifact.clone()))
        })
        .collect::<Vec<_>>();
    ensure!(
        build_paths.len() == 1,
        "Cloudflare materialization public origin must contain one canonical BuildManifestV2"
    );
    let (build_path, build_digest, claimed_build_artifact) = &build_paths[0];
    let full_build_path = format!("cloudflare-public/{build_path}");
    let build: BuildManifestV2 = load_inventory_document_v3(inventory, &full_build_path)?;
    validate_current_official_ranked_build_v2(&build)?;
    let build_bytes = canonical_json_bytes(&build)?;
    let build_sha256 = Digest32::digest_bytes(&build_bytes);
    ensure!(
        build_sha256.to_string() == *build_digest
            && receipt.source_commit == build.source_commit
            && receipt.cargo_lock_sha256 == build.cargo_lock_sha256
            && *claimed_build_artifact
                == ArtifactRefV1 {
                    sha256: build_sha256,
                    byte_length: u64::try_from(build_bytes.len())?,
                    media_type: "application/json".into(),
                },
        "Cloudflare materialization receipt/build authority is inconsistent"
    );

    let mut public_files = vec![(build_path.clone(), claimed_build_artifact.clone())];
    public_files.extend(
        build
            .viewer
            .engine
            .artifacts
            .iter()
            .map(|named| {
                Ok((
                    build_artifact_object_path_v1(build_sha256, named)?,
                    named.artifact.clone(),
                ))
            })
            .collect::<Result<Vec<_>>>()?,
    );
    public_files.extend(
        build
            .viewer
            .pages_shell
            .public_origin_artifacts
            .iter()
            .map(|file| (file.path.clone(), file.artifact.clone())),
    );
    let signer_files = build
        .viewer
        .identity_signer
        .identity_signer_origin_artifacts
        .iter()
        .map(|file| (file.path.clone(), file.artifact.clone()))
        .collect();

    let exposure_path = "deployment/exposure-v3.json";
    let datadir_authority_path = "deployment/datadir-authority.json";
    let datadir_receipt_path = "deployment/datadir-deployment.json";
    let _: DeploymentExposureV3 = load_inventory_document_v3(inventory, exposure_path)?;
    let datadir_authority: DatadirReleaseAuthorityV1 =
        load_cloudflare_materialization_canonical_document_v1(inventory, datadir_authority_path)?;
    let datadir_receipt: DatadirDeploymentReceiptV1 =
        load_cloudflare_materialization_canonical_document_v1(inventory, datadir_receipt_path)?;
    let datadir_authority_artifact =
        inventory_artifact_v3(inventory, datadir_authority_path, "application/json")?;
    validate_datadir_binding(
        &datadir_authority,
        datadir_authority_artifact.sha256,
        &datadir_receipt,
    )?;
    let deployment_files = [
        (
            "exposure-v3.json".to_owned(),
            inventory_artifact_v3(inventory, exposure_path, "application/json")?,
        ),
        (
            "datadir-authority.json".to_owned(),
            datadir_authority_artifact,
        ),
        (
            "datadir-deployment.json".to_owned(),
            inventory_artifact_v3(inventory, datadir_receipt_path, "application/json")?,
        ),
    ];
    let expected = vec![
        cloudflare_origin_inventory_v1(CloudflareMaterializationOriginV1::Public, public_files)?,
        cloudflare_origin_inventory_v1(
            CloudflareMaterializationOriginV1::IdentitySigner,
            signer_files,
        )?,
        cloudflare_origin_inventory_v1(
            CloudflareMaterializationOriginV1::DeploymentAuthority,
            deployment_files.into_iter().collect(),
        )?,
    ];
    ensure!(
        expected
            .iter()
            .all(|origin| admitted.get(&origin.origin) == Some(origin)),
        "Cloudflare materialization origin inventories differ from their embedded typed authority"
    );
    Ok(expected)
}

#[cfg(target_os = "linux")]
pub(super) fn validate_cloudflare_materialization_inventory_v1(
    root_path: &Path,
    root: &fs::File,
    expected_receipt_sha256: Digest32,
) -> Result<(
    CloudflarePublicationMaterializationV1,
    PublicationTreeInventoryV3,
)> {
    let mut inventory = publication_tree_inventory_v3_from_fd(root_path, root)?;
    let initial_snapshot = inventory.snapshot();
    let receipt: CloudflarePublicationMaterializationV1 =
        load_inventory_document_v3(&mut inventory, CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH)?;
    let receipt_sha256 = receipt.canonical_digest()?;
    ensure!(
        receipt_sha256 == expected_receipt_sha256
            && read_inventory_file_v3(
                &mut inventory,
                CLOUDFLARE_MATERIALIZATION_RECEIPT_SIDECAR_PATH,
                64,
            )? == receipt_sha256.to_string().as_bytes(),
        "Cloudflare materialization receipt differs from its approved digest or sidecar"
    );
    let mut admitted_origins = BTreeMap::new();
    for binding in &receipt.origins {
        let document: CloudflareMaterializedOriginInventoryV1 =
            load_inventory_document_v3(&mut inventory, &binding.inventory_path)?;
        ensure!(
            document.origin == binding.origin
                && document.root == binding.root
                && inventory_artifact_v3(&inventory, &binding.inventory_path, "application/json")?
                    == binding.inventory,
            "Cloudflare materialization origin inventory is substituted"
        );
        ensure!(
            admitted_origins.insert(document.origin, document).is_none(),
            "Cloudflare materialization repeats one origin inventory"
        );
    }
    let expected_origins = derive_cloudflare_materialized_origin_authority_v1(
        &mut inventory,
        &receipt,
        &admitted_origins,
    )?;
    let mut expected_pre_receipt = ExpectedPublicationTopologyV3::new_with_root_mode(0o555);
    for document in &expected_origins {
        register_prefixed_materialized_origin_v1(&mut expected_pre_receipt, document)?;
    }
    for binding in &receipt.origins {
        expected_pre_receipt.register_inventory_file(
            &inventory,
            binding.inventory_path.clone(),
            false,
        )?;
    }
    ensure!(
        expected_pre_receipt.materialized_inventory() == receipt.output_inventory,
        "Cloudflare materialization receipt output inventory differs from its typed origin authorities"
    );
    let mut expected_final =
        expected_topology_from_materialized_inventory_v1(&receipt.output_inventory)?;
    expected_final.register_canonical(CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH.into(), &receipt)?;
    expected_final.register_bytes(
        CLOUDFLARE_MATERIALIZATION_RECEIPT_SIDECAR_PATH.into(),
        receipt_sha256.to_string().as_bytes(),
    )?;
    expected_final.validate_inventory(&inventory)?;
    ensure!(
        publication_tree_inventory_v3_from_fd(root_path, root)?.snapshot() == initial_snapshot,
        "Cloudflare materialization changed during receipt validation"
    );
    Ok((receipt, inventory))
}

#[cfg(target_os = "linux")]
pub(super) fn populate_cloudflare_materialization_staging_v1<F>(
    staging: &PinnedPublicationStagingV3,
    publication: &mut ValidatedPublicationV3,
    provenance: &CloudflareMaterializationProvenanceV1,
    origins: &[CloudflareMaterializedOriginInventoryV1],
    before_source_acceptance: F,
) -> Result<(Digest32, ValidatedPublicationV3)>
where
    F: FnOnce(),
{
    let mut expected = ExpectedPublicationTopologyV3::new_with_root_mode(0o555);
    for origin in origins {
        register_cloudflare_origin_v1(&mut expected, origin)?;
    }
    expected.register_directory("inventories")?;
    let mut builder = CloudflareMaterializationOutputBuilderV1::new(staging)?;
    for directory in expected
        .directories
        .keys()
        .filter(|path| path.as_str() != ".")
    {
        builder.create_directory(directory)?;
    }
    for origin in origins {
        for file in &origin.files {
            let source_path = format!("{}/{}", origin.root, file.path);
            let mut output = builder.create_file(&source_path)?;
            let copied = publication.copy_file_to(&source_path, &mut output)?;
            ensure!(
                same_artifact_bytes_v1(&copied, &file.artifact),
                "retained PublicationV3 extraction differs at {source_path}"
            );
        }
    }
    let mut bindings = Vec::with_capacity(origins.len());
    for origin in origins {
        let bytes = canonical_json_bytes(origin)?;
        let artifact = builder.write_bytes(origin.origin.inventory_path(), &bytes)?;
        expected.register_file(origin.origin.inventory_path().into(), &artifact, false)?;
        bindings.push(CloudflareMaterializedOriginAuthorityV1 {
            origin: origin.origin,
            root: origin.root.clone(),
            inventory_path: origin.origin.inventory_path().into(),
            inventory: artifact,
        });
    }
    let receipt = CloudflarePublicationMaterializationV1 {
        schema_version: CLOUDFLARE_MATERIALIZATION_SCHEMA_VERSION,
        publication_schema_version: PUBLICATION_MANIFEST_SCHEMA_VERSION,
        source_commit: provenance.source_commit.clone(),
        source_tree_sha1: provenance.source_tree_sha1.clone(),
        cargo_lock_sha256: provenance.cargo_lock_sha256,
        publication_manifest_sha256: provenance.publication_manifest_sha256,
        publication_lock_sha256: provenance.publication_lock_sha256,
        origins: bindings,
        output_inventory: expected.materialized_inventory(),
    };
    receipt.validate()?;
    let receipt_bytes = canonical_json_bytes(&receipt)?;
    let receipt_sha256 = Digest32::digest_bytes(&receipt_bytes);
    let receipt_artifact =
        builder.write_bytes(CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH, &receipt_bytes)?;
    expected.register_file(
        CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH.into(),
        &receipt_artifact,
        false,
    )?;
    let sidecar_artifact = builder.write_bytes(
        CLOUDFLARE_MATERIALIZATION_RECEIPT_SIDECAR_PATH,
        receipt_sha256.to_string().as_bytes(),
    )?;
    expected.register_file(
        CLOUDFLARE_MATERIALIZATION_RECEIPT_SIDECAR_PATH.into(),
        &sidecar_artifact,
        false,
    )?;
    expected.seal_and_validate(staging.path(), &staging.root)?;
    let (_, inventory) = validate_cloudflare_materialization_inventory_v1(
        staging.path(),
        &staging.root,
        receipt_sha256,
    )?;
    builder.ensure_exact_retained_nodes(&inventory)?;
    before_source_acceptance();
    publication.ensure_live()?;
    let candidate = ValidatedPublicationV3 {
        root_path: staging.path.clone(),
        root: staging.root.try_clone()?,
        root_parent_path: staging.parent_path.clone(),
        root_parent: staging.parent.try_clone()?,
        root_parent_identity: staging.parent_identity.clone(),
        root_name: staging.name.clone(),
        inventory,
        lock_sha256: receipt_sha256,
    };
    candidate.ensure_live()?;
    Ok((receipt_sha256, candidate))
}

#[cfg(target_os = "linux")]
pub(super) fn persist_cloudflare_materialization_v1(
    staging: PinnedPublicationStagingV3,
    publication: &ValidatedPublicationV3,
    candidate: &ValidatedPublicationV3,
    output: &Path,
    materialization_sha256: Digest32,
) -> Result<Digest32> {
    let persistence = publication
        .ensure_live()
        .and_then(|()| candidate.ensure_live())
        .and_then(|()| persist_publication_staging(&staging, candidate, output));
    match persistence {
        Ok(PublicationPersistenceOutcome::Published) => Ok(materialization_sha256),
        Ok(PublicationPersistenceOutcome::PublishedButParentSyncFailed(source)) => {
            Err(CloudflareMaterializationInstalledButParentSyncFailed {
                output: output.to_path_buf(),
                materialization_sha256,
                source,
            }
            .into())
        }
        Err(error) => match error.downcast::<PublicationPersistenceStateUncertain>() {
            Ok(state) => Err(CloudflareMaterializationPersistenceStateUncertain {
                last_staging_path: state.staging_path,
                candidate_device: state.candidate_device,
                candidate_inode: state.candidate_inode,
                last_parent_path: state.parent_path,
                parent_device: state.parent_device,
                parent_inode: state.parent_inode,
                intended_output: state.intended_output,
            }
            .into()),
            Err(persist_error) => match discard_failed_publication_staging(staging) {
                Ok(()) => Err(persist_error),
                Err(cleanup_error) => Err(persist_error.context(format!(
                    "Cloudflare materialization persistence also failed to securely remove staging: {cleanup_error:#}"
                ))),
            },
        },
    }
}

/// Copy the exact public, isolated-signer, and deployment-authority closure
/// from one retained PublicationV3 into an independently reviewable immutable
/// Cloudflare materialization. The Publication path is never reopened after
/// validation; every copied byte comes from the retained validated file FD.
pub fn materialize_cloudflare_publication_v3(
    publication_root: &Path,
    output: &Path,
    repo_root: &Path,
    expected_publication_lock_sha256: Digest32,
) -> Result<Digest32> {
    ensure!(
        publication_root.is_absolute() && output.is_absolute() && repo_root.is_absolute(),
        "Cloudflare materialization paths must be absolute"
    );
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            publication_root,
            output,
            repo_root,
            expected_publication_lock_sha256,
        );
        anyhow::bail!("Cloudflare PublicationV3 materialization requires Linux openat2");
    }
    #[cfg(target_os = "linux")]
    {
        validate_mount_root(publication_root)?;
        let mut publication = validate_publication_v3_authority(publication_root)?;
        let (manifest, build, provenance) = load_cloudflare_materialization_provenance_v1(
            &mut publication,
            expected_publication_lock_sha256,
            repo_root,
        )?;
        let origins = derive_cloudflare_origin_inventories_v1(&publication, &manifest, &build)?;
        let staging = create_pinned_publication_staging_v3(output)?;
        let assembled = populate_cloudflare_materialization_staging_v1(
            &staging,
            &mut publication,
            &provenance,
            &origins,
            || {},
        );
        match assembled {
            Ok((materialization_sha256, candidate)) => persist_cloudflare_materialization_v1(
                staging,
                &publication,
                &candidate,
                output,
                materialization_sha256,
            ),
            Err(assembly_error) => match discard_failed_publication_staging(staging) {
                Ok(()) => Err(assembly_error),
                Err(cleanup_error) => Err(assembly_error.context(format!(
                    "Cloudflare materialization assembly also failed to securely remove staging: {cleanup_error:#}"
                ))),
            },
        }
    }
}

/// Revalidate a materialized Cloudflare V1 output against an independently
/// recorded receipt digest. This library boundary is used by tests and
/// embedding tools; the operator CLI deliberately exposes one atomic
/// materialization command rather than a validate-then-copy workflow.
pub fn validate_cloudflare_publication_materialization_v1(
    root_path: &Path,
    expected_materialization_sha256: Digest32,
) -> Result<CloudflarePublicationMaterializationV1> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root_path, expected_materialization_sha256);
        anyhow::bail!("Cloudflare materialization validation requires Linux openat2");
    }
    #[cfg(target_os = "linux")]
    {
        validate_mount_root(root_path)?;
        let root = open_publication_root_v3(root_path)?;
        let (receipt, _) = validate_cloudflare_materialization_inventory_v1(
            root_path,
            &root,
            expected_materialization_sha256,
        )?;
        Ok(receipt)
    }
}
