#![forbid(unsafe_code)]

#[cfg(not(target_os = "linux"))]
compile_error!("robin-highscores-server is Linux-only");

use clap::Parser;
use robin_highscores::{
    Database, ReplayStore, ServerConfig, garbage_collect_replays, reconcile_replay_inventory,
    safe_error_code,
    web::{AppState, ChallengeRateLimiter, router},
};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

const MAINTENANCE_WRITE_LEASE_TTL: Duration = Duration::from_secs(5 * 60);

async fn run_with_maintenance_write_lease<T, F>(
    database: &Database,
    owner: &str,
    operation: F,
) -> anyhow::Result<T>
where
    F: Future<Output = anyhow::Result<T>>,
{
    let token = database
        .acquire_maintenance_write_lease(
            robin_highscores::db::MaintenanceWriteClass::ApiMaintenance,
            owner,
            MAINTENANCE_WRITE_LEASE_TTL,
        )
        .await?;
    tokio::pin!(operation);
    let result = loop {
        tokio::select! {
            result = &mut operation => break result,
            () = tokio::time::sleep(MAINTENANCE_WRITE_LEASE_TTL / 3) => {
                match database
                    .refresh_maintenance_write_lease(&token, MAINTENANCE_WRITE_LEASE_TTL)
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => break Err(anyhow::anyhow!(
                        "API maintenance-write lease expired or was replaced"
                    )),
                    Err(error) => break Err(error.into()),
                }
            }
        }
    };
    let release = database.release_maintenance_write_lease(&token).await;
    match (result, release) {
        (Ok(value), Ok(true)) => Ok(value),
        (Ok(_), Ok(false)) => anyhow::bail!("API maintenance-write lease disappeared"),
        (Ok(_), Err(error)) => Err(error.into()),
        (Err(operation), Ok(true)) => Err(operation),
        (Err(operation), Ok(false)) => {
            Err(operation.context("API maintenance failed and its write lease disappeared"))
        }
        (Err(operation), Err(release)) => Err(operation.context(format!(
            "API maintenance failed and releasing its write lease also failed: {release}"
        ))),
    }
}

async fn perform_storage_maintenance(
    database: &Database,
    replay_store: &ReplayStore,
    config: &ServerConfig,
) -> anyhow::Result<()> {
    replay_store.readiness_check().await?;
    let recovered_uploads = database.recover_upload_reservations().await?;
    tracing::info!(
        recovered_uploads,
        "recovered expired submission upload reservations"
    );
    let reconciled = reconcile_replay_inventory(database, replay_store).await?;
    tracing::debug!(reconciled, "reconciled replay inventory");
    let collected = garbage_collect_replays(database, replay_store, config, 1_000).await?;
    tracing::info!(collected, "completed replay garbage collection");
    Ok(())
}

async fn initialize_api_storage(
    database: &Database,
    config: &ServerConfig,
) -> anyhow::Result<ReplayStore> {
    run_with_maintenance_write_lease(database, "robin-highscores-api-startup", async {
        if let Some(parent) = config.replay_directory.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let replay_store =
            ReplayStore::create(config.replay_directory.clone(), config.max_replay_bytes).await?;
        perform_storage_maintenance(database, &replay_store, config).await?;
        Ok(replay_store)
    })
    .await
}

#[derive(Debug, Parser)]
#[command(about = "Verified Robin Hood replay and leaderboard API")]
struct Arguments {
    #[arg(
        long,
        env = "ROBIN_HIGHSCORES_CONFIG",
        default_value = "highscores-server.toml"
    )]
    config: PathBuf,
}

use robin_highscores::service::{ServiceNotifier, StartupStatusNotifier, SystemdNotifier};

struct ApiRuntime {
    application: axum::Router,
    listener: tokio::net::TcpListener,
    gc_task: tokio::task::JoinHandle<()>,
    database: Database,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    if let Err(error) = run().await {
        tracing::error!(
            error_code = safe_error_code(&error),
            "highscore API terminated"
        );
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let arguments = Arguments::parse();
    let notifier = SystemdNotifier::Api;
    let runtime = initialize_api(&arguments.config, &notifier).await?;
    if let Err(error) = notifier.ready() {
        return finish_api_runtime(runtime, Err(error)).await;
    }
    serve_api(runtime).await
}

#[cfg(test)]
async fn run_api_lifecycle<N, T, Startup, Serve, ServeFuture>(
    notifier: &N,
    startup: Startup,
    serve: Serve,
) -> anyhow::Result<()>
where
    N: ServiceNotifier,
    Startup: Future<Output = anyhow::Result<T>>,
    Serve: FnOnce(T) -> ServeFuture,
    ServeFuture: Future<Output = anyhow::Result<()>>,
{
    let runtime = startup.await?;
    notifier.ready()?;
    serve(runtime).await
}

async fn initialize_api(
    config_path: &std::path::Path,
    notifier: &impl StartupStatusNotifier,
) -> anyhow::Result<ApiRuntime> {
    notifier.status("Loading server configuration and boards")?;
    let config = ServerConfig::load(config_path)?;
    notifier.status("Opening database and recovering upload reservations")?;
    let database = Database::connect(&config).await?;
    let result = initialize_connected_api(config, database.clone(), notifier).await;
    match result {
        Ok(runtime) => Ok(runtime),
        Err(error) => {
            database.close().await;
            Err(error)
        }
    }
}

async fn initialize_connected_api(
    config: ServerConfig,
    database: Database,
    notifier: &impl StartupStatusNotifier,
) -> anyhow::Result<ApiRuntime> {
    let cursor_hmac_key = config.load_cursor_key()?;
    notifier
        .status("Opening the replay store and reconciling its inventory under maintenance lease")?;
    let replay_store = initialize_api_storage(&database, &config).await?;

    notifier.status("Building API routes and binding the listener")?;
    let state = AppState {
        config: config.clone(),
        database: database.clone(),
        replay_store: replay_store.clone(),
        cursor_hmac_key,
        challenge_rate_limiter: ChallengeRateLimiter::new(
            config.challenge_requests_per_minute_per_ip,
        ),
    };
    let application = router(state)?;
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(bind = %config.bind, "highscore API startup validation complete");

    let gc_config = Arc::new(config);
    let gc_database = database.clone();
    let gc_store = replay_store.clone();
    let gc_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60 * 60));
        interval.tick().await;
        loop {
            interval.tick().await;
            let operation_database = gc_database.clone();
            let operation_store = gc_store.clone();
            let operation_config = Arc::clone(&gc_config);
            // Run in an owned task so aborting the scheduler on shutdown does
            // not cancel a purge between its filesystem and database steps.
            let operation = tokio::spawn(async move {
                run_with_maintenance_write_lease(
                    &operation_database,
                    "robin-highscores-api-hourly-maintenance",
                    async {
                        perform_storage_maintenance(
                            &operation_database,
                            &operation_store,
                            &operation_config,
                        )
                        .await
                    },
                )
                .await
            });
            let result = operation
                .await
                .map_err(|error| anyhow::anyhow!(error).context("API maintenance task failed"))
                .and_then(|result| result);
            if let Err(error) = result {
                tracing::warn!(
                    error_code = safe_error_code(&error),
                    "scheduled state maintenance could not acquire its write lease or failed"
                );
            }
        }
    });

    Ok(ApiRuntime {
        application,
        listener,
        gc_task,
        database,
    })
}

async fn serve_api(runtime: ApiRuntime) -> anyhow::Result<()> {
    let ApiRuntime {
        application,
        listener,
        gc_task,
        database,
    } = runtime;
    let result = axum::serve(
        listener,
        application.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .map_err(anyhow::Error::from);
    finish_api_components(database, gc_task, result).await
}

async fn finish_api_runtime(runtime: ApiRuntime, result: anyhow::Result<()>) -> anyhow::Result<()> {
    let ApiRuntime {
        application: _,
        listener: _,
        gc_task,
        database,
    } = runtime;
    finish_api_components(database, gc_task, result).await
}

async fn finish_api_components(
    database: Database,
    gc_task: tokio::task::JoinHandle<()>,
    result: anyhow::Result<()>,
) -> anyhow::Result<()> {
    gc_task.abort();
    let _ = gc_task.await;
    database.close().await;
    result
}

async fn shutdown_signal() {
    robin_highscores::service::wait_for_shutdown_signal()
        .await
        .expect("receive service shutdown signal");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn unavailable_write_lease_prevents_api_startup_from_creating_storage_roots() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("highscores.sqlite3");
        config.replay_directory = directory.path().join("objects/replays");
        let database = Database::migrate(&config).await.unwrap();
        let held = database
            .acquire_maintenance_write_lease(
                robin_highscores::db::MaintenanceWriteClass::ApiMaintenance,
                "startup-gate-test-holder",
                Duration::from_secs(60),
            )
            .await
            .unwrap();

        let result = initialize_api_storage(&database, &config).await;

        assert!(result.is_err());
        assert!(!config.replay_directory.exists());
        assert!(!directory.path().join("objects").exists());
        assert!(
            database
                .release_maintenance_write_lease(&held)
                .await
                .unwrap()
        );
    }

    struct FailingReadyNotifier;

    impl StartupStatusNotifier for FailingReadyNotifier {
        fn status(&self, _status: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    impl ServiceNotifier for FailingReadyNotifier {
        fn ready(&self) -> anyhow::Result<()> {
            anyhow::bail!("test readiness notification failure")
        }
    }

    #[tokio::test]
    async fn readiness_notification_failure_is_fatal_before_serving() {
        let served = Arc::new(AtomicBool::new(false));
        let served_by_callback = Arc::clone(&served);
        let result = run_api_lifecycle(
            &FailingReadyNotifier,
            async { Ok(()) },
            move |()| async move {
                served_by_callback.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .await;

        assert!(result.is_err());
        assert!(!served.load(Ordering::SeqCst));
    }

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
                .any(|message| message.contains("Process test: serving API"))
        );
    }

    #[test]
    fn systemd_process_success_sends_ready_before_serving() {
        let (output, messages) = run_systemd_notifier_child("success");

        assert!(
            output.status.success(),
            "notifier child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_ready_precedes_serving(&messages);
    }

    #[test]
    fn systemd_process_delayed_startup_does_not_send_ready_early() {
        let NotifierChild {
            child,
            socket,
            _temporary,
        } = spawn_systemd_notifier_child("delayed");
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let first = receive_notification(&socket).expect("child sent no startup status");
        assert!(first.contains("validating startup"));

        socket
            .set_read_timeout(Some(Duration::from_millis(150)))
            .unwrap();
        assert!(
            receive_notification(&socket).is_none(),
            "delayed startup announced readiness before validation completed"
        );

        let output = child.wait_with_output().unwrap();
        let mut messages = vec![first];
        messages.extend(drain_notifications(&socket));
        assert!(
            output.status.success(),
            "delayed notifier child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_ready_precedes_serving(&messages);
    }

    fn assert_ready_precedes_serving(messages: &[String]) {
        let ready = messages
            .iter()
            .position(|message| message.contains("READY=1"))
            .expect("child did not send READY=1");
        let serving = messages
            .iter()
            .position(|message| message.contains("Process test: serving API"))
            .expect("child did not begin serving");
        assert!(ready < serving, "READY=1 must precede serving");
    }

    struct NotifierChild {
        child: std::process::Child,
        socket: std::os::unix::net::UnixDatagram,
        _temporary: tempfile::TempDir,
    }

    fn spawn_systemd_notifier_child(mode: &str) -> NotifierChild {
        use std::os::unix::net::UnixDatagram;
        use std::process::Stdio;

        let temporary = tempfile::tempdir().unwrap();
        let socket_path = temporary.path().join("notify.socket");
        let socket = UnixDatagram::bind(&socket_path).unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("tests::systemd_notifier_process_child")
            .arg("--ignored")
            .arg("--nocapture")
            .env("ROBIN_HIGHSCORES_API_NOTIFY_TEST_MODE", mode)
            .env("NOTIFY_SOCKET", &socket_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        NotifierChild {
            child,
            socket,
            _temporary: temporary,
        }
    }

    fn run_systemd_notifier_child(mode: &str) -> (std::process::Output, Vec<String>) {
        let NotifierChild {
            child,
            socket,
            _temporary,
        } = spawn_systemd_notifier_child(mode);
        let output = child.wait_with_output().unwrap();
        (output, drain_notifications(&socket))
    }

    fn drain_notifications(socket: &std::os::unix::net::UnixDatagram) -> Vec<String> {
        socket
            .set_read_timeout(Some(Duration::from_millis(250)))
            .unwrap();
        let mut messages = Vec::new();
        while let Some(message) = receive_notification(socket) {
            messages.push(message);
        }
        messages
    }

    fn receive_notification(socket: &std::os::unix::net::UnixDatagram) -> Option<String> {
        let mut bytes = [0_u8; 4096];
        match socket.recv(&mut bytes) {
            Ok(length) => Some(String::from_utf8(bytes[..length].to_vec()).unwrap()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                None
            }
            Err(error) => panic!("could not read notifier child datagram: {error}"),
        }
    }

    #[ignore = "process helper invoked by the systemd notification integration tests"]
    #[tokio::test]
    async fn systemd_notifier_process_child() {
        let mode = std::env::var("ROBIN_HIGHSCORES_API_NOTIFY_TEST_MODE").unwrap();
        let notifier = SystemdNotifier::Api;
        let startup_notifier = &notifier;
        let serving_notifier = &notifier;
        run_api_lifecycle(
            &notifier,
            async move {
                startup_notifier.status("Process test: validating startup")?;
                match mode.as_str() {
                    "success" => {}
                    "delayed" => tokio::time::sleep(Duration::from_millis(750)).await,
                    "failure" => anyhow::bail!("test startup validation failure"),
                    _ => anyhow::bail!("unknown notifier process test mode"),
                }
                Ok(())
            },
            move |()| async move {
                serving_notifier.status("Process test: serving API")?;
                Ok(())
            },
        )
        .await
        .unwrap();
    }
}
