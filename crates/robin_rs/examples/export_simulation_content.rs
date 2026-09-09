//! Private, fresh-process exporter for one official content/source lane.
//!
//! The operator runs four isolated invocations (Demo/Full × loose/shipping).
//! Full licensed bytes remain only in the private source mount and the private
//! verifier catalog; stdout contains one canonical, non-secret typed report.

use std::collections::BTreeSet;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};
use clap::{Parser, ValueEnum};
use robin_rs::main_entry::{CliArgs, SimulationContentExportRequest};
use robin_run_protocol::{
    ArtifactRefV1, BuildManifestV2, CanonicalDocument as _, ContentClosureKindV1,
    ContentManifestV1, Digest32, OFFICIAL_PROJECTION_EXPORT_REPORT_SCHEMA_VERSION_V2,
    OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2, OFFICIAL_PROJECTION_EXPORTER_VERSION_V2,
    OFFICIAL_PROJECTION_RECEIPT_SCHEMA_VERSION_V2,
    OFFICIAL_SIMULATION_CONTENT_PROJECTION_SCHEMA_VERSION_V1, OfficialBuiltInOverlayBindingV2,
    OfficialBuiltInOverlaySourceManifestV2, OfficialContentEditionV1, OfficialContentSubjectV1,
    OfficialProjectionAuthorityManifestV2, OfficialProjectionExecutionPolicyV1,
    OfficialProjectionExportReportV2, OfficialProjectionExporterIdentityV2,
    OfficialProjectionSourceFormatV1, OfficialProjectionSubjectReceiptV1,
    OfficialSimulationProjectionReceiptV2, OfficialSourceFileV1, OfficialSourceTreeManifestV2,
    ResourceLocaleRootV1, RulesConfigIdentityV1, SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
    SimulationContentComponentDocumentV1, SimulationContentComponentV1,
    SimulationSpeechTimingSourceV1, Validate as _, official_content_manifest_name_v1,
    official_content_subjects_v1, simulation_content_component_relative_path_v1,
};
use sha2::Digest as _;

#[cfg(test)]
#[path = "../build_support/projection_static.rs"]
mod projection_static_build_policy;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Edition {
    Demo,
    Full,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SourceLane {
    LooseNative,
    ShippingDatadirV10,
}

#[derive(Debug, Parser)]
#[command(about = "Export exact canonical official simulation-content documents")]
struct Args {
    #[arg(long)]
    source_root: PathBuf,
    #[arg(long)]
    source_manifest: PathBuf,
    #[arg(long)]
    core_overlay_root: PathBuf,
    #[arg(long)]
    core_overlay_manifest: PathBuf,
    #[arg(long)]
    public_build_manifest: PathBuf,
    #[arg(long)]
    projection_authority_manifest: PathBuf,
    #[arg(long)]
    rules_config: PathBuf,
    #[arg(long)]
    execution_policy: PathBuf,
    #[arg(long)]
    catalog_output: PathBuf,
    #[arg(long)]
    receipt_output: PathBuf,
    #[arg(long, value_enum)]
    edition: Edition,
    #[arg(long, value_enum)]
    source: SourceLane,
    #[arg(long)]
    compare_to: Option<PathBuf>,
}

fn edition(value: Edition) -> OfficialContentEditionV1 {
    match value {
        Edition::Demo => OfficialContentEditionV1::Demo,
        Edition::Full => OfficialContentEditionV1::Full,
    }
}

fn source_format(value: SourceLane) -> OfficialProjectionSourceFormatV1 {
    match value {
        SourceLane::LooseNative => OfficialProjectionSourceFormatV1::LooseNativeV1,
        SourceLane::ShippingDatadirV10 => OfficialProjectionSourceFormatV1::ShippingDatadirV10,
    }
}

fn resource_locale(value: Edition) -> ResourceLocaleRootV1 {
    ResourceLocaleRootV1::new(match value {
        Edition::Demo => "1033",
        Edition::Full => "2047",
    })
    .expect("official LCIDs are protocol constants")
}

fn exact_directory(path: &Path, label: &str) -> Result<PathBuf> {
    ensure!(
        path.is_absolute(),
        "{label} must be absolute: {}",
        path.display()
    );
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspect {label} {}", path.display()))?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "{label} must be a non-symlink directory: {}",
        path.display()
    );
    let canonical = std::fs::canonicalize(path)?;
    ensure!(canonical == path, "{label} must be normalized");
    Ok(canonical)
}

fn exact_file(path: &Path, label: &str) -> Result<PathBuf> {
    ensure!(
        path.is_absolute(),
        "{label} must be absolute: {}",
        path.display()
    );
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspect {label} {}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "{label} must be a non-symlink regular file: {}",
        path.display()
    );
    let canonical = std::fs::canonicalize(path)?;
    ensure!(canonical == path, "{label} must be normalized");
    Ok(canonical)
}

fn fresh_output(path: &Path, label: &str) -> Result<()> {
    ensure!(
        path.is_absolute() && !path.exists(),
        "{label} must be an absent absolute path"
    );
    let parent = path.parent().context("output path has no parent")?;
    ensure!(
        std::fs::canonicalize(parent)? == parent,
        "{label} parent is not normalized"
    );
    Ok(())
}

fn read_canonical<T>(path: &Path, label: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned + serde::Serialize + robin_run_protocol::Validate,
{
    let bytes = std::fs::read(path).with_context(|| format!("read {label} {}", path.display()))?;
    let value: T = serde_json::from_slice(&bytes).with_context(|| format!("decode {label}"))?;
    value.validate()?;
    ensure!(
        value.canonical_bytes()? == bytes,
        "{label} is not canonical JSON"
    );
    Ok(value)
}

fn hash_file(path: &Path) -> Result<(Digest32, u64)> {
    let mut file = std::fs::File::open(path)?;
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
            .context("file length overflow")?;
    }
    Ok((Digest32::from_bytes(hasher.finalize().into()), length))
}

fn protocol_files(
    files: &[robin_official_content::SourceClosureFile],
) -> Vec<OfficialSourceFileV1> {
    files
        .iter()
        .map(|file| OfficialSourceFileV1 {
            path: file.path.clone(),
            sha256: Digest32::from_bytes(file.sha256),
            byte_length: file.byte_length,
        })
        .collect()
}

fn exact_regular_inventory(root: &Path) -> Result<BTreeSet<String>> {
    fn visit(root: &Path, directory: &Path, output: &mut BTreeSet<String>) -> Result<()> {
        let mut entries = std::fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "tree contains symlink {}",
                path.display()
            );
            if metadata.is_dir() {
                visit(root, &path, output)?;
            } else if metadata.is_file() {
                output.insert(
                    path.strip_prefix(root)
                        .expect("walk remains below root")
                        .to_str()
                        .context("tree path is not UTF-8")?
                        .replace('\\', "/"),
                );
            } else {
                bail!("tree contains non-regular node {}", path.display());
            }
        }
        Ok(())
    }
    let mut output = BTreeSet::new();
    visit(root, root, &mut output)?;
    Ok(output)
}

fn validate_source_closure(
    root: &Path,
    manifest: &OfficialSourceTreeManifestV2,
    locale: &ResourceLocaleRootV1,
) -> Result<()> {
    let selected = match manifest.source_format {
        OfficialProjectionSourceFormatV1::LooseNativeV1 => {
            let inventory =
                robin_official_content::inventory_loose_source_closure(root, locale.as_str())?;
            ensure!(
                inventory.policy == robin_official_content::LOOSE_NATIVE_SOURCE_CLOSURE_V2
                    && inventory.resource_locale_root == locale.as_str(),
                "loose source selector returned the wrong policy"
            );
            protocol_files(&inventory.files)
        }
        OfficialProjectionSourceFormatV1::ShippingDatadirV10 => {
            let datadir_path =
                robin_engine::sbfile::resolve_case_insensitive(&root.join("Data/datadir.bin"))
                    .context("shipping source has no Data/datadir.bin")?;
            let before =
                robin_assets::shipping_datadir::ShippingDatadir::load_from_file(&datadir_path)?;
            let references =
                robin_official_content::shipping_projection_external_file_paths_v2(&before)?;
            let inventory =
                robin_official_content::inventory_shipping_source_closure(root, &references)?;
            let paths = inventory
                .files
                .iter()
                .map(|file| file.path.clone())
                .collect::<Vec<_>>();
            robin_official_content::validate_shipping_projection_source_relative_paths_v2(
                &before, &paths,
            )?;
            let after =
                robin_assets::shipping_datadir::ShippingDatadir::load_from_file(&datadir_path)?;
            ensure!(
                robin_official_content::shipping_projection_external_file_paths_v2(&after)?
                    == references,
                "shipping reference closure changed while hashing"
            );
            protocol_files(&inventory.files)
        }
    };
    ensure!(
        selected == manifest.files,
        "selected source closure differs from manifest"
    );
    let physical = exact_regular_inventory(root)?;
    let declared = manifest
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    ensure!(
        physical == declared,
        "sanitized source root differs from exact manifest inventory"
    );
    Ok(())
}

fn expected_paths(subjects: &[OfficialContentSubjectV1]) -> Result<BTreeSet<String>> {
    subjects
        .iter()
        .flat_map(|subject| {
            robin_engine::simulation_inputs::SIMULATION_CONTENT_COMPONENT_ORDER_V1
                .into_iter()
                .map(move |kind| {
                    simulation_content_component_relative_path_v1(subject, kind)
                        .map_err(anyhow::Error::from)
                })
        })
        .collect()
}

fn content_receipts(
    root: &Path,
    edition: OfficialContentEditionV1,
    locale: &ResourceLocaleRootV1,
    subjects: &[OfficialContentSubjectV1],
) -> Result<Vec<OfficialProjectionSubjectReceiptV1>> {
    subjects
        .iter()
        .map(|subject| {
            let components = robin_engine::simulation_inputs::SIMULATION_CONTENT_COMPONENT_ORDER_V1
                .into_iter()
                .map(|kind| {
                    let relative = simulation_content_component_relative_path_v1(subject, kind)?;
                    let bytes = std::fs::read(root.join(&relative))?;
                    let document: SimulationContentComponentDocumentV1 =
                        SimulationContentComponentDocumentV1::from_bitcode(&bytes)?;
                    document.validate()?;
                    ensure!(
                        document.kind == kind && document.bitcode_bytes()? == bytes,
                        "component is not exact canonical bitcode v2: {relative}"
                    );
                    Ok(SimulationContentComponentV1 {
                        kind,
                        component_schema_version: document.component_schema_version,
                        artifact: ArtifactRefV1 {
                            sha256: Digest32::digest_bytes(&bytes),
                            byte_length: u64::try_from(bytes.len())?,
                            media_type: SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1.to_owned(),
                        },
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let content_manifest = ContentManifestV1 {
                schema_version: 1,
                name: official_content_manifest_name_v1(edition, subject),
                edition,
                subject: subject.clone(),
                closure: ContentClosureKindV1::StaticPreparedMissionContentProjection,
                projection_schema_version: 2,
                resource_locale_root: locale.clone(),
                speech_timing: SimulationSpeechTimingSourceV1::CoreAudioDurationsV1,
                components,
            };
            content_manifest.validate()?;
            Ok(OfficialProjectionSubjectReceiptV1 { content_manifest })
        })
        .collect()
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let source_root = exact_directory(&args.source_root, "source root")?;
    let core_root = exact_directory(&args.core_overlay_root, "core overlay root")?;
    let source_manifest_path = exact_file(&args.source_manifest, "source manifest")?;
    let core_manifest_path = exact_file(&args.core_overlay_manifest, "core manifest")?;
    let build_path = exact_file(&args.public_build_manifest, "build manifest")?;
    let authority_path = exact_file(&args.projection_authority_manifest, "projection authority")?;
    let rules_path = exact_file(&args.rules_config, "rules config")?;
    let policy_path = exact_file(&args.execution_policy, "execution policy")?;
    fresh_output(&args.catalog_output, "catalog output")?;
    fresh_output(&args.receipt_output, "receipt output")?;
    let compare_to = args
        .compare_to
        .as_deref()
        .map(|path| exact_directory(path, "comparison catalog"))
        .transpose()?;
    ensure!(
        !args.catalog_output.starts_with(&source_root)
            && !args.receipt_output.starts_with(&source_root)
            && !args.catalog_output.starts_with(&core_root)
            && !args.receipt_output.starts_with(&core_root),
        "private inputs and output roots must be disjoint"
    );

    let edition = edition(args.edition);
    let format = source_format(args.source);
    let locale = resource_locale(args.edition);
    let source_manifest: OfficialSourceTreeManifestV2 =
        read_canonical(&source_manifest_path, "source manifest")?;
    ensure!(
        source_manifest.edition == edition && source_manifest.source_format == format,
        "source manifest differs from explicit lane"
    );
    let core_manifest: OfficialBuiltInOverlaySourceManifestV2 =
        read_canonical(&core_manifest_path, "core manifest")?;
    let build: BuildManifestV2 = read_canonical(&build_path, "build manifest")?;
    let authority: OfficialProjectionAuthorityManifestV2 =
        read_canonical(&authority_path, "projection authority")?;
    authority.validate_against(&build)?;
    let rules: RulesConfigIdentityV1 = read_canonical(&rules_path, "rules config")?;
    let policy: OfficialProjectionExecutionPolicyV1 =
        read_canonical(&policy_path, "execution policy")?;
    ensure!(
        policy.rules_config == rules,
        "execution policy substituted its rules config"
    );
    robin_engine::simulation_inputs::validate_official_projection_rules_config_v1(&rules)?;
    validate_source_closure(&source_root, &source_manifest, &locale)?;
    robin_rs::core_overlay::validate_official_projection_source(&core_root, &core_manifest)?;

    let executable = std::fs::canonicalize(std::env::current_exe()?)?;
    let (exporter_sha256, exporter_length) = hash_file(&executable)?;
    let exporter_artifact = ArtifactRefV1 {
        sha256: exporter_sha256,
        byte_length: exporter_length,
        media_type: OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2.to_owned(),
    };
    ensure!(
        authority.projection_exporter.artifact == exporter_artifact,
        "running exporter differs from projection authority"
    );

    std::fs::create_dir(&args.catalog_output)?;
    std::fs::create_dir(&args.receipt_output)?;
    let (_, profiles, context) = robin_rs::main_entry::rust_init_official_projection(
        &source_root,
        &core_root,
        &core_manifest,
        &locale,
        format,
        &rules,
    )?;
    let subjects = official_content_subjects_v1(edition);
    for subject in &subjects {
        let campaign =
            robin_engine::campaign::Campaign::create(&profiles, context.sim_config().difficulty);
        let mut run = CliArgs {
            no_sound: true,
            rollback_check: false,
            headless: true,
            http_server: 0,
            simulation_content_export: Some(SimulationContentExportRequest {
                output_root: args.catalog_output.clone(),
                subject: subject.clone(),
            }),
            global_options: context.clone().into(),
            ..CliArgs::default()
        };
        robin_engine::engine::GlobalOptions::set_global(context.options().clone());
        match subject {
            OfficialContentSubjectV1::FieldMission { mission_id } => {
                run.mission = Some(mission_id.clone());
            }
            OfficialContentSubjectV1::Headquarters { .. } => run.sherwood = true,
        }
        pollster::block_on(robin_rs::main_entry::run_rust_game_headless(
            campaign,
            profiles.clone(),
            context.clone(),
            &run,
        ))
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("export {subject:?}"))?;
    }

    let expected = expected_paths(&subjects)?;
    ensure!(
        exact_regular_inventory(&args.catalog_output)? == expected,
        "catalog inventory differs"
    );
    if let Some(other) = compare_to {
        ensure!(
            exact_regular_inventory(&other)? == expected,
            "comparison inventory differs"
        );
        for relative in &expected {
            ensure!(
                std::fs::read(args.catalog_output.join(relative))?
                    == std::fs::read(other.join(relative))?,
                "component differs between source lanes: {relative}"
            );
        }
    }

    // Re-hash every authority after the expensive preparation before minting
    // the private receipt.
    validate_source_closure(&source_root, &source_manifest, &locale)?;
    robin_rs::core_overlay::validate_official_projection_source(&core_root, &core_manifest)?;
    ensure!(
        hash_file(&executable)? == (exporter_sha256, exporter_length),
        "exporter changed during run"
    );
    ensure!(
        read_canonical::<BuildManifestV2>(&build_path, "build manifest")? == build,
        "build authority changed"
    );
    ensure!(
        read_canonical::<OfficialProjectionAuthorityManifestV2>(
            &authority_path,
            "projection authority"
        )? == authority,
        "projection authority changed"
    );
    ensure!(
        read_canonical::<RulesConfigIdentityV1>(&rules_path, "rules config")? == rules,
        "rules changed"
    );
    ensure!(
        read_canonical::<OfficialProjectionExecutionPolicyV1>(&policy_path, "execution policy")?
            == policy,
        "policy changed"
    );

    let receipt = OfficialSimulationProjectionReceiptV2 {
        schema_version: OFFICIAL_PROJECTION_RECEIPT_SCHEMA_VERSION_V2,
        exporter: OfficialProjectionExporterIdentityV2 {
            exporter_version: OFFICIAL_PROJECTION_EXPORTER_VERSION_V2,
            simulation_content_projection_schema_version:
                OFFICIAL_SIMULATION_CONTENT_PROJECTION_SCHEMA_VERSION_V1,
            platform: authority.projection_exporter.platform,
            source_format: format,
            projection_authority_manifest_sha256: authority.canonical_digest()?,
            exporter_artifact,
        },
        edition,
        source_tree_manifest_sha256: source_manifest.canonical_digest()?,
        source_file_count: u32::try_from(source_manifest.files.len())?,
        built_in_overlay: OfficialBuiltInOverlayBindingV2 {
            kind: core_manifest.kind,
            source_manifest_sha256: core_manifest.canonical_digest()?,
            source_file_count: u32::try_from(core_manifest.files.len())?,
        },
        rules_config_sha256: rules.canonical_digest()?,
        execution_policy_sha256: policy.canonical_digest()?,
        execution_policy: policy,
        subjects: content_receipts(&args.catalog_output, edition, &locale, &subjects)?,
    };
    receipt.validate_against(&build, &authority, &rules, &source_manifest, &core_manifest)?;
    let source_bytes = source_manifest.canonical_bytes()?;
    let receipt_bytes = receipt.canonical_bytes()?;
    write_new(
        &args.receipt_output.join("source-tree-manifest.json"),
        &source_bytes,
    )?;
    write_new(
        &args.receipt_output.join("projection-receipt.json"),
        &receipt_bytes,
    )?;

    let report = OfficialProjectionExportReportV2 {
        schema_version: OFFICIAL_PROJECTION_EXPORT_REPORT_SCHEMA_VERSION_V2,
        edition,
        source_format: format,
        catalog_root: args
            .catalog_output
            .to_str()
            .context("catalog path is not UTF-8")?
            .into(),
        receipt_root: args
            .receipt_output
            .to_str()
            .context("receipt path is not UTF-8")?
            .into(),
        source_tree_manifest_sha256: receipt.source_tree_manifest_sha256,
        projection_receipt_sha256: receipt.canonical_digest()?,
        public_build_manifest_sha256: build.canonical_digest()?,
        projection_authority_manifest_sha256: receipt.exporter.projection_authority_manifest_sha256,
        rules_config_sha256: receipt.rules_config_sha256,
        execution_policy_sha256: receipt.execution_policy_sha256,
        core_overlay_manifest_sha256: receipt.built_in_overlay.source_manifest_sha256,
        exporter_artifact: receipt.exporter.exporter_artifact.clone(),
        source_file_count: receipt.source_file_count,
        core_overlay_file_count: receipt.built_in_overlay.source_file_count,
        resource_locale_root: locale,
        subjects,
        component_file_count: u32::try_from(expected.len())?,
    };
    report.validate_against(
        &receipt,
        &build,
        &authority,
        &rules,
        &source_manifest,
        &core_manifest,
    )?;
    std::io::stdout()
        .lock()
        .write_all(&report.canonical_bytes()?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Args, Edition, SourceLane};
    use clap::Parser as _;

    #[test]
    fn private_exporter_feature_is_storage_and_presentation_minimal() {
        assert!(cfg!(feature = "projection-export"));
        assert!(!cfg!(feature = "native-fs"));
        assert!(!cfg!(feature = "audio"));
        assert!(!cfg!(feature = "video"));
        let parsed = Args::try_parse_from([
            "export_simulation_content",
            "--source-root",
            "/source",
            "--source-manifest",
            "/authority/source.json",
            "--core-overlay-root",
            "/core",
            "--core-overlay-manifest",
            "/authority/core.json",
            "--public-build-manifest",
            "/authority/build.json",
            "--projection-authority-manifest",
            "/authority/projection.json",
            "--rules-config",
            "/authority/rules.json",
            "--execution-policy",
            "/authority/policy.json",
            "--catalog-output",
            "/output/catalog",
            "--receipt-output",
            "/output/receipt",
            "--edition",
            "demo",
            "--source",
            "loose-native",
        ])
        .unwrap();
        assert!(matches!(parsed.edition, Edition::Demo));
        assert!(matches!(parsed.source, SourceLane::LooseNative));
    }
}
