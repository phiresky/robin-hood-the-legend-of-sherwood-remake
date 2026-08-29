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

enum Command {
    CreateGame {
        mission_id: u32,
        mission_name: String,
    },
    JoinGame {
        game_id: String,
    },
    LeaveGame,
    StartGame,
}

/// Live matchmaking session: membership in the game-discovery gossip
/// swarm plus the local player's hosting / joining state.  Dropping
/// it leaves the swarm (the hosted listing expires from everyone's
/// browser within [`SOFT_STATE_TTL`]).
pub struct MatchmakingSession {
    commands: Sender<Command>,
    events: Receiver<MatchmakingEvent>,
}

impl MatchmakingSession {
    /// Join the matchmaking swarm.  Returns immediately; discovery
    /// progress arrives as [`MatchmakingEvent::Neighbors`] events.
    pub fn open(nickname: String) -> Result<Self, String> {
        open_native(nickname)
    }

    pub fn create_game(&self, mission_id: u32, mission_name: String) -> Result<(), String> {
        self.send(Command::CreateGame {
            mission_id,
            mission_name,
        })
    }

    pub fn join_game(&self, game_id: String) -> Result<(), String> {
        self.send(Command::JoinGame { game_id })
    }

    pub fn leave_game(&self) -> Result<(), String> {
        self.send(Command::LeaveGame)
    }

    pub fn start_game(&self) -> Result<(), String> {
        self.send(Command::StartGame)
    }

    pub fn try_recv(&self) -> Option<MatchmakingEvent> {
        self.events.try_recv().ok()
    }

    fn send(&self, command: Command) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|_| "matchmaking session is closed".to_string())
    }
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

#[cfg(target_arch = "wasm32")]
fn open_native(_nickname: String) -> Result<MatchmakingSession, String> {
    // TODO: browser matchmaking needs iroh's wasm support wired into
    // the wasm transport before this can come back to the web build.
    Err("multiplayer matchmaking is not available in browser builds".to_string())
}

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
        let bootstrap: Vec<EndpointId> = rendezvous
            .bootstrap_ids()
            .await
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
                Command::CreateGame {
                    mission_id,
                    mission_name,
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
                Command::JoinGame { game_id } => {
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
                Command::LeaveGame => {
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
                Command::StartGame => {
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
                        let start_at_epoch_ms = current_epoch_ms().saturating_add(START_DELAY_MS);
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
                let ids: Vec<EndpointId> = rendezvous
                    .bootstrap_ids()
                    .await
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
            connect_addr: listing.id.clone(),
            expected_players,
            start_at_epoch_ms: listing.start_at_epoch_ms,
        }
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    #[test]
    fn topic_messages_roundtrip_json() {
        let listing = GameListing {
            id: "abc".into(),
            mission_id: 3,
            mission_name: "Dem_Lei_MP".into(),
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
}
