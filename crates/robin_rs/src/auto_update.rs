//! Background Velopack updates for installed desktop builds.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use velopack::{UpdateCheck, UpdateManager, VelopackAsset, sources::GithubSource};

const GITHUB_REPOSITORY_URL: &str =
    "https://github.com/phiresky/robin-hood-the-legend-of-sherwood-remake";

/// A downloaded asset together with the manager that owns its package cache.
pub type DownloadedUpdate = (UpdateManager, VelopackAsset);

/// Start checking the public GitHub Releases feed in the background.
///
/// Returns `None` for unpackaged developer/standalone builds because Velopack
/// cannot locate an installed application manifest in those environments.
pub fn start_github_auto_update() -> Option<Receiver<DownloadedUpdate>> {
    let stable_source = GithubSource::new(GITHUB_REPOSITORY_URL, None, false);
    let stable_manager = match UpdateManager::new(stable_source, None, None) {
        Ok(manager) => manager,
        Err(error) => {
            tracing::debug!("Velopack auto-update is unavailable for this build: {error}");
            return None;
        }
    };

    let current_version = stable_manager.get_current_version();
    let include_prereleases = should_include_prereleases(&current_version);
    let manager = if include_prereleases {
        let nightly_source = GithubSource::new(GITHUB_REPOSITORY_URL, None, true);
        match UpdateManager::new(nightly_source, None, None) {
            Ok(manager) => manager,
            Err(error) => {
                tracing::warn!("Could not initialize the Velopack nightly update source: {error}");
                return None;
            }
        }
    } else {
        stable_manager
    };

    if let Some(pending) = manager.get_update_pending_restart() {
        tracing::info!(
            version = %pending.Version,
            "Applying the previously downloaded update before startup"
        );
        if let Err(error) = manager.apply_updates_and_restart(pending) {
            tracing::warn!("Could not apply the pending Velopack update: {error}");
        }
    }

    let (sender, receiver) = mpsc::channel();
    let thread_manager = manager.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("github-auto-update".to_owned())
        .spawn(move || {
            let result = (|| {
                tracing::debug!(
                    current_version = %thread_manager.get_current_version_as_string(),
                    include_prereleases,
                    "Checking GitHub Releases for updates"
                );
                let update = match thread_manager.check_for_updates()? {
                    UpdateCheck::UpdateAvailable(update) => update,
                    UpdateCheck::NoUpdateAvailable => {
                        tracing::debug!("The installed build is up to date");
                        return Ok(());
                    }
                    UpdateCheck::RemoteIsEmpty => {
                        tracing::warn!("The GitHub Velopack release feed contains no updates");
                        return Ok(());
                    }
                };

                tracing::info!(
                    version = %update.TargetFullRelease.Version,
                    "Downloading a Velopack update from GitHub Releases"
                );
                thread_manager.download_updates(&update, None)?;
                let asset = update.TargetFullRelease.clone();
                sender.send((thread_manager, asset)).map_err(|_| {
                    velopack::Error::Other(
                        "the game exited before the downloaded update could be queued".to_owned(),
                    )
                })?;
                tracing::info!("The update will be installed after the game exits");
                Ok::<(), velopack::Error>(())
            })();

            if let Err(error) = result {
                tracing::warn!("GitHub auto-update failed: {error}");
            }
        })
    {
        tracing::warn!("Could not start the GitHub auto-update thread: {error}");
        return None;
    }

    Some(receiver)
}

/// Apply a completed background download after the game has shut down cleanly.
pub fn apply_downloaded_update(receiver: Option<Receiver<DownloadedUpdate>>) {
    let Some(receiver) = receiver else {
        return;
    };
    match receiver.try_recv() {
        Ok((manager, asset)) => {
            tracing::info!(version = %asset.Version, "Installing downloaded update");
            if let Err(error) =
                manager.wait_exit_then_apply_updates(asset, true, false, Vec::<String>::new())
            {
                tracing::warn!("Could not install the downloaded Velopack update: {error}");
            }
        }
        Err(TryRecvError::Empty) => {
            tracing::debug!("No completed update download is waiting at shutdown");
        }
        Err(TryRecvError::Disconnected) => {
            tracing::debug!("The auto-update worker finished without a queued update");
        }
    }
}

fn should_include_prereleases(version: &semver::Version) -> bool {
    !version.pre.is_empty()
}

#[cfg(test)]
mod tests {
    use super::should_include_prereleases;

    #[test]
    fn stable_builds_ignore_prereleases() {
        let version = semver::Version::parse("1.2.3").unwrap();
        assert!(!should_include_prereleases(&version));
    }

    #[test]
    fn nightly_builds_follow_prereleases() {
        let version = semver::Version::parse("1.2.3-nightly.42+abcdef").unwrap();
        assert!(should_include_prereleases(&version));
    }
}
