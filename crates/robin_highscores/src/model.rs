use serde::{Deserialize, Serialize};

pub const API_SCHEMA_VERSION: u32 = 1;
pub const MAX_PARTICIPANT_SEATS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengePurpose {
    Submission,
    UsernameUpdate,
    Deletion,
    OwnerStatus,
}

impl ChallengePurpose {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Submission => "submission",
            Self::UsernameUpdate => "username_update",
            Self::Deletion => "deletion",
            Self::OwnerStatus => "owner_status",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionStatus {
    Queued,
    Verifying,
    RetryPending,
    Accepted,
    Rejected,
    Failed,
}

impl SubmissionStatus {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "queued" => Ok(Self::Queued),
            "verifying" => Ok(Self::Verifying),
            "retry_pending" => Ok(Self::RetryPending),
            "accepted" => Ok(Self::Accepted),
            "rejected" => Ok(Self::Rejected),
            "failed" => Ok(Self::Failed),
            other => Err(format!("invalid stored submission status `{other}`")),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Verifying => "verifying",
            Self::RetryPending => "retry_pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantClaim {
    pub seat: u16,
    pub participant_instance_id: [u8; 32],
    pub public_key: [u8; 32],
    pub public_disclosure: String,
}

#[derive(Debug, Clone)]
pub(crate) struct NewSubmission {
    pub id: String,
    pub upload_challenge_id: String,
    pub offer_json: String,
    pub envelope_json: String,
    pub signatures_json: String,
    pub public_metadata_json: String,
    pub replay_sha256: [u8; 32],
    pub replay_bytes: u64,
    pub build_manifest_id: [u8; 32],
    pub content_manifest_id: [u8; 32],
    pub campaign_content_manifest_id: Option<[u8; 32]>,
    pub config_id: [u8; 32],
    pub ruleset_id: [u8; 32],
    pub mission_id: String,
    pub scope_kind: String,
    pub starting_campaign_sha256: [u8; 32],
    pub starting_campaign_bytes: u64,
    pub canonical_campaign_state_json: String,
    pub controller_public_key: [u8; 32],
    pub starting_state_json: String,
    pub campaign_chain_id: Option<String>,
    pub predecessor_run_id: Option<String>,
    pub competition_manifest_id: Option<[u8; 32]>,
    pub requested_metrics_json: String,
    pub participant_claims_json: String,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    pub session_genesis_sha256: [u8; 32],
    pub session_genesis_host_public_key: [u8; 32],
    pub replay_session_id: [u8; 32],
    pub session_genesis_host_nonce: [u8; 32],
    pub participants: Vec<ParticipantClaim>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionLifecycle {
    pub id: String,
    pub state: SubmissionState,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

/// Validated database state; terminal payloads cannot be omitted by callers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum SubmissionState {
    Queued,
    Verifying,
    RetryPending,
    Accepted {
        run_id: robin_run_protocol::OpaqueId,
    },
    Rejected {
        code: robin_run_protocol::VerificationRejectionCodeV1,
    },
    Failed,
}

impl SubmissionState {
    pub(crate) fn from_columns(
        status: &str,
        run_id: Option<String>,
        rejection_code: Option<String>,
    ) -> Result<Self, String> {
        let status = SubmissionStatus::parse(status)?;
        if status != SubmissionStatus::Accepted && run_id.is_some() {
            return Err("non-accepted submission has a run ID".into());
        }
        if status != SubmissionStatus::Rejected && rejection_code.is_some() {
            return Err("non-rejected submission has a rejection code".into());
        }
        Ok(match status {
            SubmissionStatus::Queued => Self::Queued,
            SubmissionStatus::Verifying => Self::Verifying,
            SubmissionStatus::RetryPending => Self::RetryPending,
            SubmissionStatus::Failed => Self::Failed,
            SubmissionStatus::Accepted => Self::Accepted {
                run_id: robin_run_protocol::OpaqueId::new(
                    run_id.ok_or("accepted submission has no run ID")?,
                )
                .map_err(|error| error.to_string())?,
            },
            SubmissionStatus::Rejected => Self::Rejected {
                code: rejection_code
                    .ok_or("rejected submission has no rejection code")?
                    .parse::<robin_run_protocol::VerificationRejectionCodeV1>()
                    .map_err(|error| error.to_string())?,
            },
        })
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[test]
    fn terminal_rows_require_valid_payloads() {
        assert!(SubmissionState::from_columns("accepted", None, None).is_err());
        assert!(SubmissionState::from_columns("accepted", Some(String::new()), None).is_err());
        assert!(SubmissionState::from_columns("rejected", None, None).is_err());
        assert!(SubmissionState::from_columns("rejected", None, Some("unknown".into())).is_err());
        assert!(SubmissionState::from_columns("queued", Some("run".into()), None).is_err());
        assert!(SubmissionState::from_columns("accepted", Some("run".into()), None).is_ok());
        assert!(
            SubmissionState::from_columns("rejected", None, Some("malformed_replay".into()))
                .is_ok()
        );
    }
}

#[derive(Debug, Clone)]
pub struct WorkerJob {
    pub submission_id: String,
    pub replay_sha256: [u8; 32],
    pub replay_bytes: u64,
    pub starting_campaign_sha256: [u8; 32],
    pub starting_campaign_bytes: u64,
    pub envelope_json: String,
    pub attempts: u32,
}

pub fn now_epoch_ms() -> Result<i64, std::time::SystemTimeError> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    Ok(i64::try_from(millis).expect("current epoch milliseconds must fit in i64"))
}
