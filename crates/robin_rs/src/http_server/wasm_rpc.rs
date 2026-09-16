//! Wasm JS bridge.
//!
//! Browser has no loopback socket, so we expose the same request/reply
//! pipeline as a JS-callable `rh_rpc({ method, params }) -> Promise`.
//! Requests land on the current application's weakly bound queue,
//! drain on the game tick, and resolve the Promise through an internal
//! one-shot channel.

use super::{BROWSER_QUEUE, HttpPayload, HttpRequest, Reply, ReplyBody, Responder};
use wasm_bindgen::JsValue;

fn reply_to_js(reply: Reply) -> Result<JsValue, JsValue> {
    match reply {
        Ok(ReplyBody::ReplayExport(_)) => {
            panic!("deferred replay export must be resolved before JavaScript encoding")
        }
        Ok(ReplyBody::Json(value)) => {
            use serde::Serialize;

            let serializer = serde_wasm_bindgen::Serializer::json_compatible();
            value
                .serialize(&serializer)
                .map_err(|e| JsValue::from_str(&format!("encode reply: {e}")))
        }
        Ok(ReplyBody::Binary { content_type, data }) => {
            let array = js_sys::Uint8Array::from(data.as_slice());
            let out = js_sys::Object::new();
            js_sys::Reflect::set(
                &out,
                &JsValue::from_str("contentType"),
                &JsValue::from_str(content_type),
            )
            .map_err(|e| JsValue::from_str(&format!("set contentType: {e:?}")))?;
            js_sys::Reflect::set(&out, &JsValue::from_str("data"), &array)
                .map_err(|e| JsValue::from_str(&format!("set data: {e:?}")))?;
            Ok(out.into())
        }
        Err(message) => Err(JsValue::from_str(&message.to_string())),
    }
}

/// JS → Rust entry point.  Accepts `{ method, params }` and returns
/// a Promise resolved once the game loop drains the request on a
/// frame boundary.
#[wasm_bindgen::prelude::wasm_bindgen]
pub async fn rh_rpc(request: JsValue) -> Result<JsValue, JsValue> {
    #[derive(serde::Deserialize)]
    struct Req {
        method: String,
        #[serde(default)]
        #[serde(with = "serde_wasm_bindgen::preserve")]
        params: JsValue,
    }
    let req: Req = serde_wasm_bindgen::from_value(request).map_err(|e| {
        JsValue::from_str(
            &super::RpcError::invalid_request(format!("bad request: {e}")).to_string(),
        )
    })?;
    // Pure-introspection methods don't need a live engine — resolve
    // inline without touching the tick queue.
    match req.method.as_str() {
        "info" => {
            return reply_to_js(Ok(ReplyBody::Json(super::info_json())));
        }
        "natives" => {
            return reply_to_js(Ok(ReplyBody::Json(super::list_natives_json())));
        }
        _ => {}
    }
    let payload = if req.method == "load-replay" {
        use wasm_bindgen::JsCast as _;
        let data = js_sys::Reflect::get(&req.params, &JsValue::from_str("data"))?;
        let bytes = data
            .dyn_into::<js_sys::Uint8Array>()
            .map_err(|_| JsValue::from_str("load-replay data must be a Uint8Array"))?;
        if bytes.length() as usize
            > crate::replay_format::LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS.max_input_bytes
        {
            return Err(JsValue::from_str("replay exceeds binary input limit"));
        }
        let data = bytes.to_vec();
        crate::replay_format::preflight_compact_transport(
            &data,
            &crate::replay_format::LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS,
        )
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let paused = js_sys::Reflect::get(&req.params, &JsValue::from_str("paused"))?;
        let paused = if paused.is_undefined() {
            false
        } else {
            paused
                .as_bool()
                .ok_or_else(|| JsValue::from_str("paused must be a boolean"))?
        };
        HttpPayload::LoadReplay { data, paused }
    } else {
        let params = if req.params.is_undefined() {
            serde_json::Value::Null
        } else {
            serde_wasm_bindgen::from_value(req.params)
                .map_err(|error| JsValue::from_str(&error.to_string()))?
        };
        decode_request(&req.method, params).map_err(|e| JsValue::from_str(&e.to_string()))?
    };
    let queue = BROWSER_QUEUE
        .with(|binding| binding.borrow().upgrade())
        .ok_or_else(|| {
            JsValue::from_str(
                &super::RpcError::unavailable_capability("RPC bridge not initialized").to_string(),
            )
        })?;
    let retirement = queue
        .lock()
        .expect("queue mutex poisoned")
        .retirement_receiver();
    let (response_tx, rx) = Responder::channel();
    queue
        .lock()
        .expect("queue mutex poisoned")
        .push_back(HttpRequest {
            payload,
            response_tx: response_tx.with_router(&queue),
        });
    use futures::FutureExt as _;
    let reply = futures::select_biased! {
        _ = retirement.recv().fuse() => return reply_to_js(Err(super::RpcError::retired("HTTP transport stopped"))),
        reply = async {
            let reply = rx.recv().await.map_err(|e| JsValue::from_str(&super::RpcError::internal(format!("RPC response dropped: {e}")).to_string()))?;
            Ok::<_, JsValue>(super::resolve_deferred_reply(reply).await)
        }.fuse() => reply?,
    };
    reply_to_js(reply)
}

fn decode_request(method: &str, params: serde_json::Value) -> Result<HttpPayload, super::RpcError> {
    super::request_decode::decode_browser(method, params)
}
