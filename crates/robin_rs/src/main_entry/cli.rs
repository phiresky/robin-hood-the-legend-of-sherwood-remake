//! Command-line argument parsing (and the wasm URL-query equivalent).

use std::ffi::OsString;

use clap::Parser;
use serde::Deserialize;

use crate::host::ApplicationContext;
use crate::replay_format::COMPACT_PREFIX;
use robin_engine::engine as engine_api;
use robin_engine::replay as engine_replay;

/// Extension required for replay files — keeps the format searchable
/// and future-proofs us if we ever want to associate `.rhrec.jsonl` with
/// a handler at the OS level.
pub const RHREC_EXT: &str = ".rhrec.jsonl";

/// clap `value_parser` for `--record`: rejects anything that doesn't
/// end in `.rhrec.jsonl`. Recording always goes to the legacy JSONL
/// streaming format for crash-safety; the compact sharing format is
/// produced on demand from a finished recording.
fn parse_record_path(s: &str) -> Result<String, String> {
    if s.ends_with(RHREC_EXT) {
        Ok(s.to_string())
    } else {
        Err(format!("record path must end in `{RHREC_EXT}` (got `{s}`)"))
    }
}

/// clap `value_parser` for `--replay`: accepts either an inline
/// `rhrec-…` compact string (shared replay pasted on the command line)
/// or a filesystem path. Path validation is lenient because the loader
/// (`replay_format::load_replay_spec`) auto-detects JSONL vs. a file
/// holding a `rhrec-…` string, regardless of extension.
fn parse_replay_spec(s: &str) -> Result<String, String> {
    if s.trim_start().starts_with(COMPACT_PREFIX) {
        return Ok(s.to_string());
    }
    // A path — we don't require any particular extension, but a
    // friendlier error is easy to give when the user clearly fat-
    // fingered a `rhrec` variant.
    Ok(s.to_string())
}

pub(super) fn requested_replay_data(
    args: &CliArgs,
) -> Result<Option<engine_replay::ReplayData>, String> {
    if let Some(data) = args.replay_data.clone() {
        return Ok(Some(data));
    }
    args.replay
        .as_deref()
        .map(crate::replay_format::load_replay_spec)
        .transpose()
        .map_err(|error| format!("failed to load replay: {error}"))
}

/// Robin Hood — The Legend of Sherwood (Rust port)
#[derive(Parser, Debug, Clone, Deserialize)]
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

    /// Bypass fog sprite loading that can crash on some converted data.
    #[arg(long)]
    pub no_fog: bool,

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

    /// Record a replay to the given file path (must end in `.rhrec.jsonl`)
    #[arg(long, value_parser = parse_record_path)]
    pub record: Option<String>,

    /// Play back a replay. Accepts any of:
    ///   - an inline `rhrec-…` compact string (the sharing format),
    ///   - a file containing a `rhrec-…` string,
    ///   - a legacy `*.rhrec.jsonl` recording.
    ///
    /// The replay's header picks the mission to load.
    #[arg(long, value_parser = parse_replay_spec)]
    pub replay: Option<String>,

    /// Decoded replay payload supplied by the wasm shell over script RPC.
    ///
    /// Kept separate from `replay` so the engine can be seeded before
    /// construction without serializing an already-decoded replay back
    /// into a command-line string.
    #[arg(skip)]
    #[serde(skip)]
    pub replay_data: Option<engine_replay::ReplayData>,

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

    /// TCP port for the local script-RPC HTTP server.
    /// Default 17640 (loopback only). Set to 0 to disable.
    /// See `crate::http_server` for the wire format.
    #[arg(long, default_value_t = crate::http_server::DEFAULT_PORT)]
    pub http_server: u16,

    /// Run the frame loop with no 25 fps pacing sleep — ticks and
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

    /// Internal matchmaking handoff: keep the simulation paused until this
    /// wall-clock timestamp so host and joiners begin together.
    #[arg(long, hide = true)]
    pub mp_start_at_epoch_ms: Option<u64>,

    /// Internal matchmaking handoff: total player count the host should wait
    /// for at the multiplayer ready barrier.
    #[arg(long, hide = true)]
    pub mp_expected_players: Option<u32>,

    /// Nickname shown in the portrait "controlled by" overlay on
    /// peers.  Defaults to a host-name-derived fallback when omitted.
    #[arg(long, value_name = "NICKNAME", default_value = "")]
    pub mp_nickname: String,

    /// Runtime startup options consumed by engine/UI layers that have
    /// not been threaded through `CliArgs` directly.
    #[clap(skip)]
    #[serde(skip)]
    pub global_options: ApplicationContext,

    /// Internal handoff from the custom-mission picker. Spellforge-tagged
    /// launches carry the bits needed to construct a required `LuaSession`;
    /// Vanilla-tagged custom missions carry the same launch metadata but
    /// intentionally produce no Lua state. `None` for every non-mod launch.
    /// Not a real CLI flag; not serialised.
    #[clap(skip)]
    #[serde(skip)]
    pub pending_lua_mission: Option<PendingLuaMission>,

    /// Internal one-shot render request used by the `render_mission_map`
    /// example. The mission session captures the complete level through the
    /// regular screenshot machinery, writes it here, and exits. This is
    /// deliberately not a launcher flag:
    /// the Cargo example is the supported CLI for this specialized tool.
    #[clap(skip)]
    #[serde(skip)]
    pub mission_start_map_output: Option<std::path::PathBuf>,

    /// Absolute simulation frame for `mission_start_map_output`. Frame zero is
    /// the post-`Initialize`, pre-tick state.
    #[clap(skip)]
    #[serde(skip)]
    pub mission_start_map_frame: u32,

    /// Apply the original `UBIQUITY` / `UNBLIP` reveal-all-NPCs cheat to
    /// the one-shot mission-start map before it is rendered.
    #[clap(skip)]
    #[serde(skip)]
    pub mission_start_reveal_all: bool,

    /// Internal one-shot capture mode used by parity tooling. Unlike the map
    /// exporter, this captures the saved viewport and includes the ordinary
    /// gameplay HUD.
    #[clap(skip)]
    #[serde(skip)]
    pub mission_start_viewport_capture: bool,

    /// Exact Original v48 save bytes to adopt after constructing the mission
    /// topology and before the one-shot frame-zero capture.
    #[clap(skip)]
    #[serde(skip)]
    pub mission_start_legacy_save: Option<Vec<u8>>,

    /// Preserve the caller-supplied campaign when `--mission` selects the
    /// one-shot capture mission. Parity captures supply the exact recorded
    /// roster and campaign state instead of the map exporter's representative
    /// team.
    #[clap(skip)]
    #[serde(skip)]
    pub preserve_forced_mission_campaign: bool,
}

/// Subset of [`crate::main_menu::custom_missions::CustomMissionLaunch`]
/// needed to decide containment and, for Spellforge, construct a
/// [`crate::lua_session::LuaSession`] inside `run_mission`. Kept as a flat
/// clonable struct so it can ride along on `CliArgs` (which is `Clone`).
#[derive(Debug, Clone)]
pub struct PendingLuaMission {
    pub slug: String,
    pub rhm_basename: String,
    pub version_zip: std::path::PathBuf,
    pub mods_root: std::path::PathBuf,
    pub requires_spellforge: bool,
}

impl Default for CliArgs {
    fn default() -> Self {
        let mut args = Self {
            no_sound: false,
            no_script: false,
            goldeneye: false,
            highlander2: false,
            no_fog: false,
            whatsup: false,
            no_default_loose: false,
            check_sound_data: false,
            view_cones: false,
            debug_surfaces: false,
            record: None,
            replay: None,
            replay_data: None,
            rollback_check: true,
            sherwood: false,
            force_main_menu: false,
            mission: None,
            proto: None,
            custom_mission: None,
            http_server: crate::http_server::DEFAULT_PORT,
            fast_forward: false,
            headless: false,
            start_paused: false,
            wait_for_command: false,
            server: false,
            connect: None,
            mp_start_at_epoch_ms: None,
            mp_expected_players: None,
            mp_nickname: String::new(),
            global_options: ApplicationContext::default(),
            pending_lua_mission: None,
            mission_start_map_output: None,
            mission_start_map_frame: 0,
            mission_start_reveal_all: false,
            mission_start_viewport_capture: false,
            mission_start_legacy_save: None,
            preserve_forced_mission_campaign: false,
        };
        install_global_options(&mut args);
        args
    }
}

fn install_global_options(args: &mut CliArgs) {
    let opts = engine_api::GlobalOptions {
        sound_enabled: !args.no_sound,
        script_enabled: !args.no_script,
        highlander2: args.highlander2,
        bypass_fog_sprites_crash: args.no_fog,
        whatsup: args.whatsup,
        debug_surfaces: args.debug_surfaces,
        golden_eye: args.goldeneye,
        ignore_default_loose: args.no_default_loose,
        check_sound_data: args.check_sound_data,
        ..Default::default()
    };

    args.global_options = ApplicationContext::bootstrap(opts.clone());
    // Install the process-wide `GlobalOptions` so UI layers that don't
    // have a `Game` or `CliArgs` in scope can still read startup flags.
    engine_api::GlobalOptions::set_global(opts);
}

pub fn try_parse_cli_from<I, T>(itr: I) -> Result<CliArgs, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut args = CliArgs::try_parse_from(itr)?;
    install_global_options(&mut args);
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
    if args.replay.is_some() {
        // URL replays are loaded by the shell over RPC after Rust has
        // finished initialization, so the mission header can choose the
        // correct mission without racing demo auto-start.
        args.wait_for_command = true;
        args.replay = None;
    }
    install_global_options(&mut args);
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

    use super::{requested_replay_data, try_parse_cli_from};
    use crate::main_entry::callbacks::{
        current_mission_id, preflight_or_use_decoded_load, recommended_export_team,
        required_mission_id, validate_save_mission, validated_save_reload_target,
    };
    use robin_engine::campaign::Campaign;

    #[test]
    fn clap_launcher_flags_populate_global_options() {
        let args = try_parse_cli_from([
            "robin",
            "--no-sound",
            "--no-script",
            "--highlander2",
            "--no-fog",
            "--whatsup",
            "--goldeneye",
            "--no-default-loose",
            "--check-sound-data",
        ])
        .unwrap();

        assert!(!args.global_options.sound_enabled);
        assert!(!args.global_options.script_enabled);
        assert!(args.global_options.highlander2);
        assert!(args.global_options.bypass_fog_sprites_crash);
        assert!(args.global_options.whatsup);
        assert!(args.goldeneye);
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
        ])
        .unwrap();

        assert_eq!(args.mission.as_deref(), Some("Str03_Yor_MK"));
        assert_eq!(args.proto.as_deref(), Some("Str03_Yor"));
        assert_eq!(
            args.custom_mission.as_deref(),
            Some(std::path::Path::new("mods/york.zip"))
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
        let error = recommended_export_team(&profiles, "Emb_Test").unwrap_err();
        assert!(error.contains("prerequisite mission profile id 99"));
    }

    #[test]
    fn decoded_replay_payload_wins_over_the_original_spec() {
        use robin_engine::replay::{ReplayFile, ReplayHeader};
        use std::collections::BTreeMap;

        let data = ReplayFile {
            header: ReplayHeader {
                mission_id: "MissionA".into(),
                rng_seed: 0x55aa,
                sim_config: robin_engine::engine::SimConfig::default(),
                version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
                total_frames: 0,
                campaign: bitcode::serialize(&Campaign::default()).unwrap(),
            },
            frames: BTreeMap::new(),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        }
        .into();
        let args = super::CliArgs {
            replay: Some("this-path-must-never-be-read".into()),
            replay_data: Some(data),
            ..Default::default()
        };

        let selected = requested_replay_data(&args).unwrap().unwrap();
        assert_eq!(selected.header.rng_seed, 0x55aa);
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
        let mut campaign = Campaign::default();
        campaign.missions = vec![
            Mission {
                profile_idx: Some(0),
                ..Default::default()
            },
            Mission {
                profile_idx: Some(1),
                ..Default::default()
            },
        ];
        campaign.current_mission_idx = Some(0);
        let mut assets = LevelAssets::new();
        assets.profile_manager = std::sync::Arc::new(profiles.clone());
        let engine = Engine::new_for_test(800.0, 600.0, campaign, &mut assets).unwrap();
        let host = crate::host::Host::scratch(800.0, 600.0);
        let save = crate::save_file::GameSaveFile::capture(&engine, &host, 20, "mismatch".into());

        let error = validate_save_mission(&save, &profiles).unwrap_err();
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
        let mut campaign = Campaign::default();
        campaign.missions = vec![
            Mission {
                profile_idx: Some(0),
                ..Default::default()
            },
            Mission {
                profile_idx: Some(1),
                ..Default::default()
            },
        ];
        campaign.current_mission_idx = Some(1);
        campaign
            .snapshot_preselected_with_simulation(7, robin_engine::engine::SimConfig::default());
        let mut assets = LevelAssets::new();
        assets.profile_manager = std::sync::Arc::new(profiles.clone());
        let engine = Engine::new_for_test(800.0, 600.0, campaign, &mut assets).unwrap();
        let host = crate::host::Host::scratch(800.0, 600.0);
        let mut save = crate::save_file::GameSaveFile::capture(&engine, &host, 20, "route".into());

        assert_eq!(
            validated_save_reload_target(&save, &profiles, 10).unwrap(),
            Some(20)
        );
        assert_eq!(
            validated_save_reload_target(&save, &profiles, 20).unwrap(),
            None
        );
        save.header.mission_id = 0;
        assert_eq!(
            validated_save_reload_target(&save, &profiles, 10).unwrap_err(),
            "save header mission ID zero is invalid"
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

        let error = validate_save_mission(&save, &profiles).unwrap_err();
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
        let mut original =
            Engine::new_for_test(800.0, 600.0, Campaign::default(), &mut assets).unwrap();
        original.test_set_frame_counter(111);
        let host = crate::host::Host::scratch(800.0, 600.0);
        crate::save_file::GameSaveFile::capture(&original, &host, 1, "original".into())
            .write_to(&manager.save_path(slot))
            .unwrap();
        let (decoded_slot, decoded) = manager.preflight_load(Some(slot)).unwrap().unwrap();

        let mut replacement = original.clone();
        replacement.test_set_frame_counter(222);
        crate::save_file::GameSaveFile::capture(&replacement, &host, 1, "replacement".into())
            .write_to(&manager.save_path(slot))
            .unwrap();

        let (resolved_slot, resolved) =
            preflight_or_use_decoded_load(&manager, Some(decoded_slot), Some(decoded))
                .unwrap()
                .unwrap();
        assert_eq!(resolved_slot, slot);
        assert_eq!(resolved.engine.frame_counter(), 111);
    }
}
