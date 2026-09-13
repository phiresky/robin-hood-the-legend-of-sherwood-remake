#![forbid(unsafe_code)]
#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;

    use anyhow::Result;
    use clap::{Parser, Subcommand};
    use robin_manifest_tool::campaign_template_v1::{
        author_campaign_template_matrix_v1, validate_campaign_template_matrix_v1,
    };
    use robin_manifest_tool::plan_v3::author_official_content_v3;
    use robin_manifest_tool::sandbox_v3::probe_sandbox_runtime_v1;
    use robin_manifest_tool::verifier_catalog_v1::{
        author_verifier_job_config_catalog_v1, validate_verifier_job_config_catalog_v1,
    };
    use robin_manifest_tool::{
        DocumentKind, author_build_v2, author_projection_authority_v2,
        author_viewer_build_report_v2, canonicalize_document, hash_file, validate_document,
    };

    #[derive(Debug, Parser)]
    #[command(
        name = "robin-highscores-manifestctl",
        about = "Author and verify Robin Hood ranked-run authority documents"
    )]
    struct Cli {
        #[command(subcommand)]
        command: Command,
    }

    #[derive(Debug, Subcommand)]
    enum Command {
        /// Hash one regular non-symlink artifact.
        Hash { input: PathBuf },
        /// Fail closed unless the pinned bubblewrap/prlimit runtime is installed.
        ProbeSandbox,
        /// Validate a digest-bearing canonical document without rewriting it.
        ValidateDocument {
            #[arg(long, value_enum)]
            kind: DocumentKind,
            input: PathBuf,
        },
        /// Strict-parse, validate and write canonical JSON without overwriting.
        Canonicalize {
            #[arg(long, value_enum)]
            kind: DocumentKind,
            input: PathBuf,
            output: PathBuf,
        },
        /// Recompute BuildManifestV2 with exact wasm-bindgen, Binaryen and WABT authorities.
        AuthorBuildV2 { draft: PathBuf, output: PathBuf },
        /// Bind one private static exporter to an exact public BuildManifestV2.
        AuthorProjectionAuthorityV2 { draft: PathBuf, output: PathBuf },
        /// Derive the exact three-origin viewer build report from a public build.
        AuthorViewerBuildReportV2 { build: PathBuf, output: PathBuf },
        /// Atomically derive all Demo/Full templates from one admitted Plan-V3 authority.
        AuthorCampaignTemplateMatrixV1 { plan: PathBuf, output: PathBuf },
        /// Re-derive and validate an exact campaign-template matrix.
        ValidateCampaignTemplateMatrixV1 { plan: PathBuf, root: PathBuf },
        /// Author the complete private verifier route matrix from final server admission state.
        AuthorVerifierCatalogV1 {
            plan: PathBuf,
            server_config: PathBuf,
            output: PathBuf,
        },
        /// Re-derive and validate the complete private verifier route matrix.
        ValidateVerifierCatalogV1 {
            plan: PathBuf,
            server_config: PathBuf,
            catalog: PathBuf,
        },
        /// Cross-bind all three WASM tool authorities, sandbox four lanes, and publish atomically.
        AuthorOfficialContentV3 { plan: PathBuf, output: PathBuf },
    }

    pub(super) fn main() -> Result<()> {
        match Cli::parse().command {
            Command::Hash { input } => {
                let artifact = hash_file(&input)?;
                println!("sha256={}", artifact.sha256);
                println!("byte_length={}", artifact.byte_length);
            }
            Command::ProbeSandbox => {
                println!("{}", serde_json::to_string(&probe_sandbox_runtime_v1()?)?);
            }
            Command::ValidateDocument { kind, input } => {
                println!("{}", validate_document(kind, &input)?);
            }
            Command::Canonicalize {
                kind,
                input,
                output,
            } => {
                println!("{}", canonicalize_document(kind, &input, &output)?.digest);
            }
            Command::AuthorBuildV2 { draft, output } => {
                println!("{}", author_build_v2(&draft, &output)?.digest);
            }
            Command::AuthorProjectionAuthorityV2 { draft, output } => {
                println!(
                    "{}",
                    author_projection_authority_v2(&draft, &output)?.digest
                );
            }
            Command::AuthorViewerBuildReportV2 { build, output } => {
                println!("{}", author_viewer_build_report_v2(&build, &output)?.digest);
            }
            Command::AuthorCampaignTemplateMatrixV1 { plan, output } => {
                println!("{}", author_campaign_template_matrix_v1(&plan, &output)?);
            }
            Command::ValidateCampaignTemplateMatrixV1 { plan, root } => {
                println!("{}", validate_campaign_template_matrix_v1(&plan, &root)?);
            }
            Command::AuthorVerifierCatalogV1 {
                plan,
                server_config,
                output,
            } => {
                let artifact =
                    author_verifier_job_config_catalog_v1(&plan, &server_config, &output)?;
                println!("sha256={}", artifact.sha256);
                println!("byte_length={}", artifact.byte_length);
            }
            Command::ValidateVerifierCatalogV1 {
                plan,
                server_config,
                catalog,
            } => {
                let artifact =
                    validate_verifier_job_config_catalog_v1(&plan, &server_config, &catalog)?;
                println!("sha256={}", artifact.sha256);
                println!("byte_length={}", artifact.byte_length);
            }
            Command::AuthorOfficialContentV3 { plan, output } => {
                let digests = author_official_content_v3(&plan, &output)?;
                println!(
                    "demo_campaign_content_manifest_sha256={}",
                    digests.demo_campaign_content_manifest_sha256
                );
                println!(
                    "full_campaign_content_manifest_sha256={}",
                    digests.full_campaign_content_manifest_sha256
                );
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    linux::main()
}

#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("manifestctl authoring requires Linux (procfs, bubblewrap sandbox)")
}
