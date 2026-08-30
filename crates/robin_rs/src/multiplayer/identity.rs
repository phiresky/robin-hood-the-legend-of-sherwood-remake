//! Persistent iroh identity for multiplayer.
//!
//! Every install keeps one stable [`SecretKey`] on disk.  Its public
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

use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey, endpoint::presets};
use std::path::PathBuf;

/// ALPN for game-session connections (`--server` / `--connect`).
pub const GAME_ALPN: &[u8] = b"robinhood/game/0";

/// Where the per-install game identity key lives.
fn identity_key_path(file_name: &str) -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("ROBINHOOD_SAVE_DIR") {
        return Ok(PathBuf::from(dir).join(file_name));
    }
    #[cfg(feature = "native-fs")]
    if let Some(data_dir) = dirs::data_dir() {
        return Ok(data_dir.join("robin_hood").join(file_name));
    }
    Err(format!(
        "no data directory available to store the multiplayer identity key `{file_name}`"
    ))
}

fn load_or_create_key(file_name: &str) -> Result<SecretKey, String> {
    let path = identity_key_path(file_name)?;
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            let hex = contents.trim();
            let mut bytes = [0u8; 32];
            let decoded = (0..32)
                .map(|i| u8::from_str_radix(hex.get(i * 2..i * 2 + 2).unwrap_or(""), 16))
                .collect::<Result<Vec<u8>, _>>()
                .map_err(|e| format!("corrupt multiplayer identity key {}: {e}", path.display()))?;
            bytes.copy_from_slice(&decoded);
            Ok(SecretKey::from_bytes(&bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = SecretKey::generate();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create {}: {e}", parent.display()))?;
            }
            let hex: String = key.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
            std::fs::write(&path, hex).map_err(|e| format!("write {}: {e}", path.display()))?;
            tracing::info!(
                path = %path.display(),
                endpoint_id = %key.public(),
                "generated new multiplayer identity key"
            );
            Ok(key)
        }
        Err(e) => Err(format!(
            "read multiplayer identity key {}: {e}",
            path.display()
        )),
    }
}

/// The per-install game identity key (created on first use).
pub fn game_secret_key() -> Result<SecretKey, String> {
    load_or_create_key("multiplayer_identity.key")
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
    bind_endpoint_with_relay(key, alpn, None).await
}

/// Bind while optionally retaining the exact relay route of a cross-mission
/// session, so already-redeemed browser peers can reach the replacement
/// transport through the route authenticated by their invitation.
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
    Endpoint::builder(presets::N0)
        .secret_key(SecretKey::generate())
        .address_lookup(iroh_mainline_address_lookup::DhtAddressLookup::builder())
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
}
