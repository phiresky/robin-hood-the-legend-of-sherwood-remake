//! Render a mission's complete initial map scene to a PNG.
//!
//! This is a CLI wrapper around the regular game loader and renderer: mission
//! scripts, sprite resources, ambiance, masks, decals, and entities all use
//! the same code paths as the game. Frame zero is after mission `Initialize`,
//! before the first simulation tick and `PostInitialize`; `--frame N` runs N
//! normal game frames before capture.
//!
//! Usage:
//!   ROBINHOOD_DATA_DIR=datadirs/fullgame_gog \
//!     cargo run --example render_mission_map -- S02_Lei_MP \
//!       --frame 10 --reveal-all --headless -o "Save Scarlett.png"
#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(about = "Render a mission's initial complete map scene to PNG")]
struct Args {
    /// Mission filename without the `.rhm` extension.
    #[arg(value_name = "MISSION")]
    mission: String,

    /// Proto-level filename without `.rhp`; defaults to the mission name.
    #[arg(long, value_name = "PROTO")]
    proto: Option<String>,

    /// Destination PNG. Defaults to `<mission>-start.png` in the invoking directory.
    #[arg(short, long, value_name = "PNG")]
    output: Option<PathBuf>,

    /// Absolute simulation frame to capture. Zero is the pristine pre-tick
    /// mission state.
    #[arg(long, default_value_t = 0, value_name = "N")]
    frame: u32,

    /// Reveal every blipped NPC before rendering, like the original
    /// `UBIQUITY` / `UNBLIP` cheat.
    #[arg(long, visible_alias = "unblip-all")]
    reveal_all: bool,

    /// Directory containing the game's `Data` folder. Equivalent to
    /// `ROBINHOOD_DATA_DIR`.
    #[arg(long, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    /// Keep the required GPU-backed window hidden. Rendering still occurs for
    /// the final offscreen screenshot.
    #[arg(long)]
    headless: bool,

    /// Fog/night-tint all Day-based world sprites on fog or night maps.
    #[arg(long, conflicts_with = "no_fog_tint_all_sprites")]
    fog_tint_all_sprites: bool,

    /// Force original sprite-variant behavior, regardless of profile config.
    #[arg(long)]
    no_fog_tint_all_sprites: bool,

    /// Render the current explored/visible gameplay fog. By default complete
    /// map exports bypass fog so the entire authored scene is captured.
    #[arg(long)]
    fog_of_war: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(0) => ExitCode::SUCCESS,
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(err) => {
            robin_rs::init_tracing();
            tracing::error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<i32> {
    let args = Args::parse();
    let window_visible = !args.headless;
    let fog_tint_all_sprites = if args.fog_tint_all_sprites {
        Some(true)
    } else if args.no_fog_tint_all_sprites {
        Some(false)
    } else {
        None
    };
    let invocation_dir =
        std::env::current_dir().context("failed to determine current directory")?;
    let output = absolute_path(
        &invocation_dir,
        args.output
            .unwrap_or_else(|| PathBuf::from(format!("{}-start.png", args.mission))),
    );

    let data_dir = args
        .data_dir
        .map(|data_dir| absolute_path(&invocation_dir, data_dir));

    // Reuse the launcher's parser so GlobalOptions and ApplicationContext
    // receive exactly the same settings as a direct `robin --mission` run.
    let mut launcher_args = vec![
        OsString::from("render_mission_map"),
        OsString::from("--mission"),
        OsString::from(&args.mission),
        OsString::from("--no-sound"),
        OsString::from("--http-server=0"),
        OsString::from("--rollback-check=false"),
    ];
    if let Some(proto) = args.proto {
        launcher_args.push(OsString::from("--proto"));
        launcher_args.push(OsString::from(proto));
    }
    let mut game_args = robin_rs::main_entry::MissionLaunch::from(
        robin_rs::main_entry::try_parse_cli_from(launcher_args)?,
    );
    game_args.mission_start_map_output = Some(output.clone());
    game_args.mission_start_map_frame = args.frame;
    game_args.mission_start_reveal_all = args.reveal_all;
    game_args.mission_start_fog_of_war = args.fog_of_war;
    game_args.fast_forward = true;

    let (campaign, profiles, application_context) =
        robin_rs::main_entry::rust_init_with_data_dir(data_dir.as_deref())?;
    if let Some(enabled) = fog_tint_all_sprites {
        application_context
            .set_fog_tint_all_sprites_for_tool(enabled)
            .map_err(anyhow::Error::msg)?;
    }
    robin_rs::window::run_with_game_visibility(
        "Robin Hood — mission map renderer",
        1024,
        768,
        window_visible,
        move |mut window| async move {
            match robin_rs::main_entry::run_rust_game(
                &mut window,
                campaign,
                profiles,
                application_context,
                &game_args,
            )
            .await
            {
                Ok(code) => code,
                Err(err) => {
                    tracing::error!("Mission map render failed: {err}");
                    1
                }
            }
        },
    )
    .map_err(anyhow::Error::msg)
}

fn absolute_path(base: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}
