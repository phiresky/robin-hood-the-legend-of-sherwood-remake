#![forbid(unsafe_code)]

use clap::Parser;
use robin_highscores::verifier::{
    DirectVerifierLauncherConfig, ProcessError, VerifierProcessConfig, build_verification_request,
};
use robin_highscores::{
    CampaignStore, Database, ReplayStore, ServerConfig,
    deployment::validate_worker_authority_layout,
    garbage_collect_campaigns, reconcile_campaign_inventory,
    storage_admission::{ensure_worker_final_campaign_capacity, ensure_worker_lease_capacity},
};
use robin_run_protocol::{
    CanonicalDocument as _, Digest32, MAX_VERIFIER_JOB_CONFIG_BYTES_V1, Validate as _,
    VerificationInfrastructureFailureCodeV1, VerificationLimitsV1, VerificationRejectionCodeV1,
    VerificationStatusV1, VerifierAdmissionFailureCodeV1, VerifierJobConfigCatalogV1,
    VerifierJobConfigV1, VerifierJobRouteV1, VerifierWorkerOutputV1,
};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(about = "Isolated Robin Hood replay-verification queue worker")]
struct Arguments {
    #[arg(
        long,
        env = "ROBIN_HIGHSCORES_WORKER_CONFIG",
        default_value = "highscores-worker.toml"
    )]
    config: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerConfig {
    server_config: PathBuf,
    worker_id: String,
    campaign_state_directory: PathBuf,
    verifier_launcher: DirectVerifierLauncherConfig,
    verifier_job_config_catalog: PathBuf,
    verifier_job_config_catalog_sha256: String,
    demo_raw_content_manifest: PathBuf,
    full_raw_content_manifest: PathBuf,
    poll_interval_ms: u64,
    lease_seconds: u64,
    retry_seconds: u64,
    max_verifier_attempts: u32,
    limits: VerificationLimitsV1,
}

async fn run_with_write_lease_heartbeat<T, F>(
    database: &Database,
    token: &str,
    ttl: Duration,
    operation: F,
) -> anyhow::Result<T>
where
    F: Future<Output = anyhow::Result<T>>,
{
    run_with_write_lease_heartbeat_using(database, token, ttl, operation, || async {
        Ok(database.refresh_maintenance_write_lease(token, ttl).await?)
    })
    .await
}

async fn run_with_write_lease_heartbeat_using<T, F, R, RF>(
    database: &Database,
    token: &str,
    ttl: Duration,
    operation: F,
    mut refresh: R,
) -> anyhow::Result<T>
where
    F: Future<Output = anyhow::Result<T>>,
    R: FnMut() -> RF,
    RF: Future<Output = anyhow::Result<bool>>,
{
    use futures_util::FutureExt as _;

    let refresh_every = ttl
        .checked_div(3)
        .filter(|interval| !interval.is_zero())
        .ok_or_else(|| anyhow::anyhow!("maintenance-write lease TTL is too short"))?;
    // Catch unwind and join queued/running physical work before releasing the
    // lease. The outer detached fence owner keeps this future alive even when
    // its caller is cancelled.
    let operation = robin_highscores::physical_work::drain(operation);
    tokio::pin!(operation);
    let result = loop {
        tokio::select! {
            result = &mut operation => break result,
            () = tokio::time::sleep(refresh_every) => {
                // A refresh panic must take the same drain path as a returned
                // error, not unwind past the pending physical-work owner.
                let refresh_result = std::panic::AssertUnwindSafe(async { refresh().await })
                    .catch_unwind().await;
                let refresh_error = match refresh_result {
                    Ok(Ok(true)) => continue,
                    Ok(Ok(false)) => anyhow::anyhow!(
                        "maintenance-write lease expired or was replaced during an active operation"
                    ),
                    Ok(Err(error)) => error,
                    Err(_) => anyhow::anyhow!("maintenance-write lease refresh panicked"),
                };
                // Dropping only the waiter cannot stop a queued hard link,
                // rename, unlink or verifier process. Retain the fence until
                // terminal completion, while preserving the heartbeat error.
                let drained = (&mut operation).await;
                break Err(match drained {
                    Ok(_) => refresh_error,
                    Err(error) => refresh_error.context(format!(
                        "worker operation also failed while draining: {error:#}"
                    )),
                });
            }
        }
    };
    let release = database.release_maintenance_write_lease(token).await;
    match (result, release) {
        (Ok(value), Ok(true)) => Ok(value),
        (Ok(_), Ok(false)) => anyhow::bail!("maintenance-write lease disappeared before release"),
        (Ok(_), Err(error)) => Err(error.into()),
        (Err(operation), Ok(true)) => Err(operation),
        (Err(operation), Ok(false)) => {
            Err(operation.context("operation failed and its maintenance-write lease disappeared"))
        }
        (Err(operation), Err(release)) => Err(operation.context(format!(
            "operation failed and releasing its maintenance-write lease also failed: {release}"
        ))),
    }
}

fn run_owned_fenced_operation<T, F>(
    database: Database,
    operation: F,
) -> impl Future<Output = anyhow::Result<T>> + Send
where
    T: Send + 'static,
    F: Future<Output = anyhow::Result<T>> + Send + 'static,
{
    // Keep the large replay-processing future out of each enclosing fence,
    // task-local scope, and worker-loop future. Otherwise their construction
    // can exhaust a Tokio worker thread's stack before the first job is leased.
    let operation = Box::pin(operation);
    async move {
        tokio::spawn(async move { database.run_fenced_operation(operation).await })
            .await
            .map_err(|error| {
                anyhow::anyhow!(error).context("owned database operation task failed")
            })?
    }
}

async fn initialize_worker_storage(
    database: &Database,
    server: &ServerConfig,
    owner: &str,
    lease_ttl: Duration,
    campaign_retention: Duration,
) -> anyhow::Result<(ReplayStore, CampaignStore)> {
    let write_lease = database
        .acquire_maintenance_write_lease(
            robin_highscores::db::MaintenanceWriteClass::Worker,
            owner,
            lease_ttl,
        )
        .await?;
    run_with_write_lease_heartbeat(database, &write_lease, lease_ttl, async {
        let replay_store =
            ReplayStore::create(server.replay_directory.clone(), server.max_replay_bytes).await?;
        let campaign_store = CampaignStore::create(
            server.campaign_state_directory.clone(),
            server.max_campaign_bytes,
        )
        .await?;
        database.health_check().await?;
        replay_store.readiness_check().await?;
        campaign_store.readiness_check().await?;
        let reconciled = reconcile_campaign_inventory(database, &campaign_store).await?;
        tracing::info!(reconciled, "reconciled private campaign object inventory");
        let collected =
            garbage_collect_campaigns(database, &campaign_store, campaign_retention, 1_000).await?;
        tracing::info!(collected, "completed campaign object garbage collection");
        Ok((replay_store, campaign_store))
    })
    .await
}

impl WorkerConfig {
    fn load(path: &Path) -> anyhow::Result<Self> {
        let config: Self = toml::from_str(std::str::from_utf8(&read_worker_config(path)?)?)?;
        let minimum_lease_seconds = config
            .verifier_launcher
            .wall_timeout_seconds
            .checked_add(30)
            .ok_or_else(|| anyhow::anyhow!("verifier timeout overflows the lease safety margin"))?;
        anyhow::ensure!(
            !config.worker_id.is_empty() && config.worker_id.len() <= 128,
            "worker_id must contain 1..=128 characters"
        );
        anyhow::ensure!(
            config.server_config.is_absolute()
                && config.campaign_state_directory.is_absolute()
                && config.verifier_job_config_catalog.is_absolute()
                && config.demo_raw_content_manifest.is_absolute()
                && config.full_raw_content_manifest.is_absolute(),
            "worker filesystem paths must be absolute"
        );
        anyhow::ensure!(
            config.poll_interval_ms > 0
                && config.lease_seconds > minimum_lease_seconds
                && config.retry_seconds > 0
                && (1..=100).contains(&config.max_verifier_attempts),
            "worker polling, lease, retry, and verifier timeout are inconsistent"
        );
        config.limits.validate()?;
        config
            .verifier_launcher
            .process_config(config.limits.max_campaign_bytes)?;
        Ok(config)
    }

    fn exact_digest(value: &str, name: &str) -> anyhow::Result<[u8; 32]> {
        anyhow::ensure!(
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
            "{name} must be 64 lowercase hexadecimal digits"
        );
        let bytes = hex::decode(value)?;
        anyhow::ensure!(bytes.len() == 32, "{name} must be 64 hexadecimal digits");
        let mut digest = [0; 32];
        digest.copy_from_slice(&bytes);
        Ok(digest)
    }

    fn verifier_digest(&self) -> anyhow::Result<[u8; 32]> {
        Ok(self.verifier_launcher.verifier_digest()?)
    }

    fn load_job_catalog(&self) -> anyhow::Result<VerifierJobConfigCatalogV1> {
        let bytes = read_regular_file_no_symlinks(
            &self.verifier_job_config_catalog,
            MAX_VERIFIER_JOB_CONFIG_BYTES_V1 as u64,
        )?;
        let expected = Self::exact_digest(
            &self.verifier_job_config_catalog_sha256,
            "verifier_job_config_catalog_sha256",
        )?;
        anyhow::ensure!(
            Digest32::digest_bytes(&bytes).as_bytes() == &expected,
            "verifier job-config catalog digest does not match its allowlist"
        );
        let catalog: VerifierJobConfigCatalogV1 = serde_json::from_slice(&bytes)?;
        catalog.validate()?;
        anyhow::ensure!(
            catalog.canonical_bytes()? == bytes,
            "verifier job-config catalog is not canonical JSON"
        );
        Ok(catalog)
    }
}

use robin_highscores::secure_fs::read_bounded_no_symlinks as read_regular_file_no_symlinks;

fn read_worker_config(path: &Path) -> anyhow::Result<Vec<u8>> {
    read_regular_file_no_symlinks(path, 1024 * 1024)
}

use robin_highscores::runtime_authority::validate_catalog_covers_server;
use robin_highscores::service::{
    ServiceNotifier, StartupStatusNotifier, SystemdNotifier, wait_for_shutdown_signal,
};

struct WorkerRuntime {
    worker: WorkerConfig,
    server: ServerConfig,
    database: Database,
    replay_store: ReplayStore,
    campaign_store: CampaignStore,
    campaign_retention: Duration,
    verifier_digest: [u8; 32],
    job_catalog: VerifierJobConfigCatalogV1,
    verifier: VerifierProcessConfig,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    if let Err(error) = run().await {
        tracing::error!(
            error_code = robin_highscores::safe_error_code(&error),
            "highscore verifier worker terminated"
        );
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let arguments = Arguments::parse();
    let notifier = SystemdNotifier::Worker;
    let runtime = initialize_worker(&arguments.config, &notifier).await?;
    if let Err(error) = notifier.ready() {
        return match runtime.database.close_fenced().await {
            Ok(()) => Err(error),
            Err(close) => Err(error.context(format!(
                "worker readiness failed and closing its fenced database pool also failed: {close:#}"
            ))),
        };
    }
    process_jobs(runtime).await
}

#[cfg(test)]
async fn run_worker_lifecycle<N, T, Startup, Process, ProcessFuture>(
    notifier: &N,
    startup: Startup,
    process: Process,
) -> anyhow::Result<()>
where
    N: ServiceNotifier,
    Startup: Future<Output = anyhow::Result<T>>,
    Process: FnOnce(T) -> ProcessFuture,
    ProcessFuture: Future<Output = anyhow::Result<()>>,
{
    let runtime = startup.await?;
    notifier.ready()?;
    process(runtime).await
}

async fn initialize_worker(
    config_path: &Path,
    notifier: &impl StartupStatusNotifier,
) -> anyhow::Result<WorkerRuntime> {
    notifier.status("Loading sealed worker and server configuration")?;
    let worker = WorkerConfig::load(config_path)?;
    let server = ServerConfig::load_for_worker(&worker.server_config)?;
    anyhow::ensure!(
        server.max_replay_bytes <= worker.limits.max_input_bytes,
        "server replay admission limit exceeds the worker's sealed-input limit"
    );
    anyhow::ensure!(
        server.max_campaign_bytes <= worker.limits.max_campaign_bytes,
        "server campaign admission limit exceeds the worker's sealed-input limit"
    );
    anyhow::ensure!(
        worker.campaign_state_directory == server.campaign_state_directory,
        "API and verifier worker must use the exact same campaign store"
    );
    let campaign_retention = Duration::from_secs(
        server
            .orphan_replay_retention_hours
            .checked_mul(60 * 60)
            .ok_or_else(|| anyhow::anyhow!("campaign retention overflow"))?,
    );
    notifier.status("Validating catalogs, authority, and raw Demo and Full content")?;
    let verifier_digest = worker.verifier_digest()?;
    let job_catalog = worker.load_job_catalog()?;
    validate_worker_authority_layout(
        &worker.verifier_job_config_catalog,
        &job_catalog,
        &server,
        &worker.demo_raw_content_manifest,
        &worker.full_raw_content_manifest,
    )?;
    for profile in &server.admission_profiles {
        let build_digest = Digest32::from_bytes(WorkerConfig::exact_digest(
            &profile.build_manifest_id,
            "admission profile build_manifest_id",
        )?);
        let build =
            server.manifests.builds.get(&build_digest).ok_or_else(|| {
                anyhow::anyhow!("admission profile build manifest is unavailable")
            })?;
        let ruleset_digest = Digest32::from_bytes(WorkerConfig::exact_digest(
            &profile.ruleset_id,
            "admission profile ruleset_id",
        )?);
        let published = server
            .manifests
            .rulesets
            .get(&ruleset_digest)
            .ok_or_else(|| anyhow::anyhow!("admission profile ruleset manifest is unavailable"))?;
        let policy = server
            .manifests
            .policies
            .get(&published.manifest.verifier_policy.manifest_sha256)
            .ok_or_else(|| anyhow::anyhow!("admission profile verifier policy is unavailable"))?;
        anyhow::ensure!(
            build.semantics().verifier.sha256.as_bytes() == &verifier_digest
                && policy.kind == published.manifest.verifier_policy.kind
                && policy.version == published.manifest.verifier_policy.version,
            "worker verifier executable or immutable policy does not match admission profile {}",
            profile.id
        );
    }
    validate_catalog_covers_server(&job_catalog, &server)?;
    notifier.status("Validating the isolated verifier launcher")?;
    let verifier = worker
        .verifier_launcher
        .process_config(worker.limits.max_campaign_bytes)?;
    verifier.validate().await?;

    notifier.status("Opening the database and acquiring the maintenance-write lease")?;
    let database = Database::connect(&server).await?;
    notifier.status("Opening stores and reconciling campaign inventory")?;
    let startup_database = database.clone();
    let startup_server = server.clone();
    let startup_owner = worker.worker_id.clone();
    let startup_lease_ttl = Duration::from_secs(worker.lease_seconds);
    let storage_result = run_owned_fenced_operation(database.clone(), async move {
        initialize_worker_storage(
            &startup_database,
            &startup_server,
            &startup_owner,
            startup_lease_ttl,
            campaign_retention,
        )
        .await
    })
    .await;
    let (replay_store, campaign_store) = match storage_result {
        Ok(stores) => stores,
        Err(error) => {
            let close = database.close_fenced().await;
            return match close {
                Ok(()) => Err(error),
                Err(close) => Err(error.context(format!(
                    "worker storage initialization failed and closing its fenced database pool also failed: {close:#}"
                ))),
            };
        }
    };

    Ok(WorkerRuntime {
        worker,
        server,
        database,
        replay_store,
        campaign_store,
        campaign_retention,
        verifier_digest,
        job_catalog,
        verifier,
    })
}

async fn process_jobs(runtime: WorkerRuntime) -> anyhow::Result<()> {
    let WorkerRuntime {
        worker,
        server,
        database,
        replay_store,
        campaign_store,
        campaign_retention,
        verifier_digest,
        job_catalog,
        verifier,
    } = runtime;
    let loop_database = database.clone();
    let mut worker_task = tokio::spawn(async move {
        let database = loop_database;
        let mut next_maintenance = tokio::time::Instant::now() + Duration::from_secs(60 * 60);
        let mut storage_admission_red = false;
        loop {
            if tokio::time::Instant::now() >= next_maintenance {
                let operation_database = database.clone();
                let operation_campaign_store = campaign_store.clone();
                let operation_owner = worker.worker_id.clone();
                let lease_ttl = Duration::from_secs(worker.lease_seconds);
                if let Err(error) = run_owned_fenced_operation(database.clone(), async move {
                    let maintenance_lease = operation_database
                        .acquire_maintenance_write_lease(
                            robin_highscores::db::MaintenanceWriteClass::Worker,
                            &operation_owner,
                            lease_ttl,
                        )
                        .await
                        .map_err(|error| {
                            tracing::warn!(
                                error_code = error.safe_log_code(),
                                "skipping campaign maintenance: could not acquire write lease"
                            );
                            error
                        })
                        .ok();
                    if let Some(maintenance_lease) = maintenance_lease {
                        run_with_write_lease_heartbeat(
                            &operation_database,
                            &maintenance_lease,
                            lease_ttl,
                            async {
                                match reconcile_campaign_inventory(
                                    &operation_database,
                                    &operation_campaign_store,
                                )
                                .await
                                {
                                    Ok(reconciled) => {
                                        tracing::debug!(reconciled, "reconciled campaign inventory")
                                    }
                                    Err(error) => {
                                        tracing::error!(%error, "campaign reconciliation failed")
                                    }
                                }
                                match garbage_collect_campaigns(
                                    &operation_database,
                                    &operation_campaign_store,
                                    campaign_retention,
                                    1_000,
                                )
                                .await
                                {
                                    Ok(collected) => tracing::info!(
                                        collected,
                                        "completed campaign garbage collection"
                                    ),
                                    Err(error) => tracing::error!(
                                        %error,
                                        "campaign garbage collection failed"
                                    ),
                                }
                                Ok(())
                            },
                        )
                        .await?;
                    }
                    Ok(())
                })
                .await
                {
                    tracing::warn!(
                        error_code = robin_highscores::safe_error_code(&error),
                        "scheduled worker maintenance could not enter the database fence"
                    );
                }
                next_maintenance = tokio::time::Instant::now() + Duration::from_secs(60 * 60);
            }
            if let Err(error) =
                ensure_worker_lease_capacity(&server, &database, &replay_store, &campaign_store)
            {
                if !storage_admission_red {
                    tracing::warn!(
                        error_code = error.safe_log_code(),
                        "storage admission is red; verifier worker will not lease a new job"
                    );
                    storage_admission_red = true;
                }
                tokio::time::sleep(Duration::from_millis(worker.poll_interval_ms)).await;
                continue;
            }
            if storage_admission_red {
                tracing::info!("storage admission recovered; verifier leasing resumed");
                storage_admission_red = false;
            }
            let operation_database = database.clone();
            let operation_worker = worker.clone();
            let operation_server = server.clone();
            let operation_replay_store = replay_store.clone();
            let operation_campaign_store = campaign_store.clone();
            let operation_verifier = verifier.clone();
            let operation_job_catalog = job_catalog.clone();
            let processed = run_owned_fenced_operation(database.clone(), async move {
                let write_lease = match operation_database
                    .acquire_maintenance_write_lease(
                        robin_highscores::db::MaintenanceWriteClass::Worker,
                        &operation_worker.worker_id,
                        Duration::from_secs(operation_worker.lease_seconds),
                    )
                    .await
                {
                    Ok(lease) => lease,
                    Err(error) => {
                        tracing::debug!(
                            error_code = error.safe_log_code(),
                            "worker write admission is quiesced for backup"
                        );
                        return Ok(None);
                    }
                };
                run_with_write_lease_heartbeat(
                    &operation_database,
                    &write_lease,
                    Duration::from_secs(operation_worker.lease_seconds),
                    async {
                        let Some(job) = operation_database
                            .lease_next(
                                &operation_worker.worker_id,
                                Duration::from_secs(operation_worker.lease_seconds),
                            )
                            .await?
                        else {
                            return Ok(Some(false));
                        };
                        let submission_id = job.submission_id.clone();
                        let attempts = job.attempts;
                        let outcome = Box::pin(process_job(
                            &operation_worker,
                            &operation_server,
                            &operation_database,
                            &operation_replay_store,
                            &operation_campaign_store,
                            &operation_verifier,
                            &operation_job_catalog,
                            verifier_digest,
                            job,
                        ))
                        .await;
                        if let Err(error) = outcome {
                            let typed_failure =
                                error.downcast_ref::<TypedWorkerInfrastructureFailure>();
                            let request_artifact_sha256 =
                                typed_failure.map(|failure| failure.request_artifact_sha256);
                            let private_detail = typed_failure.map_or_else(
                                || bounded_private_detail(&format!("worker_failure:{error:#}")),
                                |failure| {
                                    bounded_private_detail(
                                        failure.bounded_detail.as_deref().unwrap_or_else(|| {
                                            infrastructure_code_name(failure.code)
                                        }),
                                    )
                                },
                            );
                            log_verification_job_failure(
                                &submission_id,
                                attempts,
                                &error,
                                attempts >= operation_worker.max_verifier_attempts,
                            );
                            if attempts >= operation_worker.max_verifier_attempts {
                                if let Err(failure_error) = operation_database
                                    .fail_job(
                                        &submission_id,
                                        &operation_worker.worker_id,
                                        request_artifact_sha256.as_ref(),
                                        &private_detail,
                                    )
                                    .await
                                {
                                    tracing::error!(
                                        submission_id,
                                        error_code = failure_error.safe_log_code(),
                                        "could not record terminal infrastructure failure"
                                    );
                                }
                            } else if let Err(retry_error) = operation_database
                                .retry_job(
                                    &submission_id,
                                    &operation_worker.worker_id,
                                    Duration::from_secs(operation_worker.retry_seconds),
                                    &private_detail,
                                )
                                .await
                            {
                                tracing::error!(
                                    submission_id,
                                    error_code = retry_error.safe_log_code(),
                                    "could not schedule retry"
                                );
                            }
                        }
                        Ok(Some(true))
                    },
                )
                .await
            })
            .await?;
            if processed != Some(true) {
                tokio::time::sleep(Duration::from_millis(worker.poll_interval_ms)).await;
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    });

    let worker_result = tokio::select! {
        result = &mut worker_task => match result {
            Ok(result) => result,
            Err(error) => Err(anyhow::anyhow!(error).context("worker loop task failed")),
        },
        shutdown = wait_for_shutdown_signal() => {
            match shutdown {
                Ok(()) => {
                    tracing::info!("shutdown requested; draining database fence");
                    worker_task.abort();
                    let _ = worker_task.await;
                    Ok(())
                }
                Err(error) => {
                    worker_task.abort();
                    let _ = worker_task.await;
                    Err(error.context("worker shutdown signal failed"))
                }
            }
        }
    };
    let close_result = database.close_fenced().await;
    match (worker_result, close_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(error)) => Err(error.context("closing fenced worker database pool")),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(close)) => Err(error.context(format!(
            "worker failed and closing its fenced database pool also failed: {close:#}"
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_job(
    worker: &WorkerConfig,
    server: &ServerConfig,
    database: &Database,
    replay_store: &ReplayStore,
    campaign_store: &CampaignStore,
    verifier: &VerifierProcessConfig,
    job_catalog: &VerifierJobConfigCatalogV1,
    verifier_digest: [u8; 32],
    job: robin_highscores::model::WorkerJob,
) -> anyhow::Result<()> {
    let request = build_verification_request(&job, worker.limits.clone())?;
    let route = VerifierJobRouteV1::from_request(&request);
    let ranked = &request
        .submission
        .submission
        .offer
        .session_genesis
        .claim
        .ranked_session;
    let mut template = job_catalog
        .entries
        .iter()
        .find(|entry| {
            let mut candidate_route = entry.route.clone();
            if entry.ruleset_manifest.rules_config_constraint
                == robin_run_protocol::RulesConfigConstraintV1::AnyCanonicalSimConfig
            {
                candidate_route.rules_config_sha256 = route.rules_config_sha256;
            }
            candidate_route == route
        })
        .ok_or_else(|| {
            anyhow::anyhow!("authenticated job route is absent from the pinned catalog")
        })?
        .clone();
    if template.ruleset_manifest.rules_config_constraint
        == robin_run_protocol::RulesConfigConstraintV1::AnyCanonicalSimConfig
    {
        let custom = ranked.custom_rules_config.as_ref().ok_or_else(|| {
            anyhow::anyhow!("open-ruleset job is missing its signed configuration")
        })?;
        anyhow::ensure!(
            custom.rules == template.rules_config.rules
                && custom.replay_schema_version == template.rules_config.replay_schema_version,
            "custom configuration changes the immutable ranking policy"
        );
        robin_engine::simulation_inputs::validate_ranked_simulation_policy_rules_config_v1(custom)?;
        anyhow::ensure!(
            custom.canonical_digest()? == route.rules_config_sha256,
            "signed custom configuration digest differs from job route"
        );
        template.rules_config = custom.clone();
        template.route = route.clone();
        template
            .canonical_campaign_state
            .requirement
            .rules_config_sha256 = route.rules_config_sha256;
        template.canonical_campaign_state.artifact =
            ranked.custom_canonical_campaign.clone().ok_or_else(|| {
                anyhow::anyhow!("custom job is missing its canonical campaign proposal")
            })?;
    }
    let published_ruleset = server
        .manifests
        .rulesets
        .get(&route.ruleset_manifest_sha256)
        .ok_or_else(|| anyhow::anyhow!("authenticated job ruleset is unavailable"))?;
    let verifier_policy_manifest = server
        .manifests
        .policies
        .get(&published_ruleset.manifest.verifier_policy.manifest_sha256)
        .ok_or_else(|| anyhow::anyhow!("authenticated job verifier policy is unavailable"))?
        .clone();
    let campaign_authority = database
        .campaign_authority_for_verification(
            &job.submission_id,
            &worker.worker_id,
            &request,
            &published_ruleset.manifest,
        )
        .await?;
    anyhow::ensure!(
        campaign_authority.canonical_campaign_state == template.canonical_campaign_state,
        "database-persisted campaign-state pin differs from the exact verifier template"
    );
    let ranked = &request
        .submission
        .submission
        .offer
        .session_genesis
        .claim
        .ranked_session;
    let job_config = VerifierJobConfigV1 {
        schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
        template,
        verifier_policy_manifest,
        expected_prepared_inputs_projection_sha256: ranked.prepared_inputs_projection_sha256,
        expected_prepared_mission_inputs_seal_sha256: ranked.prepared_mission_inputs_seal_sha256,
        campaign_session: campaign_authority.campaign_session,
    };
    job_config.validate()?;
    let job_config_sha256 = job_config.canonical_digest()?;
    database
        .record_verification_request(
            &job.submission_id,
            &worker.worker_id,
            &request,
            &route,
            job_config_sha256,
            published_ruleset.manifest.verifier_policy.manifest_sha256,
        )
        .await?;
    let mut replay = replay_store
        .open_verified(&job.replay_sha256, job.replay_bytes)
        .await?;
    let mut starting = campaign_store
        .open_verified(&job.starting_campaign_sha256)
        .await?;
    let request_artifact_sha256 =
        Digest32::digest_bytes(&serde_json::to_vec(&request)?).into_bytes();
    let output = verifier
        .run(&request, &job_config, &mut replay, &mut starting)
        .await
        .map_err(|error| process_infrastructure_failure(error, request_artifact_sha256))?;
    anyhow::ensure!(
        output.job_config_artifact_sha256 == job_config_sha256,
        "supervisor sealed a different per-job verifier authority"
    );
    match &output.outcome {
        VerifierWorkerOutputV1::VerificationResult { result, .. } => match &result.status {
            VerificationStatusV1::Verified(verified) => {
                let offer = &request.submission.submission.offer;
                let build_manifest = server
                    .manifests
                    .builds
                    .get(&offer.build_manifest_sha256)
                    .ok_or_else(|| anyhow::anyhow!("signed offer build manifest is unavailable"))?;
                let published_ruleset = server
                    .manifests
                    .rulesets
                    .get(&offer.ruleset_manifest_sha256)
                    .ok_or_else(|| anyhow::anyhow!("signed offer ruleset is unavailable"))?;
                let content_manifest = server
                    .manifests
                    .content_manifests
                    .get(&offer.content_manifest_sha256)
                    .ok_or_else(|| anyhow::anyhow!("signed content manifest is unavailable"))?;
                let campaign_content_manifest = offer
                    .session_genesis
                    .claim
                    .ranked_session
                    .campaign_content_manifest_sha256
                    .map(|digest| {
                        server
                            .manifests
                            .campaign_content_manifests
                            .get(&digest)
                            .ok_or_else(|| {
                                anyhow::anyhow!("signed campaign content catalog is unavailable")
                            })
                    })
                    .transpose()?;
                if offer.starting_state.scope_kind() == robin_run_protocol::RunScopeKindV1::Campaign
                    && campaign_content_manifest.is_none()
                {
                    anyhow::bail!("campaign verification omitted its signed campaign catalog")
                }
                result.validate_campaign_complete_evidence(
                    &request,
                    &published_ruleset.manifest,
                    campaign_content_manifest,
                )?;
                let final_campaign_bytes = output.final_campaign.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("verified result omitted its required final campaign output")
                })?;
                anyhow::ensure!(
                    u64::try_from(final_campaign_bytes.len())?
                        == verified.final_campaign.byte_length
                        && Digest32::digest_bytes(final_campaign_bytes)
                            == verified.final_campaign.sha256,
                    "verified final campaign output differs from its typed artifact reference"
                );
                ensure_worker_final_campaign_capacity(
                    server,
                    database,
                    replay_store,
                    campaign_store,
                    verified.final_campaign.byte_length,
                )?;
                campaign_store
                    .import_bytes(
                        verified.final_campaign.sha256.as_bytes(),
                        final_campaign_bytes,
                    )
                    .await?;
                database
                    .register_campaign_object(
                        verified.final_campaign.sha256.as_bytes(),
                        verified.final_campaign.byte_length,
                    )
                    .await?;
                let competition_manifest = offer
                    .competition_manifest_sha256
                    .map(|digest| {
                        server.manifests.competitions.get(&digest).ok_or_else(|| {
                            anyhow::anyhow!("signed competition manifest is unavailable")
                        })
                    })
                    .transpose()?;
                let run_id = match database
                    .accept_job(
                        &job.submission_id,
                        &worker.worker_id,
                        verifier_digest,
                        output.job_config_artifact_sha256,
                        result,
                        build_manifest,
                        content_manifest,
                        campaign_content_manifest,
                        published_ruleset,
                        competition_manifest,
                    )
                    .await
                {
                    Ok(run_id) => run_id,
                    Err(robin_highscores::db::DbError::CampaignFork) => {
                        database
                            .reject_job(
                                &job.submission_id,
                                &worker.worker_id,
                                "starting_state_mismatch",
                                "campaign_predecessor_consumed",
                            )
                            .await?;
                        tracing::info!(
                            submission_id = job.submission_id,
                            "verified run lost campaign predecessor compare-and-swap"
                        );
                        return Ok(());
                    }
                    Err(error) => return Err(error.into()),
                };
                tracing::info!(
                    submission_id = job.submission_id,
                    run_id,
                    "verified run accepted"
                );
            }
            VerificationStatusV1::Rejected(rejection) => {
                let code = rejection_code(rejection.code);
                database
                    .reject_job(
                        &job.submission_id,
                        &worker.worker_id,
                        code,
                        rejection
                            .detail_code
                            .as_deref()
                            .unwrap_or("typed_rejection"),
                    )
                    .await?;
                tracing::info!(submission_id = job.submission_id, code, "run rejected");
            }
            VerificationStatusV1::FailedInfrastructure(failure) => {
                return Err(TypedWorkerInfrastructureFailure {
                    code: failure.code,
                    request_artifact_sha256: output.request_artifact_sha256.into_bytes(),
                    bounded_detail: failure.private_detail_code.clone(),
                }
                .into());
            }
        },
        VerifierWorkerOutputV1::AdmissionFailure {
            code,
            bounded_detail,
            request_artifact_sha256,
            ..
        } => {
            if matches!(code, VerifierAdmissionFailureCodeV1::WorkerInternalFailure) {
                return Err(TypedWorkerInfrastructureFailure {
                    code: VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
                    request_artifact_sha256: request_artifact_sha256.into_bytes(),
                    bounded_detail: bounded_detail.clone(),
                }
                .into());
            }
            let rejection = admission_failure_rejection_code(*code)?;
            database
                .reject_job(
                    &job.submission_id,
                    &worker.worker_id,
                    rejection,
                    bounded_detail
                        .as_deref()
                        .unwrap_or("typed_admission_failure"),
                )
                .await?;
            tracing::warn!(
                submission_id = job.submission_id,
                rejection,
                ?code,
                "verifier rejected the sealed request before proof construction"
            );
        }
    }
    Ok(())
}

fn rejection_code(code: VerificationRejectionCodeV1) -> &'static str {
    code.as_str()
}

fn admission_failure_rejection_code(
    code: VerifierAdmissionFailureCodeV1,
) -> anyhow::Result<&'static str> {
    match code {
        VerifierAdmissionFailureCodeV1::RequestTooLarge => Ok("resource_limit"),
        VerifierAdmissionFailureCodeV1::UnsupportedRequestSchema => Ok("unsupported_schema"),
        VerifierAdmissionFailureCodeV1::MalformedRequest
        | VerifierAdmissionFailureCodeV1::RequestAuthenticationFailed => Ok("malformed_replay"),
        VerifierAdmissionFailureCodeV1::WorkerInternalFailure => {
            anyhow::bail!("worker-internal admission failure must use infrastructure retry policy")
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error(
    "verifier worker internal failure for sealed request {request_artifact_sha256:?}: {bounded_detail:?}"
)]
struct TypedWorkerInfrastructureFailure {
    code: VerificationInfrastructureFailureCodeV1,
    bounded_detail: Option<String>,
    request_artifact_sha256: [u8; 32],
}

const fn infrastructure_code_name(code: VerificationInfrastructureFailureCodeV1) -> &'static str {
    match code {
        VerificationInfrastructureFailureCodeV1::WorkerInternalFailure => "worker_internal_failure",
        VerificationInfrastructureFailureCodeV1::WorkerUnavailable => "worker_unavailable",
        VerificationInfrastructureFailureCodeV1::VerifierProcessFailure => {
            "verifier_process_failure"
        }
        VerificationInfrastructureFailureCodeV1::ArtifactIoFailure => "artifact_io_failure",
        VerificationInfrastructureFailureCodeV1::InfrastructureTimeout => "infrastructure_timeout",
    }
}

fn process_infrastructure_failure(
    error: ProcessError,
    request_artifact_sha256: [u8; 32],
) -> TypedWorkerInfrastructureFailure {
    let code = match &error {
        ProcessError::Timeout => VerificationInfrastructureFailureCodeV1::InfrastructureTimeout,
        ProcessError::Io(_) => VerificationInfrastructureFailureCodeV1::ArtifactIoFailure,
        ProcessError::Exit(_) => VerificationInfrastructureFailureCodeV1::VerifierProcessFailure,
        ProcessError::Configuration(_) | ProcessError::InvalidResult(_) => {
            VerificationInfrastructureFailureCodeV1::WorkerInternalFailure
        }
    };
    TypedWorkerInfrastructureFailure {
        code,
        bounded_detail: Some(error.safe_log_code().to_owned()),
        request_artifact_sha256,
    }
}

fn safe_worker_error_code(error: &anyhow::Error) -> &'static str {
    if error
        .downcast_ref::<TypedWorkerInfrastructureFailure>()
        .is_some()
    {
        "verifier_worker_internal"
    } else {
        robin_highscores::safe_error_code(error)
    }
}

fn log_verification_job_failure(
    submission_id: &str,
    attempts: u32,
    error: &anyhow::Error,
    exhausted: bool,
) {
    let error_code = safe_worker_error_code(error);
    if exhausted {
        tracing::error!(
            submission_id,
            attempts,
            error_code,
            "verification job exhausted its bounded retry policy"
        );
    } else {
        tracing::error!(
            submission_id,
            attempts,
            error_code,
            "verification job failed; scheduling retry"
        );
    }
}

fn bounded_private_detail(detail: &str) -> String {
    const LIMIT: usize = 2000;
    if detail.len() <= LIMIT {
        return detail.to_owned();
    }
    let mut end = LIMIT;
    while !detail.is_char_boundary(end) {
        end -= 1;
    }
    detail[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn owned_fence_keeps_large_operations_out_of_the_callers_future() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("highscores.sqlite3");
        let database = Database::migrate(&config).await.unwrap();
        let payload = [7_u8; 256 * 1024];
        let operation = async move {
            tokio::task::yield_now().await;
            Ok(std::hint::black_box(payload)[0])
        };
        assert!(std::mem::size_of_val(&operation) >= 256 * 1024);
        let owned = run_owned_fenced_operation(database.clone(), operation);
        assert!(
            std::mem::size_of_val(&owned) <= 1024,
            "owned fence must not embed the replay operation in its caller"
        );
        assert_eq!(owned.await.unwrap(), 7);
        database.close_fenced().await.unwrap();
    }

    async fn assert_physical_work_is_fenced(mode: &'static str) {
        let directory = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("highscores.sqlite3");
        let database = Database::migrate(&config).await.unwrap();
        let mutation = directory.path().join("published-object");
        let write_path = mutation.clone();
        let (started, wait_started) = tokio::sync::oneshot::channel();
        let (release, wait_release) = std::sync::mpsc::channel();
        let refresh_seen = Arc::new(tokio::sync::Notify::new());
        let notify_refresh = Arc::clone(&refresh_seen);
        let owner_database = database.clone();
        let operation_database = database.clone();
        let mut caller = tokio::spawn(async move {
            run_owned_fenced_operation(owner_database, async move {
                let token = operation_database
                    .acquire_maintenance_write_lease(
                        robin_highscores::db::MaintenanceWriteClass::Worker,
                        "physical-work-test",
                        Duration::from_secs(60),
                    )
                    .await?;
                run_with_write_lease_heartbeat_using(
                    &operation_database,
                    &token,
                    Duration::from_millis(30),
                    async move {
                        let physical = robin_highscores::physical_work::spawn_blocking(move || {
                            started.send(()).unwrap();
                            // A failing assertion must not leave the Tokio runtime
                            // hanging forever on an intentionally blocked closure.
                            wait_release.recv_timeout(Duration::from_secs(10)).unwrap();
                            std::fs::write(write_path, b"published").unwrap();
                        });
                        if mode == "operation_error" {
                            drop(physical);
                            anyhow::bail!("injected operation error");
                        }
                        if mode == "operation_panic" {
                            drop(physical);
                            panic!("injected operation panic");
                        }
                        physical.await?;
                        if mode == "heartbeat_then_operation_panic" {
                            panic!("injected operation panic after heartbeat failure");
                        }
                        Ok(())
                    },
                    move || {
                        let notify_refresh = Arc::clone(&notify_refresh);
                        async move {
                            notify_refresh.notify_one();
                            match mode {
                                "heartbeat_lost" => Ok(false),
                                "heartbeat_error" | "heartbeat_then_operation_panic" => {
                                    anyhow::bail!("injected heartbeat error")
                                }
                                "heartbeat_panic" => panic!("injected heartbeat panic"),
                                _ => Ok(true),
                            }
                        }
                    },
                )
                .await
            })
            .await
        });
        tokio::time::timeout(Duration::from_secs(5), wait_started)
            .await
            .unwrap()
            .unwrap();
        if mode.starts_with("heartbeat_") {
            tokio::time::timeout(Duration::from_secs(5), refresh_seen.notified())
                .await
                .unwrap();
        }
        if mode == "caller_cancelled" {
            caller.abort();
            assert!((&mut caller).await.unwrap_err().is_cancelled());
        } else {
            // A notification proves refresh was attempted, not that its owner
            // has reached release. Require it to remain pending long enough to
            // expose the old release-before-join policy, not just sample it.
            assert!(
                tokio::time::timeout(Duration::from_millis(100), &mut caller)
                    .await
                    .is_err(),
                "worker owner returned while physical mutation was still blocked"
            );
        }
        assert!(!mutation.exists());
        assert!(
            database
                .runtime_fence()
                .try_lock_exclusive_quiescence()
                .unwrap()
                .is_none(),
            "physical work outlived the shared database fence"
        );
        assert_eq!(
            database
                .active_maintenance_write_lease_count()
                .await
                .unwrap(),
            1,
            "physical work outlived lease ownership"
        );
        release.send(()).unwrap();
        if mode != "caller_cancelled" {
            let result = tokio::time::timeout(Duration::from_secs(5), caller)
                .await
                .unwrap()
                .unwrap();
            if mode == "success" {
                result.unwrap();
            } else {
                let error = format!("{:#}", result.unwrap_err());
                let expected = match mode {
                    "heartbeat_lost" => "expired or was replaced",
                    "heartbeat_error" | "heartbeat_then_operation_panic" => {
                        "injected heartbeat error"
                    }
                    "heartbeat_panic" => "lease refresh panicked",
                    "operation_error" => "injected operation error",
                    "operation_panic" => "panicked after physical-work drain",
                    _ => unreachable!(),
                };
                assert!(error.contains(expected), "{error}");
            }
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if database
                    .runtime_fence()
                    .try_lock_exclusive_quiescence()
                    .unwrap()
                    .is_some()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(mutation).unwrap(), b"published");
        assert_eq!(
            database
                .active_maintenance_write_lease_count()
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn physical_work_is_drained_after_heartbeat_loss() {
        assert_physical_work_is_fenced("heartbeat_lost").await;
    }

    #[tokio::test]
    async fn physical_work_is_drained_after_heartbeat_error() {
        assert_physical_work_is_fenced("heartbeat_error").await;
    }

    #[tokio::test]
    async fn physical_work_is_drained_after_operation_error() {
        assert_physical_work_is_fenced("operation_error").await;
    }

    #[tokio::test]
    async fn physical_work_is_drained_after_caller_cancellation() {
        assert_physical_work_is_fenced("caller_cancelled").await;
    }

    #[tokio::test]
    async fn physical_work_is_drained_after_success() {
        assert_physical_work_is_fenced("success").await;
    }

    #[tokio::test]
    #[ignore = "requires explicit LLVM backend for actual catch_unwind/destructor execution"]
    async fn physical_work_is_drained_after_operation_panic() {
        assert_physical_work_is_fenced("operation_panic").await;
    }

    #[tokio::test]
    #[ignore = "requires explicit LLVM backend for actual catch_unwind/destructor execution"]
    async fn physical_work_is_drained_after_heartbeat_panic() {
        assert_physical_work_is_fenced("heartbeat_panic").await;
    }

    #[tokio::test]
    #[ignore = "requires explicit LLVM backend for actual catch_unwind/destructor execution"]
    async fn physical_work_is_drained_after_heartbeat_then_operation_panic() {
        assert_physical_work_is_fenced("heartbeat_then_operation_panic").await;
    }

    #[derive(Clone, Default)]
    struct RecordingNotifier {
        events: Arc<Mutex<Vec<&'static str>>>,
        fail_ready: bool,
    }

    impl StartupStatusNotifier for RecordingNotifier {
        fn status(&self, _status: &str) -> anyhow::Result<()> {
            self.events.lock().unwrap().push("status");
            Ok(())
        }
    }

    impl ServiceNotifier for RecordingNotifier {
        fn ready(&self) -> anyhow::Result<()> {
            if self.fail_ready {
                self.events.lock().unwrap().push("ready_failed");
                anyhow::bail!("test readiness notification failure");
            }
            self.events.lock().unwrap().push("ready");
            Ok(())
        }
    }

    #[tokio::test]
    async fn startup_validation_failure_never_announces_ready_or_processes_jobs() {
        let notifier = RecordingNotifier::default();
        let startup_events = Arc::clone(&notifier.events);
        let process_events = Arc::clone(&notifier.events);
        let result = run_worker_lifecycle(
            &notifier,
            async move {
                startup_events.lock().unwrap().push("validation_failed");
                anyhow::bail!("test startup validation failure")
            },
            move |()| async move {
                process_events.lock().unwrap().push("processing");
                Ok(())
            },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(
            notifier.events.lock().unwrap().as_slice(),
            ["validation_failed"]
        );
    }

    #[tokio::test]
    async fn successful_startup_announces_ready_immediately_before_processing_jobs() {
        let notifier = RecordingNotifier::default();
        let startup_events = Arc::clone(&notifier.events);
        let process_events = Arc::clone(&notifier.events);
        run_worker_lifecycle(
            &notifier,
            async move {
                startup_events.lock().unwrap().push("validation_complete");
                Ok(())
            },
            move |()| async move {
                process_events.lock().unwrap().push("processing");
                Ok(())
            },
        )
        .await
        .unwrap();

        assert_eq!(
            notifier.events.lock().unwrap().as_slice(),
            ["validation_complete", "ready", "processing"]
        );
    }

    #[tokio::test]
    async fn readiness_notification_failure_is_fatal_before_processing_jobs() {
        let notifier = RecordingNotifier {
            fail_ready: true,
            ..RecordingNotifier::default()
        };
        let startup_events = Arc::clone(&notifier.events);
        let process_events = Arc::clone(&notifier.events);
        let result = run_worker_lifecycle(
            &notifier,
            async move {
                startup_events.lock().unwrap().push("validation_complete");
                Ok(())
            },
            move |()| async move {
                process_events.lock().unwrap().push("processing");
                Ok(())
            },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(
            notifier.events.lock().unwrap().as_slice(),
            ["validation_complete", "ready_failed"]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn systemd_process_validation_failure_exits_without_ready() {
        let (output, messages) = run_systemd_notifier_child("failure");

        assert!(!output.status.success());
        assert!(
            messages
                .iter()
                .any(|message| message.contains("validating startup"))
        );
        assert!(!messages.iter().any(|message| message.contains("READY=1")));
        assert!(
            !messages
                .iter()
                .any(|message| message.contains("processing queue"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn systemd_process_success_sends_ready_before_processing() {
        let (output, messages) = run_systemd_notifier_child("success");

        assert!(
            output.status.success(),
            "notifier child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let ready = messages
            .iter()
            .position(|message| message.contains("READY=1"))
            .expect("child did not send READY=1");
        let processing = messages
            .iter()
            .position(|message| message.contains("processing queue"))
            .expect("child did not enter processing");
        assert!(ready < processing, "READY=1 must precede processing");
    }

    #[cfg(target_os = "linux")]
    fn run_systemd_notifier_child(mode: &str) -> (std::process::Output, Vec<String>) {
        use std::os::unix::net::UnixDatagram;

        let temporary = tempfile::tempdir().unwrap();
        let socket_path = temporary.path().join("notify.socket");
        let socket = UnixDatagram::bind(&socket_path).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("tests::systemd_notifier_process_child")
            .arg("--ignored")
            .arg("--nocapture")
            .env("ROBIN_HIGHSCORES_NOTIFY_TEST_MODE", mode)
            .env("NOTIFY_SOCKET", &socket_path)
            .output()
            .unwrap();

        socket
            .set_read_timeout(Some(Duration::from_millis(250)))
            .unwrap();
        let mut messages = Vec::new();
        loop {
            let mut bytes = [0_u8; 4096];
            match socket.recv(&mut bytes) {
                Ok(length) => messages.push(String::from_utf8(bytes[..length].to_vec()).unwrap()),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(error) => panic!("could not read notifier child datagram: {error}"),
            }
        }
        (output, messages)
    }

    #[cfg(target_os = "linux")]
    #[ignore = "process helper invoked by the systemd notification integration tests"]
    #[tokio::test]
    async fn systemd_notifier_process_child() {
        let mode = std::env::var("ROBIN_HIGHSCORES_NOTIFY_TEST_MODE").unwrap();
        let notifier = SystemdNotifier::Worker;
        let startup_notifier = &notifier;
        let processing_notifier = &notifier;
        run_worker_lifecycle(
            &notifier,
            async move {
                startup_notifier.status("Process test: validating startup")?;
                anyhow::ensure!(mode == "success", "test startup validation failure");
                Ok(())
            },
            move |()| async move {
                processing_notifier.status("Process test: processing queue")?;
                Ok(())
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn active_backup_gate_prevents_worker_startup_from_creating_storage_roots() {
        let directory = tempfile::tempdir().unwrap();
        let mut server = ServerConfig::default();
        server.database_path = directory.path().join("highscores.sqlite3");
        server.replay_directory = directory.path().join("objects/replays");
        server.campaign_state_directory = directory.path().join("objects/campaigns");
        let database = Database::migrate(&server).await.unwrap();
        let backup = database
            .acquire_backup_lock("worker-startup-gate-test", Duration::from_secs(60))
            .await
            .unwrap();

        let result = initialize_worker_storage(
            &database,
            &server,
            "worker-startup-gate-test",
            Duration::from_secs(60),
            Duration::from_secs(60),
        )
        .await;

        assert!(result.is_err());
        assert!(!server.replay_directory.exists());
        assert!(!server.campaign_state_directory.exists());
        assert!(!directory.path().join("objects").exists());
        assert!(database.release_backup_lock(&backup).await.unwrap());
    }

    #[test]
    fn worker_internal_failure_can_never_become_a_replay_rejection() {
        assert!(
            admission_failure_rejection_code(VerifierAdmissionFailureCodeV1::WorkerInternalFailure)
                .is_err()
        );
        assert_eq!(
            admission_failure_rejection_code(VerifierAdmissionFailureCodeV1::MalformedRequest)
                .unwrap(),
            "malformed_replay"
        );
    }

    #[test]
    fn private_worker_diagnostics_are_byte_bounded_on_utf8_boundaries() {
        let detail = "é".repeat(2000);
        let bounded = bounded_private_detail(&detail);
        assert!(bounded.len() <= 2000);
        assert!(bounded.is_char_boundary(bounded.len()));
    }

    #[tracing_test::traced_test]
    #[test]
    fn verifier_private_detail_is_not_a_log_field() {
        let private_path = "/srv/private/fullgame/content.bundle";
        let anonymous_key = "a1".repeat(32);
        let signature = "b2".repeat(64);
        let private_detail = format!(
            "path={private_path} anonymous_key={anonymous_key} signature={signature} evidence=PrivateVerifierEvidence"
        );
        let error = anyhow::Error::new(TypedWorkerInfrastructureFailure {
            code: VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
            bounded_detail: Some(private_detail.clone()),
            request_artifact_sha256: [7; 32],
        });
        assert_eq!(safe_worker_error_code(&error), "verifier_worker_internal");
        assert!(format!("{error:#}").contains(&private_detail));

        log_verification_job_failure("submission-safe-id", 3, &error, false);
        assert!(logs_contain("verifier_worker_internal"));
        for sentinel in [
            private_path,
            anonymous_key.as_str(),
            signature.as_str(),
            "PrivateVerifierEvidence",
        ] {
            assert!(
                !logs_contain(sentinel),
                "private log sentinel leaked: {sentinel}"
            );
        }
    }
}
