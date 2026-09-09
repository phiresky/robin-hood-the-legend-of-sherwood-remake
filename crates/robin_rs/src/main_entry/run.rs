//! Outer run loops: main menu → mission selection → mission → repeat.

use crate::game_session::{SessionResult, run_mission, run_mission_headless, run_session};
use crate::host::ApplicationContext;
#[cfg(feature = "multiplayer")]
use crate::main_menu::multiplayer_menu::MultiplayerRole;
use crate::main_menu::{MainMenuChoice, show_main_menu};
use crate::window::GameWindow;
use robin_engine::campaign::Campaign;
use robin_engine::profiles as engine_profiles;
use robin_engine::profiles::MissionLocation;

use super::callbacks::{RustCallbacks, detect_demo_mode_with_context, force_mission_launch};
use super::cli::{CliArgs, requested_replay_data};

type ReplayLaunch = (
    Campaign,
    usize,
    MissionLocation,
    CliArgs,
    u64,
    robin_engine::engine::SimConfig,
);

/// In-process ownership handoff, never serialized or reconstructed from JS.
struct PreparedInitialReplay {
    profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    launch: ReplayLaunch,
    #[cfg(target_arch = "wasm32")]
    downloads: Option<crate::shipping_mission::EarlyMissionDownloads>,
}

#[cfg(target_arch = "wasm32")]
pub struct BrowserReplayPreparation {
    receiver: async_channel::Receiver<Result<PreparedInitialReplay, String>>,
    abort: futures::future::AbortHandle,
}

#[cfg(target_arch = "wasm32")]
impl Drop for BrowserReplayPreparation {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn replay_preparation_mode(value: Option<&str>) -> Result<bool, String> {
    match value {
        Some("late") => Ok(false),
        None | Some("early") => Ok(true),
        Some(value) => Err(format!(
            "invalid replay-preparation {value:?}; expected early or late"
        )),
    }
}

/// Start the admitted URL replay early; `replay-preparation=late` retains a
/// same-package comparison. Interactive and multiplayer ordering is unchanged.
#[cfg(target_arch = "wasm32")]
pub fn start_browser_replay_preparation(
    args: &CliArgs,
    profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    context: crate::host::ReadyApplicationContext,
) -> Result<Option<BrowserReplayPreparation>, String> {
    let window = web_sys::window().ok_or_else(|| "browser window is unavailable".to_string())?;
    let query = window
        .location()
        .search()
        .map_err(|error| format!("read browser query: {error:?}"))?;
    let query = web_sys::UrlSearchParams::new_with_str(&query)
        .map_err(|error| format!("parse browser query: {error:?}"))?;
    if !replay_preparation_mode(query.get("replay-preparation").as_deref())?
        || !args.wait_for_command
        || query.get("replay").is_none_or(|replay| replay.is_empty())
        || args.join.is_some()
        || args.force_main_menu
        || query.has("join")
    {
        return Ok(None);
    }
    let context: ApplicationContext = context
        .with_options(args.global_options.options().clone())
        .into();
    let mut args = args.clone();
    args.global_options = context.clone();
    let (sender, receiver) = async_channel::bounded(1);
    let (abort, registration) = futures::future::AbortHandle::new_pair();
    wasm_bindgen_futures::spawn_local(async move {
        let prepare = async move {
            // wasm_boot queues wasm_main before the shell awaits rpc(info)
            // and sends load-replay. Wait for the admitted queue, not a timing
            // assumption or another parse of URL bytes.
            loop {
                context.drain_http_pre_engine()?;
                if let Some(pending) = args.global_options.replay_launches().take_pending() {
                    let mut prepared_profiles = profiles.clone();
                    let launch = crate::game_session::prepare_replay_launch(
                        &context,
                        std::sync::Arc::make_mut(&mut prepared_profiles),
                        &args,
                        pending.data,
                        pending.paused,
                    )
                    .await?;
                    // Cold/custom resolution can yield. Respect a newer
                    // queued replay before starting any earlier one's I/O.
                    context.drain_http_pre_engine()?;
                    if args
                        .global_options
                        .replay_launches()
                        .pending_mission()
                        .is_some()
                    {
                        continue;
                    }
                    let shipping = context.shipping_arc()?;
                    let archive = launch
                        .3
                        .resolved_mission_assets
                        .as_ref()
                        .is_some_and(|resolved| resolved.is_archive());
                    let mission = launch.0.missions[launch.1]
                        .profile(&prepared_profiles)
                        .mission_filename
                        .clone();
                    let downloads = match shipping {
                        Some(datadir) if !archive && datadir.has_mission(&mission) => Some(
                            crate::shipping_mission::start_early_downloads(
                                datadir,
                                &mission,
                                &launch.0,
                                &prepared_profiles,
                            )
                            .map_err(|error| format!("early replay downloads: {error:#}"))?,
                        ),
                        _ => None,
                    };
                    return Ok(PreparedInitialReplay {
                        profiles: prepared_profiles,
                        launch,
                        downloads,
                    });
                }
                crate::window::sleep_ms(1).await;
            }
        };
        if let Ok(result) = futures::future::Abortable::new(prepare, registration).await {
            let _ = sender.send(result).await;
        }
    });
    Ok(Some(BrowserReplayPreparation { receiver, abort }))
}

#[cfg(target_arch = "wasm32")]
pub async fn run_rust_game_with_browser_preparation(
    window: &mut GameWindow,
    campaign: Campaign,
    profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    context: crate::host::ReadyApplicationContext,
    args: &CliArgs,
    preparation: Option<BrowserReplayPreparation>,
) -> Result<i32, String> {
    let owner = (*context).clone();
    let result = async {
        let prepared = match preparation {
            Some(preparation) => {
                let prepared = preparation
                    .receiver
                    .recv()
                    .await
                    .map_err(|error| format!("early replay preparation dropped: {error}"))??;
                context.drain_http_pre_engine()?;
                if context.replay_launches().pending_mission().is_some() {
                    // Supersession before mission construction releases and aborts
                    // the old prefix. The normal queue path takes the latest one.
                    drop(prepared);
                    None
                } else {
                    Some(prepared)
                }
            }
            None => None,
        };
        run_rust_game_active(window, campaign, profiles, context, args, prepared).await
    }
    .await;
    finish_application(result, owner.shutdown().await)
}

/// Run the game loop: main menu -> mission selection -> game -> repeat.
///
/// Outer loop: main menu (Start/Exit) -> campaign map -> game loop ->
/// back to menu.
pub async fn run_rust_game(
    window: &mut GameWindow,
    campaign: Campaign,
    profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    application_context: crate::host::ReadyApplicationContext,
    args: &CliArgs,
) -> Result<i32, String> {
    run_rust_game_inner(window, campaign, profiles, application_context, args, None).await
}

async fn run_rust_game_inner(
    window: &mut GameWindow,
    campaign: Campaign,
    profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    application_context: crate::host::ReadyApplicationContext,
    args: &CliArgs,
    prepared_replay: Option<PreparedInitialReplay>,
) -> Result<i32, String> {
    let owner = (*application_context).clone();
    let result = run_rust_game_active(
        window,
        campaign,
        profiles,
        application_context,
        args,
        prepared_replay,
    )
    .await;
    finish_application(result, owner.shutdown().await)
}

fn finish_application(
    result: Result<i32, String>,
    shutdown: Result<(), String>,
) -> Result<i32, String> {
    match (result, shutdown) {
        (result, Ok(())) => result,
        (Ok(_), Err(error)) => Err(format!("application shutdown: {error}")),
        (Err(error), Err(shutdown)) => Err(format!("{error}; application shutdown: {shutdown}")),
    }
}

async fn run_rust_game_active(
    window: &mut GameWindow,
    mut campaign: Campaign,
    mut profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    application_context: crate::host::ReadyApplicationContext,
    args: &CliArgs,
    prepared_replay: Option<PreparedInitialReplay>,
) -> Result<i32, String> {
    // Combine parsed launcher options with the services loaded by `rust_init`.
    // Every lock-backed value used below is copied into an owned snapshot
    // before the first `.await`; futures never retain a profile/key guard.
    let application_context: ApplicationContext = application_context
        .with_options(args.global_options.options().clone())
        .into();
    let mut run_args = args.clone();
    super::cli::resolve_join_ticket(&mut run_args)?;
    run_args.global_options = application_context.clone();
    let args = &run_args;

    // Respect both launch forms before admitting speculative menu audio.
    let wait_for_command =
        args.wait_for_command || std::env::var_os("ROBIN_WAIT_FOR_COMMAND").is_some();

    // Replay viewers bypass menus. Their mission audio is warmed by the
    // mission loader; prefetching menu music here wastes replay bandwidth.
    #[cfg(all(target_arch = "wasm32", feature = "audio"))]
    if application_context.options().sound_enabled
        && !wait_for_command
        && args.replay.is_none()
        && args.replay_data.is_none()
    {
        match application_context.browser_audio() {
            Ok(session) => wasm_bindgen_futures::spawn_local(async move {
                if let Err(error) = crate::audio_backend::preload_boot_catalog(&session).await {
                    tracing::warn!(
                        error,
                        "browser boot/menu audio warmup failed; lazy playback will retry"
                    );
                }
            }),
            Err(error) => tracing::warn!(error, "browser boot/menu audio unavailable"),
        }
    }

    // Bring up the script-RPC transport. Native binds a loopback HTTP
    // listener; wasm installs the in-process JS bridge queue. The
    // process-owned router binds requests to the active mission's ingress;
    // deferred work is retired when that mission ends.
    application_context.start_http_transport(args.http_server)?;

    // Warm this application's asset cache (sprite bank, sound banks,
    // exclamations) on a background thread while the menu runs, so the
    // first mission load doesn't pay for application-lifetime parsing.
    let shipping_for_warmup = application_context.shipping_arc()?;
    #[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
    let projection_export = args.simulation_content_export.is_some();
    #[cfg(not(all(feature = "projection-export", not(target_arch = "wasm32"))))]
    let projection_export = false;
    if !projection_export
        && !shipping_for_warmup
            .as_ref()
            .is_some_and(|datadir| !datadir.missions.is_empty())
    {
        application_context.asset_cache()?.start_background_warmup(
            shipping_for_warmup,
            profiles.clone(),
            application_context.preparation_files()?.clone(),
        );
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
        // Keep the I/O owner alive through the mission; all unused entries
        // are canceled if setup fails or this replay is superseded.
        #[cfg(target_arch = "wasm32")]
        let mut _early_downloads = None;
        let (replay_campaign, idx, location, replay_args, replay_rng_seed, replay_sim_config) =
            if let Some(prepared) = prepared_replay {
                profiles = prepared.profiles;
                #[cfg(target_arch = "wasm32")]
                {
                    _early_downloads = prepared.downloads;
                }
                prepared.launch
            } else {
                wait_for_replay_command(window, &args.global_options).await?;
                let pending = args
                    .global_options
                    .replay_launches()
                    .take_pending()
                    .ok_or_else(|| {
                        "--wait-for-command: replay disappeared before mission start".to_string()
                    })?;
                crate::game_session::prepare_replay_launch(
                    &application_context,
                    std::sync::Arc::make_mut(&mut profiles),
                    args,
                    pending.data,
                    pending.paused,
                )
                .await?
            };
        let Some(mut callbacks) =
            RustCallbacks::new_for_window(application_context.clone(), window).await?
        else {
            return Ok(0);
        };
        let outcome = Box::pin(run_mission(
            window,
            &mut callbacks,
            replay_campaign,
            std::sync::Arc::make_mut(&mut profiles),
            idx,
            location,
            replay_args,
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
            crate::game_session::prepare_replay_launch(
                &application_context,
                std::sync::Arc::make_mut(&mut profiles),
                args,
                data,
                false,
            )
            .await?;
        let Some(mut callbacks) =
            RustCallbacks::new_for_window(application_context.clone(), window).await?
        else {
            return Ok(0);
        };
        let outcome = Box::pin(run_mission(
            window,
            &mut callbacks,
            replay_campaign,
            std::sync::Arc::make_mut(&mut profiles),
            idx,
            location,
            replay_args,
            rng_seed,
            sim_config,
        ))
        .await;
        outcome.result?;
        return Ok(0);
    }

    // Direct custom missions cross the same exact-byte admission boundary as
    // picker launches before profile selection or engine construction.
    let direct_args = prepare_direct_custom_mission_args(args, &profiles, &application_context)?;
    let args = direct_args.as_ref().unwrap_or(args);

    // ── `--mission`: original-launcher style direct mission forcing. ──
    // Mirrors `-MISSION foo [-PROTO bar]`: select an existing profile
    // when present, otherwise append a synthetic profile and launch it.
    if let Some((idx, location)) =
        force_mission_launch(&mut campaign, &mut profiles, &application_context, args)?
    {
        let Some(mut callbacks) =
            RustCallbacks::new_for_window(application_context.clone(), window).await?
        else {
            return Ok(0);
        };
        let mission_args = args.clone();
        let sim_config = crate::game_session::initial_sim_config(args);
        // Transfer the sole prepared overlay lease into the direct loop. A
        // later RPC replay may replace it before resolving a different archive.
        drop(direct_args);
        let outcome = Box::pin(run_mission(
            window,
            &mut callbacks,
            campaign,
            std::sync::Arc::make_mut(&mut profiles),
            idx,
            location,
            mission_args,
            0,
            sim_config,
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
        let Some(mut callbacks) =
            RustCallbacks::new_for_window(application_context.clone(), window).await?
        else {
            return Ok(0);
        };
        let outcome = Box::pin(run_mission(
            window,
            &mut callbacks,
            campaign,
            std::sync::Arc::make_mut(&mut profiles),
            1,
            location,
            args.clone(),
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
        let Some(mut callbacks) =
            RustCallbacks::new_for_window(application_context.clone(), window).await?
        else {
            return Ok(0);
        };
        let outcome = Box::pin(run_mission(
            window,
            &mut callbacks,
            campaign,
            std::sync::Arc::make_mut(&mut profiles),
            0,
            MissionLocation::Sherwood,
            args.clone(),
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
    let mut reopen_main_options = false;
    #[cfg(target_arch = "wasm32")]
    let mut pending_direct_browser_invite = args.join.as_deref();
    #[cfg(not(target_arch = "wasm32"))]
    let mut pending_direct_browser_invite: Option<&str> = None;
    loop {
        let open_options_initially = std::mem::take(&mut reopen_main_options);
        let menu_choice = Box::pin(show_main_menu(
            window,
            &campaign,
            &profiles,
            &application_context,
            open_options_initially,
            pending_direct_browser_invite.take(),
        ))
        .await?;

        match menu_choice {
            MainMenuChoice::RedisplayOptions => {
                reopen_main_options = true;
                continue;
            }
            MainMenuChoice::Start => {
                // Play resumes the latest checkpoint through the same save
                // preflight as Load, including its saved mission and campaign.
                let Some(callbacks) =
                    RustCallbacks::new_for_window(application_context.clone(), window).await?
                else {
                    if window.close_requested {
                        return Ok(0);
                    }
                    continue;
                };
                if let Some(index) = callbacks.save_manager.find_resume_target() {
                    let slot = callbacks.save_manager.slot_name(index)?;
                    let mission_id = callbacks
                        .save_manager
                        .slot_mission_id(index)
                        .expect("resume slot disappeared from the loaded save index");
                    drop(callbacks);
                    let outcome = Box::pin(run_session(
                        window,
                        campaign,
                        std::sync::Arc::make_mut(&mut profiles),
                        &application_context,
                        args,
                        Some((slot, mission_id)),
                    ))
                    .await;
                    campaign = outcome.campaign;
                    if outcome.result? == SessionResult::ExitRequested {
                        return Ok(0);
                    }
                    continue;
                }
                drop(callbacks);
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
                    let Some(mut callbacks) =
                        RustCallbacks::new_for_window(application_context.clone(), window).await?
                    else {
                        if window.close_requested {
                            return Ok(0);
                        }
                        continue;
                    };
                    let outcome = Box::pin(run_mission(
                        window,
                        &mut callbacks,
                        campaign,
                        std::sync::Arc::make_mut(&mut profiles),
                        idx,
                        location,
                        args.clone(),
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
                if outcome.result? == SessionResult::ExitRequested {
                    return Ok(0);
                }
                tracing::info!("Returned to main menu");
            }
            MainMenuChoice::Load { slot, mission_id } => {
                // Do not consult the ambient profile graph here. The session
                // first decodes the selected current-schema payload, restores
                // and mounts its exact mission descriptor, and only then
                // validates/reconstructs the saved profile index.
                tracing::info!(
                    "Main menu Load: slot={}, mission_id={mission_id}",
                    slot.as_str()
                );
                let outcome = Box::pin(run_session(
                    window,
                    campaign,
                    std::sync::Arc::make_mut(&mut profiles),
                    &application_context,
                    args,
                    Some((slot, mission_id)),
                ))
                .await;
                campaign = outcome.campaign;
                if outcome.result? == SessionResult::ExitRequested {
                    return Ok(0);
                }
                tracing::info!("Returned to main menu from Load");
            }
            #[cfg(feature = "multiplayer")]
            MainMenuChoice::Multiplayer(launch) => {
                let mut mp_args = args.clone();
                // A shell-provided join artifact has now been consumed by the
                // authenticated interactive preflight. The exact connection
                // address and prepared package below are its sole mission
                // bootstrap authority.
                mp_args.join = None;
                if let Some(encoded) = launch.distributed_mod.as_ref() {
                    let validated = crate::distributed_mod::DistributedModPackage::decode(encoded)
                        .map_err(|error| {
                            format!("Multiplayer full-mod package is invalid: {error}")
                        })?;
                    if validated.package.manifest.mission_basename != launch.mission_name {
                        return Err(format!(
                            "Multiplayer full mod contains mission `{}`, lobby selected `{}`",
                            validated.package.manifest.mission_basename, launch.mission_name
                        ));
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    let prepared =
                        crate::mission_asset_launch::prepare_cached_distributed_custom_mission(
                            &application_context,
                            &validated,
                            std::sync::Arc::clone(encoded),
                            launch.distributed_installed_locator.clone(),
                        )
                        .map_err(|error| {
                            format!("Prepare exact multiplayer mission assets: {error}")
                        })?;
                    #[cfg(target_arch = "wasm32")]
                    let prepared = crate::mission_asset_launch::prepare_distributed_custom_mission(
                        &validated,
                        encoded.len() as u64,
                        launch.distributed_installed_locator.clone(),
                        application_context.preparation_files()?.clone(),
                    )
                    .map_err(|error| {
                        format!("Prepare exact browser multiplayer mission assets: {error}")
                    })?;
                    let profiles_mut = std::sync::Arc::make_mut(&mut profiles);
                    campaign.reset(profiles_mut, application_context.sim_config().difficulty);
                    let idx = campaign
                        .force_next_mission_by_name(
                            profiles_mut,
                            &validated.package.manifest.mission_basename,
                            &validated.package.manifest.map_filename,
                            true,
                        )
                        .ok_or_else(|| {
                            format!(
                                "failed to construct multiplayer custom mission `{}`",
                                validated.package.manifest.mission_basename
                            )
                        })?;
                    campaign.current_mission_idx = Some(idx);
                    if let Some((_, _, pcs, _)) =
                        detect_demo_mode_with_context(&application_context)
                    {
                        campaign.create_gang_from_pcs(
                            pcs,
                            profiles_mut,
                            application_context.sim_config().difficulty,
                        );
                        campaign.add_all_to_mission_team();
                    }
                    mp_args.pending_lua_mission = Some(crate::main_entry::PendingLuaMission {
                        rhm_basename: validated.package.manifest.mission_basename.clone(),
                        requires_spellforge: validated.package.manifest.requires_spellforge,
                        spellforge_package: prepared.spellforge_package,
                    });
                    mp_args.pending_distributed_mod = Some(std::sync::Arc::clone(encoded));
                    mp_args.resolved_mission_assets = Some(prepared.resolved);
                } else {
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
                    if let Some((_, _, pcs, _)) =
                        detect_demo_mode_with_context(&application_context)
                    {
                        campaign.create_gang_from_pcs(
                            pcs,
                            &profiles,
                            application_context.sim_config().difficulty,
                        );
                    }
                    campaign.force_next_mission(idx);
                }
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
                if outcome.result? == SessionResult::ExitRequested {
                    return Ok(0);
                }
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
                let Some(mut callbacks) =
                    RustCallbacks::new_for_window(application_context.clone(), window).await?
                else {
                    if window.close_requested {
                        return Ok(0);
                    }
                    continue;
                };
                let mut sim_config = crate::game_session::initial_sim_config(args);
                // Hackable descriptors carry no SCB StartUp class, so the
                // script VM must stay off.
                sim_config.script_enabled = false;
                let outcome = Box::pin(run_mission(
                    window,
                    &mut callbacks,
                    campaign,
                    std::sync::Arc::make_mut(&mut profiles),
                    idx,
                    location,
                    args.clone(),
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
                tracing::info!(
                    "Main menu CustomMission: slug={} rhm={} map={} spellforge={}",
                    launch.slug,
                    launch.rhm_basename,
                    launch.map_filename,
                    launch.requires_spellforge
                );
                let prepared = match crate::mission_asset_launch::prepare_installed_custom_mission(
                    &launch,
                    application_context.preparation_files()?.clone(),
                ) {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        tracing::error!("CustomMission: exact asset preparation failed: {error}");
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
                // Hand the exact admission-produced package to mission
                // startup. Vanilla launches intentionally carry `None`; a
                // Spellforge startup is never allowed to reread local paths.
                let mut session_args = args.clone();
                session_args.pending_lua_mission = Some(crate::main_entry::PendingLuaMission {
                    rhm_basename: launch.rhm_basename.clone(),
                    requires_spellforge: launch.requires_spellforge,
                    spellforge_package: prepared.spellforge_package,
                });
                session_args.resolved_mission_assets = Some(prepared.resolved);
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
                if outcome.result? == SessionResult::ExitRequested {
                    return Ok(0);
                }
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
    campaign: Campaign,
    profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    application_context: crate::host::ReadyApplicationContext,
    args: &CliArgs,
) -> Result<i32, String> {
    let owner = (*application_context).clone();
    let result = run_rust_game_headless_active(campaign, profiles, application_context, args).await;
    finish_application(result, owner.shutdown().await)
}

async fn run_rust_game_headless_active(
    mut campaign: Campaign,
    mut profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    application_context: crate::host::ReadyApplicationContext,
    args: &CliArgs,
) -> Result<i32, String> {
    let application_context: ApplicationContext = application_context
        .with_options(args.global_options.options().clone())
        .into();
    let mut run_args = args.clone();
    super::cli::resolve_join_ticket(&mut run_args)?;
    run_args.global_options = application_context.clone();
    let args = &run_args;

    #[cfg(not(target_arch = "wasm32"))]
    application_context.start_http_transport(args.http_server)?;

    let shipping_for_warmup = application_context.shipping_arc()?;
    #[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
    let projection_export = args.simulation_content_export.is_some();
    #[cfg(not(all(feature = "projection-export", not(target_arch = "wasm32"))))]
    let projection_export = false;
    if !projection_export
        && !shipping_for_warmup
            .as_ref()
            .is_some_and(|datadir| !datadir.missions.is_empty())
    {
        application_context.asset_cache()?.start_background_warmup(
            shipping_for_warmup,
            profiles.clone(),
            application_context.preparation_files()?.clone(),
        );
    }

    tracing::info!("--headless: running without winit, wgpu, renderer, or audio backend");

    let mut prepared_args = None;
    let wait_for_command =
        args.wait_for_command || std::env::var_os("ROBIN_WAIT_FOR_COMMAND").is_some();
    let pending_replay = if wait_for_command {
        tracing::info!(
            "--headless --wait-for-command: data loaded, idling until load-replay RPC arrives"
        );
        wait_for_replay_command_headless(&args.global_options).await?;
        Some(
            args.global_options
                .replay_launches()
                .take_pending()
                .ok_or_else(|| {
                    "--headless --wait-for-command: replay disappeared before mission start"
                        .to_owned()
                })?,
        )
    } else {
        None
    };
    let (replay_data, replay_paused) = match pending_replay {
        Some(pending) => (Some(pending.data), pending.paused),
        None => (requested_replay_data(args)?, false),
    };
    if replay_data.is_none() {
        prepared_args = prepare_direct_custom_mission_args(args, &profiles, &application_context)?;
    }
    let selection_args = prepared_args.as_ref().unwrap_or(args);
    let initial_sim_config = crate::game_session::initial_sim_config(selection_args);
    let launch = if let Some(data) = replay_data {
        let prepared = crate::game_session::prepare_replay_launch(
            &application_context,
            std::sync::Arc::make_mut(&mut profiles),
            args,
            data,
            replay_paused,
        )
        .await?;
        campaign = prepared.0;
        prepared_args = Some(prepared.3);
        Some((prepared.1, prepared.2, prepared.4, prepared.5))
    } else if let Some((idx, location)) = force_mission_launch(
        &mut campaign,
        &mut profiles,
        &application_context,
        selection_args,
    )? {
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
    } else if selection_args.sherwood {
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

    #[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
    if projection_export {
        crate::game_session::export_official_mission_headless(
            campaign,
            &profiles,
            idx,
            location,
            mission_args,
            rng_seed,
            sim_config,
        )
        .await?;
        return Ok(0);
    }

    let mut callbacks =
        RustCallbacks::new(application_context).map_err(|error| error.to_string())?;
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

async fn wait_for_replay_command_headless(context: &ApplicationContext) -> Result<(), String> {
    loop {
        context.drain_http_pre_engine()?;
        if context.replay_launches().pending_mission().is_some() {
            return Ok(());
        }
        crate::window::sleep_ms(50).await;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn prepare_direct_custom_mission_args(
    args: &CliArgs,
    profiles: &engine_profiles::ProfileManager,
    application_context: &ApplicationContext,
) -> Result<Option<CliArgs>, String> {
    let Some(archive) = args.custom_mission.as_deref() else {
        return Ok(None);
    };
    let mission = args.mission.as_deref().ok_or_else(|| {
        "--custom-mission requires --mission even when arguments bypass clap".to_owned()
    })?;
    let map = args
        .proto
        .clone()
        .or_else(|| {
            profiles
                .missions
                .iter()
                .find(|profile| profile.mission_filename.eq_ignore_ascii_case(mission))
                .map(|profile| profile.proto_level_filename.clone())
        })
        .unwrap_or_else(|| mission.to_owned());

    let prepared = crate::mission_asset_launch::prepare_direct_custom_mission(
        application_context,
        archive,
        mission,
        &map,
        args.custom_mission_entry.as_deref(),
    )
    .map_err(|error| format!("--custom-mission: {error}"))?;

    if prepared.spellforge_package.is_some() {
        return Err(
            "--custom-mission accepts vanilla archives only; launch Spellforge content from the Custom Missions menu"
                .to_owned(),
        );
    }
    let mut prepared_args = args.clone();
    prepared_args.resolved_mission_assets = Some(prepared.resolved);
    Ok(Some(prepared_args))
}

#[cfg(target_arch = "wasm32")]
fn prepare_direct_custom_mission_args(
    args: &CliArgs,
    _profiles: &engine_profiles::ProfileManager,
    _application_context: &ApplicationContext,
) -> Result<Option<CliArgs>, String> {
    if args.custom_mission.is_some() {
        return Err(
            "--custom-mission filesystem paths are unavailable in browser builds; use canonical host-distributed content"
                .to_owned(),
        );
    }
    Ok(None)
}

/// Block until a `load-replay` RPC call queues a pending replay,
/// returning the mission-id stamped in that replay's header.
///
/// Paints a dark blue canvas (so the user sees *something* other
/// than the browser's default white) and pumps window events on a 20 Hz
/// poll. The pending replay is only peeked here; the caller consumes and
/// prepares all frame-0 metadata before constructing the mission Engine.
async fn wait_for_replay_command(
    window: &mut GameWindow,
    context: &ApplicationContext,
) -> Result<(), String> {
    loop {
        // Pump events — winit needs the app to drain its queue every
        // frame to stay responsive.
        let _ = window.poll_events();

        // Drain RPCs — the `load-replay` endpoint is how this loop
        // exits, and the normal `drain_global` path needs an engine.
        // `drain_pre_engine` handles `load-replay` / `info` and
        // rejects everything else with an "engine not ready" reply.
        context.drain_http_pre_engine()?;

        window.clear_to_color(wgpu::Color {
            r: 0.01,
            g: 0.02,
            b: 0.08,
            a: 1.0,
        });

        if context.replay_launches().pending_mission().is_some() {
            return Ok(());
        }

        crate::window::sleep_ms(50).await;
    }
}

#[cfg(test)]
mod early_replay_tests {
    #[test]
    fn shutdown_failure_does_not_hide_the_original_application_failure() {
        assert_eq!(super::finish_application(Ok(7), Ok(())), Ok(7));
        assert_eq!(
            super::finish_application(Err("run".into()), Ok(())),
            Err("run".into())
        );
        assert_eq!(
            super::finish_application(Ok(7), Err("drain".into())),
            Err("application shutdown: drain".into())
        );
        assert_eq!(
            super::finish_application(Err("run".into()), Err("drain".into())),
            Err("run; application shutdown: drain".into())
        );
    }

    #[test]
    fn early_mode_is_default_and_late_ablation_rejects_typos() {
        assert_eq!(super::replay_preparation_mode(None), Ok(true));
        assert_eq!(super::replay_preparation_mode(Some("late")), Ok(false));
        assert_eq!(super::replay_preparation_mode(Some("early")), Ok(true));
        assert!(super::replay_preparation_mode(Some("ealry")).is_err());
    }
}
