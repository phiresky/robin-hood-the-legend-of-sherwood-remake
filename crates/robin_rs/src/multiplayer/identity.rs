//! Persistent transport identity for multiplayer.
//!
//! Every install keeps one stable Ed25519 seed on disk. Iroh derives its
//! [`SecretKey`] from that shared durable game identity. Its public
//! half — the [`EndpointId`] — is the address other players use to
//! connect, both directly (`--connect <endpoint-id>`) and through
//! matchmaking (the hosted game advertises this id as its `connect_addr`).
//!
//! Keeping the key persistent means the endpoint id is known *before*
//! the game endpoint is actually bound: matchmaking can advertise the
//! host's id at create-game time, and the real endpoint only comes up
//! when the mission launches.  Joining peers resolve the id through
//! iroh's relay + DNS address lookup, so no bind address, port, or NAT
//! configuration is ever exchanged.

use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey, endpoint::presets};
#[cfg(not(target_arch = "wasm32"))]
use iroh::{RelayMode, RelayUrl};

/// ALPN for game-session connections (`--server` / `--connect`).
pub const GAME_ALPN: &[u8] = b"robinhood/game/0";

/// The per-install game identity key (created on first use).
#[cfg(not(target_arch = "wasm32"))]
pub fn game_secret_key() -> Result<SecretKey, String> {
    Ok(secret_key_from_seed(
        crate::native_game_identity::durable_game_identity_seed()?,
    ))
}

#[cfg(not(target_arch = "wasm32"))]
fn secret_key_from_seed(seed: [u8; 32]) -> SecretKey {
    SecretKey::from_bytes(&seed)
}

/// Browser gameplay uses an ephemeral iroh transport endpoint and the isolated
/// stable-shell signer as its one durable identity. Refuse to manufacture a
/// second durable seed inside game WASM.
#[cfg(target_arch = "wasm32")]
pub fn game_secret_key() -> Result<SecretKey, String> {
    Err("browser hosting has no durable iroh key; use the isolated durable identity signer".into())
}

/// The endpoint id other players dial to reach games hosted from this
/// install.  Stable across restarts.
pub fn local_endpoint_id_string() -> Result<String, String> {
    Ok(game_secret_key()?.public().to_string())
}

/// Bind an iroh endpoint with the given identity and single ALPN.
///
/// Address lookup is layered: the n0 DNS/pkarr system from the N0
/// preset (fast when its servers are reachable) plus publish/resolve
/// on the BitTorrent Mainline DHT, which works with no hosted
/// infrastructure at all.
pub async fn bind_endpoint(key: SecretKey, alpn: &[u8]) -> Result<Endpoint, String> {
    #[cfg(target_arch = "wasm32")]
    {
        return Endpoint::builder(presets::N0)
            .secret_key(key)
            .alpns(vec![alpn.to_vec()])
            .bind()
            .await
            .map_err(|e| format!("bind iroh endpoint: {e}"));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        bind_endpoint_with_relay(key, alpn, None).await
    }
}

/// Bind while optionally retaining the exact relay route of a cross-mission
/// session, so already-redeemed browser peers can reach the replacement
/// transport through the route authenticated by their invitation.
#[cfg(not(target_arch = "wasm32"))]
pub async fn bind_endpoint_with_relay(
    key: SecretKey,
    alpn: &[u8],
    relay_url: Option<RelayUrl>,
) -> Result<Endpoint, String> {
    let mut builder = Endpoint::builder(presets::N0)
        .secret_key(key)
        .alpns(vec![alpn.to_vec()])
        .address_lookup(iroh_mainline_address_lookup::DhtAddressLookup::builder());
    if let Some(relay_url) = relay_url {
        builder = builder.relay_mode(RelayMode::custom([relay_url]));
    }
    builder
        .bind()
        .await
        .map_err(|e| format!("bind iroh endpoint: {e}"))
}

/// Bind an endpoint on a fresh throwaway identity (matchmaking swarm
/// membership, joining clients) with the same lookup layering.
pub async fn bind_ephemeral_endpoint() -> Result<Endpoint, String> {
    let builder = Endpoint::builder(presets::N0).secret_key(SecretKey::generate());
    #[cfg(not(target_arch = "wasm32"))]
    let builder = builder.address_lookup(iroh_mainline_address_lookup::DhtAddressLookup::builder());
    builder
        .bind()
        .await
        .map_err(|e| format!("bind iroh endpoint: {e}"))
}

/// Parse a connect string into an [`EndpointAddr`].
///
/// Accepts either a bare endpoint id (the normal case — addresses are
/// resolved through relay/DNS lookup) or a JSON-serialized
/// [`EndpointAddr`] carrying explicit transport addresses (used by
/// tests and relay-less setups).
pub fn parse_connect_addr(raw: &str) -> Result<EndpointAddr, String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        return serde_json::from_str::<EndpointAddr>(trimmed)
            .map_err(|e| format!("parse endpoint address `{trimmed}`: {e}"));
    }
    trimmed
        .parse::<EndpointId>()
        .map(EndpointAddr::from)
        .map_err(|e| format!("parse endpoint id `{trimmed}`: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_addr_roundtrips_json_and_id() {
        let key = SecretKey::generate();
        let id = key.public();
        let parsed = parse_connect_addr(&id.to_string()).expect("bare id parses");
        assert_eq!(parsed.id, id);

        let addr = EndpointAddr::from(id);
        let json = serde_json::to_string(&addr).expect("addr serializes");
        let parsed = parse_connect_addr(&json).expect("json addr parses");
        assert_eq!(parsed.id, id);
    }

    #[test]
    fn iroh_key_preserves_the_ed25519_game_identity() {
        let seed = [0x57; 32];
        let key = secret_key_from_seed(seed);
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);

        assert_eq!(key.to_bytes(), signing_key.to_bytes());
        assert_eq!(
            key.public().as_bytes(),
            signing_key.verifying_key().as_bytes()
        );
    }
}
