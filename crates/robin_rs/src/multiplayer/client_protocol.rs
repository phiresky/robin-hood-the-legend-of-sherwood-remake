//! Transport-independent framing and client admission decisions.
//!
//! Adapters own streams, deadlines and cancellation. This core owns byte limits,
//! message ordering and authenticated metadata checks; it never emits simulation
//! events or acknowledges content before the adapter has prepared it.
//!
//! TODO: Move later gameplay/ranked transitions here in separate slices. Native
//! unresolved ranked BeginSim downgrades to browse-only; browser rejects it.
//! Those intentional adapter policies must not be accidentally normalized.

use super::{
    InboundFramePolicy, MultiplayerSessionId, NetFrameClass, NetMsg, decode_msg, encode_msg,
    net_frame_class,
};
use robin_engine::{engine::SimConfig, multiplayer::DistributedModOffer, player_command::PlayerId};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests {
    use super::*;

    fn offer() -> DistributedModOffer {
        DistributedModOffer {
            schema_version: 1,
            full_mod_sha256: [1; 32],
            spellforge_package_sha256: None,
            spellforge_vm_abi: None,
            encoded_bytes: 1,
            mission_basename: "Mission".into(),
            mission_rhm_entry: "Data/Levels/Mission.rhm".into(),
            map_filename: "Mission".into(),
            title: "Mission".into(),
            claimed_author: "Author".into(),
            version: "1".into(),
            source_url: "https://example.invalid".into(),
            license: "CC0-1.0".into(),
            host_endpoint_id: "endpoint-key".into(),
        }
    }

    fn welcome() -> NetMsg {
        NetMsg::Welcome {
            your_seat: PlayerId(1),
            mission_id: "Mission".into(),
            mission_seed: 7,
            sim_config: SimConfig::default(),
            speech_timing_locale: Some("eng".into()),
            session_id: MultiplayerSessionId([2; 32]),
            host_nickname: "Host".into(),
        }
    }

    #[test]
    fn every_direction_checks_exact_limit_before_allocation() {
        for policy in [
            InboundFramePolicy::ClientHello,
            InboundFramePolicy::ClientToServer,
            InboundFramePolicy::ServerToClient,
        ] {
            for class in [
                NetFrameClass::Control,
                NetFrameClass::Input,
                NetFrameClass::Snapshot,
                NetFrameClass::Content,
            ] {
                let mut header = [0; 5];
                header[0] = class as u8;
                match policy.limit(class) {
                    Some(limit) => {
                        header[1..].copy_from_slice(&(limit as u32).to_le_bytes());
                        assert_eq!(decode_header(header, policy).unwrap(), (class, limit));
                        header[1..].copy_from_slice(&((limit + 1) as u32).to_le_bytes());
                        assert!(decode_header(header, policy).is_err());
                    }
                    None => assert!(decode_header(header, policy).is_err()),
                }
            }
            assert!(decode_header([255, 0, 0, 0, 0], policy).is_err());
        }
    }

    #[test]
    fn frame_codec_preserves_wire_bytes_and_rejects_misclassification() {
        let message = welcome();
        let (header, bytes) = encode_frame(&message).unwrap();
        assert_eq!(bytes, encode_msg(&message));
        assert_eq!(header[0], NetFrameClass::Control as u8);
        assert_eq!(&header[1..], &(bytes.len() as u32).to_le_bytes());
        let (class, length) = decode_header(header, InboundFramePolicy::ServerToClient).unwrap();
        assert_eq!(length, bytes.len());
        assert_eq!(encode_msg(&decode_body(class, &bytes).unwrap()), bytes);
        assert!(decode_body(NetFrameClass::Input, &bytes).is_err());
        assert!(decode_body(class, &bytes[..bytes.len() - 1]).is_err());
        assert!(decode_body(class, &[]).is_err());
    }

    #[test]
    fn shared_native_and_browser_content_admission_trace() {
        // The only prelude policy difference is the signed browser invitation.
        for expected in [None, Some(MultiplayerSessionId([2; 32]))] {
            let mut state = ClientHandshake::new("endpoint-key".into(), expected);
            assert!(matches!(
                state
                    .receive(Some(NetMsg::ContentOffer { offer: offer() }))
                    .unwrap(),
                HandshakeAction::PrepareContent(_)
            ));
            assert_eq!(state.phase, HandshakePhase::ContentPending);
            state.content_ready().unwrap();
            assert!(matches!(
                state.receive(Some(welcome())).unwrap(),
                HandshakeAction::Welcome(_)
            ));
            assert_eq!(state.phase, HandshakePhase::Complete);
            assert!(
                state.receive(Some(welcome())).is_err(),
                "late Welcome must not readmit"
            );
        }
    }

    #[test]
    fn prelude_rejects_reordered_closed_and_late_events() {
        for expected in [None, Some(MultiplayerSessionId([2; 32]))] {
            for unexpected in [
                None,
                Some(NetMsg::Note("too early".into())),
                Some(NetMsg::Reject {
                    reason: "denied".into(),
                }),
            ] {
                let mut state = ClientHandshake::new("endpoint-key".into(), expected);
                assert!(state.receive(unexpected).is_err());
                assert!(
                    state.receive(Some(welcome())).is_err(),
                    "failure is terminal"
                );
            }
            let mut state = ClientHandshake::new("endpoint-key".into(), expected);
            assert!(state.content_ready().is_err());
            assert!(state.receive(Some(welcome())).is_err());
            let mut state = ClientHandshake::new("endpoint-key".into(), expected);
            state
                .receive(Some(NetMsg::ContentOffer { offer: offer() }))
                .unwrap();
            assert!(
                state.receive(Some(welcome())).is_err(),
                "Welcome cannot bypass content readiness"
            );
            let mut state = ClientHandshake::new("endpoint-key".into(), expected);
            state
                .receive(Some(NetMsg::ContentOffer { offer: offer() }))
                .unwrap();
            state.content_ready().unwrap();
            assert!(
                state
                    .receive(Some(NetMsg::ContentOffer { offer: offer() }))
                    .is_err(),
                "a second offer cannot replace mounted content"
            );
        }
    }

    #[test]
    fn authenticated_host_and_signed_invitation_checks_survive_content_phase() {
        let mut state = ClientHandshake::new("other-endpoint".into(), None);
        assert!(
            state
                .receive(Some(NetMsg::ContentOffer { offer: offer() }))
                .is_err()
        );
        let mut invalid = offer();
        invalid.encoded_bytes = 0;
        let mut state = ClientHandshake::new("endpoint-key".into(), None);
        assert!(
            state
                .receive(Some(NetMsg::ContentOffer { offer: invalid }))
                .is_err()
        );
        for content in [false, true] {
            let mut state =
                ClientHandshake::new("endpoint-key".into(), Some(MultiplayerSessionId([3; 32])));
            if content {
                state
                    .receive(Some(NetMsg::ContentOffer { offer: offer() }))
                    .unwrap();
                state.content_ready().unwrap();
            }
            assert!(state.receive(Some(welcome())).is_err());
        }
    }

    #[test]
    fn reconnect_requires_exact_offer_not_just_same_content_hash() {
        let admitted = offer();
        assert!(validate_reconnect_content(None, None).is_ok());
        assert!(validate_reconnect_content(Some(&admitted), Some(&admitted)).is_ok());
        assert!(validate_reconnect_content(None, Some(&admitted)).is_err());
        assert!(validate_reconnect_content(Some(&admitted), None).is_err());
        let mut changed = admitted.clone();
        changed.title.push('!');
        assert!(validate_reconnect_content(Some(&changed), Some(&admitted)).is_err());
        changed = admitted.clone();
        changed.full_mod_sha256 = [9; 32];
        assert!(validate_reconnect_content(Some(&changed), Some(&admitted)).is_err());
    }

    #[test]
    fn reconnect_rejects_each_authoritative_metadata_change() {
        let HandshakeAction::Welcome(expected) = ClientHandshake::new("endpoint-key".into(), None)
            .receive(Some(welcome()))
            .unwrap()
        else {
            panic!("expected Welcome")
        };
        let check = |actual: &WelcomeData| {
            validate_reconnect_state(
                expected.seat,
                &expected.mission_id,
                expected.mission_seed,
                expected.sim_config,
                expected.speech_timing_locale.as_deref(),
                expected.session_id,
                actual.seat,
                &actual.mission_id,
                actual.mission_seed,
                actual.sim_config,
                actual.speech_timing_locale.as_deref(),
                actual.session_id,
            )
        };
        assert!(check(&expected).is_ok());
        for field in 0..6 {
            let mut actual = expected.clone();
            match field {
                0 => actual.seat = PlayerId(2),
                1 => actual.mission_id.push('!'),
                2 => actual.mission_seed += 1,
                3 => actual.sim_config.fog_of_war = !actual.sim_config.fog_of_war,
                4 => actual.speech_timing_locale = None,
                5 => actual.session_id = MultiplayerSessionId([3; 32]),
                _ => unreachable!(),
            }
            assert!(check(&actual).is_err(), "metadata field {field}");
        }
    }
}

pub(super) fn encode_frame(message: &NetMsg) -> Result<([u8; 5], Vec<u8>), String> {
    let bytes = encode_msg(message);
    let class = net_frame_class(message);
    if bytes.len() > class.absolute_limit() {
        return Err(format!(
            "outbound {class:?} frame of {} bytes exceeds {}-byte limit",
            bytes.len(),
            class.absolute_limit()
        ));
    }
    let len = u32::try_from(bytes.len()).map_err(|_| "outbound frame exceeds u32".to_string())?;
    let mut header = [0; 5];
    header[0] = class as u8;
    header[1..].copy_from_slice(&len.to_le_bytes());
    Ok((header, bytes))
}

/// Must be called before allocating a body, on both transport implementations.
pub(super) fn decode_header(
    header: [u8; 5],
    policy: InboundFramePolicy,
) -> Result<(NetFrameClass, usize), String> {
    let class = NetFrameClass::from_byte(header[0])?;
    let len = u32::from_le_bytes(header[1..].try_into().expect("four-byte frame length")) as usize;
    let limit = policy
        .limit(class)
        .ok_or_else(|| format!("{policy:?} may not send {class:?} frames"))?;
    if len > limit {
        return Err(format!(
            "inbound {class:?} frame of {len} bytes exceeds {limit}-byte {policy:?} limit"
        ));
    }
    Ok((class, len))
}

pub(super) fn decode_body(class: NetFrameClass, bytes: &[u8]) -> Result<NetMsg, String> {
    let message = decode_msg(bytes).map_err(|error| format!("decode frame: {error}"))?;
    if net_frame_class(&message) != class {
        return Err(format!(
            "declared {class:?} frame decoded as {:?}",
            net_frame_class(&message)
        ));
    }
    Ok(message)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WelcomeData {
    pub(super) seat: PlayerId,
    pub(super) mission_id: String,
    pub(super) mission_seed: u64,
    pub(super) sim_config: SimConfig,
    pub(super) speech_timing_locale: Option<String>,
    pub(super) session_id: MultiplayerSessionId,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) enum HandshakeAction {
    Welcome(WelcomeData),
    PrepareContent(DistributedModOffer),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum HandshakePhase {
    Prelude,
    ContentPending,
    Welcome,
    Complete,
    Failed,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct ClientHandshake {
    phase: HandshakePhase,
    authenticated_host: String,
    expected_session: Option<MultiplayerSessionId>,
}

impl ClientHandshake {
    pub(super) fn new(
        authenticated_host: String,
        expected_session: Option<MultiplayerSessionId>,
    ) -> Self {
        Self {
            phase: HandshakePhase::Prelude,
            authenticated_host,
            expected_session,
        }
    }

    /// Advance only after the platform's existing content admission flow has
    /// sent ContentReady. This does not mount content or bypass that flow.
    pub(super) fn content_ready(&mut self) -> Result<(), String> {
        if self.phase != HandshakePhase::ContentPending {
            let phase = self.phase;
            self.phase = HandshakePhase::Failed;
            return Err(format!(
                "content readiness received in {phase:?} handshake phase"
            ));
        }
        self.phase = HandshakePhase::Welcome;
        Ok(())
    }

    pub(super) fn receive(&mut self, message: Option<NetMsg>) -> Result<HandshakeAction, String> {
        let result = self.receive_inner(message);
        if result.is_err() {
            self.phase = HandshakePhase::Failed;
        }
        result
    }

    fn receive_inner(&mut self, message: Option<NetMsg>) -> Result<HandshakeAction, String> {
        if !matches!(
            self.phase,
            HandshakePhase::Prelude | HandshakePhase::Welcome
        ) {
            return Err(format!(
                "handshake message received in {:?} phase",
                self.phase
            ));
        }
        match message {
            Some(NetMsg::Welcome {
                your_seat,
                mission_id,
                mission_seed,
                sim_config,
                speech_timing_locale,
                session_id,
                host_nickname,
            }) => {
                if self
                    .expected_session
                    .is_some_and(|expected| expected != session_id)
                {
                    return Err("host Welcome session does not match the signed invitation".into());
                }
                tracing::info!(?your_seat, seed = mission_seed, host = %host_nickname, "received authoritative multiplayer Welcome");
                self.phase = HandshakePhase::Complete;
                Ok(HandshakeAction::Welcome(WelcomeData {
                    seat: your_seat,
                    mission_id,
                    mission_seed,
                    sim_config,
                    speech_timing_locale,
                    session_id,
                }))
            }
            Some(NetMsg::ContentOffer { offer }) if self.phase == HandshakePhase::Prelude => {
                offer
                    .validate()
                    .map_err(|error| format!("invalid distributed-mod offer: {error}"))?;
                if offer.host_endpoint_id != self.authenticated_host {
                    return Err(format!(
                        "distributed-mod offer claims host `{}`, but the authenticated iroh endpoint is `{}`",
                        offer.host_endpoint_id, self.authenticated_host
                    ));
                }
                self.phase = HandshakePhase::ContentPending;
                Ok(HandshakeAction::PrepareContent(offer))
            }
            Some(NetMsg::Reject { reason }) => Err(format!("host rejected connection: {reason}")),
            Some(other) => Err(format!(
                "unexpected message in {:?} handshake phase: {other:?}",
                self.phase
            )),
            None => Err(format!(
                "connection closed in {:?} handshake phase",
                self.phase
            )),
        }
    }
}

/// Reconnect may skip byte transfer only for the identical mounted offer.
pub(super) fn validate_reconnect_content(
    actual: Option<&DistributedModOffer>,
    admitted: Option<&DistributedModOffer>,
) -> Result<(), String> {
    match (actual, admitted) {
        (None, None) => Ok(()),
        (Some(actual), Some(expected)) if actual == expected => Ok(()),
        (Some(actual), Some(expected)) => Err(format!(
            "reconnect host content changed from {} to {}",
            robin_engine::spellforge::hex_hash(&expected.full_mod_sha256),
            robin_engine::spellforge::hex_hash(&actual.full_mod_sha256)
        )),
        (Some(actual), None) => Err(format!(
            "reconnect unexpectedly introduced host content {}",
            robin_engine::spellforge::hex_hash(&actual.full_mod_sha256)
        )),
        (None, Some(expected)) => Err(format!(
            "reconnect omitted previously admitted host content {}",
            robin_engine::spellforge::hex_hash(&expected.full_mod_sha256)
        )),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_reconnect_state(
    expected_seat: PlayerId,
    expected_mission_id: &str,
    expected_seed: u64,
    expected_config: SimConfig,
    expected_speech_timing_locale: Option<&str>,
    expected_session_id: MultiplayerSessionId,
    seat: PlayerId,
    mission_id: &str,
    seed: u64,
    config: SimConfig,
    speech_timing_locale: Option<&str>,
    session_id: MultiplayerSessionId,
) -> Result<(), String> {
    if seat != expected_seat
        || mission_id != expected_mission_id
        || seed != expected_seed
        || config != expected_config
        || speech_timing_locale != expected_speech_timing_locale
        || session_id != expected_session_id
    {
        return Err(format!(
            "reconnect joined incompatible seat {seat:?} mission `{mission_id}` seed {seed} config {config:?} speech timing {speech_timing_locale:?} session {session_id:?}; expected seat {expected_seat:?} mission `{expected_mission_id}` seed {expected_seed} config {expected_config:?} speech timing {expected_speech_timing_locale:?} session {expected_session_id:?}"
        ));
    }
    Ok(())
}
