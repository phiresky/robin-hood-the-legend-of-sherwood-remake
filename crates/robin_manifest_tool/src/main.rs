use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail, ensure};
use clap::{Parser, Subcommand};
use robin_manifest_tool::campaign_template_v1::{
    author_campaign_template_matrix_v1, validate_campaign_template_matrix_v1,
};
use robin_manifest_tool::plan_v3::author_official_content_v3;
use robin_manifest_tool::publication_v3::{
    assemble_publication_v3, materialize_cloudflare_publication_v3, validate_publication_v3,
};
use robin_manifest_tool::sandbox_v3::probe_sandbox_runtime_v1;
use robin_manifest_tool::verifier_catalog_v1::{
    author_verifier_job_config_catalog_v1, validate_verifier_job_config_catalog_v1,
};
use robin_manifest_tool::vps_release_v2::{
    assemble_vps_release_v2, consume_vps_sources_v2, exec_vps_deploy_activation_v2,
    exec_vps_rollback_activation_v2, initialize_vps_runtime_fence_v1,
    project_vps_publication_lock_v2, promote_inherited_vps_release_v2, promote_vps_release_v2,
    validate_vps_release_v2,
};
use robin_manifest_tool::{
    DocumentKind, author_build_v2, author_projection_authority_v2, author_viewer_build_report_v2,
    canonicalize_document, hash_file, validate_document,
};

#[derive(Debug, Parser)]
#[command(
    name = "robin-highscores-manifestctl",
    about = "Author and verify immutable Robin Hood ranked-run releases"
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
    /// Refuse historical BuildManifestV1 authoring.
    AuthorBuild { draft: PathBuf, output: PathBuf },
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
    /// Refuse historical whole-tree V1 receipt authorization.
    AuthorOfficialContent { plan: PathBuf, output: PathBuf },
    /// Cross-bind all three WASM tool authorities, sandbox four lanes, and publish atomically.
    AuthorOfficialContentV3 { plan: PathBuf, output: PathBuf },
    /// Assemble backend/private/Cloudflare-static publication from admitted authority.
    AssemblePublicationV3 { plan: PathBuf, output: PathBuf },
    /// Validate one immutable V3 publication, including its complete file and directory lock.
    ValidatePublicationV3 { release: PathBuf },
    /// Atomically extract the exact Cloudflare authority from one retained PublicationV3.
    MaterializeCloudflarePublicationV3 {
        publication: PathBuf,
        output: PathBuf,
        repo_root: PathBuf,
        expected_publication_lock_sha256: String,
    },
    /// Assemble an immutable VPS install bundle from one admitted publication.
    AssembleVpsReleaseV2 { plan: PathBuf, output: PathBuf },
    /// Validate one immutable VPS install bundle without its source plan.
    ValidateVpsReleaseV2 { release: PathBuf },
    /// Validate and atomically promote one pinned VPS release partial without replacement.
    PromoteVpsReleaseV2 {
        partial: PathBuf,
        output: PathBuf,
        expected_sha256sums_sha256: String,
    },
    /// Consume one exact plan-bound uploader source closure under the outer activation lock.
    ConsumeVpsSourcesV2 {
        plan_fd: PathBuf,
        expected_plan_sha256: String,
        expected_release_manifest_sha256: String,
        #[arg(long)]
        candidate_root_fd: i32,
        #[arg(long)]
        activation_lock_fd: i32,
    },
    /// Promote the exact retained release-partial inode without copying or replacement.
    PromoteInheritedVpsReleaseV2 {
        expected_release_manifest_sha256: String,
        #[arg(long)]
        candidate_root_fd: i32,
        #[arg(long)]
        activation_lock_fd: i32,
    },
    /// Create or finish the exact persistent runtime-fence topology under the activation lock.
    InitializeVpsRuntimeFenceV1 {
        source_commit: String,
        #[arg(long)]
        activation_lock_fd: i32,
    },
    /// Project one typed publication-lock digest from a pinned V2 manifest.
    ProjectVpsPublicationLockV2 {
        #[arg(long)]
        release_manifest_fd: i32,
        #[arg(long)]
        expected_vps_release_manifest_sha256: String,
    },
    /// Hold the canonical activation lock across one descriptor-pinned deploy or rollback.
    ExecVpsActivationV2 {
        #[command(subcommand)]
        operation: VpsActivationV2,
    },
    /// Refuse the historical V1 release assembler; use plan-v3 content authority.
    AssembleRelease { plan: PathBuf, output: PathBuf },
    /// Refuse historical V1 release validation; use plan-v3 content authority.
    ValidateRelease { plan: PathBuf, release: PathBuf },
}

#[derive(Debug, Subcommand)]
enum VpsActivationV2 {
    /// Execute one deploy transaction with its retained, out-of-band-pinned source plan.
    Deploy {
        script_fd: PathBuf,
        bootstrap_manifest_fd: PathBuf,
        validator_fd: PathBuf,
        manifest_tool_fd: PathBuf,
        plan_fd: PathBuf,
        expected_plan_sha256: String,
        expected_vps_release_manifest_sha256: String,
        #[arg(last = true, required = true)]
        business_arguments: Vec<OsString>,
    },
    /// Execute one rollback transaction; rollback never admits or consumes uploader sources.
    Rollback {
        script_fd: PathBuf,
        bootstrap_manifest_fd: PathBuf,
        validator_fd: PathBuf,
        manifest_tool_fd: PathBuf,
        #[arg(last = true, required = true)]
        business_arguments: Vec<OsString>,
    },
}

fn main() -> Result<()> {
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
        Command::AuthorBuild { draft, output } => {
            let _ = (draft, output);
            bail!(
                "historical BuildManifestV1 authoring cannot authorize a release; use author-build-v2"
            );
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
            let artifact = author_verifier_job_config_catalog_v1(&plan, &server_config, &output)?;
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
        Command::AuthorOfficialContent { plan, output } => {
            let _ = (plan, output);
            bail!(
                "historical V1 whole-tree projection receipts never authorize official content; use author-official-content-v3"
            );
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
        Command::AssemblePublicationV3 { plan, output } => {
            println!("{}", assemble_publication_v3(&plan, &output)?);
        }
        Command::ValidatePublicationV3 { release } => {
            println!("{}", validate_publication_v3(&release)?);
        }
        Command::MaterializeCloudflarePublicationV3 {
            publication,
            output,
            repo_root,
            expected_publication_lock_sha256,
        } => {
            ensure!(
                expected_publication_lock_sha256.len() == 64
                    && expected_publication_lock_sha256
                        .bytes()
                        .all(|byte| { byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) }),
                "expected PublicationV3 lock is not a lowercase SHA-256 digest"
            );
            let expected_publication_lock_sha256 = expected_publication_lock_sha256
                .parse()
                .context("expected PublicationV3 lock is not a lowercase SHA-256 digest")?;
            println!(
                "{}",
                materialize_cloudflare_publication_v3(
                    &publication,
                    &output,
                    &repo_root,
                    expected_publication_lock_sha256,
                )?
            );
        }
        Command::AssembleVpsReleaseV2 { plan, output } => {
            println!("{}", assemble_vps_release_v2(&plan, &output)?);
        }
        Command::ValidateVpsReleaseV2 { release } => {
            println!("{}", validate_vps_release_v2(&release)?);
        }
        Command::PromoteVpsReleaseV2 {
            partial,
            output,
            expected_sha256sums_sha256,
        } => {
            println!(
                "{}",
                promote_vps_release_v2(&partial, &output, &expected_sha256sums_sha256)?
            );
        }
        Command::ConsumeVpsSourcesV2 {
            plan_fd,
            expected_plan_sha256,
            expected_release_manifest_sha256,
            candidate_root_fd,
            activation_lock_fd,
        } => {
            println!(
                "{}",
                consume_vps_sources_v2(
                    &plan_fd,
                    &expected_plan_sha256,
                    &expected_release_manifest_sha256,
                    candidate_root_fd,
                    activation_lock_fd,
                )?
            );
        }
        Command::PromoteInheritedVpsReleaseV2 {
            expected_release_manifest_sha256,
            candidate_root_fd,
            activation_lock_fd,
        } => {
            println!(
                "{}",
                promote_inherited_vps_release_v2(
                    &expected_release_manifest_sha256,
                    candidate_root_fd,
                    activation_lock_fd,
                )?
            );
        }
        Command::InitializeVpsRuntimeFenceV1 {
            source_commit,
            activation_lock_fd,
        } => {
            initialize_vps_runtime_fence_v1(&source_commit, activation_lock_fd)?;
        }
        Command::ProjectVpsPublicationLockV2 {
            release_manifest_fd,
            expected_vps_release_manifest_sha256,
        } => {
            print!(
                "{}",
                project_vps_publication_lock_v2(
                    release_manifest_fd,
                    &expected_vps_release_manifest_sha256,
                )?
            );
        }
        Command::ExecVpsActivationV2 { operation } => match operation {
            VpsActivationV2::Deploy {
                script_fd,
                bootstrap_manifest_fd,
                validator_fd,
                manifest_tool_fd,
                plan_fd,
                expected_plan_sha256,
                expected_vps_release_manifest_sha256,
                business_arguments,
            } => {
                exec_vps_deploy_activation_v2(
                    &script_fd,
                    &bootstrap_manifest_fd,
                    &validator_fd,
                    &manifest_tool_fd,
                    &plan_fd,
                    &expected_plan_sha256,
                    &expected_vps_release_manifest_sha256,
                    &business_arguments,
                )?;
            }
            VpsActivationV2::Rollback {
                script_fd,
                bootstrap_manifest_fd,
                validator_fd,
                manifest_tool_fd,
                business_arguments,
            } => {
                exec_vps_rollback_activation_v2(
                    &script_fd,
                    &bootstrap_manifest_fd,
                    &validator_fd,
                    &manifest_tool_fd,
                    &business_arguments,
                )?;
            }
        },
        Command::AssembleRelease { plan, output } => {
            let _ = (plan, output);
            bail!(
                "historical V1 release assembly is disabled because it admits V1 receipts; author plan-v3 content first"
            );
        }
        Command::ValidateRelease { plan, release } => {
            let _ = (plan, release);
            bail!("historical V1 release validation is disabled because it reauthors V1 receipts");
        }
    }
    Ok(())
}
