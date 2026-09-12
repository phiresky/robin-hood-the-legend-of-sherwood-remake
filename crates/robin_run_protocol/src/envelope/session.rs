//! Session genesis, authenticated seat joins and anonymous replay transcript validation.

use super::{
    CampaignContinuationPreflightGrantV1, CompetitionRunGrantV1, FreshRunPreflightGrantV1,
    MAX_PARTICIPANT_INSTANCES_V1, MAX_REPLAY_SEATS_V1, NAMED_SEAT_JOIN_SIGNATURE_DOMAIN_V1,
    PreparedMissionInputsSealV1, REPLAY_SESSION_GENESIS_SIGNATURE_DOMAIN_V1, SignatureAlgorithmV1,
};
use crate::CanonicalDocument as _;
use crate::{
    ChallengeNonce32, ContentManifestV1, Digest32, OfficialContentEditionV1,
    OfficialContentSubjectV1, PublicKey32, ResourceLocaleRootV1, Signature64, SimulationSeed64,
    SimulationSpeechTimingSourceV1, Validate, ValidationError,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::BTreeSet;

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
    pub(super) fn validate(&self) -> Result<(), ValidationError> {
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

/// Exact deterministic authority configured before `BeginSim`. Runtime
/// `Option::None` means explicitly unranked multiplayer and must never create
/// or sign a ranked genesis after simulation begins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankedSessionConfigV1 {
    pub schema_version: u32,
    pub mission_id: String,
    pub content_edition: OfficialContentEditionV1,
    pub content_subject: OfficialContentSubjectV1,
    pub simulation_seed: SimulationSeed64,
    /// SHA-256 and length of the exact canonical campaign bytes consumed by
    /// the ranked Engine and co-signed as `SubmissionArtifactsV1::starting_campaign`.
    pub starting_campaign_sha256: Digest32,
    pub starting_campaign_byte_length: u64,
    /// Digest of the canonical run-specific engine-input projection. This
    /// binds team, inventory, reinforcement and dependency closure derived
    /// from the exact starting campaign, not just static official content.
    pub prepared_inputs_projection_sha256: Digest32,
    /// Digest of `PreparedMissionInputsSealV1`, cross-binding the projection
    /// with every engine-input authority below.
    pub prepared_mission_inputs_seal_sha256: Digest32,
    pub build_manifest_sha256: Digest32,
    pub content_manifest_sha256: Digest32,
    pub campaign_content_manifest_sha256: Option<Digest32>,
    pub rules_config_sha256: Digest32,
    /// Complete custom settings, authenticated by the genesis signature and
    /// their canonical digest. Absent on historical exact-preset sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_rules_config: Option<crate::RulesConfigIdentityV1>,
    /// Fresh campaign proposal checked independently by the verifier. Each
    /// continuation retains the accepted genesis artifact unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_canonical_campaign: Option<crate::ArtifactRefV1>,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
    pub spellforge_content_sha256: Option<Digest32>,
    pub resource_locale_root: ResourceLocaleRootV1,
    pub speech_timing: SpeechTimingAuthorityV1,
}

impl Validate for RankedSessionConfigV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("RankedSessionConfigV1", self.schema_version)?;
        crate::validation::text("ranked_session.mission_id", &self.mission_id, 256)?;
        self.content_subject.validate()?;
        if self.content_subject.mission_id() != self.mission_id {
            return Err(ValidationError::ClaimMismatch {
                field: "ranked_session.content_subject.mission_id",
            });
        }
        if [
            self.starting_campaign_sha256,
            self.prepared_inputs_projection_sha256,
            self.prepared_mission_inputs_seal_sha256,
            self.build_manifest_sha256,
            self.content_manifest_sha256,
            self.rules_config_sha256,
            self.ruleset_manifest_sha256,
        ]
        .into_iter()
        .any(|digest| digest.is_zero())
            || self.starting_campaign_byte_length == 0
            || self
                .competition_manifest_sha256
                .is_some_and(|digest| digest.is_zero())
            || self
                .campaign_content_manifest_sha256
                .is_some_and(|digest| digest.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "ranked_session.identity_digest",
            });
        }
        if self.spellforge_content_sha256.is_some() {
            return Err(ValidationError::ClaimMismatch {
                field: "ranked_session.spellforge_unranked",
            });
        }
        if self.custom_rules_config.is_some() != self.custom_canonical_campaign.is_some() {
            return Err(ValidationError::ClaimMismatch {
                field: "ranked_session.custom_canonical_campaign",
            });
        }
        if let Some(artifact) = &self.custom_canonical_campaign {
            artifact.validate()?;
            if artifact.media_type != crate::RANKED_CAMPAIGN_MEDIA_TYPE_V1 {
                return Err(ValidationError::ClaimMismatch {
                    field: "ranked_session.custom_canonical_campaign.media_type",
                });
            }
        }
        if let Some(config) = &self.custom_rules_config {
            config.validate()?;
            if config.canonical_digest().ok() != Some(self.rules_config_sha256)
                || self.competition_manifest_sha256.is_some()
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "ranked_session.custom_rules_config",
                });
            }
        }
        self.resource_locale_root.validate()?;
        self.speech_timing.validate()
    }
}

impl RankedSessionConfigV1 {
    pub fn validate_content_manifest(
        &self,
        manifest: &ContentManifestV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        manifest.validate()?;
        let digest = manifest
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "ranked_session.content_manifest",
            })?;
        if digest != self.content_manifest_sha256 {
            return Err(ValidationError::ClaimMismatch {
                field: "ranked_session.content_manifest_sha256",
            });
        }
        if manifest.subject != self.content_subject {
            return Err(ValidationError::ClaimMismatch {
                field: "ranked_session.content_subject",
            });
        }
        if manifest.edition != self.content_edition {
            return Err(ValidationError::ClaimMismatch {
                field: "ranked_session.content_edition",
            });
        }
        if manifest.resource_locale_root != self.resource_locale_root {
            return Err(ValidationError::ClaimMismatch {
                field: "ranked_session.resource_locale_root",
            });
        }
        let speech_matches = matches!(
            (&self.speech_timing, &manifest.speech_timing),
            (
                SpeechTimingAuthorityV1::BaseInstallation,
                SimulationSpeechTimingSourceV1::BaseInstallation
            ) | (
                SpeechTimingAuthorityV1::CoreAudioDurationsV1,
                SimulationSpeechTimingSourceV1::CoreAudioDurationsV1
            )
        ) || matches!(
            (&self.speech_timing, &manifest.speech_timing),
            (
                SpeechTimingAuthorityV1::LanguagePack { canonical_locale: ranked },
                SimulationSpeechTimingSourceV1::LanguagePack { canonical_locale: manifest }
            ) if ranked == manifest
        );
        if !speech_matches {
            return Err(ValidationError::ClaimMismatch {
                field: "ranked_session.speech_timing.content_projection",
            });
        }
        Ok(())
    }

    pub fn validate_prepared_inputs_seal(
        &self,
        seal: &PreparedMissionInputsSealV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        seal.validate_rankable()?;
        let seal_digest = seal
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "ranked_session.prepared_mission_inputs_seal",
            })?;
        if seal_digest != self.prepared_mission_inputs_seal_sha256
            || seal.prepared_inputs_projection_sha256 != self.prepared_inputs_projection_sha256
            || seal.content_manifest_sha256 != self.content_manifest_sha256
            || seal.content_edition != self.content_edition
            || seal.content_subject != self.content_subject
            || seal.starting_campaign_sha256 != self.starting_campaign_sha256
            || seal.starting_campaign_byte_length != self.starting_campaign_byte_length
            || seal.simulation_seed != self.simulation_seed
            || seal.rules_config_sha256 != self.rules_config_sha256
            || seal.resource_locale_root != self.resource_locale_root
            || seal.speech_timing != self.speech_timing
            || seal.spellforge_content_sha256 != self.spellforge_content_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "ranked_session.prepared_mission_inputs_seal",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplaySessionGenesisClaimV1 {
    pub schema_version: u32,
    pub network_protocol_version: u32,
    pub host_public_key: PublicKey32,
    pub replay_session_id: Digest32,
    pub host_participant_instance_id: Digest32,
    pub host_nonce: ChallengeNonce32,
    pub ranked_session: RankedSessionConfigV1,
    /// Present exactly for fresh individual-level and campaign-genesis runs.
    /// Scope is rechecked when the server later authors an upload offer;
    /// continuations instead bind a verified predecessor.
    #[serde(deserialize_with = "Option::deserialize")]
    pub fresh_run_preflight_grant: Option<FreshRunPreflightGrantV1>,
    /// Present exactly for campaign continuations. It proves that the active
    /// chain controller and the new host authorized this session before frame
    /// zero and that the service recognized the exact predecessor.
    #[serde(deserialize_with = "Option::deserialize")]
    pub campaign_continuation_preflight_grant: Option<CampaignContinuationPreflightGrantV1>,
    /// Required exactly for scheduled competition sessions. The explicit
    /// `null` in ordinary sessions keeps the current schema unambiguous; a
    /// missing field is not an accepted older-schema compatibility lane.
    #[serde(deserialize_with = "Option::deserialize")]
    pub competition_run_grant: Option<CompetitionRunGrantV1>,
}

impl ReplaySessionGenesisClaimV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        <Self as crate::canonical::DomainSignedClaim>::signing_bytes(self)
    }
}

impl crate::canonical::DomainSignedClaim for ReplaySessionGenesisClaimV1 {
    const DOMAIN: &'static [u8] = REPLAY_SESSION_GENESIS_SIGNATURE_DOMAIN_V1;
}

impl Validate for ReplaySessionGenesisClaimV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("ReplaySessionGenesisClaimV1", self.schema_version)?;
        if self.network_protocol_version == 0 {
            return Err(ValidationError::Zero {
                field: "session_genesis.network_protocol_version",
            });
        }
        if self.host_public_key.is_zero()
            || self.host_nonce.is_zero()
            || [self.replay_session_id, self.host_participant_instance_id]
                .into_iter()
                .any(|digest| digest.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "session_genesis.identity",
            });
        }
        self.ranked_session.validate()?;
        if self.fresh_run_preflight_grant.is_some()
            && self.campaign_continuation_preflight_grant.is_some()
        {
            return Err(ValidationError::ClaimMismatch {
                field: "session_genesis.run_preflight_grant_presence",
            });
        }
        if let Some(grant) = &self.fresh_run_preflight_grant {
            grant.validate()?;
            let ranked_session_sha256 = self.ranked_session.canonical_digest().map_err(|_| {
                ValidationError::ClaimMismatch {
                    field: "session_genesis.fresh_run_preflight_grant.ranked_session_sha256",
                }
            })?;
            if grant.claim.host_public_key != self.host_public_key
                || grant.claim.ranked_session_sha256 != ranked_session_sha256
                || grant.claim.replay_session_id != self.replay_session_id
                || grant.claim.host_participant_instance_id != self.host_participant_instance_id
                || grant.claim.host_nonce != self.host_nonce
                || grant.claim.starting_campaign.sha256
                    != self.ranked_session.starting_campaign_sha256
                || grant.claim.starting_campaign.byte_length
                    != self.ranked_session.starting_campaign_byte_length
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "session_genesis.fresh_run_preflight_grant",
                });
            }
        }
        if let Some(grant) = &self.campaign_continuation_preflight_grant {
            grant.validate()?;
            let ranked_session_sha256 = self.ranked_session.canonical_digest().map_err(|_| {
                ValidationError::ClaimMismatch {
                    field: "session_genesis.campaign_continuation_preflight_grant.ranked_session_sha256",
                }
            })?;
            if grant.claim.host_public_key != self.host_public_key
                || grant.claim.ranked_session_sha256 != ranked_session_sha256
                || grant.claim.replay_session_id != self.replay_session_id
                || grant.claim.host_participant_instance_id != self.host_participant_instance_id
                || grant.claim.host_nonce != self.host_nonce
                || grant.claim.starting_campaign.sha256
                    != self.ranked_session.starting_campaign_sha256
                || grant.claim.starting_campaign.byte_length
                    != self.ranked_session.starting_campaign_byte_length
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "session_genesis.campaign_continuation_preflight_grant",
                });
            }
        }
        match (
            self.ranked_session.competition_manifest_sha256,
            &self.competition_run_grant,
        ) {
            (None, None) => Ok(()),
            (Some(competition), Some(grant)) => {
                grant.validate()?;
                let ranked_sha256 = self.ranked_session.canonical_digest().map_err(|_| {
                    ValidationError::ClaimMismatch {
                        field: "session_genesis.competition_run_grant.ranked_session_sha256",
                    }
                })?;
                if grant.claim.host_public_key != self.host_public_key
                    || grant.claim.competition_manifest_sha256 != competition
                    || grant.claim.ranked_session_sha256 != ranked_sha256
                    || grant.claim.replay_session_id != self.replay_session_id
                    || grant.claim.host_participant_instance_id != self.host_participant_instance_id
                    || grant.claim.host_nonce != self.host_nonce
                {
                    return Err(ValidationError::ClaimMismatch {
                        field: "session_genesis.competition_run_grant",
                    });
                }
                Ok(())
            }
            _ => Err(ValidationError::ClaimMismatch {
                field: "session_genesis.competition_run_grant_presence",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplaySessionGenesisV1 {
    pub claim: ReplaySessionGenesisClaimV1,
    pub algorithm: SignatureAlgorithmV1,
    pub host_signature: Signature64,
}

impl ReplaySessionGenesisV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        self.claim.signing_bytes()
    }
}

impl Validate for ReplaySessionGenesisV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.claim.validate()?;
        crate::validation::nonzero("session_genesis.host_signature", &self.host_signature)?;
        Ok(())
    }
}

/// Guest-authored claim signed by the durable ranking key. The server verifies
/// the signature and independently compares `transport_endpoint_id` to the
/// authenticated remote iroh EndpointId at join; the verifier later
/// cross-binds it to the replay transcript and final submission co-sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedSeatJoinClaimV1 {
    pub schema_version: u32,
    pub session_genesis_sha256: Digest32,
    /// Durable ranking identity which must co-sign the final submission.
    pub public_key: PublicKey32,
    /// Ephemeral iroh transport identity observed on the authenticated join
    /// stream. The host must compare this exact value to the remote EndpointId;
    /// it is deliberately independent from the durable ranking key.
    pub transport_endpoint_id: PublicKey32,
    pub host_endpoint_id: PublicKey32,
    pub replay_session_id: Digest32,
    pub participant_instance_id: Digest32,
    pub seat: u16,
    pub connection_epoch: u32,
    pub join_event_ordinal: u32,
    pub mission_id: String,
    pub content_manifest_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
    pub host_nonce: ChallengeNonce32,
}

impl NamedSeatJoinClaimV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        <Self as crate::canonical::DomainSignedClaim>::signing_bytes(self)
    }
}

impl crate::canonical::DomainSignedClaim for NamedSeatJoinClaimV1 {
    const DOMAIN: &'static [u8] = NAMED_SEAT_JOIN_SIGNATURE_DOMAIN_V1;
}

impl Validate for NamedSeatJoinClaimV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("NamedSeatJoinClaimV1", self.schema_version)?;
        if self.public_key.is_zero()
            || self.transport_endpoint_id.is_zero()
            || self.host_endpoint_id.is_zero()
            || self.host_nonce.is_zero()
            || [
                self.session_genesis_sha256,
                self.replay_session_id,
                self.participant_instance_id,
                self.content_manifest_sha256,
                self.rules_config_sha256,
                self.ruleset_manifest_sha256,
            ]
            .into_iter()
            .any(|digest| digest.is_zero())
            || self
                .competition_manifest_sha256
                .is_some_and(|digest| digest.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "named_seat_join.identity",
            });
        }
        crate::validation::text("named_seat_join.mission_id", &self.mission_id, 256)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedSeatJoinAttestationV1 {
    pub claim: NamedSeatJoinClaimV1,
    pub algorithm: SignatureAlgorithmV1,
    pub signature: Signature64,
}

impl NamedSeatJoinAttestationV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        self.claim.signing_bytes()
    }
}

impl Validate for NamedSeatJoinAttestationV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.claim.validate()?;
        crate::validation::nonzero("named_seat_join.signature", &self.signature)?;
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
