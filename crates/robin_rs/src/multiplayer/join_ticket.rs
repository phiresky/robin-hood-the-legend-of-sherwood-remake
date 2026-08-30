//! Canonical, host-signed browser multiplayer invitations.
//!
//! A ticket is public bootstrap data, not a bearer authentication secret. Its
//! signature binds the exact host endpoint, HTTPS relay, build, content
//! edition, mission, session, and 30-minute invitation window. iroh still
//! authenticates the endpoint at connection time.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iroh::{EndpointAddr, EndpointId, PublicKey, RelayUrl, SecretKey, Signature, TransportAddr};
use serde::{Deserialize, Serialize};

use super::NET_PROTOCOL_VERSION;

pub const JOIN_CODE_PREFIX: &str = "rhmp2-";
pub const JOIN_TICKET_SCHEMA: u32 = 2;
pub const IROH_RELAY_TRANSPORT: &str = "iroh-relay-websocket";
pub const MAX_JOIN_CODE_BYTES: usize = 16 * 1024;
pub const INVITATION_LIFETIME_SECS: u64 = 30 * 60;
pub const MAX_CLOCK_SKEW_SECS: u64 = 2 * 60;
pub const DEFAULT_BROWSER_URL: &str = "https://robinhood.phiresky.xyz/";
pub const MAX_MULTIPLAYER_PLAYERS: u32 = 4;

const SIGNING_DOMAIN: &[u8] = b"robinhood/browser-join-ticket/v2\0";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BrowserContentEdition {
    Demo,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BrowserJoinTicketPayload {
    pub schema: u32,
    pub transport: String,
    pub net_protocol: u32,
    pub engine_version: String,
    pub host_endpoint_id: String,
    pub host_public_key: String,
    pub relay_url: String,
    pub session_id: String,
    pub issued_at_epoch_s: u64,
    pub expires_at_epoch_s: u64,
    pub content_edition: BrowserContentEdition,
    pub mission_id: String,
    pub mission_profile_id: Option<u32>,
    pub expected_players: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserJoinTicket {
    payload: BrowserJoinTicketPayload,
    canonical_payload: Vec<u8>,
    signature: Signature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvitationUse {
    /// A previously unused invitation. Its wall-clock window is mandatory.
    Initial,
    /// A durable identity already redeemed this invitation on this host.
    /// Reconnect does not silently mint a new invitation lifetime.
    RedeemedReconnect,
}

impl BrowserJoinTicket {
    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        host_key: &SecretKey,
        endpoint_addr: &EndpointAddr,
        session_id: [u8; 32],
        issued_at_epoch_s: u64,
        content_edition: BrowserContentEdition,
        mission_id: String,
        mission_profile_id: Option<u32>,
        expected_players: u32,
    ) -> Result<Self, String> {
        if endpoint_addr.id != host_key.public() {
            return Err("browser join ticket host key does not own the advertised endpoint".into());
        }
        let relay_url = endpoint_addr
            .relay_urls()
            .next()
            .ok_or_else(|| {
                "browser join ticket requires an iroh relay route; the host is not relay-online"
                    .to_string()
            })?
            .to_string();
        let expires_at_epoch_s = issued_at_epoch_s
            .checked_add(INVITATION_LIFETIME_SECS)
            .ok_or_else(|| "browser invitation timestamp overflow".to_string())?;
        let payload = BrowserJoinTicketPayload {
            schema: JOIN_TICKET_SCHEMA,
            transport: IROH_RELAY_TRANSPORT.to_string(),
            net_protocol: NET_PROTOCOL_VERSION,
            engine_version: crate::replay_format::ENGINE_SOURCE_COMMIT.to_string(),
            host_endpoint_id: endpoint_addr.id.to_string(),
            host_public_key: URL_SAFE_NO_PAD.encode(endpoint_addr.id.as_bytes()),
            relay_url,
            session_id: URL_SAFE_NO_PAD.encode(session_id),
            issued_at_epoch_s,
            expires_at_epoch_s,
            content_edition,
            mission_id,
            mission_profile_id,
            expected_players,
        };
        Self::sign(host_key, payload)
    }

    fn sign(host_key: &SecretKey, payload: BrowserJoinTicketPayload) -> Result<Self, String> {
        validate_static_payload(&payload)?;
        let canonical_payload = canonical_payload_bytes(&payload)?;
        let signature = host_key.sign(&signing_message(&canonical_payload));
        Ok(Self {
            payload,
            canonical_payload,
            signature,
        })
    }

    pub fn encode(&self) -> String {
        format!(
            "{JOIN_CODE_PREFIX}{}.{}",
            URL_SAFE_NO_PAD.encode(&self.canonical_payload),
            URL_SAFE_NO_PAD.encode(self.signature.to_bytes())
        )
    }

    /// Decode and authenticate all non-temporal ticket fields. Call
    /// [`Self::validate_use_at`] before using its mission or relay.
    pub fn decode_authenticated(encoded: &str) -> Result<Self, String> {
        let encoded = encoded.trim();
        if encoded.len() > MAX_JOIN_CODE_BYTES {
            return Err(format!(
                "browser join code exceeds the {MAX_JOIN_CODE_BYTES}-byte safety limit"
            ));
        }
        let envelope = encoded
            .strip_prefix(JOIN_CODE_PREFIX)
            .ok_or_else(|| format!("browser join code must start with `{JOIN_CODE_PREFIX}`"))?;
        let (payload_part, signature_part) = envelope
            .split_once('.')
            .ok_or_else(|| "browser join code is missing its host signature".to_string())?;
        if payload_part.is_empty() || signature_part.is_empty() || signature_part.contains('.') {
            return Err("browser join code has a malformed signed envelope".to_string());
        }
        let canonical_payload = URL_SAFE_NO_PAD
            .decode(payload_part)
            .map_err(|error| format!("decode browser join ticket payload: {error}"))?;
        // Refuse alternate encodings of the signed bytes. This keeps one exact
        // artifact representation for shell parsing, tests, and sharing.
        if URL_SAFE_NO_PAD.encode(&canonical_payload) != payload_part {
            return Err("browser join ticket payload is not canonical base64url".to_string());
        }
        let signature_bytes = URL_SAFE_NO_PAD
            .decode(signature_part)
            .map_err(|error| format!("decode browser join ticket signature: {error}"))?;
        let signature_bytes: [u8; Signature::LENGTH] = signature_bytes
            .try_into()
            .map_err(|_| "browser join ticket signature must be 64 bytes".to_string())?;
        if URL_SAFE_NO_PAD.encode(signature_bytes) != signature_part {
            return Err("browser join ticket signature is not canonical base64url".to_string());
        }
        let payload: BrowserJoinTicketPayload = serde_json::from_slice(&canonical_payload)
            .map_err(|error| format!("parse browser join ticket: {error}"))?;
        validate_static_payload(&payload)?;
        if canonical_payload_bytes(&payload)? != canonical_payload {
            return Err("browser join ticket JSON is not canonical".to_string());
        }
        let host = payload
            .host_endpoint_id
            .parse::<PublicKey>()
            .map_err(|error| format!("invalid host endpoint id: {error}"))?;
        let signature = Signature::from_bytes(&signature_bytes);
        host.verify(&signing_message(&canonical_payload), &signature)
            .map_err(|_| "browser join ticket host signature is invalid".to_string())?;
        Ok(Self {
            payload,
            canonical_payload,
            signature,
        })
    }

    pub fn decode_for_initial_use(encoded: &str, now_epoch_s: u64) -> Result<Self, String> {
        let ticket = Self::decode_authenticated(encoded)?;
        ticket.validate_use_at(now_epoch_s, InvitationUse::Initial)?;
        Ok(ticket)
    }

    pub fn validate_use_at(&self, now_epoch_s: u64, use_kind: InvitationUse) -> Result<(), String> {
        if self.payload.issued_at_epoch_s > now_epoch_s.saturating_add(MAX_CLOCK_SKEW_SECS) {
            return Err("browser invitation was issued too far in the future".to_string());
        }
        if use_kind == InvitationUse::Initial && now_epoch_s >= self.payload.expires_at_epoch_s {
            return Err("browser invitation expired before first use".to_string());
        }
        Ok(())
    }

    pub fn payload(&self) -> &BrowserJoinTicketPayload {
        &self.payload
    }

    pub fn session_id(&self) -> Result<[u8; 32], String> {
        decode_32("session id", &self.payload.session_id)
    }

    pub fn endpoint_addr(&self) -> Result<EndpointAddr, String> {
        let endpoint_id = self
            .payload
            .host_endpoint_id
            .parse::<EndpointId>()
            .map_err(|error| format!("invalid host endpoint id: {error}"))?;
        let relay = self
            .payload
            .relay_url
            .parse::<RelayUrl>()
            .map_err(|error| format!("invalid iroh relay URL: {error}"))?;
        Ok(EndpointAddr::from_parts(
            endpoint_id,
            [TransportAddr::Relay(relay)],
        ))
    }

    pub fn share_url(&self, browser_base_url: &str) -> Result<String, String> {
        let mut url = url::Url::parse(browser_base_url)
            .map_err(|error| format!("invalid browser multiplayer base URL: {error}"))?;
        if url.scheme() != "https" {
            return Err("browser multiplayer share URL must use HTTPS".to_string());
        }
        // Fragment data is not sent in HTTP requests or Referrer headers. The
        // stable shell captures it once and immediately replaces browser
        // history before loading an artifact.
        url.set_fragment(Some(&format!("join={}", self.encode())));
        Ok(url.into())
    }
}

fn signing_message(canonical_payload: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(SIGNING_DOMAIN.len() + canonical_payload.len());
    message.extend_from_slice(SIGNING_DOMAIN);
    message.extend_from_slice(canonical_payload);
    message
}

fn canonical_payload_bytes(payload: &BrowserJoinTicketPayload) -> Result<Vec<u8>, String> {
    serde_json::to_vec(payload).map_err(|error| format!("serialize browser join ticket: {error}"))
}

fn decode_32(label: &str, encoded: &str) -> Result<[u8; 32], String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|error| format!("invalid browser join ticket {label}: {error}"))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| format!("browser join ticket {label} must be 32 bytes"))?;
    if URL_SAFE_NO_PAD.encode(bytes) != encoded {
        return Err(format!(
            "browser join ticket {label} is not canonical base64url"
        ));
    }
    Ok(bytes)
}

fn validate_static_payload(payload: &BrowserJoinTicketPayload) -> Result<(), String> {
    if payload.schema != JOIN_TICKET_SCHEMA {
        return Err(format!(
            "join-code schema mismatch: host uses {}, this game supports {JOIN_TICKET_SCHEMA}",
            payload.schema
        ));
    }
    if payload.transport != IROH_RELAY_TRANSPORT {
        return Err(format!(
            "unsupported browser transport `{}`; expected `{IROH_RELAY_TRANSPORT}`",
            payload.transport
        ));
    }
    if payload.net_protocol != NET_PROTOCOL_VERSION {
        return Err(format!(
            "multiplayer protocol mismatch: host uses {}, this game uses {NET_PROTOCOL_VERSION}",
            payload.net_protocol
        ));
    }
    if payload.engine_version != crate::replay_format::ENGINE_SOURCE_COMMIT {
        return Err(format!(
            "multiplayer build mismatch: host uses `{}`, this game uses `{}`",
            payload.engine_version,
            crate::replay_format::ENGINE_SOURCE_COMMIT
        ));
    }
    let endpoint = payload
        .host_endpoint_id
        .parse::<EndpointId>()
        .map_err(|error| format!("invalid host endpoint id: {error}"))?;
    if endpoint.to_string() != payload.host_endpoint_id {
        return Err("host endpoint id is not canonical".to_string());
    }
    if decode_32("host public key", &payload.host_public_key)? != *endpoint.as_bytes() {
        return Err("host public key does not match the iroh endpoint id".to_string());
    }
    let relay = payload
        .relay_url
        .parse::<RelayUrl>()
        .map_err(|error| format!("invalid iroh relay URL: {error}"))?;
    if relay.scheme() != "https"
        || relay.host_str().is_none()
        || !relay.username().is_empty()
        || relay.password().is_some()
        || relay.query().is_some()
        || relay.fragment().is_some()
        || relay.to_string() != payload.relay_url
    {
        return Err(
            "iroh relay URL must be canonical HTTPS without credentials, query, or fragment"
                .to_string(),
        );
    }
    if decode_32("session id", &payload.session_id)? == [0; 32] {
        return Err("browser join ticket session id must be non-zero".to_string());
    }
    if payload
        .expires_at_epoch_s
        .checked_sub(payload.issued_at_epoch_s)
        != Some(INVITATION_LIFETIME_SECS)
    {
        return Err(format!(
            "browser invitation lifetime must be exactly {INVITATION_LIFETIME_SECS} seconds"
        ));
    }
    robin_engine::multiplayer::validate_mission_id(&payload.mission_id)
        .map_err(|error| format!("invalid browser mission id: {error}"))?;
    if !(1..=MAX_MULTIPLAYER_PLAYERS).contains(&payload.expected_players) {
        return Err(format!(
            "browser join ticket player count must be between 1 and {}",
            MAX_MULTIPLAYER_PLAYERS
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 2_000_000_000;

    fn ticket() -> BrowserJoinTicket {
        let key = SecretKey::from_bytes(&[7; 32]);
        let addr = EndpointAddr::from(key.public())
            .with_relay_url("https://relay.example.invalid/".parse().unwrap());
        BrowserJoinTicket::issue(
            &key,
            &addr,
            [9; 32],
            NOW,
            BrowserContentEdition::Demo,
            "Dem_Lei_MP".to_string(),
            Some(4),
            2,
        )
        .unwrap()
    }

    #[test]
    fn signed_ticket_roundtrips_as_one_canonical_artifact() {
        let ticket = ticket();
        let code = ticket.encode();
        assert!(code.starts_with(JOIN_CODE_PREFIX));
        assert_eq!(
            BrowserJoinTicket::decode_for_initial_use(&code, NOW).unwrap(),
            ticket
        );
        assert_eq!(
            BrowserJoinTicket::decode_authenticated(&code)
                .unwrap()
                .encode(),
            code
        );
    }

    #[test]
    fn tampering_any_signed_payload_byte_fails_closed() {
        let code = ticket().encode();
        let payload_at = JOIN_CODE_PREFIX.len() + 8;
        let mut bytes = code.into_bytes();
        bytes[payload_at] = if bytes[payload_at] == b'A' {
            b'B'
        } else {
            b'A'
        };
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(BrowserJoinTicket::decode_authenticated(&tampered).is_err());
    }

    #[test]
    fn invitation_time_boundaries_and_clock_skew_fail_closed() {
        let ticket = ticket();
        assert!(ticket.validate_use_at(NOW, InvitationUse::Initial).is_ok());
        assert!(
            ticket
                .validate_use_at(NOW + INVITATION_LIFETIME_SECS - 1, InvitationUse::Initial)
                .is_ok()
        );
        assert!(
            ticket
                .validate_use_at(NOW + INVITATION_LIFETIME_SECS, InvitationUse::Initial)
                .unwrap_err()
                .contains("expired")
        );
        assert!(
            ticket
                .validate_use_at(
                    NOW + INVITATION_LIFETIME_SECS,
                    InvitationUse::RedeemedReconnect
                )
                .is_ok()
        );
        assert!(
            ticket
                .validate_use_at(NOW - MAX_CLOCK_SKEW_SECS, InvitationUse::Initial)
                .is_ok()
        );
        assert!(
            ticket
                .validate_use_at(NOW - MAX_CLOCK_SKEW_SECS - 1, InvitationUse::Initial)
                .unwrap_err()
                .contains("future")
        );
    }

    #[test]
    fn relay_is_https_canonical_and_covered_by_signature() {
        let ticket = ticket();
        assert_eq!(ticket.payload().relay_url, "https://relay.example.invalid/");
        let endpoint = ticket.endpoint_addr().unwrap();
        assert_eq!(endpoint.relay_urls().next().unwrap().scheme(), "https");

        let key = SecretKey::from_bytes(&[7; 32]);
        let bad = EndpointAddr::from(key.public())
            .with_relay_url("http://relay.example.invalid/".parse().unwrap());
        let error = BrowserJoinTicket::issue(
            &key,
            &bad,
            [9; 32],
            NOW,
            BrowserContentEdition::Full,
            "M01".into(),
            None,
            2,
        )
        .unwrap_err();
        assert!(error.contains("canonical HTTPS"));
    }

    #[test]
    fn direct_addresses_never_enter_shareable_ticket() {
        let key = SecretKey::from_bytes(&[7; 32]);
        let addr = EndpointAddr::from(key.public())
            .with_relay_url("https://relay.example.invalid/".parse().unwrap())
            .with_ip_addr("192.0.2.7:4433".parse().unwrap());
        let ticket = BrowserJoinTicket::issue(
            &key,
            &addr,
            [9; 32],
            NOW,
            BrowserContentEdition::Full,
            "M01".into(),
            None,
            2,
        )
        .unwrap();
        assert!(
            ticket
                .endpoint_addr()
                .unwrap()
                .addrs
                .iter()
                .all(TransportAddr::is_relay)
        );
    }
}
