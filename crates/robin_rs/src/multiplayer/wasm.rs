//! WebAssembly (browser) WebSocket client for the multiplayer
//! transport.  Mirrors [`super::native`]'s [`connect_client`] surface
//! but uses the browser's `WebSocket` API instead of `tungstenite` —
//! `std::net` and synchronous reads aren't available in wasm.
//!
//! Server-side hosting is **not** supported on wasm: a browser tab
//! can't open a listening TCP socket.  Wasm clients can only connect
//! to a native `--server` running on a desktop / dedicated host.

use super::{NET_PROTOCOL_VERSION, NetEvent, NetMsg, NetOutbound, decode_msg, encode_msg};
use crate::player_command::PlayerId;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
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

/// Schedule a 40 ms (~25 Hz) interval-driven drain of `outgoing_rx`.
/// Each tick pulls every queued [`NetOutbound`] and pushes the
/// encoded frames through the WebSocket.  The closure leaks
/// intentionally — `setInterval` keeps it alive until the page
/// closes.
fn schedule_outgoing_pump(socket: web_sys::WebSocket, outgoing_rx: Receiver<NetOutbound>) {
    use wasm_bindgen::closure::Closure;

    let outgoing_rx = Rc::new(RefCell::new(outgoing_rx));
    let socket = Rc::new(socket);
    let pump = Closure::<dyn FnMut()>::new({
        let outgoing_rx = Rc::clone(&outgoing_rx);
        let socket = Rc::clone(&socket);
        move || {
            let rx = outgoing_rx.borrow();
            while let Ok(outbound) = rx.try_recv() {
                let frame = match outbound {
                    NetOutbound::Input {
                        origin_frame,
                        command,
                    } => encode_msg(&NetMsg::Input {
                        origin_frame,
                        command,
                    }),
                    NetOutbound::StateHash { .. } => continue, // host-only
                    NetOutbound::InitialSnapshot { .. } => continue, // host-only
                    NetOutbound::ReadyToSim { frame } => encode_msg(&NetMsg::ReadyToSim { frame }),
                    NetOutbound::ModalDismiss { kind, result } => {
                        encode_msg(&NetMsg::ModalDismiss { kind, result })
                    }
                };
                if let Err(e) = socket.send_with_u8_array(&frame) {
                    tracing::warn!("wasm-mp: send failed: {e:?}");
                    break;
                }
            }
        }
    });

    if let Some(window) = web_sys::window() {
        let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
            pump.as_ref().unchecked_ref(),
            40,
        );
    }
    // Leak the closure so the browser can keep invoking it.
    pump.forget();
}

/// Server-side launch is not available in the browser.  The shape
/// matches the native [`super::native::ServerHandle`] so callers can
/// share one match arm; constructing it always fails here.
pub struct ServerHandle {
    pub local_seat: PlayerId,
    pub mission_seed: u64,
}
