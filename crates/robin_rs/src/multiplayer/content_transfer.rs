//! Shared sequential content-transfer validation, independent of stream and clock.

use super::{NetEvent, NetMsg};
use robin_engine::multiplayer::{DISTRIBUTED_MOD_CHUNK_LIMIT, DistributedModOffer};

/// Validate before publishing any bytes to the cache consumer. Both transports
/// retain their own cancellation/deadline mechanism, but admit identical chunks.
pub(super) fn accept_chunk(
    offer: &DistributedModOffer,
    received: u64,
    message: Option<NetMsg>,
) -> Result<(u64, NetEvent), String> {
    let Some(NetMsg::ContentChunk {
        full_mod_sha256,
        offset,
        total_bytes,
        bytes,
    }) = message
    else {
        return Err(format!(
            "expected sequential ContentChunk at offset {received}, got {message:?}"
        ));
    };
    if full_mod_sha256 != offer.full_mod_sha256
        || total_bytes != offer.encoded_bytes
        || offset != received
        || bytes.is_empty()
        || bytes.len() > DISTRIBUTED_MOD_CHUNK_LIMIT
    {
        return Err(format!(
            "invalid distributed-mod chunk: hash={} offset={offset} total={total_bytes} bytes={}; expected hash={} offset={received} total={} and 1..={} bytes",
            robin_engine::spellforge::hex_hash(&full_mod_sha256),
            bytes.len(),
            robin_engine::spellforge::hex_hash(&offer.full_mod_sha256),
            offer.encoded_bytes,
            DISTRIBUTED_MOD_CHUNK_LIMIT
        ));
    }
    let end = received
        .checked_add(bytes.len() as u64)
        .ok_or_else(|| "distributed-mod chunk offset overflow".to_owned())?;
    if end > offer.encoded_bytes {
        return Err(format!(
            "distributed-mod chunk ends at {end}, beyond offered {} bytes",
            offer.encoded_bytes
        ));
    }
    Ok((
        end,
        NetEvent::ContentChunk {
            full_mod_sha256,
            offset,
            total_bytes,
            bytes,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer() -> DistributedModOffer {
        DistributedModOffer {
            schema_version: 1,
            full_mod_sha256: [1; 32],
            spellforge_package_sha256: None,
            spellforge_vm_abi: None,
            encoded_bytes: 4,
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

    fn chunk(hash: [u8; 32], offset: u64, total_bytes: u64, bytes: Vec<u8>) -> Option<NetMsg> {
        Some(NetMsg::ContentChunk {
            full_mod_sha256: hash,
            offset,
            total_bytes,
            bytes,
        })
    }

    #[test]
    fn resumed_transfer_publishes_exact_bytes_after_validation() {
        let (next, event) = accept_chunk(&offer(), 2, chunk([1; 32], 2, 4, vec![7, 8])).unwrap();
        assert_eq!(next, 4);
        assert!(
            matches!(event, NetEvent::ContentChunk { offset: 2, total_bytes: 4, bytes, .. } if bytes == [7, 8])
        );
    }

    #[test]
    fn invalid_chunks_are_rejected_without_a_publishable_event() {
        for message in [
            None,
            chunk([2; 32], 0, 4, vec![1]),
            chunk([1; 32], 1, 4, vec![1]),
            chunk([1; 32], 0, 5, vec![1]),
            chunk([1; 32], 0, 4, vec![]),
            chunk([1; 32], 0, 4, vec![1; 5]),
            chunk([1; 32], 0, 4, vec![1; DISTRIBUTED_MOD_CHUNK_LIMIT + 1]),
        ] {
            assert!(accept_chunk(&offer(), 0, message).is_err());
        }
        let mut overflowing = offer();
        overflowing.encoded_bytes = u64::MAX;
        assert!(
            accept_chunk(
                &overflowing,
                u64::MAX,
                chunk([1; 32], u64::MAX, u64::MAX, vec![1])
            )
            .is_err()
        );
    }
}
