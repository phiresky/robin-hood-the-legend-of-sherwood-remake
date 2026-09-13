//! Worker startup checks for the private verifier authority installed on the VPS.
//!
//! Publication artifacts contain no licensed raw game data. The authority
//! release carries projection bundles, campaign templates and source-tree
//! manifests; Demo and Full raw roots are installed separately by the operator.
//! The verifier validates its read-only content mounts for every job, so
//! startup only checks that the worker and server configuration agree and never
//! hashes the raw trees.

pub mod paths;

use crate::ServerConfig;
use crate::config::ViewerContentRequirementConfig;
use robin_run_protocol::{
    CanonicalCampaignStatePinV1, CanonicalDocument as _, Digest32, OfficialContentEditionV1,
    OfficialSourceTreeManifestV2, RunScopeKindV1, Validate as _, VerifierJobConfigCatalogV1,
    VerifierJobRouteV1, canonical_json_bytes,
};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Component, Path};

pub use paths::{DEMO_RAW_CONTENT_ROOT, FULL_RAW_CONTENT_ROOT};

const MAX_SOURCE_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

/// Every catalog entry must embed exactly the manifests the server admits, and
/// the catalog must cover exactly the route matrix of the admission profiles.
pub fn validate_catalog_covers_server(
    catalog: &VerifierJobConfigCatalogV1,
    server: &ServerConfig,
) -> anyhow::Result<()> {
    for entry in &catalog.entries {
        let route = &entry.route;
        let build = server
            .manifests
            .builds
            .get(&route.build_manifest_sha256)
            .ok_or_else(|| anyhow::anyhow!("job catalog references an unavailable build"))?;
        let content = server
            .manifests
            .content_manifests
            .get(&route.content_manifest_sha256)
            .ok_or_else(|| anyhow::anyhow!("job catalog references unavailable content"))?;
        let rules = server
            .manifests
            .rules_configs
            .get(&route.rules_config_sha256)
            .ok_or_else(|| anyhow::anyhow!("job catalog references unavailable rules"))?;
        let ruleset = server
            .manifests
            .rulesets
            .get(&route.ruleset_manifest_sha256)
            .ok_or_else(|| anyhow::anyhow!("job catalog references unavailable ruleset"))?;
        anyhow::ensure!(
            build.public_document() == &entry.build_manifest
                && content == &entry.content_manifest
                && rules == &entry.rules_config
                && ruleset.manifest == entry.ruleset_manifest,
            "job catalog embeds a substituted manifest"
        );
        match (
            route.campaign_content_manifest_sha256,
            &entry.campaign_content_manifest,
        ) {
            (None, None) => {}
            (Some(digest), Some(document))
                if server.manifests.campaign_content_manifests.get(&digest) == Some(document) => {}
            _ => anyhow::bail!("job catalog campaign authority is unavailable or substituted"),
        }
        match (
            route.competition_manifest_sha256,
            &entry.competition_manifest,
        ) {
            (None, None) => {}
            (Some(digest), Some(document))
                if server.manifests.competitions.get(&digest) == Some(document) => {}
            _ => anyhow::bail!("job catalog competition authority is unavailable or substituted"),
        }
    }

    let mut expected = BTreeMap::<Vec<u8>, CanonicalCampaignStatePinV1>::new();
    for profile in &server.admission_profiles {
        let content_digest = parse_digest(&profile.content_manifest_id, "profile content")?;
        let content = server
            .manifests
            .content_manifests
            .get(&content_digest)
            .ok_or_else(|| anyhow::anyhow!("profile content is unavailable"))?;
        anyhow::ensure!(
            (!profile.viewer_available && profile.viewer_content_requirement.is_none())
                || matches!(
                    (content.edition, profile.viewer_content_requirement),
                    (
                        OfficialContentEditionV1::Demo,
                        Some(ViewerContentRequirementConfig::BundledDemo),
                    ) | (
                        OfficialContentEditionV1::Full,
                        Some(ViewerContentRequirementConfig::UserLocalRetail),
                    )
                ),
            "profile has a substituted viewer entitlement"
        );
        let build = parse_digest(&profile.build_manifest_id, "profile build")?;
        let rules = parse_digest(&profile.config_id, "profile rules")?;
        let ruleset = parse_digest(&profile.ruleset_id, "profile ruleset")?;
        let competitions = std::iter::once(None)
            .chain(
                server
                    .competitions
                    .iter()
                    .filter(|competition| competition.admission_profile_id == profile.id)
                    .map(|competition| {
                        parse_digest(&competition.manifest_sha256, "competition").map(Some)
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?,
            )
            .collect::<Vec<_>>();
        let mut scopes = profile
            .allowed_scopes
            .iter()
            .map(|scope| match scope.as_str() {
                "individual_level" => Ok(RunScopeKindV1::IndividualLevel),
                "campaign_genesis" | "campaign_continuation" => Ok(RunScopeKindV1::Campaign),
                _ => anyhow::bail!("invalid profile scope"),
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        scopes.sort_by_key(|scope| match scope {
            RunScopeKindV1::IndividualLevel => 0,
            RunScopeKindV1::Campaign => 1,
        });
        scopes.dedup();
        for scope_kind in scopes {
            let campaign = match scope_kind {
                RunScopeKindV1::IndividualLevel => None,
                RunScopeKindV1::Campaign => Some(parse_digest(
                    profile
                        .campaign_content_manifest_id
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("campaign profile omits catalog"))?,
                    "campaign catalog",
                )?),
            };
            for competition in &competitions {
                let route = VerifierJobRouteV1 {
                    schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
                    scope_kind,
                    content_edition: content.edition,
                    content_subject: content.subject.clone(),
                    build_manifest_sha256: build,
                    content_manifest_sha256: content_digest,
                    campaign_content_manifest_sha256: campaign,
                    rules_config_sha256: rules,
                    ruleset_manifest_sha256: ruleset,
                    competition_manifest_sha256: *competition,
                };
                route.validate()?;
                let bytes = canonical_json_bytes(&route)?;
                if let Some(previous) =
                    expected.insert(bytes, profile.canonical_campaign_state.clone())
                {
                    anyhow::ensure!(
                        previous == profile.canonical_campaign_state,
                        "profiles substitute campaign state for one worker route"
                    );
                }
            }
        }
    }
    let actual = catalog
        .entries
        .iter()
        .map(|entry| {
            Ok((
                canonical_json_bytes(&entry.route)?,
                entry.canonical_campaign_state.clone(),
            ))
        })
        .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    anyhow::ensure!(
        actual == expected,
        "worker catalog is not the exact admitted route matrix"
    );
    Ok(())
}

fn parse_digest(value: &str, label: &str) -> anyhow::Result<Digest32> {
    anyhow::ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && value.bytes().any(|byte| byte != b'0'),
        "{label} is not canonical nonzero lowercase SHA-256"
    );
    Ok(value.parse()?)
}

/// Check that the worker's catalog, profiles and source-tree manifests describe
/// one consistent installed authority. Content itself is validated by the
/// verifier against its read-only mounts on every job.
pub fn validate_worker_authority_layout(
    catalog_path: &Path,
    catalog: &VerifierJobConfigCatalogV1,
    server: &ServerConfig,
    demo_source_manifest: &Path,
    full_source_manifest: &Path,
) -> anyhow::Result<()> {
    ensure_normalized_absolute(catalog_path)?;
    anyhow::ensure!(
        !catalog.entries.is_empty(),
        "verifier catalog contains no entries"
    );
    for template in &catalog.entries {
        ensure_normalized_absolute(&template.content_catalog_root)?;
        anyhow::ensure!(
            std::fs::metadata(&template.content_catalog_root)?.is_dir(),
            "projection catalog root is not an installed directory"
        );
        anyhow::ensure!(
            template.raw_content_root == raw_content_root(template.raw_content_edition),
            "verifier catalog selects a raw content root outside its edition root"
        );
    }
    for profile in &server.admission_profiles {
        // ServerConfig loading already hashed this file against its pin.
        let path = profile
            .canonical_campaign_state_path
            .as_deref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "installed profile {} has no canonical campaign-state path",
                    profile.id
                )
            })?;
        ensure_normalized_absolute(path)?;
    }
    load_source_manifest(demo_source_manifest, OfficialContentEditionV1::Demo)?;
    load_source_manifest(full_source_manifest, OfficialContentEditionV1::Full)?;
    Ok(())
}

fn raw_content_root(edition: OfficialContentEditionV1) -> &'static Path {
    match edition {
        OfficialContentEditionV1::Demo => Path::new(DEMO_RAW_CONTENT_ROOT),
        OfficialContentEditionV1::Full => Path::new(FULL_RAW_CONTENT_ROOT),
    }
}

/// Parse a digest-named canonical source-tree manifest for one edition.
fn load_source_manifest(
    path: &Path,
    edition: OfficialContentEditionV1,
) -> anyhow::Result<OfficialSourceTreeManifestV2> {
    ensure_normalized_absolute(path)?;
    let file = std::fs::File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.is_file(),
        "raw source manifest is not a regular file"
    );
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_SOURCE_MANIFEST_BYTES,
        "raw source manifest exceeds its byte limit"
    );
    let expected_digest = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".json"))
        .ok_or_else(|| anyhow::anyhow!("raw source manifest must be named <sha256>.json"))?;
    anyhow::ensure!(
        hex::encode(Sha256::digest(&bytes)) == expected_digest,
        "raw source manifest bytes differ from their content-addressed filename"
    );
    let manifest: OfficialSourceTreeManifestV2 = serde_json::from_slice(&bytes)?;
    manifest.validate()?;
    anyhow::ensure!(
        manifest.canonical_bytes()? == bytes,
        "raw source manifest is not canonical JSON"
    );
    anyhow::ensure!(
        manifest.edition == edition,
        "raw source manifest selects the wrong official edition"
    );
    Ok(manifest)
}

fn ensure_normalized_absolute(path: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
            && path.file_name().is_some(),
        "authority path is not normalized and absolute: {}",
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_run_protocol::{
        OfficialProjectionSourceFormatV1, OfficialSourceClosureKindV2, OfficialSourceFileV1,
    };

    #[test]
    fn raw_roots_are_exact_edition_specific_operator_paths() {
        assert_eq!(
            raw_content_root(OfficialContentEditionV1::Demo),
            Path::new("/home/robinhood/.local/share/robin-highscores/raw-content/demo")
        );
        assert_ne!(
            raw_content_root(OfficialContentEditionV1::Demo),
            raw_content_root(OfficialContentEditionV1::Full)
        );
    }

    #[test]
    fn authority_paths_must_be_normalized_and_absolute() {
        ensure_normalized_absolute(Path::new("/releases/abc/private/catalog")).unwrap();
        // `Path::components` already folds interior `.` segments away.
        for bad in ["relative/catalog", "/releases/../catalog", "/"] {
            assert!(ensure_normalized_absolute(Path::new(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn source_manifest_must_be_canonical_digest_named_and_match_its_edition() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = raw_manifest();
        let bytes = manifest.canonical_bytes().unwrap();
        let digest = hex::encode(Sha256::digest(&bytes));
        let path = directory.path().join(format!("{digest}.json"));
        std::fs::write(&path, &bytes).unwrap();

        assert_eq!(
            load_source_manifest(&path, OfficialContentEditionV1::Demo).unwrap(),
            manifest
        );
        assert!(load_source_manifest(&path, OfficialContentEditionV1::Full).is_err());

        let misnamed = directory.path().join(format!("{}.json", "ab".repeat(32)));
        std::fs::write(&misnamed, &bytes).unwrap();
        assert!(load_source_manifest(&misnamed, OfficialContentEditionV1::Demo).is_err());

        let pretty = serde_json::to_vec_pretty(&manifest).unwrap();
        let pretty_path = directory
            .path()
            .join(format!("{}.json", hex::encode(Sha256::digest(&pretty))));
        std::fs::write(&pretty_path, &pretty).unwrap();
        assert!(load_source_manifest(&pretty_path, OfficialContentEditionV1::Demo).is_err());
    }

    fn raw_manifest() -> OfficialSourceTreeManifestV2 {
        OfficialSourceTreeManifestV2 {
            schema_version: 2,
            edition: OfficialContentEditionV1::Demo,
            source_format: OfficialProjectionSourceFormatV1::LooseNativeV1,
            closure_kind: OfficialSourceClosureKindV2::LooseNativeSimulationConsumedV1,
            files: vec![OfficialSourceFileV1 {
                path: "Data/mission.bin".into(),
                sha256: Digest32::digest_bytes(b"exact raw input"),
                byte_length: 15,
            }],
        }
    }
}
