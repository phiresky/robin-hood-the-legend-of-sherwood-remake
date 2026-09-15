//! Shared, frame-polled username registration before any replay upload.

use crate::leaderboard::{
    service::{LeaderboardApi, LeaderboardServiceError, decode_validated_json},
    signing::{GameIdentitySigner, PlatformSigner},
    task::PollTask,
};
use robin_run_protocol::{PlayerProfileV1, PublicKey32, UsernameUpdateV2, Validate};
use serde::Serialize;

pub(crate) type RegistrationHandle = std::sync::Arc<std::sync::Mutex<Option<Registration>>>;

#[derive(Debug, Serialize)]
pub struct Registration {
    #[serde(skip)]
    api: LeaderboardApi,
    pub input: crate::widget::WidgetInputField,
    pub message: String,
    pub needs_name: bool,
    pub cancelled: bool,
    input_started: bool,
    #[serde(skip)]
    task: Option<PollTask<Result<bool, String>>>,
}
robin_util::deny_deserialize!(Registration, "registration owns a live HTTP task");

impl Registration {
    pub fn new(api: LeaderboardApi) -> Result<Self, String> {
        let worker_api = api.clone();
        let task = PollTask::spawn_background("leaderboard-profile", move || async move {
            registered(&worker_api).await.map_err(|e| e.to_string())
        })
        .map_err(|e| e.to_string())?;
        Ok(Self::with_task(api, task))
    }

    fn with_task(api: LeaderboardApi, task: PollTask<Result<bool, String>>) -> Self {
        let mut input = crate::widget::WidgetInputField::new(0);
        input.max_length = 49;
        input.enter_edit_mode();
        Self {
            api,
            input,
            message: "Checking leaderboard username...".into(),
            needs_name: false,
            cancelled: false,
            input_started: false,
            task: Some(task),
        }
    }

    #[cfg(test)]
    pub(crate) fn awaiting_name() -> Self {
        let mut registration = Self::test_result(Ok(false));
        assert!(!registration.poll());
        registration
    }

    #[cfg(test)]
    pub(crate) fn test_result(result: Result<bool, String>) -> Self {
        Self::with_task(
            LeaderboardApi::from_preferences(&Default::default()).unwrap(),
            PollTask::ready(result),
        )
    }

    pub(crate) fn activate_input(&mut self) {
        if self.needs_name && !self.input_started {
            crate::window::start_text_input();
            self.input_started = true;
        }
    }

    pub(crate) fn cancel(&mut self) {
        self.cancelled = true;
        self.task = None;
        crate::window::stop_text_input();
    }

    pub fn busy(&self) -> bool {
        self.task.is_some()
    }

    pub fn poll(&mut self) -> bool {
        let Some(result) = self
            .task
            .as_ref()
            .and_then(|task| task.poll(|| "Username request stopped unexpectedly".into()))
        else {
            return false;
        };
        self.task = None;
        match result {
            Ok(true) => return true,
            Ok(false) => {
                self.needs_name = true;
                self.message = "Choose your public leaderboard name.".into();
            }
            Err(error) => {
                tracing::error!("Leaderboard username: {error}");
                self.message = error;
            }
        }
        false
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
        let api = self.api.clone();
        match PollTask::spawn_background("leaderboard-username", move || async move {
            if needs_name {
                register(&api, name).await
            } else {
                registered(&api).await
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

    /// Handle only prompt events; callers keep them away from underlying controls.
    pub(crate) fn event(
        &mut self,
        event: &crate::gfx_types::GameEvent,
        transform: crate::ingame_menu::layout::MenuTransform,
        origin: (i32, i32),
    ) {
        use crate::gfx_types::{GameEvent, Keycode};
        self.activate_input();
        let click = match event {
            GameEvent::MouseDown(x, y, 1, _) => {
                let (x, y) = transform.from_screen(*x, *y);
                Some((x - origin.0, y - origin.1))
            }
            _ => None,
        };
        if matches!(
            event,
            GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                }
        ) || click.is_some_and(|(x, y)| (300..=520).contains(&x) && (230..=270).contains(&y))
        {
            self.cancel();
            return;
        }
        if self.busy() {
            return;
        }
        if matches!(
            event,
            GameEvent::KeyDown {
                keycode: Keycode::Return,
                ..
            }
        ) || click.is_some_and(|(x, y)| (24..=284).contains(&x) && (230..=270).contains(&y))
        {
            self.confirm();
        } else if self.needs_name {
            match event {
                GameEvent::TextInput { text } => {
                    for ch in text.chars().filter(|ch| !ch.is_control()) {
                        self.input.insert_character(ch);
                    }
                }
                _ => {
                    crate::ingame_menu::save_load::edit_save_name(&mut self.input, event);
                }
            }
        }
    }

    pub(crate) fn draw(
        &mut self,
        renderer: &mut crate::renderer::Renderer,
        font: &crate::native_font::Font,
        transform: crate::ingame_menu::layout::MenuTransform,
        origin: (i32, i32),
    ) {
        use crate::ingame_menu::layout::{self, MenuRect, TextAlign, VAlign};
        self.activate_input();
        let rect = |renderer: &mut crate::renderer::Renderer, x, y, w, h, color| {
            renderer.render_gpu_rect(
                transform.origin_x + origin.0 + x,
                transform.origin_y + origin.1 + y,
                w,
                h,
                color,
            );
        };
        let text = |renderer: &mut crate::renderer::Renderer, value: &str, x, y, w, h| {
            layout::render_clipped_text_in_box_font(
                renderer,
                font,
                transform,
                value,
                MenuRect {
                    x: origin.0 + x,
                    y: origin.1 + y,
                    w,
                    h,
                },
                TextAlign::Left,
                VAlign::Top,
            );
        };
        rect(renderer, 0, 0, 544, 304, [24, 34, 25, 255]);
        text(renderer, "Leaderboard username", 24, 24, 496, 36);
        if self.needs_name {
            rect(renderer, 24, 74, 496, 48, [8, 14, 9, 255]);
            let mut value = self.input.edit_text.clone();
            let caret = value
                .char_indices()
                .nth(self.input.caret_offset)
                .map_or(value.len(), |(offset, _)| offset);
            value.insert(caret, '|');
            text(renderer, &value, 36, 84, 472, 36);
        }
        for (line, value) in layout::wrap_text_for_box_font(font, &self.message, 496, 3)
            .lines
            .iter()
            .enumerate()
        {
            text(renderer, &value.text, 24, 140 + line as i32 * 24, 496, 24);
        }
        for (x, w, label, active) in [
            (
                24,
                260,
                if self.needs_name {
                    "Register and submit"
                } else {
                    "Retry"
                },
                !self.busy(),
            ),
            (300, 220, "Cancel (Esc)", true),
        ] {
            rect(
                renderer,
                x,
                230,
                w,
                40,
                if active {
                    [48, 39, 17, 255]
                } else {
                    [15, 23, 16, 255]
                },
            );
            rect(
                renderer,
                x,
                268,
                w,
                2,
                if active {
                    [191, 164, 93, 255]
                } else {
                    [74, 82, 66, 255]
                },
            );
            text(renderer, label, x + 12, 240, w - 24, 30);
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

async fn registered(api: &LeaderboardApi) -> anyhow::Result<bool> {
    let key = PlatformSigner::public_key().await?;
    Ok(checked_profile(api.player_profile(key)?.take().await, key)?.is_some())
}

async fn register(api: &LeaderboardApi, name: String) -> anyhow::Result<bool> {
    let key = PlatformSigner::public_key().await?;
    let signed = PlatformSigner::sign_username_update(username_claim(
        key,
        name.clone(),
        crate::leaderboard_receipt_watcher::now_unix_ms()?,
    ))
    .await?;
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

    #[test]
    fn registration_gates_upload_until_success() {
        let mut registration = Registration::awaiting_name();
        assert!(registration.needs_name);
        registration.confirm();
        assert!(
            !registration.busy(),
            "empty names must not start network requests"
        );
        assert!(!registration.poll());
        registration.task = Some(PollTask::ready(Err("network unavailable".into())));
        assert!(!registration.poll());
        assert_eq!(registration.message, "network unavailable");
        registration.task = Some(PollTask::ready(Ok(true)));
        assert!(registration.poll());
        assert!(!registration.poll(), "resume only once");
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
    fn mission_prompt_buttons_validate_and_cancel_at_their_drawn_positions() {
        let mut registration = Registration::awaiting_name();
        let transform = crate::ingame_menu::layout::MenuTransform::centered(640, 480);
        registration.event(
            &crate::gfx_types::GameEvent::MouseDown(80, 330, 1, 1),
            transform,
            (48, 88),
        );
        assert!(!registration.busy(), "empty username stays in the prompt");
        assert!(!registration.cancelled);
        registration.event(
            &crate::gfx_types::GameEvent::MouseDown(400, 330, 1, 1),
            transform,
            (48, 88),
        );
        assert!(registration.cancelled);
        assert!(!registration.poll());
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
