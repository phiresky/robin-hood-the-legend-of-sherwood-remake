//! Versioned verifier, viewer and exporter build identities and artifact-closure validation.

use super::{ArtifactRefV1, NamedArtifactV1, ViewerArtifactRoleV1};
use crate::CanonicalDocument as _;
use crate::{Digest32, Validate, ValidationError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Immutable identity of one native verifier / browser-viewer build.
///
/// Schema numbers are facts, not compatibility policy.  A service can retain
/// historical manifests while allowlisting only selected combinations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildManifestV1 {
    pub schema_version: u32,
    pub source_commit: String,
    pub cargo_lock_sha256: Digest32,
    pub target_triple: String,
    pub cargo_profile: String,
    pub cargo_features: Vec<String>,
    pub replay_schema_version: u32,
    pub save_schema_version: u32,
    pub network_protocol_version: u32,
    pub verifier: ArtifactRefV1,
    pub viewer_artifacts: Vec<NamedArtifactV1>,
}

/// Private executable media type for the operator-owned official projection
/// exporter. Unlike viewer artifacts, this binary is never published below a
/// public build origin.
pub const OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2: &str =
    "application/vnd.robinhood.official-simulation-projection-exporter-v2";
pub const RANKED_REPLAY_VERIFIER_MEDIA_TYPE_V2: &str =
    "application/vnd.robinhood.ranked-replay-verifier-v2";
pub const BINARYEN_WASM_OPT_VERSION_V1: &str = "version_132";
pub const WABT_WASM_STRIP_VERSION_V1: &str = "1.0.41";
pub const WASM_BINDGEN_CLI_VERSION_V1: &str = "0.2.127";
/// Canonical SHA-256 of
/// `.github/tool-authorities/wasm-bindgen-cli-v0.2.127.json`.
pub const WASM_BINDGEN_CLI_AUTHORITY_SHA256_V1: Digest32 = Digest32::from_bytes([
    0x68, 0xee, 0x22, 0xd8, 0xda, 0x66, 0x2e, 0x20, 0xa7, 0xaa, 0x63, 0xd4, 0x33, 0x54, 0xc5, 0x91,
    0x94, 0x53, 0x03, 0x78, 0x89, 0x8e, 0x20, 0x52, 0x07, 0x9b, 0x90, 0x84, 0x58, 0x0b, 0x89, 0xba,
]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficialProjectionExporterPlatformV2 {
    X86_64UnknownLinuxMusl,
}

impl OfficialProjectionExporterPlatformV2 {
    pub const fn target_triple(self) -> &'static str {
        match self {
            Self::X86_64UnknownLinuxMusl => "x86_64-unknown-linux-musl",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeBuildPlatformV2 {
    X86_64UnknownLinuxMusl,
}

impl NativeBuildPlatformV2 {
    pub const fn target_triple(self) -> &'static str {
        match self {
            Self::X86_64UnknownLinuxMusl => "x86_64-unknown-linux-musl",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeLinkageV2 {
    FullyStaticNoInterpreterOrNeededLibraries,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierBuildIdentityV2 {
    pub platform: NativeBuildPlatformV2,
    pub target_triple: String,
    pub cargo_profile: String,
    pub cargo_features: Vec<String>,
    pub cargo_package: String,
    pub cargo_binary: String,
    pub linkage: NativeLinkageV2,
    pub artifact: ArtifactRefV1,
}

impl Validate for VerifierBuildIdentityV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.target_triple != self.platform.target_triple()
            || self.target_triple != "x86_64-unknown-linux-musl"
            || self.cargo_profile != "release"
            || !self.cargo_features.is_empty()
            || self.cargo_package != "robin_replay_verifier"
            || self.cargo_binary != "robin-replay-verifier"
            || self.linkage != NativeLinkageV2::FullyStaticNoInterpreterOrNeededLibraries
        {
            return Err(ValidationError::ClaimMismatch {
                field: "build.verifier.recipe",
            });
        }
        self.artifact.validate()?;
        if self.artifact.media_type != RANKED_REPLAY_VERIFIER_MEDIA_TYPE_V2 {
            return Err(ValidationError::ClaimMismatch {
                field: "build.verifier.artifact.media_type",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustToolchainAuthorityV1 {
    pub schema_version: u32,
    pub channel: String,
    pub components: Vec<String>,
    pub targets: Vec<String>,
}

impl Validate for RustToolchainAuthorityV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "RustToolchainAuthorityV1",
            crate::SCHEMA_VERSION_V1,
            self.schema_version,
        )?;
        if self.channel != "nightly-2026-08-25"
            || self.components
                != [
                    "rust-src".to_owned(),
                    "rustc-codegen-cranelift-preview".to_owned(),
                ]
            || self.targets != ["wasm32-unknown-unknown".to_owned()]
        {
            return Err(ValidationError::ClaimMismatch {
                field: "build.viewer.rust_toolchain_authority",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildToolAuthorityV1 {
    pub version: String,
    /// Digest of the operator-validated canonical distribution/tool authority
    /// document, never an ambiguous digest of whichever binary happened to be
    /// found on PATH.
    pub authority_sha256: Digest32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildToolRoleV1 {
    WasmBindgenCli,
    BinaryenWasmOpt,
    WabtWasmStrip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildToolAuthorityDocumentV1 {
    pub schema_version: u32,
    pub role: BuildToolRoleV1,
    pub version: String,
    /// Exact official upstream release URL. Redirect resolution is allowed
    /// only after this string has matched the closed role policy.
    pub distribution_url: String,
    pub distribution: ArtifactRefV1,
    pub executable_relative_path: String,
}

impl Validate for BuildToolAuthorityDocumentV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "BuildToolAuthorityDocumentV1",
            crate::SCHEMA_VERSION_V1,
            self.schema_version,
        )?;
        self.distribution.validate()?;
        let (version, url, sha256, byte_length, executable_relative_path) = match self.role {
            BuildToolRoleV1::WasmBindgenCli => (
                WASM_BINDGEN_CLI_VERSION_V1,
                "https://static.crates.io/crates/wasm-bindgen-cli/wasm-bindgen-cli-0.2.127.crate",
                Digest32::from_bytes([
                    0x61, 0x23, 0xf5, 0x25, 0xba, 0x36, 0xdf, 0x42, 0xe5, 0x7b, 0x67, 0x02, 0x76,
                    0x37, 0xa7, 0x85, 0x91, 0xe7, 0x12, 0xa9, 0xf6, 0xa0, 0x25, 0xff, 0xd3, 0xd7,
                    0x42, 0x98, 0xfa, 0x1c, 0x3f, 0x4c,
                ]),
                52_654,
                "bin/wasm-bindgen",
            ),
            BuildToolRoleV1::BinaryenWasmOpt => (
                BINARYEN_WASM_OPT_VERSION_V1,
                "https://github.com/WebAssembly/binaryen/releases/download/version_132/binaryen-version_132-x86_64-linux.tar.gz",
                Digest32::from_bytes([
                    0x19, 0x5d, 0xdc, 0x94, 0xf9, 0xbc, 0x89, 0xf4, 0x5a, 0xbd, 0xab, 0xb0, 0xb9,
                    0xee, 0xa8, 0x60, 0x23, 0xd7, 0x27, 0xba, 0x90, 0xea, 0xc8, 0xb3, 0x5b, 0x80,
                    0xf2, 0x54, 0x4f, 0xc3, 0x05, 0x72,
                ]),
                117_057_408,
                "binaryen-version_132/bin/wasm-opt",
            ),
            BuildToolRoleV1::WabtWasmStrip => (
                WABT_WASM_STRIP_VERSION_V1,
                "https://github.com/WebAssembly/wabt/releases/download/1.0.41/wabt-1.0.41-linux-x64.tar.gz",
                Digest32::from_bytes([
                    0x83, 0xf8, 0x12, 0x2e, 0x92, 0x47, 0x45, 0xfc, 0xd7, 0x06, 0x36, 0xe3, 0x59,
                    0x4b, 0xc0, 0x1c, 0x4c, 0x47, 0xf2, 0xd4, 0xc8, 0xf3, 0xc6, 0x3b, 0x5d, 0x70,
                    0xd3, 0xf8, 0x3a, 0x48, 0x26, 0x77,
                ]),
                5_076_269,
                "wabt-1.0.41/bin/wasm-strip",
            ),
        };
        if self.version != version
            || self.distribution_url != url
            || self.distribution.sha256 != sha256
            || self.distribution.byte_length != byte_length
            || self.distribution.media_type != "application/gzip"
            || self.executable_relative_path != executable_relative_path
        {
            return Err(ValidationError::ClaimMismatch {
                field: "build.tool.authority_document",
            });
        }
        Ok(())
    }
}

impl Validate for BuildToolAuthorityV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::text("build.tool.version", &self.version, 128)?;
        if self.authority_sha256.is_zero() {
            return Err(ValidationError::Zero {
                field: "build.tool.authority_sha256",
            });
        }
        Ok(())
    }
}

fn validate_exact_release_semver(
    field: &'static str,
    tool: &BuildToolAuthorityV1,
) -> Result<(), ValidationError> {
    let parsed = semver::Version::parse(&tool.version)
        .map_err(|_| ValidationError::ClaimMismatch { field })?;
    if parsed.to_string() != tool.version || !parsed.pre.is_empty() || !parsed.build.is_empty() {
        return Err(ValidationError::ClaimMismatch { field });
    }
    Ok(())
}

impl BuildToolAuthorityV1 {
    pub fn validate_against(
        &self,
        role: BuildToolRoleV1,
        authority: &BuildToolAuthorityDocumentV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        authority.validate()?;
        if authority.role != role
            || self.version != authority.version
            || self.authority_sha256
                != authority
                    .canonical_digest()
                    .map_err(|_| ValidationError::ClaimMismatch {
                        field: "build.tool.authority_canonicalization",
                    })?
        {
            return Err(ValidationError::ClaimMismatch {
                field: "build.tool.authority_binding",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserViewerEngineBuildRecipeV2 {
    /// `cargo build -Zbuild-std=std,panic_abort --target
    /// wasm32-unknown-unknown --profile wasm-release --no-default-features
    /// --features audio -p robin_rs --bin robin`, then wasm-bindgen `--target
    /// web --out-name robin`, wasm-opt `-Oz --strip-debug --strip-dwarf`, and
    /// wasm-strip.
    WasmBindgenWebBinaryenOzStripDebugDwarfWabtStripV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewerEngineBuildIdentityV2 {
    pub target_triple: String,
    pub cargo_profile: String,
    pub cargo_features: Vec<String>,
    pub cargo_package: String,
    pub cargo_binary: String,
    pub recipe: BrowserViewerEngineBuildRecipeV2,
    pub rust_toolchain: RustToolchainAuthorityV1,
    pub rust_toolchain_sha256: Digest32,
    pub wasm_bindgen_cli: BuildToolAuthorityV1,
    pub binaryen_wasm_opt: BuildToolAuthorityV1,
    pub wabt_wasm_strip: BuildToolAuthorityV1,
    /// Only engine artifacts consumed by VerifiedViewer. Pages and signer
    /// files deliberately live in separate closures below.
    pub artifacts: Vec<NamedArtifactV1>,
}

impl Validate for BrowserViewerEngineBuildIdentityV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.target_triple != "wasm32-unknown-unknown"
            || self.cargo_profile != "wasm-release"
            || self.cargo_features != ["audio".to_owned()]
            || self.cargo_package != "robin_rs"
            || self.cargo_binary != "robin"
            || self.recipe
                != BrowserViewerEngineBuildRecipeV2::WasmBindgenWebBinaryenOzStripDebugDwarfWabtStripV1
        {
            return Err(ValidationError::ClaimMismatch {
                field: "build.viewer.recipe",
            });
        }
        self.rust_toolchain.validate()?;
        if self
            .rust_toolchain
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "build.viewer.rust_toolchain_canonicalization",
            })?
            != self.rust_toolchain_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "build.viewer.rust_toolchain_sha256",
            });
        }
        for tool in [
            &self.wasm_bindgen_cli,
            &self.binaryen_wasm_opt,
            &self.wabt_wasm_strip,
        ] {
            tool.validate()?;
        }
        validate_exact_release_semver(
            "build.viewer.engine.wasm_bindgen_version",
            &self.wasm_bindgen_cli,
        )?;
        validate_exact_release_semver("build.viewer.engine.wabt_version", &self.wabt_wasm_strip)?;
        if self.wasm_bindgen_cli.version != WASM_BINDGEN_CLI_VERSION_V1
            || self.wasm_bindgen_cli.authority_sha256 != WASM_BINDGEN_CLI_AUTHORITY_SHA256_V1
            || self.binaryen_wasm_opt.version != BINARYEN_WASM_OPT_VERSION_V1
            || self.wabt_wasm_strip.version != WABT_WASM_STRIP_VERSION_V1
        {
            return Err(ValidationError::ClaimMismatch {
                field: "build.viewer.engine.wasm_bindgen_version",
            });
        }
        validate_viewer_artifacts_v1(&self.artifacts)?;
        let entry_javascript = self
            .artifacts
            .iter()
            .find(|artifact| artifact.role == ViewerArtifactRoleV1::EntryJavaScript);
        let webassembly = self
            .artifacts
            .iter()
            .find(|artifact| artifact.role == ViewerArtifactRoleV1::WebAssembly);
        if !matches!(entry_javascript, Some(artifact) if artifact.path == "viewer/robin.js")
            || !matches!(webassembly, Some(artifact) if artifact.path == "viewer/robin_bg.wasm")
        {
            return Err(ValidationError::ClaimMismatch {
                field: "build.viewer.published_bundle",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserPagesArtifactV2 {
    /// Relative to one explicitly separated immutable deployment origin.
    pub path: String,
    pub artifact: ArtifactRefV1,
}

impl Validate for BrowserPagesArtifactV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::artifact_relative_url_path("build.pages_artifact.path", &self.path)?;
        self.artifact.validate()
    }
}

fn validate_pages_artifact_closure_v2(
    field: &'static str,
    artifacts: &[BrowserPagesArtifactV2],
) -> Result<(), ValidationError> {
    if artifacts.is_empty() || artifacts.len() > 512 {
        return Err(ValidationError::CountOutOfRange { field });
    }
    for artifact in artifacts {
        artifact.validate()?;
    }
    if !artifacts.windows(2).all(|pair| pair[0].path < pair[1].path) {
        return Err(ValidationError::NotCanonicalOrder { field });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserPagesShellBuildRecipeV2 {
    PnpmFrozenLockfileViteStaticShellV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserPagesShellBuildIdentityV2 {
    pub recipe: BrowserPagesShellBuildRecipeV2,
    pub node: BuildToolAuthorityV1,
    pub pnpm: BuildToolAuthorityV1,
    pub package_json_sha256: Digest32,
    pub pnpm_lock_sha256: Digest32,
    /// Exact, sorted, no-extra closure for the public viewer/leaderboards
    /// origin. Identity-signer files are forbidden from this origin.
    pub public_origin_artifacts: Vec<BrowserPagesArtifactV2>,
}

impl Validate for BrowserPagesShellBuildIdentityV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.node.validate()?;
        self.pnpm.validate()?;
        validate_exact_release_semver("build.viewer.pages_shell.node_version", &self.node)?;
        validate_exact_release_semver("build.viewer.pages_shell.pnpm_version", &self.pnpm)?;
        if self.recipe != BrowserPagesShellBuildRecipeV2::PnpmFrozenLockfileViteStaticShellV1
            || self.node.version != "24.19.0"
            || self.pnpm.version != "9.15.0"
            || self.package_json_sha256.is_zero()
            || self.pnpm_lock_sha256.is_zero()
        {
            return Err(ValidationError::ClaimMismatch {
                field: "build.viewer.pages_shell.recipe",
            });
        }
        validate_pages_artifact_closure_v2(
            "build.viewer.pages_shell.public_origin_artifacts",
            &self.public_origin_artifacts,
        )?;
        if !self.public_origin_artifacts.iter().any(|artifact| {
            artifact.path == "index.html" && artifact.artifact.media_type == "text/html"
        }) {
            return Err(ValidationError::ClaimMismatch {
                field: "build.viewer.pages_shell.entry",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserIdentitySignerBuildRecipeV2 {
    WasmBindgenWebSeparateOriginBridgeV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserIdentitySignerDeploymentPolicyV2 {
    SeparateAllowlistedOriginCspFrameAncestorsAndBridgeShaV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserIdentitySignerBuildIdentityV2 {
    pub target_triple: String,
    pub cargo_profile: String,
    pub cargo_features: Vec<String>,
    pub cargo_package: String,
    pub cargo_binary: String,
    pub recipe: BrowserIdentitySignerBuildRecipeV2,
    pub deployment_policy: BrowserIdentitySignerDeploymentPolicyV2,
    pub rust_toolchain: RustToolchainAuthorityV1,
    pub rust_toolchain_sha256: Digest32,
    pub wasm_bindgen_cli: BuildToolAuthorityV1,
    /// Exact sorted closure for the separately allowlisted identity-signer
    /// origin. The deployment CSP/origin gate must not co-host these paths
    /// with `pages_shell.public_origin_artifacts`.
    pub identity_signer_origin_artifacts: Vec<BrowserPagesArtifactV2>,
}

impl Validate for BrowserIdentitySignerBuildIdentityV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.target_triple != "wasm32-unknown-unknown"
            || self.cargo_profile != "wasm-release"
            || self.cargo_features != ["identity-signer-bridge".to_owned()]
            // robin_rs is retained only for historical signed build identities.
            || !matches!(self.cargo_package.as_str(), "robin_rs" | "robin_identity_signer")
            || self.cargo_binary != "leaderboard_identity_bridge"
            || self.recipe
                != BrowserIdentitySignerBuildRecipeV2::WasmBindgenWebSeparateOriginBridgeV1
            || self.deployment_policy
                != BrowserIdentitySignerDeploymentPolicyV2::SeparateAllowlistedOriginCspFrameAncestorsAndBridgeShaV1
            || self.wasm_bindgen_cli.version != WASM_BINDGEN_CLI_VERSION_V1
            || self.wasm_bindgen_cli.authority_sha256
                != WASM_BINDGEN_CLI_AUTHORITY_SHA256_V1
        {
            return Err(ValidationError::ClaimMismatch {
                field: "build.viewer.identity_signer.recipe",
            });
        }
        self.rust_toolchain.validate()?;
        self.wasm_bindgen_cli.validate()?;
        validate_exact_release_semver(
            "build.viewer.identity_signer.wasm_bindgen_version",
            &self.wasm_bindgen_cli,
        )?;
        if self
            .rust_toolchain
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "build.viewer.identity_signer.rust_toolchain_canonicalization",
            })?
            != self.rust_toolchain_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "build.viewer.identity_signer.rust_toolchain_sha256",
            });
        }
        validate_pages_artifact_closure_v2(
            "build.viewer.identity_signer.origin_artifacts",
            &self.identity_signer_origin_artifacts,
        )?;
        for (path, media_type) in [
            ("identity-signer/index.html", "text/html"),
            (
                "identity-signer/bridge/leaderboard_identity_bridge.js",
                "text/javascript",
            ),
            (
                "identity-signer/bridge/leaderboard_identity_bridge_bg.wasm",
                "application/wasm",
            ),
        ] {
            if !self
                .identity_signer_origin_artifacts
                .iter()
                .any(|artifact| artifact.path == path && artifact.artifact.media_type == media_type)
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "build.viewer.identity_signer.required_artifacts",
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewerBuildIdentityV2 {
    pub engine: BrowserViewerEngineBuildIdentityV2,
    pub pages_shell: BrowserPagesShellBuildIdentityV2,
    pub identity_signer: BrowserIdentitySignerBuildIdentityV2,
}

impl Validate for BrowserViewerBuildIdentityV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.engine.validate()?;
        self.pages_shell.validate()?;
        self.identity_signer.validate()?;
        if self.identity_signer.rust_toolchain != self.engine.rust_toolchain
            || self.identity_signer.rust_toolchain_sha256 != self.engine.rust_toolchain_sha256
            || self.identity_signer.wasm_bindgen_cli != self.engine.wasm_bindgen_cli
        {
            return Err(ValidationError::ClaimMismatch {
                field: "build.viewer.shared_rust_wasm_bindgen_authority",
            });
        }
        let public_artifacts = self
            .pages_shell
            .public_origin_artifacts
            .iter()
            .map(|artifact| artifact.artifact.sha256)
            .chain(
                self.engine
                    .artifacts
                    .iter()
                    .map(|artifact| artifact.artifact.sha256),
            )
            .collect::<BTreeSet<_>>();
        if self
            .identity_signer
            .identity_signer_origin_artifacts
            .iter()
            .any(|artifact| public_artifacts.contains(&artifact.artifact.sha256))
        {
            return Err(ValidationError::ClaimMismatch {
                field: "build.viewer.cross_origin_artifact_substitution",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialProjectionExporterBuildIdentityV2 {
    pub platform: OfficialProjectionExporterPlatformV2,
    pub target_triple: String,
    pub cargo_profile: String,
    pub cargo_features: Vec<String>,
    pub cargo_package: String,
    pub cargo_example: String,
    pub linkage: NativeLinkageV2,
    pub exporter_version: u32,
    pub simulation_content_projection_schema_version: u32,
    pub artifact: ArtifactRefV1,
}

impl Validate for OfficialProjectionExporterBuildIdentityV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.target_triple != self.platform.target_triple()
            || self.cargo_profile != "release"
            || self.cargo_features != ["projection-export".to_owned()]
            || self.cargo_package != "robin_rs"
            || self.cargo_example != "export_simulation_content"
            || self.linkage != NativeLinkageV2::FullyStaticNoInterpreterOrNeededLibraries
            || self.exporter_version != 2
            || self.simulation_content_projection_schema_version == 0
        {
            return Err(ValidationError::ClaimMismatch {
                field: "projection_authority.exporter.recipe",
            });
        }
        self.artifact.validate()?;
        if self.artifact.media_type != OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2 {
            return Err(ValidationError::ClaimMismatch {
                field: "projection_authority.exporter.artifact.media_type",
            });
        }
        Ok(())
    }
}

/// Public verifier/browser build identity. Private operator executables are
/// deliberately absent: this document and its digest are safe to publish in
/// run proofs and public build catalogs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildManifestV2 {
    pub schema_version: u32,
    pub source_commit: String,
    pub cargo_lock_sha256: Digest32,
    pub replay_schema_version: u32,
    pub save_schema_version: u32,
    pub network_protocol_version: u32,
    pub verifier: VerifierBuildIdentityV2,
    pub viewer: BrowserViewerBuildIdentityV2,
}

impl Validate for BuildManifestV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact("BuildManifestV2", 2, self.schema_version)?;
        validate_shared_build_facts_v2(
            &self.source_commit,
            self.cargo_lock_sha256,
            self.replay_schema_version,
            self.save_schema_version,
            self.network_protocol_version,
        )?;
        self.verifier.validate()?;
        self.viewer.validate()?;
        Ok(())
    }
}

impl BuildManifestV2 {
    /// Exact public V1 view required by the existing backend build identity.
    /// This projection cannot authorize official projection receipts.
    pub fn backend_visible_v1(&self) -> Result<BuildManifestV1, ValidationError> {
        self.validate()?;
        let projected = BuildManifestV1 {
            schema_version: crate::SCHEMA_VERSION_V1,
            source_commit: self.source_commit.clone(),
            cargo_lock_sha256: self.cargo_lock_sha256,
            // Historical V1's flat recipe describes the browser viewer even
            // though its verifier is a separately referenced native artifact.
            target_triple: self.viewer.engine.target_triple.clone(),
            cargo_profile: self.viewer.engine.cargo_profile.clone(),
            cargo_features: self.viewer.engine.cargo_features.clone(),
            replay_schema_version: self.replay_schema_version,
            save_schema_version: self.save_schema_version,
            network_protocol_version: self.network_protocol_version,
            verifier: self.verifier.artifact.clone(),
            viewer_artifacts: self.viewer.engine.artifacts.clone(),
        };
        projected.validate()?;
        Ok(projected)
    }

    /// Cross-binds downloaded upstream distributions to the tool authority
    /// digests carried by this public build.
    pub fn validate_wasm_tool_authorities(
        &self,
        wasm_bindgen: &BuildToolAuthorityDocumentV1,
        binaryen: &BuildToolAuthorityDocumentV1,
        wabt: &BuildToolAuthorityDocumentV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        self.viewer
            .engine
            .wasm_bindgen_cli
            .validate_against(BuildToolRoleV1::WasmBindgenCli, wasm_bindgen)?;
        self.viewer
            .engine
            .binaryen_wasm_opt
            .validate_against(BuildToolRoleV1::BinaryenWasmOpt, binaryen)?;
        self.viewer
            .engine
            .wabt_wasm_strip
            .validate_against(BuildToolRoleV1::WabtWasmStrip, wabt)
    }
}

pub const OFFICIAL_VIEWER_BUILD_REPORT_SCHEMA_VERSION_V2: u32 = 2;

/// Producer-side physical output inventory. It is intentionally not a build
/// authority by itself: the operator re-hashes it and later emits the
/// `OfficialViewerBuildReportV2` cross-bound to a complete public manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfficialViewerOriginArtifactInventoryV2 {
    Engine {
        schema_version: u32,
        source_commit: String,
        cargo_lock_sha256: Digest32,
        artifacts: Vec<NamedArtifactV1>,
    },
    PagesShell {
        schema_version: u32,
        source_commit: String,
        cargo_lock_sha256: Digest32,
        artifacts: Vec<BrowserPagesArtifactV2>,
    },
    IdentitySigner {
        schema_version: u32,
        source_commit: String,
        cargo_lock_sha256: Digest32,
        artifacts: Vec<BrowserPagesArtifactV2>,
    },
}

impl Validate for OfficialViewerOriginArtifactInventoryV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        let (schema_version, source_commit, cargo_lock_sha256) = match self {
            Self::Engine {
                schema_version,
                source_commit,
                cargo_lock_sha256,
                artifacts,
            } => {
                validate_viewer_artifacts_v1(artifacts)?;
                (*schema_version, source_commit, *cargo_lock_sha256)
            }
            Self::PagesShell {
                schema_version,
                source_commit,
                cargo_lock_sha256,
                artifacts,
            } => {
                validate_pages_artifact_closure_v2("viewer_inventory.pages_shell", artifacts)?;
                (*schema_version, source_commit, *cargo_lock_sha256)
            }
            Self::IdentitySigner {
                schema_version,
                source_commit,
                cargo_lock_sha256,
                artifacts,
            } => {
                validate_pages_artifact_closure_v2("viewer_inventory.identity_signer", artifacts)?;
                (*schema_version, source_commit, *cargo_lock_sha256)
            }
        };
        crate::validation::schema_exact(
            "OfficialViewerOriginArtifactInventoryV2",
            OFFICIAL_VIEWER_BUILD_REPORT_SCHEMA_VERSION_V2,
            schema_version,
        )?;
        crate::validation::text("viewer_inventory.source_commit", source_commit, 128)?;
        if cargo_lock_sha256.is_zero() {
            return Err(ValidationError::Zero {
                field: "viewer_inventory.cargo_lock_sha256",
            });
        }
        Ok(())
    }
}

/// Canonical per-origin output inventory emitted once a public build body has
/// been authored. The operator independently hashes every local file and
/// requires exact set equality; paths are never inferred by this document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialViewerBuildReportV2 {
    pub schema_version: u32,
    pub public_build_manifest_sha256: Digest32,
    pub source_commit: String,
    pub cargo_lock_sha256: Digest32,
    pub verifier_artifact: ArtifactRefV1,
    pub rust_toolchain_sha256: Digest32,
    pub wasm_bindgen_cli: BuildToolAuthorityV1,
    pub binaryen_wasm_opt: BuildToolAuthorityV1,
    pub wabt_wasm_strip: BuildToolAuthorityV1,
    pub node: BuildToolAuthorityV1,
    pub pnpm: BuildToolAuthorityV1,
    pub package_json_sha256: Digest32,
    pub pnpm_lock_sha256: Digest32,
    pub engine_artifacts: Vec<NamedArtifactV1>,
    pub pages_shell_artifacts: Vec<BrowserPagesArtifactV2>,
    pub identity_signer_artifacts: Vec<BrowserPagesArtifactV2>,
}

impl Validate for OfficialViewerBuildReportV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "OfficialViewerBuildReportV2",
            OFFICIAL_VIEWER_BUILD_REPORT_SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        crate::validation::text("viewer_report.source_commit", &self.source_commit, 128)?;
        if self.public_build_manifest_sha256.is_zero()
            || self.cargo_lock_sha256.is_zero()
            || self.rust_toolchain_sha256.is_zero()
            || self.package_json_sha256.is_zero()
            || self.pnpm_lock_sha256.is_zero()
        {
            return Err(ValidationError::Zero {
                field: "viewer_report.authority_digest",
            });
        }
        self.verifier_artifact.validate()?;
        for tool in [
            &self.wasm_bindgen_cli,
            &self.binaryen_wasm_opt,
            &self.wabt_wasm_strip,
            &self.node,
            &self.pnpm,
        ] {
            tool.validate()?;
        }
        validate_viewer_artifacts_v1(&self.engine_artifacts)?;
        validate_pages_artifact_closure_v2(
            "viewer_report.pages_shell_artifacts",
            &self.pages_shell_artifacts,
        )?;
        validate_pages_artifact_closure_v2(
            "viewer_report.identity_signer_artifacts",
            &self.identity_signer_artifacts,
        )
    }
}

impl OfficialViewerBuildReportV2 {
    pub fn from_public_build(build: &BuildManifestV2) -> Result<Self, ValidationError> {
        build.validate()?;
        Ok(Self {
            schema_version: OFFICIAL_VIEWER_BUILD_REPORT_SCHEMA_VERSION_V2,
            public_build_manifest_sha256: build.canonical_digest().map_err(|_| {
                ValidationError::ClaimMismatch {
                    field: "viewer_report.public_build_canonicalization",
                }
            })?,
            source_commit: build.source_commit.clone(),
            cargo_lock_sha256: build.cargo_lock_sha256,
            verifier_artifact: build.verifier.artifact.clone(),
            rust_toolchain_sha256: build.viewer.engine.rust_toolchain_sha256,
            wasm_bindgen_cli: build.viewer.engine.wasm_bindgen_cli.clone(),
            binaryen_wasm_opt: build.viewer.engine.binaryen_wasm_opt.clone(),
            wabt_wasm_strip: build.viewer.engine.wabt_wasm_strip.clone(),
            node: build.viewer.pages_shell.node.clone(),
            pnpm: build.viewer.pages_shell.pnpm.clone(),
            package_json_sha256: build.viewer.pages_shell.package_json_sha256,
            pnpm_lock_sha256: build.viewer.pages_shell.pnpm_lock_sha256,
            engine_artifacts: build.viewer.engine.artifacts.clone(),
            pages_shell_artifacts: build.viewer.pages_shell.public_origin_artifacts.clone(),
            identity_signer_artifacts: build
                .viewer
                .identity_signer
                .identity_signer_origin_artifacts
                .clone(),
        })
    }

    pub fn validate_against(&self, build: &BuildManifestV2) -> Result<(), ValidationError> {
        self.validate()?;
        if self != &Self::from_public_build(build)? {
            return Err(ValidationError::ClaimMismatch {
                field: "viewer_report.public_build_binding",
            });
        }
        Ok(())
    }
}

/// Private operator authority. This document and its digest must never be
/// placed in public build catalogs, content catalogs, or replay/run proofs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialProjectionAuthorityManifestV2 {
    pub schema_version: u32,
    pub public_build_manifest_sha256: Digest32,
    pub source_commit: String,
    pub cargo_lock_sha256: Digest32,
    pub replay_schema_version: u32,
    pub save_schema_version: u32,
    pub network_protocol_version: u32,
    pub projection_exporter: OfficialProjectionExporterBuildIdentityV2,
}

impl Validate for OfficialProjectionAuthorityManifestV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "OfficialProjectionAuthorityManifestV2",
            2,
            self.schema_version,
        )?;
        if self.public_build_manifest_sha256.is_zero() {
            return Err(ValidationError::Zero {
                field: "projection_authority.public_build_manifest_sha256",
            });
        }
        validate_shared_build_facts_v2(
            &self.source_commit,
            self.cargo_lock_sha256,
            self.replay_schema_version,
            self.save_schema_version,
            self.network_protocol_version,
        )?;
        self.projection_exporter.validate()
    }
}

impl OfficialProjectionAuthorityManifestV2 {
    pub fn validate_against(&self, public: &BuildManifestV2) -> Result<(), ValidationError> {
        self.validate()?;
        public.validate()?;
        if self.public_build_manifest_sha256
            != public
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "projection_authority.public_build_canonicalization",
                })?
            || self.source_commit != public.source_commit
            || self.cargo_lock_sha256 != public.cargo_lock_sha256
            || self.replay_schema_version != public.replay_schema_version
            || self.save_schema_version != public.save_schema_version
            || self.network_protocol_version != public.network_protocol_version
        {
            return Err(ValidationError::ClaimMismatch {
                field: "projection_authority.public_build",
            });
        }
        Ok(())
    }

    pub fn official_projection_exporter(
        &self,
    ) -> Result<&OfficialProjectionExporterBuildIdentityV2, ValidationError> {
        self.validate()?;
        Ok(&self.projection_exporter)
    }
}

fn validate_shared_build_facts_v2(
    source_commit: &str,
    cargo_lock_sha256: Digest32,
    replay_schema_version: u32,
    save_schema_version: u32,
    network_protocol_version: u32,
) -> Result<(), ValidationError> {
    if cargo_lock_sha256.is_zero() {
        return Err(ValidationError::Zero {
            field: "build.cargo_lock_sha256",
        });
    }
    if !matches!(source_commit.len(), 40 | 64)
        || !source_commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ValidationError::InvalidRelativePath {
            field: "build.source_commit",
            reason: "expected a full lowercase 40- or 64-character Git object id",
        });
    }
    for (field, value) in [
        ("build.replay_schema_version", replay_schema_version),
        ("build.save_schema_version", save_schema_version),
        ("build.network_protocol_version", network_protocol_version),
    ] {
        if value == 0 {
            return Err(ValidationError::Zero { field });
        }
    }
    Ok(())
}

/// Explicit decoder result for services which retain historical public
/// builds. Neither variant can authorize official projection authoring; that
/// additionally requires a private OfficialProjectionAuthorityManifestV2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VersionedBuildManifest {
    V1(BuildManifestV1),
    V2(BuildManifestV2),
}

impl Validate for VersionedBuildManifest {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::V1(manifest) => manifest.validate(),
            Self::V2(manifest) => manifest.validate(),
        }
    }
}

impl VersionedBuildManifest {
    pub fn require_public_v2(&self) -> Result<&BuildManifestV2, ValidationError> {
        match self {
            Self::V2(manifest) => {
                manifest.validate()?;
                Ok(manifest)
            }
            Self::V1(_) => Err(ValidationError::ClaimMismatch {
                field: "build.public_v2_required",
            }),
        }
    }

    pub fn backend_visible_v1(&self) -> Result<BuildManifestV1, ValidationError> {
        match self {
            Self::V1(manifest) => {
                manifest.validate()?;
                Ok(manifest.clone())
            }
            Self::V2(manifest) => manifest.backend_visible_v1(),
        }
    }
}

impl Validate for BuildManifestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("BuildManifestV1", self.schema_version)?;
        if self.cargo_lock_sha256.is_zero() {
            return Err(ValidationError::Zero {
                field: "build.cargo_lock_sha256",
            });
        }
        if !matches!(self.source_commit.len(), 40 | 64)
            || !self
                .source_commit
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ValidationError::InvalidRelativePath {
                field: "build.source_commit",
                reason: "expected a full lowercase 40- or 64-character Git object id",
            });
        }
        crate::validation::text("build.target_triple", &self.target_triple, 128)?;
        crate::validation::text("build.cargo_profile", &self.cargo_profile, 64)?;
        for feature in &self.cargo_features {
            crate::validation::text("build.cargo_features", feature, 128)?;
        }
        if !crate::validation::strictly_sorted(&self.cargo_features) {
            return Err(ValidationError::NotCanonicalOrder {
                field: "build.cargo_features",
            });
        }
        for (field, value) in [
            ("build.replay_schema_version", self.replay_schema_version),
            ("build.save_schema_version", self.save_schema_version),
            (
                "build.network_protocol_version",
                self.network_protocol_version,
            ),
        ] {
            if value == 0 {
                return Err(ValidationError::Zero { field });
            }
        }
        self.verifier.validate()?;
        validate_viewer_artifacts_v1(&self.viewer_artifacts)
    }
}

pub(super) fn validate_viewer_artifacts_v1(
    viewer_artifacts: &[NamedArtifactV1],
) -> Result<(), ValidationError> {
    if viewer_artifacts.len() < 2 || viewer_artifacts.len() > 64 {
        return Err(ValidationError::CountOutOfRange {
            field: "build.viewer_artifacts",
        });
    }
    for artifact in viewer_artifacts {
        artifact.validate()?;
    }
    if !viewer_artifacts
        .windows(2)
        .all(|pair| pair[0].path < pair[1].path)
    {
        return Err(ValidationError::NotCanonicalOrder {
            field: "build.viewer_artifacts",
        });
    }
    let entry_count = viewer_artifacts
        .iter()
        .filter(|artifact| matches!(artifact.role, ViewerArtifactRoleV1::EntryJavaScript))
        .count();
    let wasm_count = viewer_artifacts
        .iter()
        .filter(|artifact| matches!(artifact.role, ViewerArtifactRoleV1::WebAssembly))
        .count();
    let unique_roles = viewer_artifacts
        .iter()
        .map(|artifact| match &artifact.role {
            ViewerArtifactRoleV1::EntryJavaScript => "entry_javascript".to_owned(),
            ViewerArtifactRoleV1::WebAssembly => "webassembly".to_owned(),
            ViewerArtifactRoleV1::JavaScriptModule { name } => {
                format!("javascript_module:{name}")
            }
            ViewerArtifactRoleV1::Auxiliary { name } => format!("auxiliary:{name}"),
        })
        .collect::<BTreeSet<_>>();
    if entry_count != 1 || wasm_count != 1 || unique_roles.len() != viewer_artifacts.len() {
        return Err(ValidationError::Duplicate {
            field: "build.viewer_artifacts.role",
            value: "viewer artifact role".into(),
        });
    }
    Ok(())
}

/// Immutable artifact-origin path below the deployment's allowlisted base.
pub fn build_artifact_object_path_v1(
    build_manifest_sha256: Digest32,
    artifact: &NamedArtifactV1,
) -> Result<String, ValidationError> {
    if build_manifest_sha256.is_zero() {
        return Err(ValidationError::Zero {
            field: "build_artifact_path.build_manifest_sha256",
        });
    }
    artifact.validate()?;
    Ok(format!("builds/{build_manifest_sha256}/{}", artifact.path))
}
