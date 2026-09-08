//! Mission-lifetime inputs for the post-mission leaderboard surface.
//!
//! Ranked authority must exist before simulation begins. This module therefore
//! captures the exact pre-mission campaign and owns an explicit admission
//! state instead of trying to reconstruct a signed genesis at debrief time.
//! Native, browser, and multiplayer frontends all resolve the same prepared
//! authority before frame zero. Multiplayer transports retain the signed
//! lifecycle; this module never creates a second replay or identity lane.

use crate::leaderboard_browse::{LeaderboardBrowseEvent, LeaderboardBrowser};
use crate::leaderboard_mission_end::{
    ActiveMissionReplayExporter, HttpMissionEndLeaderboardBackend,
    LocalMissionEndSubmissionAuthorizer, MissionEndBoard, MissionEndLeaderboardAction,
    MissionEndLeaderboardController, MissionEndLeaderboardEvent, MissionEndOutcome,
    MissionEndPeerCoSigner, MissionEndReplayExporter, MissionEndRunBundle,
    MissionEndSubmissionAuthorizer, MissionEndSubmissionInput, MissionEndTask,
    ParticipantSigningProgress, PeerCoSignPoll, SubmissionAuthorizationRequest,
    SubmissionAuthorizationTask,
};
use crate::leaderboard_preferences::{LeaderboardPreferences, LeaderboardScope, LeaderboardTab};
use crate::leaderboard_service::{DEFAULT_BOARD_PAGE_LIMIT, LeaderboardApi};
use robin_engine::campaign::Campaign;
use robin_run_protocol::{
    BoardCategoryV1, BoardMetricV1, CampaignContentManifestV1, CampaignContinuationAuthorizationV1,
    CanonicalDocument as _, CompetitionStateV1, ContentManifestV1, Digest32, LeaderboardMetadataV1,
    LeaderboardQuerySubjectV1, LeaderboardQueryV1, LeaderboardSubjectV1, ParticipantSignatureV1,
    PublishedRulesetV1, RulesConfigIdentityV1, RulesetBoardScopeV1, RulesetOperationalStatusV1,
    RunContentIdentityV1, SCHEMA_VERSION_V1, ScopeRequestV1, Signature64, SignatureAlgorithmV1,
    SignedSubmissionV1, SimulationSpeechTimingSourceV1, SpeechTimingAuthorityV1,
    SubmissionOfferRequestV1, Validate as _, VersionedBuildManifest,
    official_content_manifest_name_v1, official_content_subjects_v1,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use crate::leaderboard_ranked_session::{
    CampaignContinuationReceiptSelectionRequestV1, CampaignContinuationReceiptSelectionResponseV1,
    CampaignContinuationReceiptSelectionV1, OfficialRankedSessionExpectationV1,
    OfficialRankedSessionSetupV1, OfficialRankedSessionWireSetupV1, RankedPreflightLobbyV1,
    RankedRunPreflightAdmissionV1,
};

use crate::ingame_menu::layout::{
    MenuTransform, dim_screen, draw_screen_background, enter_modal_gpu_phase, render_text_virt_font,
};
use crate::ingame_menu::widget_bridge::ModalCursor;
use crate::ingame_menu::{IngameMenuResources, MissionEndLeaderboardScreen};
use crate::renderer::Renderer;
use crate::window::GameWindow;

/// Server-published immutable authorities selected before the prepared engine
/// capability is consumed. The setup layer still has to compare the exact
/// eight local projection documents and admit this authority through
/// `RankedPreparedMissionInputs` before it may construct a ranked engine.
pub(super) struct RankedMissionAuthority {
    pub(super) content_manifest: ContentManifestV1,
    pub(super) rules_config: RulesConfigIdentityV1,
    pub(super) published_ruleset: PublishedRulesetV1,
    pub(super) build_manifest_sha256: Digest32,
    pub(super) scope_request: ScopeRequestV1,
    pub(super) requested_metrics: Vec<BoardMetricV1>,
    pub(super) campaign_content_manifest_sha256: Option<Digest32>,
    pub(super) campaign_controller_public_key: Option<robin_run_protocol::PublicKey32>,
    /// Published campaign roster continuity is part of the immutable ranked
    /// ruleset. Multiplayer receipt discovery must use this retained policy;
    /// it may never infer continuity from the predecessor receipt or a host
    /// transport proposal.
    pub(super) campaign_roster_continuity: robin_run_protocol::CampaignRosterContinuityV1,
    /// Exact pre-frame grant authority pinned by the selected immutable
    /// ruleset. This must survive prepared-input admission; rediscovering a
    /// key from later metadata would open a second authority-selection lane.
    pub(super) run_preflight_grant_public_key: robin_run_protocol::PublicKey32,
    /// Locally retained server receipt for a campaign continuation. It is
    /// handed to the closed controller/transport preflight flow and is never
    /// reconstructed from campaign totals or a public board response.
    pub(super) local_campaign_chain_receipt: Option<robin_run_protocol::CampaignChainReceiptV1>,
}

pub(super) enum RankedPreFramePlan {
    BrowseOnly { reason: String },
    Authority(RankedMissionAuthority),
}

pub(super) enum PreparedRankedAdmission {
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
    pub(super) async fn sign_before_frame_zero(&mut self, custom_package_present: bool) {
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
    pub(super) async fn install_multiplayer_before_frame_zero(
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

    pub(super) fn take_mission_admission(&mut self) -> RankedMissionAdmission {
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
const RANKED_PREFLIGHT_PHASE_TIMEOUT_MS: u128 = 120_000;
/// A client cannot observe the host's full-lobby transition for a fresh run,
/// so its first setup wait composes lobby formation plus one service phase.
const RANKED_PREFLIGHT_INITIAL_CLIENT_TIMEOUT_MS: u128 = 2 * RANKED_PREFLIGHT_PHASE_TIMEOUT_MS;
/// Four bounded phases: authenticated lobby formation, receipt selection,
/// controller authorization, and service-grant/setup publication. Per-phase
/// progress can extend an early peer's wait, but cannot keep it alive forever.
const RANKED_PREFLIGHT_TOTAL_TIMEOUT_MS: u128 = 4 * RANKED_PREFLIGHT_PHASE_TIMEOUT_MS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RankedPreflightTimeout {
    PhaseInactivity,
    Total,
}

fn ranked_preflight_timeout(
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

fn ensure_ranked_preflight_deadline(
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

fn select_campaign_receipt_from_store(
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

fn validate_controller_preflight_claim(
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

fn enforce_controller_selection_result(
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

fn validate_host_proposal_against_local_prepared(
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
    pub(super) fn browse_only(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        assert!(!reason.trim().is_empty(), "browse-only reason is required");
        Self::BrowseOnly { reason }
    }

    pub(super) fn consume_prepared(
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
pub(super) async fn fetch_single_player_authority(
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

/// Authority fixed before the first simulation frame.
///
/// `BrowseOnly` is a deliberate state, not a failed attempt to invent the
/// missing signatures later. The authorized arm is the sole integration seam
/// for the ranked-session setup flow.
pub(super) enum RankedMissionAdmission {
    BrowseOnly { reason: String },
    Authorized(MissionEndSubmissionInput),
    Signed(SignedRankedMissionAdmission),
}

pub(super) struct SignedRankedMissionAdmission {
    lifecycle: crate::leaderboard_ranked_session::SharedRankedSessionLifecycle,
    scope_request: ScopeRequestV1,
    requested_metrics: Vec<BoardMetricV1>,
    campaign_controller_public_key: Option<robin_run_protocol::PublicKey32>,
}

impl RankedMissionAdmission {
    fn browse_only(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        assert!(!reason.is_empty(), "browse-only admission needs a reason");
        Self::BrowseOnly { reason }
    }

    #[allow(dead_code)]
    pub(super) fn authorized(input: MissionEndSubmissionInput) -> Self {
        Self::Authorized(input)
    }

    fn materialize_terminal(
        &mut self,
        mission_id: &str,
        starting_campaign_bytes: Arc<[u8]>,
    ) -> Result<(), String> {
        if !matches!(self, Self::Signed(_)) {
            return Ok(());
        }
        let replay = crate::http_server::active_replay_snapshot()?.parse_sync()?;
        self.materialize_terminal_from_replay(mission_id, starting_campaign_bytes, &replay)
    }

    fn materialize_terminal_from_replay(
        &mut self,
        mission_id: &str,
        starting_campaign_bytes: Arc<[u8]>,
        replay: &robin_engine::replay::ReplayData,
    ) -> Result<(), String> {
        let Self::Signed(signed) = self else {
            return Ok(());
        };
        let evidence = signed
            .lifecycle
            .lock()
            .map_err(|_| "ranked session lifecycle lock was poisoned".to_owned())?
            .evidence_for_replay(replay)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "ranked session was downgraded before terminal evidence".to_owned())?;
        if evidence.session_genesis.claim.ranked_session.mission_id != mission_id {
            return Err("ranked session evidence names a different mission".to_owned());
        }
        let claim_count = u16::try_from(evidence.participant_claims.len())
            .map_err(|_| "ranked participant roster exceeds protocol bounds".to_owned())?;
        let (participant_instance_count, max_concurrent_players) = evidence
            .replay_session_transcript
            .validate_and_derive_counts()
            .map_err(|error| error.to_string())?;
        if claim_count != participant_instance_count {
            return Err(format!(
                "ranked participant claim count {claim_count} differs from authenticated transcript count {participant_instance_count}"
            ));
        }
        let ruleset_manifest_sha256 = evidence
            .session_genesis
            .claim
            .ranked_session
            .ruleset_manifest_sha256;
        let competition_manifest_sha256 = evidence
            .session_genesis
            .claim
            .ranked_session
            .competition_manifest_sha256;
        let offer_request = SubmissionOfferRequestV1 {
            schema_version: SCHEMA_VERSION_V1,
            max_concurrent_players,
            participant_instance_count,
            participant_claims: evidence.participant_claims,
            session_genesis: evidence.session_genesis,
            mission_id: mission_id.to_owned(),
            scope_request: signed.scope_request.clone(),
            ruleset_manifest_sha256,
            competition_manifest_sha256,
        };
        offer_request
            .validate()
            .map_err(|error| error.to_string())?;
        let input = MissionEndSubmissionInput {
            offer_request,
            replay_session_transcript: evidence.replay_session_transcript,
            requested_metrics: signed.requested_metrics.clone(),
            campaign_controller_public_key: signed.campaign_controller_public_key,
            starting_campaign_bytes,
        };
        // Use the public bundle validator as the final cross-document gate;
        // no partially constructed terminal evidence becomes submittable.
        let probe = MissionEndRunBundle {
            outcome: MissionEndOutcome::Won,
            multiplayer: false,
            boards: authorized_boards(&input, None)?,
            eligible_submission: Some(input.clone()),
            submission_unavailable_reason: None,
        };
        probe.validate().map_err(|error| error.to_string())?;
        *self = Self::Authorized(input);
        Ok(())
    }
}

enum MetadataLoad {
    Loading(LeaderboardBrowser),
    Ready(LeaderboardMetadataV1),
    Failed(String),
}

/// State constructed at mission bootstrap and consumed exactly once when the
/// engine first reports a terminal result.
pub(super) struct MissionLeaderboardRuntime {
    preparation: Option<MissionEndPreparation>,
}

impl MissionLeaderboardRuntime {
    pub(super) fn new(
        starting_campaign: &Campaign,
        mission_id: String,
        multiplayer: bool,
        admission: RankedMissionAdmission,
        ranked_multiplayer_port: Option<crate::multiplayer::RankedMultiplayerPort>,
    ) -> Self {
        assert!(!mission_id.is_empty(), "leaderboard mission id is required");
        let starting_campaign_bytes: Arc<[u8]> = bitcode::encode(starting_campaign).into();
        assert!(
            !starting_campaign_bytes.is_empty(),
            "bitcode campaign encoding cannot be empty"
        );
        let (preferences, metadata) = match crate::leaderboard_preferences::load() {
            Ok(preferences) => match LeaderboardApi::from_preferences(&preferences) {
                Ok(api) => {
                    let mut browser = LeaderboardBrowser::new(api);
                    let metadata = match browser.begin_metadata() {
                        Ok(()) => MetadataLoad::Loading(browser),
                        Err(error) => MetadataLoad::Failed(error.to_string()),
                    };
                    (preferences, metadata)
                }
                Err(error) => (
                    preferences,
                    MetadataLoad::Failed(format!("leaderboard endpoint unavailable: {error}")),
                ),
            },
            Err(error) => (
                LeaderboardPreferences::default(),
                MetadataLoad::Failed(format!(
                    "leaderboard preferences could not be loaded: {error}"
                )),
            ),
        };
        Self {
            preparation: Some(MissionEndPreparation {
                mission_id,
                multiplayer,
                starting_campaign_bytes,
                admission,
                ranked_multiplayer_port,
                preferences,
                metadata,
                outcome: None,
            }),
        }
    }

    /// Freeze the terminal outcome and transfer the prestarted metadata task
    /// to the cooperative UI owner. The caller invokes this before recorder
    /// finalization; the returned task must not be polled until the next outer
    /// frame, after the terminal replay record has been flushed.
    pub(super) fn capture_terminal(
        &mut self,
        outcome: MissionEndOutcome,
    ) -> Result<MissionEndPreparation, String> {
        let mut preparation = self
            .preparation
            .take()
            .ok_or_else(|| "mission-end leaderboard was captured more than once".to_owned())?;
        preparation.outcome = Some(outcome);
        Ok(preparation)
    }
}

/// Frame-polled work required before a validated [`MissionEndRunBundle`] can
/// be handed to the existing controller.
pub(super) struct MissionEndPreparation {
    mission_id: String,
    multiplayer: bool,
    starting_campaign_bytes: Arc<[u8]>,
    admission: RankedMissionAdmission,
    ranked_multiplayer_port: Option<crate::multiplayer::RankedMultiplayerPort>,
    preferences: LeaderboardPreferences,
    metadata: MetadataLoad,
    outcome: Option<MissionEndOutcome>,
}

impl MissionEndPreparation {
    pub(super) fn preferences(&self) -> &LeaderboardPreferences {
        &self.preferences
    }

    /// Poll once. `None` means the bounded HTTP task is still in flight.
    pub(super) fn poll_bundle(&mut self) -> Option<Result<MissionEndRunBundle, String>> {
        let authors_submission = self
            .ranked_multiplayer_port
            .as_ref()
            .is_none_or(|port| port.role() == crate::multiplayer::RankedMultiplayerRole::Host);
        if authors_submission
            && matches!(self.admission, RankedMissionAdmission::Signed(_))
            && let Err(error) = self
                .admission
                .materialize_terminal(&self.mission_id, self.starting_campaign_bytes.clone())
        {
            self.admission = RankedMissionAdmission::browse_only(format!(
                "terminal ranked evidence could not be sealed: {error}"
            ));
        }
        // Signed admission already fixes all submission and query facets.
        // Metadata is only browse/display authority and must never block or
        // suppress an eligible always-submit flow.
        if matches!(self.admission, RankedMissionAdmission::Authorized(_)) {
            let metadata = poll_metadata_once(&mut self.metadata).ok().flatten();
            return Some(build_bundle(
                &self.mission_id,
                self.multiplayer,
                self.starting_campaign_bytes.clone(),
                self.outcome
                    .expect("terminal preparation must freeze an outcome before polling"),
                &self.admission,
                &self.preferences,
                metadata.as_ref(),
            ));
        }
        if matches!(self.admission, RankedMissionAdmission::Signed(_)) {
            return Some(build_bundle(
                &self.mission_id,
                self.multiplayer,
                self.starting_campaign_bytes.clone(),
                self.outcome
                    .expect("terminal preparation must freeze an outcome before polling"),
                &self.admission,
                &self.preferences,
                None,
            ));
        }
        let metadata = match poll_metadata_once(&mut self.metadata) {
            Ok(Some(metadata)) => metadata,
            Ok(None) => return None,
            Err(error) => return Some(Err(error)),
        };
        Some(build_bundle(
            &self.mission_id,
            self.multiplayer,
            self.starting_campaign_bytes.clone(),
            self.outcome
                .expect("terminal preparation must freeze an outcome before polling"),
            &self.admission,
            &self.preferences,
            Some(&metadata),
        ))
    }
}

fn poll_metadata_once(
    metadata: &mut MetadataLoad,
) -> Result<Option<LeaderboardMetadataV1>, String> {
    match metadata {
        MetadataLoad::Loading(browser) => match browser.poll() {
            None => Ok(None),
            Some(Ok(LeaderboardBrowseEvent::Metadata(loaded))) => {
                *metadata = MetadataLoad::Ready(loaded.clone());
                Ok(Some(loaded))
            }
            Some(Ok(other)) => Err(format!(
                "leaderboard metadata request returned the wrong event: {other:?}"
            )),
            Some(Err(error)) => Err(error.to_string()),
        },
        MetadataLoad::Ready(metadata) => Ok(Some(metadata.clone())),
        MetadataLoad::Failed(error) => Err(error.clone()),
    }
}

/// Host-side coordinator for the only ranked multiplayer upload. Remote
/// peers receive a typed, locally checkable context followed by the fixed
/// purpose-bound request; the authenticated transport stamps the responding
/// seat and rejects request replay before this task sees it.
struct MultiplayerHostSubmissionAuthorizer {
    port: crate::multiplayer::RankedMultiplayerPort,
    notification_participants: Vec<robin_run_protocol::PublicKey32>,
    campaign_controller: Option<robin_run_protocol::PublicKey32>,
}

impl MultiplayerHostSubmissionAuthorizer {
    fn new(port: crate::multiplayer::RankedMultiplayerPort) -> Result<Self, String> {
        if port.role() != crate::multiplayer::RankedMultiplayerRole::Host
            || port.local_seat() != robin_engine::player_command::PlayerId::HOST
        {
            return Err(
                "multiplayer submission authorizer requires the authenticated host port".to_owned(),
            );
        }
        Ok(Self {
            port,
            notification_participants: Vec::new(),
            campaign_controller: None,
        })
    }

    fn host_public_key(&self) -> Result<robin_run_protocol::PublicKey32, String> {
        let lifecycle = self.port.lifecycle();
        Ok(lifecycle
            .lock()
            .map_err(|_| "ranked host lifecycle lock is poisoned".to_owned())?
            .ranked_session()
            .ok_or_else(|| {
                "ranked host lifecycle ended before submission acknowledgement".to_owned()
            })?
            .genesis()
            .claim
            .host_public_key)
    }
}

impl MissionEndSubmissionAuthorizer for MultiplayerHostSubmissionAuthorizer {
    fn begin(
        &mut self,
        request: SubmissionAuthorizationRequest,
    ) -> Result<Box<dyn SubmissionAuthorizationTask>, String> {
        let host_key = self.host_public_key()?;
        self.notification_participants = request
            .offer_request
            .participant_claims
            .iter()
            .filter(|claim| claim.public_key != host_key)
            .map(|claim| claim.public_key)
            .collect();
        self.campaign_controller = match request.offer_request.scope_request {
            ScopeRequestV1::IndividualLevel => None,
            ScopeRequestV1::CampaignGenesis => request
                .offer_request
                .participant_claims
                .iter()
                .find(|claim| claim.seat == 0)
                .map(|claim| claim.public_key),
            ScopeRequestV1::CampaignContinuation { .. } => request.campaign_controller_public_key,
        };
        Ok(Box::new(MultiplayerHostAuthorizationTask::begin(
            self.port.clone(),
            request,
        )?))
    }

    fn submission_accepted(
        &mut self,
        accepted: &robin_run_protocol::SubmissionAcceptedV1,
    ) -> Result<(), String> {
        for participant in self.notification_participants.iter().copied() {
            self.port
                .host_publish_submission_accepted(participant, accepted.clone())?;
        }
        Ok(())
    }

    fn owns_receipt_watch(&self) -> Result<bool, String> {
        match self.campaign_controller {
            None => Ok(true),
            Some(controller) => Ok(controller == self.host_public_key()?),
        }
    }
}

enum MultiplayerHostAuthorizationPhase {
    AwaitingLocalContinuation,
    AwaitingContinuation {
        claim: robin_run_protocol::CampaignContinuationAuthorizationClaimV1,
        controller_key: robin_run_protocol::PublicKey32,
        request_instance: robin_run_protocol::LeaderboardCoSignInstanceV1,
    },
    AwaitingLocalSubmission {
        envelope: robin_run_protocol::SubmissionEnvelopeV1,
    },
    AwaitingSubmission {
        envelope: robin_run_protocol::SubmissionEnvelopeV1,
        pending: BTreeMap<
            robin_run_protocol::PublicKey32,
            (
                robin_engine::player_command::PlayerId,
                robin_run_protocol::LeaderboardCoSignInstanceV1,
            ),
        >,
    },
    Finished,
}

struct MultiplayerHostAuthorizationTask {
    port: crate::multiplayer::RankedMultiplayerPort,
    request: SubmissionAuthorizationRequest,
    expected: Vec<robin_run_protocol::PublicKey32>,
    signatures: BTreeMap<robin_run_protocol::PublicKey32, ParticipantSignatureV1>,
    local_signature_task: Option<Box<dyn MissionEndTask<HostLocalSignature>>>,
    phase: MultiplayerHostAuthorizationPhase,
    result: Option<Result<SignedSubmissionV1, String>>,
}

impl MultiplayerHostAuthorizationTask {
    fn begin(
        port: crate::multiplayer::RankedMultiplayerPort,
        request: SubmissionAuthorizationRequest,
    ) -> Result<Self, String> {
        request
            .validate_exact_context()
            .map_err(|error| error.to_string())?;
        let expected = request.expected_participants();
        let host_key = {
            let shared_lifecycle = port.lifecycle();
            let lifecycle = shared_lifecycle
                .lock()
                .map_err(|_| "ranked session lifecycle lock is poisoned".to_owned())?;
            lifecycle
                .ranked_session()
                .ok_or_else(|| "ranked host lifecycle is no longer eligible".to_owned())?
                .genesis()
                .claim
                .host_public_key
        };
        if expected.binary_search(&host_key).is_err() {
            return Err("ranked host identity is absent from the final participant set".to_owned());
        }
        let mut task = Self {
            port,
            request,
            expected,
            signatures: BTreeMap::new(),
            local_signature_task: None,
            phase: MultiplayerHostAuthorizationPhase::Finished,
            result: None,
        };
        match task
            .request
            .continuation_claim()
            .map_err(|error| error.to_string())?
        {
            Some(claim) if claim.campaign_controller_public_key == host_key => {
                task.local_signature_task = Some(start_host_continuation_signature_task(
                    task.request.offer.clone(),
                    claim,
                )?);
                task.phase = MultiplayerHostAuthorizationPhase::AwaitingLocalContinuation;
            }
            Some(claim) => {
                let controller_key = claim.campaign_controller_public_key;
                let controller_seat = task.participant_seat(controller_key)?;
                let context =
                    crate::leaderboard_ranked_session::RankedCoSignContextV1::CampaignContinuation(
                        crate::leaderboard_ranked_session::RankedContinuationContextV1 {
                            offer_request: task.request.offer_request.clone(),
                            offer: task.request.offer.clone(),
                            continuation_claim: claim.clone(),
                        },
                    );
                let request = task
                    .port
                    .host_publish_co_sign_operation(controller_seat, &context)?;
                task.phase = MultiplayerHostAuthorizationPhase::AwaitingContinuation {
                    claim,
                    controller_key,
                    request_instance: request.instance,
                };
            }
            None => task.begin_submission(None)?,
        }
        Ok(task)
    }

    fn participant_seat(
        &self,
        key: robin_run_protocol::PublicKey32,
    ) -> Result<robin_engine::player_command::PlayerId, String> {
        let mut claims = self
            .request
            .offer_request
            .participant_claims
            .iter()
            .filter(|claim| claim.public_key == key);
        let claim = claims
            .next()
            .ok_or_else(|| "co-sign identity is absent from the authenticated roster".to_owned())?;
        if claims.next().is_some() {
            return Err("co-sign identity owns multiple authenticated seats".to_owned());
        }
        Ok(robin_engine::player_command::PlayerId(
            u8::try_from(claim.seat)
                .map_err(|_| "ranked participant seat exceeds the game wire range".to_owned())?,
        ))
    }

    fn begin_submission(
        &mut self,
        continuation: Option<CampaignContinuationAuthorizationV1>,
    ) -> Result<(), String> {
        let envelope = self.request.envelope(continuation);
        envelope.validate().map_err(|error| error.to_string())?;
        self.local_signature_task = Some(start_host_submission_signature_task(envelope.clone())?);
        self.phase = MultiplayerHostAuthorizationPhase::AwaitingLocalSubmission { envelope };
        Ok(())
    }

    fn publish_submission(
        &mut self,
        envelope: robin_run_protocol::SubmissionEnvelopeV1,
        local: ParticipantSignatureV1,
    ) -> Result<(), String> {
        self.signatures.insert(local.public_key, local);
        let context = crate::leaderboard_ranked_session::RankedCoSignContextV1::Submission(
            crate::leaderboard_ranked_session::RankedSubmissionContextV1 {
                offer_request: self.request.offer_request.clone(),
                replay_session_transcript: self.request.replay_session_transcript.clone(),
                submission: envelope.clone(),
            },
        );
        let remote = self
            .request
            .offer_request
            .participant_claims
            .iter()
            .filter(|claim| claim.seat != 0)
            .map(|claim| (claim.public_key, claim.seat))
            .collect::<Vec<_>>();
        let mut pending = BTreeMap::new();
        for (key, seat) in remote {
            let seat =
                robin_engine::player_command::PlayerId(u8::try_from(seat).map_err(|_| {
                    "ranked participant seat exceeds the game wire range".to_owned()
                })?);
            let request = self.port.host_publish_co_sign_operation(seat, &context)?;
            if pending.insert(key, (seat, request.instance)).is_some() {
                return Err("ranked participant identity appears more than once".to_owned());
            }
        }
        self.phase = MultiplayerHostAuthorizationPhase::AwaitingSubmission { envelope, pending };
        self.finish_if_complete()
    }

    fn poll_local_signature(&mut self) -> Result<(), String> {
        let Some(result) = self
            .local_signature_task
            .as_mut()
            .and_then(|task| task.try_take())
        else {
            return Ok(());
        };
        self.local_signature_task = None;
        match (
            std::mem::replace(&mut self.phase, MultiplayerHostAuthorizationPhase::Finished),
            result?,
        ) {
            (
                MultiplayerHostAuthorizationPhase::AwaitingLocalContinuation,
                HostLocalSignature::Continuation(authorization),
            ) => self.begin_submission(Some(authorization)),
            (
                MultiplayerHostAuthorizationPhase::AwaitingLocalSubmission { envelope },
                HostLocalSignature::Submission(signature),
            ) => self.publish_submission(envelope, signature),
            (phase, _) => {
                self.phase = phase;
                Err("durable host signer returned the wrong closed ranked operation".to_owned())
            }
        }
    }

    fn finish_if_complete(&mut self) -> Result<(), String> {
        let MultiplayerHostAuthorizationPhase::AwaitingSubmission { envelope, pending } =
            &self.phase
        else {
            return Ok(());
        };
        if !pending.is_empty() {
            return Ok(());
        }
        let signed = SignedSubmissionV1 {
            schema_version: SCHEMA_VERSION_V1,
            submission: envelope.clone(),
            algorithm: SignatureAlgorithmV1::Ed25519,
            participant_signatures: self.signatures.values().cloned().collect(),
        };
        crate::leaderboard_mission_end::validate_authorized_submission(&self.request, &signed)
            .map_err(|error| error.to_string())?;
        self.phase = MultiplayerHostAuthorizationPhase::Finished;
        self.result = Some(Ok(signed));
        Ok(())
    }

    fn accept_response(
        &mut self,
        from: robin_engine::player_command::PlayerId,
        response: robin_engine::multiplayer::LeaderboardCoSignResponse,
    ) -> Result<(), String> {
        let key = robin_run_protocol::PublicKey32::from_bytes(response.signer_public_key);
        let continuation_seat = match &self.phase {
            MultiplayerHostAuthorizationPhase::AwaitingContinuation { controller_key, .. } => {
                Some(self.participant_seat(*controller_key)?)
            }
            _ => None,
        };
        match &mut self.phase {
            MultiplayerHostAuthorizationPhase::AwaitingContinuation {
                claim,
                controller_key,
                request_instance,
            } => {
                if Some(from) != continuation_seat
                    || key != *controller_key
                    || response.instance != *request_instance
                {
                    return Err("campaign continuation co-sign response changed its authenticated target or request".to_owned());
                }
                let authorization = CampaignContinuationAuthorizationV1 {
                    claim: claim.clone(),
                    algorithm: SignatureAlgorithmV1::Ed25519,
                    signature: Signature64::from_bytes(response.signature),
                };
                self.begin_submission(Some(authorization))
            }
            MultiplayerHostAuthorizationPhase::AwaitingSubmission { pending, .. } => {
                let Some((expected_seat, expected_instance)) = pending.get(&key).copied() else {
                    return Err(
                        "unexpected or duplicate ranked participant co-sign response".to_owned(),
                    );
                };
                if from != expected_seat || response.instance != expected_instance {
                    return Err("ranked participant co-sign response changed its authenticated seat or request".to_owned());
                }
                pending.remove(&key);
                self.signatures.insert(
                    key,
                    ParticipantSignatureV1 {
                        public_key: key,
                        signature: Signature64::from_bytes(response.signature),
                    },
                );
                self.finish_if_complete()
            }
            MultiplayerHostAuthorizationPhase::AwaitingLocalContinuation
            | MultiplayerHostAuthorizationPhase::AwaitingLocalSubmission { .. } => Err(
                "ranked co-sign response arrived while the durable host signer was active"
                    .to_owned(),
            ),
            MultiplayerHostAuthorizationPhase::Finished => {
                Err("ranked co-sign response arrived after authorization completed".to_owned())
            }
        }
    }
}

impl MissionEndTask<SignedSubmissionV1> for MultiplayerHostAuthorizationTask {
    fn try_take(&mut self) -> Option<Result<SignedSubmissionV1, String>> {
        if self.result.is_some() {
            return self.result.take();
        }
        if let Err(error) = self.poll_local_signature() {
            return Some(Err(error));
        }
        for _ in 0..64 {
            let event = match self.port.try_recv_authorization_event() {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(error) => return Some(Err(error)),
            };
            match event {
                crate::multiplayer::RankedAuthorizationEvent::CoSignResponse { from, response } => {
                    if let Err(error) = self.accept_response(from, response) {
                        return Some(Err(error));
                    }
                }
                _ => {
                    return Some(Err(
                        "ranked host received a client-only authorization event".to_owned(),
                    ));
                }
            }
        }
        self.result.take()
    }
}

impl SubmissionAuthorizationTask for MultiplayerHostAuthorizationTask {
    fn progress(&self) -> ParticipantSigningProgress {
        ParticipantSigningProgress {
            expected: self.expected.clone(),
            signed: self.signatures.keys().copied().collect(),
        }
    }
}

enum HostLocalSignature {
    Continuation(CampaignContinuationAuthorizationV1),
    Submission(ParticipantSignatureV1),
}

#[cfg(not(target_arch = "wasm32"))]
struct ImmediateHostSignatureTask(Option<Result<HostLocalSignature, String>>);

#[cfg(not(target_arch = "wasm32"))]
impl MissionEndTask<HostLocalSignature> for ImmediateHostSignatureTask {
    fn try_take(&mut self) -> Option<Result<HostLocalSignature, String>> {
        self.0.take()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn start_host_continuation_signature_task(
    offer: robin_run_protocol::SubmissionOfferV1,
    claim: robin_run_protocol::CampaignContinuationAuthorizationClaimV1,
) -> Result<Box<dyn MissionEndTask<HostLocalSignature>>, String> {
    Ok(Box::new(ImmediateHostSignatureTask(Some(
        crate::leaderboard_signing::sign_campaign_continuation(&offer, claim)
            .map(HostLocalSignature::Continuation)
            .map_err(|error| error.to_string()),
    ))))
}

#[cfg(not(target_arch = "wasm32"))]
fn start_host_submission_signature_task(
    envelope: robin_run_protocol::SubmissionEnvelopeV1,
) -> Result<Box<dyn MissionEndTask<HostLocalSignature>>, String> {
    Ok(Box::new(ImmediateHostSignatureTask(Some(
        crate::leaderboard_signing::sign_submission_claim(&envelope)
            .map(HostLocalSignature::Submission)
            .map_err(|error| error.to_string()),
    ))))
}

#[cfg(target_arch = "wasm32")]
struct BrowserHostSignatureTask(async_channel::Receiver<Result<HostLocalSignature, String>>);

#[cfg(target_arch = "wasm32")]
impl MissionEndTask<HostLocalSignature> for BrowserHostSignatureTask {
    fn try_take(&mut self) -> Option<Result<HostLocalSignature, String>> {
        match self.0.try_recv() {
            Ok(result) => Some(result),
            Err(async_channel::TryRecvError::Empty) => None,
            Err(async_channel::TryRecvError::Closed) => {
                Some(Err("browser host signer stopped unexpectedly".to_owned()))
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn start_host_continuation_signature_task(
    offer: robin_run_protocol::SubmissionOfferV1,
    claim: robin_run_protocol::CampaignContinuationAuthorizationClaimV1,
) -> Result<Box<dyn MissionEndTask<HostLocalSignature>>, String> {
    let (sender, receiver) = async_channel::bounded(1);
    wasm_bindgen_futures::spawn_local(async move {
        let result =
            crate::leaderboard_signing::browser_game_sign_campaign_continuation(&offer, &claim)
                .await
                .map(HostLocalSignature::Continuation)
                .map_err(|error| error.to_string());
        let _ = sender.send(result).await;
    });
    Ok(Box::new(BrowserHostSignatureTask(receiver)))
}

#[cfg(target_arch = "wasm32")]
fn start_host_submission_signature_task(
    envelope: robin_run_protocol::SubmissionEnvelopeV1,
) -> Result<Box<dyn MissionEndTask<HostLocalSignature>>, String> {
    let (sender, receiver) = async_channel::bounded(1);
    wasm_bindgen_futures::spawn_local(async move {
        let result = crate::leaderboard_signing::browser_game_sign_submission_claim(&envelope)
            .await
            .map(HostLocalSignature::Submission)
            .map_err(|error| error.to_string());
        let _ = sender.send(result).await;
    });
    Ok(Box::new(BrowserHostSignatureTask(receiver)))
}

struct MultiplayerPeerCoSigner {
    port: crate::multiplayer::RankedMultiplayerPort,
    client: crate::leaderboard_ranked_session::RankedSessionClientV1,
    scope_request: ScopeRequestV1,
    requested_metrics: Vec<BoardMetricV1>,
    campaign_controller_public_key: Option<robin_run_protocol::PublicKey32>,
    mission_id: String,
    starting_campaign_bytes: Arc<[u8]>,
    consented: bool,
    replay_task: Option<Box<dyn MissionEndTask<Arc<[u8]>>>>,
    replay_bytes: Option<Arc<[u8]>>,
    local_replay: Option<robin_engine::replay::ReplayData>,
    armed_request: Option<robin_run_protocol::LeaderboardCoSignRequestV1>,
    signature_task: Option<Box<dyn MissionEndTask<ParticipantSignatureV1>>>,
    final_response_sent: bool,
    accepted: Option<robin_run_protocol::SubmissionAcceptedV1>,
    failure: Option<String>,
}

impl MultiplayerPeerCoSigner {
    fn new(
        port: crate::multiplayer::RankedMultiplayerPort,
        signed: &SignedRankedMissionAdmission,
        mission_id: String,
        starting_campaign_bytes: Arc<[u8]>,
    ) -> Result<Self, String> {
        if port.role() != crate::multiplayer::RankedMultiplayerRole::Client
            || port.local_seat() == robin_engine::player_command::PlayerId::HOST
        {
            return Err("ranked peer co-signer requires an authenticated client port".to_owned());
        }
        let client = signed
            .lifecycle
            .lock()
            .map_err(|_| "ranked client lifecycle lock is poisoned".to_owned())?
            .ranked_client()
            .cloned()
            .ok_or_else(|| {
                "ranked client admission did not finish before mission runtime".to_owned()
            })?;
        client.validate().map_err(|error| error.to_string())?;
        if client.local_seat != u16::from(port.local_seat().0)
            || client.session_genesis.claim.ranked_session.mission_id != mission_id
        {
            return Err(
                "ranked client lifecycle differs from the active mission seat or mission"
                    .to_owned(),
            );
        }
        Ok(Self {
            port,
            client,
            scope_request: signed.scope_request.clone(),
            requested_metrics: signed.requested_metrics.clone(),
            campaign_controller_public_key: signed.campaign_controller_public_key,
            mission_id,
            starting_campaign_bytes,
            consented: false,
            replay_task: None,
            replay_bytes: None,
            local_replay: None,
            armed_request: None,
            signature_task: None,
            final_response_sent: false,
            accepted: None,
            failure: None,
        })
    }

    fn local_public_key(&self) -> robin_run_protocol::PublicKey32 {
        self.client.admission.local_public_key
    }

    fn prepare_replay(&mut self) -> Result<bool, String> {
        if self.replay_bytes.is_some() && self.local_replay.is_some() {
            return Ok(true);
        }
        if self.replay_task.is_none() {
            self.replay_task = Some(ActiveMissionReplayExporter.begin()?);
        }
        let Some(result) = self.replay_task.as_mut().and_then(|task| task.try_take()) else {
            return Ok(false);
        };
        self.replay_task = None;
        let bytes = result?;
        let replay = crate::http_server::active_replay_snapshot()?.parse_sync()?;
        self.replay_bytes = Some(bytes);
        self.local_replay = Some(replay);
        Ok(true)
    }

    fn validate_offer_request(&self, request: &SubmissionOfferRequestV1) -> Result<(), String> {
        request.validate().map_err(|error| error.to_string())?;
        if request.session_genesis != self.client.session_genesis
            || request.participant_claims != self.client.participant_claims
            || request.mission_id != self.mission_id
            || request.scope_request != self.scope_request
        {
            return Err(
                "host co-sign context differs from the client's admitted session, roster, mission, or campaign scope"
                    .to_owned(),
            );
        }
        Ok(())
    }

    fn arm_context(
        &mut self,
        context: crate::leaderboard_ranked_session::RankedCoSignContextV1,
    ) -> Result<(), String> {
        if self.armed_request.is_some() || self.signature_task.is_some() {
            return Err("host sent overlapping ranked co-sign operations".to_owned());
        }
        let request = match context {
            crate::leaderboard_ranked_session::RankedCoSignContextV1::CampaignContinuation(
                context,
            ) => {
                self.validate_offer_request(&context.offer_request)?;
                let local_key = self.local_public_key();
                if self.campaign_controller_public_key != Some(local_key) {
                    return Err(
                        "host targeted a non-controller peer for campaign continuation authorization"
                            .to_owned(),
                    );
                }
                let expected =
                    crate::leaderboard_ranked_session::RankedLocalContinuationEvidenceV1 {
                        offer_request: context.offer_request.clone(),
                        continuation_claim: self.local_continuation_claim(&context)?,
                        local_public_key: local_key,
                    };
                context
                    .validate_and_co_sign_request(&expected)
                    .map_err(|error| error.to_string())?
            }
            crate::leaderboard_ranked_session::RankedCoSignContextV1::Submission(context) => {
                self.validate_offer_request(&context.offer_request)?;
                let replay_bytes = self
                    .replay_bytes
                    .as_ref()
                    .ok_or_else(|| "canonical replay export is not ready".to_owned())?;
                let local_replay = self
                    .local_replay
                    .as_ref()
                    .ok_or_else(|| "local replay evidence is not ready".to_owned())?;
                let replay = crate::leaderboard_mission_end::canonical_replay_artifact(
                    replay_bytes,
                    &self.starting_campaign_bytes,
                    &self.mission_id,
                    &context.replay_session_transcript,
                )
                .map_err(|error| error.to_string())?;
                let campaign = robin_run_protocol::ArtifactRefV1 {
                    sha256: Digest32::digest_bytes(&self.starting_campaign_bytes),
                    byte_length: u64::try_from(self.starting_campaign_bytes.len()).map_err(
                        |_| "starting campaign length exceeds protocol bounds".to_owned(),
                    )?,
                    media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
                };
                let continuation_claim = self.local_submission_continuation_claim(
                    &context,
                    robin_run_protocol::SubmissionArtifactsV1 {
                        replay: replay.clone(),
                        starting_campaign: campaign.clone(),
                    },
                )?;
                let expected = crate::leaderboard_ranked_session::RankedLocalSubmissionEvidenceV1 {
                    offer_request: context.offer_request.clone(),
                    replay_session_transcript: context.replay_session_transcript.clone(),
                    artifacts: robin_run_protocol::SubmissionArtifactsV1 {
                        replay,
                        starting_campaign: campaign,
                    },
                    campaign_aggregation_consent: match self.scope_request {
                        ScopeRequestV1::IndividualLevel => {
                            robin_run_protocol::CampaignAggregationConsentV1::NotAuthorized
                        }
                        ScopeRequestV1::CampaignGenesis
                        | ScopeRequestV1::CampaignContinuation { .. } => robin_run_protocol::CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1,
                    },
                    campaign_continuation_claim: continuation_claim,
                    requested_metrics: self.requested_metrics.clone(),
                    local_public_key: self.local_public_key(),
                };
                context
                    .validate_and_co_sign_request(&expected, local_replay)
                    .map_err(|error| error.to_string())?
            }
        };
        self.port.client_arm_co_sign_request(request)?;
        self.armed_request = Some(request);
        Ok(())
    }

    fn local_continuation_claim(
        &self,
        context: &crate::leaderboard_ranked_session::RankedContinuationContextV1,
    ) -> Result<robin_run_protocol::CampaignContinuationAuthorizationClaimV1, String> {
        let ScopeRequestV1::CampaignContinuation {
            chain_id,
            predecessor_run_id,
        } = &self.scope_request
        else {
            return Err(
                "continuation context arrived outside a locally admitted continuation".to_owned(),
            );
        };
        let claim = &context.continuation_claim;
        if &claim.chain_id != chain_id
            || &claim.predecessor_run_id != predecessor_run_id
            || Some(claim.campaign_controller_public_key) != self.campaign_controller_public_key
        {
            return Err(
                "continuation context differs from the locally retained chain receipt".to_owned(),
            );
        }
        Ok(claim.clone())
    }

    fn local_submission_continuation_claim(
        &self,
        context: &crate::leaderboard_ranked_session::RankedSubmissionContextV1,
        artifacts: robin_run_protocol::SubmissionArtifactsV1,
    ) -> Result<Option<robin_run_protocol::CampaignContinuationAuthorizationClaimV1>, String> {
        let Some(authorization) = &context.submission.campaign_continuation_authorization else {
            if matches!(
                self.scope_request,
                ScopeRequestV1::CampaignContinuation { .. }
            ) {
                return Err("host omitted the locally required campaign continuation".to_owned());
            }
            return Ok(None);
        };
        let claim = &authorization.claim;
        let ScopeRequestV1::CampaignContinuation {
            chain_id,
            predecessor_run_id,
        } = &self.scope_request
        else {
            return Err("host inserted campaign continuation into another local scope".to_owned());
        };
        if &claim.chain_id != chain_id
            || &claim.predecessor_run_id != predecessor_run_id
            || Some(claim.campaign_controller_public_key) != self.campaign_controller_public_key
            || claim.next_artifacts != artifacts
        {
            return Err(
                "host continuation authorization differs from local chain/artifact evidence"
                    .to_owned(),
            );
        }
        Ok(Some(claim.clone()))
    }

    fn poll_inner(&mut self) -> Result<(), String> {
        if !self.prepare_replay()? {
            return Ok(());
        }
        if let Some(task) = self.signature_task.as_mut() {
            let Some(signature) = task.try_take() else {
                return Ok(());
            };
            self.signature_task = None;
            let signature = signature?;
            let request = self
                .armed_request
                .take()
                .ok_or_else(|| "co-signature completed without an armed request".to_owned())?;
            if signature.public_key != self.local_public_key() {
                return Err("durable signer returned another participant identity".to_owned());
            }
            self.port.client_respond_co_sign(
                robin_engine::multiplayer::LeaderboardCoSignResponse {
                    instance: request.instance,
                    signer_public_key: *signature.public_key.as_bytes(),
                    signature: *signature.signature.as_bytes(),
                },
            )?;
            self.final_response_sent = matches!(
                request.instance.purpose,
                robin_run_protocol::LeaderboardCoSignPurposeV1::Submission
            );
        }
        for _ in 0..64 {
            let Some(event) = self.port.try_recv_authorization_event()? else {
                break;
            };
            match event {
                crate::multiplayer::RankedAuthorizationEvent::CoSignContext(context) => {
                    self.arm_context(context)?;
                }
                crate::multiplayer::RankedAuthorizationEvent::CoSignRequest(request) => {
                    if self.armed_request.as_ref() != Some(&request) {
                        return Err("transport released a co-sign request other than the locally armed request".to_owned());
                    }
                    self.signature_task = Some(start_peer_signature_task(request)?);
                }
                crate::multiplayer::RankedAuthorizationEvent::SubmissionAccepted(accepted) => {
                    accepted.validate().map_err(|error| error.to_string())?;
                    self.accepted = Some(accepted);
                }
                crate::multiplayer::RankedAuthorizationEvent::CoSignResponse { .. } => {
                    return Err("ranked client received a host-only co-sign response".to_owned());
                }
                crate::multiplayer::RankedAuthorizationEvent::OfficialSessionSetup(_)
                | crate::multiplayer::RankedAuthorizationEvent::ContinuationReceiptSelectionRequest(_)
                | crate::multiplayer::RankedAuthorizationEvent::ContinuationReceiptSelectionResponse { .. }
                | crate::multiplayer::RankedAuthorizationEvent::ContinuationPreflightClaim(_)
                | crate::multiplayer::RankedAuthorizationEvent::ContinuationPreflightSignature { .. } => {
                    return Err(
                        "pre-frame ranked authorization event arrived at the mission-end co-signer"
                            .to_owned(),
                    );
                }
            }
        }
        Ok(())
    }
}

impl MissionEndPeerCoSigner for MultiplayerPeerCoSigner {
    fn consent(&mut self) -> Result<(), String> {
        if self.consented {
            return Err("ranked peer consent was already recorded".to_owned());
        }
        self.consented = true;
        Ok(())
    }

    fn poll(&mut self) -> PeerCoSignPoll {
        if let Some(error) = &self.failure {
            return PeerCoSignPoll::Failed(error.clone());
        }
        if !self.consented {
            return PeerCoSignPoll::AwaitingConsent;
        }
        if let Err(error) = self.poll_inner() {
            self.failure = Some(error.clone());
            return PeerCoSignPoll::Failed(error);
        }
        if let Some(accepted) = &self.accepted {
            return PeerCoSignPoll::Accepted(accepted.clone());
        }
        let expected = vec![self.local_public_key()];
        if self.signature_task.is_some() {
            return PeerCoSignPoll::Signing(ParticipantSigningProgress {
                expected,
                signed: Vec::new(),
            });
        }
        if self.final_response_sent {
            return PeerCoSignPoll::ResponseSent(ParticipantSigningProgress {
                expected: expected.clone(),
                signed: expected,
            });
        }
        PeerCoSignPoll::AwaitingHost
    }

    fn has_pending_work(&self) -> bool {
        self.consented && self.failure.is_none() && self.accepted.is_none()
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct ImmediatePeerSignatureTask(Option<Result<ParticipantSignatureV1, String>>);

#[cfg(not(target_arch = "wasm32"))]
impl MissionEndTask<ParticipantSignatureV1> for ImmediatePeerSignatureTask {
    fn try_take(&mut self) -> Option<Result<ParticipantSignatureV1, String>> {
        self.0.take()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn start_peer_signature_task(
    request: robin_run_protocol::LeaderboardCoSignRequestV1,
) -> Result<Box<dyn MissionEndTask<ParticipantSignatureV1>>, String> {
    Ok(Box::new(ImmediatePeerSignatureTask(Some(
        crate::leaderboard_signing::sign_multiplayer_leaderboard_request(&request)
            .map_err(|error| error.to_string()),
    ))))
}

#[cfg(target_arch = "wasm32")]
struct BrowserPeerSignatureTask(async_channel::Receiver<Result<ParticipantSignatureV1, String>>);

#[cfg(target_arch = "wasm32")]
impl MissionEndTask<ParticipantSignatureV1> for BrowserPeerSignatureTask {
    fn try_take(&mut self) -> Option<Result<ParticipantSignatureV1, String>> {
        match self.0.try_recv() {
            Ok(result) => Some(result),
            Err(async_channel::TryRecvError::Empty) => None,
            Err(async_channel::TryRecvError::Closed) => {
                Some(Err("browser peer signer stopped unexpectedly".to_owned()))
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn start_peer_signature_task(
    request: robin_run_protocol::LeaderboardCoSignRequestV1,
) -> Result<Box<dyn MissionEndTask<ParticipantSignatureV1>>, String> {
    let (sender, receiver) = async_channel::bounded(1);
    wasm_bindgen_futures::spawn_local(async move {
        let result =
            crate::leaderboard_signing::browser_game_sign_multiplayer_leaderboard_request(&request)
                .await
                .map_err(|error| error.to_string());
        let _ = sender.send(result).await;
    });
    Ok(Box::new(BrowserPeerSignatureTask(receiver)))
}

/// Cooperative presentation/background state installed only after the final
/// authoritative debrief decision has been recorded. The first poll therefore
/// happens one outer frame after that decision and cannot export a replay that
/// is missing its terminal record.
pub(super) struct MissionEndLeaderboardTaskState {
    phase: MissionEndLeaderboardTaskPhase,
    application_context: crate::host::ApplicationContext,
}

enum MissionEndLeaderboardTaskPhase {
    Preparing(MissionEndPreparation),
    Visible(MissionEndLeaderboardScreen),
    Finished,
}

pub(super) enum MissionEndLeaderboardTaskProgress {
    Pending,
    Finished,
    Detach(MissionEndLeaderboardController),
}

impl MissionEndLeaderboardTaskState {
    pub(super) fn new(
        preparation: MissionEndPreparation,
        application_context: crate::host::ApplicationContext,
    ) -> Self {
        Self {
            phase: MissionEndLeaderboardTaskPhase::Preparing(preparation),
            application_context,
        }
    }

    pub(super) fn owns_presentation(&self) -> bool {
        matches!(
            &self.phase,
            MissionEndLeaderboardTaskPhase::Preparing(_)
                | MissionEndLeaderboardTaskPhase::Visible(_)
        )
    }

    pub(super) fn tick(
        &mut self,
        window: &mut GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) -> MissionEndLeaderboardTaskProgress {
        match &mut self.phase {
            MissionEndLeaderboardTaskPhase::Preparing(preparation) => {
                let Some(result) = preparation.poll_bundle() else {
                    render_preparing(renderer, resources, cursor);
                    return MissionEndLeaderboardTaskProgress::Pending;
                };
                let bundle = match result {
                    Ok(bundle) => bundle,
                    Err(error) => {
                        tracing::warn!("mission-end leaderboards unavailable: {error}");
                        self.phase = MissionEndLeaderboardTaskPhase::Finished;
                        return MissionEndLeaderboardTaskProgress::Finished;
                    }
                };
                let preferences = preparation.preferences().clone();
                let api = match LeaderboardApi::from_preferences(&preferences) {
                    Ok(api) => api,
                    Err(error) => {
                        tracing::warn!("mission-end leaderboard endpoint unavailable: {error}");
                        self.phase = MissionEndLeaderboardTaskPhase::Finished;
                        return MissionEndLeaderboardTaskProgress::Finished;
                    }
                };
                let peer_co_signer = if preparation.ranked_multiplayer_port.as_ref().is_some_and(
                    |port| port.role() == crate::multiplayer::RankedMultiplayerRole::Client,
                ) {
                    let Some(port) = preparation.ranked_multiplayer_port.take() else {
                        unreachable!("ranked client port was present")
                    };
                    let RankedMissionAdmission::Signed(signed) = &preparation.admission else {
                        tracing::warn!(
                            "ranked client port reached mission end without retained signed admission"
                        );
                        self.phase = MissionEndLeaderboardTaskPhase::Finished;
                        return MissionEndLeaderboardTaskProgress::Finished;
                    };
                    match MultiplayerPeerCoSigner::new(
                        port,
                        signed,
                        preparation.mission_id.clone(),
                        preparation.starting_campaign_bytes.clone(),
                    ) {
                        Ok(peer) => {
                            let receipt_controller_public_key = peer
                                .campaign_controller_public_key
                                .filter(|controller| *controller == peer.local_public_key());
                            Some((
                                Box::new(peer) as Box<dyn MissionEndPeerCoSigner>,
                                receipt_controller_public_key,
                            ))
                        }
                        Err(error) => {
                            tracing::warn!("ranked peer co-signing unavailable: {error}");
                            self.phase = MissionEndLeaderboardTaskPhase::Finished;
                            return MissionEndLeaderboardTaskProgress::Finished;
                        }
                    }
                } else {
                    None
                };
                let authorizer: Box<dyn MissionEndSubmissionAuthorizer> = if bundle.multiplayer
                    && bundle.eligible_submission.is_some()
                {
                    let Some(port) = preparation.ranked_multiplayer_port.take() else {
                        tracing::error!(
                            "ranked multiplayer admission reached mission end without its authenticated authorization port"
                        );
                        self.phase = MissionEndLeaderboardTaskPhase::Finished;
                        return MissionEndLeaderboardTaskProgress::Finished;
                    };
                    match MultiplayerHostSubmissionAuthorizer::new(port) {
                        Ok(authorizer) => Box::new(authorizer),
                        Err(error) => {
                            tracing::error!("ranked multiplayer authorizer unavailable: {error}");
                            self.phase = MissionEndLeaderboardTaskPhase::Finished;
                            return MissionEndLeaderboardTaskProgress::Finished;
                        }
                    }
                } else {
                    Box::new(LocalMissionEndSubmissionAuthorizer)
                };
                let controller = if let Some((peer, receipt_controller_public_key)) = peer_co_signer
                {
                    MissionEndLeaderboardController::new_peer(
                        bundle,
                        preferences,
                        Box::new(HttpMissionEndLeaderboardBackend::new(api)),
                        peer,
                        receipt_controller_public_key,
                    )
                } else {
                    MissionEndLeaderboardController::new(
                        bundle,
                        preferences,
                        Box::new(HttpMissionEndLeaderboardBackend::new(api)),
                        authorizer,
                        Box::new(ActiveMissionReplayExporter),
                    )
                };
                let mut controller = match controller {
                    Ok(controller) => controller,
                    Err(error) => {
                        tracing::warn!("mission-end leaderboard setup failed: {error}");
                        self.phase = MissionEndLeaderboardTaskPhase::Finished;
                        return MissionEndLeaderboardTaskProgress::Finished;
                    }
                };
                if controller.is_visible() {
                    self.phase = MissionEndLeaderboardTaskPhase::Visible(
                        MissionEndLeaderboardScreen::new(controller, resources),
                    );
                    // Do not poll the freshly-created controller a second time
                    // in this host frame.
                    MissionEndLeaderboardTaskProgress::Pending
                } else {
                    controller
                        .apply_action(MissionEndLeaderboardAction::Close)
                        .unwrap_or_else(|error| {
                            panic!("validated hidden leaderboard could not close: {error}")
                        });
                    self.phase = MissionEndLeaderboardTaskPhase::Finished;
                    retire_or_detach(controller)
                }
            }
            MissionEndLeaderboardTaskPhase::Visible(screen) => {
                let event = screen.tick(window, renderer, resources, cursor);
                if let Err(error) = screen
                    .controller_mut()
                    .persist_queued_receipt_watch(&self.application_context)
                {
                    tracing::error!(
                        "queued leaderboard verification could not be handed to durable tracking: {error}"
                    );
                }
                if event != Some(MissionEndLeaderboardEvent::Closed) {
                    return MissionEndLeaderboardTaskProgress::Pending;
                }
                let MissionEndLeaderboardTaskPhase::Visible(screen) =
                    std::mem::replace(&mut self.phase, MissionEndLeaderboardTaskPhase::Finished)
                else {
                    unreachable!()
                };
                let controller = screen.into_controller();
                retire_or_detach(controller)
            }
            MissionEndLeaderboardTaskPhase::Finished => MissionEndLeaderboardTaskProgress::Finished,
        }
    }
}

fn retire_or_detach(
    controller: MissionEndLeaderboardController,
) -> MissionEndLeaderboardTaskProgress {
    if controller.can_retire_after_close() {
        MissionEndLeaderboardTaskProgress::Finished
    } else {
        MissionEndLeaderboardTaskProgress::Detach(controller)
    }
}

fn render_preparing(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<&ModalCursor<'_>>,
) {
    enter_modal_gpu_phase(renderer);
    dim_screen(renderer);
    if let Some(background) = resources.menu_bg[0] {
        draw_screen_background(renderer, &background);
    }
    if let Some(font) = resources.title_font_any() {
        let transform = MenuTransform::centered(
            i32::from(renderer.screen_width()),
            i32::from(renderer.screen_height()),
        );
        let text = "Loading verified leaderboards...";
        render_text_virt_font(
            renderer,
            font,
            transform,
            text,
            (crate::ingame_menu::layout::MENU_W - font.text_width(text)) / 2,
            220,
        );
        if let Some(cursor) = cursor {
            // The preparation page has no controls, but retaining the current
            // cursor avoids a visible jump when the board becomes ready.
            cursor.draw(
                renderer,
                transform,
                &crate::ingame_menu::widget_bridge::ModalInputState::default(),
            );
        }
    }
    renderer.present();
}

fn build_bundle(
    mission_id: &str,
    multiplayer: bool,
    starting_campaign_bytes: Arc<[u8]>,
    outcome: MissionEndOutcome,
    admission: &RankedMissionAdmission,
    preferences: &LeaderboardPreferences,
    metadata: Option<&LeaderboardMetadataV1>,
) -> Result<MissionEndRunBundle, String> {
    if let Some(metadata) = metadata {
        metadata
            .validate()
            .map_err(|error| format!("leaderboard metadata is invalid: {error}"))?;
    }
    let (boards, eligible_submission, unavailable_reason) = match admission {
        RankedMissionAdmission::Authorized(input) => {
            if input.offer_request.mission_id != mission_id {
                return Err(format!(
                    "ranked admission mission `{}` does not match loaded mission `{mission_id}`",
                    input.offer_request.mission_id
                ));
            }
            if input.starting_campaign_bytes.as_ref() != starting_campaign_bytes.as_ref() {
                return Err(
                    "ranked admission starting campaign differs from the exact engine input"
                        .to_owned(),
                );
            }
            let boards = authorized_boards(input, metadata)?;
            if outcome.can_submit() {
                (boards, Some(input.clone()), None)
            } else {
                (
                    boards,
                    None,
                    Some("only won missions can be submitted".to_owned()),
                )
            }
        }
        RankedMissionAdmission::BrowseOnly { reason } => {
            let metadata = metadata.ok_or_else(|| {
                "leaderboard metadata is required to browse a run without ranked admission"
                    .to_owned()
            })?;
            (
                browse_boards(mission_id, multiplayer, preferences, metadata)?,
                None,
                Some(reason.clone()),
            )
        }
        RankedMissionAdmission::Signed(signed) => {
            (
                signed_admission_boards(mission_id, signed)?,
                None,
                Some(
                    "this peer retained ranked evidence; the host controls submission and requests each participant's co-signature"
                        .to_owned(),
                ),
            )
        }
    };
    let bundle = MissionEndRunBundle {
        outcome,
        multiplayer,
        boards,
        eligible_submission,
        submission_unavailable_reason: unavailable_reason,
    };
    bundle.validate().map_err(|error| error.to_string())?;
    // Cross-check the immutable campaign bytes even for browse-only sessions.
    // This catches accidental replacement of the bootstrap capture before a
    // future admission implementation can make the run eligible.
    if starting_campaign_bytes.is_empty() {
        return Err("captured starting campaign is empty".to_owned());
    }
    Ok(bundle)
}

fn signed_admission_boards(
    mission_id: &str,
    signed: &SignedRankedMissionAdmission,
) -> Result<Vec<MissionEndBoard>, String> {
    let lifecycle = signed
        .lifecycle
        .lock()
        .map_err(|_| "ranked multiplayer lifecycle lock is poisoned".to_owned())?;
    let config = if let Some(host) = lifecycle.ranked_session() {
        &host.genesis().claim.ranked_session
    } else if let Some(client) = lifecycle.ranked_client() {
        &client.session_genesis.claim.ranked_session
    } else if let Some(reason) = lifecycle.browse_only_reason() {
        return Err(format!("ranked multiplayer was downgraded: {reason}"));
    } else {
        return Err("ranked multiplayer admission is unresolved at mission end".to_owned());
    };
    if config.mission_id != mission_id {
        return Err("ranked multiplayer config names another mission".to_owned());
    }
    let category = match signed.scope_request {
        ScopeRequestV1::IndividualLevel => BoardCategoryV1::IndividualLevel,
        ScopeRequestV1::CampaignGenesis | ScopeRequestV1::CampaignContinuation { .. } => {
            BoardCategoryV1::Campaign
        }
    };
    let boards = metric_boards(
        LeaderboardQuerySubjectV1::Mission,
        Some(mission_id.to_owned()),
        Some(category),
        config.content_manifest_sha256,
        config.rules_config_sha256,
        config.ruleset_manifest_sha256,
        config.competition_manifest_sha256,
        None,
        &signed.requested_metrics,
    );
    if boards.is_empty() {
        return Err("ranked multiplayer admission exposes no supported board metrics".to_owned());
    }
    Ok(boards)
}

fn browse_boards(
    mission_id: &str,
    multiplayer: bool,
    preferences: &LeaderboardPreferences,
    metadata: &LeaderboardMetadataV1,
) -> Result<Vec<MissionEndBoard>, String> {
    let mission = metadata
        .missions
        .iter()
        .find(|mission| mission.mission_id == mission_id)
        .ok_or_else(|| format!("mission `{mission_id}` is not published for leaderboards"))?;
    let category = match preferences.preferred_scope {
        LeaderboardScope::IndividualLevel => BoardCategoryV1::IndividualLevel,
        LeaderboardScope::Campaign => BoardCategoryV1::Campaign,
        LeaderboardScope::FullCampaign => {
            return Err(
                "full-campaign boards require a verified campaign-chain context".to_owned(),
            );
        }
    };
    let ruleset = metadata
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
        .min_by_key(|ruleset| ruleset.ruleset_manifest_sha256)
        .ok_or_else(|| {
            format!(
                "no published {:?} ruleset matches mission `{mission_id}` and the selected board facets",
                preferences.preferred_scope
            )
        })?;
    // The current transport does not expose an authenticated final roster to
    // this layer. A multiplayer browse query therefore leaves player count
    // unfiltered instead of falsely presenting the single-player default as
    // the just-played composition.
    let max_players = (!multiplayer)
        .then_some(preferences.preferred_max_concurrent_players)
        .flatten();
    let subject = LeaderboardQuerySubjectV1::Mission;
    let mut boards = metric_boards(
        subject,
        Some(mission_id.to_owned()),
        Some(category),
        mission.content_manifest_sha256,
        ruleset.rules_config_sha256,
        ruleset.ruleset_manifest_sha256,
        None,
        max_players,
        &ruleset.metrics,
    );
    if let Some(competition) = select_competition(
        metadata,
        mission_id,
        category,
        ruleset.content,
        ruleset.rules_config_sha256,
        ruleset.ruleset_manifest_sha256,
        max_players,
        preferences.preferred_competition_id.as_deref(),
    ) {
        boards.push(MissionEndBoard {
            tab: LeaderboardTab::Challenge,
            label: competition.manifest.display_name.clone(),
            query: query(
                subject,
                Some(mission_id.to_owned()),
                Some(category),
                mission.content_manifest_sha256,
                competition.manifest.rules_config_sha256,
                competition.manifest.ruleset_manifest_sha256,
                Some(competition.competition_manifest_sha256),
                max_players,
                competition.manifest.metric,
            ),
        });
    }
    if boards.is_empty() {
        return Err("selected ruleset publishes no score or time boards".to_owned());
    }
    Ok(boards)
}

fn authorized_boards(
    input: &MissionEndSubmissionInput,
    metadata: Option<&LeaderboardMetadataV1>,
) -> Result<Vec<MissionEndBoard>, String> {
    let request = &input.offer_request;
    let ranked = &request.session_genesis.claim.ranked_session;
    let category = match request.scope_request {
        ScopeRequestV1::IndividualLevel => BoardCategoryV1::IndividualLevel,
        ScopeRequestV1::CampaignGenesis | ScopeRequestV1::CampaignContinuation { .. } => {
            BoardCategoryV1::Campaign
        }
    };
    let mut boards = metric_boards(
        LeaderboardQuerySubjectV1::Mission,
        Some(request.mission_id.clone()),
        Some(category),
        ranked.content_manifest_sha256,
        ranked.rules_config_sha256,
        ranked.ruleset_manifest_sha256,
        None,
        Some(request.max_concurrent_players),
        &input.requested_metrics,
    );
    if let Some(competition_sha256) = request.competition_manifest_sha256 {
        let competition = metadata.and_then(|metadata| {
            metadata
                .competitions
                .iter()
                .find(|competition| competition.competition_manifest_sha256 == competition_sha256)
        });
        // The competition digest does not itself identify its scoring metric.
        // Omit the optional Challenge tab until authenticated metadata names
        // that facet; submission authority remains wholly unaffected.
        if let Some(competition) = competition {
            boards.push(MissionEndBoard {
                tab: LeaderboardTab::Challenge,
                label: competition.manifest.display_name.clone(),
                query: query(
                    LeaderboardQuerySubjectV1::Mission,
                    Some(request.mission_id.clone()),
                    Some(category),
                    ranked.content_manifest_sha256,
                    ranked.rules_config_sha256,
                    ranked.ruleset_manifest_sha256,
                    Some(competition_sha256),
                    Some(request.max_concurrent_players),
                    competition.manifest.metric,
                ),
            });
        }
    }
    if boards.is_empty() {
        return Err("ranked admission requested no supported board metrics".to_owned());
    }
    Ok(boards)
}

#[allow(clippy::too_many_arguments)]
fn metric_boards(
    subject_kind: LeaderboardQuerySubjectV1,
    mission_id: Option<String>,
    mission_scope: Option<BoardCategoryV1>,
    content_identity_sha256: robin_run_protocol::Digest32,
    rules_config_sha256: robin_run_protocol::Digest32,
    ruleset_manifest_sha256: robin_run_protocol::Digest32,
    competition_manifest_sha256: Option<robin_run_protocol::Digest32>,
    max_concurrent_players: Option<u16>,
    metrics: &[BoardMetricV1],
) -> Vec<MissionEndBoard> {
    [
        (BoardMetricV1::OriginalScore, LeaderboardTab::Score, "Score"),
        (BoardMetricV1::FastestSuccess, LeaderboardTab::Time, "Time"),
    ]
    .into_iter()
    .filter(|(metric, _, _)| metrics.contains(metric))
    .map(|(metric, tab, label)| MissionEndBoard {
        tab,
        label: label.to_owned(),
        query: query(
            subject_kind,
            mission_id.clone(),
            mission_scope,
            content_identity_sha256,
            rules_config_sha256,
            ruleset_manifest_sha256,
            competition_manifest_sha256,
            max_concurrent_players,
            metric,
        ),
    })
    .collect()
}

#[allow(clippy::too_many_arguments)]
fn query(
    subject_kind: LeaderboardQuerySubjectV1,
    mission_id: Option<String>,
    mission_scope: Option<BoardCategoryV1>,
    content_identity_sha256: robin_run_protocol::Digest32,
    rules_config_sha256: robin_run_protocol::Digest32,
    ruleset_manifest_sha256: robin_run_protocol::Digest32,
    competition_manifest_sha256: Option<robin_run_protocol::Digest32>,
    max_concurrent_players: Option<u16>,
    metric: BoardMetricV1,
) -> LeaderboardQueryV1 {
    LeaderboardQueryV1 {
        schema_version: SCHEMA_VERSION_V1,
        subject_kind,
        mission_id,
        mission_scope,
        metric,
        content_identity_sha256,
        rules_config_sha256,
        ruleset_manifest_sha256,
        competition_manifest_sha256,
        max_concurrent_players,
        player_public_key: None,
        limit: DEFAULT_BOARD_PAGE_LIMIT,
        cursor: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn select_competition<'a>(
    metadata: &'a LeaderboardMetadataV1,
    mission_id: &str,
    category: BoardCategoryV1,
    content: RunContentIdentityV1,
    rules_config_sha256: robin_run_protocol::Digest32,
    ruleset_manifest_sha256: robin_run_protocol::Digest32,
    max_players: Option<u16>,
    preferred_id: Option<&str>,
) -> Option<&'a robin_run_protocol::CompetitionSummaryV1> {
    metadata
        .competitions
        .iter()
        .filter(|competition| {
            competition.state == CompetitionStateV1::Active
                && competition.manifest.subject
                    == LeaderboardSubjectV1::Mission {
                        mission_id: mission_id.to_owned(),
                        category,
                    }
                && competition.manifest.content == content
                && competition.manifest.rules_config_sha256 == rules_config_sha256
                && competition.manifest.ruleset_manifest_sha256 == ruleset_manifest_sha256
                && max_players.is_none_or(|players| {
                    competition
                        .manifest
                        .participant_composition
                        .max_concurrent_players()
                        == players
                })
                && preferred_id.is_none_or(|id| competition.manifest.competition_id.as_str() == id)
        })
        .min_by_key(|competition| competition.competition_manifest_sha256)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};
    use robin_engine::campaign::Campaign;
    use robin_engine::engine::{SimConfig, SimulationFrameInput};
    use robin_engine::replay::{ReplayFile, ReplayFrame, ReplayHeader};
    use robin_engine::replay_rankability::ReplayRankability;
    use robin_run_protocol::{
        ArtifactRefV1, CampaignChainReceiptV1, CampaignChainStateV1, CampaignRosterContinuityV1,
        ChallengeNonce32, Digest32, FreshRunPreflightGrantClaimV1, FreshRunPreflightGrantV1,
        FreshRunPreflightRequestClaimV1, FreshRunPreflightRequestV1, FreshRunScopeV1,
        MissionFacetV1, OfficialContentEditionV1, OfficialContentSubjectV1, OpaqueId, PublicKey32,
        RANKED_CAMPAIGN_MEDIA_TYPE_V1, RankedSessionConfigV1, ResourceLocaleRootV1, RulesetFacetV1,
        RunContentIdentityV1, Signature64, SimulationSeed64, SpeechTimingAuthorityV1,
    };
    use std::collections::BTreeMap;

    fn digest(byte: u8) -> Digest32 {
        Digest32::from_bytes([byte; 32])
    }

    fn metadata() -> LeaderboardMetadataV1 {
        LeaderboardMetadataV1 {
            schema_version: SCHEMA_VERSION_V1,
            missions: vec![MissionFacetV1 {
                mission_id: "H01".to_owned(),
                display_name: "Huntingdon".to_owned(),
                content_manifest_sha256: digest(1),
            }],
            rulesets: vec![RulesetFacetV1 {
                ruleset_manifest_sha256: digest(3),
                rules_config_sha256: digest(2),
                display_name: "Original".to_owned(),
                preset_id: OpaqueId::new("original").unwrap(),
                preset_name: "Original".to_owned(),
                difficulty_id: OpaqueId::new("normal").unwrap(),
                difficulty_name: "Normal".to_owned(),
                content: RunContentIdentityV1::Mission {
                    content_manifest_sha256: digest(1),
                },
                categories: vec![BoardCategoryV1::IndividualLevel, BoardCategoryV1::Campaign],
                metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
                supports_full_campaign_boards: false,
            }],
            competitions: Vec::new(),
            full_campaign: None,
        }
    }

    fn official_fresh_setup(
        host_key: &SigningKey,
        ranked_session: RankedSessionConfigV1,
    ) -> OfficialRankedSessionSetupV1 {
        fn public_key(key: &SigningKey) -> PublicKey32 {
            PublicKey32::from_bytes(key.verifying_key().to_bytes())
        }
        fn signature(key: &SigningKey, bytes: &[u8]) -> Signature64 {
            Signature64::from_bytes(key.sign(bytes).to_bytes())
        }

        let authority_key = SigningKey::from_bytes(&[0x7a; 32]);
        let request_claim = FreshRunPreflightRequestClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            request_nonce: ChallengeNonce32::from_bytes([0x31; 32]),
            host_public_key: public_key(host_key),
            replay_session_id: digest(0x32),
            host_participant_instance_id: digest(0x33),
            host_nonce: ChallengeNonce32::from_bytes([0x34; 32]),
            scope: FreshRunScopeV1::IndividualLevel,
            starting_campaign: ArtifactRefV1 {
                sha256: ranked_session.starting_campaign_sha256,
                byte_length: ranked_session.starting_campaign_byte_length,
                media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
            },
            ranked_session: ranked_session.clone(),
        };
        let request = FreshRunPreflightRequestV1 {
            host_signature: signature(host_key, &request_claim.signing_bytes().unwrap()),
            claim: request_claim,
            algorithm: SignatureAlgorithmV1::Ed25519,
        };
        let grant_claim = FreshRunPreflightGrantClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            grant_id: OpaqueId::new("runtime-fresh-grant-test").unwrap(),
            grant_nonce: ChallengeNonce32::from_bytes([0x35; 32]),
            grant_authority_public_key: public_key(&authority_key),
            host_public_key: public_key(host_key),
            grant_request_sha256: request.canonical_digest().unwrap(),
            ranked_session_sha256: ranked_session.canonical_digest().unwrap(),
            replay_session_id: request.claim.replay_session_id,
            host_participant_instance_id: request.claim.host_participant_instance_id,
            host_nonce: request.claim.host_nonce,
            scope: request.claim.scope,
            starting_campaign: request.claim.starting_campaign.clone(),
            admitted_at_unix_ms: 1_000,
            expires_at_unix_ms: 2_000,
        };
        let grant = FreshRunPreflightGrantV1 {
            authority_signature: signature(&authority_key, &grant_claim.signing_bytes().unwrap()),
            claim: grant_claim,
            algorithm: SignatureAlgorithmV1::Ed25519,
        };
        OfficialRankedSessionSetupV1 {
            ranked_session,
            custom_package_present: false,
            run_preflight: RankedRunPreflightAdmissionV1::Fresh { request, grant },
            run_preflight_grant_public_key: public_key(&authority_key),
            trusted_now_unix_ms: 1_500,
        }
    }

    fn test_ranked_config(campaign_bytes: &[u8]) -> RankedSessionConfigV1 {
        let mission_id = "Dem_Lei_MP";
        RankedSessionConfigV1 {
            schema_version: SCHEMA_VERSION_V1,
            mission_id: mission_id.to_owned(),
            content_edition: OfficialContentEditionV1::Demo,
            content_subject: OfficialContentSubjectV1::FieldMission {
                mission_id: mission_id.to_owned(),
            },
            simulation_seed: SimulationSeed64::new(7),
            starting_campaign_sha256: Digest32::digest_bytes(campaign_bytes),
            starting_campaign_byte_length: u64::try_from(campaign_bytes.len()).unwrap(),
            prepared_inputs_projection_sha256: digest(2),
            prepared_mission_inputs_seal_sha256: digest(3),
            build_manifest_sha256: digest(4),
            content_manifest_sha256: digest(5),
            campaign_content_manifest_sha256: None,
            rules_config_sha256: digest(6),
            ruleset_manifest_sha256: digest(7),
            competition_manifest_sha256: None,
            spellforge_content_sha256: None,
            resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
            speech_timing: SpeechTimingAuthorityV1::BaseInstallation,
        }
    }

    fn signed_single_player_admission(
        campaign_bytes: &[u8],
    ) -> (RankedMissionAdmission, robin_engine::replay::ReplayData) {
        let mission_id = "Dem_Lei_MP";
        let config = test_ranked_config(campaign_bytes);
        let key = ed25519_dalek::SigningKey::from_bytes(&[0x44; 32]);
        let host = crate::leaderboard_ranked_session::RankedSessionHost::new_official(
            &key,
            robin_engine::multiplayer::NET_PROTOCOL_VERSION,
            official_fresh_setup(&key, config),
        )
        .unwrap();
        let replay = ReplayFile {
            header: ReplayHeader {
                mission_id: mission_id.to_owned(),
                mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                    mission_id, mission_id, mission_id,
                )
                .expect("valid built-in leaderboard-runtime test descriptor"),
                rng_seed: 7,
                sim_config: SimConfig::default(),
                spellforge_package: None,
                version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
                total_frames: 1,
                rankability: ReplayRankability::rankable(),
                campaign: campaign_bytes.to_vec(),
            },
            frames: BTreeMap::from([(
                0,
                ReplayFrame {
                    timeline_before: 0,
                    timeline_after: 1,
                    input: SimulationFrameInput::default(),
                    host_controls: Vec::new(),
                },
            )]),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        }
        .try_into()
        .expect("valid replay fixture");
        (
            RankedMissionAdmission::Signed(SignedRankedMissionAdmission {
                lifecycle: Arc::new(Mutex::new(
                    crate::leaderboard_ranked_session::RankedSessionLifecycle::ranked(host),
                )),
                scope_request: ScopeRequestV1::IndividualLevel,
                requested_metrics: vec![BoardMetricV1::OriginalScore],
                campaign_controller_public_key: None,
            }),
            replay,
        )
    }

    #[test]
    fn multiplayer_host_proposal_may_only_change_published_board_documents() {
        let local = test_ranked_config(b"campaign");
        let mut board_only = local.clone();
        board_only.ruleset_manifest_sha256 = digest(0xa1);
        board_only.campaign_content_manifest_sha256 = Some(digest(0xa2));
        validate_host_proposal_against_local_prepared(&local, &board_only).unwrap();

        let mut changed_simulation = board_only;
        changed_simulation.simulation_seed = SimulationSeed64::new(8);
        assert!(
            validate_host_proposal_against_local_prepared(&local, &changed_simulation).is_err()
        );
    }

    #[test]
    fn early_client_progress_survives_a_late_peer_without_becoming_unbounded() {
        // An early client may spend almost one full phase waiting for the last
        // authenticated peer. Fresh setup therefore gets one composed initial
        // interval; a valid campaign selection then resets inactivity, while
        // the independent total bound continues to advance.
        assert_eq!(
            ranked_preflight_timeout(
                RANKED_PREFLIGHT_PHASE_TIMEOUT_MS + 1,
                RANKED_PREFLIGHT_PHASE_TIMEOUT_MS + 1,
                RANKED_PREFLIGHT_INITIAL_CLIENT_TIMEOUT_MS,
            ),
            None
        );
        assert_eq!(
            ranked_preflight_timeout(
                2 * RANKED_PREFLIGHT_PHASE_TIMEOUT_MS + 1,
                1,
                RANKED_PREFLIGHT_PHASE_TIMEOUT_MS,
            ),
            None
        );
        assert_eq!(
            ranked_preflight_timeout(
                RANKED_PREFLIGHT_TOTAL_TIMEOUT_MS,
                1,
                RANKED_PREFLIGHT_INITIAL_CLIENT_TIMEOUT_MS,
            ),
            Some(RankedPreflightTimeout::Total)
        );
    }

    #[test]
    fn ranked_preflight_no_progress_times_out_per_phase() {
        assert_eq!(
            ranked_preflight_timeout(
                RANKED_PREFLIGHT_PHASE_TIMEOUT_MS - 1,
                RANKED_PREFLIGHT_PHASE_TIMEOUT_MS,
                RANKED_PREFLIGHT_PHASE_TIMEOUT_MS,
            ),
            Some(RankedPreflightTimeout::PhaseInactivity)
        );
        assert_eq!(
            ranked_preflight_timeout(
                RANKED_PREFLIGHT_INITIAL_CLIENT_TIMEOUT_MS,
                RANKED_PREFLIGHT_INITIAL_CLIENT_TIMEOUT_MS,
                RANKED_PREFLIGHT_INITIAL_CLIENT_TIMEOUT_MS,
            ),
            Some(RankedPreflightTimeout::PhaseInactivity)
        );
    }

    fn continuation_selection_fixture() -> (
        CampaignContinuationReceiptSelectionRequestV1,
        CampaignChainReceiptV1,
        PublicKey32,
        PublicKey32,
    ) {
        let host = PublicKey32::from_bytes([1; 32]);
        let controller = PublicKey32::from_bytes([2; 32]);
        let mut config = test_ranked_config(b"campaign");
        config.content_edition = OfficialContentEditionV1::Full;
        config.campaign_content_manifest_sha256 = Some(digest(8));
        let request = CampaignContinuationReceiptSelectionRequestV1::from_lobby(
            RankedPreflightLobbyV1 {
                host_public_key: host,
                max_concurrent_players: 2,
                participant_public_keys: vec![host, controller],
            },
            config.clone(),
            CampaignRosterContinuityV1::ExactSameAuthenticatedKeysEverySession,
        )
        .unwrap();
        let receipt = CampaignChainReceiptV1 {
            schema_version: SCHEMA_VERSION_V1,
            chain_id: OpaqueId::new("chain-runtime-test").unwrap(),
            predecessor_run_id: OpaqueId::new("run-runtime-test").unwrap(),
            predecessor_verification_sha256: digest(9),
            expected_starting_campaign: request.starting_campaign.clone(),
            rules_config_sha256: config.rules_config_sha256,
            ruleset_manifest_sha256: config.ruleset_manifest_sha256,
            competition_manifest_sha256: config.competition_manifest_sha256,
            campaign_content_manifest_sha256: config.campaign_content_manifest_sha256.unwrap(),
            expected_max_concurrent_players: 2,
            participant_public_keys: vec![host, controller],
            campaign_controller_public_key: controller,
            state: CampaignChainStateV1::Active,
        };
        (request, receipt, host, controller)
    }

    #[test]
    fn transport_lobby_discovers_exact_multiplayer_receipt_without_pretransport_snapshot() {
        let (request, receipt, _, controller) = continuation_selection_fixture();
        let mut store = crate::leaderboard_chains::CampaignChainStore::empty();
        store.accepted(receipt.clone()).unwrap();
        assert!(
            select_campaign_receipt_from_store(&store, &request, controller)
                .unwrap()
                .is_some()
        );

        let mut different_cap = receipt;
        different_cap.expected_max_concurrent_players = 3;
        let mut different_store = crate::leaderboard_chains::CampaignChainStore::empty();
        different_store.accepted(different_cap).unwrap();
        assert!(
            select_campaign_receipt_from_store(&different_store, &request, controller)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn controller_signs_only_the_exact_selected_continuation_claim() {
        let (request, receipt, host, controller) = continuation_selection_fixture();
        let selection = CampaignContinuationReceiptSelectionV1 { request, receipt };
        let mut claim = crate::leaderboard_ranked_session::RankedSessionHost::prepare_campaign_continuation_preflight_claim(
            host,
            selection.request.ranked_session.clone(),
            selection.preflight_setup().unwrap(),
        )
        .unwrap();
        validate_controller_preflight_claim(&claim, &selection, host, controller).unwrap();

        claim.predecessor_verification_sha256 = digest(0xaa);
        assert!(validate_controller_preflight_claim(&claim, &selection, host, controller).is_err());
    }

    #[test]
    fn signed_single_player_win_materializes_exact_terminal_submission() {
        let campaign_bytes = bitcode::encode(&Campaign::default());
        let (mut admission, replay) = signed_single_player_admission(&campaign_bytes);
        admission
            .materialize_terminal_from_replay(
                "Dem_Lei_MP",
                Arc::from(campaign_bytes.clone()),
                &replay,
            )
            .unwrap();

        let bundle = build_bundle(
            "Dem_Lei_MP",
            false,
            Arc::from(campaign_bytes),
            MissionEndOutcome::Won,
            &admission,
            &LeaderboardPreferences::default(),
            None,
        )
        .unwrap();
        let input = bundle.eligible_submission.unwrap();
        assert_eq!(input.offer_request.max_concurrent_players, 1);
        assert_eq!(input.offer_request.participant_instance_count, 1);
        assert_eq!(input.replay_session_transcript.max_concurrent_players, 1);
    }

    #[test]
    fn signed_lost_and_interrupted_runs_keep_boards_but_cannot_submit() {
        for outcome in [MissionEndOutcome::Lost, MissionEndOutcome::Interrupted] {
            let campaign_bytes = bitcode::encode(&Campaign::default());
            let (mut admission, replay) = signed_single_player_admission(&campaign_bytes);
            admission
                .materialize_terminal_from_replay(
                    "Dem_Lei_MP",
                    Arc::from(campaign_bytes.clone()),
                    &replay,
                )
                .unwrap();
            let bundle = build_bundle(
                "Dem_Lei_MP",
                false,
                Arc::from(campaign_bytes),
                outcome,
                &admission,
                &LeaderboardPreferences::default(),
                None,
            )
            .unwrap();
            assert_eq!(bundle.boards.len(), 1);
            assert!(bundle.eligible_submission.is_none());
            assert_eq!(
                bundle.submission_unavailable_reason.as_deref(),
                Some("only won missions can be submitted")
            );
        }
    }

    #[test]
    fn browse_only_builds_real_server_facets_but_never_submission_authority() {
        let bundle = build_bundle(
            "H01",
            false,
            Arc::from([1_u8, 2, 3]),
            MissionEndOutcome::Won,
            &RankedMissionAdmission::browse_only("missing signed genesis"),
            &LeaderboardPreferences::default(),
            Some(&metadata()),
        )
        .unwrap();

        assert_eq!(bundle.boards.len(), 2);
        assert!(bundle.eligible_submission.is_none());
        assert_eq!(
            bundle.submission_unavailable_reason.as_deref(),
            Some("missing signed genesis")
        );
        assert!(
            bundle
                .boards
                .iter()
                .all(|board| board.query.content_identity_sha256 == digest(1))
        );
    }

    #[test]
    fn every_terminal_outcome_gets_the_same_browse_boards() {
        for outcome in [
            MissionEndOutcome::Won,
            MissionEndOutcome::Lost,
            MissionEndOutcome::Interrupted,
        ] {
            let bundle = build_bundle(
                "H01",
                true,
                Arc::from([9_u8]),
                outcome,
                &RankedMissionAdmission::browse_only("no authority"),
                &LeaderboardPreferences::default(),
                Some(&metadata()),
            )
            .unwrap();
            assert_eq!(bundle.boards.len(), 2);
            assert!(bundle.multiplayer);
        }
    }

    #[test]
    fn missing_published_mission_fails_instead_of_fabricating_query_digests() {
        let error = build_bundle(
            "unknown",
            false,
            Arc::from([1_u8]),
            MissionEndOutcome::Won,
            &RankedMissionAdmission::browse_only("no authority"),
            &LeaderboardPreferences::default(),
            Some(&metadata()),
        )
        .unwrap_err();
        assert!(error.contains("not published"), "{error}");
    }
}
