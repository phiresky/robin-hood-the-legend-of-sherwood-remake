//! Serverless DHT rendezvous for the matchmaking gossip topic.
//!
//! Peers meet through the BitTorrent Mainline DHT with no broker or
//! configuration: every peer derives the same throwaway ed25519
//! keypair for the current unix minute from the (public-by-design)
//! topic string and publishes a BEP-44 mutable record listing gossip
//! endpoint ids under that shared slot.  Reading the slots for the
//! current and previous minute yields live endpoint ids to bootstrap
//! iroh-gossip with; once the swarm is joined, gossip membership takes
//! over and the DHT only matters again if the local peer ends up
//! isolated.
//!
//! The slot is world-writable on purpose — anyone who derives the key
//! from the topic string can publish.  That is the same trust model as
//! the gossip topic itself (the game list is broadcast unauthenticated),
//! and a garbage record can at worst delay discovery: ids that don't
//! parse are skipped, ids that don't answer simply fail to bootstrap.
//!
//! Records rotate minutely, so stale peers age out of the DHT on their
//! own; announcing peers re-publish every [`ANNOUNCE_INTERVAL`].

use anyhow::{Context, Result};
use mainline::{MutableItem, SigningKey, async_dht::AsyncDht};
use sha2::Digest;

/// How often an announcing peer re-publishes its record (and, when it
/// has no gossip neighbors, re-reads the slot to find peers to join).
pub const ANNOUNCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Domain-separation tags for the derived slot key and salt.
const KEY_TAG: &[u8] = b"robinhood/rendezvous/slot-key/0";
const SALT_TAG: &[u8] = b"robinhood/rendezvous/slot-salt/0";

/// BEP-44 caps values at 1000 bytes; 30 ids × 32 bytes = 960.
const MAX_RECORD_IDS: usize = 30;

/// Every DHT round trip is bounded so a dead network can't wedge the
/// announce loop.
const DHT_OP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// One rendezvous handle per gossip topic.
pub struct TopicRendezvous {
    dht: AsyncDht,
    topic_hash: [u8; 32],
    /// The local gossip endpoint id published in records.
    me: [u8; 32],
}

impl TopicRendezvous {
    pub fn new(topic: &str, me: [u8; 32]) -> Result<Self> {
        let dht = mainline::Dht::builder()
            .build()
            .context("start mainline DHT client")?
            .as_async();
        let topic_hash: [u8; 32] = sha2::Sha256::digest(topic.as_bytes()).into();
        Ok(Self {
            dht,
            topic_hash,
            me,
        })
    }

    fn unix_minute() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() / 60)
            .unwrap_or(0)
    }

    /// The shared per-minute slot keypair.  Deliberately derivable by
    /// every topic member — the slot is a public meeting point, not an
    /// identity.
    fn slot_signing_key(&self, minute: u64) -> SigningKey {
        let mut h = sha2::Sha512::new();
        h.update(KEY_TAG);
        h.update(self.topic_hash);
        h.update(minute.to_be_bytes());
        let seed: [u8; 32] = h.finalize()[..32].try_into().expect("sha512 is 64 bytes");
        SigningKey::from_bytes(&seed)
    }

    fn slot_salt(&self, minute: u64) -> [u8; 16] {
        let mut h = sha2::Sha512::new();
        h.update(SALT_TAG);
        h.update(self.topic_hash);
        h.update(minute.to_be_bytes());
        h.finalize()[..16].try_into().expect("sha512 is 64 bytes")
    }

    /// Read every id currently advertised in one minute's slot.
    ///
    /// Different DHT nodes can hold different (raced) values for the
    /// same slot; all of them are collected and merged.
    async fn read_slot(&self, minute: u64) -> Vec<[u8; 32]> {
        use futures::StreamExt;
        let key = self.slot_signing_key(minute);
        let salt = self.slot_salt(minute);
        let stream = self
            .dht
            .get_mutable(key.verifying_key().as_bytes(), Some(&salt), None);
        let items: Vec<MutableItem> =
            match tokio::time::timeout(DHT_OP_TIMEOUT, stream.collect::<Vec<_>>()).await {
                Ok(items) => items,
                Err(_) => {
                    tracing::debug!(minute, "rendezvous slot read timed out");
                    Vec::new()
                }
            };
        let mut ids = Vec::new();
        for item in items {
            for chunk in item.value().chunks_exact(32) {
                let id: [u8; 32] = chunk.try_into().expect("chunks_exact yields 32 bytes");
                if id != self.me && !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        ids
    }

    /// Endpoint ids to bootstrap the gossip swarm with (current and
    /// previous minute, so a peer that just rolled over is still seen).
    pub async fn bootstrap_ids(&self) -> Vec<[u8; 32]> {
        let minute = Self::unix_minute();
        let mut ids = self.read_slot(minute).await;
        for id in self.read_slot(minute.saturating_sub(1)).await {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    }

    /// Publish the local endpoint id into the current minute's slot.
    ///
    /// Read-merge-write: whatever ids are already advertised are kept
    /// (capped, ours first), so concurrent announcers converge instead
    /// of overwriting each other.  Publishing with a wall-clock seq
    /// makes the latest merged view win on each storing node.
    pub async fn announce(&self) -> Result<()> {
        let minute = Self::unix_minute();
        let mut ids = vec![self.me];
        ids.extend(self.read_slot(minute).await);
        ids.truncate(MAX_RECORD_IDS);
        let value: Vec<u8> = ids.iter().flatten().copied().collect();

        let seq = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0) as i64;
        let key = self.slot_signing_key(minute);
        let salt = self.slot_salt(minute);
        let item = MutableItem::new(key, &value, seq, Some(&salt));
        tokio::time::timeout(DHT_OP_TIMEOUT, self.dht.put_mutable(item, None))
            .await
            .context("rendezvous announce timed out")?
            .context("rendezvous announce failed")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_derivation_is_shared_and_rotates() {
        let a = TopicRendezvous::new("topic", [1; 32]).expect("dht client");
        let b = TopicRendezvous::new("topic", [2; 32]).expect("dht client");
        // Same topic + minute → same slot, regardless of who asks.
        assert_eq!(
            a.slot_signing_key(7).verifying_key(),
            b.slot_signing_key(7).verifying_key()
        );
        assert_eq!(a.slot_salt(7), b.slot_salt(7));
        // Different minute or topic → different slot.
        assert_ne!(
            a.slot_signing_key(7).verifying_key(),
            a.slot_signing_key(8).verifying_key()
        );
        let c = TopicRendezvous::new("other-topic", [1; 32]).expect("dht client");
        assert_ne!(
            a.slot_signing_key(7).verifying_key(),
            c.slot_signing_key(7).verifying_key()
        );
    }
}
