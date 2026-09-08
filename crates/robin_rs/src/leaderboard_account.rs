//! Frame-polled username, deletion, and moderation coordination.
//!
//! This module does not own an identity implementation. It defines a closed,
//! typed task boundary which native and isolated-browser signers can adapt to;
//! there is intentionally no raw key export or generic `sign(bytes)` request.

use crate::leaderboard_http::{HttpResponse, HttpTask, HttpTransportError};
use crate::leaderboard_service::{
    LeaderboardApi, LeaderboardServiceError, decode_abuse_report_accepted,
    decode_deletion_challenge, decode_deletion_receipt, decode_player_profile,
    decode_username_challenge,
};
use robin_run_protocol::{
    AbuseReportAcceptedV1, AbuseReportV1, DeletionChallengeRequestV1, DeletionReceiptV1,
    DeletionRequestEnvelopeV1, DeletionTargetV1, OpaqueId, PlayerProfileV1, PublicKey32,
    SCHEMA_VERSION_V1, Signature64, UsernameChallengeRequestV1, UsernameUpdateEnvelopeV1,
    Validate as _,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnedLeaderboardTarget {
    Submission { submission_id: OpaqueId },
    Run { run_id: OpaqueId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaderboardSigningRequest {
    PublicKey,
    UsernameUpdate(UsernameUpdateEnvelopeV1),
    DeletionRequest(DeletionRequestEnvelopeV1),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaderboardSigningResponse {
    PublicKey(PublicKey32),
    UsernameUpdate(UsernameUpdateEnvelopeV1),
    DeletionRequest(DeletionRequestEnvelopeV1),
}

pub trait LeaderboardSigningTask {
    /// Non-blocking poll. Errors must be safe for local display and must not
    /// contain key material.
    fn try_take(&mut self) -> Option<Result<LeaderboardSigningResponse, String>>;
}

pub trait LeaderboardAccountSigner {
    fn start(
        &mut self,
        request: LeaderboardSigningRequest,
    ) -> Result<Box<dyn LeaderboardSigningTask>, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountOperationState {
    Idle,
    Renaming,
    Renamed(PlayerProfileV1),
    Deleting,
    Deleted(DeletionReceiptV1),
    Reporting,
    Reported(AbuseReportAcceptedV1),
    Failed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum AccountCoordinatorError {
    #[error("another leaderboard account operation is already in progress")]
    Busy,
    #[error("signer returned a response for a different typed operation")]
    SigningResponseMismatch,
    #[error("signer rejected the typed leaderboard operation: {0}")]
    Signing(String),
    #[error("leaderboard operation response does not match its request")]
    ResponseMismatch,
    #[error(transparent)]
    Service(#[from] LeaderboardServiceError),
}

enum AccountTask {
    PublicKey {
        task: Box<dyn LeaderboardSigningTask>,
        purpose: PublicKeyPurpose,
    },
    UsernameChallenge {
        task: HttpTask,
        username: String,
        public_key: PublicKey32,
    },
    UsernameSignature {
        task: Box<dyn LeaderboardSigningTask>,
        unsigned: UsernameUpdateEnvelopeV1,
    },
    UsernameUpdate {
        task: HttpTask,
        username: String,
        public_key: PublicKey32,
    },
    DeletionChallenge {
        task: HttpTask,
        target: DeletionTargetV1,
        public_key: PublicKey32,
    },
    DeletionSignature {
        task: Box<dyn LeaderboardSigningTask>,
        unsigned: DeletionRequestEnvelopeV1,
    },
    DeletionRequest {
        task: HttpTask,
        target: DeletionTargetV1,
    },
    Report(HttpTask),
}

enum PublicKeyPurpose {
    Rename(String),
    Delete(DeletionTargetV1),
}

enum TaskResult {
    Signing(Result<LeaderboardSigningResponse, String>),
    Http(Result<HttpResponse, HttpTransportError>),
}

pub struct LeaderboardAccountCoordinator {
    api: LeaderboardApi,
    signer: Box<dyn LeaderboardAccountSigner>,
    task: Option<AccountTask>,
    state: AccountOperationState,
}

impl LeaderboardAccountCoordinator {
    pub fn new(api: LeaderboardApi, signer: Box<dyn LeaderboardAccountSigner>) -> Self {
        Self {
            api,
            signer,
            task: None,
            state: AccountOperationState::Idle,
        }
    }

    pub fn state(&self) -> &AccountOperationState {
        &self.state
    }

    pub fn rename(&mut self, username: String) -> Result<(), AccountCoordinatorError> {
        self.require_idle_or_terminal()?;
        let task = self
            .signer
            .start(LeaderboardSigningRequest::PublicKey)
            .map_err(AccountCoordinatorError::Signing)?;
        self.task = Some(AccountTask::PublicKey {
            task,
            purpose: PublicKeyPurpose::Rename(username),
        });
        self.state = AccountOperationState::Renaming;
        Ok(())
    }

    pub fn delete(&mut self, owned: OwnedLeaderboardTarget) -> Result<(), AccountCoordinatorError> {
        self.require_idle_or_terminal()?;
        let target = match owned {
            OwnedLeaderboardTarget::Submission { submission_id } => {
                DeletionTargetV1::Submission { submission_id }
            }
            OwnedLeaderboardTarget::Run { run_id } => DeletionTargetV1::Run { run_id },
        };
        let task = self
            .signer
            .start(LeaderboardSigningRequest::PublicKey)
            .map_err(AccountCoordinatorError::Signing)?;
        self.task = Some(AccountTask::PublicKey {
            task,
            purpose: PublicKeyPurpose::Delete(target),
        });
        self.state = AccountOperationState::Deleting;
        Ok(())
    }

    pub fn report(&mut self, report: AbuseReportV1) -> Result<(), AccountCoordinatorError> {
        self.require_idle_or_terminal()?;
        self.task = Some(AccountTask::Report(self.api.report_abuse(&report)?));
        self.state = AccountOperationState::Reporting;
        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), AccountCoordinatorError> {
        self.require_idle_or_terminal()?;
        self.state = AccountOperationState::Idle;
        Ok(())
    }

    pub fn poll(&mut self) -> Result<bool, AccountCoordinatorError> {
        let Some(result) = self.task.as_mut().and_then(|task| match task {
            AccountTask::PublicKey { task, .. }
            | AccountTask::UsernameSignature { task, .. }
            | AccountTask::DeletionSignature { task, .. } => {
                task.try_take().map(TaskResult::Signing)
            }
            AccountTask::UsernameChallenge { task, .. }
            | AccountTask::UsernameUpdate { task, .. }
            | AccountTask::DeletionChallenge { task, .. }
            | AccountTask::DeletionRequest { task, .. }
            | AccountTask::Report(task) => task.try_take().map(TaskResult::Http),
        }) else {
            return Ok(false);
        };
        let task = self.task.take().expect("completed account task exists");
        let result = self.advance(task, result);
        if let Err(error) = &result {
            self.state = AccountOperationState::Failed(error.to_string());
        }
        result.map(|()| true)
    }

    fn advance(
        &mut self,
        task: AccountTask,
        result: TaskResult,
    ) -> Result<(), AccountCoordinatorError> {
        match (task, result) {
            (
                AccountTask::PublicKey { purpose, .. },
                TaskResult::Signing(Ok(LeaderboardSigningResponse::PublicKey(public_key))),
            ) => {
                if public_key.is_zero() {
                    return Err(AccountCoordinatorError::SigningResponseMismatch);
                }
                match purpose {
                    PublicKeyPurpose::Rename(username) => {
                        let request = UsernameChallengeRequestV1 {
                            schema_version: SCHEMA_VERSION_V1,
                            public_key,
                        };
                        self.task = Some(AccountTask::UsernameChallenge {
                            task: self.api.username_challenge(&request)?,
                            username,
                            public_key,
                        });
                    }
                    PublicKeyPurpose::Delete(target) => {
                        let request = DeletionChallengeRequestV1 {
                            schema_version: SCHEMA_VERSION_V1,
                            public_key,
                            target: target.clone(),
                        };
                        self.task = Some(AccountTask::DeletionChallenge {
                            task: self.api.deletion_challenge(&request)?,
                            target,
                            public_key,
                        });
                    }
                }
            }
            (
                AccountTask::UsernameChallenge {
                    username,
                    public_key,
                    ..
                },
                TaskResult::Http(result),
            ) => {
                let challenge = decode_username_challenge(result)?;
                let unsigned = UsernameUpdateEnvelopeV1 {
                    schema_version: SCHEMA_VERSION_V1,
                    username_challenge_id: challenge.username_challenge_id,
                    username_challenge_nonce: challenge.username_challenge_nonce,
                    public_key,
                    username,
                    signature: Signature64::default(),
                };
                unsigned
                    .validate_signing_claim()
                    .map_err(|error| AccountCoordinatorError::Signing(error.to_string()))?;
                let signing_task = self
                    .signer
                    .start(LeaderboardSigningRequest::UsernameUpdate(unsigned.clone()))
                    .map_err(AccountCoordinatorError::Signing)?;
                self.task = Some(AccountTask::UsernameSignature {
                    task: signing_task,
                    unsigned,
                });
            }
            (
                AccountTask::UsernameSignature { unsigned, .. },
                TaskResult::Signing(Ok(LeaderboardSigningResponse::UsernameUpdate(signed))),
            ) => {
                validate_signed_username(&unsigned, &signed)?;
                self.task = Some(AccountTask::UsernameUpdate {
                    task: self.api.update_username(&signed)?,
                    username: signed.username,
                    public_key: signed.public_key,
                });
            }
            (
                AccountTask::UsernameUpdate {
                    username,
                    public_key,
                    ..
                },
                TaskResult::Http(result),
            ) => {
                let profile = decode_player_profile(result)?;
                if profile.public_key != public_key || profile.username != username {
                    return Err(AccountCoordinatorError::ResponseMismatch);
                }
                self.state = AccountOperationState::Renamed(profile);
            }
            (
                AccountTask::DeletionChallenge {
                    target, public_key, ..
                },
                TaskResult::Http(result),
            ) => {
                let challenge = decode_deletion_challenge(result)?;
                if challenge.target != target || challenge.public_key != public_key {
                    return Err(AccountCoordinatorError::ResponseMismatch);
                }
                let unsigned = DeletionRequestEnvelopeV1 {
                    schema_version: SCHEMA_VERSION_V1,
                    challenge,
                    signature: Signature64::default(),
                };
                unsigned
                    .validate_signing_claim()
                    .map_err(|error| AccountCoordinatorError::Signing(error.to_string()))?;
                let signing_task = self
                    .signer
                    .start(LeaderboardSigningRequest::DeletionRequest(unsigned.clone()))
                    .map_err(AccountCoordinatorError::Signing)?;
                self.task = Some(AccountTask::DeletionSignature {
                    task: signing_task,
                    unsigned,
                });
            }
            (
                AccountTask::DeletionSignature { unsigned, .. },
                TaskResult::Signing(Ok(LeaderboardSigningResponse::DeletionRequest(signed))),
            ) => {
                validate_signed_deletion(&unsigned, &signed)?;
                let target = signed.challenge.target.clone();
                self.task = Some(AccountTask::DeletionRequest {
                    task: self.api.delete_owned_run(&signed)?,
                    target,
                });
            }
            (AccountTask::DeletionRequest { target, .. }, TaskResult::Http(result)) => {
                let receipt = decode_deletion_receipt(result)?;
                if receipt.target != target {
                    return Err(AccountCoordinatorError::ResponseMismatch);
                }
                self.state = AccountOperationState::Deleted(receipt);
            }
            (AccountTask::Report(_), TaskResult::Http(result)) => {
                self.state = AccountOperationState::Reported(decode_abuse_report_accepted(result)?);
            }
            (_, TaskResult::Signing(Err(error))) => {
                return Err(AccountCoordinatorError::Signing(error));
            }
            _ => return Err(AccountCoordinatorError::SigningResponseMismatch),
        }
        Ok(())
    }

    fn require_idle_or_terminal(&self) -> Result<(), AccountCoordinatorError> {
        if self.task.is_some()
            || matches!(
                self.state,
                AccountOperationState::Renaming
                    | AccountOperationState::Deleting
                    | AccountOperationState::Reporting
            )
        {
            Err(AccountCoordinatorError::Busy)
        } else {
            Ok(())
        }
    }
}

fn validate_signed_username(
    unsigned: &UsernameUpdateEnvelopeV1,
    signed: &UsernameUpdateEnvelopeV1,
) -> Result<(), AccountCoordinatorError> {
    signed
        .validate()
        .map_err(|error| AccountCoordinatorError::Signing(error.to_string()))?;
    let mut expected = signed.clone();
    expected.signature = Signature64::default();
    if &expected != unsigned {
        return Err(AccountCoordinatorError::SigningResponseMismatch);
    }
    Ok(())
}

fn validate_signed_deletion(
    unsigned: &DeletionRequestEnvelopeV1,
    signed: &DeletionRequestEnvelopeV1,
) -> Result<(), AccountCoordinatorError> {
    signed
        .validate()
        .map_err(|error| AccountCoordinatorError::Signing(error.to_string()))?;
    let mut expected = signed.clone();
    expected.signature = Signature64::default();
    if &expected != unsigned {
        return Err(AccountCoordinatorError::SigningResponseMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ImmediateTask(Option<Result<LeaderboardSigningResponse, String>>);

    impl LeaderboardSigningTask for ImmediateTask {
        fn try_take(&mut self) -> Option<Result<LeaderboardSigningResponse, String>> {
            self.0.take()
        }
    }

    struct TestSigner {
        public_key: PublicKey32,
        mutate_claim: bool,
    }

    impl LeaderboardAccountSigner for TestSigner {
        fn start(
            &mut self,
            request: LeaderboardSigningRequest,
        ) -> Result<Box<dyn LeaderboardSigningTask>, String> {
            let response = match request {
                LeaderboardSigningRequest::PublicKey => {
                    LeaderboardSigningResponse::PublicKey(self.public_key)
                }
                LeaderboardSigningRequest::UsernameUpdate(mut envelope) => {
                    if self.mutate_claim {
                        envelope.username.push_str("-changed");
                    }
                    envelope.signature = Signature64::from_bytes([9; 64]);
                    LeaderboardSigningResponse::UsernameUpdate(envelope)
                }
                LeaderboardSigningRequest::DeletionRequest(mut envelope) => {
                    envelope.signature = Signature64::from_bytes([9; 64]);
                    LeaderboardSigningResponse::DeletionRequest(envelope)
                }
            };
            Ok(Box::new(ImmediateTask(Some(Ok(response)))))
        }
    }

    #[test]
    fn typed_signer_cannot_substitute_username_claims() {
        let unsigned = UsernameUpdateEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            username_challenge_id: OpaqueId::new("challenge-1").unwrap(),
            username_challenge_nonce: robin_run_protocol::ChallengeNonce32::from_bytes([1; 32]),
            public_key: PublicKey32::from_bytes([2; 32]),
            username: "Robin".to_owned(),
            signature: Signature64::default(),
        };
        let mut changed = unsigned.clone();
        changed.username = "Marian".to_owned();
        changed.signature = Signature64::from_bytes([9; 64]);
        assert!(matches!(
            validate_signed_username(&unsigned, &changed),
            Err(AccountCoordinatorError::SigningResponseMismatch)
        ));
    }

    #[test]
    fn account_operations_begin_with_only_a_typed_public_key_request() {
        let api = LeaderboardApi::new(
            crate::leaderboard_preferences::LeaderboardApiBaseUrl::parse_development(
                "http://127.0.0.1:9/api/v1",
            )
            .unwrap(),
        );
        let signer = TestSigner {
            public_key: PublicKey32::from_bytes([3; 32]),
            mutate_claim: false,
        };
        let mut coordinator = LeaderboardAccountCoordinator::new(api, Box::new(signer));
        coordinator.rename("Robin".to_owned()).unwrap();
        assert_eq!(coordinator.state(), &AccountOperationState::Renaming);
        // First poll consumes the typed public-key task and starts HTTP. It
        // never asks the signer for generic bytes.
        assert!(coordinator.poll().unwrap());
        assert!(coordinator.poll().unwrap_or(false) == false);
    }

    #[test]
    fn owner_target_is_already_a_validated_opaque_id() {
        assert!(OpaqueId::new("bad\nidentifier").is_err());
        let target = OwnedLeaderboardTarget::Run {
            run_id: OpaqueId::new("run-1").unwrap(),
        };
        assert!(matches!(target, OwnedLeaderboardTarget::Run { .. }));
    }
}
