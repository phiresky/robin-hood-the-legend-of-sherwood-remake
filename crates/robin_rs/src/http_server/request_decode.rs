//! Transport-independent automation DTOs and semantic admission.
//!
//! HTTP framing and URL query parsing belong to the native adapter. Browser
//! `null` parameters and empty HTTP step bodies deliberately retain their
//! respective defaulting rules.

use super::{
    HttpPayload, NativeCall, PlayerCommand, RpcError, ScreenshotRequest, StepModalPolicy,
    StepRequest,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(super) enum RequestKind {
    Script,
    State,
    HostDebug,
    LevelAssets,
    Decompile,
    Native,
    Batch,
    Console,
    Command,
    Screenshot,
    StepForward,
    StepBack,
    GoToFrame,
    SetPaused,
    GetReplay,
    LoadReplay,
}

impl RequestKind {
    pub(super) fn from_method(method: &str) -> Option<Self> {
        Some(match method {
            "script" => Self::Script,
            "state" => Self::State,
            "host-debug" => Self::HostDebug,
            "level-assets" => Self::LevelAssets,
            "decompile" => Self::Decompile,
            "native" => Self::Native,
            "batch" => Self::Batch,
            "console" => Self::Console,
            "command" => Self::Command,
            "screenshot" => Self::Screenshot,
            "step-forward" => Self::StepForward,
            "step-back" => Self::StepBack,
            "go-to-frame" => Self::GoToFrame,
            "set-paused" => Self::SetPaused,
            "get-replay" => Self::GetReplay,
            "load-replay" => Self::LoadReplay,
            _ => return None,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct BatchBody {
    calls: Vec<NativeCall>,
}

#[derive(Serialize, Deserialize)]
struct ConsoleBody {
    command: String,
}

#[derive(Serialize, Deserialize)]
struct GoToBody {
    frame: u32,
    #[serde(flatten)]
    modal_policy: StepModalPolicy,
}

#[derive(Serialize, Deserialize)]
struct SetPausedBody {
    paused: bool,
}

#[derive(Serialize, Deserialize)]
struct LoadReplayBody {
    data: String,
    #[serde(default)]
    paused: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct DecompileBody {
    class: Option<String>,
}

enum Parameters<'a> {
    #[cfg(any(test, not(target_arch = "wasm32")))]
    Json(&'a [u8]),
    #[cfg(any(test, target_arch = "wasm32"))]
    Browser {
        method: &'a str,
        value: serde_json::Value,
    },
}

impl Parameters<'_> {
    fn decode<T: DeserializeOwned>(self) -> Result<T, RpcError> {
        match self {
            #[cfg(any(test, not(target_arch = "wasm32")))]
            Self::Json(bytes) => serde_json::from_slice(bytes)
                .map_err(|e| RpcError::invalid_request(format!("bad json: {e}"))),
            #[cfg(any(test, target_arch = "wasm32"))]
            Self::Browser { method, value } => serde_json::from_value(value)
                .map_err(|e| RpcError::invalid_request(format!("{method} params: {e}"))),
        }
    }

    fn step_defaults(&self) -> bool {
        match self {
            #[cfg(any(test, not(target_arch = "wasm32")))]
            Self::Json(bytes) => {
                std::str::from_utf8(bytes).is_ok_and(|text| text.trim().is_empty())
            }
            #[cfg(any(test, target_arch = "wasm32"))]
            Self::Browser { value, .. } => value.is_null(),
        }
    }

    fn browser_defaults(&self) -> bool {
        match self {
            #[cfg(any(test, not(target_arch = "wasm32")))]
            Self::Json(_) => false,
            #[cfg(any(test, target_arch = "wasm32"))]
            Self::Browser { value, .. } => value.is_null(),
        }
    }
}

#[cfg(any(test, not(target_arch = "wasm32")))]
pub(super) fn decode_json(kind: RequestKind, body: &[u8]) -> Result<HttpPayload, RpcError> {
    decode(kind, Parameters::Json(body))
}

#[cfg(any(test, target_arch = "wasm32"))]
pub(super) fn decode_browser(
    method: &str,
    value: serde_json::Value,
) -> Result<HttpPayload, RpcError> {
    let kind = RequestKind::from_method(method)
        .ok_or_else(|| RpcError::unavailable_capability(format!("unknown method: {method}")))?;
    decode(kind, Parameters::Browser { method, value })
}

fn decode(kind: RequestKind, params: Parameters<'_>) -> Result<HttpPayload, RpcError> {
    Ok(match kind {
        RequestKind::Script => HttpPayload::Script,
        RequestKind::State => HttpPayload::State,
        RequestKind::HostDebug => HttpPayload::HostDebug,
        RequestKind::LevelAssets => HttpPayload::LevelAssets,
        RequestKind::GetReplay => HttpPayload::GetReplay,
        RequestKind::Decompile => {
            let body: DecompileBody = if params.browser_defaults() {
                DecompileBody::default()
            } else {
                params.decode()?
            };
            HttpPayload::Decompile { class: body.class }
        }
        RequestKind::Native => {
            let body: NativeCall = params.decode()?;
            HttpPayload::Native {
                name: body.op,
                args: body.args,
                this: body.this,
            }
        }
        RequestKind::Batch => HttpPayload::Batch(params.decode::<BatchBody>()?.calls),
        RequestKind::Console => HttpPayload::Console(params.decode::<ConsoleBody>()?.command),
        RequestKind::Command => HttpPayload::Command(params.decode::<PlayerCommand>()?),
        RequestKind::Screenshot => HttpPayload::Screenshot(if params.browser_defaults() {
            ScreenshotRequest::default()
        } else {
            params.decode()?
        }),
        RequestKind::StepForward | RequestKind::StepBack => {
            let request: StepRequest = if params.step_defaults() {
                StepRequest::default()
            } else {
                params.decode()?
            };
            if request.n == 0 {
                return Err(RpcError::invalid_request("n must be >= 1"));
            }
            match kind {
                RequestKind::StepForward => HttpPayload::StepForward { request },
                _ => HttpPayload::StepBack { request },
            }
        }
        RequestKind::GoToFrame => {
            let body: GoToBody = params.decode()?;
            HttpPayload::GoToFrame {
                target: body.frame,
                modal_policy: body.modal_policy,
            }
        }
        RequestKind::SetPaused => HttpPayload::SetPaused {
            paused: params.decode::<SetPausedBody>()?.paused,
        },
        RequestKind::LoadReplay => {
            let body: LoadReplayBody = params.decode()?;
            crate::replay_format::preflight_compact_transport(
                &body.data,
                &crate::replay_format::LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS,
            )
            .map_err(|error| {
                RpcError::invalid_request(format!("invalid compact replay: {error}"))
            })?;
            HttpPayload::LoadReplay {
                data: body.data,
                paused: body.paused,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn malformed_parameters_and_unknown_methods_have_explicit_categories() {
        let error = decode_browser("step-forward", serde_json::json!({"n": 0}))
            .err()
            .expect("zero steps rejected");
        assert_eq!(error.kind, super::super::RpcErrorKind::InvalidRequest);
        assert_eq!(
            error.wire_body(),
            serde_json::json!({"error": "n must be >= 1"})
        );
        let error = decode_browser("engine-dump", serde_json::Value::Null)
            .err()
            .expect("native-only method rejected");
        assert_eq!(
            error.kind,
            super::super::RpcErrorKind::UnavailableCapability
        );
        assert_eq!(error.message, "unknown method: engine-dump");
        let error = decode_json(RequestKind::Console, b"{")
            .err()
            .expect("malformed JSON rejected");
        assert_eq!(error.kind, super::super::RpcErrorKind::InvalidRequest);
        assert!(error.message.starts_with("bad json: "));
    }

    fn payload_fields(payload: HttpPayload) -> serde_json::Value {
        match payload {
            HttpPayload::Native { name, args, this } => serde_json::json!([name, args, this]),
            HttpPayload::Batch(calls) => serde_json::json!(calls),
            HttpPayload::Console(command) => serde_json::json!(command),
            HttpPayload::StepForward { request } | HttpPayload::StepBack { request } => {
                serde_json::json!(request)
            }
            HttpPayload::GoToFrame {
                target,
                modal_policy,
            } => serde_json::json!([target, modal_policy]),
            HttpPayload::SetPaused { paused } => serde_json::json!(paused),
            HttpPayload::LoadReplay { data, paused } => serde_json::json!([data, paused]),
            _ => panic!("add field comparison for the new decoder fixture"),
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn shared_adapters_accept_and_reject_the_same_explicit_parameters() {
        for (method, value, accepted) in [
            ("native", serde_json::json!({"op":"Test", "args":[]}), true),
            ("native", serde_json::json!({"args":[]}), false),
            ("batch", serde_json::json!({"calls":[]}), true),
            ("batch", serde_json::json!({"calls":[{}]}), false),
            ("console", serde_json::json!({"command":"help"}), true),
            ("console", serde_json::json!({"command":false}), false),
            ("command", serde_json::json!({"not-a-command":{}}), false),
            ("step-forward", serde_json::json!({}), true),
            ("step-forward", serde_json::json!({"n":0}), false),
            ("step-back", serde_json::json!({"n":2}), true),
            ("step-back", serde_json::json!({"n":-1}), false),
            ("go-to-frame", serde_json::json!({"frame":42}), true),
            (
                "go-to-frame",
                serde_json::json!({"frame":4294967296u64}),
                false,
            ),
            ("set-paused", serde_json::json!({"paused":true}), true),
            ("set-paused", serde_json::json!({}), false),
            (
                "load-replay",
                serde_json::json!({"data":"not a replay"}),
                false,
            ),
            // Preflight checks the envelope, not the compressed replay contents.
            (
                "load-replay",
                serde_json::json!({"data":"rhrec-abcdef012345-AA"}),
                true,
            ),
        ] {
            let body = serde_json::to_vec(&value).unwrap();
            let native = decode_json(RequestKind::from_method(method).unwrap(), &body);
            let browser = decode_browser(method, value);
            assert_eq!(native.is_ok(), accepted, "native {method}");
            assert_eq!(browser.is_ok(), accepted, "browser {method}");
            if let (Ok(native), Ok(browser)) = (native, browser) {
                assert_eq!(
                    std::mem::discriminant(&native),
                    std::mem::discriminant(&browser),
                    "{method}"
                );
                assert_eq!(payload_fields(native), payload_fields(browser), "{method}");
            }
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn transport_defaulting_and_native_only_methods_remain_explicit() {
        assert!(decode_json(RequestKind::StepForward, b" \n").is_ok());
        assert!(decode_json(RequestKind::StepForward, b"null").is_err());
        assert!(decode_browser("step-forward", serde_json::Value::Null).is_ok());
        assert!(decode_browser("screenshot", serde_json::Value::Null).is_ok());
        assert!(decode_browser("decompile", serde_json::Value::Null).is_ok());
        assert!(decode_browser("engine-dump", serde_json::Value::Null).is_err());
        assert!(decode_browser("script/decompile", serde_json::Value::Null).is_err());
    }
}
