//! Pre-frame authority discovery and signing. Never reconstruct authority from a debrief.

use super::*;

/// Server-published immutable authorities selected before the prepared engine
/// capability is consumed. The setup layer still has to compare the exact
/// eight local projection documents and admit this authority through
/// `RankedPreparedMissionInputs` before it may construct a ranked engine.
pub(in crate::game_session) struct RankedMissionAuthority {
    pub(in crate::game_session) content_manifest: ContentManifestV1,
    pub(in crate::game_session) rules_config: RulesConfigIdentityV1,
    pub(in crate::game_session) published_ruleset: PublishedRulesetV1,
    pub(in crate::game_session) build_manifest_sha256: Digest32,
    pub(in crate::game_session) scope_request: ScopeRequestV1,
    pub(in crate::game_session) requested_metrics: Vec<BoardMetricV1>,
    pub(in crate::game_session) campaign_content_manifest_sha256: Option<Digest32>,
    pub(in crate::game_session) campaign_controller_public_key:
        Option<robin_run_protocol::PublicKey32>,
    /// Published campaign roster continuity is part of the immutable ranked
    /// ruleset. Multiplayer receipt discovery must use this retained policy;
    /// it may never infer continuity from the predecessor receipt or a host
    /// transport proposal.
    pub(in crate::game_session) campaign_roster_continuity:
        robin_run_protocol::CampaignRosterContinuityV1,
    /// Exact pre-frame grant authority pinned by the selected immutable
    /// ruleset. This must survive prepared-input admission; rediscovering a
    /// key from later metadata would open a second authority-selection lane.
    pub(in crate::game_session) run_preflight_grant_public_key: robin_run_protocol::PublicKey32,
    /// Locally retained server receipt for a campaign continuation. It is
    /// handed to the closed controller/transport preflight flow and is never
    /// reconstructed from campaign totals or a public board response.
    pub(in crate::game_session) local_campaign_chain_receipt:
        Option<robin_run_protocol::CampaignChainReceiptV1>,
}

pub(in crate::game_session) enum RankedPreFramePlan {
    BrowseOnly { reason: String },
    Authority(RankedMissionAuthority),
}

pub(in crate::game_session) enum PreparedRankedAdmission {
    BrowseOnly {
        reason: String,
    },
    PendingSignature {
        config: robin_run_protocol::RankedSessionConfigV1,
        scope_request: ScopeRequestV1,
        requested_metrics: Vec<BoardMetricV1>,
        campaign_controller_public_key: Option<robin_run_protocol::PublicKey32>,
        campaign_roster_continuity: robin_run_protocol::CampaignRosterContinuityV1,
        run_preflight_grant_public_key: robin_run_protocol::PublicKey32,
        local_campaign_chain_receipt: Option<robin_run_protocol::CampaignChainReceiptV1>,
    },
    Signed {
        lifecycle: crate::leaderboard_ranked_session::SharedRankedSessionLifecycle,
        scope_request: ScopeRequestV1,
        requested_metrics: Vec<BoardMetricV1>,
        campaign_controller_public_key: Option<robin_run_protocol::PublicKey32>,
    },
}

impl PreparedRankedAdmission {
    /// Sign the exact prepared authority after all package/script setup has
    /// settled but before the first simulation/replay frame can be emitted.
    pub(in crate::game_session) async fn sign_before_frame_zero(
        &mut self,
        custom_package_present: bool,
    ) {
        let pending = std::mem::replace(
            self,
            Self::BrowseOnly {
                reason: "ranked session signing was interrupted".to_owned(),
            },
        );
        let Self::PendingSignature {
            config,
            scope_request: requested_scope,
            requested_metrics,
            campaign_controller_public_key: requested_campaign_controller,
            campaign_roster_continuity: _,
            run_preflight_grant_public_key,
            local_campaign_chain_receipt,
        } = pending
        else {
            *self = pending;
            return;
        };
        if custom_package_present {
            *self = Self::BrowseOnly {
                reason: "custom package content is outside the official ranked policy".to_owned(),
            };
            return;
        }
        let run_preflight = match acquire_single_player_run_preflight(
            &config,
            &requested_scope,
            requested_campaign_controller,
            local_campaign_chain_receipt.as_ref(),
        )
        .await
        {
            Ok(preflight) => preflight,
            Err(reason) => {
                *self = Self::BrowseOnly {
                    reason: format!("ranked run preflight failed before frame zero: {reason}"),
                };
                return;
            }
        };
        let trusted_now_unix_ms = match crate::leaderboard_receipt_watcher::now_unix_ms() {
            Ok(now) => now,
            Err(error) => {
                *self = Self::BrowseOnly {
                    reason: format!(
                        "ranked run preflight has no trustworthy current-time boundary: {error}"
                    ),
                };
                return;
            }
        };
        let setup = OfficialRankedSessionSetupV1 {
            ranked_session: config,
            custom_package_present: false,
            run_preflight,
            run_preflight_grant_public_key,
            trusted_now_unix_ms,
        };
        let scope_request = setup.scope_request();
        let campaign_controller_public_key = setup.campaign_controller_public_key();
        let session = create_official_ranked_session(setup).await;
        *self = match session {
            Ok(session) => Self::Signed {
                lifecycle: Arc::new(Mutex::new(
                    crate::leaderboard_ranked_session::RankedSessionLifecycle::ranked(session),
                )),
                scope_request,
                requested_metrics,
                campaign_controller_public_key,
            },
            Err(reason) => Self::BrowseOnly {
                reason: format!("ranked genesis was not signed before frame zero: {reason}"),
            },
        };
    }

    /// Resolve the authenticated multiplayer authority exchange before any
    /// simulation/replay frame can be emitted. The host obtains one
    /// server-signed fresh/continuation grant; every peer independently
    /// reconstructs the proposed immutable ruleset and installs the exact
    /// setup using its own trusted clock.
    pub(in crate::game_session) async fn install_multiplayer_before_frame_zero(
        &mut self,
        net: &crate::multiplayer::NetChannels,
        custom_package_present: bool,
    ) {
        let pending = std::mem::replace(
            self,
            Self::BrowseOnly {
                reason: "ranked multiplayer setup was interrupted".to_owned(),
            },
        );
        let Self::PendingSignature {
            config,
            scope_request,
            requested_metrics,
            campaign_controller_public_key: _,
            campaign_roster_continuity,
            run_preflight_grant_public_key,
            local_campaign_chain_receipt: _,
        } = pending
        else {
            match pending {
                Self::BrowseOnly { reason } => {
                    downgrade_multiplayer_ranked(net, &reason);
                    *self = Self::BrowseOnly { reason };
                }
                signed @ Self::Signed { .. } => *self = signed,
                Self::PendingSignature { .. } => unreachable!(),
            }
            return;
        };
        if custom_package_present {
            let reason = "custom package content is outside the official ranked policy".to_owned();
            downgrade_multiplayer_ranked(net, &reason);
            *self = Self::BrowseOnly { reason };
            return;
        }
        let port = match net.ranked_port() {
            Ok(port) => port,
            Err(error) => {
                let reason = format!("authenticated multiplayer ranked port unavailable: {error}");
                downgrade_multiplayer_ranked(net, &reason);
                *self = Self::BrowseOnly { reason };
                return;
            }
        };
        let result = match port.role() {
            crate::multiplayer::RankedMultiplayerRole::Host => {
                install_ranked_multiplayer_host(
                    net,
                    config,
                    scope_request,
                    requested_metrics,
                    campaign_roster_continuity,
                    run_preflight_grant_public_key,
                )
                .await
            }
            crate::multiplayer::RankedMultiplayerRole::Client => {
                install_ranked_multiplayer_client(net, config).await
            }
        };
        *self = match result {
            Ok(resolved) => Self::Signed {
                lifecycle: port.lifecycle(),
                scope_request: resolved.scope_request,
                requested_metrics: resolved.requested_metrics,
                campaign_controller_public_key: resolved.campaign_controller_public_key,
            },
            Err(error) => {
                let reason =
                    format!("multiplayer ranked preflight failed before frame zero: {error}");
                downgrade_multiplayer_ranked(net, &reason);
                Self::BrowseOnly { reason }
            }
        };
    }

    pub(in crate::game_session) fn take_mission_admission(&mut self) -> RankedMissionAdmission {
        match std::mem::replace(
            self,
            Self::BrowseOnly {
                reason: "ranked admission was consumed".to_owned(),
            },
        ) {
            Self::BrowseOnly { reason } => RankedMissionAdmission::browse_only(reason),
            Self::PendingSignature { .. } => RankedMissionAdmission::browse_only(
                "ranked genesis signing was not completed before frame zero",
            ),
            Self::Signed {
                lifecycle,
                scope_request,
                requested_metrics,
                campaign_controller_public_key,
            } => RankedMissionAdmission::Signed(SignedRankedMissionAdmission {
                lifecycle,
                scope_request,
                requested_metrics,
                campaign_controller_public_key,
            }),
        }
    }
}

/// Same human/browser setup window used by the transport. It includes joining
/// from a shared link, isolated browser identity setup, service preflight, and
/// the all-peer campaign selection exchange.
pub(super) const RANKED_PREFLIGHT_PHASE_TIMEOUT_MS: u128 = 120_000;
/// A client cannot observe the host's full-lobby transition for a fresh run,
/// so its first setup wait composes lobby formation plus one service phase.
pub(super) const RANKED_PREFLIGHT_INITIAL_CLIENT_TIMEOUT_MS: u128 =
    2 * RANKED_PREFLIGHT_PHASE_TIMEOUT_MS;
/// Four bounded phases: authenticated lobby formation, receipt selection,
/// controller authorization, and service-grant/setup publication. Per-phase
/// progress can extend an early peer's wait, but cannot keep it alive forever.
pub(super) const RANKED_PREFLIGHT_TOTAL_TIMEOUT_MS: u128 = 4 * RANKED_PREFLIGHT_PHASE_TIMEOUT_MS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RankedPreflightTimeout {
    PhaseInactivity,
    Total,
}

pub(super) fn ranked_preflight_timeout(
    total_elapsed_ms: u128,
    phase_elapsed_ms: u128,
    phase_timeout_ms: u128,
) -> Option<RankedPreflightTimeout> {
    if total_elapsed_ms >= RANKED_PREFLIGHT_TOTAL_TIMEOUT_MS {
        Some(RankedPreflightTimeout::Total)
    } else if phase_elapsed_ms >= phase_timeout_ms {
        Some(RankedPreflightTimeout::PhaseInactivity)
    } else {
        None
    }
}

pub(super) fn ensure_ranked_preflight_deadline(
    total_started: web_time::Instant,
    phase_started: web_time::Instant,
    phase_timeout_ms: u128,
    phase: &str,
) -> Result<(), String> {
    match ranked_preflight_timeout(
        total_started.elapsed().as_millis(),
        phase_started.elapsed().as_millis(),
        phase_timeout_ms,
    ) {
        Some(RankedPreflightTimeout::Total) => Err(format!(
            "ranked multiplayer setup exceeded its total bounded window during {phase}"
        )),
        Some(RankedPreflightTimeout::PhaseInactivity) => Err(format!(
            "ranked multiplayer setup made no authenticated progress during {phase}"
        )),
        None => Ok(()),
    }
}

struct ResolvedMultiplayerAdmission {
    scope_request: ScopeRequestV1,
    requested_metrics: Vec<BoardMetricV1>,
    campaign_controller_public_key: Option<robin_run_protocol::PublicKey32>,
}

struct AuthorizedMultiplayerProposal {
    published_ruleset: PublishedRulesetV1,
    requested_metrics: Vec<BoardMetricV1>,
}

fn downgrade_multiplayer_ranked(net: &crate::multiplayer::NetChannels, reason: &str) {
    tracing::warn!("ranked multiplayer session is browse-only: {reason}");
    if let Err(error) = net.install_ranked_session_setup(None) {
        tracing::warn!("failed to install multiplayer browse-only setup: {error}");
    }
}

async fn wait_for_host_preflight_lobby(
    net: &crate::multiplayer::NetChannels,
    total_started: web_time::Instant,
) -> Result<
    (
        crate::multiplayer::RankedMultiplayerPort,
        RankedPreflightLobbyV1,
    ),
    String,
> {
    let phase_started = web_time::Instant::now();
    let mut last_unavailable = "authenticated ranked lobby is not ready".to_owned();
    loop {
        ensure_ranked_preflight_deadline(
            total_started,
            phase_started,
            RANKED_PREFLIGHT_PHASE_TIMEOUT_MS,
            "authenticated lobby formation",
        )
        .map_err(|error| format!("{error}: {last_unavailable}"))?;
        match net.ranked_port() {
            Ok(port) if port.role() == crate::multiplayer::RankedMultiplayerRole::Host => {
                match port.host_preflight_lobby() {
                    Ok(lobby) => return Ok((port, lobby)),
                    Err(error) => last_unavailable = error,
                }
            }
            Ok(_) => return Err("host preflight resolved a client transport capability".into()),
            Err(error) => last_unavailable = error,
        }
        crate::window::sleep_ms(10).await;
    }
}

async fn wait_for_client_ranked_identity(
    net: &crate::multiplayer::NetChannels,
    total_started: web_time::Instant,
) -> Result<crate::multiplayer::RankedMultiplayerPort, String> {
    let phase_started = web_time::Instant::now();
    let mut last_unavailable = "authenticated ranked client identity is not ready".to_owned();
    loop {
        ensure_ranked_preflight_deadline(
            total_started,
            phase_started,
            RANKED_PREFLIGHT_PHASE_TIMEOUT_MS,
            "authenticated client identity",
        )
        .map_err(|error| format!("{error}: {last_unavailable}"))?;
        match net.ranked_port() {
            Ok(port) if port.role() == crate::multiplayer::RankedMultiplayerRole::Client => {
                match port.authenticated_ranked_identity_pair() {
                    Ok(_) => return Ok(port),
                    Err(error) => last_unavailable = error,
                }
            }
            Ok(_) => return Err("client preflight resolved a host transport capability".into()),
            Err(error) => last_unavailable = error,
        }
        crate::window::sleep_ms(10).await;
    }
}

async fn install_ranked_multiplayer_host(
    net: &crate::multiplayer::NetChannels,
    config: robin_run_protocol::RankedSessionConfigV1,
    requested_scope: ScopeRequestV1,
    requested_metrics: Vec<BoardMetricV1>,
    roster_continuity: robin_run_protocol::CampaignRosterContinuityV1,
    run_preflight_grant_public_key: robin_run_protocol::PublicKey32,
) -> Result<ResolvedMultiplayerAdmission, String> {
    let total_started = web_time::Instant::now();
    let (ready_port, lobby) = wait_for_host_preflight_lobby(net, total_started).await?;
    let port = &ready_port;
    if lobby.host_public_key != local_ranked_public_key().await? {
        return Err(
            "authenticated multiplayer host identity differs from the durable leaderboard identity"
                .to_owned(),
        );
    }
    let run_preflight = if matches!(requested_scope, ScopeRequestV1::IndividualLevel) {
        acquire_fresh_run_preflight(
            lobby.host_public_key,
            &config,
            robin_run_protocol::FreshRunScopeV1::IndividualLevel,
        )
        .await?
    } else {
        acquire_host_campaign_preflight(port, &lobby, &config, roster_continuity, total_started)
            .await?
    };
    ensure_ranked_preflight_deadline(
        total_started,
        web_time::Instant::now(),
        RANKED_PREFLIGHT_PHASE_TIMEOUT_MS,
        "authority grant completion",
    )?;
    let trusted_now_unix_ms = crate::leaderboard_receipt_watcher::now_unix_ms()
        .map_err(|error| format!("ranked preflight current time is unavailable: {error}"))?;
    let setup = OfficialRankedSessionSetupV1 {
        ranked_session: config,
        custom_package_present: false,
        run_preflight,
        run_preflight_grant_public_key,
        trusted_now_unix_ms,
    };
    let resolved = ResolvedMultiplayerAdmission {
        scope_request: setup.scope_request(),
        requested_metrics,
        campaign_controller_public_key: setup.campaign_controller_public_key(),
    };
    let final_port = net.ranked_port()?;
    if final_port.role() != crate::multiplayer::RankedMultiplayerRole::Host
        || final_port.host_preflight_lobby()? != lobby
    {
        return Err("authenticated ranked lobby changed during authority preflight".into());
    }
    // Publish first: installing the host setup may immediately release the
    // genesis challenge, while clients need the independently checked setup
    // before they are allowed to answer it.
    final_port.host_publish_official_session_setup(&setup)?;
    net.install_ranked_session_setup(Some(setup))?;
    Ok(resolved)
}

async fn acquire_host_campaign_preflight(
    port: &crate::multiplayer::RankedMultiplayerPort,
    lobby: &RankedPreflightLobbyV1,
    config: &robin_run_protocol::RankedSessionConfigV1,
    roster_continuity: robin_run_protocol::CampaignRosterContinuityV1,
    total_started: web_time::Instant,
) -> Result<RankedRunPreflightAdmissionV1, String> {
    if config.campaign_content_manifest_sha256.is_none() {
        return Err("campaign-ranked multiplayer config has no campaign content catalog".into());
    }
    let selection_request = CampaignContinuationReceiptSelectionRequestV1::from_lobby(
        lobby.clone(),
        config.clone(),
        roster_continuity,
    )
    .map_err(|error| error.to_string())?;
    let local_selection = select_local_campaign_receipt(&selection_request, lobby.host_public_key)?;
    port.host_publish_continuation_receipt_selection_request(&selection_request)?;

    let expected_remote_keys = lobby
        .participant_public_keys
        .iter()
        .copied()
        .filter(|key| *key != lobby.host_public_key)
        .collect::<BTreeSet<_>>();
    let mut responded_keys = BTreeSet::new();
    let mut responded_seats = BTreeSet::new();
    let mut selections = local_selection.into_iter().collect::<Vec<_>>();
    let mut phase_started = web_time::Instant::now();
    while responded_keys.len() < expected_remote_keys.len() {
        ensure_ranked_preflight_deadline(
            total_started,
            phase_started,
            RANKED_PREFLIGHT_PHASE_TIMEOUT_MS,
            "campaign receipt selection",
        )?;
        match port.try_recv_authorization_event()? {
            Some(crate::multiplayer::RankedAuthorizationEvent::ContinuationReceiptSelectionResponse {
                from,
                response,
            }) => {
                response.validate().map_err(|error| error.to_string())?;
                if response.request() != &selection_request {
                    return Err("campaign receipt response belongs to another preflight".into());
                }
                let responder = response.responder_public_key();
                if !expected_remote_keys.contains(&responder)
                    || !responded_keys.insert(responder)
                    || !responded_seats.insert(from)
                {
                    return Err(
                        "campaign receipt response has an unexpected or duplicate authenticated responder"
                        .into(),
                    );
                }
                port.validate_authenticated_remote_identity(from, responder)?;
                phase_started = web_time::Instant::now();
                if let CampaignContinuationReceiptSelectionResponseV1::Selected { selection } =
                    response
                {
                    selections.push(selection);
                }
            }
            Some(_) => {
                return Err(
                    "ranked transport delivered an out-of-phase authorization event during campaign selection"
                        .into(),
                );
            }
            None => crate::window::sleep_ms(10).await,
        }
    }
    let run_preflight = match selections.as_slice() {
        [] => {
            acquire_fresh_run_preflight(
                lobby.host_public_key,
                config,
                robin_run_protocol::FreshRunScopeV1::CampaignGenesis,
            )
            .await?
        }
        [selection] => {
            acquire_host_continuation_preflight(port, lobby, config, selection, total_started)
                .await?
        }
        _ => {
            return Err(
                "more than one controller selected an active receipt for the exact campaign start"
                    .into(),
            );
        }
    };
    Ok(run_preflight)
}

fn select_local_campaign_receipt(
    request: &CampaignContinuationReceiptSelectionRequestV1,
    local_public_key: robin_run_protocol::PublicKey32,
) -> Result<Option<CampaignContinuationReceiptSelectionV1>, String> {
    let store = crate::leaderboard_chains::load()
        .map_err(|error| format!("load verified campaign-chain receipts: {error}"))?;
    select_campaign_receipt_from_store(&store, request, local_public_key)
}

pub(super) fn select_campaign_receipt_from_store(
    store: &crate::leaderboard_chains::CampaignChainStore,
    request: &CampaignContinuationReceiptSelectionRequestV1,
    local_public_key: robin_run_protocol::PublicKey32,
) -> Result<Option<CampaignContinuationReceiptSelectionV1>, String> {
    request.validate().map_err(|error| error.to_string())?;
    let campaign_manifest = request
        .ranked_session
        .campaign_content_manifest_sha256
        .ok_or_else(|| "campaign receipt selection has no campaign content catalog".to_owned())?;
    let receipt = store
        .continuation_for_policy(
            &request.starting_campaign,
            request.lobby.max_concurrent_players,
            &request.lobby.participant_public_keys,
            request.roster_continuity,
            campaign_manifest,
            request.ranked_session.rules_config_sha256,
            request.ranked_session.ruleset_manifest_sha256,
            request.ranked_session.competition_manifest_sha256,
            local_public_key,
        )
        .map_err(|error| error.to_string())?
        .cloned();
    receipt
        .map(|receipt| {
            let selection = CampaignContinuationReceiptSelectionV1 {
                request: request.clone(),
                receipt,
            };
            selection.validate().map_err(|error| error.to_string())?;
            Ok(selection)
        })
        .transpose()
}

async fn acquire_host_continuation_preflight(
    port: &crate::multiplayer::RankedMultiplayerPort,
    lobby: &RankedPreflightLobbyV1,
    config: &robin_run_protocol::RankedSessionConfigV1,
    selection: &CampaignContinuationReceiptSelectionV1,
    total_started: web_time::Instant,
) -> Result<RankedRunPreflightAdmissionV1, String> {
    let preflight_setup = selection
        .preflight_setup()
        .map_err(|error| error.to_string())?;
    let controller = preflight_setup.campaign_controller_public_key;
    let claim = crate::leaderboard_ranked_session::RankedSessionHost::prepare_campaign_continuation_preflight_claim(
        lobby.host_public_key,
        config.clone(),
        preflight_setup,
    )
    .map_err(|error| error.to_string())?;
    let host_signature = sign_campaign_continuation_preflight_as_host(&claim).await?;
    let controller_signature = if controller == lobby.host_public_key {
        sign_campaign_continuation_preflight_as_controller(&claim).await?
    } else {
        let expected_seat = port.host_publish_continuation_preflight_claim(&claim)?;
        let phase_started = web_time::Instant::now();
        loop {
            ensure_ranked_preflight_deadline(
                total_started,
                phase_started,
                RANKED_PREFLIGHT_PHASE_TIMEOUT_MS,
                "campaign controller signature",
            )?;
            match port.try_recv_authorization_event()? {
                Some(
                    crate::multiplayer::RankedAuthorizationEvent::ContinuationPreflightSignature {
                        from,
                        signature,
                    },
                ) if from == expected_seat && signature.public_key == controller => {
                    port.validate_authenticated_remote_identity(from, controller)?;
                    break signature;
                }
                Some(_) => {
                    return Err(
                        "ranked transport delivered an unexpected controller-signature event"
                            .into(),
                    );
                }
                None => crate::window::sleep_ms(10).await,
            }
        }
    };
    let request = crate::leaderboard_signing::assemble_campaign_continuation_preflight_request(
        claim,
        host_signature,
        controller_signature,
    )
    .map_err(|error| error.to_string())?;
    let api = preflight_api()?;
    let task = api
        .campaign_continuation_preflight_grant(&request)
        .map_err(|error| error.to_string())?;
    let grant = crate::leaderboard_service::decode_campaign_continuation_preflight_grant(
        Ok(task.take().await.map_err(|error| error.to_string())?),
        &request,
    )
    .map_err(|error| error.to_string())?;
    Ok(RankedRunPreflightAdmissionV1::CampaignContinuation { request, grant })
}

async fn install_ranked_multiplayer_client(
    net: &crate::multiplayer::NetChannels,
    locally_prepared_config: robin_run_protocol::RankedSessionConfigV1,
) -> Result<ResolvedMultiplayerAdmission, String> {
    let total_started = web_time::Instant::now();
    let port = wait_for_client_ranked_identity(net, total_started).await?;
    let (authenticated_host_public_key, local_public_key) =
        port.authenticated_ranked_identity_pair()?;
    let mut selection_request: Option<CampaignContinuationReceiptSelectionRequestV1> = None;
    let mut local_selection: Option<CampaignContinuationReceiptSelectionV1> = None;
    let mut authorized_proposal: Option<AuthorizedMultiplayerProposal> = None;
    let mut signed_continuation_claim: Option<
        robin_run_protocol::CampaignContinuationPreflightRequestClaimV1,
    > = None;
    let mut phase_started = web_time::Instant::now();
    let mut phase_timeout_ms = RANKED_PREFLIGHT_INITIAL_CLIENT_TIMEOUT_MS;
    loop {
        ensure_ranked_preflight_deadline(
            total_started,
            phase_started,
            phase_timeout_ms,
            "authenticated host setup",
        )?;
        match port.try_recv_authorization_event()? {
            Some(
                crate::multiplayer::RankedAuthorizationEvent::ContinuationReceiptSelectionRequest(
                    request,
                ),
            ) => {
                if selection_request.is_some() {
                    return Err("host published more than one campaign receipt selection".into());
                }
                if request.lobby.host_public_key != authenticated_host_public_key
                    || request
                        .lobby
                        .participant_public_keys
                        .binary_search(&local_public_key)
                        .is_err()
                {
                    return Err(
                        "campaign receipt selection does not match authenticated lobby identities"
                            .into(),
                    );
                }
                let authority = authorize_multiplayer_host_proposal(
                    &locally_prepared_config,
                    &request.ranked_session,
                    ScopeRequestV1::CampaignGenesis,
                )
                .await?;
                if request.roster_continuity
                    != authority
                        .published_ruleset
                        .manifest
                        .campaign_roster_continuity
                {
                    return Err(
                        "host campaign receipt policy differs from the published ruleset".into(),
                    );
                }
                let selected = select_local_campaign_receipt(&request, local_public_key)?;
                if let Some(selection) = selected.as_ref() {
                    port.client_respond_continuation_receipt_selection(selection)?;
                } else {
                    port.client_respond_no_matching_continuation_receipt(
                        request.clone(),
                        local_public_key,
                    )?;
                }
                selection_request = Some(request);
                local_selection = selected;
                authorized_proposal = Some(authority);
                phase_started = web_time::Instant::now();
                phase_timeout_ms = RANKED_PREFLIGHT_PHASE_TIMEOUT_MS;
            }
            Some(crate::multiplayer::RankedAuthorizationEvent::ContinuationPreflightClaim(
                claim,
            )) => {
                let selection = local_selection.as_ref().ok_or_else(|| {
                    "host requested a controller signature without this peer selecting a receipt"
                        .to_owned()
                })?;
                validate_controller_preflight_claim(
                    &claim,
                    selection,
                    authenticated_host_public_key,
                    local_public_key,
                )?;
                let signature = sign_campaign_continuation_preflight_as_controller(&claim).await?;
                port.client_respond_continuation_preflight(signature)?;
                signed_continuation_claim = Some(claim);
                phase_started = web_time::Instant::now();
                phase_timeout_ms = RANKED_PREFLIGHT_PHASE_TIMEOUT_MS;
            }
            Some(crate::multiplayer::RankedAuthorizationEvent::OfficialSessionSetup(wire)) => {
                let scope_request = wire.run_preflight.scope_request();
                let is_campaign = !matches!(scope_request, ScopeRequestV1::IndividualLevel);
                if is_campaign != selection_request.is_some() {
                    return Err("host skipped or spuriously used campaign receipt discovery".into());
                }
                if let Some(request) = selection_request.as_ref()
                    && request.ranked_session != wire.ranked_session
                {
                    return Err(
                        "official setup differs from the campaign receipt selection proposal"
                            .into(),
                    );
                }
                enforce_controller_selection_result(
                    &wire,
                    local_selection.as_ref(),
                    signed_continuation_claim.as_ref(),
                )?;
                let authority = match authorized_proposal.take() {
                    Some(authority) => authority,
                    None => {
                        authorize_multiplayer_host_proposal(
                            &locally_prepared_config,
                            &wire.ranked_session,
                            scope_request.clone(),
                        )
                        .await?
                    }
                };
                let expectation = OfficialRankedSessionExpectationV1 {
                    ranked_session: wire.ranked_session.clone(),
                    custom_package_present: false,
                    run_preflight_grant_public_key: authority
                        .published_ruleset
                        .manifest
                        .run_preflight_grant_public_key,
                };
                let trusted_now_unix_ms = crate::leaderboard_receipt_watcher::now_unix_ms()
                    .map_err(|error| {
                        format!("ranked setup current time is unavailable: {error}")
                    })?;
                let setup = wire
                    .prepare_for_authenticated_peer(
                        &expectation,
                        authenticated_host_public_key,
                        local_public_key,
                        trusted_now_unix_ms,
                    )
                    .map_err(|error| error.to_string())?;
                ensure_ranked_preflight_deadline(
                    total_started,
                    web_time::Instant::now(),
                    RANKED_PREFLIGHT_PHASE_TIMEOUT_MS,
                    "client authority reconstruction",
                )?;
                let final_port = net.ranked_port()?;
                if final_port.role() != crate::multiplayer::RankedMultiplayerRole::Client
                    || final_port.authenticated_ranked_identity_pair()?
                        != (authenticated_host_public_key, local_public_key)
                {
                    return Err(
                        "authenticated ranked identities changed during authority preflight".into(),
                    );
                }
                let resolved = ResolvedMultiplayerAdmission {
                    scope_request: setup.scope_request(),
                    requested_metrics: authority.requested_metrics,
                    campaign_controller_public_key: setup.campaign_controller_public_key(),
                };
                net.install_ranked_session_setup(Some(setup))?;
                return Ok(resolved);
            }
            Some(_) => {
                return Err(
                    "ranked transport delivered an out-of-phase mission-end authorization event before frame zero"
                        .into(),
                );
            }
            None => crate::window::sleep_ms(10).await,
        }
    }
}

pub(super) fn validate_controller_preflight_claim(
    claim: &robin_run_protocol::CampaignContinuationPreflightRequestClaimV1,
    selection: &CampaignContinuationReceiptSelectionV1,
    authenticated_host_public_key: robin_run_protocol::PublicKey32,
    local_public_key: robin_run_protocol::PublicKey32,
) -> Result<(), String> {
    claim.validate().map_err(|error| error.to_string())?;
    let expected = selection
        .preflight_setup()
        .map_err(|error| error.to_string())?;
    if claim.host_public_key != authenticated_host_public_key
        || claim.campaign_controller_public_key != local_public_key
        || expected.campaign_controller_public_key != local_public_key
        || claim.max_concurrent_players != expected.max_concurrent_players
        || claim.participant_public_keys != expected.participant_public_keys
        || claim.chain_id != expected.chain_id
        || claim.predecessor_run_id != expected.predecessor_run_id
        || claim.predecessor_verification_sha256 != expected.predecessor_verification_sha256
        || claim.starting_campaign != selection.request.starting_campaign
        || claim.ranked_session != selection.request.ranked_session
    {
        return Err(
            "continuation preflight claim differs from the locally selected verified receipt"
                .into(),
        );
    }
    Ok(())
}

pub(super) fn enforce_controller_selection_result(
    wire: &OfficialRankedSessionWireSetupV1,
    local_selection: Option<&CampaignContinuationReceiptSelectionV1>,
    signed_claim: Option<&robin_run_protocol::CampaignContinuationPreflightRequestClaimV1>,
) -> Result<(), String> {
    match (local_selection, signed_claim, &wire.run_preflight) {
        (None, None, _) => Ok(()),
        (
            Some(_),
            Some(expected),
            RankedRunPreflightAdmissionV1::CampaignContinuation { request, .. },
        ) if &request.claim == expected => Ok(()),
        (Some(_), None, _) => Err(
            "host did not request the selected controller's exact continuation signature".into(),
        ),
        (Some(_), Some(_), _) => {
            Err("official setup did not retain the controller-signed continuation".into())
        }
        (None, Some(_), _) => {
            Err("controller signature exists without a selected local receipt".into())
        }
    }
}

async fn authorize_multiplayer_host_proposal(
    locally_prepared: &robin_run_protocol::RankedSessionConfigV1,
    proposed: &robin_run_protocol::RankedSessionConfigV1,
    scope_request: ScopeRequestV1,
) -> Result<AuthorizedMultiplayerProposal, String> {
    validate_host_proposal_against_local_prepared(locally_prepared, proposed)?;
    let api = preflight_api()?;
    let content_task = api
        .content_manifest(proposed.content_manifest_sha256)
        .map_err(|error| error.to_string())?;
    let rules_task = api
        .rules_config(proposed.rules_config_sha256)
        .map_err(|error| error.to_string())?;
    let published_task = api
        .published_ruleset(proposed.ruleset_manifest_sha256)
        .map_err(|error| error.to_string())?;
    let build_task = api
        .build_manifest(proposed.build_manifest_sha256)
        .map_err(|error| error.to_string())?;
    let content = crate::leaderboard_service::decode_content_manifest(
        Ok(content_task
            .take()
            .await
            .map_err(|error| error.to_string())?),
        proposed.content_manifest_sha256,
    )
    .map_err(|error| error.to_string())?;
    let rules = crate::leaderboard_service::decode_rules_config(
        Ok(rules_task.take().await.map_err(|error| error.to_string())?),
        proposed.rules_config_sha256,
    )
    .map_err(|error| error.to_string())?;
    let published = crate::leaderboard_service::decode_published_ruleset(
        Ok(published_task
            .take()
            .await
            .map_err(|error| error.to_string())?),
        proposed.ruleset_manifest_sha256,
    )
    .map_err(|error| error.to_string())?;
    let build = crate::leaderboard_service::decode_build_manifest(
        Ok(build_task.take().await.map_err(|error| error.to_string())?),
        proposed.build_manifest_sha256,
    )
    .map_err(|error| error.to_string())?;
    validate_multiplayer_host_documents(proposed, &scope_request, &content, &rules, &published)?;
    if !build_matches_runtime(&build)? {
        return Err("host-selected verifier build does not match this runtime".into());
    }
    if let Some(campaign_digest) = proposed.campaign_content_manifest_sha256 {
        let task = api
            .campaign_content_manifest(campaign_digest)
            .map_err(|error| error.to_string())?;
        let campaign = crate::leaderboard_service::decode_campaign_content_manifest(
            Ok(task.take().await.map_err(|error| error.to_string())?),
            campaign_digest,
        )
        .map_err(|error| error.to_string())?;
        validate_campaign_content_for_mission(
            &campaign,
            &content,
            proposed.content_manifest_sha256,
        )?;
        published
            .manifest
            .validate_campaign_completion_catalog(&campaign)
            .map_err(|error| error.to_string())?;
    }
    Ok(AuthorizedMultiplayerProposal {
        requested_metrics: published.manifest.metrics.clone(),
        published_ruleset: published,
    })
}

pub(super) fn validate_host_proposal_against_local_prepared(
    locally_prepared: &robin_run_protocol::RankedSessionConfigV1,
    proposed: &robin_run_protocol::RankedSessionConfigV1,
) -> Result<(), String> {
    locally_prepared
        .validate()
        .and_then(|()| proposed.validate())
        .map_err(|error| error.to_string())?;
    let mut normalized = proposed.clone();
    // These two immutable documents select the board/campaign policy, not
    // simulation input. Every other current and future config field must be
    // byte-for-byte equal to the locally prepared engine seal.
    normalized.ruleset_manifest_sha256 = locally_prepared.ruleset_manifest_sha256;
    normalized.campaign_content_manifest_sha256 = locally_prepared.campaign_content_manifest_sha256;
    if &normalized != locally_prepared {
        return Err(
            "host ranked proposal differs from the locally prepared simulation/config seal".into(),
        );
    }
    Ok(())
}

fn validate_multiplayer_host_documents(
    proposed: &robin_run_protocol::RankedSessionConfigV1,
    scope_request: &ScopeRequestV1,
    content: &ContentManifestV1,
    rules: &RulesConfigIdentityV1,
    published: &PublishedRulesetV1,
) -> Result<(), String> {
    proposed
        .validate_content_manifest(content)
        .map_err(|error| error.to_string())?;
    let subject_is_official = official_content_subjects_v1(content.edition)
        .iter()
        .any(|subject| subject == &content.subject);
    if !subject_is_official
        || content.subject.mission_id() != proposed.mission_id
        || content.name != official_content_manifest_name_v1(content.edition, &content.subject)
        || content
            .canonical_digest()
            .map_err(|error| error.to_string())?
            != proposed.content_manifest_sha256
    {
        return Err("host content is not the exact canonical official mission authority".into());
    }
    published
        .manifest
        .validate_ranked_simulation_policy(rules)
        .map_err(|error| error.to_string())?;
    let required_board_scope = match scope_request {
        ScopeRequestV1::IndividualLevel => RulesetBoardScopeV1::IndividualLevel,
        ScopeRequestV1::CampaignGenesis | ScopeRequestV1::CampaignContinuation { .. } => {
            RulesetBoardScopeV1::CampaignMission
        }
    };
    let campaign_presence_matches = match scope_request {
        ScopeRequestV1::IndividualLevel => proposed.campaign_content_manifest_sha256.is_none(),
        ScopeRequestV1::CampaignGenesis | ScopeRequestV1::CampaignContinuation { .. } => {
            proposed.campaign_content_manifest_sha256.is_some()
        }
    };
    if !campaign_presence_matches
        || !matches!(
            published.operational_status,
            RulesetOperationalStatusV1::Active
        )
        || published.manifest.rules_config_sha256 != proposed.rules_config_sha256
        || published
            .manifest
            .allowed_build_manifest_sha256
            .binary_search(&proposed.build_manifest_sha256)
            .is_err()
        || published
            .manifest
            .allowed_content_manifest_sha256
            .binary_search(&proposed.content_manifest_sha256)
            .is_err()
        || proposed
            .campaign_content_manifest_sha256
            .is_some_and(|digest| {
                published
                    .manifest
                    .allowed_campaign_content_manifest_sha256
                    .binary_search(&digest)
                    .is_err()
            })
        || published
            .manifest
            .board_scopes
            .binary_search(&required_board_scope)
            .is_err()
        || !published
            .manifest
            .replay_schema_versions
            .contains(&robin_engine::replay::REPLAY_SCHEMA_VERSION)
        || !published
            .manifest
            .network_protocol_versions
            .contains(&robin_engine::multiplayer::NET_PROTOCOL_VERSION)
    {
        return Err("host ruleset does not admit the exact prepared mission tuple".into());
    }
    Ok(())
}

async fn acquire_single_player_run_preflight(
    ranked_session: &robin_run_protocol::RankedSessionConfigV1,
    scope_request: &ScopeRequestV1,
    campaign_controller_public_key: Option<robin_run_protocol::PublicKey32>,
    local_campaign_chain_receipt: Option<&robin_run_protocol::CampaignChainReceiptV1>,
) -> Result<crate::leaderboard_ranked_session::RankedRunPreflightAdmissionV1, String> {
    let host_public_key = local_ranked_public_key().await?;
    let admission = match scope_request {
        ScopeRequestV1::IndividualLevel => {
            acquire_fresh_run_preflight(
                host_public_key,
                ranked_session,
                robin_run_protocol::FreshRunScopeV1::IndividualLevel,
            )
            .await?
        }
        ScopeRequestV1::CampaignGenesis => {
            acquire_fresh_run_preflight(
                host_public_key,
                ranked_session,
                robin_run_protocol::FreshRunScopeV1::CampaignGenesis,
            )
            .await?
        }
        ScopeRequestV1::CampaignContinuation {
            chain_id,
            predecessor_run_id,
        } => {
            let receipt = local_campaign_chain_receipt.ok_or_else(|| {
                "campaign continuation has no exact locally retained verification receipt"
                    .to_owned()
            })?;
            receipt.validate().map_err(|error| error.to_string())?;
            let controller = campaign_controller_public_key.ok_or_else(|| {
                "campaign continuation has no durable controller identity".to_owned()
            })?;
            if controller != host_public_key
                || receipt.campaign_controller_public_key != controller
                || &receipt.chain_id != chain_id
                || &receipt.predecessor_run_id != predecessor_run_id
                || receipt.state != robin_run_protocol::CampaignChainStateV1::Active
                || receipt.expected_starting_campaign.sha256
                    != ranked_session.starting_campaign_sha256
                || receipt.expected_starting_campaign.byte_length
                    != ranked_session.starting_campaign_byte_length
                || receipt.rules_config_sha256 != ranked_session.rules_config_sha256
                || receipt.ruleset_manifest_sha256 != ranked_session.ruleset_manifest_sha256
                || receipt.campaign_content_manifest_sha256
                    != ranked_session
                        .campaign_content_manifest_sha256
                        .ok_or_else(|| {
                            "campaign continuation config has no campaign content manifest"
                                .to_owned()
                        })?
                || receipt.competition_manifest_sha256 != ranked_session.competition_manifest_sha256
                || receipt.expected_max_concurrent_players != 1
            {
                return Err(
                    "campaign continuation receipt does not match the exact local single-player tuple"
                        .to_owned(),
                );
            }
            let claim = crate::leaderboard_ranked_session::RankedSessionHost::prepare_campaign_continuation_preflight_claim(
                host_public_key,
                ranked_session.clone(),
                crate::leaderboard_ranked_session::CampaignContinuationPreflightSetupV1 {
                    campaign_controller_public_key: controller,
                    max_concurrent_players: 1,
                    participant_public_keys: vec![host_public_key],
                    chain_id: receipt.chain_id.clone(),
                    predecessor_run_id: receipt.predecessor_run_id.clone(),
                    predecessor_verification_sha256: receipt.predecessor_verification_sha256,
                },
            )
            .map_err(|error| error.to_string())?;
            let host_signature = sign_campaign_continuation_preflight_as_host(&claim).await?;
            let controller_signature =
                sign_campaign_continuation_preflight_as_controller(&claim).await?;
            let request =
                crate::leaderboard_signing::assemble_campaign_continuation_preflight_request(
                    claim,
                    host_signature,
                    controller_signature,
                )
                .map_err(|error| error.to_string())?;
            let api = preflight_api()?;
            let task = api
                .campaign_continuation_preflight_grant(&request)
                .map_err(|error| error.to_string())?;
            let grant = crate::leaderboard_service::decode_campaign_continuation_preflight_grant(
                Ok(task.take().await.map_err(|error| error.to_string())?),
                &request,
            )
            .map_err(|error| error.to_string())?;
            crate::leaderboard_ranked_session::RankedRunPreflightAdmissionV1::CampaignContinuation {
                request,
                grant,
            }
        }
    };
    Ok(admission)
}

async fn acquire_fresh_run_preflight(
    host_public_key: robin_run_protocol::PublicKey32,
    ranked_session: &robin_run_protocol::RankedSessionConfigV1,
    scope: robin_run_protocol::FreshRunScopeV1,
) -> Result<crate::leaderboard_ranked_session::RankedRunPreflightAdmissionV1, String> {
    let claim =
        crate::leaderboard_ranked_session::RankedSessionHost::prepare_fresh_run_preflight_claim(
            host_public_key,
            ranked_session.clone(),
            scope,
        )
        .map_err(|error| error.to_string())?;
    let request = sign_fresh_run_preflight_request(claim).await?;
    let api = preflight_api()?;
    let task = api
        .fresh_run_preflight_grant(&request)
        .map_err(|error| error.to_string())?;
    let grant = crate::leaderboard_service::decode_fresh_run_preflight_grant(
        Ok(task.take().await.map_err(|error| error.to_string())?),
        &request,
    )
    .map_err(|error| error.to_string())?;
    Ok(crate::leaderboard_ranked_session::RankedRunPreflightAdmissionV1::Fresh { request, grant })
}

fn preflight_api() -> Result<LeaderboardApi, String> {
    let preferences = crate::leaderboard_preferences::load()
        .map_err(|error| format!("load leaderboard preferences: {error}"))?;
    LeaderboardApi::from_preferences(&preferences)
        .map_err(|error| format!("leaderboard endpoint unavailable: {error}"))
}

#[cfg(not(target_arch = "wasm32"))]
async fn sign_fresh_run_preflight_request(
    claim: robin_run_protocol::FreshRunPreflightRequestClaimV1,
) -> Result<robin_run_protocol::FreshRunPreflightRequestV1, String> {
    crate::leaderboard_signing::sign_fresh_run_preflight_request(claim)
        .map_err(|error| error.to_string())
}

#[cfg(target_arch = "wasm32")]
async fn sign_fresh_run_preflight_request(
    claim: robin_run_protocol::FreshRunPreflightRequestClaimV1,
) -> Result<robin_run_protocol::FreshRunPreflightRequestV1, String> {
    crate::leaderboard_signing::browser_game_sign_fresh_run_preflight_request(&claim)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
async fn sign_campaign_continuation_preflight_as_host(
    claim: &robin_run_protocol::CampaignContinuationPreflightRequestClaimV1,
) -> Result<ParticipantSignatureV1, String> {
    crate::leaderboard_signing::sign_campaign_continuation_preflight_as_host(claim)
        .map_err(|error| error.to_string())
}

#[cfg(target_arch = "wasm32")]
async fn sign_campaign_continuation_preflight_as_host(
    claim: &robin_run_protocol::CampaignContinuationPreflightRequestClaimV1,
) -> Result<ParticipantSignatureV1, String> {
    crate::leaderboard_signing::browser_game_sign_campaign_continuation_preflight_as_host(claim)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
async fn sign_campaign_continuation_preflight_as_controller(
    claim: &robin_run_protocol::CampaignContinuationPreflightRequestClaimV1,
) -> Result<ParticipantSignatureV1, String> {
    crate::leaderboard_signing::sign_campaign_continuation_preflight_as_controller(claim)
        .map_err(|error| error.to_string())
}

#[cfg(target_arch = "wasm32")]
async fn sign_campaign_continuation_preflight_as_controller(
    claim: &robin_run_protocol::CampaignContinuationPreflightRequestClaimV1,
) -> Result<ParticipantSignatureV1, String> {
    crate::leaderboard_signing::browser_game_sign_campaign_continuation_preflight_as_controller(
        claim,
    )
    .await
    .map_err(|error| error.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
async fn create_official_ranked_session(
    setup: OfficialRankedSessionSetupV1,
) -> Result<crate::leaderboard_ranked_session::RankedSessionHost, String> {
    crate::leaderboard_signing::create_native_official_ranked_session(
        robin_engine::multiplayer::NET_PROTOCOL_VERSION,
        setup,
    )
    .map_err(|error| error.to_string())
}

#[cfg(target_arch = "wasm32")]
async fn create_official_ranked_session(
    setup: OfficialRankedSessionSetupV1,
) -> Result<crate::leaderboard_ranked_session::RankedSessionHost, String> {
    crate::leaderboard_signing::create_browser_official_ranked_session(
        robin_engine::multiplayer::NET_PROTOCOL_VERSION,
        setup,
    )
    .await
    .map_err(|error| error.to_string())
}

impl RankedPreFramePlan {
    pub(in crate::game_session) fn browse_only(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        assert!(!reason.trim().is_empty(), "browse-only reason is required");
        Self::BrowseOnly { reason }
    }

    pub(in crate::game_session) fn consume_prepared(
        self,
        prepared: robin_engine::simulation_inputs::PreparedMissionInputs,
    ) -> (robin_engine::engine::Engine, PreparedRankedAdmission) {
        let authority = match self {
            Self::BrowseOnly { reason } => {
                return (
                    robin_engine::engine::Engine::from_prepared(prepared),
                    PreparedRankedAdmission::BrowseOnly { reason },
                );
            }
            Self::Authority(authority) => authority,
        };
        let mounted_documents = prepared
            .static_projection()
            .components()
            .iter()
            .map(|component| component.document.clone())
            .collect::<Vec<_>>();
        let ranked = match robin_engine::simulation_inputs::RankedPreparedMissionInputs::admit(
            prepared,
            robin_engine::simulation_inputs::RankedContentAdmissionV1 {
                manifest: &authority.content_manifest,
                mounted_documents: &mounted_documents,
                rules_config: &authority.rules_config,
                speech_timing: manifest_speech_authority(&authority.content_manifest),
            },
        ) {
            Ok(ranked) => ranked,
            Err((error, prepared)) => {
                return (
                    robin_engine::engine::Engine::from_prepared(prepared),
                    PreparedRankedAdmission::BrowseOnly {
                        reason: format!(
                            "prepared mission inputs do not match published ranked authority: {error}"
                        ),
                    },
                );
            }
        };
        let seal = ranked.seal().clone();
        let ruleset_sha256 = authority.published_ruleset.ruleset_manifest_sha256;
        let config = robin_run_protocol::RankedSessionConfigV1 {
            schema_version: SCHEMA_VERSION_V1,
            mission_id: seal.content_subject.mission_id().to_owned(),
            content_edition: seal.content_edition,
            content_subject: seal.content_subject.clone(),
            simulation_seed: seal.simulation_seed,
            starting_campaign_sha256: seal.starting_campaign_sha256,
            starting_campaign_byte_length: seal.starting_campaign_byte_length,
            prepared_inputs_projection_sha256: seal.prepared_inputs_projection_sha256,
            prepared_mission_inputs_seal_sha256: seal
                .canonical_digest()
                .expect("validated prepared-input seal must canonicalize"),
            build_manifest_sha256: authority.build_manifest_sha256,
            content_manifest_sha256: seal.content_manifest_sha256,
            campaign_content_manifest_sha256: authority.campaign_content_manifest_sha256,
            rules_config_sha256: seal.rules_config_sha256,
            ruleset_manifest_sha256: ruleset_sha256,
            competition_manifest_sha256: None,
            spellforge_content_sha256: seal.spellforge_content_sha256,
            resource_locale_root: seal.resource_locale_root.clone(),
            speech_timing: seal.speech_timing.clone(),
        };
        if let Err(error) = config
            .validate()
            .and_then(|()| config.validate_content_manifest(&authority.content_manifest))
            .and_then(|()| config.validate_prepared_inputs_seal(&seal))
        {
            return (
                robin_engine::engine::Engine::from_prepared(ranked.into_unranked()),
                PreparedRankedAdmission::BrowseOnly {
                    reason: format!("ranked session config rejected prepared authority: {error}"),
                },
            );
        }
        (
            robin_engine::engine::Engine::new_ranked(ranked),
            PreparedRankedAdmission::PendingSignature {
                config,
                scope_request: authority.scope_request,
                requested_metrics: authority.requested_metrics,
                campaign_controller_public_key: authority.campaign_controller_public_key,
                campaign_roster_continuity: authority.campaign_roster_continuity,
                run_preflight_grant_public_key: authority.run_preflight_grant_public_key,
                local_campaign_chain_receipt: authority.local_campaign_chain_receipt,
            },
        )
    }
}

fn manifest_speech_authority(manifest: &ContentManifestV1) -> SpeechTimingAuthorityV1 {
    match &manifest.speech_timing {
        SimulationSpeechTimingSourceV1::BaseInstallation => {
            SpeechTimingAuthorityV1::BaseInstallation
        }
        SimulationSpeechTimingSourceV1::LanguagePack { canonical_locale } => {
            SpeechTimingAuthorityV1::LanguagePack {
                canonical_locale: canonical_locale.clone(),
            }
        }
    }
}

/// Resolve all immutable single-player authorities while the loading screen
/// is active and before `PreparedMissionInputs` is consumed. Any missing or
/// mismatched document produces an explicit browse-only reason at the caller.
pub(in crate::game_session) async fn fetch_single_player_authority(
    mission_id: &str,
    sim_config: robin_engine::engine::SimConfig,
    starting_campaign: &Campaign,
) -> Result<RankedMissionAuthority, String> {
    let preferences = crate::leaderboard_preferences::load()
        .map_err(|error| format!("load leaderboard preferences: {error}"))?;
    if preferences.preferred_competition_id.is_some() {
        return Err(
            "competition runs require a server grant before frame zero and are not armed by ordinary mission launch"
                .to_owned(),
        );
    }
    let api = LeaderboardApi::from_preferences(&preferences)
        .map_err(|error| format!("leaderboard endpoint unavailable: {error}"))?;
    let metadata = crate::leaderboard_service::decode_metadata(Ok(api
        .metadata()
        .map_err(|error| error.to_string())?
        .take()
        .await
        .map_err(|error| error.to_string())?))
    .map_err(|error| error.to_string())?;
    let mission = metadata
        .missions
        .iter()
        .find(|mission| mission.mission_id == mission_id)
        .ok_or_else(|| format!("mission `{mission_id}` has no published ranked content"))?;
    if !matches!(
        preferences.preferred_scope,
        LeaderboardScope::IndividualLevel
    ) {
        return fetch_campaign_authority(
            &api,
            &metadata,
            mission,
            mission_id,
            sim_config,
            starting_campaign,
            &preferences,
        )
        .await;
    }
    let category = BoardCategoryV1::IndividualLevel;
    let candidate_facets = metadata
        .rulesets
        .iter()
        .filter(|ruleset| {
            ruleset.categories.contains(&category)
                && ruleset.content
                    == RunContentIdentityV1::Mission {
                        content_manifest_sha256: mission.content_manifest_sha256,
                    }
                && preferences
                    .preferred_preset_id
                    .as_deref()
                    .is_none_or(|id| ruleset.preset_id.as_str() == id)
                && preferences
                    .preferred_difficulty_id
                    .as_deref()
                    .is_none_or(|id| ruleset.difficulty_id.as_str() == id)
        })
        .collect::<Vec<_>>();
    if candidate_facets.is_empty() {
        return Err(format!(
            "no published {:?} ranked ruleset matches mission `{mission_id}` and the selected facets",
            preferences.preferred_scope
        ));
    }

    let content_task = api
        .content_manifest(mission.content_manifest_sha256)
        .map_err(|error| error.to_string())?;
    let content_manifest = crate::leaderboard_service::decode_content_manifest(
        Ok(content_task
            .take()
            .await
            .map_err(|error| error.to_string())?),
        mission.content_manifest_sha256,
    )
    .map_err(|error| error.to_string())?;

    // A missing UI preference is not authority to pick the lexicographically
    // first board. Resolve every candidate against the exact loaded SimConfig
    // and the typed preset/difficulty identity, then require one unique match.
    let mut exact_matches = Vec::new();
    let mut rejected = Vec::new();
    for facet in candidate_facets {
        match fetch_and_validate_ruleset_candidate(
            &api,
            mission_id,
            sim_config,
            mission.content_manifest_sha256,
            facet,
            &content_manifest,
            RulesetBoardScopeV1::IndividualLevel,
            None,
        )
        .await
        {
            Ok(candidate) => exact_matches.push(candidate),
            Err(error) => rejected.push(format!(
                "{}/{}: {error}",
                facet.preset_id.as_str(),
                facet.difficulty_id.as_str()
            )),
        }
    }
    let [(ruleset, rules_config, published_ruleset)] = exact_matches.as_slice() else {
        return match exact_matches.len() {
            0 => Err(format!(
                "no published ranked facet exactly matches the loaded gameplay configuration ({})",
                rejected.join("; ")
            )),
            count => Err(format!(
                "{count} published ranked facets match the loaded gameplay configuration; choose an explicit preset and difficulty"
            )),
        };
    };
    let build_manifest_sha256 = select_current_build(&api, &published_ruleset).await?;
    Ok(RankedMissionAuthority {
        content_manifest,
        rules_config: rules_config.clone(),
        published_ruleset: published_ruleset.clone(),
        build_manifest_sha256,
        scope_request: ScopeRequestV1::IndividualLevel,
        requested_metrics: ruleset.metrics.clone(),
        campaign_content_manifest_sha256: None,
        campaign_controller_public_key: None,
        campaign_roster_continuity: published_ruleset.manifest.campaign_roster_continuity,
        run_preflight_grant_public_key: published_ruleset.manifest.run_preflight_grant_public_key,
        local_campaign_chain_receipt: None,
    })
}

#[allow(clippy::too_many_arguments)]
async fn fetch_campaign_authority(
    api: &LeaderboardApi,
    metadata: &LeaderboardMetadataV1,
    mission: &robin_run_protocol::MissionFacetV1,
    mission_id: &str,
    sim_config: robin_engine::engine::SimConfig,
    starting_campaign: &Campaign,
    preferences: &LeaderboardPreferences,
) -> Result<RankedMissionAuthority, String> {
    let starting_campaign_bytes = bitcode::encode(starting_campaign);
    if starting_campaign_bytes.is_empty() {
        return Err("campaign-ranked mission has no exact starting campaign bytes".to_owned());
    }
    let local_public_key = local_ranked_public_key().await?;
    let participant_public_keys = [local_public_key];
    let store = crate::leaderboard_chains::load()
        .map_err(|error| format!("load verified campaign-chain receipts: {error}"))?;

    let content_task = api
        .content_manifest(mission.content_manifest_sha256)
        .map_err(|error| error.to_string())?;
    let content_manifest = crate::leaderboard_service::decode_content_manifest(
        Ok(content_task
            .take()
            .await
            .map_err(|error| error.to_string())?),
        mission.content_manifest_sha256,
    )
    .map_err(|error| error.to_string())?;

    let candidate_facets = metadata
        .rulesets
        .iter()
        .filter(|facet| {
            facet.categories.contains(&BoardCategoryV1::Campaign)
                && facet.supports_full_campaign_boards
                && matches!(facet.content, RunContentIdentityV1::FullCampaign { .. })
                && preferences
                    .preferred_preset_id
                    .as_deref()
                    .is_none_or(|id| facet.preset_id.as_str() == id)
                && preferences
                    .preferred_difficulty_id
                    .as_deref()
                    .is_none_or(|id| facet.difficulty_id.as_str() == id)
        })
        .collect::<Vec<_>>();
    if candidate_facets.is_empty() {
        return Err(format!(
            "no published campaign-ranked facet matches mission `{mission_id}` and the selected preset/difficulty"
        ));
    }

    let mut exact_matches = Vec::new();
    let mut rejected = Vec::new();
    for facet in candidate_facets {
        let result = async {
            let RunContentIdentityV1::FullCampaign {
                campaign_content_manifest_sha256,
            } = facet.content
            else {
                unreachable!("campaign candidate filter fixed full-campaign identity")
            };
            let catalog_task = api
                .campaign_content_manifest(campaign_content_manifest_sha256)
                .map_err(|error| error.to_string())?;
            let campaign_content = crate::leaderboard_service::decode_campaign_content_manifest(
                Ok(catalog_task
                    .take()
                    .await
                    .map_err(|error| error.to_string())?),
                campaign_content_manifest_sha256,
            )
            .map_err(|error| error.to_string())?;
            validate_campaign_content_for_mission(
                &campaign_content,
                &content_manifest,
                mission.content_manifest_sha256,
            )?;
            let (_, rules_config, published_ruleset) = fetch_and_validate_ruleset_candidate(
                api,
                mission_id,
                sim_config,
                mission.content_manifest_sha256,
                facet,
                &content_manifest,
                RulesetBoardScopeV1::CampaignMission,
                Some(campaign_content_manifest_sha256),
            )
            .await?;
            published_ruleset
                .manifest
                .validate_campaign_completion_catalog(&campaign_content)
                .map_err(|error| error.to_string())?;
            if matches!(preferences.preferred_scope, LeaderboardScope::FullCampaign)
                && published_ruleset
                    .manifest
                    .board_scopes
                    .binary_search(&RulesetBoardScopeV1::FullCampaign)
                    .is_err()
            {
                return Err("ruleset does not publish full-campaign boards".to_owned());
            }
            let build_manifest_sha256 = select_current_build(api, &published_ruleset).await?;
            Ok::<_, String>((
                facet,
                rules_config,
                published_ruleset,
                build_manifest_sha256,
                campaign_content_manifest_sha256,
            ))
        }
        .await;
        match result {
            Ok(candidate) => exact_matches.push(candidate),
            Err(error) => rejected.push(format!(
                "{}/{}: {error}",
                facet.preset_id.as_str(),
                facet.difficulty_id.as_str()
            )),
        }
    }
    let [
        (
            facet,
            rules_config,
            published_ruleset,
            build_manifest_sha256,
            campaign_content_manifest_sha256,
        ),
    ] = exact_matches.as_slice()
    else {
        return match exact_matches.len() {
            0 => Err(format!(
                "no published campaign facet has exact local authority ({})",
                rejected.join("; ")
            )),
            count => Err(format!(
                "{count} campaign facets match the exact local state; choose an explicit preset and difficulty"
            )),
        };
    };
    let receipt = store
        .continuation_for_exact_campaign_policy(
            &starting_campaign_bytes,
            1,
            &participant_public_keys,
            published_ruleset.manifest.campaign_roster_continuity,
            *campaign_content_manifest_sha256,
            facet.rules_config_sha256,
            facet.ruleset_manifest_sha256,
            None,
            local_public_key,
        )
        .map_err(|error| error.to_string())?
        .cloned();
    let (scope_request, local_campaign_chain_receipt) = match receipt {
        Some(receipt) if receipt.state == robin_run_protocol::CampaignChainStateV1::Active => (
            ScopeRequestV1::CampaignContinuation {
                chain_id: receipt.chain_id.clone(),
                predecessor_run_id: receipt.predecessor_run_id.clone(),
            },
            Some(receipt),
        ),
        Some(_) => return Err("matching campaign chain is already complete".to_owned()),
        None => (ScopeRequestV1::CampaignGenesis, None),
    };
    Ok(RankedMissionAuthority {
        content_manifest,
        rules_config: rules_config.clone(),
        published_ruleset: published_ruleset.clone(),
        build_manifest_sha256: *build_manifest_sha256,
        scope_request,
        requested_metrics: facet.metrics.clone(),
        campaign_content_manifest_sha256: Some(*campaign_content_manifest_sha256),
        campaign_controller_public_key: Some(local_public_key),
        campaign_roster_continuity: published_ruleset.manifest.campaign_roster_continuity,
        run_preflight_grant_public_key: published_ruleset.manifest.run_preflight_grant_public_key,
        local_campaign_chain_receipt,
    })
}

fn validate_campaign_content_for_mission(
    campaign: &CampaignContentManifestV1,
    content: &ContentManifestV1,
    expected_content_sha256: Digest32,
) -> Result<(), String> {
    campaign.validate().map_err(|error| error.to_string())?;
    if campaign.edition != content.edition
        || campaign.content_for(&content.subject) != Some(expected_content_sha256)
    {
        return Err(
            "campaign content catalog does not contain the exact loaded official mission"
                .to_owned(),
        );
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
async fn local_ranked_public_key() -> Result<robin_run_protocol::PublicKey32, String> {
    crate::leaderboard_signing::local_public_key().map_err(|error| error.to_string())
}

#[cfg(target_arch = "wasm32")]
async fn local_ranked_public_key() -> Result<robin_run_protocol::PublicKey32, String> {
    crate::leaderboard_signing::browser_game_public_key()
        .await
        .map_err(|error| error.to_string())
}

async fn fetch_and_validate_ruleset_candidate<'a>(
    api: &LeaderboardApi,
    mission_id: &str,
    sim_config: robin_engine::engine::SimConfig,
    expected_content_sha256: Digest32,
    facet: &'a robin_run_protocol::RulesetFacetV1,
    content_manifest: &ContentManifestV1,
    required_board_scope: RulesetBoardScopeV1,
    campaign_content_manifest_sha256: Option<Digest32>,
) -> Result<
    (
        &'a robin_run_protocol::RulesetFacetV1,
        RulesConfigIdentityV1,
        PublishedRulesetV1,
    ),
    String,
> {
    let rules_task = api
        .rules_config(facet.rules_config_sha256)
        .map_err(|error| error.to_string())?;
    let published_task = api
        .published_ruleset(facet.ruleset_manifest_sha256)
        .map_err(|error| error.to_string())?;
    let rules_config = crate::leaderboard_service::decode_rules_config(
        Ok(rules_task.take().await.map_err(|error| error.to_string())?),
        facet.rules_config_sha256,
    )
    .map_err(|error| error.to_string())?;
    let published_ruleset = crate::leaderboard_service::decode_published_ruleset(
        Ok(published_task
            .take()
            .await
            .map_err(|error| error.to_string())?),
        facet.ruleset_manifest_sha256,
    )
    .map_err(|error| error.to_string())?;
    validate_single_player_authority(
        mission_id,
        sim_config,
        expected_content_sha256,
        facet,
        content_manifest,
        &rules_config,
        &published_ruleset,
        required_board_scope,
        campaign_content_manifest_sha256,
    )?;
    Ok((facet, rules_config, published_ruleset))
}

#[allow(clippy::too_many_arguments)]
fn validate_single_player_authority(
    mission_id: &str,
    sim_config: robin_engine::engine::SimConfig,
    expected_content_sha256: Digest32,
    facet: &robin_run_protocol::RulesetFacetV1,
    content: &ContentManifestV1,
    rules: &RulesConfigIdentityV1,
    published: &PublishedRulesetV1,
    required_board_scope: RulesetBoardScopeV1,
    campaign_content_manifest_sha256: Option<Digest32>,
) -> Result<(), String> {
    let subject_is_official = official_content_subjects_v1(content.edition)
        .iter()
        .any(|subject| subject == &content.subject);
    if !subject_is_official
        || content.subject.mission_id() != mission_id
        || content.name != official_content_manifest_name_v1(content.edition, &content.subject)
        || content
            .canonical_digest()
            .map_err(|error| error.to_string())?
            != expected_content_sha256
    {
        return Err(
            "content manifest is not the exact canonical official mission authority".into(),
        );
    }
    let expected_sim_config =
        robin_engine::simulation_inputs::validate_ranked_simulation_policy_rules_config_v1(rules)
            .map_err(|error| error.to_string())?
            .0;
    if expected_sim_config != sim_config {
        return Err("loaded mission SimConfig differs from the published ranked policy".into());
    }
    published
        .manifest
        .validate_ranked_simulation_policy(rules)
        .map_err(|error| error.to_string())?;
    if !matches!(
        published.operational_status,
        RulesetOperationalStatusV1::Active
    ) || published
        .manifest
        .allowed_content_manifest_sha256
        .binary_search(&expected_content_sha256)
        .is_err()
        || published
            .manifest
            .board_scopes
            .binary_search(&required_board_scope)
            .is_err()
        || campaign_content_manifest_sha256.is_some_and(|digest| {
            published
                .manifest
                .allowed_campaign_content_manifest_sha256
                .binary_search(&digest)
                .is_err()
        })
        || published.manifest.metrics != facet.metrics
        || published.manifest.rules_config_sha256 != facet.rules_config_sha256
        || published.manifest.preset_id != facet.preset_id
        || published.manifest.preset_name != facet.preset_name
        || published.manifest.difficulty_id != facet.difficulty_id
        || published.manifest.difficulty_name != facet.difficulty_name
        || !published
            .manifest
            .replay_schema_versions
            .contains(&robin_engine::replay::REPLAY_SCHEMA_VERSION)
        || !published
            .manifest
            .network_protocol_versions
            .contains(&robin_engine::multiplayer::NET_PROTOCOL_VERSION)
    {
        return Err("published ruleset does not admit the exact local mission tuple".into());
    }
    Ok(())
}

async fn select_current_build(
    api: &LeaderboardApi,
    published: &PublishedRulesetV1,
) -> Result<Digest32, String> {
    for digest in &published.manifest.allowed_build_manifest_sha256 {
        let task = api
            .build_manifest(*digest)
            .map_err(|error| error.to_string())?;
        let build = crate::leaderboard_service::decode_build_manifest(
            Ok(task.take().await.map_err(|error| error.to_string())?),
            *digest,
        )
        .map_err(|error| error.to_string())?;
        if build_matches_runtime(&build)? {
            return Ok(*digest);
        }
    }
    Err("no allowlisted verifier build matches this engine and protocol version".to_owned())
}

fn build_matches_runtime(build: &VersionedBuildManifest) -> Result<bool, String> {
    let build = build
        .backend_visible_v1()
        .map_err(|error| error.to_string())?;
    Ok(
        build.source_commit == robin_replay_format::ENGINE_VERSION_HASH
            && build.replay_schema_version == robin_engine::replay::REPLAY_SCHEMA_VERSION
            && build.save_schema_version == crate::save_file::SAVE_FORMAT_VERSION
            && build.network_protocol_version == robin_engine::multiplayer::NET_PROTOCOL_VERSION,
    )
}
