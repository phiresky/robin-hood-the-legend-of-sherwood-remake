//! Headless multiplayer lobby client.
//!
//! The lobby server is intentionally separate from the game-session
//! websocket transport in [`super::native`].  It only brokers rooms:
//! listing waiting games, creating one with an advertised game-server
//! address, joining an existing game, and marking the hosted game as
//! started.  Once a room is selected, the normal `--server` / `--connect`
//! path drives the actual simulation.

use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};

pub const LOBBY_URL_ENV: &str = "ROBINHOOD_LOBBY_WS";
pub const MP_BIND_ENV: &str = "ROBINHOOD_MP_BIND";
pub const DEFAULT_MP_BIND: &str = ":7878";
pub const DEFAULT_LOBBY_BIND: &str = "0.0.0.0:7879";
pub const START_DELAY_MS: u64 = 1_500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LobbyGame {
    pub id: String,
    pub mission_id: u32,
    pub mission_name: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub players: u32,
    #[serde(default)]
    pub max_players: u32,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub connect_addr: String,
    #[serde(default)]
    pub start_at_epoch_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinedGame {
    pub game_id: String,
    pub mission_id: u32,
    pub mission_name: String,
    pub connect_addr: String,
    #[serde(default = "default_expected_players")]
    pub expected_players: u32,
    #[serde(default)]
    pub start_at_epoch_ms: Option<u64>,
}

fn default_expected_players() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedGame {
    pub game: LobbyGame,
    pub host_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LobbyRequest {
    List,
    Create {
        mission_id: u32,
        mission_name: String,
        host_nickname: String,
        bind_addr: String,
    },
    Join {
        game_id: String,
        nickname: String,
    },
    Start {
        game_id: String,
        host_token: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LobbyResponse {
    Games { games: Vec<LobbyGame> },
    Created { game: LobbyGame, host_token: String },
    Joined { game: JoinedGame },
    Started { game: JoinedGame },
    GameUpdated { game: LobbyGame },
    GameStarted { game: JoinedGame },
    Error { message: String },
}

#[derive(Debug, Clone)]
pub enum LobbyEvent {
    Games(Vec<LobbyGame>),
    Created(CreatedGame),
    Joined(JoinedGame),
    Started(JoinedGame),
    GameUpdated(LobbyGame),
    GameStarted(JoinedGame),
    Error(String),
    Disconnected(String),
}

impl From<LobbyResponse> for LobbyEvent {
    fn from(value: LobbyResponse) -> Self {
        match value {
            LobbyResponse::Games { games } => LobbyEvent::Games(games),
            LobbyResponse::Created { game, host_token } => {
                LobbyEvent::Created(CreatedGame { game, host_token })
            }
            LobbyResponse::Joined { game } => LobbyEvent::Joined(game),
            LobbyResponse::Started { game } => LobbyEvent::Started(game),
            LobbyResponse::GameUpdated { game } => LobbyEvent::GameUpdated(game),
            LobbyResponse::GameStarted { game } => LobbyEvent::GameStarted(game),
            LobbyResponse::Error { message } => LobbyEvent::Error(message),
        }
    }
}

pub struct LobbyClient {
    outgoing: Sender<LobbyRequest>,
    incoming: Receiver<LobbyEvent>,
    #[cfg(target_arch = "wasm32")]
    _wasm: WasmLobbyHandle,
}

impl LobbyClient {
    pub fn connect(lobby_url: &str) -> Result<Self, String> {
        connect_persistent(lobby_url)
    }

    pub fn list_games(&self) -> Result<(), String> {
        self.send(LobbyRequest::List)
    }

    pub fn create_game(
        &self,
        mission_id: u32,
        mission_name: String,
        host_nickname: String,
        bind_addr: String,
    ) -> Result<(), String> {
        self.send(LobbyRequest::Create {
            mission_id,
            mission_name,
            host_nickname,
            bind_addr,
        })
    }

    pub fn join_game(&self, game_id: String, nickname: String) -> Result<(), String> {
        self.send(LobbyRequest::Join { game_id, nickname })
    }

    pub fn start_game(&self, game_id: String, host_token: String) -> Result<(), String> {
        self.send(LobbyRequest::Start {
            game_id,
            host_token,
        })
    }

    pub fn try_recv(&self) -> Option<LobbyEvent> {
        self.incoming.try_recv().ok()
    }

    fn send(&self, request: LobbyRequest) -> Result<(), String> {
        self.outgoing
            .send(request)
            .map_err(|_| "lobby connection is closed".to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_lobby_server(addr: &str) -> Result<(), String> {
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use tungstenite::Message as WsMessage;
    use tungstenite::accept as ws_accept;

    #[derive(Default)]
    struct LobbyState {
        next_id: u64,
        next_client_id: u64,
        games: HashMap<String, LobbyGame>,
        host_tokens: HashMap<String, String>,
        clients: HashMap<u64, LobbyClientState>,
    }

    struct LobbyClientState {
        sender: Sender<LobbyResponse>,
        list_subscribed: bool,
        game_id: Option<String>,
    }

    impl LobbyState {
        fn add_client(&mut self, sender: Sender<LobbyResponse>) -> u64 {
            self.next_client_id = self.next_client_id.saturating_add(1);
            let id = self.next_client_id;
            self.clients.insert(
                id,
                LobbyClientState {
                    sender,
                    list_subscribed: false,
                    game_id: None,
                },
            );
            id
        }

        fn remove_client(&mut self, id: u64) {
            self.clients.remove(&id);
        }

        fn handle(
            &mut self,
            req: LobbyRequest,
            peer_addr: Option<std::net::SocketAddr>,
            client_id: u64,
        ) -> LobbyResponse {
            match req {
                LobbyRequest::List => {
                    if let Some(client) = self.clients.get_mut(&client_id) {
                        client.list_subscribed = true;
                    }
                    LobbyResponse::Games {
                        games: self.games.values().cloned().collect(),
                    }
                }
                LobbyRequest::Create {
                    mission_id,
                    mission_name,
                    host_nickname,
                    bind_addr,
                } => {
                    self.next_id = self.next_id.saturating_add(1);
                    let id = self.next_id.to_string();
                    // Lobby host tokens are network authentication nonces, not gameplay RNG.
                    #[allow(clippy::disallowed_methods)]
                    let host_token = format!("{:016x}", fastrand::u64(..));
                    let connect_addr = advertised_connect_addr(&bind_addr, peer_addr);
                    tracing::info!(
                        client_id,
                        peer = ?peer_addr,
                        game_id = %id,
                        host = %host_nickname,
                        mission_id,
                        mission = %mission_name,
                        advertised_bind = %bind_addr,
                        advertised_connect = %connect_addr,
                        "lobby client created game"
                    );
                    let game = LobbyGame {
                        id: id.clone(),
                        mission_id,
                        mission_name,
                        host: host_nickname,
                        players: 1,
                        max_players: 4,
                        state: "waiting".to_string(),
                        connect_addr,
                        start_at_epoch_ms: None,
                    };
                    self.games.insert(id, game.clone());
                    self.host_tokens.insert(game.id.clone(), host_token.clone());
                    if let Some(client) = self.clients.get_mut(&client_id) {
                        client.game_id = Some(game.id.clone());
                    }
                    self.broadcast_game_updated(&game, Some(client_id));
                    LobbyResponse::Created { game, host_token }
                }
                LobbyRequest::Join { game_id, nickname } => match self.games.get_mut(&game_id) {
                    Some(game) if game.state == "waiting" || game.state == "started" => {
                        game.players = game.players.saturating_add(1);
                        let game_update = game.clone();
                        let joined = JoinedGame {
                            game_id,
                            mission_id: game.mission_id,
                            mission_name: game.mission_name.clone(),
                            connect_addr: game.connect_addr.clone(),
                            expected_players: game.players,
                            start_at_epoch_ms: game.start_at_epoch_ms,
                        };
                        tracing::info!(
                            client_id,
                            peer = ?peer_addr,
                            game_id = %joined.game_id,
                            nickname = %nickname,
                            advertised_connect = %joined.connect_addr,
                            expected_players = joined.expected_players,
                            "lobby client joined game"
                        );
                        if let Some(client) = self.clients.get_mut(&client_id) {
                            client.game_id = Some(joined.game_id.clone());
                        }
                        self.broadcast_game_updated(&game_update, None);
                        LobbyResponse::Joined { game: joined }
                    }
                    Some(_) => LobbyResponse::Error {
                        message: "game is not joinable".to_string(),
                    },
                    None => LobbyResponse::Error {
                        message: format!("game `{game_id}` not found"),
                    },
                },
                LobbyRequest::Start {
                    game_id,
                    host_token,
                } => match self.games.get_mut(&game_id) {
                    Some(game) => {
                        if self.host_tokens.get(&game_id) != Some(&host_token) {
                            return LobbyResponse::Error {
                                message: "only the host can start this game".to_string(),
                            };
                        }
                        let start_at_epoch_ms = current_epoch_ms().saturating_add(START_DELAY_MS);
                        game.state = "started".to_string();
                        game.start_at_epoch_ms = Some(start_at_epoch_ms);
                        let game_update = game.clone();
                        let joined = JoinedGame {
                            game_id,
                            mission_id: game.mission_id,
                            mission_name: game.mission_name.clone(),
                            connect_addr: game.connect_addr.clone(),
                            expected_players: game.players,
                            start_at_epoch_ms: game.start_at_epoch_ms,
                        };
                        self.broadcast_game_updated(&game_update, Some(client_id));
                        self.broadcast_game_started(&joined, Some(client_id));
                        LobbyResponse::Started { game: joined }
                    }
                    None => LobbyResponse::Error {
                        message: format!("game `{game_id}` not found"),
                    },
                },
            }
        }

        fn broadcast_game_updated(&mut self, game: &LobbyGame, except: Option<u64>) {
            self.broadcast_to_interested(
                game,
                except,
                LobbyResponse::GameUpdated { game: game.clone() },
            );
        }

        fn broadcast_game_started(&mut self, game: &JoinedGame, except: Option<u64>) {
            let game_id = &game.game_id;
            self.clients.retain(|id, client| {
                if Some(*id) == except || client.game_id.as_ref() != Some(game_id) {
                    return true;
                }
                client
                    .sender
                    .send(LobbyResponse::GameStarted { game: game.clone() })
                    .is_ok()
            });
        }

        fn broadcast_to_interested(
            &mut self,
            game: &LobbyGame,
            except: Option<u64>,
            response: LobbyResponse,
        ) {
            self.clients.retain(|id, client| {
                if Some(*id) == except {
                    return true;
                }
                let interested =
                    client.list_subscribed || client.game_id.as_ref() == Some(&game.id);
                if !interested {
                    return true;
                }
                client.sender.send(response.clone()).is_ok()
            });
        }
    }

    fn serve_stream(stream: TcpStream, state: Arc<Mutex<LobbyState>>) -> Result<(), String> {
        use std::io::ErrorKind;
        use std::time::Duration;

        let peer_addr = stream.peer_addr().ok();
        let mut ws = ws_accept(stream).map_err(|e| format!("websocket accept: {e}"))?;
        let (tx, rx) = channel::<LobbyResponse>();
        let client_id = state.lock().unwrap().add_client(tx);
        tracing::info!(client_id, peer = ?peer_addr, "lobby client connected");
        let _ = ws
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(20)));

        let result = 'session: loop {
            match ws.read() {
                Ok(WsMessage::Text(text)) => {
                    let req: LobbyRequest = match serde_json::from_str(&text) {
                        Ok(req) => req,
                        Err(e) => {
                            let _ = send_response(
                                &mut ws,
                                &LobbyResponse::Error {
                                    message: format!("decode lobby request `{text}`: {e}"),
                                },
                            );
                            continue;
                        }
                    };
                    let response = state.lock().unwrap().handle(req, peer_addr, client_id);
                    if let Err(err) = send_response(&mut ws, &response) {
                        break 'session Err(err);
                    }
                }
                Ok(WsMessage::Binary(bytes)) => {
                    let req: LobbyRequest = match serde_json::from_slice(&bytes) {
                        Ok(req) => req,
                        Err(e) => {
                            let _ = send_response(
                                &mut ws,
                                &LobbyResponse::Error {
                                    message: format!("decode binary lobby request: {e}"),
                                },
                            );
                            continue;
                        }
                    };
                    let response = state.lock().unwrap().handle(req, peer_addr, client_id);
                    if let Err(err) = send_response(&mut ws, &response) {
                        break 'session Err(err);
                    }
                }
                Ok(WsMessage::Close(_)) => break 'session Ok(()),
                Ok(_) => {}
                Err(tungstenite::Error::Io(io))
                    if matches!(io.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(e) => break 'session Err(format!("read request: {e}")),
            }

            while let Ok(response) = rx.try_recv() {
                if let Err(err) = send_response(&mut ws, &response) {
                    break 'session Err(err);
                }
            }
        };
        state.lock().unwrap().remove_client(client_id);
        tracing::info!(client_id, peer = ?peer_addr, "lobby client disconnected");
        let _ = ws.close(None);
        result
    }

    fn send_response(
        ws: &mut tungstenite::WebSocket<TcpStream>,
        response: &LobbyResponse,
    ) -> Result<(), String> {
        let body = serde_json::to_string(response).map_err(|e| format!("encode response: {e}"))?;
        ws.send(WsMessage::Text(body.into()))
            .map_err(|e| format!("send response: {e}"))
    }

    let bind_addr = if addr.starts_with(':') {
        format!("0.0.0.0{addr}")
    } else {
        addr.to_string()
    };
    let listener = TcpListener::bind(&bind_addr)
        .map_err(|e| format!("bind lobby server `{bind_addr}`: {e}"))?;
    tracing::info!(addr = %listener.local_addr().map_err(|e| e.to_string())?, "multiplayer lobby server listening");
    let state = Arc::new(Mutex::new(LobbyState::default()));
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::Builder::new()
                    .name("mp-lobby-peer".to_string())
                    .spawn(move || {
                        if let Err(err) = serve_stream(stream, state) {
                            tracing::warn!("lobby peer failed: {err}");
                        }
                    })
                    .map_err(|e| format!("spawn lobby peer: {e}"))?;
            }
            Err(e) => return Err(format!("accept lobby peer: {e}")),
        }
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn run_lobby_server(_addr: &str) -> Result<(), String> {
    Err("multiplayer lobby server is native-only".to_string())
}

pub fn lobby_url_from_env() -> Result<String, String> {
    let raw = std::env::var(LOBBY_URL_ENV)
        .map_err(|_| format!("{LOBBY_URL_ENV} is not set; cannot open multiplayer lobby"))?;
    Ok(normalize_ws_url(&raw))
}

pub fn bind_addr_from_env() -> String {
    std::env::var(MP_BIND_ENV).unwrap_or_else(|_| DEFAULT_MP_BIND.to_string())
}

pub fn current_epoch_ms() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0)
    }
}

pub fn list_games(lobby_url: &str) -> Result<Vec<LobbyGame>, String> {
    match request_response(lobby_url, &LobbyRequest::List)? {
        LobbyResponse::Games { games } => Ok(games),
        LobbyResponse::Error { message } => Err(message),
        other => Err(format!("lobby list: unexpected response {other:?}")),
    }
}

pub fn create_game(
    lobby_url: &str,
    mission_id: u32,
    mission_name: String,
    host_nickname: String,
    bind_addr: String,
) -> Result<CreatedGame, String> {
    match request_response(
        lobby_url,
        &LobbyRequest::Create {
            mission_id,
            mission_name,
            host_nickname,
            bind_addr,
        },
    )? {
        LobbyResponse::Created { game, host_token } => Ok(CreatedGame { game, host_token }),
        LobbyResponse::Error { message } => Err(message),
        other => Err(format!("lobby create: unexpected response {other:?}")),
    }
}

pub fn join_game(lobby_url: &str, game_id: String, nickname: String) -> Result<JoinedGame, String> {
    match request_response(lobby_url, &LobbyRequest::Join { game_id, nickname })? {
        LobbyResponse::Joined { game } => Ok(game),
        LobbyResponse::Error { message } => Err(message),
        other => Err(format!("lobby join: unexpected response {other:?}")),
    }
}

pub fn start_game(
    lobby_url: &str,
    game_id: String,
    host_token: String,
) -> Result<JoinedGame, String> {
    match request_response(
        lobby_url,
        &LobbyRequest::Start {
            game_id,
            host_token,
        },
    )? {
        LobbyResponse::Started { game } => Ok(game),
        LobbyResponse::Error { message } => Err(message),
        other => Err(format!("lobby start: unexpected response {other:?}")),
    }
}

fn normalize_ws_url(raw: &str) -> String {
    if raw.starts_with("ws://") || raw.starts_with("wss://") {
        raw.to_string()
    } else {
        format!("ws://{raw}/")
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn advertised_connect_addr(bind_addr: &str, peer_addr: Option<std::net::SocketAddr>) -> String {
    let peer_host = peer_addr
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    if let Some(port) = bind_addr.strip_prefix(':') {
        return format!("{peer_host}:{port}");
    }
    let mut parts = bind_addr.rsplitn(2, ':');
    let port = parts.next().unwrap_or_default();
    let host = parts.next().unwrap_or_default();
    if is_unroutable_advertised_host(host) {
        format!("{peer_host}:{port}")
    } else {
        bind_addr.to_string()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn is_unroutable_advertised_host(host: &str) -> bool {
    matches!(
        host,
        "" | "0.0.0.0" | "::" | "127.0.0.1" | "::1" | "localhost"
    ) || host.starts_with("127.")
}

#[cfg(not(target_arch = "wasm32"))]
fn connect_persistent(lobby_url: &str) -> Result<LobbyClient, String> {
    use std::io::ErrorKind;
    use std::time::Duration;
    use tungstenite::Message as WsMessage;
    use tungstenite::client::IntoClientRequest;

    let (out_tx, out_rx) = channel::<LobbyRequest>();
    let (event_tx, event_rx) = channel::<LobbyEvent>();
    let lobby_url = lobby_url.to_string();
    std::thread::Builder::new()
        .name("mp-lobby-client".to_string())
        .spawn(move || {
            let req = match lobby_url.into_client_request() {
                Ok(req) => req,
                Err(e) => {
                    let _ = event_tx.send(LobbyEvent::Disconnected(format!("bad lobby URL: {e}")));
                    return;
                }
            };
            let (mut ws, _resp) = match tungstenite::connect(req) {
                Ok(conn) => conn,
                Err(e) => {
                    let _ = event_tx.send(LobbyEvent::Disconnected(format!("connect lobby: {e}")));
                    return;
                }
            };
            if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_ref() {
                let _ = s.set_read_timeout(Some(Duration::from_millis(20)));
            }

            loop {
                match ws.read() {
                    Ok(WsMessage::Text(text)) => match serde_json::from_str::<LobbyResponse>(&text)
                    {
                        Ok(response) => {
                            let _ = event_tx.send(response.into());
                        }
                        Err(e) => {
                            let _ = event_tx.send(LobbyEvent::Error(format!(
                                "decode lobby response `{text}`: {e}"
                            )));
                        }
                    },
                    Ok(WsMessage::Binary(bytes)) => {
                        match serde_json::from_slice::<LobbyResponse>(&bytes) {
                            Ok(response) => {
                                let _ = event_tx.send(response.into());
                            }
                            Err(e) => {
                                let _ = event_tx.send(LobbyEvent::Error(format!(
                                    "decode binary lobby response: {e}"
                                )));
                            }
                        }
                    }
                    Ok(WsMessage::Close(_)) => {
                        let _ = event_tx.send(LobbyEvent::Disconnected(
                            "lobby connection closed".to_string(),
                        ));
                        return;
                    }
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(io))
                        if matches!(io.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                    Err(e) => {
                        let _ = event_tx
                            .send(LobbyEvent::Disconnected(format!("lobby read failed: {e}")));
                        return;
                    }
                }

                loop {
                    match out_rx.try_recv() {
                        Ok(request) => {
                            let body = match serde_json::to_string(&request) {
                                Ok(body) => body,
                                Err(e) => {
                                    let _ = event_tx.send(LobbyEvent::Error(format!(
                                        "encode lobby request: {e}"
                                    )));
                                    continue;
                                }
                            };
                            if let Err(e) = ws.send(WsMessage::Text(body.into())) {
                                let _ = event_tx.send(LobbyEvent::Disconnected(format!(
                                    "lobby send failed: {e}"
                                )));
                                return;
                            }
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            let _ = ws.close(None);
                            return;
                        }
                    }
                }
            }
        })
        .map_err(|e| format!("spawn lobby client: {e}"))?;

    Ok(LobbyClient {
        outgoing: out_tx,
        incoming: event_rx,
    })
}

#[cfg(target_arch = "wasm32")]
struct WasmLobbyHandle {
    socket: web_sys::WebSocket,
    interval_id: i32,
    _on_message: wasm_bindgen::prelude::Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_close: wasm_bindgen::prelude::Closure<dyn FnMut(web_sys::CloseEvent)>,
    _on_error: wasm_bindgen::prelude::Closure<dyn FnMut(web_sys::Event)>,
    _pump: wasm_bindgen::prelude::Closure<dyn FnMut()>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for WasmLobbyHandle {
    fn drop(&mut self) {
        if let Some(window) = web_sys::window() {
            window.clear_interval_with_handle(self.interval_id);
        }
        let _ = self.socket.close();
    }
}

#[cfg(target_arch = "wasm32")]
fn connect_persistent(lobby_url: &str) -> Result<LobbyClient, String> {
    use wasm_bindgen::JsCast;

    let socket = web_sys::WebSocket::new(lobby_url)
        .map_err(|e| format!("WebSocket::new({lobby_url}): {e:?}"))?;
    socket.set_binary_type(web_sys::BinaryType::Arraybuffer);

    let (out_tx, out_rx) = channel::<LobbyRequest>();
    let (event_tx, event_rx) = channel::<LobbyEvent>();

    let on_message = {
        let event_tx = event_tx.clone();
        wasm_bindgen::prelude::Closure::<dyn FnMut(_)>::new(move |ev: web_sys::MessageEvent| {
            let data = ev.data();
            let response = if let Some(text) = data.as_string() {
                serde_json::from_str::<LobbyResponse>(&text)
                    .map_err(|e| format!("decode lobby response `{text}`: {e}"))
            } else if let Some(buf) = data.dyn_ref::<js_sys::ArrayBuffer>() {
                let array = js_sys::Uint8Array::new(buf);
                serde_json::from_slice::<LobbyResponse>(&array.to_vec())
                    .map_err(|e| format!("decode binary lobby response: {e}"))
            } else {
                Err("decode lobby response: unsupported websocket frame".to_string())
            };
            match response {
                Ok(response) => {
                    let _ = event_tx.send(response.into());
                }
                Err(err) => {
                    let _ = event_tx.send(LobbyEvent::Error(err));
                }
            }
        })
    };
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    let on_close = {
        let event_tx = event_tx.clone();
        wasm_bindgen::prelude::Closure::<dyn FnMut(_)>::new(move |ev: web_sys::CloseEvent| {
            let reason = ev.reason();
            let msg = if reason.is_empty() {
                format!("lobby connection closed ({})", ev.code())
            } else {
                format!("lobby connection closed ({}): {reason}", ev.code())
            };
            let _ = event_tx.send(LobbyEvent::Disconnected(msg));
        })
    };
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

    let on_error = {
        let event_tx = event_tx.clone();
        wasm_bindgen::prelude::Closure::<dyn FnMut(_)>::new(move |_ev: web_sys::Event| {
            let _ = event_tx.send(LobbyEvent::Error("lobby websocket error".to_string()));
        })
    };
    socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));

    let pump = {
        let socket = socket.clone();
        let event_tx = event_tx.clone();
        wasm_bindgen::prelude::Closure::<dyn FnMut()>::new(move || {
            if socket.ready_state() != web_sys::WebSocket::OPEN {
                return;
            }
            loop {
                match out_rx.try_recv() {
                    Ok(request) => {
                        let body = match serde_json::to_string(&request) {
                            Ok(body) => body,
                            Err(e) => {
                                let _ = event_tx
                                    .send(LobbyEvent::Error(format!("encode lobby request: {e}")));
                                continue;
                            }
                        };
                        if let Err(e) = socket.send_with_str(&body) {
                            let _ = event_tx.send(LobbyEvent::Disconnected(format!(
                                "lobby send failed: {e:?}"
                            )));
                            break;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        let _ = socket.close();
                        break;
                    }
                }
            }
        })
    };
    let window = web_sys::window().ok_or("browser window is unavailable")?;
    let interval_id = window
        .set_interval_with_callback_and_timeout_and_arguments_0(pump.as_ref().unchecked_ref(), 20)
        .map_err(|e| format!("install lobby send pump: {e:?}"))?;

    Ok(LobbyClient {
        outgoing: out_tx,
        incoming: event_rx,
        _wasm: WasmLobbyHandle {
            socket,
            interval_id,
            _on_message: on_message,
            _on_close: on_close,
            _on_error: on_error,
            _pump: pump,
        },
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn request_response(lobby_url: &str, request: &LobbyRequest) -> Result<LobbyResponse, String> {
    use tungstenite::Message as WsMessage;
    use tungstenite::client::IntoClientRequest;

    let req = lobby_url
        .into_client_request()
        .map_err(|e| format!("bad lobby URL `{lobby_url}`: {e}"))?;
    let (mut ws, _resp) =
        tungstenite::connect(req).map_err(|e| format!("connect lobby `{lobby_url}`: {e}"))?;
    let body = serde_json::to_string(request).map_err(|e| format!("encode lobby request: {e}"))?;
    ws.send(WsMessage::Text(body.into()))
        .map_err(|e| format!("send lobby request: {e}"))?;

    loop {
        match ws.read().map_err(|e| format!("read lobby response: {e}"))? {
            WsMessage::Text(text) => {
                return serde_json::from_str(&text)
                    .map_err(|e| format!("decode lobby response `{text}`: {e}"));
            }
            WsMessage::Binary(bytes) => {
                return serde_json::from_slice(&bytes)
                    .map_err(|e| format!("decode binary lobby response: {e}"));
            }
            WsMessage::Close(_) => return Err("lobby closed before responding".to_string()),
            _ => {}
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn request_response(_lobby_url: &str, _request: &LobbyRequest) -> Result<LobbyResponse, String> {
    Err(
        "synchronous lobby request/response is unavailable in browsers; use LobbyClient::connect"
            .to_string(),
    )
}
