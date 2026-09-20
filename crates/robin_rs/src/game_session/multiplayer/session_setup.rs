//! `setup_multiplayer_session` with the `multiplayer` feature: host an iroh
//! session (native only) or connect to one and install it on the host.

#[cfg(not(target_arch = "wasm32"))]
use super::resolve_publication_preference;
use super::{SessionSetupFailure, validate_multiplayer_launch_args, validate_preflighted_content};
use crate::host::Host;

#[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
pub(super) async fn establish(
    host: &mut Host,
    args: &crate::main_entry::MissionRequest,
    authoritative_mission_id: &str,
    authoritative_rng_seed: u64,
    authoritative_sim_config: robin_engine::engine::SimConfig,
    campaign: &crate::multiplayer::MultiplayerCampaignSession,
) -> Result<(), SessionSetupFailure> {
    use crate::multiplayer::NetChannels;
    #[cfg(not(target_arch = "wasm32"))]
    use crate::multiplayer::NetEvent;
    #[cfg(not(target_arch = "wasm32"))]
    use std::time::{Duration, Instant};

    validate_multiplayer_launch_args(args)?;

    let nickname = if args.config.cli.mp_nickname.is_empty() {
        std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "player".to_string())
    } else {
        args.config.cli.mp_nickname.clone()
    };

    if args.multiplayer.server {
        #[cfg(target_arch = "wasm32")]
        return Err(SessionSetupFailure::Unavailable(
            "multiplayer: browser builds cannot host; connect to a native host",
        ));

        #[cfg(not(target_arch = "wasm32"))]
        host_session(
            host,
            args,
            authoritative_mission_id,
            authoritative_rng_seed,
            authoritative_sim_config,
            campaign,
            &nickname,
        )?;
    } else if let Some(addr) = args.multiplayer.connect.as_deref() {
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
                        return Err(SessionSetupFailure::WelcomeTimeout(
                            "timed out awaiting browser Welcome/content offer",
                        ));
                    }
                    handle.content_offer()
                };
                if let Err(error) = validate_preflighted_content(
                    args.content.pending_distributed_mod.as_deref(),
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
                    .map_err(|detail| SessionSetupFailure::Local {
                        context: "host-content admission failed",
                        detail,
                    })?;
                    host.transport.retain_distributed_mod(admitted);
                    let deadline = web_time::Instant::now() + std::time::Duration::from_secs(15);
                    while handle.session_metadata().is_none() && web_time::Instant::now() < deadline
                    {
                        crate::window::sleep_ms(10).await;
                    }
                    if handle.session_metadata().is_none() {
                        return Err(SessionSetupFailure::WelcomeTimeout(
                            "timed out awaiting Welcome after verified host-content admission",
                        ));
                    }
                    if offer.mission_basename != authoritative_mission_id {
                        return Err(SessionSetupFailure::Content(format!(
                            "offered mod mission `{}` does not match requested mission `{authoritative_mission_id}`",
                            offer.mission_basename
                        ).into()));
                    }
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let deadline = web_time::Instant::now() + std::time::Duration::from_secs(10);
                    while handle.session_metadata().is_none() && web_time::Instant::now() < deadline
                    {
                        if let Some(source) = handle.startup_error() {
                            return Err(SessionSetupFailure::Transport {
                                context: "browser relay startup failed",
                                source,
                            });
                        }
                        crate::window::sleep_ms(10).await;
                    }
                    if let Some(source) = handle.startup_error() {
                        return Err(SessionSetupFailure::Transport {
                            context: "browser relay startup failed",
                            source,
                        });
                    }
                    if handle.session_metadata().is_none() {
                        return Err(SessionSetupFailure::WelcomeTimeout(
                            "timed out awaiting authoritative Welcome before Engine construction",
                        ));
                    }
                }
                let session = handle
                    .session_metadata()
                    .ok_or(SessionSetupFailure::WelcomeUnavailable)?;
                channels
                    .install_session_id(session.session_id)
                    .map_err(SessionSetupFailure::SessionIdentity)?;
                let welcomed_mission = session.mission_id;
                if welcomed_mission != authoritative_mission_id {
                    return Err(SessionSetupFailure::MissionMismatch {
                        host: welcomed_mission,
                        requested: authoritative_mission_id.to_owned(),
                    });
                }
                let speech_timing_locale = session.speech_timing_locale;
                if let Some(authoritative_locale) = speech_timing_locale.as_deref() {
                    let has_timing_pack = host
                        .application_context()
                        .installed_languages()
                        .map_err(|detail| SessionSetupFailure::Local {
                            context: "cannot inspect installed voice packs",
                            detail,
                        })?
                        .into_iter()
                        .any(|pack| pack.locale == authoritative_locale && pack.has_voice);
                    if !has_timing_pack {
                        return Err(SessionSetupFailure::MissingVoicePack(
                            authoritative_locale.to_owned(),
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
                return Err(SessionSetupFailure::Io {
                    context: format!("failed to connect to {addr}").into(),
                    source: e,
                });
            }
        }
    }
    Ok(())
}

/// Host branch of [`establish`]: start the authoritative server, publish the
/// browser invitation when enabled, and install the session on `host`.
#[cfg(not(target_arch = "wasm32"))]
fn host_session(
    host: &mut Host,
    args: &crate::main_entry::MissionRequest,
    authoritative_mission_id: &str,
    authoritative_rng_seed: u64,
    authoritative_sim_config: robin_engine::engine::SimConfig,
    campaign: &crate::multiplayer::MultiplayerCampaignSession,
    nickname: &str,
) -> Result<(), SessionSetupFailure> {
    use crate::multiplayer::NetChannels;
    use crate::multiplayer::{HostedModContent, start_server_in_campaign};

    if !args.multiplayer.continue_session {
        campaign.discard_host_continuation()?;
    }
    let publish_browser_links = resolve_browser_join_publication(args)?;
    // Browser content compatibility must not prevent native peers from hosting.
    // Resolve it before server startup so unsupported content also skips the
    // browser-only relay readiness requirement.
    let browser_content_identity = if publish_browser_links {
        let files = args
            .config
            .global_options
            .preparation_files()
            .map_err(SessionSetupFailure::Preparation)?;
        let identity = browser_invitation_content_identity(files);
        if identity.is_none() {
            host.frontend.diagnostics_mut().queue_console_output(
                "Browser invitations unavailable for the active content; continuing with native multiplayer only. See the log for details.".to_owned(),
            );
        }
        identity
    } else {
        None
    };
    let speech_timing_locale = host
        .application_context()
        .canonical_speech_timing_locale()
        .map_err(|detail| SessionSetupFailure::Local {
            context: "cannot select authoritative speech timing",
            detail,
        })?;
    let (mut channels, server_channels) = NetChannels::new_server();
    let content = args
        .content
        .pending_distributed_mod
        .as_ref()
        .map(|encoded| {
            HostedModContent::from_encoded(encoded.to_vec()).map_err(|source| {
                SessionSetupFailure::Transport {
                    context: "invalid hosted full-mod package",
                    source,
                }
            })
        })
        .transpose()?;
    let started = start_server_in_campaign(
        campaign,
        crate::multiplayer::ServerConfig {
            host_nickname: nickname.to_owned(),
            mission_id: authoritative_mission_id.to_string(),
            mission_seed: authoritative_rng_seed,
            sim_config: authoritative_sim_config,
            speech_timing_locale: speech_timing_locale.clone(),
            expected_players: args.multiplayer.expected_players.unwrap_or(1),
            browser_join_enabled: browser_content_identity.is_some(),
        },
        server_channels,
        content,
    );
    match started {
        Ok(handle) => {
            channels
                .install_session_id(handle.session_id())
                .map_err(SessionSetupFailure::SessionIdentity)?;
            if let Some(content_identity_sha256) = browser_content_identity {
                let content_edition = if crate::main_entry::detect_demo_mode_with_context(
                    &args.config.global_options,
                )
                .is_some()
                {
                    crate::multiplayer::join_ticket::BrowserContentEdition::Demo
                } else {
                    crate::multiplayer::join_ticket::BrowserContentEdition::Full
                };
                let ticket = handle
                    .browser_join_ticket(
                        content_edition,
                        content_identity_sha256.clone(),
                        args.multiplayer.mission_profile_id,
                        args.multiplayer.expected_players.unwrap_or(1),
                    )
                    .map_err(|source| SessionSetupFailure::Transport {
                        context: "browser invitation unavailable",
                        source,
                    })?;
                let browser_base = std::env::var("ROBINHOOD_BROWSER_URL").unwrap_or_else(|_| {
                    crate::multiplayer::join_ticket::DEFAULT_BROWSER_URL.to_string()
                });
                let share_url = ticket.share_url(&browser_base).map_err(|source| {
                    SessionSetupFailure::Transport {
                        context: "browser share URL unavailable",
                        source,
                    }
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
            return Err(SessionSetupFailure::Io {
                context: "failed to start server".into(),
                source: e,
            });
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_browser_join_publication(
    args: &crate::main_entry::MissionRequest,
) -> Result<bool, SessionSetupFailure> {
    let saved = args
        .config
        .global_options
        .with_active_profile(|profile| profile.multiplayer_config.publish_browser_join_links)
        .map_err(|detail| SessionSetupFailure::Local {
            context: "cannot read browser publication preference",
            detail,
        })?;
    Ok(resolve_publication_preference(
        args.config.cli.mp_browser_join_links,
        saved,
    ))
}

/// An unavailable browser identity disables invitations, not native hosting.
#[cfg(not(target_arch = "wasm32"))]
fn browser_invitation_content_identity(
    files: &robin_engine::sbfile::SbFileSystem,
) -> Option<String> {
    match crate::multiplayer::content_identity::active_content_identity(files) {
        Ok(identity) => Some(identity),
        Err(error) => {
            tracing::warn!(
                %error,
                "multiplayer: cannot publish an exact browser content invitation; continuing with native multiplayer only"
            );
            None
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #[test]
    fn compatible_content_preserves_exact_browser_identity() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("Data");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(data.join("robinhood.bks"), b"base content").unwrap();
        let files = robin_engine::sbfile::SbFileSystem::new(std::sync::Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        ));
        files
            .set_primary_path(root.path().to_str().unwrap())
            .unwrap();
        let expected =
            crate::multiplayer::content_identity::source_content_identity(&data).unwrap();
        assert_eq!(
            super::browser_invitation_content_identity(&files),
            Some(expected)
        );
    }

    #[test]
    fn unsupported_overlay_disables_browser_invitations_without_aborting_native_hosting() {
        let overlay = tempfile::tempdir().unwrap();
        let files = robin_engine::sbfile::SbFileSystem::new(std::sync::Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        ));
        files
            .add_overlay_path(overlay.path().to_str().unwrap())
            .unwrap();
        assert!(super::browser_invitation_content_identity(&files).is_none());
    }
}
