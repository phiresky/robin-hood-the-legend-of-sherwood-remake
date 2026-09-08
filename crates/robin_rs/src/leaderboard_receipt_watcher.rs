//! Durable, cooperative completion tracking for queued ranked submissions.
//!
//! `POST /submissions` only admits work to the verifier queue. This watcher
//! keeps the submission id and its exact controller identity after the
//! mission-end panel closes, obtains a fresh one-use owner challenge for each
//! status read, and polls at most one native/browser task per game frame.
//! Campaign-chain receipts are persisted only from an exact, authenticated
//! terminal `Accepted` response.

use crate::leaderboard_http::{HttpTask, HttpTransportError};
use crate::leaderboard_preferences::LeaderboardPreferences;
use crate::leaderboard_service::{
    LeaderboardApi, LeaderboardServiceError, decode_submission_owner_status,
    decode_submission_owner_status_challenge,
};
use robin_run_protocol::{
    CampaignChainReceiptV1, OpaqueId, PublicKey32, SCHEMA_VERSION_V1, SubmissionAcceptedV1,
    SubmissionLifecycleV1, SubmissionOwnerStatusChallengeRequestV1,
    SubmissionOwnerStatusChallengeV1, SubmissionOwnerStatusEnvelopeV1,
    SubmissionOwnerStatusResponseV1, Validate as _,
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;

const PENDING_STORE_FORMAT: u16 = 2;
const MAX_PENDING_SUBMISSIONS: usize = 32;
const MAX_STATUS_ATTEMPTS: u32 = 20_160;
const MAX_CONSECUTIVE_FAILURES: u16 = 12;
const MAX_PENDING_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const QUEUED_POLL_MS: u64 = 5_000;
const VERIFYING_POLL_MS: u64 = 2_000;
const RETRY_PENDING_POLL_MS: u64 = 15_000;
const MIN_TRANSIENT_BACKOFF_MS: u64 = 1_000;
const MAX_TRANSIENT_BACKOFF_MS: u64 = 5 * 60 * 1_000;
const STORAGE_RETRY_MS: u64 = 5_000;
const MAX_NOTICES: usize = 32;

#[cfg(not(target_arch = "wasm32"))]
const PENDING_STORE_FILE: &str = "leaderboard-pending-submissions.json";
#[cfg(target_arch = "wasm32")]
const BROWSER_PENDING_STORE_KEY: &str = "robin-hood.leaderboard-pending-submissions.v2";

/// Exact owner context which must survive the mission UI and process lifetime.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionReceiptWatchKey {
    pub submission_id: OpaqueId,
    pub controller_public_key: PublicKey32,
}

/// Durable handoff produced from the server's queue-admission response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueuedSubmissionReceiptWatch {
    pub key: SubmissionReceiptWatchKey,
    pub retry_after_ms: u64,
}

impl QueuedSubmissionReceiptWatch {
    pub fn from_accepted(
        accepted: &SubmissionAcceptedV1,
        controller_public_key: PublicKey32,
    ) -> Result<Self, ReceiptWatcherError> {
        accepted
            .validate()
            .map_err(|error| ReceiptWatcherError::InvalidHandoff(error.to_string()))?;
        if controller_public_key.is_zero() {
            return Err(ReceiptWatcherError::InvalidHandoff(
                "controller public key is zero".to_owned(),
            ));
        }
        Ok(Self {
            key: SubmissionReceiptWatchKey {
                submission_id: accepted.submission_id.clone(),
                controller_public_key,
            },
            retry_after_ms: accepted.retry_after_ms,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingSubmissionReceiptWatch {
    key: SubmissionReceiptWatchKey,
    enqueued_at_unix_ms: u64,
    next_attempt_at_unix_ms: u64,
    status_attempts: u32,
    consecutive_failures: u16,
    /// Authenticated terminal state is first committed to the pending store.
    /// A crash or campaign-store failure can then finish local persistence
    /// without another network response and without abandoning a verified run.
    terminal_accepted: Option<VerifiedTerminalReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifiedTerminalReceipt {
    run_id: OpaqueId,
    campaign_chain_receipt: Option<CampaignChainReceiptV1>,
}

impl PendingSubmissionReceiptWatch {
    fn validate(&self) -> Result<(), ReceiptWatcherError> {
        if self.key.controller_public_key.is_zero()
            || self.enqueued_at_unix_ms == 0
            || self.next_attempt_at_unix_ms < self.enqueued_at_unix_ms
            || self
                .next_attempt_at_unix_ms
                .saturating_sub(self.enqueued_at_unix_ms)
                > MAX_PENDING_AGE_MS
            || self.status_attempts > MAX_STATUS_ATTEMPTS
            || self.consecutive_failures > MAX_CONSECUTIVE_FAILURES
        {
            return Err(ReceiptWatcherError::InvalidStore(
                "pending submission has invalid bounds or owner context".to_owned(),
            ));
        }
        if let Some(terminal) = &self.terminal_accepted {
            let lifecycle = SubmissionLifecycleV1::Accepted {
                run_id: terminal.run_id.clone(),
                campaign_chain_receipt: terminal.campaign_chain_receipt.clone(),
            };
            lifecycle
                .validate()
                .map_err(|error| ReceiptWatcherError::InvalidStore(error.to_string()))?;
            if terminal
                .campaign_chain_receipt
                .as_ref()
                .is_some_and(|receipt| {
                    receipt.predecessor_run_id != terminal.run_id
                        || receipt.campaign_controller_public_key != self.key.controller_public_key
                })
            {
                return Err(ReceiptWatcherError::InvalidStore(
                    "terminal campaign receipt differs from its verified run or controller"
                        .to_owned(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingSubmissionReceiptStore {
    format: u16,
    pending: Vec<PendingSubmissionReceiptWatch>,
}

impl Default for PendingSubmissionReceiptStore {
    fn default() -> Self {
        Self {
            format: PENDING_STORE_FORMAT,
            pending: Vec::new(),
        }
    }
}

impl PendingSubmissionReceiptStore {
    fn validate(&self) -> Result<(), ReceiptWatcherError> {
        if self.format != PENDING_STORE_FORMAT {
            return Err(ReceiptWatcherError::InvalidStore(format!(
                "unsupported pending-submission store format {}",
                self.format
            )));
        }
        if self.pending.len() > MAX_PENDING_SUBMISSIONS {
            return Err(ReceiptWatcherError::InvalidStore(format!(
                "pending-submission store exceeds its {MAX_PENDING_SUBMISSIONS}-entry bound"
            )));
        }
        for pending in &self.pending {
            pending.validate()?;
        }
        if !self
            .pending
            .windows(2)
            .all(|pair| pair[0].key < pair[1].key)
        {
            return Err(ReceiptWatcherError::InvalidStore(
                "pending submissions are not uniquely sorted".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReceiptWatcherNotice {
    Verified {
        submission_id: OpaqueId,
        run_id: OpaqueId,
        campaign_receipt_persisted: bool,
    },
    Rejected {
        submission_id: OpaqueId,
        safe_message: String,
    },
    VerificationFailed {
        submission_id: OpaqueId,
        safe_message: String,
    },
    Abandoned {
        submission_id: OpaqueId,
        reason: String,
    },
    StorageUnavailable {
        reason: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ReceiptWatcherError {
    #[error("invalid queued-submission handoff: {0}")]
    InvalidHandoff(String),
    #[error("invalid durable pending-submission store: {0}")]
    InvalidStore(String),
    #[error("pending-submission watcher is full ({MAX_PENDING_SUBMISSIONS} entries)")]
    QueueFull,
    #[error("submission id is already tracked for a different owner")]
    OwnerConflict,
    #[error("pending-submission retry window exceeds the bounded watcher lifetime")]
    RetryWindowExceeded,
    #[error("failed to read pending-submission store from {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to decode pending-submission store from {path}: {source}")]
    Decode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to persist pending-submission store to {path}: {source}")]
    Persist {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[cfg(target_arch = "wasm32")]
    #[error("browser pending-submission storage is unavailable: {0}")]
    BrowserStorage(String),
    #[error("leaderboard watcher endpoint is unavailable: {0}")]
    Endpoint(String),
    #[error("system clock is before the Unix epoch")]
    InvalidClock,
}

/// Poll-task failures distinguish retryable infrastructure from a response or
/// identity claim which must fail closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum ReceiptWatcherOperationError {
    Transient(String),
    Permanent(String),
}

trait ReceiptWatcherTask<T>: Send {
    fn try_take(&mut self) -> Option<Result<T, ReceiptWatcherOperationError>>;
}

trait ReceiptWatcherBackend: Send {
    fn challenge(
        &mut self,
        request: SubmissionOwnerStatusChallengeRequestV1,
    ) -> Result<
        Box<dyn ReceiptWatcherTask<SubmissionOwnerStatusChallengeV1>>,
        ReceiptWatcherOperationError,
    >;

    fn sign(
        &mut self,
        challenge: SubmissionOwnerStatusChallengeV1,
    ) -> Result<
        Box<dyn ReceiptWatcherTask<SubmissionOwnerStatusEnvelopeV1>>,
        ReceiptWatcherOperationError,
    >;

    fn status(
        &mut self,
        envelope: SubmissionOwnerStatusEnvelopeV1,
    ) -> Result<
        Box<dyn ReceiptWatcherTask<SubmissionOwnerStatusResponseV1>>,
        ReceiptWatcherOperationError,
    >;
}

trait ReceiptWatcherPersistence: Send {
    fn persist_pending(
        &mut self,
        store: &PendingSubmissionReceiptStore,
    ) -> Result<(), ReceiptWatcherError>;

    fn persist_campaign_receipt(
        &mut self,
        receipt: CampaignChainReceiptV1,
    ) -> Result<(), ReceiptWatcherError>;
}

enum ActiveReceiptWatcherTask {
    Challenge {
        key: SubmissionReceiptWatchKey,
        request: SubmissionOwnerStatusChallengeRequestV1,
        task: Box<dyn ReceiptWatcherTask<SubmissionOwnerStatusChallengeV1>>,
    },
    Sign {
        key: SubmissionReceiptWatchKey,
        challenge: SubmissionOwnerStatusChallengeV1,
        task: Box<dyn ReceiptWatcherTask<SubmissionOwnerStatusEnvelopeV1>>,
    },
    Status {
        key: SubmissionReceiptWatchKey,
        envelope: SubmissionOwnerStatusEnvelopeV1,
        task: Box<dyn ReceiptWatcherTask<SubmissionOwnerStatusResponseV1>>,
    },
}

/// Application-lifetime owner for all durable queued submissions.
pub struct SubmissionReceiptWatcher {
    store: PendingSubmissionReceiptStore,
    backend: Box<dyn ReceiptWatcherBackend>,
    persistence: Box<dyn ReceiptWatcherPersistence>,
    active: Option<ActiveReceiptWatcherTask>,
    notices: VecDeque<ReceiptWatcherNotice>,
    storage_retry_not_before_unix_ms: u64,
}

impl std::fmt::Debug for SubmissionReceiptWatcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SubmissionReceiptWatcher")
            .field("pending", &self.store.pending.len())
            .field("active", &self.active.is_some())
            .field("notices", &self.notices.len())
            .field(
                "storage_retry_not_before_unix_ms",
                &self.storage_retry_not_before_unix_ms,
            )
            .finish_non_exhaustive()
    }
}

/// Application-owned bridge around the durable watcher.
///
/// The bridge is intentionally independent from mission presentation. It is
/// kept in `ApplicationContext`, so a queued upload can be handed off before
/// its panel closes and status polling continues through mission transitions
/// and the main menu. The pending store remains the process-crash boundary.
#[derive(Default)]
pub(crate) struct ApplicationReceiptWatcher {
    watcher: Option<SubmissionReceiptWatcher>,
    retry_initialization_not_before_unix_ms: u64,
    last_initialization_error: Option<String>,
}

impl std::fmt::Debug for ApplicationReceiptWatcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApplicationReceiptWatcher")
            .field("watcher", &self.watcher)
            .field(
                "retry_initialization_not_before_unix_ms",
                &self.retry_initialization_not_before_unix_ms,
            )
            .field("last_initialization_error", &self.last_initialization_error)
            .finish()
    }
}

impl ApplicationReceiptWatcher {
    const INITIALIZATION_RETRY_MS: u64 = 5_000;

    /// Persist an upload handoff before the mission controller is allowed to
    /// retire. Initialization is retried immediately here: a prior transient
    /// read/API failure must not make a newly accepted submission lossy.
    pub(crate) fn enqueue(
        &mut self,
        handoff: QueuedSubmissionReceiptWatch,
        now_unix_ms: u64,
    ) -> Result<bool, ReceiptWatcherError> {
        self.ensure_loaded()?;
        self.last_initialization_error = None;
        self.retry_initialization_not_before_unix_ms = 0;
        self.watcher
            .as_mut()
            .expect("successful watcher initialization installs an owner")
            .enqueue(handoff, now_unix_ms)
    }

    /// Advance at most one status/signing task. The underlying watcher retains
    /// bounded diagnostic notices alongside the durable submission state.
    pub(crate) fn poll(&mut self, now_unix_ms: u64) -> Result<(), ReceiptWatcherError> {
        if self.watcher.is_none() {
            if now_unix_ms < self.retry_initialization_not_before_unix_ms {
                return Ok(());
            }
            if let Err(error) = self.ensure_loaded() {
                self.retry_initialization_not_before_unix_ms =
                    now_unix_ms.saturating_add(Self::INITIALIZATION_RETRY_MS);
                let detail = error.to_string();
                if self.last_initialization_error.as_deref() != Some(detail.as_str()) {
                    tracing::error!("leaderboard receipt watcher initialization failed: {detail}");
                }
                self.last_initialization_error = Some(detail);
                return Err(error);
            }
        }
        self.last_initialization_error = None;
        self.retry_initialization_not_before_unix_ms = 0;
        let watcher = self
            .watcher
            .as_mut()
            .expect("successful watcher initialization installs an owner");
        watcher.poll(now_unix_ms);
        // TODO: connect receipt notices to an actual presentation consumer;
        // do not duplicate the watcher's bounded queue in this owner.
        Ok(())
    }

    fn ensure_loaded(&mut self) -> Result<(), ReceiptWatcherError> {
        if self.watcher.is_some() {
            return Ok(());
        }
        let preferences = crate::leaderboard_preferences::load()
            .map_err(|error| ReceiptWatcherError::Endpoint(error.to_string()))?;
        self.watcher = Some(SubmissionReceiptWatcher::load_production(&preferences)?);
        Ok(())
    }
}

impl SubmissionReceiptWatcher {
    pub fn load_production(
        preferences: &LeaderboardPreferences,
    ) -> Result<Self, ReceiptWatcherError> {
        let store = load_pending_store()?;
        let api = LeaderboardApi::from_preferences(preferences)
            .map_err(|error| ReceiptWatcherError::Endpoint(error.to_string()))?;
        Ok(Self::with_parts(
            store,
            Box::new(HttpReceiptWatcherBackend { api }),
            Box::new(DurableReceiptWatcherPersistence),
        ))
    }

    fn with_parts(
        store: PendingSubmissionReceiptStore,
        backend: Box<dyn ReceiptWatcherBackend>,
        persistence: Box<dyn ReceiptWatcherPersistence>,
    ) -> Self {
        store
            .validate()
            .expect("receipt watcher must be constructed from a validated store");
        Self {
            store,
            backend,
            persistence,
            active: None,
            notices: VecDeque::new(),
            storage_retry_not_before_unix_ms: 0,
        }
    }

    /// Persist a queue admission before the presentation owner may discard it.
    /// Returns false for an exact idempotent handoff.
    pub fn enqueue(
        &mut self,
        handoff: QueuedSubmissionReceiptWatch,
        now_unix_ms: u64,
    ) -> Result<bool, ReceiptWatcherError> {
        if now_unix_ms == 0 || handoff.retry_after_ms == 0 {
            return Err(ReceiptWatcherError::InvalidHandoff(
                "enqueue time and retry interval must be positive".to_owned(),
            ));
        }
        if handoff.key.controller_public_key.is_zero() {
            return Err(ReceiptWatcherError::InvalidHandoff(
                "controller public key is zero".to_owned(),
            ));
        }
        if let Some(existing) = self
            .store
            .pending
            .iter()
            .find(|pending| pending.key.submission_id == handoff.key.submission_id)
        {
            if existing.key == handoff.key {
                return Ok(false);
            }
            return Err(ReceiptWatcherError::OwnerConflict);
        }
        if self.store.pending.len() == MAX_PENDING_SUBMISSIONS {
            return Err(ReceiptWatcherError::QueueFull);
        }
        let next_attempt_at_unix_ms = now_unix_ms
            .checked_add(handoff.retry_after_ms)
            .filter(|next| next.saturating_sub(now_unix_ms) <= MAX_PENDING_AGE_MS)
            .ok_or(ReceiptWatcherError::RetryWindowExceeded)?;
        let mut updated = self.store.clone();
        updated.pending.push(PendingSubmissionReceiptWatch {
            key: handoff.key,
            enqueued_at_unix_ms: now_unix_ms,
            next_attempt_at_unix_ms,
            status_attempts: 0,
            consecutive_failures: 0,
            terminal_accepted: None,
        });
        updated
            .pending
            .sort_by(|left, right| left.key.cmp(&right.key));
        self.commit_pending(updated)?;
        Ok(true)
    }

    pub fn enqueue_accepted(
        &mut self,
        accepted: &SubmissionAcceptedV1,
        controller_public_key: PublicKey32,
        now_unix_ms: u64,
    ) -> Result<bool, ReceiptWatcherError> {
        self.enqueue(
            QueuedSubmissionReceiptWatch::from_accepted(accepted, controller_public_key)?,
            now_unix_ms,
        )
    }

    pub fn pending_count(&self) -> usize {
        self.store.pending.len()
    }

    pub fn is_tracking(&self, key: &SubmissionReceiptWatchKey) -> bool {
        self.store.pending.iter().any(|pending| &pending.key == key)
    }

    pub fn take_notice(&mut self) -> Option<ReceiptWatcherNotice> {
        self.notices.pop_front()
    }

    /// Advance at most one already-running task, or start one due challenge.
    /// No network wait or identity operation runs on the calling frame.
    pub fn poll(&mut self, now_unix_ms: u64) {
        if now_unix_ms == 0 || now_unix_ms < self.storage_retry_not_before_unix_ms {
            return;
        }
        let Some(active) = self.active.take() else {
            self.start_due(now_unix_ms);
            return;
        };
        match active {
            ActiveReceiptWatcherTask::Challenge {
                key,
                request,
                mut task,
            } => match task.try_take() {
                None => {
                    self.active = Some(ActiveReceiptWatcherTask::Challenge { key, request, task });
                }
                Some(Ok(challenge)) => {
                    if let Err(error) = validate_challenge(&request, &challenge) {
                        self.record_operation_error(
                            &key,
                            ReceiptWatcherOperationError::Permanent(error),
                            now_unix_ms,
                        );
                    } else {
                        match self.backend.sign(challenge.clone()) {
                            Ok(task) => {
                                self.active = Some(ActiveReceiptWatcherTask::Sign {
                                    key,
                                    challenge,
                                    task,
                                });
                            }
                            Err(error) => self.record_operation_error(&key, error, now_unix_ms),
                        }
                    }
                }
                Some(Err(error)) => self.record_operation_error(&key, error, now_unix_ms),
            },
            ActiveReceiptWatcherTask::Sign {
                key,
                challenge,
                mut task,
            } => match task.try_take() {
                None => {
                    self.active = Some(ActiveReceiptWatcherTask::Sign {
                        key,
                        challenge,
                        task,
                    });
                }
                Some(Ok(envelope)) => {
                    if let Err(error) = validate_envelope(&challenge, &envelope) {
                        self.record_operation_error(
                            &key,
                            ReceiptWatcherOperationError::Permanent(error),
                            now_unix_ms,
                        );
                    } else {
                        match self.backend.status(envelope.clone()) {
                            Ok(task) => {
                                self.active = Some(ActiveReceiptWatcherTask::Status {
                                    key,
                                    envelope,
                                    task,
                                });
                            }
                            Err(error) => self.record_operation_error(&key, error, now_unix_ms),
                        }
                    }
                }
                Some(Err(error)) => self.record_operation_error(&key, error, now_unix_ms),
            },
            ActiveReceiptWatcherTask::Status {
                key,
                envelope,
                mut task,
            } => match task.try_take() {
                None => {
                    self.active = Some(ActiveReceiptWatcherTask::Status {
                        key,
                        envelope,
                        task,
                    });
                }
                Some(Ok(response)) => {
                    if let Err(error) = response.validate_against_envelope(&envelope) {
                        self.record_operation_error(
                            &key,
                            ReceiptWatcherOperationError::Permanent(format!(
                                "owner-status response mismatch: {error}"
                            )),
                            now_unix_ms,
                        );
                    } else {
                        self.handle_status(key, response, now_unix_ms);
                    }
                }
                Some(Err(error)) => self.record_operation_error(&key, error, now_unix_ms),
            },
        }
    }

    fn start_due(&mut self, now_unix_ms: u64) {
        if let Some(key) = self
            .store
            .pending
            .iter()
            .find(|pending| pending.terminal_accepted.is_some())
            .map(|pending| pending.key.clone())
        {
            self.finalize_terminal(&key, now_unix_ms);
            return;
        }
        let expired = self.store.pending.iter().find(|pending| {
            pending.terminal_accepted.is_none()
                && (now_unix_ms.saturating_sub(pending.enqueued_at_unix_ms) > MAX_PENDING_AGE_MS
                    || pending.status_attempts >= MAX_STATUS_ATTEMPTS)
        });
        if let Some(expired) = expired {
            let key = expired.key.clone();
            self.abandon(
                &key,
                "bounded verification-watch lifetime was exhausted",
                now_unix_ms,
            );
            return;
        }
        let Some(key) = self
            .store
            .pending
            .iter()
            .filter(|pending| pending.next_attempt_at_unix_ms <= now_unix_ms)
            .min_by_key(|pending| pending.next_attempt_at_unix_ms)
            .map(|pending| pending.key.clone())
        else {
            return;
        };
        let mut updated = self.store.clone();
        let pending = find_pending_mut(&mut updated, &key)
            .expect("selected pending receipt watch must remain present");
        pending.status_attempts = pending.status_attempts.saturating_add(1);
        if let Err(error) = self.commit_pending(updated) {
            self.note_storage_failure(error, now_unix_ms);
            return;
        }
        let request = SubmissionOwnerStatusChallengeRequestV1 {
            schema_version: SCHEMA_VERSION_V1,
            controller_public_key: key.controller_public_key,
            submission_id: key.submission_id.clone(),
        };
        if let Err(error) = request.validate() {
            self.record_operation_error(
                &key,
                ReceiptWatcherOperationError::Permanent(format!(
                    "stored owner-status request is invalid: {error}"
                )),
                now_unix_ms,
            );
            return;
        }
        match self.backend.challenge(request.clone()) {
            Ok(task) => {
                self.active = Some(ActiveReceiptWatcherTask::Challenge { key, request, task });
            }
            Err(error) => self.record_operation_error(&key, error, now_unix_ms),
        }
    }

    fn handle_status(
        &mut self,
        key: SubmissionReceiptWatchKey,
        response: SubmissionOwnerStatusResponseV1,
        now_unix_ms: u64,
    ) {
        match response.state {
            SubmissionLifecycleV1::Queued => {
                self.schedule_lifecycle(&key, QUEUED_POLL_MS, now_unix_ms)
            }
            SubmissionLifecycleV1::Verifying => {
                self.schedule_lifecycle(&key, VERIFYING_POLL_MS, now_unix_ms)
            }
            SubmissionLifecycleV1::RetryPending => {
                self.schedule_lifecycle(&key, RETRY_PENDING_POLL_MS, now_unix_ms)
            }
            SubmissionLifecycleV1::Accepted {
                run_id,
                campaign_chain_receipt,
            } => {
                let mut updated = self.store.clone();
                let Some(pending) = find_pending_mut(&mut updated, &key) else {
                    return;
                };
                pending.terminal_accepted = Some(VerifiedTerminalReceipt {
                    run_id,
                    campaign_chain_receipt,
                });
                pending.consecutive_failures = 0;
                if let Err(error) = self.commit_pending(updated) {
                    self.note_storage_failure(error, now_unix_ms);
                    return;
                }
                self.finalize_terminal(&key, now_unix_ms);
            }
            SubmissionLifecycleV1::Rejected { safe_message, .. } => {
                if self.remove_pending(&key, now_unix_ms) {
                    self.push_notice(ReceiptWatcherNotice::Rejected {
                        submission_id: key.submission_id,
                        safe_message,
                    });
                }
            }
            SubmissionLifecycleV1::Failed { safe_message, .. } => {
                if self.remove_pending(&key, now_unix_ms) {
                    self.push_notice(ReceiptWatcherNotice::VerificationFailed {
                        submission_id: key.submission_id,
                        safe_message,
                    });
                }
            }
        }
    }

    fn finalize_terminal(&mut self, key: &SubmissionReceiptWatchKey, now_unix_ms: u64) {
        let Some(terminal) = self
            .store
            .pending
            .iter()
            .find(|pending| &pending.key == key)
            .and_then(|pending| pending.terminal_accepted.clone())
        else {
            return;
        };
        let persisted_receipt = terminal.campaign_chain_receipt.is_some();
        if let Some(receipt) = terminal.campaign_chain_receipt
            && let Err(error) = self.persistence.persist_campaign_receipt(receipt)
        {
            self.note_storage_failure(error, now_unix_ms);
            return;
        }
        if self.remove_pending(key, now_unix_ms) {
            self.push_notice(ReceiptWatcherNotice::Verified {
                submission_id: key.submission_id.clone(),
                run_id: terminal.run_id,
                campaign_receipt_persisted: persisted_receipt,
            });
        }
    }

    fn schedule_lifecycle(
        &mut self,
        key: &SubmissionReceiptWatchKey,
        delay_ms: u64,
        now_unix_ms: u64,
    ) {
        let mut updated = self.store.clone();
        let Some(pending) = find_pending_mut(&mut updated, key) else {
            return;
        };
        pending.consecutive_failures = 0;
        pending.next_attempt_at_unix_ms = bounded_next_attempt(pending, now_unix_ms, delay_ms);
        if let Err(error) = self.commit_pending(updated) {
            self.note_storage_failure(error, now_unix_ms);
        }
    }

    fn record_operation_error(
        &mut self,
        key: &SubmissionReceiptWatchKey,
        error: ReceiptWatcherOperationError,
        now_unix_ms: u64,
    ) {
        match error {
            ReceiptWatcherOperationError::Permanent(reason) => {
                self.abandon(key, &reason, now_unix_ms);
            }
            ReceiptWatcherOperationError::Transient(reason) => {
                let mut updated = self.store.clone();
                let Some(pending) = find_pending_mut(&mut updated, key) else {
                    return;
                };
                pending.consecutive_failures = pending.consecutive_failures.saturating_add(1);
                if pending.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    self.abandon(
                        key,
                        &format!("transient retry limit exhausted: {reason}"),
                        now_unix_ms,
                    );
                    return;
                }
                let exponent = u32::from(pending.consecutive_failures.saturating_sub(1)).min(18);
                let delay = MIN_TRANSIENT_BACKOFF_MS
                    .saturating_mul(1_u64 << exponent)
                    .min(MAX_TRANSIENT_BACKOFF_MS);
                pending.next_attempt_at_unix_ms = bounded_next_attempt(pending, now_unix_ms, delay);
                if let Err(error) = self.commit_pending(updated) {
                    self.note_storage_failure(error, now_unix_ms);
                } else {
                    tracing::warn!(
                        submission_id = %key.submission_id,
                        failures = pending_failure_count(&self.store, key),
                        "leaderboard verification status will retry: {reason}"
                    );
                }
            }
        }
    }

    fn abandon(&mut self, key: &SubmissionReceiptWatchKey, reason: &str, now_unix_ms: u64) {
        if self.remove_pending(key, now_unix_ms) {
            self.push_notice(ReceiptWatcherNotice::Abandoned {
                submission_id: key.submission_id.clone(),
                reason: reason.to_owned(),
            });
        }
    }

    fn remove_pending(&mut self, key: &SubmissionReceiptWatchKey, now_unix_ms: u64) -> bool {
        let mut updated = self.store.clone();
        let before = updated.pending.len();
        updated.pending.retain(|pending| &pending.key != key);
        if updated.pending.len() == before {
            return false;
        }
        match self.commit_pending(updated) {
            Ok(()) => true,
            Err(error) => {
                self.note_storage_failure(error, now_unix_ms);
                false
            }
        }
    }

    fn commit_pending(
        &mut self,
        updated: PendingSubmissionReceiptStore,
    ) -> Result<(), ReceiptWatcherError> {
        updated.validate()?;
        self.persistence.persist_pending(&updated)?;
        self.store = updated;
        Ok(())
    }

    fn note_storage_failure(&mut self, error: ReceiptWatcherError, now_unix_ms: u64) {
        self.storage_retry_not_before_unix_ms = now_unix_ms.saturating_add(STORAGE_RETRY_MS);
        tracing::error!("leaderboard receipt watcher storage failed: {error}");
        self.push_notice(ReceiptWatcherNotice::StorageUnavailable {
            reason: error.to_string(),
        });
    }

    fn push_notice(&mut self, notice: ReceiptWatcherNotice) {
        if self.notices.len() == MAX_NOTICES {
            self.notices.pop_front();
        }
        self.notices.push_back(notice);
    }
}

fn find_pending_mut<'a>(
    store: &'a mut PendingSubmissionReceiptStore,
    key: &SubmissionReceiptWatchKey,
) -> Option<&'a mut PendingSubmissionReceiptWatch> {
    store.pending.iter_mut().find(|pending| &pending.key == key)
}

fn pending_failure_count(
    store: &PendingSubmissionReceiptStore,
    key: &SubmissionReceiptWatchKey,
) -> u16 {
    store
        .pending
        .iter()
        .find(|pending| &pending.key == key)
        .map_or(0, |pending| pending.consecutive_failures)
}

fn bounded_next_attempt(
    pending: &PendingSubmissionReceiptWatch,
    now_unix_ms: u64,
    delay_ms: u64,
) -> u64 {
    let deadline = pending
        .enqueued_at_unix_ms
        .saturating_add(MAX_PENDING_AGE_MS);
    now_unix_ms.saturating_add(delay_ms).min(deadline)
}

fn validate_challenge(
    request: &SubmissionOwnerStatusChallengeRequestV1,
    challenge: &SubmissionOwnerStatusChallengeV1,
) -> Result<(), String> {
    challenge
        .validate()
        .map_err(|error| format!("owner-status challenge is invalid: {error}"))?;
    if challenge.controller_public_key != request.controller_public_key
        || challenge.submission_id != request.submission_id
    {
        return Err("owner-status challenge changed the submission or owner".to_owned());
    }
    Ok(())
}

fn validate_envelope(
    challenge: &SubmissionOwnerStatusChallengeV1,
    envelope: &SubmissionOwnerStatusEnvelopeV1,
) -> Result<(), String> {
    envelope
        .validate()
        .map_err(|error| format!("signed owner-status envelope is invalid: {error}"))?;
    if &envelope.challenge != challenge {
        return Err("identity signer changed the owner-status challenge".to_owned());
    }
    Ok(())
}

struct DurableReceiptWatcherPersistence;

impl ReceiptWatcherPersistence for DurableReceiptWatcherPersistence {
    fn persist_pending(
        &mut self,
        store: &PendingSubmissionReceiptStore,
    ) -> Result<(), ReceiptWatcherError> {
        persist_pending_store(store)
    }

    fn persist_campaign_receipt(
        &mut self,
        receipt: CampaignChainReceiptV1,
    ) -> Result<(), ReceiptWatcherError> {
        let mut store = crate::leaderboard_chains::load()
            .map_err(|error| ReceiptWatcherError::InvalidStore(error.to_string()))?;
        store
            .accepted(receipt)
            .map_err(|error| ReceiptWatcherError::InvalidStore(error.to_string()))?;
        crate::leaderboard_chains::persist(&store)
            .map_err(|error| ReceiptWatcherError::InvalidStore(error.to_string()))
    }
}

struct HttpReceiptWatcherBackend {
    api: LeaderboardApi,
}

struct ChallengeHttpTask {
    task: HttpTask,
    request: SubmissionOwnerStatusChallengeRequestV1,
}

impl ReceiptWatcherTask<SubmissionOwnerStatusChallengeV1> for ChallengeHttpTask {
    fn try_take(
        &mut self,
    ) -> Option<Result<SubmissionOwnerStatusChallengeV1, ReceiptWatcherOperationError>> {
        self.task.try_take().map(|result| {
            decode_submission_owner_status_challenge(result, &self.request)
                .map_err(classify_service_error)
        })
    }
}

struct StatusHttpTask {
    task: HttpTask,
    envelope: SubmissionOwnerStatusEnvelopeV1,
}

impl ReceiptWatcherTask<SubmissionOwnerStatusResponseV1> for StatusHttpTask {
    fn try_take(
        &mut self,
    ) -> Option<Result<SubmissionOwnerStatusResponseV1, ReceiptWatcherOperationError>> {
        self.task.try_take().map(|result| {
            decode_submission_owner_status(result, &self.envelope).map_err(classify_service_error)
        })
    }
}

struct SigningTask {
    receiver: async_channel::Receiver<
        Result<SubmissionOwnerStatusEnvelopeV1, ReceiptWatcherOperationError>,
    >,
}

impl ReceiptWatcherTask<SubmissionOwnerStatusEnvelopeV1> for SigningTask {
    fn try_take(
        &mut self,
    ) -> Option<Result<SubmissionOwnerStatusEnvelopeV1, ReceiptWatcherOperationError>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(async_channel::TryRecvError::Empty) => None,
            Err(async_channel::TryRecvError::Closed) => {
                Some(Err(ReceiptWatcherOperationError::Transient(
                    "identity signing worker closed without a result".to_owned(),
                )))
            }
        }
    }
}

impl ReceiptWatcherBackend for HttpReceiptWatcherBackend {
    fn challenge(
        &mut self,
        request: SubmissionOwnerStatusChallengeRequestV1,
    ) -> Result<
        Box<dyn ReceiptWatcherTask<SubmissionOwnerStatusChallengeV1>>,
        ReceiptWatcherOperationError,
    > {
        let task = self
            .api
            .submission_owner_status_challenge(&request)
            .map_err(classify_service_error)?;
        Ok(Box::new(ChallengeHttpTask { task, request }))
    }

    fn sign(
        &mut self,
        challenge: SubmissionOwnerStatusChallengeV1,
    ) -> Result<
        Box<dyn ReceiptWatcherTask<SubmissionOwnerStatusEnvelopeV1>>,
        ReceiptWatcherOperationError,
    > {
        let (sender, receiver) = async_channel::bounded(1);
        spawn_signing(challenge, sender)?;
        Ok(Box::new(SigningTask { receiver }))
    }

    fn status(
        &mut self,
        envelope: SubmissionOwnerStatusEnvelopeV1,
    ) -> Result<
        Box<dyn ReceiptWatcherTask<SubmissionOwnerStatusResponseV1>>,
        ReceiptWatcherOperationError,
    > {
        let task = self
            .api
            .submission_owner_status(&envelope)
            .map_err(classify_service_error)?;
        Ok(Box::new(StatusHttpTask { task, envelope }))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_signing(
    challenge: SubmissionOwnerStatusChallengeV1,
    sender: async_channel::Sender<
        Result<SubmissionOwnerStatusEnvelopeV1, ReceiptWatcherOperationError>,
    >,
) -> Result<(), ReceiptWatcherOperationError> {
    std::thread::Builder::new()
        .name("leaderboard-owner-status-sign".to_owned())
        .spawn(move || {
            let result = crate::leaderboard_signing::sign_submission_owner_status(challenge)
                .map_err(classify_signing_error);
            let _ = sender.send_blocking(result);
        })
        .map(|_| ())
        .map_err(|error| {
            ReceiptWatcherOperationError::Transient(format!(
                "failed to start identity signing worker: {error}"
            ))
        })
}

#[cfg(target_arch = "wasm32")]
fn spawn_signing(
    challenge: SubmissionOwnerStatusChallengeV1,
    sender: async_channel::Sender<
        Result<SubmissionOwnerStatusEnvelopeV1, ReceiptWatcherOperationError>,
    >,
) -> Result<(), ReceiptWatcherOperationError> {
    wasm_bindgen_futures::spawn_local(async move {
        let result =
            crate::leaderboard_signing::browser_game_sign_submission_owner_status(&challenge)
                .await
                .map_err(classify_signing_error);
        let _ = sender.send(result).await;
    });
    Ok(())
}

fn classify_signing_error(
    error: crate::leaderboard_signing::LeaderboardSigningError,
) -> ReceiptWatcherOperationError {
    use crate::leaderboard_signing::LeaderboardSigningError as Error;
    match error {
        Error::Identity(_) => ReceiptWatcherOperationError::Transient(error.to_string()),
        Error::WrongIdentity
        | Error::IdentityNotClaimed
        | Error::InvalidClaim(_)
        | Error::Canonical(_)
        | Error::DocumentTooLarge { .. }
        | Error::InvalidJson(_)
        | Error::OriginNotAuthorized
        | Error::SignerContext => ReceiptWatcherOperationError::Permanent(error.to_string()),
    }
}

fn classify_service_error(error: LeaderboardServiceError) -> ReceiptWatcherOperationError {
    use LeaderboardServiceError as Error;
    match &error {
        Error::Transport(transport) if transient_transport(transport) => {
            ReceiptWatcherOperationError::Transient(error.to_string())
        }
        Error::HttpStatus { status }
            if matches!(*status, 408 | 425 | 429) || (500..600).contains(status) =>
        {
            ReceiptWatcherOperationError::Transient(error.to_string())
        }
        Error::Endpoint(_)
        | Error::InvalidJson(_)
        | Error::InvalidProtocol(_)
        | Error::BoardFilterMismatch
        | Error::BoardCursorMismatch
        | Error::ArtifactMismatch
        | Error::MissingStartingCampaign
        | Error::InvalidCompactReplay(_)
        | Error::UnexpectedContentType { .. }
        | Error::RequestEncoding(_)
        | Error::HttpStatus { .. }
        | Error::Transport(_) => ReceiptWatcherOperationError::Permanent(error.to_string()),
    }
}

fn transient_transport(error: &HttpTransportError) -> bool {
    matches!(
        error,
        HttpTransportError::Timeout
            | HttpTransportError::Request(_)
            | HttpTransportError::TooManyInFlight
            | HttpTransportError::WorkerClosed
    )
}

pub fn now_unix_ms() -> Result<u64, ReceiptWatcherError> {
    web_time::SystemTime::now()
        .duration_since(web_time::SystemTime::UNIX_EPOCH)
        .map_err(|_| ReceiptWatcherError::InvalidClock)
        .and_then(|duration| {
            u64::try_from(duration.as_millis()).map_err(|_| ReceiptWatcherError::InvalidClock)
        })
}

fn pending_store_path() -> PathBuf {
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::save_file::default_save_directory().join(PENDING_STORE_FILE)
    }
    #[cfg(target_arch = "wasm32")]
    {
        PathBuf::from(BROWSER_PENDING_STORE_KEY)
    }
}

fn load_pending_store() -> Result<PendingSubmissionReceiptStore, ReceiptWatcherError> {
    let Some(encoded) = read_pending_store()? else {
        return Ok(PendingSubmissionReceiptStore::default());
    };
    let store =
        serde_json::from_str::<PendingSubmissionReceiptStore>(&encoded).map_err(|source| {
            ReceiptWatcherError::Decode {
                path: pending_store_path(),
                source,
            }
        })?;
    store.validate()?;
    Ok(store)
}

fn persist_pending_store(store: &PendingSubmissionReceiptStore) -> Result<(), ReceiptWatcherError> {
    store.validate()?;
    let encoded = serde_json::to_vec_pretty(store)
        .expect("validated pending-submission store must serialize");
    write_pending_store(&encoded)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_pending_store() -> Result<Option<String>, ReceiptWatcherError> {
    let path = pending_store_path();
    crate::leaderboard_storage::read_private_utf8(&path)
        .map_err(|source| ReceiptWatcherError::Read { path, source })
}

#[cfg(not(target_arch = "wasm32"))]
fn write_pending_store(encoded: &[u8]) -> Result<(), ReceiptWatcherError> {
    let path = pending_store_path();
    crate::leaderboard_storage::replace_private(&path, ".leaderboard-pending-", encoded)
        .map_err(|source| ReceiptWatcherError::Persist { path, source })
}

#[cfg(target_arch = "wasm32")]
fn read_pending_store() -> Result<Option<String>, ReceiptWatcherError> {
    browser_storage()?
        .get_item(BROWSER_PENDING_STORE_KEY)
        .map_err(|error| ReceiptWatcherError::BrowserStorage(format!("{error:?}")))
}

#[cfg(target_arch = "wasm32")]
fn write_pending_store(encoded: &[u8]) -> Result<(), ReceiptWatcherError> {
    let encoded = std::str::from_utf8(encoded)
        .expect("serialized pending-submission store must be valid UTF-8");
    browser_storage()?
        .set_item(BROWSER_PENDING_STORE_KEY, encoded)
        .map_err(|error| ReceiptWatcherError::BrowserStorage(format!("{error:?}")))
}

#[cfg(target_arch = "wasm32")]
fn browser_storage() -> Result<web_sys::Storage, ReceiptWatcherError> {
    web_sys::window()
        .ok_or_else(|| ReceiptWatcherError::BrowserStorage("window is absent".to_owned()))?
        .local_storage()
        .map_err(|error| ReceiptWatcherError::BrowserStorage(format!("{error:?}")))?
        .ok_or_else(|| ReceiptWatcherError::BrowserStorage("localStorage is disabled".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_run_protocol::{
        ArtifactRefV1, CampaignChainStateV1, CanonicalDocument as _, ChallengeNonce32, Digest32,
        RANKED_CAMPAIGN_MEDIA_TYPE_V1, Signature64, SignatureAlgorithmV1, SubmissionFailureCodeV1,
        VerificationRejectionCodeV1,
    };
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    const NOW: u64 = 1_800_000_000_000;

    #[derive(Debug, Clone, Default)]
    struct PersistenceState {
        stores: Vec<PendingSubmissionReceiptStore>,
        receipts: Vec<CampaignChainReceiptV1>,
    }

    struct MemoryPersistence(Arc<Mutex<PersistenceState>>);

    impl ReceiptWatcherPersistence for MemoryPersistence {
        fn persist_pending(
            &mut self,
            store: &PendingSubmissionReceiptStore,
        ) -> Result<(), ReceiptWatcherError> {
            self.0.lock().unwrap().stores.push(store.clone());
            Ok(())
        }

        fn persist_campaign_receipt(
            &mut self,
            receipt: CampaignChainReceiptV1,
        ) -> Result<(), ReceiptWatcherError> {
            self.0.lock().unwrap().receipts.push(receipt);
            Ok(())
        }
    }

    struct ReadyTask<T>(Option<Result<T, ReceiptWatcherOperationError>>);

    impl<T: Send> ReceiptWatcherTask<T> for ReadyTask<T> {
        fn try_take(&mut self) -> Option<Result<T, ReceiptWatcherOperationError>> {
            self.0.take()
        }
    }

    enum BackendStep {
        Challenge(SubmissionOwnerStatusChallengeV1),
        Envelope(SubmissionOwnerStatusEnvelopeV1),
        Status(SubmissionOwnerStatusResponseV1),
        Error(ReceiptWatcherOperationError),
    }

    struct ScriptedBackend {
        steps: VecDeque<BackendStep>,
    }

    impl ScriptedBackend {
        fn next(&mut self) -> BackendStep {
            self.steps
                .pop_front()
                .expect("test backend ran out of scripted steps")
        }
    }

    impl ReceiptWatcherBackend for ScriptedBackend {
        fn challenge(
            &mut self,
            _request: SubmissionOwnerStatusChallengeRequestV1,
        ) -> Result<
            Box<dyn ReceiptWatcherTask<SubmissionOwnerStatusChallengeV1>>,
            ReceiptWatcherOperationError,
        > {
            match self.next() {
                BackendStep::Challenge(value) => Ok(Box::new(ReadyTask(Some(Ok(value))))),
                BackendStep::Error(error) => Err(error),
                _ => panic!("expected challenge step"),
            }
        }

        fn sign(
            &mut self,
            _challenge: SubmissionOwnerStatusChallengeV1,
        ) -> Result<
            Box<dyn ReceiptWatcherTask<SubmissionOwnerStatusEnvelopeV1>>,
            ReceiptWatcherOperationError,
        > {
            match self.next() {
                BackendStep::Envelope(value) => Ok(Box::new(ReadyTask(Some(Ok(value))))),
                BackendStep::Error(error) => Err(error),
                _ => panic!("expected envelope step"),
            }
        }

        fn status(
            &mut self,
            _envelope: SubmissionOwnerStatusEnvelopeV1,
        ) -> Result<
            Box<dyn ReceiptWatcherTask<SubmissionOwnerStatusResponseV1>>,
            ReceiptWatcherOperationError,
        > {
            match self.next() {
                BackendStep::Status(value) => Ok(Box::new(ReadyTask(Some(Ok(value))))),
                BackendStep::Error(error) => Err(error),
                _ => panic!("expected status step"),
            }
        }
    }

    fn key(id: &str, byte: u8) -> SubmissionReceiptWatchKey {
        SubmissionReceiptWatchKey {
            submission_id: OpaqueId::new(id).unwrap(),
            controller_public_key: PublicKey32::from_bytes([byte; 32]),
        }
    }

    fn accepted(id: &str) -> SubmissionAcceptedV1 {
        SubmissionAcceptedV1 {
            schema_version: SCHEMA_VERSION_V1,
            submission_id: OpaqueId::new(id).unwrap(),
            state: SubmissionLifecycleV1::Queued,
            retry_after_ms: 1,
        }
    }

    fn challenge(key: &SubmissionReceiptWatchKey) -> SubmissionOwnerStatusChallengeV1 {
        SubmissionOwnerStatusChallengeV1 {
            schema_version: SCHEMA_VERSION_V1,
            owner_status_challenge_id: OpaqueId::new("challenge-1").unwrap(),
            owner_status_challenge_nonce: ChallengeNonce32::from_bytes([9; 32]),
            expires_at_unix_ms: NOW + 60_000,
            controller_public_key: key.controller_public_key,
            submission_id: key.submission_id.clone(),
        }
    }

    fn envelope(challenge: &SubmissionOwnerStatusChallengeV1) -> SubmissionOwnerStatusEnvelopeV1 {
        SubmissionOwnerStatusEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            challenge: challenge.clone(),
            algorithm: SignatureAlgorithmV1::Ed25519,
            signature: Signature64::from_bytes([7; 64]),
        }
    }

    fn receipt(key: &SubmissionReceiptWatchKey, run_id: &OpaqueId) -> CampaignChainReceiptV1 {
        CampaignChainReceiptV1 {
            schema_version: SCHEMA_VERSION_V1,
            chain_id: OpaqueId::new("chain-1").unwrap(),
            predecessor_run_id: run_id.clone(),
            predecessor_verification_sha256: Digest32::from_bytes([6; 32]),
            expected_starting_campaign: ArtifactRefV1 {
                sha256: Digest32::from_bytes([2; 32]),
                byte_length: 10,
                media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
            },
            rules_config_sha256: Digest32::from_bytes([3; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([4; 32]),
            competition_manifest_sha256: None,
            campaign_content_manifest_sha256: Digest32::from_bytes([5; 32]),
            expected_max_concurrent_players: 1,
            participant_public_keys: vec![key.controller_public_key],
            campaign_controller_public_key: key.controller_public_key,
            state: CampaignChainStateV1::Active,
        }
    }

    fn response(
        envelope: &SubmissionOwnerStatusEnvelopeV1,
        state: SubmissionLifecycleV1,
    ) -> SubmissionOwnerStatusResponseV1 {
        SubmissionOwnerStatusResponseV1 {
            schema_version: SCHEMA_VERSION_V1,
            submission_id: envelope.challenge.submission_id.clone(),
            controller_public_key: envelope.challenge.controller_public_key,
            owner_status_envelope_sha256: envelope.canonical_digest().unwrap(),
            state,
        }
    }

    fn watcher(
        steps: Vec<BackendStep>,
    ) -> (SubmissionReceiptWatcher, Arc<Mutex<PersistenceState>>) {
        let persistence = Arc::new(Mutex::new(PersistenceState::default()));
        (
            SubmissionReceiptWatcher::with_parts(
                PendingSubmissionReceiptStore::default(),
                Box::new(ScriptedBackend {
                    steps: steps.into(),
                }),
                Box::new(MemoryPersistence(Arc::clone(&persistence))),
            ),
            persistence,
        )
    }

    fn drive_one_status(watcher: &mut SubmissionReceiptWatcher) {
        watcher.poll(NOW + 1);
        watcher.poll(NOW + 1);
        watcher.poll(NOW + 1);
        watcher.poll(NOW + 1);
    }

    #[test]
    fn verified_terminal_response_is_the_only_receipt_persistence_path() {
        let watch_key = key("submission-1", 4);
        let challenge = challenge(&watch_key);
        let envelope = envelope(&challenge);
        let run_id = OpaqueId::new("run-1").unwrap();
        let status = response(
            &envelope,
            SubmissionLifecycleV1::Accepted {
                run_id: run_id.clone(),
                campaign_chain_receipt: Some(receipt(&watch_key, &run_id)),
            },
        );
        let (mut watcher, persistence) = watcher(vec![
            BackendStep::Challenge(challenge),
            BackendStep::Envelope(envelope),
            BackendStep::Status(status),
        ]);
        watcher
            .enqueue_accepted(
                &accepted("submission-1"),
                watch_key.controller_public_key,
                NOW,
            )
            .unwrap();

        drive_one_status(&mut watcher);

        assert_eq!(watcher.pending_count(), 0);
        let persisted = persistence.lock().unwrap();
        assert_eq!(persisted.receipts.len(), 1);
        assert!(persisted.stores.iter().any(|store| {
            store
                .pending
                .iter()
                .any(|pending| pending.terminal_accepted.is_some())
        }));
        drop(persisted);
        assert!(matches!(
            watcher.take_notice(),
            Some(ReceiptWatcherNotice::Verified {
                submission_id,
                run_id: actual_run,
                campaign_receipt_persisted: true,
            }) if submission_id == watch_key.submission_id && actual_run == run_id
        ));
    }

    #[test]
    fn mismatched_status_fails_closed_without_persisting_receipt() {
        let watch_key = key("submission-1", 4);
        let challenge = challenge(&watch_key);
        let envelope = envelope(&challenge);
        let run_id = OpaqueId::new("run-1").unwrap();
        let mut status = response(
            &envelope,
            SubmissionLifecycleV1::Accepted {
                run_id: run_id.clone(),
                campaign_chain_receipt: Some(receipt(&watch_key, &run_id)),
            },
        );
        status.submission_id = OpaqueId::new("substituted-submission").unwrap();
        let (mut watcher, persistence) = watcher(vec![
            BackendStep::Challenge(challenge),
            BackendStep::Envelope(envelope),
            BackendStep::Status(status),
        ]);
        watcher
            .enqueue_accepted(
                &accepted("submission-1"),
                watch_key.controller_public_key,
                NOW,
            )
            .unwrap();

        drive_one_status(&mut watcher);

        assert_eq!(watcher.pending_count(), 0);
        assert!(persistence.lock().unwrap().receipts.is_empty());
        assert!(matches!(
            watcher.take_notice(),
            Some(ReceiptWatcherNotice::Abandoned { submission_id, .. })
                if submission_id == watch_key.submission_id
        ));
    }

    #[test]
    fn substituted_challenge_is_rejected_before_identity_signing() {
        let watch_key = key("submission-1", 4);
        let mut wrong = challenge(&watch_key);
        wrong.submission_id = OpaqueId::new("foreign-submission").unwrap();
        let (mut watcher, persistence) = watcher(vec![BackendStep::Challenge(wrong)]);
        watcher
            .enqueue_accepted(
                &accepted("submission-1"),
                watch_key.controller_public_key,
                NOW,
            )
            .unwrap();

        watcher.poll(NOW + 1);
        watcher.poll(NOW + 1);

        assert_eq!(watcher.pending_count(), 0);
        assert!(persistence.lock().unwrap().receipts.is_empty());
        assert!(matches!(
            watcher.take_notice(),
            Some(ReceiptWatcherNotice::Abandoned { .. })
        ));
    }

    #[test]
    fn nonterminal_lifecycle_remains_durable_and_uses_lifecycle_delay() {
        let watch_key = key("submission-1", 4);
        let challenge = challenge(&watch_key);
        let envelope = envelope(&challenge);
        let status = response(&envelope, SubmissionLifecycleV1::RetryPending);
        let (mut watcher, persistence) = watcher(vec![
            BackendStep::Challenge(challenge),
            BackendStep::Envelope(envelope),
            BackendStep::Status(status),
        ]);
        watcher
            .enqueue_accepted(
                &accepted("submission-1"),
                watch_key.controller_public_key,
                NOW,
            )
            .unwrap();

        drive_one_status(&mut watcher);

        assert!(watcher.is_tracking(&watch_key));
        let pending = watcher.store.pending.first().unwrap();
        assert_eq!(
            pending.next_attempt_at_unix_ms,
            NOW + 1 + RETRY_PENDING_POLL_MS
        );
        assert!(persistence.lock().unwrap().receipts.is_empty());
    }

    #[test]
    fn handoff_is_idempotent_but_owner_substitution_is_not() {
        let (mut watcher, _) = watcher(Vec::new());
        let accepted = accepted("submission-1");
        let owner = PublicKey32::from_bytes([4; 32]);
        assert!(watcher.enqueue_accepted(&accepted, owner, NOW).unwrap());
        assert!(!watcher.enqueue_accepted(&accepted, owner, NOW).unwrap());
        assert!(matches!(
            watcher.enqueue_accepted(&accepted, PublicKey32::from_bytes([5; 32]), NOW),
            Err(ReceiptWatcherError::OwnerConflict)
        ));
    }

    #[test]
    fn persisted_queue_can_be_reloaded_without_active_task_state() {
        let (mut watcher, persistence) = watcher(Vec::new());
        let accepted = accepted("submission-1");
        let owner = PublicKey32::from_bytes([4; 32]);
        watcher.enqueue_accepted(&accepted, owner, NOW).unwrap();
        let persisted = persistence.lock().unwrap().stores.last().unwrap().clone();
        persisted.validate().unwrap();

        let reloaded = SubmissionReceiptWatcher::with_parts(
            persisted,
            Box::new(ScriptedBackend {
                steps: VecDeque::new(),
            }),
            Box::new(MemoryPersistence(Arc::new(Mutex::new(
                PersistenceState::default(),
            )))),
        );

        assert_eq!(reloaded.pending_count(), 1);
        assert!(reloaded.active.is_none());
    }

    #[test]
    fn persisted_terminal_receipt_finishes_without_another_network_read() {
        let watch_key = key("submission-1", 4);
        let run_id = OpaqueId::new("run-1").unwrap();
        let terminal = VerifiedTerminalReceipt {
            run_id: run_id.clone(),
            campaign_chain_receipt: Some(receipt(&watch_key, &run_id)),
        };
        let store = PendingSubmissionReceiptStore {
            format: PENDING_STORE_FORMAT,
            pending: vec![PendingSubmissionReceiptWatch {
                key: watch_key.clone(),
                enqueued_at_unix_ms: NOW,
                next_attempt_at_unix_ms: NOW + 1,
                status_attempts: MAX_STATUS_ATTEMPTS,
                consecutive_failures: MAX_CONSECUTIVE_FAILURES,
                terminal_accepted: Some(terminal),
            }],
        };
        store.validate().unwrap();
        let persistence = Arc::new(Mutex::new(PersistenceState::default()));
        let mut watcher = SubmissionReceiptWatcher::with_parts(
            store,
            Box::new(ScriptedBackend {
                steps: VecDeque::new(),
            }),
            Box::new(MemoryPersistence(Arc::clone(&persistence))),
        );

        watcher.poll(NOW + MAX_PENDING_AGE_MS + 1);

        assert_eq!(watcher.pending_count(), 0);
        assert_eq!(persistence.lock().unwrap().receipts.len(), 1);
        assert!(matches!(
            watcher.take_notice(),
            Some(ReceiptWatcherNotice::Verified {
                submission_id,
                run_id: actual_run,
                campaign_receipt_persisted: true,
            }) if submission_id == watch_key.submission_id && actual_run == run_id
        ));
    }

    #[test]
    fn application_owner_is_safe_to_share_between_context_clones() {
        fn assert_send<T: Send>() {}
        assert_send::<ApplicationReceiptWatcher>();
        assert_send::<SubmissionReceiptWatcher>();
    }

    #[test]
    fn transient_errors_back_off_and_terminal_server_failures_do_not_write_receipts() {
        let watch_key = key("submission-1", 4);
        let (mut transient_watcher, persistence) = watcher(vec![BackendStep::Error(
            ReceiptWatcherOperationError::Transient("offline".to_owned()),
        )]);
        transient_watcher
            .enqueue_accepted(
                &accepted("submission-1"),
                watch_key.controller_public_key,
                NOW,
            )
            .unwrap();
        transient_watcher.poll(NOW + 1);
        let pending = transient_watcher.store.pending.first().unwrap();
        assert_eq!(pending.consecutive_failures, 1);
        assert_eq!(pending.next_attempt_at_unix_ms, NOW + 1 + 1_000);
        assert!(persistence.lock().unwrap().receipts.is_empty());

        let terminal_challenge = challenge(&watch_key);
        let terminal_envelope = envelope(&terminal_challenge);
        let terminal_status = response(
            &terminal_envelope,
            SubmissionLifecycleV1::Failed {
                code: SubmissionFailureCodeV1::VerificationInfrastructure,
                safe_message: "verification worker exhausted retries".to_owned(),
            },
        );
        let (mut terminal, terminal_persistence) = watcher(vec![
            BackendStep::Challenge(terminal_challenge),
            BackendStep::Envelope(terminal_envelope),
            BackendStep::Status(terminal_status),
        ]);
        terminal
            .enqueue_accepted(
                &accepted("submission-1"),
                watch_key.controller_public_key,
                NOW,
            )
            .unwrap();
        drive_one_status(&mut terminal);
        assert_eq!(terminal.pending_count(), 0);
        assert!(terminal_persistence.lock().unwrap().receipts.is_empty());
        assert!(matches!(
            terminal.take_notice(),
            Some(ReceiptWatcherNotice::VerificationFailed { .. })
        ));
    }

    #[test]
    fn rejection_is_terminal_and_unknown_store_fields_fail_closed() {
        let watch_key = key("submission-1", 4);
        let challenge = challenge(&watch_key);
        let envelope = envelope(&challenge);
        let status = response(
            &envelope,
            SubmissionLifecycleV1::Rejected {
                code: VerificationRejectionCodeV1::MalformedReplay,
                safe_message: "replay was rejected".to_owned(),
            },
        );
        let (mut watcher, persistence) = watcher(vec![
            BackendStep::Challenge(challenge),
            BackendStep::Envelope(envelope),
            BackendStep::Status(status),
        ]);
        watcher
            .enqueue_accepted(
                &accepted("submission-1"),
                watch_key.controller_public_key,
                NOW,
            )
            .unwrap();
        drive_one_status(&mut watcher);
        assert_eq!(watcher.pending_count(), 0);
        assert!(persistence.lock().unwrap().receipts.is_empty());

        let mut value = serde_json::to_value(PendingSubmissionReceiptStore::default()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("legacy_pending".to_owned(), serde_json::json!([]));
        assert!(serde_json::from_value::<PendingSubmissionReceiptStore>(value).is_err());
    }
}
