//! WebAssembly (browser) WebSocket client for the multiplayer
//! transport.  Mirrors [`super::native`]'s [`connect_client`] surface
//! but uses the browser's `WebSocket` API instead of `tungstenite` —
//! `std::net` and synchronous reads aren't available in wasm.
//!
//! Server-side hosting is **not** supported on wasm: a browser tab
//! can't open a listening TCP socket.  Wasm clients can only connect
//! to a native `--server` running on a desktop / dedicated host.

use super::{NET_PROTOCOL_VERSION, NetEvent, NetMsg, decode_msg};
use crate::player_command::PlayerId;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;
use web_sys::js_sys;

/// Browser-side handle to an active client connection.  Owns the
/// JavaScript closures kept alive for the lifetime of the socket
/// (open / message / error / close); dropping the handle drops the
/// closures, which is fine because the socket itself keeps its
/// listeners until it closes.
pub struct ClientHandle {
    pub assigned_seat: Rc<RefCell<Option<PlayerId>>>,
    pub mission_seed: u64,
    /// Keeps the message-pump closure live.  The browser only invokes
    /// the closure while the WebSocket is open; we Drop it when the
    /// handle goes away.
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_open: Closure<dyn FnMut(web_sys::Event)>,
    _on_close: Closure<dyn FnMut(web_sys::CloseEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::Event)>,
    _socket: web_sys::WebSocket,
}

/// Server-side launch is not available in the browser.  The shape
/// matches the native [`super::native::ServerHandle`] so callers can
/// share one match arm; constructing it always fails here.
pub struct ServerHandle {
    pub local_seat: PlayerId,
    pub mission_seed: u64,
}
