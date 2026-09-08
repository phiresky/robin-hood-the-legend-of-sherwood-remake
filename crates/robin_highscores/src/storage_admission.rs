//! Continuous, fail-closed capacity admission for ranked artifacts.
//!
//! The HTTP concurrency limit bounds how many uploads may write at once.  We
//! reserve the exact declared bytes for the current upload and the configured
//! maxima for every other slot, plus bounded multipart/SQLite and verifier
//! output headroom.  Demands are combined when paths share a filesystem so a
//! replay and campaign store on one volume cannot each spend the same free
//! bytes.

use crate::{CampaignStore, Database, ReplayStore, ServerConfig};
use std::collections::BTreeMap;

pub const MINIMUM_STORAGE_RESERVE_BYTES: u64 = 1024 * 1024 * 1024;
pub const MULTIPART_FRAMING_HEADROOM_BYTES: u64 = 1024 * 1024;
pub const SQLITE_WAL_HEADROOM_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum StorageAdmissionError {
    #[error("declared {kind} artifact length {declared} exceeds configured maximum {maximum}")]
    ArtifactTooLarge {
        kind: &'static str,
        declared: u64,
        maximum: u64,
    },
    #[error("storage capacity calculation overflow")]
    Overflow,
    #[error("could not inspect storage capacity")]
    Io(#[source] std::io::Error),
    #[error("{kind} storage has {available} free bytes but admission requires {required} bytes")]
    Insufficient {
        kind: &'static str,
        available: u64,
        required: u64,
    },
}

impl StorageAdmissionError {
    pub const fn safe_log_code(&self) -> &'static str {
        match self {
            Self::ArtifactTooLarge { .. } => "storage_artifact_too_large",
            Self::Overflow => "storage_capacity_overflow",
            Self::Io(_) => "storage_capacity_io",
            Self::Insufficient { .. } => "storage_capacity_insufficient",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Demand {
    kind: &'static str,
    volume: StorageVolume,
    bytes: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct StorageVolume {
    kind: &'static str,
    identity: FilesystemIdentity,
    available: u64,
}

impl StorageVolume {
    pub(crate) const fn available_bytes(self) -> u64 {
        self.available
    }

    /// Inspect the filesystem through the already-pinned capability rather
    /// than resolving the mutable configured path again. This preserves the
    /// stores' ancestor-swap guarantees during every admission check.
    #[cfg(unix)]
    pub(crate) fn from_pinned_dir(
        kind: &'static str,
        directory: &cap_std::fs::Dir,
    ) -> Result<Self, std::io::Error> {
        let stat = rustix::fs::fstatvfs(directory).map_err(std::io::Error::from)?;
        let metadata = rustix::fs::fstat(directory).map_err(std::io::Error::from)?;
        let available = stat
            .f_frsize
            .checked_mul(stat.f_bavail)
            .ok_or_else(|| std::io::Error::other("filesystem free-space value overflow"))?;
        Ok(Self {
            kind,
            identity: FilesystemIdentity(metadata.st_dev),
            available,
        })
    }

    #[cfg(not(unix))]
    pub(crate) fn from_pinned_dir(
        _kind: &'static str,
        _directory: &cap_std::fs::Dir,
    ) -> Result<Self, std::io::Error> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "continuous storage admission requires descriptor-based filesystem capacity",
        ))
    }
}

/// Readiness and offer issuance reserve enough room for a complete maximum
/// upload in every HTTP slot and one maximum verifier campaign output.
pub fn ensure_offer_capacity(
    config: &ServerConfig,
    database: &Database,
    replay_store: &ReplayStore,
    campaign_store: &CampaignStore,
) -> Result<(), StorageAdmissionError> {
    ensure_capacity(
        config,
        database,
        replay_store,
        campaign_store,
        config.max_replay_bytes,
        config.max_campaign_bytes,
    )
}

/// Recheck capacity immediately before the database may consume the one-use
/// challenge.  The current slot uses authenticated exact lengths; only the
/// other concurrently active slots use configured maxima.
pub fn ensure_upload_capacity(
    config: &ServerConfig,
    database: &Database,
    replay_store: &ReplayStore,
    campaign_store: &CampaignStore,
    replay_bytes: u64,
    campaign_bytes: u64,
) -> Result<(), StorageAdmissionError> {
    validate_artifact_length("replay", replay_bytes, config.max_replay_bytes)?;
    validate_artifact_length("campaign", campaign_bytes, config.max_campaign_bytes)?;
    ensure_capacity(
        config,
        database,
        replay_store,
        campaign_store,
        replay_bytes,
        campaign_bytes,
    )
}

/// A worker may lease only when a maximum-sized final campaign can be written
/// while every bounded API upload slot is active.
pub fn ensure_worker_lease_capacity(
    config: &ServerConfig,
    database: &Database,
    replay_store: &ReplayStore,
    campaign_store: &CampaignStore,
) -> Result<(), StorageAdmissionError> {
    ensure_capacity(
        config,
        database,
        replay_store,
        campaign_store,
        config.max_replay_bytes,
        config.max_campaign_bytes,
    )
}

/// Recheck the verifier's exact authenticated output before creating its
/// content-addressed object.  This is deliberately after verification and
/// before the first campaign-store write.
pub fn ensure_worker_final_campaign_capacity(
    config: &ServerConfig,
    database: &Database,
    replay_store: &ReplayStore,
    campaign_store: &CampaignStore,
    final_campaign_bytes: u64,
) -> Result<(), StorageAdmissionError> {
    validate_artifact_length(
        "final_campaign",
        final_campaign_bytes,
        config.max_campaign_bytes,
    )?;
    ensure_capacity(
        config,
        database,
        replay_store,
        campaign_store,
        config.max_replay_bytes,
        final_campaign_bytes,
    )
}

fn validate_artifact_length(
    kind: &'static str,
    declared: u64,
    maximum: u64,
) -> Result<(), StorageAdmissionError> {
    if declared > maximum {
        return Err(StorageAdmissionError::ArtifactTooLarge {
            kind,
            declared,
            maximum,
        });
    }
    Ok(())
}

fn ensure_capacity(
    config: &ServerConfig,
    database: &Database,
    replay_store: &ReplayStore,
    campaign_store: &CampaignStore,
    current_replay_bytes: u64,
    current_campaign_bytes: u64,
) -> Result<(), StorageAdmissionError> {
    let plan = capacity_demand_bytes(config, current_replay_bytes, current_campaign_bytes)?;
    let replay_volume = replay_store
        .storage_volume()
        .map_err(StorageAdmissionError::Io)?;
    let campaign_volume = campaign_store
        .storage_volume()
        .map_err(StorageAdmissionError::Io)?;
    let database_volume = database
        .storage_volume()
        .map_err(StorageAdmissionError::Io)?;
    ensure_demands(
        [
            Demand {
                kind: replay_volume.kind,
                volume: replay_volume,
                bytes: plan.replay,
            },
            Demand {
                kind: campaign_volume.kind,
                volume: campaign_volume,
                bytes: plan.campaign,
            },
            Demand {
                kind: database_volume.kind,
                volume: database_volume,
                bytes: plan.database,
            },
        ],
        config.minimum_storage_free_bytes,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityDemandBytes {
    pub replay: u64,
    pub campaign: u64,
    pub database: u64,
}

/// One API maintenance batch, one verifier worker, and one explicitly
/// serialized administrator mutation may coexist with bounded HTTP writers.
pub const MAXIMUM_AUXILIARY_DATABASE_WRITERS: u64 = 3;

/// Worst-case object/database growth already admitted when a maintenance
/// operation closes the admission gate. Backup capacity uses this exact same
/// primitive so its in-flight allowance cannot drift from HTTP/worker policy.
pub fn maximum_capacity_demand_bytes(
    config: &ServerConfig,
) -> Result<CapacityDemandBytes, StorageAdmissionError> {
    capacity_demand_bytes(config, config.max_replay_bytes, config.max_campaign_bytes)
}

fn capacity_demand_bytes(
    config: &ServerConfig,
    current_replay_bytes: u64,
    current_campaign_bytes: u64,
) -> Result<CapacityDemandBytes, StorageAdmissionError> {
    let slots = u64::try_from(config.max_concurrent_uploads)
        .map_err(|_| StorageAdmissionError::Overflow)?;
    let other_slots = slots
        .checked_sub(1)
        .ok_or(StorageAdmissionError::Overflow)?;
    let replay_demand = current_replay_bytes
        .checked_add(
            other_slots
                .checked_mul(config.max_replay_bytes)
                .ok_or(StorageAdmissionError::Overflow)?,
        )
        .ok_or(StorageAdmissionError::Overflow)?;
    let campaign_upload_demand = current_campaign_bytes
        .checked_add(
            other_slots
                .checked_mul(config.max_campaign_bytes)
                .ok_or(StorageAdmissionError::Overflow)?,
        )
        .ok_or(StorageAdmissionError::Overflow)?;
    let campaign_demand = campaign_upload_demand
        .checked_add(config.max_campaign_bytes)
        .ok_or(StorageAdmissionError::Overflow)?;
    let per_request_database = u64::try_from(config.max_metadata_bytes)
        .map_err(|_| StorageAdmissionError::Overflow)?
        .checked_add(MULTIPART_FRAMING_HEADROOM_BYTES)
        .ok_or(StorageAdmissionError::Overflow)?;
    let upload_database_demand = slots
        .checked_mul(per_request_database)
        .ok_or(StorageAdmissionError::Overflow)?;
    let metadata_writers = u64::try_from(config.max_concurrent_requests)
        .map_err(|_| StorageAdmissionError::Overflow)?
        .checked_add(MAXIMUM_AUXILIARY_DATABASE_WRITERS)
        .ok_or(StorageAdmissionError::Overflow)?;
    let metadata_database_demand = metadata_writers
        .checked_mul(
            u64::try_from(config.max_metadata_bytes)
                .map_err(|_| StorageAdmissionError::Overflow)?,
        )
        .ok_or(StorageAdmissionError::Overflow)?;
    let database_demand = upload_database_demand
        .checked_add(metadata_database_demand)
        .and_then(|bytes| bytes.checked_add(SQLITE_WAL_HEADROOM_BYTES))
        .ok_or(StorageAdmissionError::Overflow)?;
    Ok(CapacityDemandBytes {
        replay: replay_demand,
        campaign: campaign_demand,
        database: database_demand,
    })
}

#[derive(Debug)]
struct FilesystemDemand {
    kind: &'static str,
    available: u64,
    bytes: u64,
}

fn ensure_demands<const N: usize>(
    demands: [Demand; N],
    reserve: u64,
) -> Result<(), StorageAdmissionError> {
    let mut filesystems = BTreeMap::<FilesystemIdentity, FilesystemDemand>::new();
    for demand in demands {
        match filesystems.get_mut(&demand.volume.identity) {
            Some(existing) => {
                existing.bytes = existing
                    .bytes
                    .checked_add(demand.bytes)
                    .ok_or(StorageAdmissionError::Overflow)?;
                existing.available = existing.available.min(demand.volume.available);
            }
            None => {
                filesystems.insert(
                    demand.volume.identity,
                    FilesystemDemand {
                        kind: demand.kind,
                        available: demand.volume.available,
                        bytes: demand.bytes,
                    },
                );
            }
        }
    }
    for demand in filesystems.into_values() {
        let required = reserve
            .checked_add(demand.bytes)
            .ok_or(StorageAdmissionError::Overflow)?;
        if demand.available < required {
            return Err(StorageAdmissionError::Insufficient {
                kind: demand.kind,
                available: demand.available,
                required,
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FilesystemIdentity(u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_boundary_includes_shared_filesystem_and_concurrency_headroom() {
        let volume = StorageVolume {
            kind: "shared",
            identity: FilesystemIdentity(7),
            available: 100,
        };
        let demands = [
            Demand {
                kind: "replay",
                volume,
                bytes: 11,
            },
            Demand {
                kind: "campaign",
                volume,
                bytes: 13,
            },
            Demand {
                kind: "database",
                volume,
                bytes: 17,
            },
        ];
        assert!(ensure_demands(demands, 59).is_ok());
        assert!(matches!(
            ensure_demands(demands, 60),
            Err(StorageAdmissionError::Insufficient { .. })
        ));
    }

    #[test]
    fn concurrent_plan_fails_closed_on_arithmetic_overflow() {
        let mut config = ServerConfig::default();
        config.max_replay_bytes = u64::MAX;
        assert!(matches!(
            capacity_demand_bytes(&config, config.max_replay_bytes, config.max_campaign_bytes),
            Err(StorageAdmissionError::Overflow)
        ));
    }

    #[test]
    fn exact_current_lengths_and_other_concurrent_slots_share_one_capacity_budget() {
        let mut config = ServerConfig::default();
        config.max_replay_bytes = 11;
        config.max_campaign_bytes = 13;
        config.max_metadata_bytes = 17;
        config.max_concurrent_uploads = 4;

        assert_eq!(
            capacity_demand_bytes(&config, 5, 7).unwrap(),
            CapacityDemandBytes {
                replay: 5 + 3 * 11,
                campaign: 7 + 3 * 13 + 13,
                database: 4 * (17 + MULTIPART_FRAMING_HEADROOM_BYTES)
                    + (u64::try_from(config.max_concurrent_requests).unwrap()
                        + MAXIMUM_AUXILIARY_DATABASE_WRITERS)
                        * 17
                    + SQLITE_WAL_HEADROOM_BYTES,
            }
        );
    }

    #[test]
    fn exact_lengths_are_validated_before_capacity_is_consulted() {
        assert!(matches!(
            validate_artifact_length("replay", 12, 11),
            Err(StorageAdmissionError::ArtifactTooLarge { kind: "replay", .. })
        ));
        assert!(matches!(
            validate_artifact_length("final_campaign", 14, 13),
            Err(StorageAdmissionError::ArtifactTooLarge {
                kind: "final_campaign",
                ..
            })
        ));
    }
}
