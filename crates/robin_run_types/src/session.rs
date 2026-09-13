//! Prepared-input seal, speech timing authority and the anonymous replay seat
//! transcript consumed by the deterministic engine.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    Digest32, OfficialContentEditionV1, OfficialContentSubjectV1, ResourceLocaleRootV1,
    SimulationSeed64, Validate, ValidationError,
};

pub const MAX_REPLAY_SEATS_V1: u16 = 4;
pub const MAX_PARTICIPANT_INSTANCES_V1: u16 = 1_024;
/// Exact media type for the bitcode campaign artifact consumed by the ranked
/// Engine.
pub const RANKED_CAMPAIGN_MEDIA_TYPE_V1: &str = "application/x-robin-campaign+bitcode";

/// Host-authored immutable facts frozen before ranked simulation starts.
///
/// The signed genesis is retained outside the replay. Current canonical replay
/// bytes carry only its canonical digest, the random session/participant IDs,
/// and the keyless seat lifecycle transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpeechTimingAuthorityV1 {
    /// Required English timing metadata in the engine core datadir.
    CoreAudioDurationsV1,
    /// Validated base `Data/Sounds` timing with no locale override.
    BaseInstallation,
    LanguagePack {
        canonical_locale: String,
    },
}

impl SpeechTimingAuthorityV1 {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::BaseInstallation | Self::CoreAudioDurationsV1 => Ok(()),
            Self::LanguagePack { canonical_locale } => {
                crate::validation::text(
                    "ranked_session.speech_timing.canonical_locale",
                    canonical_locale,
                    64,
                )?;
                if canonical_locale.starts_with('-')
                    || canonical_locale.ends_with('-')
                    || canonical_locale
                        .split('-')
                        .any(|part| part.is_empty() || part.len() > 8)
                    || !canonical_locale
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                {
                    return Err(ValidationError::ClaimMismatch {
                        field: "ranked_session.speech_timing.canonical_locale",
                    });
                }
                Ok(())
            }
        }
    }
}

/// Canonical seal over the exact run-specific inputs consumed by the engine.
/// The static content manifest is prepublished; `prepared_inputs_projection`
/// additionally binds the mutable team/inventory/reinforcement closure
/// derived from the starting campaign. Ranked verification recomputes this
/// document before consuming the engine's single-use prepared capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedMissionInputsSealV1 {
    pub schema_version: u32,
    pub prepared_inputs_projection_sha256: Digest32,
    pub content_manifest_sha256: Digest32,
    pub content_edition: OfficialContentEditionV1,
    pub content_subject: OfficialContentSubjectV1,
    pub starting_campaign_sha256: Digest32,
    pub starting_campaign_byte_length: u64,
    pub simulation_seed: SimulationSeed64,
    pub rules_config_sha256: Digest32,
    pub resource_locale_root: ResourceLocaleRootV1,
    pub speech_timing: SpeechTimingAuthorityV1,
    /// Reserved for explicitly unranked Spellforge simulations. Official
    /// ranked seals reject it until immutable policy semantics exist.
    pub spellforge_content_sha256: Option<Digest32>,
    /// Original-parity RNG streams are replay inputs, not ranked RNG. Their
    /// presence makes the seal unrankable.
    pub original_rng_replay_sha256: Option<Digest32>,
}

impl Validate for PreparedMissionInputsSealV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PreparedMissionInputsSealV1", self.schema_version)?;
        self.content_subject.validate()?;
        if [
            self.prepared_inputs_projection_sha256,
            self.content_manifest_sha256,
            self.starting_campaign_sha256,
            self.rules_config_sha256,
        ]
        .into_iter()
        .any(|digest| digest.is_zero())
            || self.starting_campaign_byte_length == 0
            || self
                .spellforge_content_sha256
                .is_some_and(|digest| digest.is_zero())
            || self
                .original_rng_replay_sha256
                .is_some_and(|digest| digest.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "prepared_mission_inputs_seal.identity_digest",
            });
        }
        self.resource_locale_root.validate()?;
        self.speech_timing.validate()
    }
}

impl PreparedMissionInputsSealV1 {
    pub fn validate_rankable(&self) -> Result<(), ValidationError> {
        self.validate()?;
        if self.spellforge_content_sha256.is_some() || self.original_rng_replay_sha256.is_some() {
            return Err(ValidationError::ClaimMismatch {
                field: "prepared_mission_inputs_seal.unranked_input_mode",
            });
        }
        Ok(())
    }
}

/// Anonymous-safe seat lifecycle recorded by current canonical replays. Random IDs
/// remain stable across reconnects; no username, EndpointId, public key, or
/// wall-clock timestamp is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReplaySeatLifecycleKindV1 {
    Connected { connection_epoch: u32 },
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplaySeatLifecycleEventV1 {
    pub event_ordinal: u32,
    /// Dense replay ordinal containing the matching authoritative
    /// `ConnectSeat`/`DisconnectSeat` command. This is deliberately not the
    /// simulation timeline frame: paused and other non-advancing host records
    /// make those domains diverge.
    pub replay_ordinal: u32,
    pub seat: u16,
    pub participant_instance_id: Digest32,
    pub lifecycle: ReplaySeatLifecycleKindV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplaySessionTranscriptV1 {
    pub schema_version: u32,
    pub session_genesis_sha256: Digest32,
    pub replay_session_id: Digest32,
    pub host_participant_instance_id: Digest32,
    pub participant_instance_count: u16,
    pub max_concurrent_players: u16,
    pub events: Vec<ReplaySeatLifecycleEventV1>,
}

impl ReplaySessionTranscriptV1 {
    pub fn validate_and_derive_counts(&self) -> Result<(u16, u16), ValidationError> {
        crate::validation::schema("ReplaySessionTranscriptV1", self.schema_version)?;
        if self.session_genesis_sha256.is_zero()
            || self.replay_session_id.is_zero()
            || self.host_participant_instance_id.is_zero()
        {
            return Err(ValidationError::Zero {
                field: "replay_session_transcript.identity",
            });
        }
        if self.max_concurrent_players == 0
            || self.max_concurrent_players > MAX_REPLAY_SEATS_V1
            || self.participant_instance_count == 0
            || self.participant_instance_count > MAX_PARTICIPANT_INSTANCES_V1
        {
            return Err(ValidationError::CountOutOfRange {
                field: "replay_session_transcript.counts",
            });
        }
        if self.events.is_empty() || self.events.len() > 16_384 {
            return Err(ValidationError::CountOutOfRange {
                field: "replay_session_transcript.events",
            });
        }
        let first = self.events.first().expect("nonempty checked");
        if first.event_ordinal != 0
            || first.replay_ordinal != 0
            || first.seat != 0
            || first.participant_instance_id != self.host_participant_instance_id
            || first.lifecycle
                != (ReplaySeatLifecycleKindV1::Connected {
                    connection_epoch: 0,
                })
        {
            return Err(ValidationError::ClaimMismatch {
                field: "replay_session_transcript.host_genesis",
            });
        }
        let mut occupied = BTreeMap::<u16, Digest32>::new();
        let mut instance_seats = BTreeMap::<Digest32, u16>::new();
        let mut last_connection_epochs = BTreeMap::<Digest32, u32>::new();
        let mut seen_seats = BTreeSet::<u16>::new();
        let mut maximum = 0_usize;
        let mut previous_replay_ordinal = 0_u32;
        for (index, event) in self.events.iter().enumerate() {
            if event.event_ordinal != u32::try_from(index).unwrap_or(u32::MAX)
                || event.replay_ordinal < previous_replay_ordinal
                || event.participant_instance_id.is_zero()
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "replay_session_transcript.event_order",
                });
            }
            previous_replay_ordinal = event.replay_ordinal;
            match event.lifecycle {
                ReplaySeatLifecycleKindV1::Connected { connection_epoch } => {
                    let prior_epoch = last_connection_epochs.get(&event.participant_instance_id);
                    if event.seat >= MAX_REPLAY_SEATS_V1
                        || occupied.contains_key(&event.seat)
                        || instance_seats
                            .get(&event.participant_instance_id)
                            .is_some_and(|seat| *seat != event.seat)
                        || prior_epoch.map_or(connection_epoch != 0, |epoch| {
                            connection_epoch != epoch.saturating_add(1)
                        })
                        || (!seen_seats.contains(&event.seat)
                            && usize::from(event.seat) != seen_seats.len())
                        || (event.seat == 0 && index != 0)
                    {
                        return Err(ValidationError::ClaimMismatch {
                            field: "replay_session_transcript.connect",
                        });
                    }
                    occupied.insert(event.seat, event.participant_instance_id);
                    instance_seats.insert(event.participant_instance_id, event.seat);
                    seen_seats.insert(event.seat);
                    last_connection_epochs.insert(event.participant_instance_id, connection_epoch);
                    maximum = maximum.max(occupied.len());
                }
                ReplaySeatLifecycleKindV1::Disconnected => {
                    if event.seat == 0
                        || occupied.remove(&event.seat) != Some(event.participant_instance_id)
                    {
                        return Err(ValidationError::ClaimMismatch {
                            field: "replay_session_transcript.disconnect",
                        });
                    }
                }
            }
        }
        let participant_instance_count =
            u16::try_from(instance_seats.len()).map_err(|_| ValidationError::CountOutOfRange {
                field: "replay_session_transcript.participant_instance_count",
            })?;
        let max_concurrent_players =
            u16::try_from(maximum).map_err(|_| ValidationError::CountOutOfRange {
                field: "replay_session_transcript.max_concurrent_players",
            })?;
        if participant_instance_count != self.participant_instance_count
            || max_concurrent_players != self.max_concurrent_players
        {
            return Err(ValidationError::ClaimMismatch {
                field: "replay_session_transcript.derived_counts",
            });
        }
        Ok((participant_instance_count, max_concurrent_players))
    }
}

impl Validate for ReplaySessionTranscriptV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.validate_and_derive_counts().map(|_| ())
    }
}
