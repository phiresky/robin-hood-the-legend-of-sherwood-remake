//! Command-line argument parsing (and the wasm URL-query equivalent).

use std::ffi::OsString;

use clap::Parser;
use serde::{Deserialize, Serialize};

use robin_engine::engine as engine_api;

/// Extension required for replay files — keeps the format searchable
/// and future-proofs us if we ever want to associate `.rhrec.jsonl` with
/// a handler at the OS level.
pub const RHREC_EXT: &str = ".rhrec.jsonl";

/// Mission directories contain append-only replay chunks and their chronology.
fn parse_record_path(s: &str) -> Result<String, String> {
    if s.trim().is_empty() {
        Err("record directory must not be empty".into())
    } else {
        Ok(s.to_owned())
    }
}

/// Replay artifacts are binary files; developer recordings may also name an archive.
fn parse_replay_spec(s: &str) -> Result<String, String> {
    if s.is_empty() {
        return Err("replay path must not be empty".into());
    }
    Ok(s.to_owned())
}

/// Robin Hood — The Legend of Sherwood (Rust port)
#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
#[command(version, about)]
#[serde(default, rename_all = "kebab-case")]
pub struct CliArgs {
    /// Disable audio playback.
    #[arg(long)]
    pub no_sound: bool,

    /// Disable mission script execution.
    #[arg(long)]
    pub no_script: bool,

    /// GoldenEye mode: NPCs cannot see player characters
    #[arg(long)]
    pub goldeneye: bool,

    /// Spawn enemy NPCs as invulnerable.
    #[arg(long)]
    pub highlander2: bool,

    /// Show the AI "whatsup" debug overlay.
    #[arg(long)]
    pub whatsup: bool,

    /// Ignore the default mission-lost condition.
    #[arg(long)]
    pub no_default_loose: bool,

    /// Validate cached sound data during startup.
    #[arg(long)]
    pub check_sound_data: bool,

    /// Show view cones for all NPCs at all times
    #[arg(long)]
    pub view_cones: bool,

    /// Show the debug-surfaces overlay (walkable motion areas + selected
    /// character's surface and committed path).  Toggle at runtime with
    /// the `SURFACE` console command.
    #[arg(long)]
    pub debug_surfaces: bool,

    /// Record this mission in a new directory containing replay chunks
    #[arg(long, value_parser = parse_record_path)]
    pub record: Option<String>,

    /// Play back a replay. Accepts any of:
    ///   - a binary `.rhrec` file,
    ///   - a mission recording directory (or any chunk within it),
    ///   - a legacy `*.rhrec.jsonl` recording.
    ///
    /// The replay's header picks the mission to load.
    #[arg(long, value_parser = parse_replay_spec)]
    pub replay: Option<String>,

    /// Replay an existing recording twice and publish upgraded engine hashes.
    #[arg(long, value_name = "INPUT", requires = "upgraded_replay", conflicts_with_all = ["replay", "record", "mission", "server", "connect", "join"])]
    #[serde(skip)]
    pub upgrade_replay: Option<std::path::PathBuf>,

    /// New JSONL recording to publish after successful replay verification.
    #[arg(long, value_name = "OUTPUT", requires = "upgrade_replay")]
    #[serde(skip)]
    pub upgraded_replay: Option<std::path::PathBuf>,

    /// Internal replay-upgrade worker output; records actual boundary hashes.
    #[arg(long, hide = true, requires_all = ["headless", "replay"])]
    #[serde(skip)]
    pub replay_hash_output: Option<std::path::PathBuf>,

    /// Runtime rollback consistency checker: rewind a short window of
    /// engine state and re-simulate it to detect desyncs.
    /// On by default — pass `--rollback-check=false` to disable.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub rollback_check: bool,

    /// Skip the main menu and drop directly into the Sherwood (HQ)
    /// mission — useful for iterating on the Sherwood HUD without
    /// clicking through the menu + campaign-map flow.
    #[arg(long)]
    pub sherwood: bool,

    /// Force the graphical main menu even when a demo data directory is
    /// detected.  Demo datadirs normally auto-start their bundled mission.
    #[arg(long)]
    pub force_main_menu: bool,

    /// Skip the menus and launch this mission filename directly, like
    /// the original launcher's `-MISSION`. Pass the base name without
    /// `.rhm`; when `--proto` is omitted, the mission name is also used
    /// as the proto-level name.
    #[arg(
        long,
        value_name = "MISSION",
        conflicts_with_all = ["sherwood", "replay", "wait_for_command"]
    )]
    pub mission: Option<String>,

    /// Proto-level filename to use with `--mission`, like the original
    /// launcher's `-PROTO`. Pass the base name without `.rhp`. When omitted,
    /// known missions use their profile mapping; unknown/custom missions use
    /// the mission filename as the proto filename.
    #[arg(long, value_name = "PROTO", requires = "mission")]
    pub proto: Option<String>,

    /// Mount a vanilla custom-mission zip before launching `--mission`.
    /// This is the command-line equivalent of selecting that archive in the
    /// Custom Missions menu. Spellforge missions must still use the menu so
    /// their shared Lua library and compatibility mode are detected.
    #[arg(long, value_name = "ZIP", requires = "mission")]
    pub custom_mission: Option<std::path::PathBuf>,

    /// Exact `.rhm` entry inside `--custom-mission`. Optional only when the
    /// archive contains one unambiguous entry matching `--mission`.
    #[arg(long, value_name = "ARCHIVE_ENTRY", requires = "custom_mission")]
    pub custom_mission_entry: Option<String>,

    /// TCP port for the local script-RPC HTTP server.
    /// Default 17640 (loopback only). Set to 0 to disable.
    /// See `crate::http_server` for the wire format.
    #[cfg_attr(any(feature = "script-rpc", target_arch = "wasm32"), arg(long, default_value_t = crate::http_server::DEFAULT_PORT))]
    #[cfg_attr(
        all(not(feature = "script-rpc"), not(target_arch = "wasm32")),
        arg(skip = 0_u16)
    )]
    pub http_server: u16,

    /// Run the frame loop with no 25 fps pacing sleep or VSync — ticks and
    /// renders happen back-to-back at full CPU/GPU speed.  Useful for
    /// automated tests, replay scrubbing, and profiling.  Independent
    /// of the in-game fast-forward toggle (which also skips rendering);
    /// with this flag rendering still runs every frame.
    #[arg(long)]
    pub fast_forward: bool,

    /// Skip the per-frame render pass entirely: no `pre_render` GPU
    /// drains, no scene draw, no cursor update, no `present()`. The winit
    /// window and wgpu context are still created so input/events continue to
    /// flow, but no pixels are produced. Implies no pacing sleep — the loop
    /// runs at full CPU speed, just like `--fast-forward`. Useful for replay
    /// scrubbing, automated tests, and profiling simulation throughput.
    #[arg(long)]
    pub headless: bool,

    /// Open the mission with the simulation paused — the engine tick is
    /// suspended until a `/step-forward` HTTP request (or any other
    /// path that flips `pause` off) drives it forward.  Rendering, HUD,
    /// and input still run normally; the pause menu is not shown.
    /// Useful for scripted test drivers that want full control over
    /// when frames advance.
    #[arg(long)]
    pub start_paused: bool,

    /// Finish data load, then idle on a "waiting for command" loading
    /// screen until the script-RPC `load-replay` endpoint queues a
    /// replay.  The replay's header picks the mission; no auto-start
    /// (demo detection, `--sherwood`, main menu) fires.  Used by the
    /// wasm host so URL-driven replay load isn't racing the
    /// auto-start — JS needs a window after Rust init to send
    /// `load-replay` before a mission gets to consume the pending
    /// slot.
    #[arg(long)]
    pub wait_for_command: bool,

    /// Run as a multiplayer server on this install's persistent iroh
    /// identity.  Peers connect to the endpoint id logged at startup
    /// (no port forwarding or bind address needed).  This process
    /// drives seat 0 (`PlayerId::HOST`); peers receive `PlayerId(1+)`
    /// in join order.
    ///
    /// Mutually exclusive with `--connect`.
    #[arg(long)]
    pub server: bool,

    /// Run as a multiplayer client, connecting to the host's iroh
    /// endpoint id.  The server assigns a join-order seat which the
    /// client then drives for the rest of the session.
    ///
    /// Mutually exclusive with `--server`.
    #[arg(long, value_name = "ENDPOINT_ID")]
    pub connect: Option<String>,

    /// Join from a canonical host-signed browser invitation.
    #[arg(long, value_name = "RHMP2_TICKET")]
    pub join: Option<String>,

    /// Internal matchmaking handoff: keep the simulation paused until this
    /// wall-clock timestamp so host and joiners begin together.
    #[arg(long, hide = true)]
    pub mp_start_at_epoch_ms: Option<u64>,

    /// Internal matchmaking handoff: total player count the host should wait
    /// for at the multiplayer ready barrier.
    #[arg(long, hide = true)]
    pub mp_expected_players: Option<u32>,

    /// Internal multiplayer-menu handoff for the selected mission profile.
    #[arg(long, hide = true)]
    pub mp_mission_profile_id: Option<u32>,

    /// Override the active profile's browser-invitation publication setting
    /// for this hosted game. Omitted means use the saved preference.
    #[arg(long, action = clap::ArgAction::Set)]
    pub mp_browser_join_links: Option<bool>,

    /// Nickname shown in the portrait "controlled by" overlay on
    /// peers.  Defaults to a host-name-derived fallback when omitted.
    #[arg(long, value_name = "NICKNAME", default_value = "")]
    pub mp_nickname: String,
}

impl CliArgs {
    /// Resolve live presentation without modifying the saved graphics setting.
    /// Fast-forward must bypass swapchain waiting as well as the tick sleep.
    pub(crate) fn native_refresh_presentation(&self, preference: bool) -> bool {
        preference && !self.fast_forward
    }
}

impl Default for CliArgs {
    fn default() -> Self {
        let args = Self {
            no_sound: false,
            no_script: false,
            goldeneye: false,
            highlander2: false,
            whatsup: false,
            no_default_loose: false,
            check_sound_data: false,
            view_cones: false,
            debug_surfaces: false,
            record: None,
            replay: None,
            upgrade_replay: None,
            upgraded_replay: None,
            replay_hash_output: None,
            rollback_check: true,
            sherwood: false,
            force_main_menu: false,
            mission: None,
            proto: None,
            custom_mission: None,
            custom_mission_entry: None,
            http_server: if cfg!(any(feature = "script-rpc", target_arch = "wasm32")) {
                crate::http_server::DEFAULT_PORT
            } else {
                0
            },
            fast_forward: false,
            headless: false,
            start_paused: false,
            wait_for_command: false,
            server: false,
            connect: None,
            join: None,
            mp_start_at_epoch_ms: None,
            mp_expected_players: None,
            mp_mission_profile_id: None,
            mp_browser_join_links: None,
            mp_nickname: String::new(),
        };
        install_global_options(&args);
        args
    }
}

#[cfg(all(target_arch = "wasm32", feature = "multiplayer"))]
thread_local! {
    static PENDING_BROWSER_JOIN: std::cell::RefCell<Option<(String, bool)>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(all(target_arch = "wasm32", feature = "multiplayer"))]
pub fn set_pending_browser_join(code: String, redeemed: bool) -> Result<(), String> {
    // Authenticate immediately, before the shell can use ticket-selected
    // mission or relay fields. Time is rechecked when the run starts.
    // The JS shell receives the rejection as text.
    let ticket = crate::multiplayer::join_ticket::BrowserJoinTicket::decode_authenticated(&code)
        .map_err(|error| error.to_string())?;
    ticket
        .validate_use_at(
            current_epoch_seconds().map_err(|error| error.to_string())?,
            if redeemed {
                crate::multiplayer::join_ticket::InvitationUse::RedeemedReconnect
            } else {
                crate::multiplayer::join_ticket::InvitationUse::Initial
            },
        )
        .map_err(|error| error.to_string())?;
    PENDING_BROWSER_JOIN.with(|pending| {
        let mut pending = pending.borrow_mut();
        if pending.is_some() {
            return Err("browser multiplayer invitation was already installed".to_string());
        }
        *pending = Some((code, redeemed));
        Ok(())
    })
}

#[cfg(all(target_arch = "wasm32", not(feature = "multiplayer")))]
pub fn set_pending_browser_join(_code: String, _redeemed: bool) -> Result<(), String> {
    Err(
        "browser multiplayer is unavailable in this build; rebuild with `--features multiplayer`"
            .to_string(),
    )
}

/// Resolve the launcher's join route while building the run's configuration:
/// `cli` and `browser_join_redeemed` are the run config under construction.
#[cfg(feature = "multiplayer")]
pub(super) fn resolve_join_ticket(
    cli: &mut CliArgs,
    browser_join_redeemed: &mut bool,
) -> Result<(), super::LaunchError> {
    #[cfg(target_arch = "wasm32")]
    if cli.join.is_none() {
        if let Some((code, redeemed)) =
            PENDING_BROWSER_JOIN.with(|pending| pending.borrow_mut().take())
        {
            cli.join = Some(code);
            *browser_join_redeemed = redeemed;
        }
    }
    let Some(code) = cli.join.as_deref() else {
        return Ok(());
    };
    if cli.server || cli.connect.is_some() || cli.mission.is_some() {
        return Err(super::LaunchError::arguments(
            "--join cannot be combined with --server, --connect, or --mission",
        ));
    }
    let ticket = crate::multiplayer::join_ticket::BrowserJoinTicket::decode_authenticated(code)?;
    ticket.validate_use_at(
        current_epoch_seconds()?,
        if *browser_join_redeemed {
            crate::multiplayer::join_ticket::InvitationUse::RedeemedReconnect
        } else {
            crate::multiplayer::join_ticket::InvitationUse::Initial
        },
    )?;
    apply_authenticated_join_route(cli, &ticket, cfg!(target_arch = "wasm32"))
}

#[cfg(feature = "multiplayer")]
fn apply_authenticated_join_route(
    cli: &mut CliArgs,
    ticket: &crate::multiplayer::join_ticket::BrowserJoinTicket,
    browser_interactive_preflight: bool,
) -> Result<(), super::LaunchError> {
    if browser_interactive_preflight {
        // A browser direct invite must enter the graphical multiplayer
        // preflight before mission construction. That probe obtains the
        // authenticated ContentOffer without claiming a gameplay seat, shows
        // the mandatory consent UI, downloads/verifies the exact package, and
        // carries those bytes into the real connection. Setting `mission` or
        // `connect` here would instead take the direct-mission fast path and
        // reject every non-vanilla host as un-preflighted content.
        cli.force_main_menu = true;
        return Ok(());
    }

    let endpoint_addr = ticket.endpoint_addr()?;
    let connect = serde_json::to_string(&endpoint_addr)
        .expect("validated iroh EndpointAddr serialization cannot fail");
    cli.connect = Some(connect);
    cli.mission = Some(ticket.payload().mission_id.clone());
    cli.proto = None;
    cli.mp_expected_players = Some(ticket.payload().expected_players);
    cli.mp_mission_profile_id = ticket.payload().mission_profile_id;
    Ok(())
}

#[cfg(not(feature = "multiplayer"))]
pub(super) fn resolve_join_ticket(
    args: &mut CliArgs,
    _browser_join_redeemed: &mut bool,
) -> Result<(), super::LaunchError> {
    if args.join.is_some() || args.server || args.connect.is_some() {
        return Err(super::LaunchError::arguments(
            "multiplayer was requested but is unavailable in this build; rebuild with `--features multiplayer`",
        ));
    }
    Ok(())
}

#[cfg(feature = "multiplayer")]
fn current_epoch_seconds() -> Result<u64, super::LaunchError> {
    web_time::SystemTime::now()
        .duration_since(web_time::SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| {
            super::LaunchError::clock(format!("system clock precedes the Unix epoch: {error}"))
        })
}

pub(super) fn options_from_args(args: &CliArgs) -> engine_api::GlobalOptions {
    engine_api::GlobalOptions {
        sound_enabled: !args.no_sound,
        script_enabled: !args.no_script,
        highlander2: args.highlander2,
        whatsup: args.whatsup,
        debug_surfaces: args.debug_surfaces,
        golden_eye: args.goldeneye,
        ignore_default_loose: args.no_default_loose,
        check_sound_data: args.check_sound_data,
        ..Default::default()
    }
}

fn install_global_options(args: &CliArgs) {
    // Install the process-wide `GlobalOptions` so UI layers that don't
    // have a `Game` or `CliArgs` in scope can still read startup flags.
    engine_api::GlobalOptions::set_global(options_from_args(args));
}

pub fn try_parse_cli_from<I, T>(itr: I) -> Result<CliArgs, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = CliArgs::try_parse_from(itr)?;
    install_global_options(&args);
    Ok(args)
}

pub fn parse_cli_from<I, T>(itr: I) -> CliArgs
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    try_parse_cli_from(itr).unwrap_or_else(|e| e.exit())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn parse_cli() -> CliArgs {
    parse_cli_from(std::env::args_os())
}

#[cfg(target_arch = "wasm32")]
pub fn parse_cli() -> CliArgs {
    wasm_cli_args_from_location()
}

#[cfg(target_arch = "wasm32")]
fn wasm_cli_args_from_location() -> CliArgs {
    let query = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .unwrap_or_default();
    let query = query.strip_prefix('?').unwrap_or(&query);
    let query = normalize_wasm_query(query);
    let mut args = match serde_urlencoded::from_str::<CliArgs>(&query) {
        Ok(args) => args,
        Err(e) => {
            tracing::warn!("invalid wasm URL options: {e}; using defaults");
            CliArgs::default()
        }
    };
    if let Some(replay) = args.replay.take() {
        // Binary share links are loaded by the shell over RPC. A bare build
        // hash only selects the runtime and must not suppress normal startup.
        if replay.starts_with("rhrec1-") {
            args.wait_for_command = true;
        }
    }
    install_global_options(&args);
    args
}

#[cfg(target_arch = "wasm32")]
fn normalize_wasm_query(query: &str) -> String {
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let Some((key, value)) = part.split_once('=') else {
                return format!("{}=true", part.replace('_', "-"));
            };
            let key = key.replace('_', "-");
            let value = match value {
                "" | "1" | "yes" | "on" => "true",
                "0" | "no" | "off" => "false",
                _ => value,
            };
            format!("{key}={value}")
        })
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;
    use robin_engine::profiles::ProfileManager;

    #[cfg(feature = "multiplayer")]
    use super::apply_authenticated_join_route;
    use super::try_parse_cli_from;
    use crate::main_entry::callbacks::{
        PreparedLoad, current_mission_id, recommended_export_team, required_mission_id,
        validate_save_mission, validated_save_reload_target,
    };
    use crate::main_entry::launch::requested_replay_data;
    use crate::main_entry::{
        LaunchConfig, MissionContent, MissionRequest, MissionStartCapture, PendingLuaMission,
    };
    use robin_engine::campaign::Campaign;
    use std::sync::Arc;

    const RUNTIME_HANDOFF_FIELDS: &[&str] = &[
        "replay_data",
        "mission_restart",
        "resolved_mission_assets",
        "simulation_content_export",
        "pending_lua_mission",
        "pending_distributed_mod",
        "mission_start_map_output",
        "mission_start_map_frame",
        "mission_start_reveal_all",
        "mission_start_fog_of_war",
        "mission_start_viewport_capture",
        "mission_start_legacy_save",
        "preserve_forced_mission_campaign",
        "mp_continue_session",
        "browser_join_redeemed",
        "global_options",
    ];

    fn assert_unprepared_launch(launch: &MissionRequest) {
        assert!(!launch.mission_restart);
        assert!(launch.replay_data.is_none());
        assert!(launch.content.resolved_mission_assets.is_none());
        assert!(launch.content.pending_lua_mission.is_none());
        assert!(launch.content.pending_distributed_mod.is_none());
        let capture = &launch.config.capture;
        assert!(capture.map_output.is_none());
        assert_eq!(capture.map_frame, 0);
        assert!(!capture.reveal_all);
        assert!(!capture.fog_of_war);
        assert!(!capture.viewport_capture);
        assert!(capture.legacy_save.is_none());
        assert!(!capture.preserve_forced_mission_campaign);
        assert!(!launch.multiplayer.continue_session);
        assert!(!launch.config.browser_join_redeemed);
        assert!(launch.config.global_options.preparation_files().is_err());
    }

    #[test]
    fn raw_cli_type_cannot_own_process_or_mission_handoffs() {
        // Guard the Rust API as well as clap/serde: skipped fields would still
        // let callers put runtime authority back in the raw configuration.
        let source = syn::parse_file(include_str!("cli.rs")).unwrap();
        let fields = source
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Struct(item) if item.ident == "CliArgs" => Some(&item.fields),
                _ => None,
            })
            .unwrap();
        for field in fields {
            let name = field.ident.as_ref().unwrap().to_string();
            assert!(
                !RUNTIME_HANDOFF_FIELDS.contains(&name.as_str()),
                "{name} belongs to LaunchConfig or MissionRequest"
            );
        }
    }

    #[test]
    fn decoded_launch_configuration_retains_flags_without_runtime_authority() {
        let launch = LaunchConfig::from(super::CliArgs {
            no_sound: true,
            no_script: true,
            goldeneye: true,
            highlander2: true,
            whatsup: true,
            no_default_loose: true,
            check_sound_data: true,
            debug_surfaces: true,
            ..Default::default()
        });
        let wire = serde_json::to_value(&launch).unwrap();
        let decoded: LaunchConfig = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(decoded.global_options.options()).unwrap(),
            serde_json::to_value(launch.global_options.options()).unwrap()
        );
        assert!(crate::host::ReadyApplicationContext::try_from(decoded.global_options).is_err());
        let request: MissionRequest = serde_json::from_value(wire).unwrap();
        assert_unprepared_launch(&request);
    }

    #[test]
    fn parsed_configuration_cannot_install_runtime_launch_authority() {
        let mut input = serde_json::json!({"headless": true, "no-sound": true});
        for field in RUNTIME_HANDOFF_FIELDS {
            input[field.replace('_', "-")] = serde_json::json!({"forged": true});
        }
        let raw: super::CliArgs = serde_json::from_value(input).unwrap();
        let encoded_raw = serde_json::to_value(&raw).unwrap();
        for field in RUNTIME_HANDOFF_FIELDS {
            assert!(encoded_raw.get(field.replace('_', "-")).is_none());
        }
        let launch = MissionRequest::from(LaunchConfig::from(raw));
        assert!(launch.config.cli.headless);
        assert!(!launch.config.global_options.sound_enabled);
        assert_unprepared_launch(&launch);

        let mut input = serde_json::json!({"config": {"headless": true}});
        for field in RUNTIME_HANDOFF_FIELDS {
            input[*field] = serde_json::json!({"forged": true});
        }
        let injected: MissionRequest = serde_json::from_value(input).unwrap();
        assert!(injected.config.cli.headless);
        assert_unprepared_launch(&injected);
    }

    #[test]
    fn runtime_handoffs_share_configuration_but_do_not_round_trip_through_it() {
        let config = Arc::new(LaunchConfig {
            browser_join_redeemed: true,
            capture: MissionStartCapture {
                map_output: Some("capture.png".into()),
                map_frame: 42,
                reveal_all: true,
                fog_of_war: true,
                viewport_capture: true,
                legacy_save: Some(vec![4, 5, 6]),
                preserve_forced_mission_campaign: true,
            },
            ..Default::default()
        });
        let launch = MissionRequest::new(Arc::clone(&config))
            .with_content(MissionContent {
                pending_lua_mission: Some(PendingLuaMission {
                    rhm_basename: "custom".into(),
                    requires_spellforge: false,
                    spellforge_package: None,
                }),
                pending_distributed_mod: Some(Arc::from([1u8, 2, 3])),
                ..Default::default()
            })
            .with_session_continuation(true);
        // Every launch shares, never copies, the process configuration.
        let next = MissionRequest::new(Arc::clone(&config));
        assert!(Arc::ptr_eq(&launch.config, &next.config));
        assert_eq!(next.config.capture.map_frame, 42);
        assert_eq!(next.config.capture.legacy_save, Some(vec![4, 5, 6]));
        assert!(next.content.pending_distributed_mod.is_none());
        assert!(!next.multiplayer.continue_session);
        let encoded = serde_json::to_value(&launch).unwrap();
        for field in RUNTIME_HANDOFF_FIELDS {
            assert!(encoded.get(*field).is_none());
        }
        let decoded: MissionRequest = serde_json::from_value(encoded).unwrap();
        assert_unprepared_launch(&decoded);
    }

    /// Decode a launch configuration exactly as the launch `Deserialize` impl
    /// does and reduce it to comparable values: the configuration, the derived
    /// startup options, and whether any runtime authority was installed.
    /// `LaunchConfig` and `MissionRequest` must accept and reject the same
    /// inputs identically.
    fn decode_launch_fixture(
        input: &str,
    ) -> Result<(serde_json::Value, serde_json::Value, bool), String> {
        let config = serde_json::from_str::<LaunchConfig>(input).map_err(|error| error.to_string());
        let request =
            serde_json::from_str::<MissionRequest>(input).map_err(|error| error.to_string());
        match (config, request) {
            (Err(config), Err(request)) => {
                assert_eq!(config, request, "input: {input}");
                Err(config)
            }
            (Ok(config), Ok(request)) => {
                assert_unprepared_launch(&request);
                assert_eq!(
                    serde_json::to_value(&request.config.cli).unwrap(),
                    serde_json::to_value(&config.cli).unwrap()
                );
                assert_eq!(request.replay, config.cli.replay);
                assert_eq!(request.start_paused, config.cli.start_paused);
                assert_eq!(request.multiplayer.server, config.cli.server);
                Ok((
                    serde_json::to_value(&config.cli).unwrap(),
                    serde_json::to_value(config.global_options.options()).unwrap(),
                    config.global_options.preparation_files().is_ok(),
                ))
            }
            (config, request) => panic!(
                "LaunchConfig and MissionRequest disagree on {input}: {:?} vs {:?}",
                config.map(|_| ()),
                request.map(|_| ())
            ),
        }
    }

    /// Fixture table for the launch configuration `Deserialize` behaviour.
    /// Written against the hand-written impl; the derived impl must match it
    /// input for input, including ignored unknown fields and error text.
    #[test]
    fn launch_configuration_deserialize_fixtures_are_stable() {
        let expect_ok = |config: super::CliArgs| {
            Ok((
                serde_json::to_value(&config).unwrap(),
                serde_json::to_value(super::options_from_args(&config)).unwrap(),
                false,
            ))
        };
        let flagged = super::CliArgs {
            no_sound: true,
            headless: true,
            mission: Some("Dem_Lei_MP".into()),
            http_server: 9,
            rollback_check: false,
            mp_expected_players: Some(3),
            ..Default::default()
        };
        let cases: Vec<(&str, Result<(serde_json::Value, serde_json::Value, bool), String>)> = vec![
            // Missing configuration defaults.
            ("{}", expect_ok(super::CliArgs::default())),
            (r#"{"config":{}}"#, expect_ok(super::CliArgs::default())),
            (
                r#"{"config":{"no-sound":true,"headless":true,"mission":"Dem_Lei_MP","http-server":9,"rollback-check":false,"mp-expected-players":3}}"#,
                expect_ok(flagged.clone()),
            ),
            // Sequence form of the outer struct.
            (
                r#"[{"no-sound":true,"headless":true,"mission":"Dem_Lei_MP","http-server":9,"rollback-check":false,"mp-expected-players":3}]"#,
                expect_ok(flagged.clone()),
            ),
            // Unknown outer fields (including runtime handoffs) are ignored.
            (
                r#"{"config":{"headless":true},"replay_data":{"forged":true},"mission-restart":true,"bogus":1}"#,
                expect_ok(super::CliArgs {
                    headless: true,
                    ..Default::default()
                }),
            ),
            // Unknown and snake_case configuration keys are ignored.
            (
                r#"{"config":{"no_sound":true,"not-a-flag":7}}"#,
                expect_ok(super::CliArgs::default()),
            ),
            // Errors.
            (
                r#"{"config":null}"#,
                Err("invalid type: null, expected struct CliArgs at line 1 column 14".into()),
            ),
            // An empty sequence defaults its only (defaulted) field.
            ("[]", expect_ok(super::CliArgs::default())),
            (
                "[{},{}]",
                Err("trailing characters at line 1 column 5".into()),
            ),
            (
                r#""launch""#,
                Err(
                    "invalid type: string \"launch\", expected struct Configuration at line 1 column 8"
                        .into(),
                ),
            ),
            (
                r#"{"config":{"http-server":"x"}}"#,
                Err("invalid type: string \"x\", expected u16 at line 1 column 28".into()),
            ),
            (
                r#"{"config":{},"config":{}}"#,
                Err("duplicate field `config` at line 1 column 21".into()),
            ),
        ];
        let mismatches: Vec<_> = cases
            .into_iter()
            .filter_map(|(input, expected)| {
                let actual = decode_launch_fixture(input);
                (actual != expected).then(|| format!("{input}: {actual:?}"))
            })
            .collect();
        assert!(mismatches.is_empty(), "{mismatches:#?}");
    }

    /// The launch configuration's serialized form is `{"config": CliArgs}`.
    #[test]
    fn launch_configuration_serialize_fixture_is_stable() {
        let config = super::CliArgs {
            goldeneye: true,
            replay: Some("recording.rhrec.jsonl".into()),
            mp_nickname: "nick".into(),
            ..Default::default()
        };
        let launch = LaunchConfig::from(config.clone());
        assert_eq!(
            serde_json::to_value(&launch).unwrap(),
            serde_json::json!({ "config": serde_json::to_value(&config).unwrap() })
        );
        let text = serde_json::to_string(&launch).unwrap();
        assert_eq!(
            decode_launch_fixture(&text).unwrap().0,
            serde_json::to_value(&config).unwrap()
        );
    }

    #[test]
    #[cfg(feature = "multiplayer")]
    fn browser_join_route_requires_interactive_preflight_before_mission_bootstrap() {
        let key = iroh::SecretKey::from_bytes(&[7; 32]);
        let address = iroh::EndpointAddr::from(key.public())
            .with_relay_url("https://relay.example.invalid/".parse().unwrap());
        let ticket = crate::multiplayer::join_ticket::BrowserJoinTicket::issue(
            &key,
            &address,
            [9; 32],
            2_000_000_000,
            crate::multiplayer::join_ticket::BrowserJoinTicketContent {
                content_edition: crate::multiplayer::join_ticket::BrowserContentEdition::Full,
                content_identity_sha256: "01".repeat(32),
                mission_id: "Custom_MP".to_owned(),
                mission_profile_id: None,
                expected_players: 2,
            },
        )
        .unwrap();
        let code = ticket.encode();
        let mut args = super::CliArgs {
            join: Some(code.clone()),
            ..Default::default()
        };

        apply_authenticated_join_route(&mut args, &ticket, true).unwrap();

        assert_eq!(args.join.as_deref(), Some(code.as_str()));
        assert!(args.force_main_menu);
        assert!(args.connect.is_none());
        assert!(args.mission.is_none());
        let launch = MissionRequest::from(LaunchConfig::from(args));
        assert_eq!(launch.multiplayer.join.as_deref(), Some(code.as_str()));
        assert!(launch.content.pending_distributed_mod.is_none());
        assert!(launch.content.pending_lua_mission.is_none());
    }

    #[test]
    fn clap_launcher_flags_populate_global_options() {
        let args = try_parse_cli_from([
            "robin",
            "--no-sound",
            "--no-script",
            "--highlander2",
            "--whatsup",
            "--goldeneye",
            "--no-default-loose",
            "--check-sound-data",
        ])
        .unwrap();

        let args = LaunchConfig::from(args);
        assert!(!args.global_options.sound_enabled);
        assert!(!args.global_options.script_enabled);
        assert!(args.global_options.highlander2);
        assert!(args.global_options.whatsup);
        assert!(args.cli.goldeneye);
        assert!(args.global_options.golden_eye);
        assert!(args.global_options.ignore_default_loose);
        assert!(args.global_options.check_sound_data);
    }

    #[test]
    fn legacy_launcher_flags_are_rejected_by_clap() {
        let err = try_parse_cli_from(["robin", "-NOSOUND"]).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn mission_flag_defaults_proto_to_mission_name() {
        let args = try_parse_cli_from(["robin", "--mission", "Dem_Lei_MP"]).unwrap();

        assert_eq!(args.mission.as_deref(), Some("Dem_Lei_MP"));
        assert_eq!(args.proto.as_deref().unwrap_or("Dem_Lei_MP"), "Dem_Lei_MP");
    }

    #[test]
    fn custom_mission_zip_requires_and_preserves_mission() {
        let args = try_parse_cli_from([
            "robin",
            "--mission",
            "Str03_Yor_MK",
            "--proto",
            "Str03_Yor",
            "--custom-mission",
            "mods/york.zip",
            "--custom-mission-entry",
            "German/Data/Levels/Str03_Yor_MK.rhm",
        ])
        .unwrap();

        assert_eq!(args.mission.as_deref(), Some("Str03_Yor_MK"));
        assert_eq!(args.proto.as_deref(), Some("Str03_Yor"));
        assert_eq!(
            args.custom_mission.as_deref(),
            Some(std::path::Path::new("mods/york.zip"))
        );
        assert_eq!(
            args.custom_mission_entry.as_deref(),
            Some("German/Data/Levels/Str03_Yor_MK.rhm")
        );

        let err = try_parse_cli_from(["robin", "--custom-mission", "mods/york.zip"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn mission_map_uses_walkthrough_team_for_campaign_missions() {
        let profiles = ProfileManager::new();
        assert_eq!(
            recommended_export_team(&profiles, "H01_Lin_VL").unwrap(),
            "R"
        );
        assert_eq!(
            recommended_export_team(&profiles, "S02_Lei_MP").unwrap(),
            "RSBC"
        );
        assert_eq!(
            recommended_export_team(&profiles, "H09_Not_VL").unwrap(),
            "MJTB"
        );
        assert_eq!(
            recommended_export_team(&profiles, "SherwoodOutro").unwrap(),
            "RJTSWM"
        );
    }

    #[test]
    fn optional_mission_team_only_uses_recruited_heroes() {
        use robin_engine::profiles::MissionProfile;

        let mut profiles = ProfileManager::new();
        profiles.missions = vec![
            MissionProfile {
                id: 1,
                mission_filename: "S01_Not_VL".into(),
                ..Default::default()
            },
            MissionProfile {
                id: 2,
                mission_filename: "S02_Lei_MP".into(),
                missions_required_to_be_done: vec![1],
                ..Default::default()
            },
            MissionProfile {
                id: 3,
                mission_filename: "Emb_Test".into(),
                missions_required_to_be_done: vec![2],
                ..Default::default()
            },
        ];

        assert_eq!(
            recommended_export_team(&profiles, "Emb_Test").unwrap(),
            "RWSBC"
        );
    }

    #[test]
    fn optional_mission_team_rejects_missing_profiles() {
        use robin_engine::profiles::MissionProfile;

        let mut profiles = ProfileManager::new();
        assert!(recommended_export_team(&profiles, "Emb_Missing").is_err());

        profiles.missions.push(MissionProfile {
            mission_filename: "Emb_Test".into(),
            missions_required_to_be_done: vec![99],
            ..Default::default()
        });
        let error = recommended_export_team(&profiles, "Emb_Test")
            .unwrap_err()
            .to_string();
        assert!(error.contains("prerequisite mission profile id 99"));
    }

    fn replay_launch_fixture() -> robin_engine::replay::ReplayData {
        use robin_engine::replay::{ReplayFile, ReplayHeader};
        use std::collections::BTreeMap;

        ReplayFile {
            header: ReplayHeader {
                mission_id: "MissionA".into(),
                mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                    "MissionA", "MissionA", "MissionA",
                )
                .unwrap(),
                rng_seed: 0x55aa,
                sim_config: robin_engine::engine::SimConfig::default(),
                spellforge_package: None,
                version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
                total_frames: 0,
                rankability: robin_engine::replay_rankability::ReplayRankability::rankable(),
                campaign: bitcode::encode(&Campaign::default()),
            },
            frames: BTreeMap::new(),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        }
        .try_into()
        .expect("valid replay fixture")
    }

    #[test]
    fn replay_request_replaces_assets_without_changing_capture_or_pause_policy() {
        let config = Arc::new(LaunchConfig {
            cli: super::CliArgs {
                replay: Some("old recording".into()),
                custom_mission: Some("old archive".into()),
                start_paused: true,
                ..Default::default()
            },
            capture: MissionStartCapture {
                legacy_save: Some(vec![1, 2, 3]),
                map_frame: 42,
                ..Default::default()
            },
            ..Default::default()
        });
        let previous = MissionRequest {
            mission_restart: true,
            replay_data: Some(replay_launch_fixture()),
            ..MissionRequest::new(config)
        }
        .with_content(MissionContent {
            custom_mission: Some("old archive".into()),
            pending_distributed_mod: Some(Arc::from([4u8, 5, 6])),
            pending_lua_mission: Some(PendingLuaMission {
                rhm_basename: "old".into(),
                requires_spellforge: false,
                spellforge_package: None,
            }),
            ..Default::default()
        })
        .with_session_continuation(true);
        assert!(previous.replay.is_some() && previous.start_paused);
        let next = previous.for_replay(replay_launch_fixture(), false);
        assert!(!next.mission_restart);
        assert!(next.replay.is_none());
        assert!(next.content.custom_mission.is_none());
        assert!(next.content.pending_distributed_mod.is_none());
        assert!(next.content.pending_lua_mission.is_none());
        assert!(next.content.resolved_mission_assets.is_none());
        assert_eq!(next.replay_data.as_ref().unwrap().header().rng_seed, 0x55aa);
        assert!(next.start_paused);
        assert!(next.multiplayer.continue_session);
        assert_eq!(next.config.capture.map_frame, 42);
        assert_eq!(next.config.capture.legacy_save, Some(vec![1, 2, 3]));
        assert!(previous.content.pending_distributed_mod.is_some());
        let previous = previous.with_start_paused(false);
        assert!(
            previous
                .for_replay(replay_launch_fixture(), true)
                .start_paused
        );
        assert!(
            !previous
                .for_replay(replay_launch_fixture(), false)
                .start_paused
        );
    }

    #[test]
    fn decoded_replay_payload_wins_over_the_original_spec() {
        let data = replay_launch_fixture();
        let args = MissionRequest {
            replay_data: Some(data),
            ..MissionRequest::from(LaunchConfig {
                cli: super::CliArgs {
                    replay: Some("this-path-must-never-be-read".into()),
                    ..Default::default()
                },
                ..Default::default()
            })
        };

        let selected = requested_replay_data(&args).unwrap().unwrap();
        assert_eq!(selected.header().rng_seed, 0x55aa);
        let serialized = serde_json::to_value(&args).unwrap();
        assert!(serialized.get("replay_data").is_none());
        assert!(serialized.get("mission_restart").is_none());
        assert!(serialized.get("resolved_mission_assets").is_none());
        assert!(serialized.get("simulation_content_export").is_none());
        let restored: MissionRequest = serde_json::from_value(serialized).unwrap();
        assert_eq!(restored.replay, args.replay);
        assert!(restored.replay_data.is_none());
    }

    #[test]
    fn proto_requires_mission() {
        let err = try_parse_cli_from(["robin", "--proto", "Leicester"]).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    #[should_panic(expected = "required test mission: mission ID zero is invalid")]
    fn required_mission_id_rejects_zero() {
        required_mission_id(Some(0), "required test mission");
    }

    #[test]
    #[should_panic(expected = "current_mission_id: campaign must have a valid current mission")]
    fn current_mission_id_rejects_missing_current_mission() {
        current_mission_id(&Campaign::default(), &ProfileManager::new());
    }

    #[test]
    fn save_preflight_rejects_header_campaign_mission_mismatch() {
        use robin_engine::engine::{Engine, LevelAssets};
        use robin_engine::mission::Mission;
        use robin_engine::profiles::MissionProfile;

        let mut profiles = ProfileManager::new();
        profiles.missions = vec![
            MissionProfile {
                id: 10,
                ..Default::default()
            },
            MissionProfile {
                id: 20,
                ..Default::default()
            },
        ];
        let campaign = Campaign {
            missions: vec![
                Mission {
                    profile_idx: Some(0),
                    ..Default::default()
                },
                Mission {
                    profile_idx: Some(1),
                    ..Default::default()
                },
            ],
            current_mission_idx: Some(0),
            ..Default::default()
        };
        let mut assets = LevelAssets::new();
        assets.profile_manager = std::sync::Arc::new(profiles.clone());
        let engine = Engine::new_for_test(800.0, 600.0, campaign, &mut assets).unwrap();
        let host = crate::host::Host::scratch(800.0, 600.0);
        let save = crate::save_file::GameSaveFile::capture(&engine, &host, 20, "mismatch".into());

        let error = validate_save_mission(&save, &profiles)
            .unwrap_err()
            .to_string();
        assert!(error.contains("current mission Some(0)"));
        assert!(error.contains("mission id 20 at index 1"));
    }

    #[test]
    fn strict_save_route_rejects_zero_and_routes_valid_cross_mission_payload() {
        use robin_engine::engine::{Engine, LevelAssets};
        use robin_engine::mission::Mission;
        use robin_engine::profiles::MissionProfile;

        let mut profiles = ProfileManager::new();
        profiles.missions = vec![
            MissionProfile {
                id: 10,
                ..Default::default()
            },
            MissionProfile {
                id: 20,
                ..Default::default()
            },
        ];
        let mut campaign = Campaign {
            missions: vec![
                Mission {
                    profile_idx: Some(0),
                    ..Default::default()
                },
                Mission {
                    profile_idx: Some(1),
                    ..Default::default()
                },
            ],
            current_mission_idx: Some(1),
            ..Default::default()
        };
        campaign
            .snapshot_preselected_with_simulation(7, robin_engine::engine::SimConfig::default());
        let mut assets = LevelAssets::new();
        assets.profile_manager = std::sync::Arc::new(profiles.clone());
        let engine = Engine::new_for_test(800.0, 600.0, campaign, &mut assets).unwrap();
        let host = crate::host::Host::scratch(800.0, 600.0);
        let mut save = crate::save_file::GameSaveFile::capture(&engine, &host, 20, "route".into());
        let mission_10_assets = robin_engine::mission_assets::MissionAssetDescriptor::built_in(
            "TestMission10",
            "TestMap10",
            "TestMap10",
        )
        .unwrap();
        let mission_20_assets = save.header.mission_assets.clone();

        assert_eq!(
            validated_save_reload_target(&save, &profiles, 10, &mission_10_assets, None).unwrap(),
            Some(20)
        );
        assert_eq!(
            validated_save_reload_target(&save, &profiles, 20, &mission_20_assets, None).unwrap(),
            None
        );
        let same_id_different_assets =
            robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                "OtherMission",
                "OtherMap",
                "OtherMap",
            )
            .unwrap();
        assert_eq!(
            validated_save_reload_target(&save, &profiles, 20, &same_id_different_assets, None,)
                .unwrap(),
            Some(20),
            "numeric mission equality must not authorize different immutable assets"
        );
        save.header.mission_id = 0;
        assert_eq!(
            validated_save_reload_target(&save, &profiles, 10, &mission_10_assets, None)
                .unwrap_err()
                .to_string(),
            "invalid current save schema: invalid save mission ID: zero is not a valid mission"
        );
    }

    #[test]
    fn save_preflight_rejects_malformed_campaign_profile_index() {
        use robin_engine::engine::{Engine, LevelAssets};
        use robin_engine::mission::Mission;
        use robin_engine::profiles::MissionProfile;

        let mut profiles = ProfileManager::new();
        profiles.missions.push(MissionProfile {
            id: 10,
            ..Default::default()
        });
        let mut assets = LevelAssets::new();
        let mut engine =
            Engine::new_for_test(800.0, 600.0, Campaign::default(), &mut assets).unwrap();
        let mut malformed = Campaign::default();
        malformed.missions.push(Mission {
            profile_idx: Some(999),
            ..Default::default()
        });
        malformed.current_mission_idx = Some(0);
        malformed
            .snapshot_preselected_with_simulation(7, robin_engine::engine::SimConfig::default());
        let host = crate::host::Host::scratch(800.0, 600.0);
        engine
            .advance_frame(
                &assets,
                robin_engine::engine::SimulationFrameInput::no_hourglass().with_external_actions(
                    vec![robin_engine::engine::ExternalAction::ReplaceCampaign {
                        campaign: malformed,
                    }],
                ),
            )
            .expect("malformed campaign fixture admission");
        let save = crate::save_file::GameSaveFile::capture(&engine, &host, 10, "malformed".into());

        let error = validate_save_mission(&save, &profiles)
            .unwrap_err()
            .to_string();
        assert!(error.contains("out-of-range profile_idx 999"));
    }

    #[test]
    fn decoded_save_payload_and_slot_survive_file_replacement_after_preflight() {
        use robin_engine::engine::{Engine, LevelAssets};

        let directory = tempfile::tempdir().unwrap();
        let mut manager =
            crate::savegame::SaveGameManager::new(directory.path().to_string_lossy().into_owned());
        let slot = manager.create("slot".into(), 1);
        let mut assets = LevelAssets::new();
        let mut profiles = ProfileManager::default();
        profiles
            .missions
            .push(robin_engine::profiles::MissionProfile {
                id: 1,
                mission_filename: "Mission_1".into(),
                proto_level_filename: "Map_1".into(),
                mission_name: "Mission 1".into(),
                ..Default::default()
            });
        let mut campaign = Campaign::default();
        campaign.missions.push(robin_engine::mission::Mission {
            profile_idx: Some(0),
            ..Default::default()
        });
        let mut original = Engine::new_for_test(800.0, 600.0, campaign, &mut assets).unwrap();
        original.test_set_frame_counter(111);
        let path = directory.path().to_str().unwrap().to_owned();
        let mut players = robin_engine::player_profile::PlayerProfileManager::new(path.clone());
        let player = players.create_profile(
            "Preflight player".into(),
            robin_engine::player_profile::DifficultyLevel::Medium,
        );
        players.set_active(player);
        let context = crate::host::ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&path),
            robin_engine::engine::GlobalOptions::default(),
            players,
            crate::key_config_store::KeyConfigStore::new(path),
            None,
        )
        .unwrap();
        let mut host = crate::host::Host::new(context.try_into().unwrap(), 800.0, 600.0).unwrap();
        let mut game = crate::game::Game::default();
        game.set_mission_assets(
            robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                "Mission_1",
                "Map_1",
                "Map_1",
            )
            .unwrap(),
        )
        .unwrap();
        // A draft plus an externally written file is intentionally not a
        // published slot. Use the real publication boundary before preflight.
        manager
            .write_save_from_engine(&mut host, &game, slot, &original, 1, Some(&profiles), None)
            .unwrap();
        let prepared = PreparedLoad::preflight(&manager, Some(manager.slot_handle(slot).unwrap()))
            .unwrap()
            .unwrap();

        let mut replacement = original.clone();
        replacement.test_set_frame_counter(222);
        crate::save_file::GameSaveFile::capture(&replacement, &host, 1, "replacement".into())
            .write_to(&manager.save_path(slot))
            .unwrap();

        prepared.validate_slot(&manager).unwrap();
        assert_eq!(prepared.save().engine.frame_counter(), 111);
    }
}
