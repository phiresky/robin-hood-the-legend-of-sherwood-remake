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
/// (open / message / error / close) and the outgoing-pump timer.
pub struct ClientHandle {
    pub assigned_seat: Rc<RefCell<Option<PlayerId>>>,
    /// `None` until the asynchronous `Welcome` arrives. Returning a made-up
    /// seed here would let wasm initialize a divergent simulation.
    pub mission_seed: Rc<RefCell<Option<u64>>>,
    /// Keeps the message-pump closure live.  The browser only invokes
    /// the closure while the WebSocket is open; we Drop it when the
    /// handle goes away.
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_open: Closure<dyn FnMut(web_sys::Event)>,
    _on_close: Closure<dyn FnMut(web_sys::CloseEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::Event)>,
    _outgoing_pump: Closure<dyn FnMut()>,
    outgoing_interval_id: Option<i32>,
    window: web_sys::Window,
    socket: web_sys::WebSocket,
}

impl ClientHandle {
    pub fn mission_seed(&self) -> Option<u64> {
        *self.mission_seed.borrow()
    }

    pub fn shutdown(&mut self) {
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onclose(None);
        self.socket.set_onerror(None);
        if let Some(interval_id) = self.outgoing_interval_id.take() {
            self.window.clear_interval_with_handle(interval_id);
        }
        let _ = self.socket.close();
    }
}

impl Drop for ClientHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Connect to a multiplayer server from the browser. The handshake completes
/// asynchronously and reports the authoritative seat and seed as events.
pub fn connect_client(
    addr: &str,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> Result<ClientHandle, std::io::Error> {
    let window =
        web_sys::window().ok_or_else(|| std::io::Error::other("browser Window is unavailable"))?;
    let url = if addr.starts_with("ws://") || addr.starts_with("wss://") {
        addr.to_string()
    } else {
        format!("ws://{addr}/")
    };

    let socket = web_sys::WebSocket::new(&url)
        .map_err(|e| std::io::Error::other(format!("WebSocket::new: {e:?}")))?;
    socket.set_binary_type(web_sys::BinaryType::Arraybuffer);

    let assigned_seat = Rc::new(RefCell::new(None::<PlayerId>));
    let mission_seed = Rc::new(RefCell::new(None::<u64>));

    let on_open = {
        let socket = socket.clone();
        Closure::<dyn FnMut(_)>::new(move |_ev: web_sys::Event| {
            let hello = encode_msg(&NetMsg::Hello {
                protocol_version: NET_PROTOCOL_VERSION,
                nickname: nickname.clone(),
            });
            if let Err(e) = socket.send_with_u8_array(&hello) {
                tracing::error!("wasm-mp: send Hello failed: {e:?}");
            }
        })
    };
    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));

    let on_message = {
        let incoming_tx = incoming_tx.clone();
        let assigned_seat = Rc::clone(&assigned_seat);
        let mission_seed_slot = Rc::clone(&mission_seed);
        Closure::<dyn FnMut(_)>::new(move |ev: web_sys::MessageEvent| {
            let data = ev.data();
            let bytes = if let Some(buf) = data.dyn_ref::<js_sys::ArrayBuffer>() {
                js_sys::Uint8Array::new(buf).to_vec()
            } else {
                tracing::warn!("wasm-mp: non-binary frame received, ignoring");
                return;
            };
            match decode_msg(&bytes) {
                Ok(NetMsg::Welcome {
                    your_seat,
                    mission_seed,
                    host_nickname,
                }) => {
                    tracing::info!(
                        seat = your_seat.0,
                        seed = mission_seed,
                        host = %host_nickname,
                        "wasm-mp: welcomed by server"
                    );
                    *assigned_seat.borrow_mut() = Some(your_seat);
                    *mission_seed_slot.borrow_mut() = Some(mission_seed);
                    let _ = incoming_tx.send(NetEvent::AssignedLocalSeat(your_seat));
                    let _ = incoming_tx.send(NetEvent::MissionSeed(mission_seed));
                }
                Ok(NetMsg::InitialSnapshot {
                    frame,
                    engine_bytes,
                }) => {
                    let _ = incoming_tx.send(NetEvent::InitialSnapshot {
                        frame,
                        engine_bytes,
                    });
                }
                Ok(NetMsg::BeginSim {
                    frame,
                    start_epoch_ms,
                }) => {
                    let _ = incoming_tx.send(NetEvent::BeginSim {
                        frame,
                        start_epoch_ms,
                    });
                }
                Ok(NetMsg::BroadcastInput {
                    server_frame,
                    origin_frame,
                    target_frame,
                    input,
                }) => {
                    let _ = incoming_tx.send(NetEvent::Input {
                        server_frame,
                        origin_frame,
                        target_frame,
                        input,
                    });
                }
                Ok(NetMsg::Note(note)) => {
                    let _ = incoming_tx.send(NetEvent::Note(note));
                }
                Ok(NetMsg::StateHash {
                    frame,
                    hash,
                    clock_frame,
                    ms_until_next_frame,
                }) => {
                    let _ = incoming_tx.send(NetEvent::PeerStateHash {
                        frame,
                        hash,
                        clock_frame,
                        ms_until_next_frame,
                    });
                }
                Ok(NetMsg::ModalDismiss { kind, result }) => {
                    let _ = incoming_tx.send(NetEvent::ModalDismiss { kind, result });
                }
                Ok(other) => {
                    tracing::debug!(?other, "wasm-mp: ignoring unexpected wire message");
                }
                Err(e) => tracing::warn!("wasm-mp: decode error: {e}"),
            }
        })
    };
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    let on_close = {
        let incoming_tx = incoming_tx.clone();
        Closure::<dyn FnMut(_)>::new(move |ev: web_sys::CloseEvent| {
            tracing::info!(code = ev.code(), reason = %ev.reason(), "wasm-mp: socket closed");
            let _ = incoming_tx.send(NetEvent::Disconnected);
        })
    };
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

    let on_error = {
        let incoming_tx = incoming_tx.clone();
        Closure::<dyn FnMut(_)>::new(move |_ev: web_sys::Event| {
            tracing::warn!("wasm-mp: socket error event");
            let _ = incoming_tx.send(NetEvent::Note("wasm-mp: socket error".into()));
        })
    };
    socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));

    let outgoing_pump = make_outgoing_pump(socket.clone(), outgoing_rx);
    let outgoing_interval_id = match window.set_interval_with_callback_and_timeout_and_arguments_0(
        outgoing_pump.as_ref().unchecked_ref(),
        40,
    ) {
        Ok(id) => id,
        Err(e) => {
            socket.set_onopen(None);
            socket.set_onmessage(None);
            socket.set_onclose(None);
            socket.set_onerror(None);
            let _ = socket.close();
            return Err(std::io::Error::other(format!(
                "install multiplayer outgoing timer: {e:?}"
            )));
        }
    };

    Ok(ClientHandle {
        assigned_seat,
        mission_seed,
        _on_message: on_message,
        _on_open: on_open,
        _on_close: on_close,
        _on_error: on_error,
        _outgoing_pump: outgoing_pump,
        outgoing_interval_id: Some(outgoing_interval_id),
        window,
        socket,
    })
}

fn make_outgoing_pump(
    socket: web_sys::WebSocket,
    outgoing_rx: Receiver<NetOutbound>,
) -> Closure<dyn FnMut()> {
    let outgoing_rx = Rc::new(RefCell::new(outgoing_rx));
    Closure::<dyn FnMut()>::new(move || {
        while let Ok(outbound) = outgoing_rx.borrow().try_recv() {
            let frame = match outbound {
                NetOutbound::Input {
                    origin_frame,
                    command,
                } => encode_msg(&NetMsg::Input {
                    origin_frame,
                    command,
                }),
                NetOutbound::StateHash { .. } | NetOutbound::InitialSnapshot { .. } => continue,
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
    })
}
