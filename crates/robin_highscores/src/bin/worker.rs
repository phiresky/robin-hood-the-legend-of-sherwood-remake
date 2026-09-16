#![forbid(unsafe_code)]

#[cfg(not(target_os = "linux"))]
compile_error!("robin-highscores-worker is Linux-only");

use clap::Parser;
use robin_highscores::verifier::{ProcessError, decode_output};
use robin_highscores::worker_config::WorkerConfig;
use robin_highscores::{
    Database, ReplayStore, ServerConfig, storage_admission::ensure_worker_lease_capacity,
};
use robin_run_protocol::{
    ArtifactRefV1, Digest32, OpaqueId, RANKED_REPLAY_MEDIA_TYPE_V1, ReplayArtifactV1,
    SCHEMA_VERSION_V2, Validate as _, VerificationInfrastructureFailureCodeV1,
    VerificationRejectionCodeV1, VerificationStatusV2, VerifierJobV2,
};
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
    // lease. The owned task from `run_owned_operation` keeps this future alive
    // even when its caller is cancelled.
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
                // Dropping only the waiter cannot stop a queued file write or
                // verifier process. Retain the lease until terminal completion,
                // while preserving the heartbeat error.
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

fn run_owned_operation<T, F>(operation: F) -> impl Future<Output = anyhow::Result<T>> + Send
where
    T: Send + 'static,
    F: Future<Output = anyhow::Result<T>> + Send + 'static,
{
    // Keep the large replay-processing future out of each enclosing task-local
    // scope and worker-loop future. The owned task also survives cancellation
    // of the caller.
    let operation = Box::pin(operation);
    async move {
        tokio::spawn(operation).await.map_err(|error| {
            anyhow::anyhow!(error).context("owned database operation task failed")
        })?
    }
}

async fn initialize_worker_storage(
    database: &Database,
    server: &ServerConfig,
    owner: &str,
    lease_ttl: Duration,
) -> anyhow::Result<ReplayStore> {
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
        database.health_check().await?;
        replay_store.readiness_check().await?;
        Ok(replay_store)
    })
    .await
}

use robin_highscores::service::{
    ServiceNotifier, StartupStatusNotifier, SystemdNotifier, wait_for_shutdown_signal,
};

struct WorkerRuntime {
    worker: WorkerConfig,
    server: ServerConfig,
    database: Database,
    replay_store: ReplayStore,
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
        runtime.database.close().await;
        return Err(error);
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
    notifier.status("Loading worker and server configuration")?;
    let worker = WorkerConfig::load(config_path)?;
    let server = ServerConfig::load_for_worker(&worker.server_config)?;
    anyhow::ensure!(
        server.max_replay_bytes <= worker.limits.max_input_bytes,
        "server replay admission limit exceeds the verifier's input limit"
    );
    notifier.status("Validating the isolated verifier launcher and raw content roots")?;
    worker.verifier_launcher.validate()?;
    for (edition, content) in [
        ("demo", &worker.content.demo),
        ("full", &worker.content.full),
    ] {
        let metadata = std::fs::metadata(&content.root).map_err(|error| {
            anyhow::anyhow!("{edition} raw content root is not accessible: {error}")
        })?;
        anyhow::ensure!(
            metadata.is_dir(),
            "{edition} raw content root is not a directory"
        );
    }

    notifier.status("Opening the database and acquiring the maintenance-write lease")?;
    let database = Database::connect(&server).await?;
    notifier.status("Opening the replay store")?;
    let startup_database = database.clone();
    let startup_server = server.clone();
    let startup_owner = worker.worker_id.clone();
    let startup_lease_ttl = Duration::from_secs(worker.lease_seconds);
    let storage_result = run_owned_operation(async move {
        initialize_worker_storage(
            &startup_database,
            &startup_server,
            &startup_owner,
            startup_lease_ttl,
        )
        .await
    })
    .await;
    let replay_store = match storage_result {
        Ok(store) => store,
        Err(error) => {
            database.close().await;
            return Err(error);
        }
    };
    Ok(WorkerRuntime {
        worker,
        server,
        database,
        replay_store,
    })
}

async fn process_jobs(runtime: WorkerRuntime) -> anyhow::Result<()> {
    let WorkerRuntime {
        worker,
        server,
        database,
        replay_store,
    } = runtime;
    let loop_database = database.clone();
    let mut worker_task = tokio::spawn(async move {
        let database = loop_database;
        let mut storage_admission_red = false;
        loop {
            if let Err(error) = ensure_worker_lease_capacity(&server, &database, &replay_store) {
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
            let processed = run_owned_operation(async move {
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
                        tracing::warn!(
                            error_code = error.safe_log_code(),
                            "skipping job lease: could not acquire write lease"
                        );
                        return Ok(false);
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
                            return Ok(false);
                        };
                        let submission_id = job.submission_id.clone();
                        let attempts = job.attempts;
                        let outcome = Box::pin(process_job(
                            &operation_worker,
                            &operation_server,
                            &operation_database,
                            &operation_replay_store,
                            job,
                        ))
                        .await;
                        if let Err(error) = outcome {
                            let exhausted = attempts >= operation_worker.max_verifier_attempts;
                            let private_detail = private_failure_detail(&error);
                            log_verification_job_failure(
                                &submission_id,
                                attempts,
                                &error,
                                exhausted,
                            );
                            let transition = if exhausted {
                                operation_database
                                    .fail_job(
                                        &submission_id,
                                        &operation_worker.worker_id,
                                        &private_detail,
                                    )
                                    .await
                            } else {
                                operation_database
                                    .retry_job(
                                        &submission_id,
                                        &operation_worker.worker_id,
                                        Duration::from_secs(operation_worker.retry_seconds),
                                        &private_detail,
                                    )
                                    .await
                            };
                            if let Err(transition_error) = transition {
                                tracing::error!(
                                    submission_id,
                                    error_code = transition_error.safe_log_code(),
                                    "could not record verification failure or retry"
                                );
                            }
                        }
                        Ok(true)
                    },
                )
                .await
            })
            .await?;
            if !processed {
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
            worker_task.abort();
            let _ = worker_task.await;
            match shutdown {
                Ok(()) => {
                    tracing::info!("shutdown requested; stopped the worker loop");
                    Ok(())
                }
                Err(error) => Err(error.context("worker shutdown signal failed")),
            }
        }
    };
    database.close().await;
    worker_result
}

/// Build the exact verifier job for a leased submission.
fn verifier_job(
    worker: &WorkerConfig,
    board: &robin_run_protocol::BoardV2,
    job: &robin_highscores::model::WorkerJob,
) -> anyhow::Result<VerifierJobV2> {
    let verifier_job = VerifierJobV2 {
        schema_version: SCHEMA_VERSION_V2,
        job_id: OpaqueId::new(job.submission_id.clone())?,
        edition: board.edition,
        mission_id: job.mission_id.clone(),
        simulation_policy: board.simulation_policy,
        allow_state_load: board.allow_state_load,
        replay: ReplayArtifactV1 {
            artifact: ArtifactRefV1 {
                sha256: Digest32::from_bytes(job.replay_sha256),
                byte_length: job.replay_bytes,
                media_type: RANKED_REPLAY_MEDIA_TYPE_V1.to_owned(),
            },
            replay_schema_version: job.replay_schema_version,
        },
        resource_locale_root: worker
            .content
            .edition(board.edition)
            .resource_locale_root
            .clone(),
        limits: worker.limits.clone(),
    };
    verifier_job.validate()?;
    Ok(verifier_job)
}

async fn process_job(
    worker: &WorkerConfig,
    server: &ServerConfig,
    database: &Database,
    replay_store: &ReplayStore,
    job: robin_highscores::model::WorkerJob,
) -> anyhow::Result<()> {
    if job.replay_schema_version != robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1 {
        // Queued before a verifier upgrade changed the replay schema: the
        // current verifier can never resimulate it.
        database
            .reject_job(
                &job.submission_id,
                &worker.worker_id,
                VerificationRejectionCodeV1::UnsupportedSchema.as_str(),
                Some("replay_schema_superseded"),
            )
            .await?;
        return Ok(());
    }
    // TODO: a board removed from configuration while jobs are queued keeps
    // failing until the retry policy marks those submissions failed; an
    // operator-visible rejection code for retired boards may be clearer.
    let board = server
        .board(&OpaqueId::new(job.board_id.clone())?)
        .ok_or_else(|| anyhow::anyhow!("leased submission references an unconfigured board"))?;
    anyhow::ensure!(
        board.mission(&job.mission_id).is_some(),
        "leased submission mission is no longer part of its board"
    );
    let verifier_job = verifier_job(worker, board, &job)?;
    let job_bytes = serde_json::to_vec(&verifier_job)?;
    let replay = replay_store
        .open_verified(&job.replay_sha256, job.replay_bytes)
        .await?;
    let content_root = &worker.content.edition(board.edition).root;
    let result_bytes = worker
        .verifier_launcher
        .run(&job_bytes, replay, job.replay_bytes, content_root)
        .await?;
    let output = decode_output(
        &result_bytes.result,
        &job_bytes,
        Digest32::from_bytes(job.replay_sha256),
    )?;
    match &output.status {
        VerificationStatusV2::Verified(_) => {
            if !result_bytes.checkpoints.is_empty() {
                replay_store
                    .store_checkpoints(&job.replay_sha256, result_bytes.checkpoints)
                    .await?;
            }
            let run_id = database
                .accept_job(
                    &job.submission_id,
                    &worker.worker_id,
                    Digest32::digest_bytes(&job_bytes),
                    board,
                    &output,
                )
                .await?;
            tracing::info!(
                submission_id = job.submission_id,
                run_id,
                "verified run accepted"
            );
        }
        VerificationStatusV2::Rejected(rejection) => {
            let code = rejection.code.as_str();
            database
                .reject_job(
                    &job.submission_id,
                    &worker.worker_id,
                    code,
                    rejection.detail_code.as_deref(),
                )
                .await?;
            tracing::info!(submission_id = job.submission_id, code, "run rejected");
        }
        VerificationStatusV2::FailedInfrastructure(failure) => {
            return Err(VerifierReportedInfrastructureFailure {
                code: failure.code,
                private_detail_code: failure.private_detail_code.clone(),
            }
            .into());
        }
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
#[error("verifier reported an infrastructure failure {code:?}: {private_detail_code:?}")]
struct VerifierReportedInfrastructureFailure {
    code: VerificationInfrastructureFailureCodeV1,
    private_detail_code: Option<String>,
}

/// Bounded operator diagnostics persisted with the failed job.
fn private_failure_detail(error: &anyhow::Error) -> String {
    let detail =
        if let Some(failure) = error.downcast_ref::<VerifierReportedInfrastructureFailure>() {
            format!(
                "verifier_infrastructure:{:?}:{}",
                failure.code,
                failure.private_detail_code.as_deref().unwrap_or("none")
            )
        } else if let Some(process) = error.downcast_ref::<ProcessError>() {
            format!("{}:{process}", process.safe_log_code())
        } else {
            format!("worker_failure:{error:#}")
        };
    bounded_private_detail(&detail)
}

fn safe_worker_error_code(error: &anyhow::Error) -> &'static str {
    if error
        .downcast_ref::<VerifierReportedInfrastructureFailure>()
        .is_some()
    {
        "verifier_reported_infrastructure_failure"
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
    let process_detail = error
        .downcast_ref::<ProcessError>()
        .and_then(|process| match process {
            ProcessError::Exit(_) => Some(bounded_private_detail(&process.to_string())),
            _ => None,
        });
    if exhausted {
        tracing::error!(
            submission_id,
            attempts,
            error_code,
            process_detail = ?process_detail,
            "verification job exhausted its bounded retry policy"
        );
    } else {
        tracing::error!(
            submission_id,
            attempts,
            error_code,
            process_detail = ?process_detail,
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
    async fn owned_operation_keeps_large_operations_out_of_the_callers_future() {
        let payload = [7_u8; 256 * 1024];
        let operation = async move {
            tokio::task::yield_now().await;
            Ok(std::hint::black_box(payload)[0])
        };
        assert!(std::mem::size_of_val(&operation) >= 256 * 1024);
        let owned = run_owned_operation(operation);
        assert!(
            std::mem::size_of_val(&owned) <= 1024,
            "owned operation must not embed the replay operation in its caller"
        );
        assert_eq!(owned.await.unwrap(), 7);
    }

    async fn assert_physical_work_is_drained(mode: &'static str) {
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
        let operation_database = database.clone();
        let mut caller = tokio::spawn(async move {
            run_owned_operation(async move {
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
                            wait_release.recv_timeout(Duration::from_secs(10)).unwrap();
                            std::fs::write(write_path, b"published").unwrap();
                        });
                        if mode == "operation_error" {
                            drop(physical);
                            anyhow::bail!("injected operation error");
                        }
                        physical.await?;
                        Ok(())
                    },
                    move || {
                        let notify_refresh = Arc::clone(&notify_refresh);
                        async move {
                            notify_refresh.notify_one();
                            match mode {
                                "heartbeat_lost" => Ok(false),
                                "heartbeat_error" => anyhow::bail!("injected heartbeat error"),
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
            assert!(
                tokio::time::timeout(Duration::from_millis(100), &mut caller)
                    .await
                    .is_err(),
                "worker owner returned while physical mutation was still blocked"
            );
        }
        assert!(!mutation.exists());
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
                    "heartbeat_error" => "injected heartbeat error",
                    "operation_error" => "injected operation error",
                    _ => unreachable!(),
                };
                assert!(error.contains(expected), "{error}");
            }
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if database
                    .active_maintenance_write_lease_count()
                    .await
                    .unwrap()
                    == 0
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(mutation).unwrap(), b"published");
    }

    #[tokio::test]
    async fn physical_work_is_drained_after_heartbeat_loss() {
        assert_physical_work_is_drained("heartbeat_lost").await;
    }

    #[tokio::test]
    async fn physical_work_is_drained_after_heartbeat_error() {
        assert_physical_work_is_drained("heartbeat_error").await;
    }

    #[tokio::test]
    async fn physical_work_is_drained_after_operation_error() {
        assert_physical_work_is_drained("operation_error").await;
    }

    #[tokio::test]
    async fn physical_work_is_drained_after_caller_cancellation() {
        assert_physical_work_is_drained("caller_cancelled").await;
    }

    #[tokio::test]
    async fn physical_work_is_drained_after_success() {
        assert_physical_work_is_drained("success").await;
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

    #[tokio::test]
    async fn unavailable_write_lease_prevents_worker_startup_from_creating_storage_roots() {
        let directory = tempfile::tempdir().unwrap();
        let mut server = ServerConfig::default();
        server.database_path = directory.path().join("highscores.sqlite3");
        server.replay_directory = directory.path().join("objects/replays");
        let database = Database::migrate(&server).await.unwrap();
        let held = database
            .acquire_maintenance_write_lease(
                robin_highscores::db::MaintenanceWriteClass::Worker,
                "worker-startup-gate-test-holder",
                Duration::from_secs(60),
            )
            .await
            .unwrap();
        let result = initialize_worker_storage(
            &database,
            &server,
            "worker-startup-gate-test",
            Duration::from_secs(60),
        )
        .await;
        assert!(result.is_err());
        assert!(!server.replay_directory.exists());
        assert!(!directory.path().join("objects").exists());
        assert!(
            database
                .release_maintenance_write_lease(&held)
                .await
                .unwrap()
        );
    }

    #[test]
    fn verifier_job_is_built_from_board_worker_content_and_submission() {
        let worker: WorkerConfig =
            toml::from_str(include_str!("../../ops/production/worker.toml")).unwrap();
        let server: ServerConfig =
            toml::from_str(include_str!("../../ops/production/server.toml")).unwrap();
        let board = server
            .board(&OpaqueId::new("full-standard-hard").unwrap())
            .unwrap();
        let job = robin_highscores::model::WorkerJob {
            submission_id: "018f0000-0000-7000-8000-000000000001".to_owned(),
            board_id: "full-standard-hard".to_owned(),
            mission_id: "H01_Lin_VL".to_owned(),
            replay_sha256: [5; 32],
            replay_bytes: 99,
            replay_schema_version: robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            attempts: 1,
        };
        let verifier_job = verifier_job(&worker, board, &job).unwrap();
        assert_eq!(
            verifier_job.edition,
            robin_run_protocol::OfficialContentEditionV1::Full
        );
        assert_eq!(verifier_job.resource_locale_root, "2047");
        assert_eq!(verifier_job.simulation_policy, board.simulation_policy);
        assert!(verifier_job.allow_state_load);
        assert_eq!(verifier_job.job_id.as_str(), job.submission_id);
        assert!(serde_json::to_vec(&verifier_job).unwrap().len() < 64 * 1024);
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
    fn verifier_exit_status_and_stderr_are_logged_for_retry_and_exhaustion() {
        let error = anyhow::Error::new(ProcessError::Exit(
            "exit status: 1; stderr: sandbox startup failed".to_owned(),
        ));
        for exhausted in [false, true] {
            log_verification_job_failure("submission-exit-test", 3, &error, exhausted);
        }
        assert!(logs_contain("exit status: 1"));
        assert!(logs_contain("sandbox startup failed"));
        assert!(logs_contain("scheduling retry"));
        assert!(logs_contain("exhausted its bounded retry policy"));
    }

    #[tracing_test::traced_test]
    #[test]
    fn verifier_private_detail_is_not_a_log_field() {
        let private_detail = "path=/srv/private/fullgame/content PrivateVerifierEvidence";
        let error = anyhow::Error::new(VerifierReportedInfrastructureFailure {
            code: VerificationInfrastructureFailureCodeV1::WorkerInternalFailure,
            private_detail_code: Some(private_detail.to_owned()),
        });
        assert_eq!(
            safe_worker_error_code(&error),
            "verifier_reported_infrastructure_failure"
        );
        assert!(private_failure_detail(&error).contains(private_detail));
        log_verification_job_failure("submission-safe-id", 3, &error, false);
        assert!(logs_contain("verifier_reported_infrastructure_failure"));
        assert!(!logs_contain("PrivateVerifierEvidence"));
    }
}
