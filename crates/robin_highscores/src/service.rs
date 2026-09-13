//! Process lifecycle shared by the API and isolated verifier worker.

pub trait StartupStatusNotifier {
    fn status(&self, status: &str) -> anyhow::Result<()>;
}

pub trait ServiceNotifier: StartupStatusNotifier {
    fn ready(&self) -> anyhow::Result<()>;
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub enum SystemdNotifier {
    Api,
    Worker,
}

impl StartupStatusNotifier for SystemdNotifier {
    fn status(&self, status: &str) -> anyhow::Result<()> {
        sd_notify::notify(&[sd_notify::NotifyState::Status(status)])
            .map_err(|error| anyhow::anyhow!("could not update systemd startup status: {error}"))
    }
}

impl ServiceNotifier for SystemdNotifier {
    fn ready(&self) -> anyhow::Result<()> {
        let (status, service) = match self {
            Self::Api => ("Ready; serving leaderboard API", "API"),
            Self::Worker => ("Ready; processing verification queue", "worker"),
        };
        sd_notify::notify(&[
            sd_notify::NotifyState::Status(status),
            sd_notify::NotifyState::Ready,
        ])
        .map_err(|error| {
            anyhow::anyhow!("could not notify systemd of {service} readiness: {error}")
        })
    }
}

pub async fn wait_for_shutdown_signal() -> anyhow::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        signal = terminate.recv() => anyhow::ensure!(signal.is_some(), "termination signal stream closed"),
    }
    Ok(())
}
