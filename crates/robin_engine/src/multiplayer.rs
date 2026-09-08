//! Multiplayer wire-format types and channel plumbing.
//!
//! This module defines the platform-pure layer of multiplayer
//! infrastructure: the wire-format enums (`NetMsg`, `NetEvent`,
//! `NetOutbound`), the cross-thread channel bundle (`NetChannels`), and
//! the protocol constants. The actual iroh transport (native
//! QUIC/direct/relay, browser relay-over-WebSocket) lives in
//! `robin_rs::multiplayer::{native, wasm}` and feeds events into these channels.
//!
//! `EngineManager` (this crate) owns a `NetChannels` and uses it to
//! route locally-sourced player commands over the wire and drain
//! peer-sourced inputs back into the engine at the correct frames.

use crate::engine::Engine;
use crate::player_command::{DialogResult, ModalKind, PlayerCommand, PlayerId, PlayerInput};
use robin_run_protocol::{LeaderboardCoSignInstanceV1, LeaderboardCoSignRequestV1, Validate};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use unicode_security::GeneralSecurityProfile;
use unicode_security::general_security_profile::IdentifierType;

/// Cross-thread snapshot of the local game loop's current sim frame.
///
/// Updated by the game loop at the top of every tick (just after
/// rewind/auto-replay accounting).  Read by the server's broadcast
/// pump and per-peer reader threads to stamp `BroadcastInput` with a
/// fresh `target_frame` so every peer applies the input at the same
/// frame.  An `Arc<AtomicU32>` is the simplest thread-safe handoff —
/// the rate is one update per tick (25 Hz) and reads happen at most
/// per inbound input frame.
pub type FrameCursor = Arc<AtomicU32>;

/// Shared encoded initial-state snapshot offered by the host to joining peers.
///
/// Encoding once at publication time guarantees that every peer admitted at
/// this boundary receives byte-identical authoritative state.
pub type InitialSnapshot = Arc<Mutex<Option<(u32, Vec<u8>)>>>;

/// Shared bounded inbox for verified/locally armed leaderboard authorization
/// events. A capability-restricted mission-end port may retain this queue
/// without retaining or mutably borrowing the full multiplayer channel owner.
pub type LeaderboardAuthorizationInbox = Arc<Mutex<std::collections::VecDeque<NetEvent>>>;

/// Make a new [`FrameCursor`] starting at frame 0.
pub fn new_frame_cursor() -> FrameCursor {
    Arc::new(AtomicU32::new(0))
}

/// Number of frames of "input delay" the server adds when stamping
/// peer inputs with a target frame.  At 25 Hz this is ~80 ms.  The rollback path picks
/// up the slack on slower links by rewinding when an input arrives
/// late.  Tuneable; mirrors the `MAX_INPUT_DELAY` constant in classic
/// GGPO-style netcode.
pub const INPUT_DELAY_FRAMES: u32 = 2;

/// Wire-format protocol version. Bump on any breaking change to [`NetMsg`] or
/// an engine snapshot carried by it. Both sides exchange this in the
/// handshake; mismatches abort the connection. Version 35 adds completed
/// planned quick-action payloads, per-seat shield prompts, and deterministic
/// tactical queue formations to the authoritative version-34 state. Version
/// 36 adds shared-vision fog state and its host-owned runtime command.
/// Protocol 37 carries save/replay state with the mandatory mission-asset
/// descriptor introduced by save 70 / replay 28. Protocol 38 adds the typed,
/// targeted leaderboard co-sign and official-ranked-session authorization
/// messages to that complete current-main wire contract.
/// Protocol 40 adds reversible background-patch configuration and activation
/// targets to the engine snapshots exchanged by peers.
pub const NET_PROTOCOL_VERSION: u32 = 40;

/// Maximum bytes in one resumable full-mod transfer chunk. The outer native
/// transport frame has a larger bound for engine snapshots, so content must
/// enforce this dedicated limit before allocating/staging it.
pub const DISTRIBUTED_MOD_CHUNK_LIMIT: usize = 1024 * 1024;

/// Bounded, non-executable description sent before any host content bytes.
/// The authenticated distributor id comes from the iroh connection itself;
/// `host_endpoint_id` is copied here only so consent UI can display and audit
/// it. Trust authority is the exact full-mod/package hash pair.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct DistributedModOffer {
    pub schema_version: u32,
    pub full_mod_sha256: [u8; 32],
    pub spellforge_package_sha256: Option<[u8; 32]>,
    pub spellforge_vm_abi: Option<String>,
    pub encoded_bytes: u64,
    pub mission_basename: String,
    pub mission_rhm_entry: String,
    pub map_filename: String,
    pub title: String,
    pub claimed_author: String,
    pub version: String,
    pub source_url: String,
    pub license: String,
    pub host_endpoint_id: String,
}

impl DistributedModOffer {
    pub const TEXT_BYTE_LIMIT: usize = 4 * 1024;
    pub const AUTHENTICATED_HOST_ID_BYTE_LIMIT: usize = 64;
    pub const ENCODED_BYTE_LIMIT: u64 = 130 * 1024 * 1024;

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!(
                "unsupported distributed-mod offer schema {}; expected 1",
                self.schema_version
            ));
        }
        if self.encoded_bytes == 0 || self.encoded_bytes > Self::ENCODED_BYTE_LIMIT {
            return Err(format!(
                "distributed-mod offer declares {} bytes; expected 1..={}",
                self.encoded_bytes,
                Self::ENCODED_BYTE_LIMIT
            ));
        }
        for (label, value) in [
            ("mission basename", &self.mission_basename),
            ("mission RHM entry", &self.mission_rhm_entry),
            ("map filename", &self.map_filename),
            ("title", &self.title),
            ("claimed author", &self.claimed_author),
            ("version", &self.version),
            ("source URL", &self.source_url),
            ("license", &self.license),
        ] {
            validate_safe_display_text(
                &format!("distributed-mod {label}"),
                value,
                Self::TEXT_BYTE_LIMIT,
            )?;
        }
        validate_safe_display_text(
            "distributed-mod host endpoint id",
            &self.host_endpoint_id,
            Self::AUTHENTICATED_HOST_ID_BYTE_LIMIT,
        )?;
        if let Some(vm_abi) = &self.spellforge_vm_abi {
            validate_safe_display_text(
                "distributed-mod Spellforge VM ABI",
                vm_abi,
                Self::TEXT_BYTE_LIMIT,
            )?;
        }
        if self.spellforge_package_sha256.is_some() != self.spellforge_vm_abi.is_some() {
            return Err(
                "distributed-mod package hash and VM ABI must be present together".to_owned(),
            );
        }
        if self.mission_basename.contains(['/', '\\']) {
            return Err("distributed-mod mission basename is not one path component".to_owned());
        }
        Ok(())
    }
}

/// Validate untrusted text before it is rendered at a security or identity
/// boundary.
///
/// Unicode bidirectional controls and invisible formatting marks are not
/// classified as control characters by [`char::is_control`], but allowing them
/// would let metadata visually reorder or hide the hash and labels beside it.
pub fn validate_safe_display_text(
    label: &str,
    value: &str,
    max_bytes: usize,
) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.trim() != value {
        return Err(format!(
            "{label} must not have leading or trailing whitespace"
        ));
    }
    if value.len() > max_bytes {
        return Err(format!(
            "{label} is {} bytes; limit is {max_bytes}",
            value.len()
        ));
    }
    if value.chars().any(is_unsafe_display_character) {
        return Err(format!(
            "{label} contains control, bidirectional, or invisible formatting characters"
        ));
    }
    Ok(())
}

pub fn is_unsafe_display_character(character: char) -> bool {
    character.is_control()
        || (character.is_whitespace() && character != ' ')
        || character.identifier_type() == Some(IdentifierType::Default_Ignorable)
        || character == '\u{2800}'
        || matches!(
            character,
            '\u{061c}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{feff}'
        )
}

/// Preserve a diagnostic's actual text while making every invisible/control
/// character explicit and enforcing a UTF-8 byte ceiling. This is for
/// protocol-visible errors, not identity fields: unsafe characters are shown
/// as their Unicode escape instead of silently dropping the evidence.
pub fn bounded_safe_diagnostic(value: &str, max_bytes: usize) -> String {
    const FALLBACK: &str = "diagnostic unavailable";
    const ELLIPSIS: &str = "…";
    assert!(
        max_bytes >= ELLIPSIS.len(),
        "diagnostic byte limit must fit an ellipsis"
    );

    let trimmed = value.trim();
    let source = if trimmed.is_empty() {
        FALLBACK
    } else {
        trimmed
    };
    let mut sanitized = String::with_capacity(source.len().min(max_bytes));
    for character in source.chars() {
        if is_unsafe_display_character(character) {
            sanitized.extend(character.escape_unicode());
        } else {
            sanitized.push(character);
        }
    }
    if sanitized.len() <= max_bytes {
        return sanitized;
    }

    let mut end = max_bytes - ELLIPSIS.len();
    while !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    let mut bounded = sanitized[..end].trim_end().to_owned();
    bounded.push_str(ELLIPSIS);
    debug_assert!(bounded.len() <= max_bytes);
    bounded
}

/// Default TCP port for the multiplayer server.
pub const DEFAULT_PORT: u16 = 7878;

/// Frame cadence at which the host samples its engine state hash and
/// broadcasts it for clients to verify against.  Matches the replay
/// recorder's `frame % 25 == 0` cadence (one hash per simulated
/// second at 25 Hz) so the same sampling point is reused.
pub const STATE_HASH_INTERVAL: u32 = 25;

/// Unpredictable identity for one host process's multiplayer campaign session.
///
/// Modal traffic carries this value so a delayed packet from an earlier host
/// lifecycle can never resolve a UI surface in a replacement session.
#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct MultiplayerSessionId(pub [u8; 32]);

/// Stable identity for a host-authored outer-mission transition.
#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct SnapshotTransitionId {
    pub session_id: MultiplayerSessionId,
    pub sequence: u64,
}

/// Exact authoritative state retained by every participant before a
/// host-authored outer-mission transition is committed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub enum SnapshotTransitionPayload {
    Save {
        mission_id: u32,
        save_bytes: Vec<u8>,
    },
    CampaignExit {
        exit_code: crate::game_operation::GameCode,
        engine_bytes: Vec<u8>,
    },
}

/// Stable identity for one occurrence of a multiplayer modal.
///
/// `opened_frame` identifies the authoritative timeline boundary at which the
/// modal appeared. `occurrence` distinguishes repeated instances of the same
/// [`ModalKind`], including repeated unkeyed Sherwood reports.
#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct ModalInstanceId {
    pub session_id: MultiplayerSessionId,
    pub opened_frame: u32,
    pub occurrence: u64,
}

/// Fixed response to a purpose-bound leaderboard co-sign request.
///
/// The response deliberately carries no nickname, seat, arbitrary message, or
/// client-claimed sender. The server recovers the exact request (including its
/// run digest) from pending authenticated transport state and stamps the
/// [`PlayerId`] on the resulting [`NetEvent::LeaderboardCoSignResponse`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct LeaderboardCoSignResponse {
    pub instance: LeaderboardCoSignInstanceV1,
    pub signer_public_key: [u8; 32],
    #[serde(with = "serde_big_array::BigArray")]
    pub signature: [u8; 64],
}

/// Largest canonical ranked-session document admitted inside a multiplayer
/// control frame. The outer bitcode frame remains independently bounded by
/// the transport. Keeping the document bound here prevents a caller from
/// bypassing that transport check through a local [`NetOutbound`] channel.
pub const MAX_RANKED_WIRE_DOCUMENT_BYTES: usize = 64 * 1024;

fn validate_ranked_wire_document_bytes(bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.is_empty() {
        return Err("ranked wire document is empty");
    }
    if bytes.len() > MAX_RANKED_WIRE_DOCUMENT_BYTES {
        return Err("ranked wire document exceeds its decoded size limit");
    }
    Ok(())
}

macro_rules! ranked_wire_document {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(
            Clone, Debug, Eq, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
        )]
        pub struct $name(Vec<u8>);

        impl $name {
            /// Wrap bytes produced by
            /// `leaderboard_ranked_session::encode_ranked_wire_document`.
            pub fn new(bytes: Vec<u8>) -> Result<Self, &'static str> {
                let document = Self(bytes);
                document.validate()?;
                Ok(document)
            }

            pub fn as_bytes(&self) -> &[u8] {
                &self.0
            }

            pub fn into_bytes(self) -> Vec<u8> {
                self.0
            }

            fn validate(&self) -> Result<(), &'static str> {
                validate_ranked_wire_document_bytes(&self.0)
            }
        }
    };
}

ranked_wire_document!(
    RankedSessionConfigDocument,
    "Canonical JSON for one locally prepared `RankedSessionConfigV1`. This is local trust state and is never sent on the wire."
);
ranked_wire_document!(
    RankedSessionGenesisDocument,
    "Canonical JSON for one signed `ReplaySessionGenesisV1`."
);
ranked_wire_document!(
    RankedJoinClaimDocument,
    "Canonical JSON for the exact host-issued `NamedSeatJoinClaimV1`."
);
ranked_wire_document!(
    RankedJoinAttestationDocument,
    "Canonical JSON for one signed `NamedSeatJoinAttestationV1`."
);
ranked_wire_document!(
    RankedParticipantRosterDocument,
    "Canonical JSON for the complete monotonic `Vec<ParticipantClaimV1>` admitted by the host."
);
ranked_wire_document!(
    RankedCoSignContextDocument,
    "Canonical JSON for one closed `RankedCoSignContextV1` continuation or submission context."
);
ranked_wire_document!(
    RankedSubmissionAcceptedDocument,
    "Canonical JSON for one validated `SubmissionAcceptedV1` queue acknowledgement."
);
ranked_wire_document!(
    RankedOfficialSessionSetupDocument,
    "Canonical JSON for one `OfficialRankedSessionWireSetupV1`; trusted time is intentionally local and absent."
);
ranked_wire_document!(
    RankedContinuationReceiptSelectionRequestDocument,
    "Canonical JSON for one exact `CampaignContinuationReceiptSelectionRequestV1`."
);
ranked_wire_document!(
    RankedContinuationReceiptSelectionDocument,
    "Canonical JSON for one controller-selected `CampaignContinuationReceiptSelectionV1`."
);
ranked_wire_document!(
    RankedContinuationPreflightClaimDocument,
    "Canonical JSON for one exact `CampaignContinuationPreflightRequestClaimV1` controller authorization request."
);
ranked_wire_document!(
    RankedContinuationPreflightSignatureDocument,
    "Canonical JSON for one controller `ParticipantSignatureV1` over the exact continuation-preflight claim."
);

/// Exact server-issued ranked admission challenge. The client must validate
/// both documents against its authenticated host and locally prepared session
/// before signing the embedded named-seat claim.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct RankedJoinChallenge {
    pub session_genesis: RankedSessionGenesisDocument,
    pub join_claim: RankedJoinClaimDocument,
}

impl RankedJoinChallenge {
    fn validate(&self) -> Result<(), &'static str> {
        self.session_genesis.validate()?;
        self.join_claim.validate()
    }
}

/// Closed reason a client cannot provide the requested ranked admission
/// attestation. This deliberately contains no free-form signing or authority
/// material. Gameplay may continue after the host downgrades the session.
#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub enum RankedJoinUnavailableReason {
    DurableIdentityUnavailable,
    LocalRankedSessionUnavailable,
    LocalRankedSessionMismatch,
    AttestationSigningFailed,
}

/// Client answer to one ranked admission challenge.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub enum RankedJoinResponse {
    Attestation(RankedJoinAttestationDocument),
    Unavailable(RankedJoinUnavailableReason),
}

impl RankedJoinResponse {
    fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Attestation(document) => document.validate(),
            Self::Unavailable(_) => Ok(()),
        }
    }
}

/// Server acknowledgement that the exact signed join was admitted. Sending
/// an attestation is not sufficient for a client to consider itself ranked;
/// it must receive this acknowledgement first.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct RankedJoinAccepted {
    pub session_genesis: RankedSessionGenesisDocument,
    pub join_attestation: RankedJoinAttestationDocument,
    pub participant_roster: RankedParticipantRosterDocument,
}

impl RankedJoinAccepted {
    fn validate(&self) -> Result<(), &'static str> {
        self.session_genesis.validate()?;
        self.join_attestation.validate()?;
        self.participant_roster.validate()
    }
}

/// Closed, non-personal reason the multiplayer session irreversibly left the
/// verified ranking lane. The host broadcasts one of these events and every
/// participant continues in browse-only mode.
#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub enum RankedBrowseOnlyReason {
    HostRankedSessionUnavailable,
    PeerIdentityUnavailable,
    PeerRankedSessionMismatch,
    PeerAttestationRejected,
    RankedTransportInterrupted,
    RankedProtocolViolation,
}

/// Client request retained for host presentation. Requests are advisory and
/// never resolve a modal without a later [`NetMsg::ModalDecision`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisibleModalRequest {
    pub from: PlayerId,
    pub instance: ModalInstanceId,
    pub kind: ModalKind,
    pub result: DialogResult,
    pub requested_frame: u32,
}

#[derive(Debug)]
struct ModalOccurrenceState {
    kind: ModalKind,
    next_occurrence: u64,
    active: Option<ModalInstanceId>,
}

#[derive(Debug, Default)]
struct ModalSyncState {
    session_id: Option<MultiplayerSessionId>,
    occurrences: Vec<ModalOccurrenceState>,
    inbox: std::collections::VecDeque<NetEvent>,
    visible_requests: std::collections::VecDeque<VisibleModalRequest>,
}

/// Browser-only durable seat claim. The IndexedDB-held private key signs a
/// session/host/ephemeral-transport binding; only the public key and signature
/// cross the wire. Native clients rely directly on iroh's durable endpoint id.
#[derive(Clone, Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct BrowserPeerAuth {
    pub join_code: String,
    pub durable_public_key: [u8; 32],
    pub signature: Vec<u8>,
}

pub const MAX_DISPLAY_NAME_BYTES: usize = 256;
pub const MAX_DISPLAY_NAME_CHARS: usize = 64;
pub const MAX_MISSION_ID_BYTES: usize = 256;
pub const MAX_JOIN_CODE_BYTES: usize = 16 * 1024;
pub const MAX_NOTE_BYTES: usize = 4 * 1024;
pub const MAX_REJECT_REASON_BYTES: usize = 1024;
pub const MAX_SNAPSHOT_FRAME_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_LEADERBOARD_COSIGN_INBOX_EVENTS: usize = 64;

pub fn validate_display_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty()
        || name.trim() != name
        || name.len() > MAX_DISPLAY_NAME_BYTES
        || name.chars().count() > MAX_DISPLAY_NAME_CHARS
        || name.chars().any(is_unsafe_display_character)
    {
        return Err(
            "multiplayer display name must contain 1..=64 safe, unpadded characters and at most 256 UTF-8 bytes",
        );
    }
    Ok(())
}

pub fn validate_mission_id(mission_id: &str) -> Result<(), &'static str> {
    if mission_id.is_empty()
        || mission_id.trim() != mission_id
        || mission_id.len() > MAX_MISSION_ID_BYTES
        || mission_id.contains(['/', '\\'])
        || mission_id == "."
        || mission_id == ".."
        || !mission_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err("multiplayer mission id must be a safe 1..=256 byte basename");
    }
    Ok(())
}

/// Exact byte string signed by the browser's non-extractable durable key.
pub fn browser_seat_proof_message(
    session_id: [u8; 32],
    host_endpoint_id: [u8; 32],
    transport_endpoint_id: [u8; 32],
) -> Vec<u8> {
    const DOMAIN: &[u8] = b"robinhood/browser-seat-proof/v1\0";
    let mut message = Vec::with_capacity(DOMAIN.len() + 96);
    message.extend_from_slice(DOMAIN);
    message.extend_from_slice(&session_id);
    message.extend_from_slice(&host_endpoint_id);
    message.extend_from_slice(&transport_endpoint_id);
    message
}

/// One on-the-wire message.  Encoded as a bitcode binary blob inside
/// each WebSocket frame.
#[derive(Clone, Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub enum NetMsg {
    /// Client → server: opening handshake.
    Hello {
        protocol_version: u32,
        nickname: String,
        browser_auth: Option<BrowserPeerAuth>,
        /// Durable first-start leaderboard identity. Native transport keys are
        /// intentionally ephemeral and must not be treated as this identity.
        ranked_public_key: Option<[u8; 32]>,
    },
    /// Server → client: handshake response.  Tells the client which
    /// seat it owns and gives it the mission seed it must use to
    /// initialise its sim deterministically.
    Welcome {
        your_seat: PlayerId,
        session_id: MultiplayerSessionId,
        mission_id: String,
        mission_seed: u64,
        sim_config: crate::engine::SimConfig,
        /// Host-selected presentation pack used only to derive stable logical
        /// speech durations. Each peer still plays its own active language.
        speech_timing_locale: Option<String>,
        host_nickname: String,
    },
    /// Server → client, after Hello and before Welcome: exact remote content
    /// required by this session. The server sends no snapshot or simulation
    /// traffic until the client validates/mounts it and returns ContentReady.
    ContentOffer { offer: DistributedModOffer },
    /// Client → server: explicit acceptance of this immutable hash and the
    /// durable byte prefix already present locally. Reconnect repeats this
    /// request with the new exact prefix, making the transfer resumable.
    ContentRequest {
        full_mod_sha256: [u8; 32],
        resume_offset: u64,
    },
    /// Client → server: explicit refusal. There is no unsafe vanilla/SCB
    /// fallback; the server closes this peer's admission path.
    ContentReject {
        full_mod_sha256: [u8; 32],
        reason: String,
    },
    /// Server → client: one bounded sequential segment of canonical package
    /// bytes. QUIC provides reliability; offsets provide durable resumption.
    ContentChunk {
        full_mod_sha256: [u8; 32],
        offset: u64,
        total_bytes: u64,
        bytes: Vec<u8>,
    },
    /// Client → server: full content was hash-verified, admitted, and mounted.
    ContentReady { full_mod_sha256: [u8; 32] },
    /// Client → server: full content was hash-verified and cached by the
    /// interactive lobby, but this connection must not claim a gameplay seat.
    ContentPrepared { full_mod_sha256: [u8; 32] },
    /// Server → client: an understood opening request was rejected. A typed
    /// reason survives the relay path instead of becoming an opaque QUIC close.
    Reject { reason: String },
    /// Client → server: an input the client wants applied this tick,
    /// tagged with the sender's local frame at dispatch time.  The
    /// server uses `origin_frame` as a lower bound when assigning the
    /// shared target frame so a slightly-ahead client does not receive
    /// its own input in the past on localhost.
    Input {
        origin_frame: u32,
        command: PlayerCommand,
    },
    /// Server → all peers: a tagged input ready for engine dispatch
    /// at `target_frame`.
    BroadcastInput {
        /// Server/host sim frame observed when this input was stamped.
        server_frame: u32,
        /// Sender's local sim frame at dispatch time.
        origin_frame: u32,
        target_frame: u32,
        input: PlayerInput,
    },
    /// Either direction, advisory.
    Note(String),
    /// Server → all peers: deterministic engine state hash at the
    /// start of `frame` (pre-tick), broadcast every
    /// [`STATE_HASH_INTERVAL`] frames.
    StateHash {
        frame: u32,
        hash: Option<u64>,
        clock_frame: Option<u32>,
        ms_until_next_frame: Option<u32>,
    },
    /// Server → newly-handshaking peer: an authoritative engine
    /// snapshot for mid-mission joins. `engine_bytes` uses native bitcode.
    InitialSnapshot { frame: u32, engine_bytes: Vec<u8> },
    /// Client → server: this peer has loaded the mission, installed
    /// the host snapshot, and is ready to enter the synchronized sim.
    ReadyToSim { frame: u32 },
    /// Server → all peers: every expected player is loaded and ready;
    /// begin simulating `frame` at this wall-clock timestamp.
    BeginSim { frame: u32, start_epoch_ms: u64 },
    /// Client → server: a visible request for the host to choose this result.
    /// A proposal is never a vote and never changes local modal state.
    ModalProposal {
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
        requested_frame: u32,
    },
    /// Server → clients: the sole authoritative result for one exact modal
    /// occurrence. `decision_frame` is the host timeline frame on which the
    /// decision was made and recorded.
    ModalDecision {
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
        decision_frame: u32,
    },
    /// Server → peer: abandon the current prediction future and perform a
    /// complete transport handshake. The next session starts from the host's
    /// latest authoritative full snapshot.
    ReconnectRequired { reason: String },
    /// Server → clients: validate and retain these exact serialized save bytes
    /// before acknowledging a host-authored mission transition.
    PrepareSnapshotTransition {
        id: SnapshotTransitionId,
        payload: SnapshotTransitionPayload,
    },
    /// Client → server: the exact transition bytes decoded and validated.
    SnapshotTransitionReady { id: SnapshotTransitionId },
    /// Server → clients: every connected peer retained the same bytes; all
    /// participants may now leave the mission and re-handshake.
    CommitSnapshotTransition { id: SnapshotTransitionId },
    /// Server -> one client: exact ranked genesis and named-seat claim. The
    /// client transport must keep this staged until its locally prepared
    /// ranked configuration matches the signed genesis.
    RankedJoinChallenge(RankedJoinChallenge),
    /// Client -> server: either the exact signed challenge or a closed reason
    /// that ranked admission is unavailable. The authenticated stream supplies
    /// the sender seat and transport endpoint.
    RankedJoinResponse(RankedJoinResponse),
    /// Server -> one client: acknowledgement that the exact attestation was
    /// admitted. A client is not ranked merely because it sent a response.
    RankedJoinAccepted(RankedJoinAccepted),
    /// Server -> all already accepted clients after a fresh admission. Claims
    /// are complete and monotonic; reconnects retain the existing roster.
    RankedParticipantRoster(RankedParticipantRosterDocument),
    /// Server -> all clients: the session irreversibly left the verified lane.
    /// Gameplay continues and leaderboard browsing remains available.
    RankedBrowseOnly { reason: RankedBrowseOnlyReason },
    /// Server -> one client: a closed continuation/submission co-sign context.
    /// The client must stage it until an independently reconstructed local
    /// context is byte-for-byte identical.
    RankedCoSignContext(RankedCoSignContextDocument),
    /// Server -> the exact campaign-controller client after the leaderboard
    /// service accepted its submission into the verification queue. The
    /// document is typed and validated before entering or leaving the
    /// capability-restricted authorization port.
    RankedSubmissionAccepted(RankedSubmissionAcceptedDocument),
    /// Host -> all clients after authority admission and before ranked join.
    RankedOfficialSessionSetup(RankedOfficialSessionSetupDocument),
    /// Host -> authenticated clients before ranked genesis. Only the immutable
    /// controller with an exact local active receipt responds.
    RankedContinuationReceiptSelectionRequest(RankedContinuationReceiptSelectionRequestDocument),
    /// Controller -> host. Authenticated stream supplies the responder seat.
    RankedContinuationReceiptSelection(RankedContinuationReceiptSelectionDocument),
    /// Host -> immutable campaign controller before ranked genesis exists.
    RankedContinuationPreflightClaim(RankedContinuationPreflightClaimDocument),
    /// Controller -> host. The authenticated stream supplies the sender seat;
    /// the typed signature remains bound to the exact staged claim.
    RankedContinuationPreflightSignature(RankedContinuationPreflightSignatureDocument),
    /// Server -> one specifically selected client. The request is the exact
    /// closed, purpose-bound payload reconstructed by the leaderboard server;
    /// it is never a generic signing request.
    LeaderboardCoSignRequest(LeaderboardCoSignRequestV1),
    /// Client -> server. The authenticated stream supplies the sender seat;
    /// this payload must not contain or claim one.
    LeaderboardCoSignResponse(LeaderboardCoSignResponse),
}

/// One incoming wire event ready for the game loop.
#[derive(Clone, Debug)]
pub enum NetEvent {
    /// A peer's input arrived, ready to apply at `target_frame`.
    Input {
        server_frame: u32,
        origin_frame: u32,
        target_frame: u32,
        input: PlayerInput,
    },
    /// The server (or our own client connection) has decided we own
    /// this seat in the simulation.
    AssignedLocalSeat(PlayerId),
    /// Best-effort diagnostic from the network layer.
    Note(String),
    /// The connection ended.
    Disconnected,
    /// I/O thread successfully re-handshook with the server after a
    /// drop.  Followed by a fresh `AssignedLocalSeat`.
    Reconnected,
    /// Authoritative state hash and/or clock sample from the host at `frame`.
    PeerStateHash {
        frame: u32,
        hash: Option<u64>,
        clock_frame: Option<u32>,
        ms_until_next_frame: Option<u32>,
    },
    /// Mission construction state announced by the server in `Welcome`.
    /// Only the wasm path emits this; native captures it synchronously.
    MissionConfig {
        mission_id: String,
        rng_seed: u64,
        sim_config: crate::engine::SimConfig,
        speech_timing_locale: Option<String>,
    },
    /// A validated wire offer requiring local trust/cache admission before
    /// the transport is allowed to receive an engine snapshot.
    ContentOffer(DistributedModOffer),
    /// A bounded sequential content segment ready for durable staging.
    ContentChunk {
        full_mod_sha256: [u8; 32],
        offset: u64,
        total_bytes: u64,
        bytes: Vec<u8>,
    },
    /// Unrecoverable transport/session compatibility failure.
    Fatal(String),
    /// Authoritative initial-state snapshot from the host.
    InitialSnapshot {
        frame: u32,
        engine_bytes: Vec<u8>,
    },
    /// The server released the multiplayer start barrier.
    BeginSim {
        frame: u32,
        start_epoch_ms: u64,
    },
    /// A client asked the host to choose a modal result. Presentation may show
    /// this request, but only a host decision can close the modal.
    ModalProposal {
        from: PlayerId,
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
        requested_frame: u32,
    },
    /// The host chose the result for one exact modal occurrence.
    ModalDecision {
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
        decision_frame: u32,
    },
    PrepareSnapshotTransition {
        id: SnapshotTransitionId,
        payload: SnapshotTransitionPayload,
    },
    CommitSnapshotTransition {
        id: SnapshotTransitionId,
    },
    /// A locally armed ranked admission challenge arrived from the
    /// authenticated host. Platform glue emits this only after the signed
    /// genesis matches the independently prepared local ranked configuration.
    RankedJoinChallenge(RankedJoinChallenge),
    /// Host-side event stamped with the authenticated sender seat.
    RankedJoinResponse {
        from: PlayerId,
        response: RankedJoinResponse,
    },
    /// The host admitted this client's exact signed named-seat challenge.
    RankedJoinAccepted(RankedJoinAccepted),
    /// Complete monotonic participant roster after another fresh admission.
    RankedParticipantRoster(RankedParticipantRosterDocument),
    /// Irreversible verified-lane downgrade. This event is intentionally
    /// separate from fatal multiplayer compatibility or transport failures.
    RankedBrowseOnly {
        reason: RankedBrowseOnlyReason,
    },
    /// An exact locally armed continuation/submission co-sign context arrived.
    RankedCoSignContext(RankedCoSignContextDocument),
    /// Queue acknowledgement delivered only to the authenticated campaign
    /// controller selected by the host's retained ranked participant roster.
    RankedSubmissionAccepted(RankedSubmissionAcceptedDocument),
    RankedOfficialSessionSetup(RankedOfficialSessionSetupDocument),
    RankedContinuationReceiptSelectionRequest(RankedContinuationReceiptSelectionRequestDocument),
    RankedContinuationReceiptSelection {
        from: PlayerId,
        selection: RankedContinuationReceiptSelectionDocument,
    },
    RankedContinuationPreflightClaim(RankedContinuationPreflightClaimDocument),
    RankedContinuationPreflightSignature {
        from: PlayerId,
        signature: RankedContinuationPreflightSignatureDocument,
    },
    /// A locally armed request arrived from the authenticated host. The client
    /// transport emits this only after exact equality with the request derived
    /// independently from its validated offer/transcript.
    LeaderboardCoSignRequest(LeaderboardCoSignRequestV1),
    /// A verified purpose-bound proof from an authenticated multiplayer seat.
    ///
    /// This is transport evidence, not by itself a claim that the proof is
    /// admissible for a submission: the consumer must still bind `from` and
    /// `signer_public_key` to the validated participant/controller roster.
    LeaderboardCoSignResponse {
        from: PlayerId,
        response: LeaderboardCoSignResponse,
    },
}

/// What the game loop pushes into the outgoing channel.
#[derive(Clone, Debug)]
pub enum NetOutbound {
    Input {
        origin_frame: u32,
        command: PlayerCommand,
    },
    StateHash {
        frame: u32,
        hash: Option<u64>,
        clock_frame: Option<u32>,
        ms_until_next_frame: Option<u32>,
    },
    InitialSnapshot {
        frame: u32,
        engine_bytes: Vec<u8>,
    },
    ReadyToSim {
        frame: u32,
    },
    ContentRequest {
        full_mod_sha256: [u8; 32],
        resume_offset: u64,
    },
    ContentReject {
        full_mod_sha256: [u8; 32],
        reason: String,
    },
    ContentReady {
        full_mod_sha256: [u8; 32],
    },
    ContentPrepared {
        full_mod_sha256: [u8; 32],
    },
    ModalProposal {
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
        requested_frame: u32,
    },
    ModalDecision {
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
        decision_frame: u32,
    },
    /// The retained rollback horizon cannot incorporate an input from this
    /// seat. A client drops its whole live QUIC session and re-handshakes; the
    /// host drops the named peer so that peer follows the same reconnect path.
    /// The fresh handshake always carries the host's latest full snapshot.
    ReconnectForSnapshot {
        player_id: PlayerId,
        reason: String,
    },
    /// Host-only escalation: every connected client must discard its local
    /// future and rejoin from the same current authoritative snapshot.
    ReconnectAllForSnapshot {
        reason: String,
    },
    BeginSnapshotTransition {
        id: SnapshotTransitionId,
        payload: SnapshotTransitionPayload,
    },
    SnapshotTransitionReady {
        id: SnapshotTransitionId,
    },
    /// Host-only, targeted ranked admission challenge.
    RankedJoinChallenge {
        to: PlayerId,
        challenge: RankedJoinChallenge,
    },
    /// Client-local trust gate. The expected canonical ranked configuration is
    /// never serialized; it only releases a matching staged challenge.
    ArmRankedJoin {
        expected_ranked_session: RankedSessionConfigDocument,
    },
    /// Client-only answer to the exact delivered challenge.
    RankedJoinResponse(RankedJoinResponse),
    /// Host-only, targeted acknowledgement of an admitted attestation.
    RankedJoinAccepted {
        to: PlayerId,
        accepted: RankedJoinAccepted,
    },
    /// Host-only broadcast of the complete roster after a fresh admission.
    RankedParticipantRoster {
        roster: RankedParticipantRosterDocument,
    },
    /// Host-only broadcast of an irreversible browse-only downgrade.
    RankedBrowseOnly {
        reason: RankedBrowseOnlyReason,
    },
    /// Host-only, targeted publication of a closed continuation/submission
    /// context.
    RankedCoSignContext {
        to: PlayerId,
        context: RankedCoSignContextDocument,
    },
    /// Host-only, targeted publication of a validated leaderboard queue
    /// acknowledgement to the exact admitted campaign controller.
    RankedSubmissionAccepted {
        to: PlayerId,
        accepted: RankedSubmissionAcceptedDocument,
    },
    /// Host-only broadcast of an authority-admitted setup without trusted
    /// time; clients apply their own pre-frame time before installation.
    RankedOfficialSessionSetup(RankedOfficialSessionSetupDocument),
    /// Host-only broadcast requesting an exact local active campaign receipt.
    RankedContinuationReceiptSelectionRequest(RankedContinuationReceiptSelectionRequestDocument),
    /// Client-only controller response to the exact selection request.
    RankedContinuationReceiptSelection(RankedContinuationReceiptSelectionDocument),
    /// Host-only request to the exact immutable campaign-controller seat.
    RankedContinuationPreflightClaim {
        to: PlayerId,
        claim: RankedContinuationPreflightClaimDocument,
    },
    /// Client-only response to a locally validated and staged preflight claim.
    RankedContinuationPreflightSignature(RankedContinuationPreflightSignatureDocument),
    /// Host-only, targeted publication. Requests for different participants
    /// may share the same protocol instance; authenticated `to` is transport
    /// routing state and never part of the signed request.
    LeaderboardCoSignRequest {
        to: PlayerId,
        request: LeaderboardCoSignRequestV1,
    },
    /// Client-local trust gate. This is consumed by the client transport and
    /// never serialized. An inbound host request is presented only when it is
    /// exactly equal to this independently derived request.
    ArmLeaderboardCoSignRequest {
        request: LeaderboardCoSignRequestV1,
    },
    /// Client-only response to the exact delivered request instance.
    LeaderboardCoSignResponse(LeaderboardCoSignResponse),
}

/// Channel pair + frame cursor held by the [`crate::engine_manager::EngineManager`].
pub struct NetChannels {
    pub outgoing: Sender<NetOutbound>,
    pub incoming: Receiver<NetEvent>,
    pub deferred_events: Arc<Mutex<std::collections::VecDeque<NetEvent>>>,
    pub frame_cursor: FrameCursor,
    /// Latest authoritative engine snapshot the host wants to share
    /// with newly-handshaking peers.  Set once after mission init via
    /// [`Self::set_initial_snapshot`]; the server's handshake handler
    /// reads it and sends `NetMsg::InitialSnapshot` to each new peer
    /// immediately after `Welcome`.
    pub initial_snapshot: InitialSnapshot,
    modal_sync: Mutex<ModalSyncState>,
    leaderboard_cosign_inbox: LeaderboardAuthorizationInbox,
    next_transition_sequence: AtomicU64,
}

impl NetChannels {
    /// Build the channels + cursor.  Returns `(NetChannels,
    /// incoming_tx, outgoing_rx, frame_cursor, snapshot_arc)`; the
    /// transport thread keeps the latter four.
    pub fn new() -> (
        Self,
        Sender<NetEvent>,
        Receiver<NetOutbound>,
        FrameCursor,
        InitialSnapshot,
    ) {
        let (out_tx, out_rx) = channel::<NetOutbound>();
        let (in_tx, in_rx) = channel::<NetEvent>();
        let cursor = new_frame_cursor();
        let snapshot = Arc::new(std::sync::Mutex::new(None));
        let deferred_events = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        (
            Self {
                outgoing: out_tx,
                incoming: in_rx,
                deferred_events,
                frame_cursor: Arc::clone(&cursor),
                initial_snapshot: Arc::clone(&snapshot),
                modal_sync: Mutex::new(ModalSyncState::default()),
                leaderboard_cosign_inbox: Arc::new(Mutex::new(std::collections::VecDeque::new())),
                next_transition_sequence: AtomicU64::new(0),
            },
            in_tx,
            out_rx,
            cursor,
            snapshot,
        )
    }

    /// Cache an initial-state snapshot the host will offer to every
    /// new peer that handshakes.
    pub fn set_initial_snapshot(&self, frame: u32, engine: &Engine) {
        if let Ok(mut slot) = self.initial_snapshot.lock() {
            *slot = Some((frame, engine.encode_native_snapshot()));
        }
    }

    /// Cache an authoritative host snapshot and push it to peers
    /// that already handshook before the cache was populated.
    pub fn publish_initial_snapshot(&self, frame: u32, engine: &Engine) {
        self.set_initial_snapshot(frame, engine);
        let engine_bytes = engine.encode_native_snapshot();
        let _ = self.outgoing.send(NetOutbound::InitialSnapshot {
            frame,
            engine_bytes,
        });
    }

    /// Announce that this process has loaded the mission, adopted any
    /// required initial snapshot, and is ready for the host-controlled
    /// sim start barrier.
    pub fn send_ready_to_sim(&self, frame: u32) {
        let _ = self.outgoing.send(NetOutbound::ReadyToSim { frame });
    }

    pub fn request_content(&self, full_mod_sha256: [u8; 32], resume_offset: u64) {
        let _ = self.outgoing.send(NetOutbound::ContentRequest {
            full_mod_sha256,
            resume_offset,
        });
    }

    pub fn reject_content(&self, full_mod_sha256: [u8; 32], reason: String) {
        let _ = self.outgoing.send(NetOutbound::ContentReject {
            full_mod_sha256,
            reason: bounded_safe_diagnostic(&reason, MAX_REJECT_REASON_BYTES),
        });
    }

    pub fn send_content_ready(&self, full_mod_sha256: [u8; 32]) {
        let _ = self
            .outgoing
            .send(NetOutbound::ContentReady { full_mod_sha256 });
    }

    pub fn send_content_prepared(&self, full_mod_sha256: [u8; 32]) {
        let _ = self
            .outgoing
            .send(NetOutbound::ContentPrepared { full_mod_sha256 });
    }

    /// Poll a network event, including events deferred by nested UI
    /// loops that only consumed modal-specific messages.
    pub fn try_recv_event(&self) -> Result<NetEvent, std::sync::mpsc::TryRecvError> {
        if let Ok(mut deferred) = self.deferred_events.lock()
            && let Some(event) = deferred.pop_front()
        {
            return Ok(event);
        }
        self.incoming.try_recv()
    }

    /// Poll only the transport receiver.  Modal loops use this to
    /// avoid repeatedly re-reading their own deferred events.
    pub fn try_recv_transport_event(&self) -> Result<NetEvent, std::sync::mpsc::TryRecvError> {
        self.incoming.try_recv()
    }

    /// Push events back in front of the main game-loop drain.
    pub fn defer_events(&self, events: Vec<NetEvent>) {
        if events.is_empty() {
            return;
        }
        if let Ok(mut deferred) = self.deferred_events.lock() {
            for event in events.into_iter().rev() {
                deferred.push_front(event);
            }
        }
    }

    /// Update the frame cursor.  Call once per tick from the game
    /// loop with the engine's `sim_frame`.
    pub fn publish_frame(&self, frame: u32) {
        self.frame_cursor.store(frame, Ordering::Relaxed);
    }

    pub fn current_frame(&self) -> u32 {
        self.frame_cursor.load(Ordering::Relaxed)
    }

    /// Install the host-generated session identity learned during Welcome.
    /// Reinstalling the same identity on reconnect is idempotent; changing it
    /// in-place is a fatal session mismatch.
    pub fn install_session_id(&self, session_id: MultiplayerSessionId) -> Result<(), String> {
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?;
        match sync.session_id {
            Some(current) if current != session_id => Err(format!(
                "multiplayer session identity changed from {current:?} to {session_id:?}"
            )),
            Some(_) => Ok(()),
            None => {
                sync.session_id = Some(session_id);
                Ok(())
            }
        }
    }

    pub fn session_id(&self) -> Result<MultiplayerSessionId, String> {
        self.modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?
            .session_id
            .ok_or_else(|| "multiplayer session identity is not installed".to_string())
    }

    /// Return the stable token for the currently open occurrence of `kind`, or
    /// allocate the next occurrence when this is a newly opened modal.
    pub fn open_modal_instance(&self, kind: &ModalKind) -> Result<ModalInstanceId, String> {
        let opened_frame = self.frame_cursor.load(Ordering::Relaxed);
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?;
        let session_id = sync
            .session_id
            .ok_or_else(|| "multiplayer session identity is not installed".to_string())?;
        if let Some(state) = sync
            .occurrences
            .iter_mut()
            .find(|state| state.kind == *kind)
        {
            if let Some(instance) = state.active {
                return Ok(instance);
            }
            state.next_occurrence = state
                .next_occurrence
                .checked_add(1)
                .ok_or_else(|| "multiplayer modal occurrence counter overflowed".to_string())?;
            let instance = ModalInstanceId {
                session_id,
                opened_frame,
                occurrence: state.next_occurrence,
            };
            state.active = Some(instance);
            return Ok(instance);
        }
        let instance = ModalInstanceId {
            session_id,
            opened_frame,
            occurrence: 1,
        };
        sync.occurrences.push(ModalOccurrenceState {
            kind: kind.clone(),
            next_occurrence: 1,
            active: Some(instance),
        });
        Ok(instance)
    }

    pub fn complete_modal_instance(
        &self,
        kind: &ModalKind,
        instance: ModalInstanceId,
    ) -> Result<(), String> {
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?;
        let state = sync
            .occurrences
            .iter_mut()
            .find(|state| state.kind == *kind)
            .ok_or_else(|| format!("no multiplayer modal occurrence exists for {kind:?}"))?;
        if state.active != Some(instance) {
            return Err(format!(
                "multiplayer modal completion mismatch for {kind:?}: active={:?}, completed={instance:?}",
                state.active
            ));
        }
        state.active = None;
        Ok(())
    }

    /// Route a modal event out of the ordinary simulation drain and into the
    /// presentation-side modal inbox.
    pub fn defer_modal_event(&self, event: NetEvent) -> Result<(), String> {
        if !matches!(
            event,
            NetEvent::ModalProposal { .. } | NetEvent::ModalDecision { .. }
        ) {
            return Err("attempted to route a non-modal event into the modal inbox".to_string());
        }
        self.modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?
            .inbox
            .push_back(event);
        Ok(())
    }

    pub fn try_recv_modal_event(&self) -> Result<NetEvent, std::sync::mpsc::TryRecvError> {
        if let Ok(mut sync) = self.modal_sync.lock()
            && let Some(event) = sync.inbox.pop_front()
        {
            return Ok(event);
        }
        self.try_recv_transport_event()
    }

    /// Clone the bounded authorization inbox for a capability-restricted
    /// mission-end port. This does not expose the raw transport receiver.
    pub fn leaderboard_authorization_inbox(&self) -> LeaderboardAuthorizationInbox {
        Arc::clone(&self.leaderboard_cosign_inbox)
    }

    /// Route a verified/armed co-sign event out of the simulation drain and
    /// into the mission-end controller's dedicated non-blocking inbox.
    pub fn defer_leaderboard_cosign_event(&self, event: NetEvent) -> Result<(), String> {
        if !matches!(
            event,
            NetEvent::RankedCoSignContext(_)
                | NetEvent::RankedSubmissionAccepted(_)
                | NetEvent::RankedOfficialSessionSetup(_)
                | NetEvent::RankedContinuationReceiptSelectionRequest(_)
                | NetEvent::RankedContinuationReceiptSelection { .. }
                | NetEvent::RankedContinuationPreflightClaim(_)
                | NetEvent::RankedContinuationPreflightSignature { .. }
                | NetEvent::LeaderboardCoSignRequest(_)
                | NetEvent::LeaderboardCoSignResponse { .. }
        ) {
            return Err(
                "attempted to route a non-leaderboard event into the co-sign inbox".to_string(),
            );
        }
        let mut inbox = self
            .leaderboard_cosign_inbox
            .lock()
            .map_err(|_| "multiplayer leaderboard co-sign inbox lock is poisoned".to_string())?;
        if inbox.len() >= MAX_LEADERBOARD_COSIGN_INBOX_EVENTS {
            return Err(format!(
                "multiplayer leaderboard co-sign inbox exceeds its {}-event limit",
                MAX_LEADERBOARD_COSIGN_INBOX_EVENTS
            ));
        }
        inbox.push_back(event);
        Ok(())
    }

    /// Poll the mission-end co-sign inbox. Unlike the modal helper, this does
    /// not read the raw transport receiver: the main simulation drain owns
    /// that receiver and explicitly routes only the four closed authorization
    /// event variants accepted by [`Self::defer_leaderboard_cosign_event`].
    pub fn try_recv_leaderboard_cosign_event(&self) -> Result<Option<NetEvent>, String> {
        Ok(self
            .leaderboard_cosign_inbox
            .lock()
            .map_err(|_| "multiplayer leaderboard co-sign inbox lock is poisoned".to_string())?
            .pop_front())
    }

    pub fn record_visible_modal_request(&self, request: VisibleModalRequest) -> Result<(), String> {
        self.modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?
            .visible_requests
            .push_back(request);
        Ok(())
    }

    pub fn take_visible_modal_requests(
        &self,
        instance: ModalInstanceId,
    ) -> Result<Vec<VisibleModalRequest>, String> {
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?;
        let mut matched = Vec::new();
        let mut retained = std::collections::VecDeque::new();
        while let Some(request) = sync.visible_requests.pop_front() {
            if request.instance == instance {
                matched.push(request);
            } else {
                retained.push_back(request);
            }
        }
        sync.visible_requests = retained;
        Ok(matched)
    }

    pub fn take_all_visible_modal_requests(&self) -> Result<Vec<VisibleModalRequest>, String> {
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?;
        Ok(sync.visible_requests.drain(..).collect())
    }

    /// Push a locally-produced [`PlayerCommand`] onto the wire.
    pub fn send_input(&self, cmd: PlayerCommand) {
        let origin_frame = self.frame_cursor.load(Ordering::Relaxed);
        let _ = self.outgoing.send(NetOutbound::Input {
            origin_frame,
            command: cmd,
        });
    }

    /// Push an authoritative state hash for `frame`.  Server-side only.
    pub fn send_state_hash(
        &self,
        frame: u32,
        hash: u64,
        clock_frame: u32,
        ms_until_next_frame: u32,
    ) {
        let _ = self.outgoing.send(NetOutbound::StateHash {
            frame,
            hash: Some(hash),
            clock_frame: Some(clock_frame),
            ms_until_next_frame: Some(ms_until_next_frame),
        });
    }

    /// Submit a visible client request without changing local modal state.
    /// Channel closure is an authoritative session failure and is propagated.
    pub fn propose_modal_dismiss(
        &self,
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
    ) -> Result<(), String> {
        let requested_frame = self.frame_cursor.load(Ordering::Relaxed);
        self.outgoing
            .send(NetOutbound::ModalProposal {
                instance,
                kind,
                result,
                requested_frame,
            })
            .map_err(|_| "multiplayer modal proposal channel is closed".to_string())
    }

    /// Publish the host's sole authoritative result for an exact modal.
    /// Channel closure is returned so the caller keeps the modal open instead
    /// of applying a local-only result.
    pub fn decide_modal_dismiss(
        &self,
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
    ) -> Result<(), String> {
        let decision_frame = self.frame_cursor.load(Ordering::Relaxed);
        self.outgoing
            .send(NetOutbound::ModalDecision {
                instance,
                kind,
                result,
                decision_frame,
            })
            .map_err(|_| "multiplayer modal decision channel is closed".to_string())
    }

    pub fn reconnect_for_snapshot(
        &self,
        player_id: PlayerId,
        reason: String,
    ) -> Result<(), String> {
        self.outgoing
            .send(NetOutbound::ReconnectForSnapshot { player_id, reason })
            .map_err(|_| "multiplayer snapshot reconnect channel is closed".to_string())
    }

    pub fn reconnect_all_for_snapshot(&self, reason: String) -> Result<(), String> {
        self.outgoing
            .send(NetOutbound::ReconnectAllForSnapshot { reason })
            .map_err(|_| "multiplayer snapshot reconnect channel is closed".to_string())
    }

    /// Begin a host-authoritative save/load transition. The payload is encoded
    /// by the caller exactly once and cloned unchanged to every peer.
    pub fn begin_snapshot_transition(
        &self,
        mission_id: u32,
        save_bytes: Vec<u8>,
    ) -> Result<SnapshotTransitionId, String> {
        let session_id = self.session_id()?;
        let sequence = self
            .next_transition_sequence
            .fetch_add(1, Ordering::Relaxed)
            .checked_add(1)
            .ok_or_else(|| "multiplayer snapshot transition counter overflowed".to_string())?;
        let id = SnapshotTransitionId {
            session_id,
            sequence,
        };
        self.outgoing
            .send(NetOutbound::BeginSnapshotTransition {
                id,
                payload: SnapshotTransitionPayload::Save {
                    mission_id,
                    save_bytes,
                },
            })
            .map_err(|_| "multiplayer snapshot transition channel is closed".to_string())?;
        Ok(id)
    }

    pub fn begin_campaign_exit_transition(
        &self,
        exit_code: crate::game_operation::GameCode,
        engine_bytes: Vec<u8>,
    ) -> Result<SnapshotTransitionId, String> {
        let session_id = self.session_id()?;
        let sequence = self
            .next_transition_sequence
            .fetch_add(1, Ordering::Relaxed)
            .checked_add(1)
            .ok_or_else(|| "multiplayer snapshot transition counter overflowed".to_string())?;
        let id = SnapshotTransitionId {
            session_id,
            sequence,
        };
        self.outgoing
            .send(NetOutbound::BeginSnapshotTransition {
                id,
                payload: SnapshotTransitionPayload::CampaignExit {
                    exit_code,
                    engine_bytes,
                },
            })
            .map_err(|_| "multiplayer campaign transition channel is closed".to_string())?;
        Ok(id)
    }

    pub fn acknowledge_snapshot_transition(&self, id: SnapshotTransitionId) -> Result<(), String> {
        self.outgoing
            .send(NetOutbound::SnapshotTransitionReady { id })
            .map_err(|_| "multiplayer snapshot transition channel is closed".to_string())
    }

    /// Arm one exact locally reconstructed leaderboard request before allowing
    /// the authenticated host's matching request to reach presentation code.
    pub fn arm_leaderboard_cosign_request(
        &self,
        request: LeaderboardCoSignRequestV1,
    ) -> Result<(), String> {
        request
            .validate()
            .map_err(|error| format!("invalid leaderboard co-sign request: {error}"))?;
        self.outgoing
            .send(NetOutbound::ArmLeaderboardCoSignRequest { request })
            .map_err(|_| "multiplayer leaderboard co-sign channel is closed".to_string())
    }

    /// Ask one authenticated guest seat to sign an exact purpose-bound
    /// leaderboard request. Host/controller signatures for seat zero remain a
    /// local signing operation and never make a network round trip.
    pub fn request_leaderboard_cosign(
        &self,
        to: PlayerId,
        request: LeaderboardCoSignRequestV1,
    ) -> Result<(), String> {
        if to == PlayerId::HOST {
            return Err("leaderboard co-sign requests to the host must be signed locally".into());
        }
        request
            .validate()
            .map_err(|error| format!("invalid leaderboard co-sign request: {error}"))?;
        self.outgoing
            .send(NetOutbound::LeaderboardCoSignRequest { to, request })
            .map_err(|_| "multiplayer leaderboard co-sign channel is closed".to_string())
    }

    pub fn respond_leaderboard_cosign(
        &self,
        response: LeaderboardCoSignResponse,
    ) -> Result<(), String> {
        if response.signer_public_key == [0; 32] || response.signature == [0; 64] {
            return Err("leaderboard co-sign response contains zero key material".into());
        }
        response
            .instance
            .validate()
            .map_err(|error| format!("invalid leaderboard co-sign response: {error}"))?;
        self.outgoing
            .send(NetOutbound::LeaderboardCoSignResponse(response))
            .map_err(|_| "multiplayer leaderboard co-sign channel is closed".to_string())
    }
}

/// Encode a [`NetMsg`] as a binary iroh-stream payload.
pub fn encode_msg(msg: &NetMsg) -> Vec<u8> {
    bitcode::encode(msg)
}

/// Decode and structurally validate a binary iroh-stream payload.
pub fn decode_msg(bytes: &[u8]) -> Result<NetMsg, String> {
    let message: NetMsg = bitcode::decode(bytes).map_err(|error| error.to_string())?;
    match &message {
        NetMsg::Hello {
            nickname,
            browser_auth,
            ranked_public_key,
            ..
        } => {
            validate_display_name(nickname)
                .map_err(|error| format!("invalid peer display name: {error}"))?;
            if let Some(auth) = browser_auth
                && (auth.join_code.len() > MAX_JOIN_CODE_BYTES
                    || auth.durable_public_key == [0; 32]
                    || auth.signature.len() != 64)
            {
                return Err("invalid bounded browser seat authentication".to_string());
            }
            if ranked_public_key.is_some_and(|public_key| public_key == [0; 32]) {
                return Err("invalid zero ranked public key".to_string());
            }
        }
        NetMsg::Welcome {
            your_seat,
            mission_id,
            host_nickname,
            session_id,
            sim_config,
            ..
        } => {
            if your_seat.0 == 0 || session_id.0 == [0; 32] {
                return Err("host sent invalid seat/session identity".to_string());
            }
            validate_mission_id(mission_id)
                .map_err(|error| format!("host sent invalid mission id: {error}"))?;
            validate_display_name(host_nickname)
                .map_err(|error| format!("invalid host display name: {error}"))?;
            sim_config
                .validate()
                .map_err(|error| format!("host sent invalid simulation configuration: {error}"))?;
        }
        NetMsg::ContentReject { reason, .. } | NetMsg::Reject { reason } => {
            validate_safe_display_text(
                "multiplayer rejection reason",
                reason,
                MAX_REJECT_REASON_BYTES,
            )?;
        }
        NetMsg::Note(note) => {
            validate_safe_display_text("multiplayer note", note, MAX_NOTE_BYTES)?;
        }
        NetMsg::InitialSnapshot { engine_bytes, .. }
            if engine_bytes.len() > MAX_SNAPSHOT_FRAME_BYTES =>
        {
            return Err("multiplayer snapshot exceeds its decoded size limit".to_string());
        }
        NetMsg::ReconnectRequired { reason }
            if reason.is_empty() || reason.len() > MAX_REJECT_REASON_BYTES =>
        {
            return Err("invalid multiplayer reconnect reason".to_string());
        }
        NetMsg::PrepareSnapshotTransition { payload, .. } => {
            let bytes = match payload {
                SnapshotTransitionPayload::Save { save_bytes, .. } => save_bytes,
                SnapshotTransitionPayload::CampaignExit { engine_bytes, .. } => engine_bytes,
            };
            if bytes.len() > MAX_SNAPSHOT_FRAME_BYTES {
                return Err(
                    "multiplayer transition snapshot exceeds its decoded size limit".into(),
                );
            }
        }
        NetMsg::RankedJoinChallenge(challenge) => challenge
            .validate()
            .map_err(|error| format!("invalid ranked join challenge: {error}"))?,
        NetMsg::RankedJoinResponse(response) => response
            .validate()
            .map_err(|error| format!("invalid ranked join response: {error}"))?,
        NetMsg::RankedJoinAccepted(accepted) => accepted
            .validate()
            .map_err(|error| format!("invalid ranked join acknowledgement: {error}"))?,
        NetMsg::RankedParticipantRoster(roster) => roster
            .validate()
            .map_err(|error| format!("invalid ranked participant roster: {error}"))?,
        NetMsg::RankedCoSignContext(context) => context
            .validate()
            .map_err(|error| format!("invalid ranked co-sign context: {error}"))?,
        NetMsg::RankedSubmissionAccepted(accepted) => accepted
            .validate()
            .map_err(|error| format!("invalid ranked submission acknowledgement: {error}"))?,
        NetMsg::RankedOfficialSessionSetup(setup) => setup
            .validate()
            .map_err(|error| format!("invalid official ranked session setup: {error}"))?,
        NetMsg::RankedContinuationReceiptSelectionRequest(request) => request
            .validate()
            .map_err(|error| format!("invalid continuation receipt selection request: {error}"))?,
        NetMsg::RankedContinuationReceiptSelection(selection) => selection
            .validate()
            .map_err(|error| format!("invalid continuation receipt selection: {error}"))?,
        NetMsg::RankedContinuationPreflightClaim(claim) => claim
            .validate()
            .map_err(|error| format!("invalid ranked continuation preflight claim: {error}"))?,
        NetMsg::RankedContinuationPreflightSignature(signature) => signature
            .validate()
            .map_err(|error| format!("invalid ranked continuation preflight signature: {error}"))?,
        NetMsg::LeaderboardCoSignRequest(request) => request
            .validate()
            .map_err(|error| format!("invalid leaderboard co-sign request: {error}"))?,
        NetMsg::LeaderboardCoSignResponse(response) => {
            response
                .instance
                .validate()
                .map_err(|error| format!("invalid leaderboard co-sign response: {error}"))?;
            if response.signer_public_key == [0; 32] || response.signature == [0; 64] {
                return Err("leaderboard co-sign response contains zero key material".into());
            }
        }
        _ => {}
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaderboard_request(
        purpose: robin_run_protocol::LeaderboardCoSignPurposeV1,
        byte: u8,
    ) -> LeaderboardCoSignRequestV1 {
        LeaderboardCoSignRequestV1 {
            instance: LeaderboardCoSignInstanceV1 {
                purpose,
                replay_session_id: robin_run_protocol::Digest32::from_bytes([byte; 32]),
                submission_offer_sha256: robin_run_protocol::Digest32::from_bytes(
                    [byte.wrapping_add(1); 32],
                ),
            },
            run_digest: robin_run_protocol::Digest32::from_bytes([byte.wrapping_add(2); 32]),
        }
    }

    #[test]
    fn protocol_version_includes_all_version_38_authority_rules() {
        // Version 38 combines typed nullable runtime handles, exact spatial
        // and save provenance, authenticated browser seats, exact-byte
        // prepare/ready/commit snapshot transitions, canonical speech timing,
        // rebalanced item rules, deterministic achievements, authoritative
        // Sherwood trading, resolved Legendary/Custom difficulty, and
        // deterministic authored timer/ambience state and commands, mission
        // diplomacy state and relationship-change commands, plus
        // authoritative combat-gesture rules and commands, completed planned
        // quick actions, per-seat shield prompts, and deterministic tactical
        // queue formations, and authoritative shared-vision fog state and
        // commands. Older peers fail before decoding incompatible wire or
        // snapshot bytes. Exact resumable full-mod admission and Spellforge
        // package identity are also part of the protocol contract, as are the
        // targeted authenticated leaderboard co-sign and official-ranked-
        // session messages.
        assert_eq!(NET_PROTOCOL_VERSION, 40);
    }

    #[test]
    fn netmsg_roundtrips() {
        let msg = NetMsg::BroadcastInput {
            server_frame: 40,
            origin_frame: 41,
            target_frame: 42,
            input: PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown),
        };
        let bytes = encode_msg(&msg);
        let back = decode_msg(&bytes).expect("decode");
        match back {
            NetMsg::BroadcastInput {
                server_frame,
                origin_frame,
                target_frame,
                input,
            } => {
                assert_eq!(server_frame, 40);
                assert_eq!(origin_frame, 41);
                assert_eq!(target_frame, 42);
                assert_eq!(input.player_id, PlayerId(2));
                assert!(matches!(input.command, PlayerCommand::CrouchDown));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn hello_welcome_roundtrips() {
        let hello = NetMsg::Hello {
            protocol_version: NET_PROTOCOL_VERSION,
            nickname: "alice".into(),
            browser_auth: None,
            ranked_public_key: Some([8; 32]),
        };
        let welcome = NetMsg::Welcome {
            your_seat: PlayerId(2),
            session_id: MultiplayerSessionId([7; 32]),
            mission_id: "Dem_Lei_MP".into(),
            mission_seed: 42,
            sim_config: crate::engine::SimConfig::default(),
            speech_timing_locale: Some("en-US".into()),
            host_nickname: "host".into(),
        };
        let h = decode_msg(&encode_msg(&hello)).unwrap();
        let w = decode_msg(&encode_msg(&welcome)).unwrap();
        match (h, w) {
            (
                NetMsg::Hello {
                    protocol_version,
                    nickname,
                    browser_auth,
                    ranked_public_key,
                },
                NetMsg::Welcome {
                    your_seat,
                    session_id,
                    mission_id,
                    mission_seed,
                    sim_config,
                    speech_timing_locale,
                    host_nickname,
                },
            ) => {
                assert_eq!(protocol_version, NET_PROTOCOL_VERSION);
                assert_eq!(nickname, "alice");
                assert!(browser_auth.is_none());
                assert_eq!(ranked_public_key, Some([8; 32]));
                assert_eq!(your_seat, PlayerId(2));
                assert_eq!(session_id, MultiplayerSessionId([7; 32]));
                assert_eq!(mission_id, "Dem_Lei_MP");
                assert_eq!(mission_seed, 42);
                assert_eq!(sim_config, crate::engine::SimConfig::default());
                assert_eq!(speech_timing_locale.as_deref(), Some("en-US"));
                assert_eq!(host_nickname, "host");
            }
            _ => panic!("wrong variants"),
        }
    }

    #[test]
    fn distributed_mod_admission_messages_roundtrip() {
        let offer = DistributedModOffer {
            schema_version: 1,
            full_mod_sha256: [1; 32],
            spellforge_package_sha256: Some([2; 32]),
            spellforge_vm_abi: Some("spellforge-v1-sha256:test".into()),
            encoded_bytes: 1234,
            mission_basename: "Mission".into(),
            mission_rhm_entry: "Data/Levels/Mission.rhm".into(),
            map_filename: "Map".into(),
            title: "Title".into(),
            claimed_author: "Author".into(),
            version: "1".into(),
            source_url: "https://example.invalid".into(),
            license: "CC0-1.0".into(),
            host_endpoint_id: "endpoint-key".into(),
        };
        offer.validate().unwrap();
        assert!(matches!(
            decode_msg(&encode_msg(&NetMsg::ContentOffer {
                offer: offer.clone()
            }))
            .unwrap(),
            NetMsg::ContentOffer { offer: decoded } if decoded == offer
        ));
        assert!(matches!(
            decode_msg(&encode_msg(&NetMsg::ContentChunk {
                full_mod_sha256: [1; 32],
                offset: 7,
                total_bytes: 10,
                bytes: vec![8, 9, 10],
            }))
            .unwrap(),
            NetMsg::ContentChunk { offset: 7, bytes, .. } if bytes == [8, 9, 10]
        ));
        assert!(matches!(
            decode_msg(&encode_msg(&NetMsg::ContentReady {
                full_mod_sha256: [1; 32]
            }))
            .unwrap(),
            NetMsg::ContentReady { full_mod_sha256 } if full_mod_sha256 == [1; 32]
        ));
        assert!(matches!(
            decode_msg(&encode_msg(&NetMsg::ContentPrepared {
                full_mod_sha256: [1; 32]
            }))
            .unwrap(),
            NetMsg::ContentPrepared { full_mod_sha256 } if full_mod_sha256 == [1; 32]
        ));

        for unsafe_license in [
            "CC0\nFull-mod SHA-256: fake",
            " padded",
            "padded ",
            "CC0\u{202e}fake",
            "CC0\u{2066}fake",
            "CC0\u{feff}fake",
        ] {
            let mut spoofed = offer.clone();
            spoofed.license = unsafe_license.into();
            assert!(spoofed.validate().is_err(), "accepted {unsafe_license:?}");
        }
    }

    #[test]
    fn safe_display_text_rejects_formatting_and_invisible_spoof_classes() {
        for character in [
            // Bidirectional controls.
            '\u{061c}',
            '\u{200e}',
            '\u{200f}',
            '\u{202a}',
            '\u{202e}',
            '\u{2066}',
            '\u{2069}',
            // Other Unicode Default_Ignorable_Code_Point classes.
            '\u{00ad}',
            '\u{034f}',
            '\u{180e}',
            '\u{200b}',
            '\u{2060}',
            '\u{3164}',
            '\u{fe0f}',
            '\u{feff}',
            '\u{e0100}',
            // Layout-breaking whitespace and a visible-as-blank glyph.
            '\u{00a0}',
            '\u{2028}',
            '\u{2800}',
        ] {
            let value = format!("safe{character}spoof");
            assert!(
                validate_safe_display_text("test", &value, 128).is_err(),
                "accepted U+{:04X}",
                character as u32
            );
        }
        assert!(validate_safe_display_text("test", "safe metadata", 128).is_ok());
    }

    #[test]
    fn bounded_diagnostics_escape_spoofs_and_enforce_utf8_byte_limits() {
        let diagnostic = bounded_safe_diagnostic("  host\nsaid \u{202e}no  ", 128);
        assert_eq!(diagnostic, r"host\u{a}said \u{202e}no");
        assert!(validate_safe_display_text("diagnostic", &diagnostic, 128).is_ok());

        let bounded = bounded_safe_diagnostic(&"🦊".repeat(1_024), 63);
        assert!(bounded.len() <= 63);
        assert!(bounded.ends_with('…'));
        assert!(bounded.is_char_boundary(bounded.len()));
        assert!(validate_safe_display_text("diagnostic", &bounded, 63).is_ok());
    }

    #[test]
    fn content_rejection_channel_never_emits_an_unsafe_or_oversized_reason() {
        let (channels, _incoming, outgoing, _cursor, _snapshot) = NetChannels::new();
        channels.reject_content([7; 32], format!("host\n{}", "🦊".repeat(1_024)));
        let NetOutbound::ContentReject {
            full_mod_sha256,
            reason,
        } = outgoing.recv().expect("content rejection")
        else {
            panic!("content rejection helper emitted the wrong outbound message");
        };
        assert_eq!(full_mod_sha256, [7; 32]);
        assert!(reason.contains(r"\u{a}"));
        assert!(reason.len() <= MAX_REJECT_REASON_BYTES);
        assert!(
            validate_safe_display_text(
                "multiplayer rejection reason",
                &reason,
                MAX_REJECT_REASON_BYTES
            )
            .is_ok()
        );
    }

    #[test]
    fn modal_proposal_and_decision_roundtrip_with_exact_identity() {
        let instance = ModalInstanceId {
            session_id: MultiplayerSessionId([9; 32]),
            opened_frame: 120,
            occurrence: 3,
        };
        let kind = ModalKind::Dialog { dialog_id: 44 };
        let proposal = decode_msg(&encode_msg(&NetMsg::ModalProposal {
            instance,
            kind: kind.clone(),
            result: DialogResult::Aborted,
            requested_frame: 123,
        }))
        .expect("decode proposal");
        let decision = decode_msg(&encode_msg(&NetMsg::ModalDecision {
            instance,
            kind: kind.clone(),
            result: DialogResult::Completed,
            decision_frame: 125,
        }))
        .expect("decode decision");

        assert!(matches!(
            proposal,
            NetMsg::ModalProposal {
                instance: decoded,
                kind: ModalKind::Dialog { dialog_id: 44 },
                result: DialogResult::Aborted,
                requested_frame: 123,
            } if decoded == instance
        ));
        assert!(matches!(
            decision,
            NetMsg::ModalDecision {
                instance: decoded,
                kind: ModalKind::Dialog { dialog_id: 44 },
                result: DialogResult::Completed,
                decision_frame: 125,
            } if decoded == instance
        ));
    }

    #[test]
    fn modal_instances_are_stable_until_completed_and_session_bound() {
        let (channels, _incoming, _outgoing, _cursor, _snapshot) = NetChannels::new();
        let session = MultiplayerSessionId([3; 32]);
        channels.install_session_id(session).unwrap();
        channels.publish_frame(17);
        let kind = ModalKind::SherwoodReport;

        let first = channels.open_modal_instance(&kind).unwrap();
        assert_eq!(channels.open_modal_instance(&kind).unwrap(), first);
        channels.complete_modal_instance(&kind, first).unwrap();
        channels.publish_frame(20);
        let second = channels.open_modal_instance(&kind).unwrap();

        assert_eq!(first.session_id, session);
        assert_eq!(first.opened_frame, 17);
        assert_eq!(first.occurrence, 1);
        assert_eq!(second.opened_frame, 20);
        assert_eq!(second.occurrence, 2);
        assert_ne!(first, second);
        assert!(
            channels
                .install_session_id(MultiplayerSessionId([4; 32]))
                .is_err()
        );
    }

    #[test]
    fn decode_rejects_unsafe_rejection_and_note_text() {
        for message in [
            NetMsg::Reject {
                reason: "trusted\u{202e}failure".into(),
            },
            NetMsg::ContentReject {
                full_mod_sha256: [1; 32],
                reason: "padded ".into(),
            },
            NetMsg::Note("line\nspoof".into()),
        ] {
            assert!(decode_msg(&encode_msg(&message)).is_err());
        }
        let oversized = NetMsg::ContentReject {
            full_mod_sha256: [1; 32],
            reason: "x".repeat(MAX_REJECT_REASON_BYTES + 1),
        };
        assert!(decode_msg(&encode_msg(&oversized)).is_err());
    }

    #[test]
    fn snapshot_reconnect_directive_roundtrips() {
        let reconnect = decode_msg(&encode_msg(&NetMsg::ReconnectRequired {
            reason: "rollback horizon".to_string(),
        }))
        .expect("decode reconnect directive");
        assert!(matches!(
            reconnect,
            NetMsg::ReconnectRequired { reason } if reason == "rollback horizon"
        ));
    }

    #[test]
    fn snapshot_transition_roundtrips_exact_bytes() {
        let id = SnapshotTransitionId {
            session_id: MultiplayerSessionId([8; 32]),
            sequence: 3,
        };
        let bytes = vec![0, 1, 2, 3, 254, 255];
        let decoded = decode_msg(&encode_msg(&NetMsg::PrepareSnapshotTransition {
            id,
            payload: SnapshotTransitionPayload::Save {
                mission_id: 42,
                save_bytes: bytes.clone(),
            },
        }))
        .expect("decode snapshot transition");
        assert!(matches!(
            decoded,
            NetMsg::PrepareSnapshotTransition {
                id: decoded_id,
                payload: SnapshotTransitionPayload::Save {
                    mission_id: 42,
                    save_bytes,
                },
            } if decoded_id == id && save_bytes == bytes
        ));
        assert!(matches!(
            decode_msg(&encode_msg(&NetMsg::SnapshotTransitionReady { id })).unwrap(),
            NetMsg::SnapshotTransitionReady { id: decoded_id } if decoded_id == id
        ));
        assert!(matches!(
            decode_msg(&encode_msg(&NetMsg::CommitSnapshotTransition { id })).unwrap(),
            NetMsg::CommitSnapshotTransition { id: decoded_id } if decoded_id == id
        ));
    }

    #[test]
    fn leaderboard_cosign_wire_roundtrips_exact_typed_request_and_response() {
        let request = leaderboard_request(
            robin_run_protocol::LeaderboardCoSignPurposeV1::CampaignContinuation,
            7,
        );
        let response = LeaderboardCoSignResponse {
            instance: request.instance,
            signer_public_key: [10; 32],
            signature: [11; 64],
        };

        assert!(matches!(
            decode_msg(&encode_msg(&NetMsg::LeaderboardCoSignRequest(request))).unwrap(),
            NetMsg::LeaderboardCoSignRequest(decoded) if decoded == request
        ));
        assert!(matches!(
            decode_msg(&encode_msg(&NetMsg::LeaderboardCoSignResponse(response.clone()))).unwrap(),
            NetMsg::LeaderboardCoSignResponse(decoded) if decoded == response
        ));
    }

    #[test]
    fn leaderboard_cosign_decode_rejects_zero_digest_key_and_signature() {
        let mut request = leaderboard_request(
            robin_run_protocol::LeaderboardCoSignPurposeV1::Submission,
            4,
        );
        request.run_digest = robin_run_protocol::Digest32::default();
        assert!(
            decode_msg(&encode_msg(&NetMsg::LeaderboardCoSignRequest(request)))
                .unwrap_err()
                .contains("run_digest")
        );

        let valid = leaderboard_request(
            robin_run_protocol::LeaderboardCoSignPurposeV1::Submission,
            5,
        );
        for response in [
            LeaderboardCoSignResponse {
                instance: valid.instance,
                signer_public_key: [0; 32],
                signature: [9; 64],
            },
            LeaderboardCoSignResponse {
                instance: valid.instance,
                signer_public_key: [9; 32],
                signature: [0; 64],
            },
        ] {
            assert!(
                decode_msg(&encode_msg(&NetMsg::LeaderboardCoSignResponse(response)))
                    .unwrap_err()
                    .contains("zero key material")
            );
        }
    }

    #[test]
    fn leaderboard_cosign_channel_api_keeps_target_and_local_arm_out_of_wire_payload() {
        let (channels, _incoming, outgoing, _cursor, _snapshot) = NetChannels::new();
        let continuation = leaderboard_request(
            robin_run_protocol::LeaderboardCoSignPurposeV1::CampaignContinuation,
            12,
        );
        channels
            .request_leaderboard_cosign(PlayerId(2), continuation)
            .unwrap();
        assert!(matches!(
            outgoing.recv().unwrap(),
            NetOutbound::LeaderboardCoSignRequest { to: PlayerId(2), request }
                if request == continuation
        ));
        assert!(
            channels
                .request_leaderboard_cosign(PlayerId::HOST, continuation)
                .unwrap_err()
                .contains("signed locally")
        );

        let submission = leaderboard_request(
            robin_run_protocol::LeaderboardCoSignPurposeV1::Submission,
            13,
        );
        channels.arm_leaderboard_cosign_request(submission).unwrap();
        assert!(matches!(
            outgoing.recv().unwrap(),
            NetOutbound::ArmLeaderboardCoSignRequest { request } if request == submission
        ));

        let response = LeaderboardCoSignResponse {
            instance: submission.instance,
            signer_public_key: [15; 32],
            signature: [16; 64],
        };
        channels
            .respond_leaderboard_cosign(response.clone())
            .unwrap();
        assert!(matches!(
            outgoing.recv().unwrap(),
            NetOutbound::LeaderboardCoSignResponse(decoded) if decoded == response
        ));
    }

    #[test]
    fn leaderboard_cosign_inbox_is_dedicated_ordered_and_fail_closed_at_bound() {
        let (channels, _incoming, _outgoing, _cursor, _snapshot) = NetChannels::new();
        let request = leaderboard_request(
            robin_run_protocol::LeaderboardCoSignPurposeV1::Submission,
            20,
        );
        assert!(
            channels
                .defer_leaderboard_cosign_event(NetEvent::Note("not authorization".into()))
                .unwrap_err()
                .contains("non-leaderboard")
        );

        for _ in 0..MAX_LEADERBOARD_COSIGN_INBOX_EVENTS {
            channels
                .defer_leaderboard_cosign_event(NetEvent::LeaderboardCoSignRequest(request))
                .unwrap();
        }
        assert!(
            channels
                .defer_leaderboard_cosign_event(NetEvent::LeaderboardCoSignRequest(request))
                .unwrap_err()
                .contains("event limit")
        );
        for _ in 0..MAX_LEADERBOARD_COSIGN_INBOX_EVENTS {
            assert!(matches!(
                channels.try_recv_leaderboard_cosign_event().unwrap(),
                Some(NetEvent::LeaderboardCoSignRequest(decoded)) if decoded == request
            ));
        }
        assert!(
            channels
                .try_recv_leaderboard_cosign_event()
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn host_transition_api_queues_exact_save_and_campaign_bytes() {
        let (channels, _incoming, outgoing, _cursor, _snapshot) = NetChannels::new();
        let session_id = MultiplayerSessionId([11; 32]);
        channels.install_session_id(session_id).unwrap();

        let save_bytes = vec![9, 8, 7, 6];
        let save_id = channels
            .begin_snapshot_transition(41, save_bytes.clone())
            .unwrap();
        assert_eq!(save_id.sequence, 1);
        assert!(matches!(
            outgoing.recv().unwrap(),
            NetOutbound::BeginSnapshotTransition {
                id,
                payload: SnapshotTransitionPayload::Save {
                    mission_id: 41,
                    save_bytes: actual,
                },
            } if id == save_id && actual == save_bytes
        ));

        let engine_bytes = vec![1, 3, 3, 7];
        let exit_id = channels
            .begin_campaign_exit_transition(
                crate::game_operation::GameCode::LevelInterrupted,
                engine_bytes.clone(),
            )
            .unwrap();
        assert_eq!(exit_id.sequence, 2);
        assert!(matches!(
            outgoing.recv().unwrap(),
            NetOutbound::BeginSnapshotTransition {
                id,
                payload: SnapshotTransitionPayload::CampaignExit {
                    exit_code: crate::game_operation::GameCode::LevelInterrupted,
                    engine_bytes: actual,
                },
            } if id == exit_id && actual == engine_bytes
        ));
    }

    #[test]
    fn decode_rejects_unsafe_or_oversized_wire_strings() {
        for nickname in ["", " padded", "bidi\u{202e}name"] {
            let encoded = encode_msg(&NetMsg::Hello {
                protocol_version: NET_PROTOCOL_VERSION,
                nickname: nickname.to_string(),
                browser_auth: None,
                ranked_public_key: None,
            });
            assert!(decode_msg(&encoded).is_err());
        }
        let encoded = encode_msg(&NetMsg::Note("x".repeat(MAX_NOTE_BYTES + 1)));
        assert!(decode_msg(&encoded).is_err());
    }

    #[test]
    fn browser_seat_proof_is_domain_and_endpoint_bound() {
        const DOMAIN: &[u8] = b"robinhood/browser-seat-proof/v1\0";
        let proof = browser_seat_proof_message([1; 32], [2; 32], [3; 32]);
        assert_eq!(proof.len(), DOMAIN.len() + 96);
        assert_eq!(&proof[..DOMAIN.len()], DOMAIN);
        assert_eq!(&proof[DOMAIN.len()..DOMAIN.len() + 32], &[1; 32]);
        assert_eq!(&proof[DOMAIN.len() + 32..DOMAIN.len() + 64], &[2; 32]);
        assert_eq!(&proof[DOMAIN.len() + 64..], &[3; 32]);
        assert_ne!(proof, browser_seat_proof_message([9; 32], [2; 32], [3; 32]));
        assert_ne!(proof, browser_seat_proof_message([1; 32], [9; 32], [3; 32]));
        assert_ne!(proof, browser_seat_proof_message([1; 32], [2; 32], [9; 32]));
    }

    #[test]
    fn quit_updates_roundtrip_the_resolved_difficulty() {
        let msg = NetMsg::Input {
            origin_frame: 73,
            command: PlayerCommand::ApplyQuitMissionUpdates {
                exit_code: crate::game_operation::GameCode::LevelSucceeded,
                difficulty: crate::player_profile::DifficultyLevel::Hard,
                completed_at_unix_seconds: None,
                campaign_run_nonce: Some(1),
            },
        };

        let decoded = decode_msg(&encode_msg(&msg)).expect("decode quit-update command");
        assert!(matches!(
            decoded,
            NetMsg::Input {
                origin_frame: 73,
                command: PlayerCommand::ApplyQuitMissionUpdates {
                    exit_code: crate::game_operation::GameCode::LevelSucceeded,
                    difficulty: crate::player_profile::DifficultyLevel::Hard,
                    completed_at_unix_seconds: None,
                    campaign_run_nonce: Some(1),
                },
            }
        ));
    }

    #[test]
    fn welcome_roundtrips_host_authoritative_custom_difficulty_rules() {
        let mut rules = crate::player_profile::DifficultyRules::MEDIUM;
        rules.enemy_fighting_percent = 175;
        rules.reaction_time_percent = 65;
        rules.legacy_level = crate::player_profile::LegacyDifficultyLevel::Hard;
        let sim_config = crate::engine::SimConfig {
            difficulty: crate::player_profile::DifficultyLevel::custom(rules).unwrap(),
            ..Default::default()
        };
        let msg = NetMsg::Welcome {
            your_seat: PlayerId(1),
            session_id: MultiplayerSessionId([19; 32]),
            mission_id: "custom".to_owned(),
            mission_seed: 19,
            sim_config,
            speech_timing_locale: Some("en-US".to_owned()),
            host_nickname: "host".to_owned(),
        };

        let decoded = decode_msg(&encode_msg(&msg)).expect("decode custom Welcome");
        assert!(matches!(
            decoded,
            NetMsg::Welcome {
                sim_config: decoded_config,
                speech_timing_locale: Some(locale),
                ..
            } if decoded_config == sim_config && locale == "en-US"
        ));
    }

    #[test]
    fn welcome_rejects_invalid_host_difficulty_rules() {
        let mut rules = crate::player_profile::DifficultyRules::MEDIUM;
        rules.enemy_fighting_percent = 0;
        // Construct the malformed wire value directly to verify the network
        // boundary; ordinary callers must use `DifficultyLevel::custom`.
        let sim_config = crate::engine::SimConfig {
            difficulty: crate::player_profile::DifficultyLevel::Custom(rules),
            ..Default::default()
        };
        let message = NetMsg::Welcome {
            your_seat: PlayerId(1),
            session_id: MultiplayerSessionId([1; 32]),
            mission_id: "invalid".to_owned(),
            mission_seed: 1,
            sim_config,
            speech_timing_locale: None,
            host_nickname: "host".to_owned(),
        };

        let error = decode_msg(&encode_msg(&message)).unwrap_err();
        assert!(error.contains("enemy_fighting_percent"));
    }

    #[test]
    fn resolved_drop_ale_route_roundtrips_over_bitcode_wire() {
        let route = crate::gate::RecordedGatePath {
            source_sector: crate::sector::SectorNumber::new(133),
            source_sector_index: crate::fast_find_grid::SectorIndex::new(57),
            source_layer: 11,
            outcome: crate::gate::RecordedGateOutcome::Success(vec![crate::gate::GatePathStep {
                door_index: crate::gate::DoorIndex::new(7).expect("valid door index"),
                direct: false,
            }]),
        };
        let msg = NetMsg::Input {
            origin_frame: 35_283,
            command: PlayerCommand::DropAleAt {
                actor: crate::element::EntityId::Pc(crate::entity_id::PcId(36)),
                target_pos: crate::coordinates::MapPoint::new(778.0, 1714.0),
                running: false,
                already_authorized: true,
                goal_override: Some((crate::sector::SectorNumber::new(0), 0)),
                goal_sector_index_override: crate::fast_find_grid::SectorIndex::new(0),
                recorded_gate_path: Some(route.clone()),
            },
        };

        let decoded = decode_msg(&encode_msg(&msg)).expect("decode resolved DropAle command");
        let NetMsg::Input {
            origin_frame: 35_283,
            command:
                PlayerCommand::DropAleAt {
                    actor,
                    target_pos,
                    running: false,
                    already_authorized: true,
                    goal_override: Some((goal_sector, 0)),
                    goal_sector_index_override,
                    recorded_gate_path: Some(decoded_route),
                },
        } = decoded
        else {
            panic!("resolved DropAle command must survive bitcode wire round-trip");
        };
        assert_eq!(
            actor,
            crate::element::EntityId::Pc(crate::entity_id::PcId(36))
        );
        assert_eq!(target_pos.x.to_bits(), 778.0_f32.to_bits());
        assert_eq!(target_pos.y.to_bits(), 1714.0_f32.to_bits());
        assert_eq!(goal_sector, crate::sector::SectorNumber::new(0));
        assert_eq!(
            goal_sector_index_override,
            crate::fast_find_grid::SectorIndex::new(0)
        );
        assert_eq!(decoded_route, route);
    }
}
