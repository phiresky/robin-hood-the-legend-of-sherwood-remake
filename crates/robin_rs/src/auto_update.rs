//! Background Velopack updates for installed desktop builds.

use std::sync::RwLock;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use velopack::{UpdateCheck, UpdateManager, VelopackAsset, sources::GithubSource};

const GITHUB_REPOSITORY_URL: &str =
    "https://github.com/phiresky/robin-hood-the-legend-of-sherwood-remake";

/// Progress of the background update download, for UI display (main menu).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// An update was found on the release feed and is downloading.
    Downloading { version: String },
    /// The update is fully downloaded and will install after the game exits.
    ReadyOnExit { version: String },
}

static UPDATE_STATUS: RwLock<Option<UpdateStatus>> = RwLock::new(None);

/// Current background-update progress, if an update was found.
///
/// Written by the `github-auto-update` worker thread; safe to poll every
/// frame from UI code.
pub fn update_status() -> Option<UpdateStatus> {
    UPDATE_STATUS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn set_update_status(status: Option<UpdateStatus>) {
    *UPDATE_STATUS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = status;
}

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

                let version = update.TargetFullRelease.Version.clone();
                tracing::info!(
                    version = %version,
                    "Downloading a Velopack update from GitHub Releases"
                );
                set_update_status(Some(UpdateStatus::Downloading {
                    version: version.clone(),
                }));
                thread_manager.download_updates(&update, None)?;
                let asset = update.TargetFullRelease.clone();
                sender.send((thread_manager, asset)).map_err(|_| {
                    velopack::Error::Other(
                        "the game exited before the downloaded update could be queued".to_owned(),
                    )
                })?;
                set_update_status(Some(UpdateStatus::ReadyOnExit { version }));
                tracing::info!("The update will be installed after the game exits");
                Ok::<(), velopack::Error>(())
            })();

            if let Err(error) = result {
                // Clear any stale "downloading" line so the menu doesn't
                // advertise an update that will never arrive.
                set_update_status(None);
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
