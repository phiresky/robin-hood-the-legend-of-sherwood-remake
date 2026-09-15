//! Username registration before uploading an archived replay.

use crate::leaderboard::{
    service::{LeaderboardApi, LeaderboardServiceError, decode_validated_json},
    signing::{GameIdentitySigner, PlatformSigner},
    task::PollTask,
};
use robin_run_protocol::{PlayerProfileV1, PublicKey32, UsernameUpdateV2, Validate};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PendingReplay {
    pub path: std::path::PathBuf,
    pub identity: (
        robin_engine::campaign_history::MissionAttemptKey,
        Option<i64>,
    ),
    pub edition: robin_run_protocol::OfficialContentEditionV1,
}

#[derive(Serialize)]
pub(crate) struct Registration {
    pending: PendingReplay,
    pub input: crate::widget::WidgetInputField,
    pub message: String,
    pub needs_name: bool,
    #[serde(skip)]
    task: Option<PollTask<Result<bool, String>>>,
}
robin_util::deny_deserialize!(Registration, "registration owns a live HTTP task");

impl Registration {
    pub fn new(pending: PendingReplay) -> Result<Self, String> {
        let task = PollTask::spawn_background("leaderboard-profile", || async {
            registered().await.map_err(|e| e.to_string())
        })
        .map_err(|e| e.to_string())?;
        Ok(Self::with_task(pending, task))
    }

    fn with_task(pending: PendingReplay, task: PollTask<Result<bool, String>>) -> Self {
        let mut input = crate::widget::WidgetInputField::new(0);
        input.max_length = 49;
        input.enter_edit_mode();
        Self {
            pending,
            input,
            message: "Checking leaderboard username...".into(),
            needs_name: false,
            task: Some(task),
        }
    }

    #[cfg(test)]
    pub(crate) fn awaiting_name(pending: PendingReplay) -> Self {
        let mut registration = Self::with_task(pending, PollTask::ready(Ok(false)));
        assert!(registration.poll().is_none());
        registration
    }

    pub fn busy(&self) -> bool {
        self.task.is_some()
    }

    pub fn poll(&mut self) -> Option<PendingReplay> {
        let result = self
            .task
            .as_ref()?
            .poll(|| "Username request stopped unexpectedly".into())?;
        self.task = None;
        match result {
            Ok(true) => return Some(self.pending.clone()),
            Ok(false) => {
                self.needs_name = true;
                self.message = "Choose your public leaderboard name.".into();
                crate::window::start_text_input();
            }
            Err(error) => {
                tracing::error!("Leaderboard username: {error}");
                self.message = error;
            }
        }
        None
    }

    pub fn confirm(&mut self) {
        if self.busy() {
            return;
        }
        let name = self.input.edit_text.trim().to_owned();
        if self.needs_name {
            // Validate before starting any request; use the same rules as the server.
            let claim = username_claim(PublicKey32::from_bytes([1; 32]), name.clone(), 1);
            if let Err(error) = claim.validate() {
                self.message = error.to_string();
                return;
            }
        }
        let needs_name = self.needs_name;
        match PollTask::spawn_background("leaderboard-username", move || async move {
            if needs_name {
                register(name).await
            } else {
                registered().await
            }
            .map_err(|e| e.to_string())
        }) {
            Ok(task) => {
                self.task = Some(task);
                self.message = if needs_name {
                    "Registering username..."
                } else {
                    "Checking leaderboard username..."
                }
                .into();
            }
            Err(error) => self.message = error.to_string(),
        }
    }
}

fn username_claim(
    public_key: PublicKey32,
    username: String,
    signed_at_unix_ms: u64,
) -> UsernameUpdateV2 {
    UsernameUpdateV2 {
        schema_version: robin_run_protocol::SCHEMA_VERSION_V2,
        public_key,
        username,
        signed_at_unix_ms,
    }
}

async fn api_identity() -> anyhow::Result<(LeaderboardApi, PublicKey32)> {
    let preferences = crate::leaderboard::preferences::load()?;
    Ok((
        LeaderboardApi::from_preferences(&preferences)?,
        PlatformSigner::public_key().await?,
    ))
}

fn checked_profile(
    response: Result<
        crate::leaderboard_http::HttpResponse,
        crate::leaderboard_http::HttpTransportError,
    >,
    key: PublicKey32,
) -> Result<Option<PlayerProfileV1>, LeaderboardServiceError> {
    let response = response?;
    if response.status == 404 {
        return Ok(None);
    }
    let profile: PlayerProfileV1 = decode_validated_json(Ok(response))?;
    if profile.public_key != key {
        return Err(LeaderboardServiceError::InvalidProtocol(
            "player profile names another identity".into(),
        ));
    }
    Ok(Some(profile))
}

async fn registered() -> anyhow::Result<bool> {
    let (api, key) = api_identity().await?;
    Ok(checked_profile(api.player_profile(key)?.take().await, key)?.is_some())
}

async fn register(name: String) -> anyhow::Result<bool> {
    let (api, key) = api_identity().await?;
    let signed = PlatformSigner::sign_username_update(username_claim(
        key,
        name.clone(),
        crate::leaderboard_receipt_watcher::now_unix_ms()?,
    ))?;
    let profile = checked_profile(api.update_username(&signed)?.take().await, key)?
        .ok_or_else(|| anyhow::anyhow!("Username registration endpoint was not found"))?;
    anyhow::ensure!(
        profile.username == name,
        "Server returned a different username"
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leaderboard_http::HttpResponse;

    fn pending() -> PendingReplay {
        PendingReplay {
            path: "selected.rhrec.jsonl".into(),
            identity: (
                robin_engine::campaign_history::MissionAttemptKey {
                    campaign_run_id: 7,
                    sequence: 2,
                },
                Some(123),
            ),
            edition: robin_run_protocol::OfficialContentEditionV1::Full,
        }
    }

    #[test]
    fn registration_gates_upload_and_retains_selected_replay() {
        let mut registration = Registration::awaiting_name(pending());
        assert!(registration.needs_name);
        registration.confirm();
        assert!(
            !registration.busy(),
            "empty names must not start network requests"
        );
        assert!(registration.poll().is_none());
        registration.task = Some(PollTask::ready(Err("network unavailable".into())));
        assert!(registration.poll().is_none());
        assert_eq!(registration.message, "network unavailable");
        registration.task = Some(PollTask::ready(Ok(true)));
        let replay = registration.poll().unwrap();
        assert_eq!(replay.path, pending().path);
        assert_eq!(replay.identity, pending().identity);
        assert!(registration.poll().is_none(), "resume only once");
    }

    #[test]
    fn only_missing_profiles_require_registration() {
        let key = PublicKey32::from_bytes([1; 32]);
        assert!(
            checked_profile(
                Ok(HttpResponse {
                    status: 404,
                    content_type: None,
                    body: vec![]
                }),
                key
            )
            .unwrap()
            .is_none()
        );
        assert!(
            checked_profile(
                Ok(HttpResponse {
                    status: 503,
                    content_type: None,
                    body: b"unavailable".to_vec()
                }),
                key
            )
            .is_err()
        );
        let profile = PlayerProfileV1 {
            schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
            username: "Robin".into(),
            public_key: key,
            public_key_fingerprint: key.short_fingerprint(),
        };
        let response = HttpResponse {
            status: 200,
            content_type: Some("application/json".into()),
            body: serde_json::to_vec(&profile).unwrap(),
        };
        assert_eq!(
            checked_profile(Ok(response.clone()), key).unwrap(),
            Some(profile)
        );
        assert!(checked_profile(Ok(response), PublicKey32::from_bytes([2; 32])).is_err());
    }

    #[test]
    fn names_use_protocol_validation() {
        let key = PublicKey32::from_bytes([1; 32]);
        for invalid in ["".to_owned(), "é".repeat(25), "bad\nname".into()] {
            assert!(username_claim(key, invalid, 1).validate().is_err());
        }
        assert!(username_claim(key, "Robin".into(), 1).validate().is_ok());
    }
}
