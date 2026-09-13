//! Native profile archive: `profiles.json` inside the selected save directory;
//! deleted profiles' save directories are renamed aside, never removed.

use super::{PlayerProfileStore, finish_loading_archive};
use robin_engine::player_profile::PlayerProfileManager;

impl PlayerProfileStore {
    pub fn for_directory(directory: &str) -> Self {
        Self::Native {
            directory: directory.into(),
        }
    }

    pub(crate) fn directory(&self) -> std::io::Result<&str> {
        match self {
            Self::Native { directory } => directory.to_str().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "player-profile directory cannot be represented as UTF-8 archive metadata",
                )
            }),
            Self::Unavailable { reason } => Err(std::io::Error::other(reason.clone())),
        }
    }

    /// `Ok(None)` only when no archive has been published yet.
    pub(super) fn load_existing(
        &self,
        directory: &str,
    ) -> std::io::Result<Option<PlayerProfileManager>> {
        match self {
            Self::Native { directory: root } => {
                match std::fs::File::open(root.join("profiles.json")) {
                    Ok(file) => Ok(Some(decode_native_archive(
                        std::io::BufReader::new(file),
                        directory,
                    )?)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(error) => Err(error),
                }
            }
            Self::Unavailable { .. } => unreachable!("directory checked authority"),
        }
    }

    pub(super) fn publish(&self, manager: &PlayerProfileManager) -> std::io::Result<()> {
        match self {
            Self::Native { directory } => {
                manager
                    .validate_archive()
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
                crate::desktop_persistence::write_json(&directory.join("profiles.json"), manager)
            }
            Self::Unavailable { .. } => unreachable!("directory checked authority"),
        }
    }

    pub(super) fn move_profile_saves(&self, profile_id: u32, restore: bool) -> std::io::Result<()> {
        self.directory()?;
        match self {
            Self::Native { directory } => {
                let name = robin_engine::player_profile::profile_save_subdirectory(profile_id);
                let live = directory.join(&name);
                let deleted = directory.join(format!(".deleted-{name}"));
                let (source, destination) = if restore {
                    (deleted, live)
                } else {
                    (live, deleted)
                };
                match std::fs::symlink_metadata(&source) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                    Err(error) => return Err(error),
                    Ok(metadata) if !metadata.is_dir() => {
                        return Err(std::io::Error::other(format!(
                            "profile saves are not a directory: {}",
                            source.display()
                        )));
                    }
                    Ok(_) => {}
                }
                match std::fs::symlink_metadata(&destination) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                    Ok(_) => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::AlreadyExists,
                            format!(
                                "refusing to replace profile saves at {}",
                                destination.display()
                            ),
                        ));
                    }
                }
                std::fs::rename(&source, &destination)?;
                #[cfg(unix)]
                std::fs::File::open(directory)?.sync_all()?;
                tracing::info!(
                    "Moved profile saves {} → {}",
                    source.display(),
                    destination.display()
                );
                Ok(())
            }
            Self::Unavailable { .. } => unreachable!("directory checked authority"),
        }
    }
}

pub(super) fn decode_native_archive(
    input: impl std::io::Read,
    directory: &str,
) -> std::io::Result<PlayerProfileManager> {
    let manager = serde_json::from_reader(input).map_err(|error| {
        std::io::Error::new(
            error
                .io_error_kind()
                .unwrap_or(std::io::ErrorKind::InvalidData),
            error,
        )
    })?;
    finish_loading_archive(manager, directory)
}
