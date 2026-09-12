//! Serverless multiplayer matchmaking over iroh-gossip.
//!
//! There is no broker anywhere.  Every player who opens the
//! multiplayer menu joins one well-known gossip topic; peers for the
//! topic are found through the BitTorrent Mainline DHT (see
//! [`super::rendezvous`]), so no address, server, or environment
//! variable is ever configured.
//!
//! Hosts periodically broadcast their game as a [`GameListing`]
//! (soft state — listings expire when the announcements stop).
//! Joiners broadcast their intent to join; the host counts them.
//! When the host starts the game it broadcasts the start signal with
//! the synchronized `start_at_epoch_ms`, and everyone launches the
//! actual game session through the normal `--server` / `--connect`
//! path against the host's game endpoint id.
//!
//! TODO: announcements are unauthenticated — any peer could announce
//! a game under another host's endpoint id.  Sign announcements with
//! the game identity key if this ever matters.

use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{Receiver, Sender};

pub const START_DELAY_MS: u64 = 1_500;

/// How often hosts re-announce and joiners re-signal, and how long
/// soft state lives without a refresh.
#[cfg(not(target_arch = "wasm32"))]
const BROADCAST_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
#[cfg(not(target_arch = "wasm32"))]
const SOFT_STATE_TTL: std::time::Duration = std::time::Duration::from_secs(8);

/// One advertised game, as seen in the browser list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameListing {
    /// The host's game endpoint id — doubles as the game id and as
    /// what joiners pass to `--connect`.
    pub id: String,
    pub mission_id: u32,
    pub mission_name: String,
    /// Exact bounded content identity for a custom mission. Bytes travel only
    /// over the authenticated game connection after Start.
    #[serde(default)]
    pub host_content: Option<robin_engine::multiplayer::DistributedModOffer>,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub players: u32,
    #[serde(default)]
    pub max_players: u32,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub start_at_epoch_ms: Option<u64>,
}

impl GameListing {
    /// The connect string joiners dial — the listing id *is* the
    /// host's game endpoint id.
    pub fn connect_addr(&self) -> &str {
        &self.id
    }
}

/// The launch handoff for a game the local player is part of.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinedGame {
    pub game_id: String,
    pub mission_id: u32,
    pub mission_name: String,
    #[serde(default)]
    pub host_content: Option<robin_engine::multiplayer::DistributedModOffer>,
    /// The host's game endpoint id — what joiners pass to `--connect`.
    pub connect_addr: String,
    #[serde(default = "default_expected_players")]
    pub expected_players: u32,
    #[serde(default)]
    pub start_at_epoch_ms: Option<u64>,
}

fn default_expected_players() -> u32 {
    1
}

/// Everything broadcast on the matchmaking topic.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TopicMsg {
    /// Host: my game exists, in this state, with this many players.
    Announce { game: GameListing },
    /// Joiner: I want a seat in this game (repeated while waiting).
    Join { game_id: String, nickname: String },
    /// Joiner: I backed out.
    Leave { game_id: String, nickname: String },
    /// Host: the game starts — connect and be ready at `start_at`.
    Start { game: JoinedGame },
}

#[derive(Debug, Clone)]
pub enum MatchmakingEvent {
    /// Fresh snapshot of every live listing.
    Games(Vec<GameListing>),
    /// The local player's game was created and is now being announced.
    Created(GameListing),
    /// The local player joined a game (start pending unless
    /// `start_at_epoch_ms` is already set).
    Joined(JoinedGame),
    /// The local host pressed Start.
    Started(JoinedGame),
    /// A listing the local player cares about changed.
    GameUpdated(GameListing),
    /// The host of the game the local player joined pressed Start.
    GameStarted(JoinedGame),
    /// Gossip swarm connectivity changed — `0` means still searching.
    Neighbors(usize),
    Error(String),
    Disconnected(String),
}

#[cfg(not(target_arch = "wasm32"))]
enum Command {
    Create {
        mission_id: u32,
        mission_name: String,
        host_content: Option<robin_engine::multiplayer::DistributedModOffer>,
    },
    Join {
        game_id: String,
    },
    Leave,
    Start,
}

/// Live matchmaking session: membership in the game-discovery gossip
/// swarm plus the local player's hosting / joining state.  Dropping
/// it leaves the swarm (the hosted listing expires from everyone's
/// browser within [`SOFT_STATE_TTL`]).
#[cfg(not(target_arch = "wasm32"))]
pub struct MatchmakingSession {
    commands: Sender<Command>,
    events: Receiver<MatchmakingEvent>,
    command_worker_closed: std::cell::Cell<bool>,
}

#[cfg(not(target_arch = "wasm32"))]
impl MatchmakingSession {
    /// Join the matchmaking swarm.  Returns immediately; discovery
    /// progress arrives as [`MatchmakingEvent::Neighbors`] events.
    pub fn open(nickname: String) -> Result<Self, String> {
        open_native(nickname)
    }

    pub fn create_game(&self, mission_id: u32, mission_name: String) -> Result<(), String> {
        self.send(Command::Create {
            mission_id,
            mission_name,
            host_content: None,
        })
    }

    pub fn create_game_with_content(
        &self,
        mission_id: u32,
        mission_name: String,
        host_content: robin_engine::multiplayer::DistributedModOffer,
    ) -> Result<(), String> {
        host_content.validate()?;
        self.send(Command::Create {
            mission_id,
            mission_name,
            host_content: Some(host_content),
        })
    }

    pub fn join_game(&self, game_id: String) -> Result<(), String> {
        self.send(Command::Join { game_id })
    }

    pub fn leave_game(&self) -> Result<(), String> {
        self.send(Command::Leave)
    }

    pub fn start_game(&self) -> Result<(), String> {
        self.send(Command::Start)
    }

    pub fn try_recv(&self) -> Result<Option<MatchmakingEvent>, String> {
        if self.command_worker_closed.get() {
            return Err("matchmaking command worker is closed".to_string());
        }
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("matchmaking worker is closed".to_string())
            }
        }
    }

    fn send(&self, command: Command) -> Result<(), String> {
        self.commands.send(command).map_err(|_| {
            self.command_worker_closed.set(true);
            "matchmaking session is closed".to_string()
        })
    }
}

/// Browser discovery is not implemented, so no live gossip session can exist.
/// Direct browser invites use the game transport instead of this session.
#[cfg(target_arch = "wasm32")]
pub enum MatchmakingSession {}

#[cfg(target_arch = "wasm32")]
impl MatchmakingSession {
    pub fn open(_nickname: String) -> Result<Self, String> {
        // TODO: wire browser gossip discovery into the wasm transport.
        Err("multiplayer matchmaking is not available in browser builds".to_string())
    }

    pub fn create_game(&self, _mission_id: u32, _mission_name: String) -> Result<(), String> {
        match *self {}
    }

    pub fn create_game_with_content(
        &self,
        _mission_id: u32,
        _mission_name: String,
        _host_content: robin_engine::multiplayer::DistributedModOffer,
    ) -> Result<(), String> {
        match *self {}
    }

    pub fn join_game(&self, _game_id: String) -> Result<(), String> {
        match *self {}
    }

    pub fn leave_game(&self) -> Result<(), String> {
        match *self {}
    }

    pub fn start_game(&self) -> Result<(), String> {
        match *self {}
    }

    pub fn try_recv(&self) -> Result<Option<MatchmakingEvent>, String> {
        match *self {}
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn checked_start_epoch_ms(now_epoch_ms: u64) -> Result<u64, String> {
    now_epoch_ms
        .checked_add(START_DELAY_MS)
        .ok_or_else(|| "matchmaking start timestamp exceeds the u64 Unix range".to_owned())
}

pub use super::clock::current_epoch_ms;
#[cfg(all(test, not(target_arch = "wasm32")))]
use super::clock::epoch_ms_at as native_epoch_ms_at;
#[cfg(not(target_arch = "wasm32"))]
use super::clock::try_current_epoch_ms;

#[cfg(not(target_arch = "wasm32"))]
fn open_native(nickname: String) -> Result<MatchmakingSession, String> {
    use std::sync::mpsc::channel;

    let (cmd_tx, cmd_rx) = channel::<Command>();
    let (event_tx, event_rx) = channel::<MatchmakingEvent>();
    std::thread::Builder::new()
        .name("mp-matchmaking".to_string())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = event_tx.send(MatchmakingEvent::Disconnected(format!(
                        "build tokio runtime: {e}"
                    )));
                    return;
                }
            };
            rt.block_on(native::run_worker(nickname, cmd_rx, event_tx));
        })
        .map_err(|e| format!("spawn matchmaking worker: {e}"))?;

    Ok(MatchmakingSession {
        command_worker_closed: std::cell::Cell::new(false),
        commands: cmd_tx,
        events: event_rx,
    })
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use crate::multiplayer::rendezvous::{ANNOUNCE_INTERVAL, TopicRendezvous};
    use futures::StreamExt;
    use iroh::EndpointId;
    use iroh_gossip::net::Gossip;
    use iroh_gossip::proto::TopicId;
    use sha2::Digest;
    use std::collections::HashMap;
    use std::sync::mpsc::TryRecvError;
    use std::time::Instant;

    /// Well-known topic every copy of the game rendezvouses on —
    /// public by design, the game list is public.
    const TOPIC: &str = "robinhood-legend-of-sherwood/matchmaking/0";

    const TICK: std::time::Duration = std::time::Duration::from_millis(250);

    /// The local player's current involvement.
    enum Role {
        Browsing,
        Hosting {
            game: GameListing,
            /// Joiner nickname → last time their Join signal was seen.
            joiners: HashMap<String, Instant>,
            /// Set once Start was pressed.
            started: Option<JoinedGame>,
        },
        Joined {
            game_id: String,
        },
    }

    struct Worker {
        nickname: String,
        role: Role,
        /// Live listings by game id, with the last refresh time.
        listings: HashMap<String, (GameListing, Instant)>,
        neighbors: usize,
        /// Mirrors `neighbors` for the DHT rendezvous task, which only
        /// re-bootstraps while the swarm is empty.
        neighbors_watch: tokio::sync::watch::Sender<usize>,
        events: Sender<MatchmakingEvent>,
        sender: iroh_gossip::api::GossipSender,
        last_broadcast: Instant,
        listings_dirty: bool,
    }

    pub(super) async fn run_worker(
        nickname: String,
        commands: std::sync::mpsc::Receiver<Command>,
        events: Sender<MatchmakingEvent>,
    ) {
        let endpoint = match crate::multiplayer::identity::bind_ephemeral_endpoint().await {
            Ok(endpoint) => endpoint,
            Err(e) => {
                let _ = events.send(MatchmakingEvent::Disconnected(e));
                return;
            }
        };
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let router = iroh::protocol::Router::builder(endpoint.clone())
            .accept(iroh_gossip::ALPN, gossip.clone())
            .spawn();

        let local_id = endpoint.secret_key().public();
        let rendezvous = match TopicRendezvous::new(TOPIC, *local_id.as_bytes()) {
            Ok(rendezvous) => rendezvous,
            Err(e) => {
                let _ = events.send(MatchmakingEvent::Disconnected(format!(
                    "start matchmaking rendezvous: {e:#}"
                )));
                let _ = router.shutdown().await;
                endpoint.close().await;
                return;
            }
        };
        let topic_id = TopicId::from_bytes(sha2::Sha256::digest(TOPIC.as_bytes()).into());
        let bootstrap_ids = match rendezvous.bootstrap_ids().await {
            Ok(ids) => ids,
            Err(error) => {
                let _ = events.send(MatchmakingEvent::Disconnected(format!(
                    "read matchmaking rendezvous clock: {error:#}"
                )));
                let _ = router.shutdown().await;
                endpoint.close().await;
                return;
            }
        };
        let bootstrap: Vec<EndpointId> = bootstrap_ids
            .iter()
            .filter_map(|id| EndpointId::from_bytes(id).ok())
            .collect();
        tracing::debug!(
            peers = bootstrap.len(),
            "matchmaking rendezvous bootstrap ids from DHT"
        );
        let topic = match gossip.subscribe(topic_id, bootstrap).await {
            Ok(topic) => topic,
            Err(e) => {
                let _ = events.send(MatchmakingEvent::Disconnected(format!(
                    "join matchmaking topic: {e}"
                )));
                let _ = router.shutdown().await;
                endpoint.close().await;
                return;
            }
        };
        let (sender, mut receiver) = topic.split();
        let (neighbors_watch, neighbors_rx) = tokio::sync::watch::channel(0usize);
        // Keep announcing on the DHT (and re-bootstrapping while the
        // swarm is empty) in the background.  The task dies with the
        // worker's runtime.
        tokio::spawn(rendezvous_loop(rendezvous, sender.clone(), neighbors_rx));
        let _ = events.send(MatchmakingEvent::Neighbors(0));

        let mut worker = Worker {
            nickname,
            role: Role::Browsing,
            listings: HashMap::new(),
            neighbors: 0,
            neighbors_watch,
            events,
            sender,
            last_broadcast: Instant::now() - BROADCAST_INTERVAL,
            listings_dirty: false,
        };

        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        'session: loop {
            tokio::select! {
                event = receiver.next() => {
                    match event {
                        Some(Ok(event)) => worker.handle_gossip_event(event).await,
                        Some(Err(e)) => {
                            let _ = worker.events.send(MatchmakingEvent::Disconnected(format!(
                                "matchmaking gossip stream ended: {e}"
                            )));
                            break 'session;
                        }
                        None => {
                            let _ = worker.events.send(MatchmakingEvent::Disconnected(
                                "matchmaking gossip stream closed".to_string(),
                            ));
                            break 'session;
                        }
                    }
                }
                _ = ticker.tick() => {
                    loop {
                        match commands.try_recv() {
                            Ok(command) => {
                                if !worker.handle_command(command).await {
                                    break 'session;
                                }
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => break 'session,
                        }
                    }
                    worker.tick().await;
                }
            }
        }

        // Tell the swarm we're gone before dropping the endpoint so
        // the soft state clears faster than the TTL.
        if let Role::Joined { game_id } = &worker.role {
            worker
                .broadcast(&TopicMsg::Leave {
                    game_id: game_id.clone(),
                    nickname: worker.nickname.clone(),
                })
                .await;
        }
        let _ = router.shutdown().await;
        endpoint.close().await;
        tracing::info!("matchmaking worker stopped");
    }

    impl Worker {
        async fn broadcast(&self, msg: &TopicMsg) {
            let bytes = match serde_json::to_vec(msg) {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::error!("encode matchmaking message: {e}");
                    return;
                }
            };
            if let Err(e) = self.sender.broadcast(bytes.into()).await {
                tracing::debug!("matchmaking broadcast failed (no neighbors yet?): {e}");
            }
        }

        async fn handle_gossip_event(&mut self, event: iroh_gossip::api::Event) {
            use iroh_gossip::api::Event;
            match event {
                Event::Received(msg) => match serde_json::from_slice::<TopicMsg>(&msg.content) {
                    Ok(msg) => self.handle_topic_msg(msg),
                    Err(e) => {
                        tracing::debug!("undecodable matchmaking message: {e}");
                    }
                },
                Event::NeighborUp(_) => {
                    self.neighbors = self.neighbors.saturating_add(1);
                    let _ = self.neighbors_watch.send(self.neighbors);
                    let _ = self
                        .events
                        .send(MatchmakingEvent::Neighbors(self.neighbors));
                }
                Event::NeighborDown(_) => {
                    self.neighbors = self.neighbors.saturating_sub(1);
                    let _ = self.neighbors_watch.send(self.neighbors);
                    let _ = self
                        .events
                        .send(MatchmakingEvent::Neighbors(self.neighbors));
                }
                Event::Lagged => {
                    tracing::warn!("matchmaking gossip lagged; soft state will resync");
                }
            }
        }

        fn handle_topic_msg(&mut self, msg: TopicMsg) {
            match msg {
                TopicMsg::Announce { game } => {
                    self.listings
                        .insert(game.id.clone(), (game.clone(), Instant::now()));
                    self.listings_dirty = true;
                    match &self.role {
                        Role::Joined { game_id } if *game_id == game.id => {
                            // The host's announce doubles as the start
                            // signal in case the Start broadcast was
                            // missed.
                            if game.state == "started" && game.start_at_epoch_ms.is_some() {
                                let _ = self.events.send(MatchmakingEvent::GameStarted(
                                    joined_from_listing(&game, game.players),
                                ));
                            } else {
                                let _ = self.events.send(MatchmakingEvent::GameUpdated(game));
                            }
                        }
                        _ => {}
                    }
                }
                TopicMsg::Join { game_id, nickname } => {
                    if let Role::Hosting { game, joiners, .. } = &mut self.role
                        && game.id == game_id
                        && nickname != self.nickname
                    {
                        joiners.insert(nickname, Instant::now());
                    }
                }
                TopicMsg::Leave { game_id, nickname } => {
                    if let Role::Hosting { game, joiners, .. } = &mut self.role
                        && game.id == game_id
                    {
                        joiners.remove(&nickname);
                    }
                }
                TopicMsg::Start { game } => {
                    if let Role::Joined { game_id } = &self.role
                        && *game_id == game.game_id
                    {
                        let _ = self.events.send(MatchmakingEvent::GameStarted(game));
                    }
                }
            }
        }

        /// Returns `false` when the worker should shut down.
        async fn handle_command(&mut self, command: Command) -> bool {
            match command {
                Command::Create {
                    mission_id,
                    mission_name,
                    host_content,
                } => {
                    let id = match crate::multiplayer::identity::local_endpoint_id_string() {
                        Ok(id) => id,
                        Err(e) => {
                            let _ = self.events.send(MatchmakingEvent::Error(e));
                            return true;
                        }
                    };
                    let game = GameListing {
                        id,
                        mission_id,
                        mission_name,
                        host_content,
                        host: self.nickname.clone(),
                        players: 1,
                        max_players: 4,
                        state: "waiting".to_string(),
                        start_at_epoch_ms: None,
                    };
                    self.role = Role::Hosting {
                        game: game.clone(),
                        joiners: HashMap::new(),
                        started: None,
                    };
                    self.broadcast(&TopicMsg::Announce { game: game.clone() })
                        .await;
                    self.last_broadcast = Instant::now();
                    let _ = self.events.send(MatchmakingEvent::Created(game));
                }
                Command::Join { game_id } => {
                    let Some((listing, _)) = self.listings.get(&game_id) else {
                        let _ = self.events.send(MatchmakingEvent::Error(format!(
                            "game `{game_id}` is no longer advertised"
                        )));
                        return true;
                    };
                    let joined = joined_from_listing(listing, listing.players.saturating_add(1));
                    self.role = Role::Joined {
                        game_id: game_id.clone(),
                    };
                    self.broadcast(&TopicMsg::Join {
                        game_id,
                        nickname: self.nickname.clone(),
                    })
                    .await;
                    self.last_broadcast = Instant::now();
                    let _ = self.events.send(MatchmakingEvent::Joined(joined));
                }
                Command::Leave => {
                    if let Role::Joined { game_id } = &self.role {
                        self.broadcast(&TopicMsg::Leave {
                            game_id: game_id.clone(),
                            nickname: self.nickname.clone(),
                        })
                        .await;
                    }
                    // A host backing out simply stops announcing; the
                    // listing expires from every browser via the TTL.
                    self.role = Role::Browsing;
                }
                Command::Start => {
                    let (joined, announce) = {
                        let Role::Hosting {
                            game,
                            joiners,
                            started,
                        } = &mut self.role
                        else {
                            let _ = self.events.send(MatchmakingEvent::Error(
                                "only a hosting player can start the game".to_string(),
                            ));
                            return true;
                        };
                        let start_at_epoch_ms =
                            match try_current_epoch_ms().and_then(checked_start_epoch_ms) {
                                Ok(start_at_epoch_ms) => start_at_epoch_ms,
                                Err(error) => {
                                    let _ = self.events.send(MatchmakingEvent::Error(format!(
                                        "cannot start multiplayer game: {error}"
                                    )));
                                    return true;
                                }
                            };
                        game.state = "started".to_string();
                        game.start_at_epoch_ms = Some(start_at_epoch_ms);
                        game.players = 1 + joiners.len() as u32;
                        let joined = joined_from_listing(game, game.players);
                        *started = Some(joined.clone());
                        (joined, game.clone())
                    };
                    // Send the explicit signal several times right away
                    // (gossip is fire-and-forget); the periodic
                    // started-state Announce is the fallback path.
                    for _ in 0..3 {
                        self.broadcast(&TopicMsg::Start {
                            game: joined.clone(),
                        })
                        .await;
                    }
                    self.broadcast(&TopicMsg::Announce { game: announce }).await;
                    self.last_broadcast = Instant::now();
                    let _ = self.events.send(MatchmakingEvent::Started(joined));
                }
            }
            true
        }

        async fn tick(&mut self) {
            let now = Instant::now();

            // Expire listings that stopped being announced.
            let before = self.listings.len();
            self.listings
                .retain(|_, (_, seen)| now.duration_since(*seen) < SOFT_STATE_TTL);
            if self.listings.len() != before {
                self.listings_dirty = true;
            }

            if now.duration_since(self.last_broadcast) >= BROADCAST_INTERVAL {
                self.last_broadcast = now;
                let mut outgoing: Vec<TopicMsg> = Vec::new();
                match &mut self.role {
                    Role::Browsing => {}
                    Role::Hosting {
                        game,
                        joiners,
                        started,
                    } => {
                        joiners.retain(|_, seen| now.duration_since(*seen) < SOFT_STATE_TTL);
                        let players = 1 + joiners.len() as u32;
                        if players != game.players {
                            game.players = players;
                            let _ = self
                                .events
                                .send(MatchmakingEvent::GameUpdated(game.clone()));
                        }
                        outgoing.push(TopicMsg::Announce { game: game.clone() });
                        if let Some(joined) = started {
                            outgoing.push(TopicMsg::Start {
                                game: joined.clone(),
                            });
                        }
                    }
                    Role::Joined { game_id } => {
                        outgoing.push(TopicMsg::Join {
                            game_id: game_id.clone(),
                            nickname: self.nickname.clone(),
                        });
                    }
                }
                for msg in &outgoing {
                    self.broadcast(msg).await;
                }
            }

            if self.listings_dirty {
                self.listings_dirty = false;
                let mut games: Vec<GameListing> = self
                    .listings
                    .values()
                    .map(|(game, _)| game.clone())
                    .collect();
                games.sort_by(|a, b| a.id.cmp(&b.id));
                let _ = self.events.send(MatchmakingEvent::Games(games));
            }
        }
    }

    /// Background DHT presence: keep the local endpoint id announced in
    /// the rendezvous slot, and while the gossip swarm has no neighbors
    /// keep pulling fresh ids from the DHT and asking gossip to join
    /// them.  Runs until the worker's runtime is dropped.
    async fn rendezvous_loop(
        rendezvous: TopicRendezvous,
        sender: iroh_gossip::api::GossipSender,
        neighbors: tokio::sync::watch::Receiver<usize>,
    ) {
        loop {
            if let Err(e) = rendezvous.announce().await {
                tracing::debug!("matchmaking rendezvous announce failed: {e:#}");
            }
            if *neighbors.borrow() == 0 {
                let bootstrap_ids = match rendezvous.bootstrap_ids().await {
                    Ok(ids) => ids,
                    Err(error) => {
                        tracing::debug!(
                            "matchmaking rendezvous clock unavailable; skipping DHT bootstrap: {error:#}"
                        );
                        tokio::time::sleep(ANNOUNCE_INTERVAL).await;
                        continue;
                    }
                };
                let ids: Vec<EndpointId> = bootstrap_ids
                    .iter()
                    .filter_map(|id| EndpointId::from_bytes(id).ok())
                    .collect();
                if !ids.is_empty()
                    && let Err(e) = sender.join_peers(ids).await
                {
                    tracing::debug!("matchmaking rendezvous join_peers failed: {e}");
                }
            }
            tokio::time::sleep(ANNOUNCE_INTERVAL).await;
        }
    }

    fn joined_from_listing(listing: &GameListing, expected_players: u32) -> JoinedGame {
        JoinedGame {
            game_id: listing.id.clone(),
            mission_id: listing.mission_id,
            mission_name: listing.mission_name.clone(),
            host_content: listing.host_content.clone(),
            connect_addr: listing.id.clone(),
            expected_players,
            start_at_epoch_ms: listing.start_at_epoch_ms,
        }
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn worker_closure_is_distinct_from_idle_and_commands_fail() {
        let (commands, receiver) = std::sync::mpsc::channel();
        let (sender, events) = std::sync::mpsc::channel();
        let session = super::MatchmakingSession {
            commands,
            events,
            command_worker_closed: std::cell::Cell::new(false),
        };
        assert!(session.try_recv().unwrap().is_none());
        sender.send(super::MatchmakingEvent::Neighbors(1)).unwrap();
        drop(sender);
        assert!(matches!(
            session.try_recv().unwrap(),
            Some(super::MatchmakingEvent::Neighbors(1))
        ));
        assert!(session.try_recv().is_err());
        drop(receiver);
        assert!(session.start_game().is_err());
        assert!(session.leave_game().is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn failed_command_closes_poll_even_if_event_sender_stays_alive() {
        let (commands, receiver) = std::sync::mpsc::channel();
        let (_sender, events) = std::sync::mpsc::channel();
        let session = super::MatchmakingSession {
            commands,
            events,
            command_worker_closed: std::cell::Cell::new(false),
        };
        assert!(session.try_recv().unwrap().is_none());
        drop(receiver);
        assert!(session.start_game().is_err());
        assert!(session.try_recv().is_err());
    }

    use super::*;

    #[test]
    fn topic_messages_roundtrip_json() {
        let listing = GameListing {
            id: "abc".into(),
            mission_id: 3,
            mission_name: "Dem_Lei_MP".into(),
            host_content: None,
            host: "robin".into(),
            players: 2,
            max_players: 4,
            state: "waiting".into(),
            start_at_epoch_ms: None,
        };
        let announce = TopicMsg::Announce {
            game: listing.clone(),
        };
        let bytes = serde_json::to_vec(&announce).expect("encode");
        match serde_json::from_slice::<TopicMsg>(&bytes).expect("decode") {
            TopicMsg::Announce { game } => {
                assert_eq!(game.id, listing.id);
                assert_eq!(game.players, 2);
            }
            other => panic!("wrong variant {other:?}"),
        }
    }

    #[test]
    fn epoch_conversion_rejects_pre_epoch_clocks() {
        let before_epoch = std::time::UNIX_EPOCH
            .checked_sub(std::time::Duration::from_millis(1))
            .expect("one millisecond before Unix epoch is representable");
        let error = native_epoch_ms_at(before_epoch).expect_err("pre-epoch clock must fail");
        assert!(error.contains("precedes the Unix epoch"), "{error}");
    }

    #[test]
    fn epoch_conversion_preserves_milliseconds() {
        let after_epoch = std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_234);
        assert_eq!(native_epoch_ms_at(after_epoch).expect("valid clock"), 1_234);
    }

    #[test]
    fn start_timestamp_rejects_overflow_instead_of_saturating() {
        let error = checked_start_epoch_ms(u64::MAX)
            .expect_err("overflowing matchmaking start time must fail");
        assert!(error.contains("exceeds the u64 Unix range"), "{error}");
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod browser_tests {
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn discovery_rejects_open_without_creating_a_session() {
        match super::MatchmakingSession::open("Browser player".into()) {
            Err(error) => assert_eq!(
                error,
                "multiplayer matchmaking is not available in browser builds"
            ),
            Ok(session) => match session {},
        }
    }
}
