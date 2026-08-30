//! Outer run loops: main menu → mission selection → mission → repeat.

use crate::game_session::{SessionResult, run_mission, run_mission_headless, run_session};
use crate::host::ApplicationContext;
use crate::main_menu::multiplayer_menu::MultiplayerRole;
use crate::main_menu::{MainMenuChoice, show_main_menu};
use crate::window::GameWindow;
use robin_engine::campaign::Campaign;
use robin_engine::profiles as engine_profiles;
use robin_engine::profiles::MissionLocation;

use super::SaveLoadRequest;
use super::callbacks::{RustCallbacks, detect_demo_mode_with_context, force_mission_launch};
use super::cli::{CliArgs, requested_replay_data};

/// Run the game loop: main menu -> mission selection -> game -> repeat.
///
/// Outer loop: main menu (Start/Exit) -> campaign map -> game loop ->
/// back to menu.
pub async fn run_rust_game(
    window: &mut GameWindow,
    mut campaign: Campaign,
    mut profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    application_context: ApplicationContext,
    args: &CliArgs,
) -> Result<i32, String> {
    // Combine parsed launcher options with the services loaded by `rust_init`.
    // Every lock-backed value used below is copied into an owned snapshot
    // before the first `.await`; futures never retain a profile/key guard.
    let application_context =
        application_context.with_options(args.global_options.options().clone());
    let mut run_args = args.clone();
    super::cli::resolve_join_ticket(&mut run_args)?;
    run_args.global_options = application_context.clone();
    let args = &run_args;

    // Bring up the script-RPC transport. Native binds a loopback HTTP
    // listener; wasm installs the in-process JS bridge queue. The
    // handle lives in a process-global so the per-tick drain in
    // `game_session` can reach it without threading the queue through
    // every signature.
    crate::http_server::start_global(args.http_server)?;

    // Warm the process asset cache (sprite bank, sound banks,
    // exclamations) on a background thread while the menu runs, so the
    // first mission load doesn't pay for process-global parsing.
    let shipping_for_warmup = application_context.shipping_arc().ok().flatten();
    if !shipping_for_warmup
        .as_ref()
        .is_some_and(|datadir| !datadir.missions.is_empty())
    {
        crate::process_asset_cache::start_background_warmup(shipping_for_warmup, profiles.clone());
    }

    // The headless code in `game_session` short-circuits the per-frame render
    // block. Window and GPU initialization still happen before this point.
    if args.headless {
        tracing::info!("--headless: rendering disabled in game_session");
    }

    // The window/GPU was constructed by `crate::window::run_with_game`
    // and handed in as `window: &mut GameWindow`.  Just stamp the
    // logical render size so cursor/mouse events get back-transformed
    // through the present-time letterbox into logical coords.
    window.set_logical_size(window.width, window.height);

    // Env-var equivalent of `--wait-for-command`, used by the wasm
    // host — `Module.arguments` doesn't reach `std::env::args()` on
    // the `-sPROXY_TO_PTHREAD` worker, but `preRun`-set env vars do.
    let wait_for_command =
        args.wait_for_command || std::env::var_os("ROBIN_WAIT_FOR_COMMAND").is_some();

    // ── `--wait-for-command`: idle until a replay arrives via RPC ──
    // Data is fully loaded at this point (`rust_init` ran before
    // `run_rust_game`), so we just spin on the pending-replay slot
    // while pumping window events. When a replay lands, its header
    // picks the mission, then we move the decoded replay into
    // `CliArgs::replay_data` before `run_mission` so engine
    // construction can use the recording's RNG seed. Skips every
    // auto-start branch below (demo / sherwood / --replay / menu) by
    // design — the whole point is to let the JS side drive mission
    // selection without racing a hard-coded default.
    if wait_for_command {
        tracing::info!("--wait-for-command: data loaded, idling until load-replay RPC arrives");
        wait_for_replay_command(window).await;
        let Some(pending) = crate::http_server::take_pending_replay() else {
            return Err("--wait-for-command: replay disappeared before mission start".into());
        };
        let (replay_campaign, idx, location, replay_args, replay_rng_seed, replay_sim_config) =
            crate::game_session::prepare_replay_mission(
                std::sync::Arc::make_mut(&mut profiles),
                args,
                pending.data,
                pending.paused,
            )?;
        let mut callbacks = RustCallbacks::new(application_context.clone());
        let outcome = Box::pin(run_mission(
            window,
            &mut callbacks,
            replay_campaign,
            &profiles,
            idx,
            location,
            &replay_args,
            replay_rng_seed,
            replay_sim_config,
        ))
        .await;
        outcome.result?;
        return Ok(0);
    }

    // Replay metadata is authoritative for mission selection and frame-0
    // construction, so it must win over direct-mission, demo, and Sherwood
    // auto-detection.
    let replay_data = requested_replay_data(args)?;
    if let Some(data) = replay_data {
        let (replay_campaign, idx, location, replay_args, rng_seed, sim_config) =
            crate::game_session::prepare_replay_mission(
                std::sync::Arc::make_mut(&mut profiles),
                args,
                data,
                false,
            )?;
        let mut callbacks = RustCallbacks::new(application_context.clone());
        let outcome = Box::pin(run_mission(
            window,
            &mut callbacks,
            replay_campaign,
            &profiles,
            idx,
            location,
            &replay_args,
            rng_seed,
            sim_config,
        ))
        .await;
        outcome.result?;
        return Ok(0);
    }

    // Keep the same archive overlay alive for the entire direct mission that
    // the Custom Missions menu would mount around run_session.
    let _custom_mission_mount = args
        .custom_mission
        .as_deref()
        .map(|zip| {
            crate::mod_pack::mount_for_launch(zip, false, std::path::Path::new("."))
                .map_err(|error| format!("--custom-mission: {error}"))
        })
        .transpose()?;

    // ── `--mission`: original-launcher style direct mission forcing. ──
    // Mirrors `-MISSION foo [-PROTO bar]`: select an existing profile
    // when present, otherwise append a synthetic profile and launch it.
    if let Some((idx, location)) =
        force_mission_launch(&mut campaign, &mut profiles, &application_context, args)?
    {
        let mut callbacks = RustCallbacks::new(application_context.clone());
        let outcome = Box::pin(run_mission(
            window,
            &mut callbacks,
            campaign,
            &profiles,
            idx,
            location,
            args,
            0,
            crate::game_session::initial_sim_config(args),
        ))
        .await;
        outcome.result?;
        return Ok(0);
    }

    // Demo detection: check which demo data files exist.
    let demo_config = if args.force_main_menu {
        tracing::info!("--force-main-menu: skipping demo auto-start detection");
        None
    } else {
        detect_demo_mode_with_context(&application_context)
    };
    if let Some((mission_name, proto_name, pcs, location)) = demo_config {
        tracing::info!(
            "Demo mode detected — mission={mission_name}, proto={proto_name}, PCs={pcs}"
        );
        campaign.reset(&profiles, application_context.sim_config().difficulty);
        // Parse the PC string to build the gang from specific characters.
        campaign.create_gang_from_pcs(pcs, &profiles, application_context.sim_config().difficulty);
        campaign.add_all_to_mission_team();
        // Demo mission is index 1 (index 0 = Sherwood)
        campaign.current_mission_idx = Some(1);
        let mut callbacks = RustCallbacks::new(application_context.clone());
        let outcome = Box::pin(run_mission(
            window,
            &mut callbacks,
            campaign,
            &profiles,
            1,
            location,
            args,
            0,
            crate::game_session::initial_sim_config(args),
        ))
        .await;
        outcome.result?;
        return Ok(0);
    }

    // ── `--sherwood`: skip the main menu, drop into Sherwood HQ. ──
    // Resets the campaign (same as clicking "Start"), forces the next
    // mission slot to Sherwood (idx 0), and runs the mission directly
    // — bypassing the campaign-map overlay that normally sits between
    // menu and Sherwood.
    if args.sherwood {
        tracing::info!("--sherwood: launching directly into the Sherwood HQ mission");
        campaign.reset(&profiles, application_context.sim_config().difficulty);
        campaign.force_next_mission(0);
        campaign.current_mission_idx = Some(0);
        let mut callbacks = RustCallbacks::new(application_context.clone());
        let outcome = Box::pin(run_mission(
            window,
            &mut callbacks,
            campaign,
            &profiles,
            0,
            MissionLocation::Sherwood,
            args,
            0,
            crate::game_session::initial_sim_config(args),
        ))
        .await;
        outcome.result?;
        return Ok(0);
    }

    // ── `--headless` requires a non-menu entry path ──
    // The main menu is fully rendered: with no display there's no way
    // to navigate it.  The demo and `--sherwood` branches above
    // already cover the headless use cases (replay scrubbing,
    // automated tests, CI).
    if args.headless {
        return Err(
            "--headless requires --sherwood or a demo data dir; the main \
             menu cannot be navigated without a display."
                .into(),
        );
    }

    // ── Full game: outer main menu loop ──
    loop {
        let menu_choice = Box::pin(show_main_menu(
            window,
            &campaign,
            &profiles,
            &application_context,
        ))
        .await?;

        match menu_choice {
            MainMenuChoice::Start => {
                // Reset campaign for a new game
                campaign.reset(&profiles, application_context.sim_config().difficulty);
                tracing::info!("Campaign reset for new game");

                if let Some((mission_name, _proto_name, pcs, location)) =
                    detect_demo_mode_with_context(&application_context)
                {
                    tracing::info!(
                        "Main menu Start: demo datadir detected, launching `{mission_name}`"
                    );
                    campaign.create_gang_from_pcs(
                        pcs,
                        &profiles,
                        application_context.sim_config().difficulty,
                    );
                    campaign.add_all_to_mission_team();
                    let idx = campaign
                        .missions
                        .iter()
                        .position(|m| m.profile(&profiles).mission_filename == mission_name)
                        .ok_or_else(|| {
                            format!("demo mission `{mission_name}` is present in data but missing from campaign")
                        })?;
                    campaign.current_mission_idx = Some(idx);
                    let mut callbacks = RustCallbacks::new(application_context.clone());
                    let outcome = Box::pin(run_mission(
                        window,
                        &mut callbacks,
                        campaign,
                        &profiles,
                        idx,
                        location,
                        args,
                        0,
                        crate::game_session::initial_sim_config(args),
                    ))
                    .await;
                    campaign = outcome.campaign;
                    outcome.result?;
                    tracing::info!("Returned to main menu");
                    continue;
                }

                // Session always returns to menu (window close causes Quit → QuitToMenu)
                let outcome = Box::pin(run_session(
                    window,
                    campaign,
                    std::sync::Arc::make_mut(&mut profiles),
                    &application_context,
                    args,
                    None,
                ))
                .await;
                campaign = outcome.campaign;
                let SessionResult::QuitToMenu = outcome.result?;
                tracing::info!("Returned to main menu");
            }
            MainMenuChoice::Load { slot, mission_id } => {
                // Route the save into the session's `perform_pending_save_load`
                // path.  Point `next_mission_idx` at the save's mission so
                // the first `determine_next_mission` call enters the right
                // level; the session's cross-mission logic
                // (`game_session::run_session:LevelLoad`) handles any
                // mismatch if needed.
                if let Some(idx) = campaign
                    .missions
                    .iter()
                    .position(|m| m.profile(&profiles).id == mission_id)
                {
                    campaign.force_next_mission(idx);
                } else {
                    tracing::warn!(
                        "Main menu Load: no mission matching save header id {mission_id} — \
                         session will start at the default mission and apply the save in place"
                    );
                }
                tracing::info!("Main menu Load: slot={slot}, mission_id={mission_id}");
                let outcome = Box::pin(run_session(
                    window,
                    campaign,
                    std::sync::Arc::make_mut(&mut profiles),
                    &application_context,
                    args,
                    Some(SaveLoadRequest::Load {
                        slot: Some(slot),
                        mission_id,
                        save: None,
                    }),
                ))
                .await;
                campaign = outcome.campaign;
                let SessionResult::QuitToMenu = outcome.result?;
                tracing::info!("Returned to main menu from Load");
            }
            MainMenuChoice::Multiplayer(launch) => {
                let Some(idx) = campaign
                    .missions
                    .iter()
                    .position(|m| m.profile(&profiles).id == launch.mission_id)
                else {
                    return Err(format!(
                        "Multiplayer menu selected unknown mission id {} ({})",
                        launch.mission_id, launch.mission_name
                    ));
                };
                campaign.reset(&profiles, application_context.sim_config().difficulty);
                if let Some((_, _, pcs, _)) = detect_demo_mode_with_context(&application_context) {
                    campaign.create_gang_from_pcs(
                        pcs,
                        &profiles,
                        application_context.sim_config().difficulty,
                    );
                }
                campaign.force_next_mission(idx);
                let mut mp_args = args.clone();
                match launch.role {
                    MultiplayerRole::Host => {
                        tracing::info!(
                            mission = %launch.mission_name,
                            "Main menu Multiplayer: hosting selected mission"
                        );
                        mp_args.server = true;
                        mp_args.connect = None;
                    }
                    MultiplayerRole::Client { connect_addr } => {
                        tracing::info!(
                            mission = %launch.mission_name,
                            connect = %connect_addr,
                            "Main menu Multiplayer: joining selected mission"
                        );
                        mp_args.server = false;
                        mp_args.connect = Some(connect_addr);
                    }
                }
                mp_args.mp_start_at_epoch_ms = launch.start_at_epoch_ms;
                mp_args.mp_expected_players = Some(launch.expected_players);
                mp_args.mp_mission_profile_id = Some(launch.mission_id);
                let outcome = Box::pin(run_session(
                    window,
                    campaign,
                    std::sync::Arc::make_mut(&mut profiles),
                    &application_context,
                    &mp_args,
                    None,
                ))
                .await;
                campaign = outcome.campaign;
                let SessionResult::QuitToMenu = outcome.result?;
                tracing::info!("Returned to main menu from Multiplayer");
            }
            MainMenuChoice::CustomMission(
                crate::main_menu::custom_missions::CustomMissionChoice::Hackable { mission, title },
            ) => {
                tracing::info!("Main menu CustomMission (hackable): {title} ({mission})");
                let profiles_mut = std::sync::Arc::make_mut(&mut profiles);
                campaign.reset(profiles_mut, application_context.sim_config().difficulty);
                // Hackable levels are standalone sandboxes with no preceding
                // campaign mission to inherit a gang from; start with Robin.
                campaign.create_gang_from_pcs(
                    "R",
                    profiles_mut,
                    application_context.sim_config().difficulty,
                );
                let idx = campaign
                    .force_next_mission_by_name(profiles_mut, &mission, &mission, true)
                    .ok_or_else(|| format!("failed to create hackable mission `{mission}`"))?;
                campaign.current_mission_idx = Some(idx);
                let location = campaign.missions[idx].profile(profiles_mut).location;
                let mut callbacks = RustCallbacks::new(application_context.clone());
                let mut sim_config = crate::game_session::initial_sim_config(args);
                // Hackable descriptors carry no SCB StartUp class, so the
                // script VM must stay off.
                sim_config.script_enabled = false;
                let outcome = Box::pin(run_mission(
                    window,
                    &mut callbacks,
                    campaign,
                    &profiles,
                    idx,
                    location,
                    args,
                    0,
                    sim_config,
                ))
                .await;
                campaign = outcome.campaign;
                outcome.result?;
                tracing::info!("Returned to main menu from hackable level `{mission}`");
            }
            MainMenuChoice::CustomMission(
                crate::main_menu::custom_missions::CustomMissionChoice::Mod(launch),
            ) => {
                let mods_root = crate::mod_pack::default_mods_root();
                tracing::info!(
                    "Main menu CustomMission: slug={} rhm={} map={} spellforge={}",
                    launch.slug,
                    launch.rhm_basename,
                    launch.map_filename,
                    launch.requires_spellforge
                );
                let mount_guard = match crate::mod_pack::mount_for_launch(
                    &launch.version_zip,
                    launch.requires_spellforge,
                    &mods_root,
                ) {
                    Ok(g) => g,
                    Err(e) => {
                        tracing::error!("CustomMission: mount failed: {e}");
                        continue;
                    }
                };
                let profiles_mut = std::sync::Arc::make_mut(&mut profiles);
                campaign.reset(profiles_mut, application_context.sim_config().difficulty);
                let idx = match campaign.force_next_mission_by_name(
                    profiles_mut,
                    &launch.rhm_basename,
                    &launch.map_filename,
                    true,
                ) {
                    Some(i) => i,
                    None => {
                        tracing::error!(
                            "CustomMission: force_next_mission_by_name returned None for rhm={} proto={}",
                            launch.rhm_basename,
                            launch.map_filename
                        );
                        drop(mount_guard);
                        continue;
                    }
                };
                campaign.current_mission_idx = Some(idx);
                // Demo-mode init: if the active datadir is a demo, the
                // gang has to be created from the PCs declared in the
                // demo manifest, same as MainMenuChoice::Start. Custom
                // missions don't dictate roster, they piggyback on
                // whatever the datadir's campaign would have used.
                if let Some((_, _, pcs, _)) = detect_demo_mode_with_context(&application_context) {
                    campaign.create_gang_from_pcs(
                        pcs,
                        &profiles,
                        application_context.sim_config().difficulty,
                    );
                    campaign.add_all_to_mission_team();
                }
                // If the mission ships a Lua companion, hand it off
                // to `run_mission` via the CLI args so `game_session`
                // can build a `LuaSession` against the just-loaded
                // engine. `LuaSession::start` is the one that decides
                // there's nothing to do for vanilla launches; passing
                // the pending struct unconditionally keeps the
                // decision in one place.
                let mut session_args = args.clone();
                session_args.pending_lua_mission = Some(crate::main_entry::PendingLuaMission {
                    slug: launch.slug.clone(),
                    rhm_basename: launch.rhm_basename.clone(),
                    version_zip: launch.version_zip.clone(),
                    mods_root: mods_root.clone(),
                    requires_spellforge: launch.requires_spellforge,
                });
                if launch.requires_spellforge && session_args.rollback_check {
                    // The checker is a default diagnostic, not a mode selected
                    // by the custom-mission picker. Spellforge cannot
                    // participate because its Lua state is not snapshotted;
                    // make that opt-out explicit and visible while preserving
                    // ordinary single-player custom-mission launch.
                    tracing::warn!(
                        mission = %launch.rhm_basename,
                        "Spellforge: disabling the default rollback checker for this normal single-player launch; explicit deterministic modes reject Spellforge"
                    );
                    session_args.rollback_check = false;
                }
                let outcome = Box::pin(run_session(
                    window,
                    campaign,
                    std::sync::Arc::make_mut(&mut profiles),
                    &application_context,
                    &session_args,
                    None,
                ))
                .await;
                campaign = outcome.campaign;
                let SessionResult::QuitToMenu = outcome.result?;
                drop(mount_guard);
                tracing::info!("Returned to main menu from CustomMission");
            }
            MainMenuChoice::Exit => {
                tracing::info!("Player exited from main menu");
                return Ok(0);
            }
        }
    }
}

pub async fn run_rust_game_headless(
    mut campaign: Campaign,
    mut profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    application_context: ApplicationContext,
    args: &CliArgs,
) -> Result<i32, String> {
    let application_context =
        application_context.with_options(args.global_options.options().clone());
    let mut run_args = args.clone();
    super::cli::resolve_join_ticket(&mut run_args)?;
    run_args.global_options = application_context.clone();
    let args = &run_args;

    #[cfg(not(target_arch = "wasm32"))]
    crate::http_server::start_global(args.http_server)?;

    let shipping_for_warmup = application_context.shipping_arc().ok().flatten();
    if !shipping_for_warmup
        .as_ref()
        .is_some_and(|datadir| !datadir.missions.is_empty())
    {
        crate::process_asset_cache::start_background_warmup(shipping_for_warmup, profiles.clone());
    }

    tracing::info!("--headless: running without winit, wgpu, renderer, or audio backend");

    let initial_sim_config = crate::game_session::initial_sim_config(args);
    let mut prepared_args = None;
    let replay_data = requested_replay_data(args)?;
    let launch = if let Some(data) = replay_data {
        let prepared = crate::game_session::prepare_replay_mission(
            std::sync::Arc::make_mut(&mut profiles),
            args,
            data,
            false,
        )?;
        campaign = prepared.0;
        prepared_args = Some(prepared.3);
        Some((prepared.1, prepared.2, prepared.4, prepared.5))
    } else if let Some((idx, location)) =
        force_mission_launch(&mut campaign, &mut profiles, &application_context, args)?
    {
        Some((idx, location, 0, initial_sim_config))
    } else if let Some((mission_name, _proto_name, pcs, location)) =
        detect_demo_mode_with_context(&application_context)
    {
        campaign.reset(&profiles, application_context.sim_config().difficulty);
        campaign.create_gang_from_pcs(pcs, &profiles, application_context.sim_config().difficulty);
        campaign.add_all_to_mission_team();
        let idx = campaign
            .missions
            .iter()
            .position(|m| m.profile(&profiles).mission_filename == mission_name)
            .ok_or_else(|| {
                format!(
                    "demo mission `{mission_name}` is present in data but missing from campaign"
                )
            })?;
        campaign.current_mission_idx = Some(idx);
        Some((idx, location, 0, initial_sim_config))
    } else if args.sherwood {
        campaign.reset(&profiles, application_context.sim_config().difficulty);
        campaign.force_next_mission(0);
        campaign.current_mission_idx = Some(0);
        Some((0, MissionLocation::Sherwood, 0, initial_sim_config))
    } else {
        None
    };
    let mission_args = prepared_args.as_ref().unwrap_or(args);

    let Some((idx, location, rng_seed, sim_config)) = launch else {
        return Err(
            "--headless requires --sherwood, --replay, or a demo data dir; the main menu cannot be navigated without a display."
                .into(),
        );
    };

    let mut callbacks = RustCallbacks::new(application_context);
    let outcome = run_mission_headless(
        &mut callbacks,
        campaign,
        &profiles,
        idx,
        location,
        mission_args,
        rng_seed,
        sim_config,
    )
    .await;
    outcome.result?;
    Ok(0)
}

/// Block until a `load-replay` RPC call queues a pending replay,
/// returning the mission-id stamped in that replay's header.
///
/// Paints a dark blue canvas (so the user sees *something* other
/// than the browser's default white) and pumps window events on a 20 Hz
/// poll. The pending replay is only peeked here; the caller consumes and
/// prepares all frame-0 metadata before constructing the mission Engine.
async fn wait_for_replay_command(window: &mut GameWindow) {
    loop {
        // Pump events — winit needs the app to drain its queue every
        // frame to stay responsive.
        let _ = window.poll_events();

        // Drain RPCs — the `load-replay` endpoint is how this loop
        // exits, and the normal `drain_global` path needs an engine.
        // `drain_pre_engine` handles `load-replay` / `info` and
        // rejects everything else with an "engine not ready" reply.
        crate::http_server::drain_pre_engine();

        window.clear_to_color(wgpu::Color {
            r: 0.01,
            g: 0.02,
            b: 0.08,
            a: 1.0,
        });

        if crate::http_server::peek_pending_replay_mission_id().is_some() {
            return;
        }

        crate::window::sleep_ms(50).await;
    }
}
