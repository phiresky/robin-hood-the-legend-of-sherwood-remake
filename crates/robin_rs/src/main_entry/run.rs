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

use std::sync::Arc;

use super::LaunchError;
use super::callbacks::{RustCallbacks, detect_demo_mode_with_context, force_mission_launch};
use super::launch::{LaunchConfig, MissionRequest, requested_replay_data};
#[cfg(feature = "multiplayer")]
use super::launch::{MissionContent, MultiplayerRoute};
use super::platform::prepare_direct_custom_mission_args;

use crate::game_session::PreparedReplayLaunch;

/// In-process ownership handoff, never serialized or reconstructed from JS.
struct PreparedInitialReplay {
    profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    launch: PreparedReplayLaunch,
    #[cfg(target_arch = "wasm32")]
    downloads: Option<crate::shipping_mission::EarlyMissionDownloads>,
}

#[cfg(target_arch = "wasm32")]
pub struct BrowserReplayPreparation {
    receiver: async_channel::Receiver<Result<PreparedInitialReplay, LaunchError>>,
    abort: futures::future::AbortHandle,
}

#[cfg(target_arch = "wasm32")]
impl Drop for BrowserReplayPreparation {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn replay_preparation_mode(value: Option<&str>) -> Result<bool, LaunchError> {
    match value {
        Some("late") => Ok(false),
        None | Some("early") => Ok(true),
        Some(value) => Err(LaunchError::arguments(format!(
            "invalid replay-preparation {value:?}; expected early or late"
        ))),
    }
}

/// Start the admitted URL replay early; `replay-preparation=late` retains a
/// same-package comparison. Interactive and multiplayer ordering is unchanged.
#[cfg(target_arch = "wasm32")]
pub fn start_browser_replay_preparation(
    args: &LaunchConfig,
    profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    context: crate::host::ReadyApplicationContext,
) -> Result<Option<BrowserReplayPreparation>, LaunchError> {
    let window =
        web_sys::window().ok_or_else(|| LaunchError::browser("browser window is unavailable"))?;
    let query = window
        .location()
        .search()
        .map_err(|error| LaunchError::browser(format!("read browser query: {error:?}")))?;
    let query = web_sys::UrlSearchParams::new_with_str(&query)
        .map_err(|error| LaunchError::browser(format!("parse browser query: {error:?}")))?;
    if !replay_preparation_mode(query.get("replay-preparation").as_deref())?
        || !args.cli.wait_for_command
        || query.get("replay").is_none_or(|replay| replay.is_empty())
        || args.cli.join.is_some()
        || args.cli.force_main_menu
        || query.has("join")
    {
        return Ok(None);
    }
    let context: ApplicationContext = context
        .with_options(args.global_options.options().clone())
        .into();
    // The early preparation owns its own binding of the launcher config to
    // the ready context; the run binds (and join-resolves) it again later.
    let args = MissionRequest::new(args.bind_run(
        context.clone(),
        args.cli.clone(),
        args.browser_join_redeemed,
    ));
    let (sender, receiver) = async_channel::bounded(1);
    let (abort, registration) = futures::future::AbortHandle::new_pair();
    wasm_bindgen_futures::spawn_local(async move {
        let prepare = async move {
            // wasm_boot queues wasm_main before the shell awaits rpc(info)
            // and sends load-replay. Wait for the admitted queue, not a timing
            // assumption or another parse of URL bytes.
            loop {
                // TODO(10/F11): leaf returns String (application RPC drain).
                context
                    .drain_http_pre_engine()
                    .map_err(LaunchError::application)?;
                if let Some(pending) = args.config.global_options.replay_launches().take_pending() {
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
                    context
                        .drain_http_pre_engine()
                        .map_err(LaunchError::application)?;
                    if args
                        .config
                        .global_options
                        .replay_launches()
                        .pending_mission()
                        .is_some()
                    {
                        continue;
                    }
                    let shipping = context.shipping_arc().map_err(LaunchError::application)?;
                    let archive = launch
                        .launch
                        .content
                        .resolved_mission_assets
                        .as_ref()
                        .is_some_and(|resolved| resolved.is_archive());
                    let mission = launch.campaign.missions[launch.mission_idx]
                        .profile(&prepared_profiles)
                        .mission_filename
                        .clone();
                    let downloads = match shipping {
                        Some(datadir) if !archive && datadir.has_mission(&mission) => Some(
                            crate::shipping_mission::start_early_downloads(
                                datadir,
                                &mission,
                                &launch.campaign,
                                &prepared_profiles,
                            )
                            .map_err(|error| {
                                LaunchError::replay(format!("early replay downloads: {error:#}"))
                            })?,
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
    args: &LaunchConfig,
    preparation: Option<BrowserReplayPreparation>,
) -> Result<i32, LaunchError> {
    let owner = (*context).clone();
    let result = async {
        let prepared = match preparation {
            Some(preparation) => {
                let prepared = preparation.receiver.recv().await.map_err(|error| {
                    LaunchError::replay(format!("early replay preparation dropped: {error}"))
                })??;
                context
                    .drain_http_pre_engine()
                    .map_err(LaunchError::application)?;
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
    args: &LaunchConfig,
) -> Result<i32, LaunchError> {
    run_rust_game_inner(window, campaign, profiles, application_context, args, None).await
}

async fn run_rust_game_inner(
    window: &mut GameWindow,
    campaign: Campaign,
    profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    application_context: crate::host::ReadyApplicationContext,
    args: &LaunchConfig,
    prepared_replay: Option<PreparedInitialReplay>,
) -> Result<i32, LaunchError> {
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

/// `shutdown` is the application-service drain; it reports text.
// TODO(10/F11): leaf returns String (`ApplicationContext::shutdown`).
fn finish_application(
    result: Result<i32, LaunchError>,
    shutdown: Result<(), String>,
) -> Result<i32, LaunchError> {
    LaunchError::with_shutdown(result, shutdown)
}

/// Resolve the launch route before starting any transport or speculative work.
/// Builds the run's shared configuration once; it owns snapshots, never
/// profile/key lock guards.
fn prepare_run_args(
    context: crate::host::ReadyApplicationContext,
    args: &LaunchConfig,
) -> Result<Arc<LaunchConfig>, LaunchError> {
    let context = context.with_options(args.global_options.options().clone());
    let mut cli = args.cli.clone();
    let mut browser_join_redeemed = args.browser_join_redeemed;
    super::cli::resolve_join_ticket(&mut cli, &mut browser_join_redeemed)?;
    Ok(args.bind_run(context.into(), cli, browser_join_redeemed))
}

fn projection_export_requested(args: &LaunchConfig) -> bool {
    #[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
    {
        args.simulation_content_export.is_some()
    }
    #[cfg(not(all(feature = "projection-export", not(target_arch = "wasm32"))))]
    {
        let _ = args;
        false
    }
}

/// Called after transport startup, in both graphical and headless entry paths.
fn warm_run_assets(
    args: &LaunchConfig,
    profiles: &std::sync::Arc<engine_profiles::ProfileManager>,
) -> Result<(), LaunchError> {
    let context = &args.global_options;
    let shipping = context.shipping_arc().map_err(LaunchError::application)?;
    if !projection_export_requested(args)
        && !shipping
            .as_ref()
            .is_some_and(|datadir| !datadir.missions.is_empty())
    {
        context
            .asset_cache()
            .map_err(LaunchError::application)?
            .start_background_warmup(
                shipping,
                profiles.clone(),
                context
                    .preparation_files()
                    .map_err(LaunchError::application)?
                    .clone(),
            );
    }
    Ok(())
}

async fn run_rust_game_active(
    window: &mut GameWindow,
    mut campaign: Campaign,
    mut profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    application_context: crate::host::ReadyApplicationContext,
    args: &LaunchConfig,
    prepared_replay: Option<PreparedInitialReplay>,
) -> Result<i32, LaunchError> {
    // Combine parsed launcher options with the services loaded by `rust_init`.
    // Every lock-backed value used below is copied into an owned snapshot
    // before the first `.await`; futures never retain a profile/key guard.
    let config = prepare_run_args(application_context, args)?;
    let application_context = config.global_options.clone();
    // The launch the configuration describes; direct paths move it into
    // their mission, the menu builds a fresh request per launch.
    let request = MissionRequest::new(Arc::clone(&config));

    // Respect both launch forms before admitting speculative menu audio.
    let wait_for_command = wait_for_command_requested(&config);

    // Replay viewers bypass menus. Their mission audio is warmed by the
    // mission loader; prefetching menu music here wastes replay bandwidth.
    #[cfg(all(target_arch = "wasm32", feature = "audio"))]
    if application_context.options().sound_enabled
        && !wait_for_command
        && request.replay.is_none()
        && request.replay_data.is_none()
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
    application_context
        .start_http_transport(config.cli.http_server)
        .map_err(LaunchError::application)?;

    // Warm this application's asset cache (sprite bank, sound banks,
    // exclamations) on a background thread while the menu runs, so the
    // first mission load doesn't pay for application-lifetime parsing.
    warm_run_assets(&config, &profiles)?;

    // The headless code in `game_session` short-circuits the per-frame render
    // block. Window and GPU initialization still happen before this point.
    if config.cli.headless {
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
    // `MissionRequest::replay_data` before `run_mission` so engine
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
        let prepared = if let Some(prepared) = prepared_replay {
            profiles = prepared.profiles;
            #[cfg(target_arch = "wasm32")]
            {
                _early_downloads = prepared.downloads;
            }
            prepared.launch
        } else {
            let pending = take_commanded_replay(
                &config.global_options,
                Some(&mut *window),
                "--wait-for-command",
            )
            .await?;
            crate::game_session::prepare_replay_launch(
                &application_context,
                std::sync::Arc::make_mut(&mut profiles),
                &request,
                pending.data,
                pending.paused,
            )
            .await?
        };
        return run_prepared_replay(window, profiles, application_context, prepared).await;
    }

    // Replay metadata is authoritative for mission selection and frame-0
    // construction, so it must win over direct-mission, demo, and Sherwood
    // auto-detection.
    let replay_data = requested_replay_data(&request)?;
    if let Some(data) = replay_data {
        let prepared = crate::game_session::prepare_replay_launch(
            &application_context,
            std::sync::Arc::make_mut(&mut profiles),
            &request,
            data,
            false,
        )
        .await?;
        return run_prepared_replay(window, profiles, application_context, prepared).await;
    }

    // Direct custom missions cross the same exact-byte admission boundary as
    // picker launches before profile selection or engine construction.
    let request = prepare_direct_custom_mission_args(request, &profiles, &application_context)?;

    // ── `--mission`: original-launcher style direct mission forcing. ──
    // Mirrors `-MISSION foo [-PROTO bar]`: select an existing profile
    // when present, otherwise append a synthetic profile and launch it.
    if let Some((idx, location)) =
        force_mission_launch(&mut campaign, &mut profiles, &application_context, &config)?
    {
        let Some(mut callbacks) =
            RustCallbacks::new_for_window(application_context.clone(), window).await?
        else {
            return Ok(0);
        };
        let sim_config = crate::game_session::initial_sim_config(&config);
        // Transfer the sole prepared overlay lease into the direct loop. A
        // later RPC replay may replace it before resolving a different archive.
        let outcome = Box::pin(run_mission(
            window,
            &mut callbacks,
            campaign,
            std::sync::Arc::make_mut(&mut profiles),
            idx,
            location,
            request,
            0,
            sim_config,
        ))
        .await;
        outcome.result?;
        return Ok(0);
    }

    // Demo detection: check which demo data files exist.
    let demo_config = if config.cli.force_main_menu {
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
            request,
            0,
            crate::game_session::initial_sim_config(&config),
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
    if config.cli.sherwood {
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
            request,
            0,
            crate::game_session::initial_sim_config(&config),
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
    if config.cli.headless {
        return Err(LaunchError::arguments(
            "--headless requires --sherwood or a demo data dir; the main \
             menu cannot be navigated without a display.",
        ));
    }

    // A `--custom-mission` request always has `--mission` and launched above,
    // so every menu launch builds its own request from the shared config.
    drop(request);
    run_main_menu(window, campaign, profiles, application_context, config).await
}

/// Both early/RPC and CLI replays cross this same post-admission boundary.
/// Callback construction stays after canonical asset/profile preparation.
async fn run_prepared_replay(
    window: &mut GameWindow,
    mut profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    context: ApplicationContext,
    prepared: PreparedReplayLaunch,
) -> Result<i32, LaunchError> {
    let Some(mut callbacks) = RustCallbacks::new_for_window(context, window).await? else {
        return Ok(0);
    };
    let outcome = Box::pin(run_mission(
        window,
        &mut callbacks,
        prepared.campaign,
        std::sync::Arc::make_mut(&mut profiles),
        prepared.mission_idx,
        prepared.location,
        prepared.launch,
        prepared.rng_seed,
        prepared.sim_config,
    ))
    .await;
    outcome.result?;
    Ok(0)
}

/// Services every main-menu launch uses. The campaign stays a local of
/// [`run_main_menu`] so each launch visibly moves it into the mission and back.
struct MainMenuContext<'a> {
    window: &'a mut GameWindow,
    profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    application_context: ApplicationContext,
    config: Arc<LaunchConfig>,
}

impl MainMenuContext<'_> {
    /// The plain launch of the run's configuration, built fresh per launch.
    fn launch_request(&self) -> MissionRequest {
        MissionRequest::new(Arc::clone(&self.config))
    }

    /// Run one menu-launched session. `None` means the session requested
    /// application exit; otherwise the campaign comes back for the next menu.
    async fn launch_session(
        &mut self,
        campaign: Campaign,
        request: MissionRequest,
        initial_load: Option<(crate::savegame::SlotName, u32)>,
    ) -> Result<Option<Campaign>, LaunchError> {
        let outcome = Box::pin(run_session(
            self.window,
            campaign,
            std::sync::Arc::make_mut(&mut self.profiles),
            &self.application_context,
            request,
            initial_load,
        ))
        .await;
        let campaign = outcome.campaign;
        if outcome.result.map_err(LaunchError::session)? == SessionResult::ExitRequested {
            return Ok(None);
        }
        Ok(Some(campaign))
    }

    /// Run one directly selected mission with the launcher's arguments.
    async fn launch_mission(
        &mut self,
        callbacks: &mut RustCallbacks,
        campaign: Campaign,
        idx: usize,
        location: MissionLocation,
        sim_config: robin_engine::engine::SimConfig,
    ) -> Result<Campaign, LaunchError> {
        let request = self.launch_request();
        let outcome = Box::pin(run_mission(
            self.window,
            callbacks,
            campaign,
            std::sync::Arc::make_mut(&mut self.profiles),
            idx,
            location,
            request,
            0,
            sim_config,
        ))
        .await;
        let campaign = outcome.campaign;
        outcome.result?;
        Ok(campaign)
    }

    /// Main-menu Start on a demo datadir: build the demo gang and select the
    /// demo mission. `None` when the datadir is not a demo.
    fn select_demo_start_mission(
        &self,
        campaign: &mut Campaign,
    ) -> Result<Option<(usize, MissionLocation)>, LaunchError> {
        let Some((mission_name, _proto_name, pcs, location)) =
            detect_demo_mode_with_context(&self.application_context)
        else {
            return Ok(None);
        };
        tracing::info!("Main menu Start: demo datadir detected, launching `{mission_name}`");
        campaign.create_gang_from_pcs(
            pcs,
            &self.profiles,
            self.application_context.sim_config().difficulty,
        );
        campaign.add_all_to_mission_team();
        let idx = campaign
            .missions
            .iter()
            .position(|m| m.profile(&self.profiles).mission_filename == mission_name)
            .ok_or_else(|| {
                LaunchError::campaign(format!(
                    "demo mission `{mission_name}` is present in data but missing from campaign"
                ))
            })?;
        campaign.current_mission_idx = Some(idx);
        Ok(Some((idx, location)))
    }

    /// Multiplayer lobby launch: prepare the exact mission (full-mod package
    /// or campaign profile), then apply the host/client role and lobby timing
    /// to a session-local launch.
    #[cfg(feature = "multiplayer")]
    fn prepare_multiplayer_launch(
        &mut self,
        campaign: &mut Campaign,
        launch: crate::main_menu::multiplayer_menu::MultiplayerLaunch,
    ) -> Result<MissionRequest, LaunchError> {
        let request = self.launch_request();
        let content = if let Some(encoded) = launch.distributed_mod.as_ref() {
            let validated = crate::distributed_mod::DistributedModPackage::decode(encoded)
                .map_err(|error| {
                    LaunchError::content(format!(
                        "Multiplayer full-mod package is invalid: {error}"
                    ))
                })?;
            if validated.package.manifest.mission_basename != launch.mission_name {
                return Err(LaunchError::content(format!(
                    "Multiplayer full mod contains mission `{}`, lobby selected `{}`",
                    validated.package.manifest.mission_basename, launch.mission_name
                )));
            }
            // TODO(10/F11): leaf returns String (distributed-mod asset admission).
            #[cfg(not(target_arch = "wasm32"))]
            let prepared = crate::mission_asset_launch::prepare_cached_distributed_custom_mission(
                &self.application_context,
                &validated,
                std::sync::Arc::clone(encoded),
                launch.distributed_installed_locator.clone(),
            )
            .map_err(|error| {
                LaunchError::content(format!("Prepare exact multiplayer mission assets: {error}"))
            })?;
            #[cfg(target_arch = "wasm32")]
            let prepared = crate::mission_asset_launch::prepare_distributed_custom_mission(
                &validated,
                encoded.len() as u64,
                launch.distributed_installed_locator.clone(),
                self.application_context
                    .preparation_files()
                    .map_err(LaunchError::application)?
                    .clone(),
            )
            .map_err(|error| {
                LaunchError::content(format!(
                    "Prepare exact browser multiplayer mission assets: {error}"
                ))
            })?;
            let profiles_mut = std::sync::Arc::make_mut(&mut self.profiles);
            campaign.reset(
                profiles_mut,
                self.application_context.sim_config().difficulty,
            );
            let idx = campaign
                .force_next_mission_by_name(
                    profiles_mut,
                    &validated.package.manifest.mission_basename,
                    &validated.package.manifest.map_filename,
                    true,
                )
                .ok_or_else(|| {
                    LaunchError::campaign(format!(
                        "failed to construct multiplayer custom mission `{}`",
                        validated.package.manifest.mission_basename
                    ))
                })?;
            campaign.current_mission_idx = Some(idx);
            if let Some((_, _, pcs, _)) = detect_demo_mode_with_context(&self.application_context) {
                campaign.create_gang_from_pcs(
                    pcs,
                    profiles_mut,
                    self.application_context.sim_config().difficulty,
                );
                campaign.add_all_to_mission_team();
            }
            Some(MissionContent {
                pending_lua_mission: Some(crate::main_entry::PendingLuaMission {
                    rhm_basename: validated.package.manifest.mission_basename.clone(),
                    requires_spellforge: validated.package.manifest.requires_spellforge,
                    spellforge_package: prepared.spellforge_package,
                }),
                pending_distributed_mod: Some(std::sync::Arc::clone(encoded)),
                resolved_mission_assets: Some(prepared.resolved),
                custom_mission: None,
            })
        } else {
            let Some(idx) = campaign
                .missions
                .iter()
                .position(|m| m.profile(&self.profiles).id == launch.mission_id)
            else {
                return Err(LaunchError::campaign(format!(
                    "Multiplayer menu selected unknown mission id {} ({})",
                    launch.mission_id, launch.mission_name
                )));
            };
            campaign.reset(
                &self.profiles,
                self.application_context.sim_config().difficulty,
            );
            if let Some((_, _, pcs, _)) = detect_demo_mode_with_context(&self.application_context) {
                campaign.create_gang_from_pcs(
                    pcs,
                    &self.profiles,
                    self.application_context.sim_config().difficulty,
                );
            }
            campaign.force_next_mission(idx);
            None
        };
        let (server, connect) = match launch.role {
            MultiplayerRole::Host => {
                tracing::info!(
                    mission = %launch.mission_name,
                    "Main menu Multiplayer: hosting selected mission"
                );
                (true, None)
            }
            MultiplayerRole::Client { connect_addr } => {
                tracing::info!(
                    mission = %launch.mission_name,
                    connect = %connect_addr,
                    "Main menu Multiplayer: joining selected mission"
                );
                (false, Some(connect_addr))
            }
        };
        let multiplayer = MultiplayerRoute {
            // A shell-provided join artifact has now been consumed by the
            // authenticated interactive preflight. The exact connection
            // address and prepared package are its sole mission bootstrap
            // authority.
            join: None,
            server,
            connect,
            start_at_epoch_ms: launch.start_at_epoch_ms,
            expected_players: Some(launch.expected_players),
            mission_profile_id: Some(launch.mission_id),
            continue_session: request.multiplayer.continue_session,
        };
        // A full-mod lobby replaces the launch's content; a campaign mission
        // keeps the configuration's.
        let request = match content {
            Some(content) => request.with_content(content),
            None => request,
        };
        Ok(MissionRequest {
            multiplayer,
            ..request
        })
    }

    /// Hackable levels: reset, give the sandbox its Robin-only gang, and
    /// select the level by filename.
    fn prepare_hackable_mission(
        &mut self,
        campaign: &mut Campaign,
        mission: &str,
    ) -> Result<(usize, MissionLocation), LaunchError> {
        let profiles_mut = std::sync::Arc::make_mut(&mut self.profiles);
        campaign.reset(
            profiles_mut,
            self.application_context.sim_config().difficulty,
        );
        // Hackable levels are standalone sandboxes with no preceding
        // campaign mission to inherit a gang from; start with Robin.
        campaign.create_gang_from_pcs(
            "R",
            profiles_mut,
            self.application_context.sim_config().difficulty,
        );
        let idx = campaign
            .force_next_mission_by_name(profiles_mut, mission, mission, true)
            .ok_or_else(|| {
                LaunchError::campaign(format!("failed to create hackable mission `{mission}`"))
            })?;
        campaign.current_mission_idx = Some(idx);
        let location = campaign.missions[idx].profile(profiles_mut).location;
        Ok((idx, location))
    }

    /// Installed mod pack: admit its exact assets and select its mission.
    /// `None` (after logging) when preparation fails and the menu reopens.
    fn prepare_installed_mod_launch(
        &mut self,
        campaign: &mut Campaign,
        launch: crate::main_menu::custom_missions::CustomMissionLaunch,
    ) -> Result<Option<MissionRequest>, LaunchError> {
        tracing::info!(
            "Main menu CustomMission: slug={} rhm={} map={} spellforge={}",
            launch.slug,
            launch.rhm_basename,
            launch.map_filename,
            launch.requires_spellforge
        );
        let prepared = match crate::mission_asset_launch::prepare_installed_custom_mission(
            &launch,
            self.application_context
                .preparation_files()
                .map_err(LaunchError::application)?
                .clone(),
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                tracing::error!("CustomMission: exact asset preparation failed: {error}");
                return Ok(None);
            }
        };
        let profiles_mut = std::sync::Arc::make_mut(&mut self.profiles);
        campaign.reset(
            profiles_mut,
            self.application_context.sim_config().difficulty,
        );
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
                return Ok(None);
            }
        };
        campaign.current_mission_idx = Some(idx);
        // Demo-mode init: if the active datadir is a demo, the
        // gang has to be created from the PCs declared in the
        // demo manifest, same as MainMenuChoice::Start. Custom
        // missions don't dictate roster, they piggyback on
        // whatever the datadir's campaign would have used.
        if let Some((_, _, pcs, _)) = detect_demo_mode_with_context(&self.application_context) {
            campaign.create_gang_from_pcs(
                pcs,
                &self.profiles,
                self.application_context.sim_config().difficulty,
            );
            campaign.add_all_to_mission_team();
        }
        // Hand the exact admission-produced package to mission
        // startup. Vanilla launches intentionally carry `None`; a
        // Spellforge startup is never allowed to reread local paths.
        let request = self.launch_request();
        let content = super::launch::MissionContent {
            pending_lua_mission: Some(crate::main_entry::PendingLuaMission {
                rhm_basename: launch.rhm_basename.clone(),
                requires_spellforge: launch.requires_spellforge,
                spellforge_package: prepared.spellforge_package,
            }),
            resolved_mission_assets: Some(prepared.resolved),
            ..request.content
        };
        Ok(Some(MissionRequest { content, ..request }))
    }
}

async fn run_main_menu(
    window: &mut GameWindow,
    mut campaign: Campaign,
    profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    application_context: ApplicationContext,
    config: Arc<LaunchConfig>,
) -> Result<i32, LaunchError> {
    #[cfg(target_arch = "wasm32")]
    let invite_config = Arc::clone(&config);
    // ── Full game: outer main menu loop ──
    let mut menu = MainMenuContext {
        window,
        profiles,
        application_context,
        config,
    };
    let mut reopen_main_options = false;
    #[cfg(target_arch = "wasm32")]
    let mut pending_direct_browser_invite = invite_config.cli.join.as_deref();
    #[cfg(not(target_arch = "wasm32"))]
    let mut pending_direct_browser_invite: Option<&str> = None;
    loop {
        let open_options_initially = std::mem::take(&mut reopen_main_options);
        let menu_choice = Box::pin(show_main_menu(
            menu.window,
            &campaign,
            &menu.profiles,
            &menu.application_context,
            open_options_initially,
            pending_direct_browser_invite.take(),
        ))
        .await
        // TODO(10/F11): leaf returns String (main menu screen).
        .map_err(LaunchError::menu)?;

        match menu_choice {
            MainMenuChoice::RedisplayOptions => {
                reopen_main_options = true;
                continue;
            }
            MainMenuChoice::Start => {
                // Play resumes the latest checkpoint through the same save
                // preflight as Load, including its saved mission and campaign.
                let Some(callbacks) =
                    RustCallbacks::new_for_window(menu.application_context.clone(), menu.window)
                        .await?
                else {
                    if menu.window.close_requested {
                        return Ok(0);
                    }
                    continue;
                };
                if let Some(index) = callbacks.save_manager.find_resume_target() {
                    let slot = callbacks
                        .save_manager
                        .slot_name(index)
                        .map_err(LaunchError::save)?;
                    let mission_id = callbacks
                        .save_manager
                        .slot_mission_id(index)
                        .expect("resume slot disappeared from the loaded save index");
                    drop(callbacks);
                    let request = menu.launch_request();
                    let Some(next) = menu
                        .launch_session(campaign, request, Some((slot, mission_id)))
                        .await?
                    else {
                        return Ok(0);
                    };
                    campaign = next;
                    continue;
                }
                drop(callbacks);
                // Reset campaign for a new game
                campaign.reset(
                    &menu.profiles,
                    menu.application_context.sim_config().difficulty,
                );
                tracing::info!("Campaign reset for new game");

                if let Some((idx, location)) = menu.select_demo_start_mission(&mut campaign)? {
                    let Some(mut callbacks) = RustCallbacks::new_for_window(
                        menu.application_context.clone(),
                        menu.window,
                    )
                    .await?
                    else {
                        if menu.window.close_requested {
                            return Ok(0);
                        }
                        continue;
                    };
                    let sim_config = crate::game_session::initial_sim_config(&menu.config);
                    campaign = menu
                        .launch_mission(&mut callbacks, campaign, idx, location, sim_config)
                        .await?;
                    tracing::info!("Returned to main menu");
                    continue;
                }

                // Session always returns to menu (window close causes Quit → QuitToMenu)
                let request = menu.launch_request();
                let Some(next) = menu.launch_session(campaign, request, None).await? else {
                    return Ok(0);
                };
                campaign = next;
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
                let request = menu.launch_request();
                let Some(next) = menu
                    .launch_session(campaign, request, Some((slot, mission_id)))
                    .await?
                else {
                    return Ok(0);
                };
                campaign = next;
                tracing::info!("Returned to main menu from Load");
            }
            #[cfg(feature = "multiplayer")]
            MainMenuChoice::Multiplayer(launch) => {
                let request = menu.prepare_multiplayer_launch(&mut campaign, launch)?;
                let Some(next) = menu.launch_session(campaign, request, None).await? else {
                    return Ok(0);
                };
                campaign = next;
                tracing::info!("Returned to main menu from Multiplayer");
            }
            MainMenuChoice::CustomMission(
                crate::main_menu::custom_missions::CustomMissionChoice::Hackable { mission, title },
            ) => {
                tracing::info!("Main menu CustomMission (hackable): {title} ({mission})");
                let (idx, location) = menu.prepare_hackable_mission(&mut campaign, &mission)?;
                let Some(mut callbacks) =
                    RustCallbacks::new_for_window(menu.application_context.clone(), menu.window)
                        .await?
                else {
                    if menu.window.close_requested {
                        return Ok(0);
                    }
                    continue;
                };
                let mut sim_config = crate::game_session::initial_sim_config(&menu.config);
                // Hackable descriptors carry no SCB StartUp class, so the
                // script VM must stay off.
                sim_config.script_enabled = false;
                campaign = menu
                    .launch_mission(&mut callbacks, campaign, idx, location, sim_config)
                    .await?;
                tracing::info!("Returned to main menu from hackable level `{mission}`");
            }
            MainMenuChoice::CustomMission(
                crate::main_menu::custom_missions::CustomMissionChoice::Mod(launch),
            ) => {
                let Some(request) = menu.prepare_installed_mod_launch(&mut campaign, launch)?
                else {
                    continue;
                };
                let Some(next) = menu.launch_session(campaign, request, None).await? else {
                    return Ok(0);
                };
                campaign = next;
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
    args: &LaunchConfig,
) -> Result<i32, LaunchError> {
    let owner = (*application_context).clone();
    let result = run_rust_game_headless_active(campaign, profiles, application_context, args).await;
    finish_application(result, owner.shutdown().await)
}

async fn run_rust_game_headless_active(
    mut campaign: Campaign,
    mut profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    application_context: crate::host::ReadyApplicationContext,
    args: &LaunchConfig,
) -> Result<i32, LaunchError> {
    let config = prepare_run_args(application_context, args)?;
    let application_context = config.global_options.clone();
    let request = MissionRequest::new(Arc::clone(&config));

    #[cfg(not(target_arch = "wasm32"))]
    application_context
        .start_http_transport(config.cli.http_server)
        .map_err(LaunchError::application)?;

    warm_run_assets(&config, &profiles)?;

    tracing::info!("--headless: running without winit, wgpu, renderer, or audio backend");

    let pending_replay = if wait_for_command_requested(&config) {
        tracing::info!(
            "--headless --wait-for-command: data loaded, idling until load-replay RPC arrives"
        );
        Some(
            take_commanded_replay(
                &config.global_options,
                None,
                "--headless --wait-for-command",
            )
            .await?,
        )
    } else {
        None
    };
    let (replay_data, replay_paused) = match pending_replay {
        Some(pending) => (Some(pending.data), pending.paused),
        None => (requested_replay_data(&request)?, false),
    };
    let (request, launch) = if let Some(data) = replay_data {
        let prepared = crate::game_session::prepare_replay_launch(
            &application_context,
            std::sync::Arc::make_mut(&mut profiles),
            &request,
            data,
            replay_paused,
        )
        .await?;
        campaign = prepared.campaign;
        let launch = Some((
            prepared.mission_idx,
            prepared.location,
            prepared.rng_seed,
            prepared.sim_config,
        ));
        (prepared.launch, launch)
    } else {
        let request = prepare_direct_custom_mission_args(request, &profiles, &application_context)?;
        // After admission: a mounted archive can supply a hackable level.
        let initial_sim_config = crate::game_session::initial_sim_config(&config);
        let launch = if let Some((idx, location)) = force_mission_launch(
            &mut campaign,
            &mut profiles,
            &application_context,
            &request.config,
        )? {
            Some((idx, location, 0, initial_sim_config))
        } else if let Some((mission_name, _proto_name, pcs, location)) =
            detect_demo_mode_with_context(&application_context)
        {
            campaign.reset(&profiles, application_context.sim_config().difficulty);
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
                    LaunchError::campaign(format!(
                        "demo mission `{mission_name}` is present in data but missing from campaign"
                    ))
                })?;
            campaign.current_mission_idx = Some(idx);
            Some((idx, location, 0, initial_sim_config))
        } else if config.cli.sherwood {
            campaign.reset(&profiles, application_context.sim_config().difficulty);
            campaign.force_next_mission(0);
            campaign.current_mission_idx = Some(0);
            Some((0, MissionLocation::Sherwood, 0, initial_sim_config))
        } else {
            None
        };
        (request, launch)
    };

    let Some((idx, location, rng_seed, sim_config)) = launch else {
        return Err(LaunchError::arguments(
            "--headless requires --sherwood, --replay, or a demo data dir; the main menu cannot be navigated without a display.",
        ));
    };

    #[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
    if projection_export_requested(&config) {
        crate::game_session::export_official_mission_headless(
            campaign, &profiles, idx, location, &request, rng_seed, sim_config,
        )
        .await?;
        return Ok(0);
    }

    let mut callbacks = RustCallbacks::new(application_context)?;
    let outcome = run_mission_headless(
        &mut callbacks,
        campaign,
        &profiles,
        idx,
        location,
        request,
        rng_seed,
        sim_config,
    )
    .await;
    outcome.result?;
    Ok(0)
}

/// Both launch forms (`--wait-for-command` and `ROBIN_WAIT_FOR_COMMAND`)
/// select the RPC-driven replay entry, in graphical and headless runs alike.
fn wait_for_command_requested(config: &LaunchConfig) -> bool {
    config.cli.wait_for_command || std::env::var_os("ROBIN_WAIT_FOR_COMMAND").is_some()
}

/// Block until a `load-replay` RPC queues a pending replay, then take it.
/// `mode` names the launch form in the error if the replay vanished first.
async fn take_commanded_replay(
    context: &ApplicationContext,
    window: Option<&mut GameWindow>,
    mode: &str,
) -> Result<crate::replay_service::PendingReplay, LaunchError> {
    wait_for_replay_command(context, window).await?;
    context.replay_launches().take_pending().ok_or_else(|| {
        LaunchError::replay(format!("{mode}: replay disappeared before mission start"))
    })
}

/// Block until a `load-replay` RPC call queues a pending replay.
///
/// With a window (graphical runs) this paints a dark blue canvas (so the
/// user sees *something* other than the browser's default white) and pumps
/// window events; headless runs only drain RPCs. Both poll at 20 Hz. The
/// pending replay is only peeked here; the caller consumes and prepares all
/// frame-0 metadata before constructing the mission Engine.
async fn wait_for_replay_command(
    context: &ApplicationContext,
    mut window: Option<&mut GameWindow>,
) -> Result<(), LaunchError> {
    loop {
        // Pump events — winit needs the app to drain its queue every
        // frame to stay responsive.
        if let Some(window) = window.as_deref_mut() {
            let _ = window.poll_events();
        }

        // Drain RPCs — the `load-replay` endpoint is how this loop
        // exits, and the normal `drain_global` path needs an engine.
        // `drain_pre_engine` handles `load-replay` / `info` and
        // rejects everything else with an "engine not ready" reply.
        context
            .drain_http_pre_engine()
            .map_err(LaunchError::application)?;

        if let Some(window) = window.as_deref_mut() {
            window.clear_to_color(wgpu::Color {
                r: 0.01,
                g: 0.02,
                b: 0.08,
                a: 1.0,
            });
        }

        if context.replay_launches().pending_mission().is_some() {
            return Ok(());
        }

        crate::window::sleep_ms(50).await;
    }
}

#[cfg(test)]
mod early_replay_tests {
    #[derive(Default, serde::Serialize, serde::Deserialize)]
    struct StartupCalls(Vec<String>);

    impl<'ast> syn::visit::Visit<'ast> for StartupCalls {
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            if let syn::Expr::Path(path) = call.func.as_ref() {
                self.0
                    .push(path.path.segments.last().unwrap().ident.to_string());
            }
            syn::visit::visit_expr_call(self, call);
        }

        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            self.0.push(call.method.to_string());
            syn::visit::visit_expr_method_call(self, call);
        }
    }

    #[test]
    fn both_entry_paths_resolve_then_start_transport_then_warm_assets_once() {
        use syn::visit::Visit;
        let source = syn::parse_file(include_str!("run.rs")).unwrap();
        for entry in ["run_rust_game_active", "run_rust_game_headless_active"] {
            let function = source
                .items
                .iter()
                .find_map(|item| match item {
                    syn::Item::Fn(function) if function.sig.ident == entry => Some(function),
                    _ => None,
                })
                .unwrap();
            let mut calls = StartupCalls::default();
            calls.visit_block(&function.block);
            let phases: Vec<_> = calls
                .0
                .iter()
                .filter(|name| {
                    matches!(
                        name.as_str(),
                        "prepare_run_args" | "start_http_transport" | "warm_run_assets"
                    )
                })
                .map(String::as_str)
                .collect();
            assert_eq!(
                phases,
                [
                    "prepare_run_args",
                    "start_http_transport",
                    "warm_run_assets"
                ],
                "{entry}"
            );
            let warmed = calls
                .0
                .iter()
                .position(|name| name == "warm_run_assets")
                .unwrap();
            let replay = calls
                .0
                .iter()
                .position(|name| name == "prepare_replay_launch")
                .unwrap();
            assert!(
                warmed < replay,
                "{entry}: replay preparation must follow startup"
            );
        }
    }

    #[test]
    fn shutdown_failure_does_not_hide_the_original_application_failure() {
        use super::LaunchError;
        let finish = |result: Result<i32, &str>, shutdown: Result<(), &str>| {
            super::finish_application(
                result.map_err(|run| LaunchError::arguments(run.to_owned())),
                shutdown.map_err(str::to_owned),
            )
            .map_err(|error| error.to_string())
        };
        assert_eq!(finish(Ok(7), Ok(())), Ok(7));
        assert_eq!(finish(Err("run"), Ok(())), Err("run".into()));
        assert_eq!(
            finish(Ok(7), Err("drain")),
            Err("application shutdown: drain".into())
        );
        assert_eq!(
            finish(Err("run"), Err("drain")),
            Err("run; application shutdown: drain".into())
        );
    }

    #[test]
    fn early_mode_is_default_and_late_ablation_rejects_typos() {
        assert_eq!(super::replay_preparation_mode(None).ok(), Some(true));
        assert_eq!(
            super::replay_preparation_mode(Some("late")).ok(),
            Some(false)
        );
        assert_eq!(
            super::replay_preparation_mode(Some("early")).ok(),
            Some(true)
        );
        assert!(super::replay_preparation_mode(Some("ealry")).is_err());
    }
}
