//! Launch configuration (process lifetime) and per-mission launch requests.
//!
//! [`LaunchConfig`] is built once per run from the parsed CLI/URL
//! configuration and the application services, then shared by `Arc`.
//! [`MissionRequest`] is constructed for every mission launch and carries only
//! the transient per-launch state: the replay selection, custom-mission
//! content with its admitted asset lease, the multiplayer route, and restart
//! evidence. Launch paths build a fresh request (or derive one through a named
//! transition below) instead of cloning and patching a shared launch.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::cli::{CliArgs, options_from_args};
use crate::host::ApplicationContext;
use robin_engine::replay as engine_replay;

/// One-shot mission-start capture requested by developer tools
/// (`render_mission_map`, Original parity frame-zero capture). Deliberately
/// not a launcher flag; process lifetime, so replays keep the same policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MissionStartCapture {
    /// The mission session captures the complete level through the regular
    /// screenshot machinery, writes it here, and exits.
    pub map_output: Option<PathBuf>,
    /// Absolute simulation frame for `map_output`. Frame zero is the
    /// post-`Initialize`, pre-tick state.
    pub map_frame: u32,
    /// Apply the original `UBIQUITY` / `UNBLIP` reveal-all-NPCs cheat to the
    /// one-shot mission-start map before it is rendered.
    pub reveal_all: bool,
    /// Include gameplay fog in a one-shot mission map export. Full-map exports
    /// default this off so tool output remains complete.
    pub fog_of_war: bool,
    /// Parity capture mode: unlike the map exporter, this captures the saved
    /// viewport and includes the ordinary gameplay HUD.
    pub viewport_capture: bool,
    /// Exact Original v48 save bytes to adopt after constructing the mission
    /// topology and before the one-shot frame-zero capture.
    pub legacy_save: Option<Vec<u8>>,
    /// Preserve the caller-supplied campaign when `--mission` selects the
    /// one-shot capture mission. Parity captures supply the exact recorded
    /// roster and campaign state instead of the map exporter's representative
    /// team.
    pub preserve_forced_mission_campaign: bool,
}

/// Immutable process-lifetime launch configuration, shared by `Arc`.
///
/// The CLI fields that a launch may override per mission (`replay`,
/// `custom_mission`, `start_paused` and the multiplayer route) seed each
/// [`MissionRequest`]; mission code must read those from the request, never
/// from `cli`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(from = "Configuration")]
pub struct LaunchConfig {
    /// Raw CLI/URL configuration, without process or mission handoffs.
    #[serde(rename = "config")]
    pub cli: CliArgs,
    /// Process services and startup options bound to this run. Parsed options
    /// seed a bootstrap context; application startup installs readiness.
    #[serde(skip)]
    pub global_options: ApplicationContext,
    /// Browser shell attestation that this durable local identity previously
    /// redeemed the invitation. The host remains authoritative.
    #[serde(skip)]
    pub browser_join_redeemed: bool,
    /// Developer-tool one-shot capture policy.
    #[serde(skip)]
    pub capture: MissionStartCapture,
}

/// Serialized form of a [`LaunchConfig`]. Decoding restores only the raw
/// configuration and recomputes its effective startup options; defaulting a
/// skipped context would silently discard nondefault flags. No services,
/// capture/export authority or decoded payloads are ever restored.
///
/// The name is part of serde's error text and is kept stable.
#[derive(Serialize, Deserialize)]
struct Configuration {
    #[serde(default)]
    config: CliArgs,
}

impl From<Configuration> for LaunchConfig {
    fn from(decoded: Configuration) -> Self {
        Self::from(decoded.config)
    }
}

impl From<CliArgs> for LaunchConfig {
    fn from(cli: CliArgs) -> Self {
        // Derive startup policy without reinstalling process-wide options or
        // constructing a discarded default configuration.
        let global_options = ApplicationContext::bootstrap(options_from_args(&cli));
        Self {
            cli,
            global_options,
            browser_join_redeemed: false,
            capture: MissionStartCapture::default(),
        }
    }
}

impl LaunchConfig {
    /// Install a developer-tool capture policy while building the config.
    pub fn with_mission_start_capture(self, capture: MissionStartCapture) -> Self {
        Self { capture, ..self }
    }

    /// Bind this launcher configuration to the run's application services.
    /// This is the single construction of a run's shared configuration; the
    /// (possibly join-resolved) CLI replaces the launcher's.
    pub(super) fn bind_run(
        &self,
        global_options: ApplicationContext,
        cli: CliArgs,
        browser_join_redeemed: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            cli,
            global_options,
            browser_join_redeemed,
            capture: self.capture.clone(),
        })
    }
}

/// Launch-supplied custom-mission content and its admitted asset lease.
/// Replaced as a unit whenever a replay or save becomes the content authority.
#[derive(Debug, Default, Serialize)]
pub struct MissionContent {
    /// Direct `--custom-mission` archive path, admitted before construction.
    pub custom_mission: Option<PathBuf>,
    /// Internal handoff from the custom-mission picker. Spellforge-tagged
    /// launches carry the bits needed to construct a required `LuaSession`;
    /// Vanilla-tagged custom missions carry the same launch metadata but
    /// intentionally produce no Lua state. `None` for every non-mod launch.
    #[serde(skip)]
    pub pending_lua_mission: Option<PendingLuaMission>,
    /// Exact canonical full-mod envelope used by a custom multiplayer host or
    /// joiner. The host distributes these bytes before Welcome; the joiner
    /// keeps the independently validated cache lease/mount in HostTransport.
    #[serde(skip)]
    pub pending_distributed_mod: Option<Arc<[u8]>>,
    /// Exact admitted asset mount, retained throughout the session and its
    /// same-mission restarts. Persisted replay/save formats retain their own
    /// executable packages; this lease owns only process-local mount lifetimes.
    #[serde(skip)]
    pub resolved_mission_assets: Option<Arc<crate::mission_asset_restore::ResolvedMissionAssets>>,
}

robin_util::deny_deserialize!(
    MissionContent,
    "mission content leases are installed in process and cannot be deserialized"
);

impl MissionContent {
    /// Content of a preflighted save or admitted replay: the persisted
    /// descriptor's exact resolved assets are the sole authority; no
    /// launch-supplied custom mission or package survives.
    pub(crate) fn exact_assets(
        resolved: Arc<crate::mission_asset_restore::ResolvedMissionAssets>,
    ) -> Self {
        Self {
            resolved_mission_assets: Some(resolved),
            ..Self::default()
        }
    }
}

/// Multiplayer route of one launch: CLI-seeded, replaced by the lobby.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MultiplayerRoute {
    /// Canonical host-signed browser invitation (`--join`).
    pub join: Option<String>,
    /// Host this launch (`--server`).
    pub server: bool,
    /// Host endpoint to join (`--connect`).
    pub connect: Option<String>,
    /// Matchmaking handoff: keep the simulation paused until this time.
    pub start_at_epoch_ms: Option<u64>,
    /// Total player count the host waits for at the ready barrier.
    pub expected_players: Option<u32>,
    /// Selected mission profile of a lobby launch.
    pub mission_profile_id: Option<u32>,
    /// Internal outer-mission handoff. A replacement host consumes the exact
    /// authenticated session/seat roster retained by the previous transport.
    #[serde(skip)]
    pub continue_session: bool,
}

impl MultiplayerRoute {
    fn from_cli(cli: &CliArgs) -> Self {
        Self {
            join: cli.join.clone(),
            server: cli.server,
            connect: cli.connect.clone(),
            start_at_epoch_ms: cli.mp_start_at_epoch_ms,
            expected_players: cli.mp_expected_players,
            mission_profile_id: cli.mp_mission_profile_id,
            continue_session: false,
        }
    }
}

/// Per-mission launch request. Built fresh for every launch from the shared
/// [`LaunchConfig`]; session transitions derive the next request by value.
///
/// Serialization is diagnostic: decoding accepts exactly the
/// [`LaunchConfig`] form and yields an unprepared request seeded from it.
#[derive(Debug, Serialize, Deserialize)]
#[serde(from = "LaunchConfig")]
pub struct MissionRequest {
    /// Shared process-lifetime configuration.
    #[serde(flatten)]
    pub config: Arc<LaunchConfig>,
    /// Undecoded replay input (path or compact string) of this launch.
    pub replay: Option<String>,
    /// Already-decoded replay, installed before engine construction so the
    /// canonical header supplies the initial world, seed and simulation config.
    #[serde(skip)]
    pub replay_data: Option<engine_replay::ReplayData>,
    /// Open the mission with the simulation paused.
    pub start_paused: bool,
    /// Custom-mission content and its asset lease.
    pub content: MissionContent,
    /// Multiplayer route and outer-mission session continuation.
    pub multiplayer: MultiplayerRoute,
    /// Evidence for this cold reconstruction; reset for unrelated launches.
    #[serde(skip)]
    pub mission_restart: bool,
}

impl From<LaunchConfig> for MissionRequest {
    fn from(config: LaunchConfig) -> Self {
        Self::new(Arc::new(config))
    }
}

#[cfg(test)]
impl Default for MissionRequest {
    fn default() -> Self {
        Self::new(Arc::default())
    }
}

impl MissionRequest {
    /// The unprepared launch the configuration describes: its CLI replay,
    /// custom mission, pause and multiplayer route, with no handoffs.
    pub fn new(config: Arc<LaunchConfig>) -> Self {
        Self {
            replay: config.cli.replay.clone(),
            replay_data: None,
            start_paused: config.cli.start_paused,
            content: MissionContent {
                custom_mission: config.cli.custom_mission.clone(),
                ..MissionContent::default()
            },
            multiplayer: MultiplayerRoute::from_cli(&config.cli),
            mission_restart: false,
            config,
        }
    }

    /// Start a canonical replay request without cloning the previous
    /// recording or retaining its admitted custom content. Capture/process
    /// policy (the shared config), the multiplayer route with its session
    /// continuation, and a requested pause intentionally survive, just as they
    /// do across startup.
    pub(crate) fn for_replay(&self, data: engine_replay::ReplayData, paused: bool) -> Self {
        Self {
            config: Arc::clone(&self.config),
            replay: None,
            replay_data: Some(data),
            start_paused: self.start_paused || paused,
            content: MissionContent::default(),
            multiplayer: self.multiplayer.clone(),
            mission_restart: false,
        }
    }

    /// The same launch with its content authority replaced as a unit.
    pub(crate) fn with_content(self, content: MissionContent) -> Self {
        Self { content, ..self }
    }

    /// The same launch with an explicit pause policy (an RPC replay's).
    pub(crate) fn with_start_paused(self, start_paused: bool) -> Self {
        Self {
            start_paused,
            ..self
        }
    }

    /// Re-enter the same exact mission after `LevelRestart`.
    pub(crate) fn restarting(self) -> Self {
        Self {
            mission_restart: true,
            ..self
        }
    }

    /// Restart evidence belongs to the just-consumed reconstruction only.
    pub(crate) fn after_attempt(self) -> Self {
        Self {
            mission_restart: false,
            ..self
        }
    }

    /// Release the process-local asset lease of a completed mission. The
    /// custom-mission launch metadata (path, Lua/distributed package handoff)
    /// is deliberately carried to the next campaign mission, as before.
    pub(crate) fn releasing_asset_lease(self) -> Self {
        Self {
            content: MissionContent {
                resolved_mission_assets: None,
                ..self.content
            },
            ..self
        }
    }

    /// Outer-mission transition of a campaign session: a host continues its
    /// authenticated transport session; any other role does not.
    pub(crate) fn continuing_host_session(self) -> Self {
        let continue_session = self.multiplayer.server;
        self.with_session_continuation(continue_session)
    }

    /// Admit (or keep) the outer-mission session continuation.
    pub(crate) fn with_session_continuation(self, continue_session: bool) -> Self {
        Self {
            multiplayer: MultiplayerRoute {
                continue_session,
                ..self.multiplayer
            },
            ..self
        }
    }
}

pub(super) fn requested_replay_data(
    request: &MissionRequest,
) -> Result<Option<engine_replay::ReplayData>, super::LaunchError> {
    if let Some(data) = request.replay_data.clone() {
        return Ok(Some(data));
    }
    request
        .replay
        .as_deref()
        .map(crate::replay_format::load_replay_spec)
        .transpose()
        .map_err(|error| super::LaunchError::replay(format!("failed to load replay: {error}")))
}

/// Exact executable-package handoff for one live custom mission. Archive
/// identity/mount lifetime lives in `resolved_mission_assets`; this structure
/// never carries a path which Lua startup could reopen.
#[derive(Debug, Clone)]
pub struct PendingLuaMission {
    pub rhm_basename: String,
    pub requires_spellforge: bool,
    /// Exact already-admitted package supplied by live, replay, save, or
    /// multiplayer preparation. Startup never consults a local library.
    pub spellforge_package: Option<robin_engine::spellforge::SpellforgePackage>,
}
