//! `setup_multiplayer_session` with the `multiplayer` feature: host an iroh
//! session (native only) or connect to one and install it on the host.

#[cfg(not(target_arch = "wasm32"))]
use super::resolve_publication_preference;
use super::{validate_multiplayer_launch_args, validate_preflighted_content};
use crate::host::Host;

#[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
pub(super) async fn establish(
    host: &mut Host,
    args: &crate::main_entry::MissionLaunch,
    authoritative_mission_id: &str,
    authoritative_rng_seed: u64,
    authoritative_sim_config: robin_engine::engine::SimConfig,
    campaign: &crate::multiplayer::MultiplayerCampaignSession,
) -> Result<(), String> {
    use crate::multiplayer::NetChannels;
    #[cfg(not(target_arch = "wasm32"))]
    use crate::multiplayer::NetEvent;
    #[cfg(not(target_arch = "wasm32"))]
    use crate::multiplayer::{HostedModContent, start_server_in_campaign};
    #[cfg(not(target_arch = "wasm32"))]
    use std::time::{Duration, Instant};

    validate_multiplayer_launch_args(args)?;

    let nickname = if args.mp_nickname.is_empty() {
        std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "player".to_string())
    } else {
        args.mp_nickname.clone()
    };

    if args.server {
        #[cfg(target_arch = "wasm32")]
        return Err(
            "multiplayer: browser builds cannot host; connect to a native host".to_string(),
        );

        #[cfg(not(target_arch = "wasm32"))]
        {
            if !args.mp_continue_session {
                campaign.discard_host_continuation()?;
            }
            let publish_browser_links = resolve_browser_join_publication(args)?;
            let speech_timing_locale = host
                .application_context()
                .canonical_speech_timing_locale()
                .map_err(|error| {
                    format!("multiplayer: cannot select authoritative speech timing: {error}")
                })?;
            let (mut channels, server_channels) = NetChannels::new_server();
            let content = args
                .pending_distributed_mod
                .as_ref()
                .map(|encoded| {
                    HostedModContent::from_encoded(encoded.to_vec()).map_err(|error| {
                        format!("multiplayer: invalid hosted full-mod package: {error}")
                    })
                })
                .transpose()?;
            let started = start_server_in_campaign(
                campaign,
                crate::multiplayer::ServerConfig {
                    host_nickname: nickname.clone(),
                    mission_id: authoritative_mission_id.to_string(),
                    mission_seed: authoritative_rng_seed,
                    sim_config: authoritative_sim_config,
                    speech_timing_locale: speech_timing_locale.clone(),
                    expected_players: args.mp_expected_players.unwrap_or(1),
                    browser_join_enabled: publish_browser_links,
                },
                server_channels,
                content,
            );
            match started {
                Ok(handle) => {
                    channels
                        .install_session_id(handle.session_id())
                        .map_err(|error| format!("multiplayer: {error}"))?;
                    if publish_browser_links {
                        let content_edition = if crate::main_entry::detect_demo_mode_with_context(
                            &args.global_options,
                        )
                        .is_some()
                        {
                            crate::multiplayer::join_ticket::BrowserContentEdition::Demo
                        } else {
                            crate::multiplayer::join_ticket::BrowserContentEdition::Full
                        };
                        let content_identity_sha256 =
                            crate::multiplayer::content_identity::active_content_identity(args.global_options.preparation_files()?)
                                .map_err(|error| {
                                    format!(
                                        "multiplayer: cannot publish an exact browser content invitation: {error}"
                                    )
                                })?;
                        let ticket = handle
                            .browser_join_ticket(
                                content_edition,
                                content_identity_sha256.clone(),
                                args.mp_mission_profile_id,
                                args.mp_expected_players.unwrap_or(1),
                            )
                            .map_err(|error| {
                                format!("multiplayer: browser invitation unavailable: {error}")
                            })?;
                        let browser_base =
                            std::env::var("ROBINHOOD_BROWSER_URL").unwrap_or_else(|_| {
                                crate::multiplayer::join_ticket::DEFAULT_BROWSER_URL.to_string()
                            });
                        let share_url = ticket.share_url(&browser_base).map_err(|error| {
                            format!("multiplayer: browser share URL unavailable: {error}")
                        })?;
                        tracing::info!(
                            browser_join_code = %ticket.encode(),
                            %share_url,
                            relay = %ticket.payload().relay_url,
                            ?content_edition,
                            %content_identity_sha256,
                            "browser multiplayer invitation (relay can observe participant IPs, connection times, and byte counts; game traffic remains end-to-end encrypted)"
                        );
                        host.frontend
                            .diagnostics_mut()
                            .queue_console_output(format!(
                                "Browser join code (expires after 30 minutes if unused): {}",
                                ticket.encode()
                            ));
                        host.frontend
                            .diagnostics_mut()
                            .queue_console_output(format!("Browser join link: {share_url}"));
                        host.frontend.diagnostics_mut().queue_console_output(format!(
                            "Privacy: relay {} can observe IPs, timing, and byte counts; gameplay is end-to-end encrypted.",
                            ticket.payload().relay_url
                        ));
                    }
                    tracing::info!(
                        endpoint_id = %handle.endpoint_id(),
                        nickname = %nickname,
                        seed = authoritative_rng_seed,
                        "multiplayer: hosting on iroh endpoint {}",
                        handle.endpoint_id()
                    );
                    let seat = handle.local_seat;
                    channels.attach_runtime(handle);
                    host.transport.install_session(
                        channels,
                        seat,
                        authoritative_mission_id.to_string(),
                        authoritative_rng_seed,
                        authoritative_sim_config,
                        speech_timing_locale,
                    );
                }
                Err(e) => {
                    return Err(format!("multiplayer: failed to start server: {e}"));
                }
            }
        }
    } else if let Some(addr) = args.connect.as_deref() {
        let (mut channels, in_tx, out_rx, _client_frame_cursor, _client_snapshot) =
            NetChannels::new();
        let connection = crate::multiplayer::connect_client_in_campaign(
            campaign,
            addr,
            nickname.clone(),
            in_tx,
            out_rx,
        );
        match connection {
            Ok(handle) => {
                #[cfg(not(target_arch = "wasm32"))]
                let offered_content = handle.content_offer();
                #[cfg(target_arch = "wasm32")]
                let offered_content = {
                    let deadline = web_time::Instant::now() + std::time::Duration::from_secs(15);
                    while handle.content_offer().is_none()
                        && handle.session_metadata().is_none()
                        && web_time::Instant::now() < deadline
                    {
                        crate::window::sleep_ms(10).await;
                    }
                    if handle.content_offer().is_none() && handle.session_metadata().is_none() {
                        return Err(
                            "multiplayer: timed out awaiting browser Welcome/content offer"
                                .to_owned(),
                        );
                    }
                    handle.content_offer()
                };
                if let Err(error) = validate_preflighted_content(
                    args.pending_distributed_mod.as_deref(),
                    offered_content.as_ref(),
                ) {
                    if let Some(offer) = offered_content.as_ref() {
                        channels.reject_content(
                            offer.full_mod_sha256,
                            "host content differs from interactive preflight".to_owned(),
                        );
                    }
                    return Err(error);
                }
                if let Some(offer) = offered_content {
                    let admitted = crate::distributed_mod_admission::admit_trusted_distributed_mod(
                        host.application_context(),
                        &channels,
                        &offer,
                        crate::distributed_mod_admission::DistributedModAdmissionPurpose::JoinSession,
                    )
                    .await
                    .map_err(|error| {
                        format!("multiplayer: host-content admission failed: {error}")
                    })?;
                    host.transport.retain_distributed_mod(admitted);
                    let deadline = web_time::Instant::now() + std::time::Duration::from_secs(15);
                    while handle.session_metadata().is_none() && web_time::Instant::now() < deadline
                    {
                        crate::window::sleep_ms(10).await;
                    }
                    if handle.session_metadata().is_none() {
                        return Err(
                            "multiplayer: timed out awaiting Welcome after verified host-content admission"
                                .to_owned(),
                        );
                    }
                    if offer.mission_basename != authoritative_mission_id {
                        return Err(format!(
                            "multiplayer: offered mod mission `{}` does not match requested mission `{authoritative_mission_id}`",
                            offer.mission_basename
                        ));
                    }
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let deadline = web_time::Instant::now() + std::time::Duration::from_secs(10);
                    while handle.session_metadata().is_none() && web_time::Instant::now() < deadline
                    {
                        if let Some(error) = handle.startup_error() {
                            return Err(format!(
                                "multiplayer: browser relay startup failed: {error}"
                            ));
                        }
                        crate::window::sleep_ms(10).await;
                    }
                    if let Some(error) = handle.startup_error() {
                        return Err(format!(
                            "multiplayer: browser relay startup failed: {error}"
                        ));
                    }
                    if handle.session_metadata().is_none() {
                        return Err(
                            "multiplayer: timed out awaiting authoritative Welcome before Engine construction"
                                .to_string(),
                        );
                    }
                }
                let session = handle.session_metadata().ok_or_else(|| {
                    "multiplayer: authoritative Welcome is not available".to_string()
                })?;
                channels
                    .install_session_id(session.session_id)
                    .map_err(|error| format!("multiplayer: {error}"))?;
                let welcomed_mission = session.mission_id;
                if welcomed_mission != authoritative_mission_id {
                    return Err(format!(
                        "multiplayer: host mission `{welcomed_mission}` does not match requested mission `{authoritative_mission_id}`"
                    ));
                }
                let speech_timing_locale = session.speech_timing_locale;
                if let Some(authoritative_locale) = speech_timing_locale.as_deref() {
                    let has_timing_pack = host
                        .application_context()
                        .installed_languages()
                        .map_err(|error| {
                            format!("multiplayer: cannot inspect installed voice packs: {error}")
                        })?
                        .into_iter()
                        .any(|pack| pack.locale == authoritative_locale && pack.has_voice);
                    if !has_timing_pack {
                        return Err(format!(
                            "multiplayer: host requires voice pack `{authoritative_locale}` for deterministic speech timing, but that validated pack is not installed"
                        ));
                    }
                }
                tracing::info!(
                    server = %addr,
                    nickname = %nickname,
                    "multiplayer: connected to {addr}"
                );
                // Wait briefly for the AssignedLocalSeat event so
                // host.transport.local_seat() is correct before the mission
                // starts emitting outgoing inputs.  Long timeouts
                // get logged but don't abort — inputs queued before
                // the assignment lands just sit in the channel until
                // the I/O thread drains them.  Skipped on wasm —
                // blocking on a channel would freeze the browser
                // event loop, so we let the per-frame
                // `drain_net_inputs` pick up the AssignedLocalSeat
                // event when it arrives.
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while Instant::now() < deadline {
                        match channels.incoming.recv_timeout(Duration::from_millis(100)) {
                            Ok(NetEvent::AssignedLocalSeat(seat)) => {
                                assert_eq!(
                                    seat, session.seat,
                                    "assigned seat differs from admitted Welcome"
                                );
                                tracing::info!(?seat, "multiplayer: assigned seat");
                                break;
                            }
                            Ok(NetEvent::Note(s)) => tracing::info!(note = %s, "mp note"),
                            Ok(event) => channels.defer_events(vec![event]),
                            Err(_) => continue,
                        }
                    }
                }
                channels.attach_runtime(handle);
                host.transport.install_session(
                    channels,
                    session.seat,
                    welcomed_mission.to_string(),
                    session.mission_seed,
                    session.sim_config,
                    speech_timing_locale,
                );
            }
            Err(e) => {
                return Err(format!("multiplayer: failed to connect to {addr}: {e}"));
            }
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_browser_join_publication(
    args: &crate::main_entry::MissionLaunch,
) -> Result<bool, String> {
    let saved = args
        .global_options
        .with_active_profile(|profile| profile.multiplayer_config.publish_browser_join_links)
        .map_err(|error| {
            format!("multiplayer: cannot read browser publication preference: {error}")
        })?;
    Ok(resolve_publication_preference(
        args.mp_browser_join_links,
        saved,
    ))
}
