//! Shared sequential content-transfer validation, independent of stream and clock.

use super::{MultiplayerError, NetEvent, NetMsg, NetOutbound};
use robin_engine::multiplayer::{
    ContentChunk, ContentReject, ContentRequest, DISTRIBUTED_MOD_CHUNK_LIMIT, DistributedModOffer,
};

fn mismatch(message: String) -> MultiplayerError {
    MultiplayerError::ContentMismatch(message.into())
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(super) enum ContentDecision {
    Request { resume_offset: u64 },
    Reject { reason: String },
}

impl ContentDecision {
    pub(super) fn decode(
        offer: &DistributedModOffer,
        decision: NetOutbound,
    ) -> Result<Self, MultiplayerError> {
        match decision {
            NetOutbound::ContentRequest(ContentRequest {
                full_mod_sha256,
                resume_offset,
            }) if full_mod_sha256 == offer.full_mod_sha256
                && resume_offset <= offer.encoded_bytes =>
            {
                Ok(Self::Request { resume_offset })
            }
            NetOutbound::ContentRequest(ContentRequest {
                full_mod_sha256,
                resume_offset,
            }) => Err(mismatch(format!(
                "invalid content request for {} at offset {resume_offset}; offered {} with {} bytes",
                robin_engine::spellforge::hex_hash(&full_mod_sha256),
                robin_engine::spellforge::hex_hash(&offer.full_mod_sha256),
                offer.encoded_bytes
            ))),
            NetOutbound::ContentReject(ContentReject {
                full_mod_sha256,
                reason,
            }) if full_mod_sha256 == offer.full_mod_sha256 => Ok(Self::Reject { reason }),
            other => Err(MultiplayerError::LocalState(
                format!("expected local ContentRequest/ContentReject, got {other:?}").into(),
            )),
        }
    }

    pub(super) fn message(&self, offer: &DistributedModOffer) -> NetMsg {
        let full_mod_sha256 = offer.full_mod_sha256;
        match self {
            Self::Request { resume_offset } => NetMsg::ContentRequest(ContentRequest {
                full_mod_sha256,
                resume_offset: *resume_offset,
            }),
            Self::Reject { reason } => NetMsg::ContentReject(ContentReject {
                full_mod_sha256,
                reason: reason.clone(),
            }),
        }
    }

    pub(super) fn operation(&self) -> &'static str {
        match self {
            Self::Request { .. } => "content request",
            Self::Reject { .. } => "content rejection",
        }
    }

    /// Called only after the decision has been sent, including a rejection.
    pub(super) fn resume_offset(self) -> Result<u64, MultiplayerError> {
        match self {
            Self::Request { resume_offset } => Ok(resume_offset),
            Self::Reject { reason } => Err(MultiplayerError::ContentDeclined(
                format!("local player declined exact host content: {reason}").into(),
            )),
        }
    }
}

/// Validate before publishing any bytes to the cache consumer. Both transports
/// retain their own cancellation/deadline mechanism, but admit identical chunks.
pub(super) fn accept_chunk(
    offer: &DistributedModOffer,
    received: u64,
    message: Option<NetMsg>,
) -> Result<(u64, NetEvent), MultiplayerError> {
    let Some(NetMsg::ContentChunk(ContentChunk {
        full_mod_sha256,
        offset,
        total_bytes,
        bytes,
    })) = message
    else {
        return Err(mismatch(format!(
            "expected sequential ContentChunk at offset {received}, got {message:?}"
        )));
    };
    if full_mod_sha256 != offer.full_mod_sha256
        || total_bytes != offer.encoded_bytes
        || offset != received
        || bytes.is_empty()
        || bytes.len() > DISTRIBUTED_MOD_CHUNK_LIMIT
    {
        return Err(mismatch(format!(
            "invalid distributed-mod chunk: hash={} offset={offset} total={total_bytes} bytes={}; expected hash={} offset={received} total={} and 1..={} bytes",
            robin_engine::spellforge::hex_hash(&full_mod_sha256),
            bytes.len(),
            robin_engine::spellforge::hex_hash(&offer.full_mod_sha256),
            offer.encoded_bytes,
            DISTRIBUTED_MOD_CHUNK_LIMIT
        )));
    }
    let end = received
        .checked_add(bytes.len() as u64)
        .ok_or_else(|| mismatch("distributed-mod chunk offset overflow".to_owned()))?;
    if end > offer.encoded_bytes {
        return Err(mismatch(format!(
            "distributed-mod chunk ends at {end}, beyond offered {} bytes",
            offer.encoded_bytes
        )));
    }
    Ok((
        end,
        NetEvent::ContentChunk(ContentChunk {
            full_mod_sha256,
            offset,
            total_bytes,
            bytes,
        }),
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
        Some(NetMsg::ContentChunk(ContentChunk {
            full_mod_sha256: hash,
            offset,
            total_bytes,
            bytes,
        }))
    }

    #[test]
    fn resumed_transfer_publishes_exact_bytes_after_validation() {
        let (next, event) = accept_chunk(&offer(), 2, chunk([1; 32], 2, 4, vec![7, 8])).unwrap();
        assert_eq!(next, 4);
        assert!(
            matches!(event, NetEvent::ContentChunk(ContentChunk { offset: 2, total_bytes: 4, bytes, .. }) if bytes == [7, 8])
        );
    }

    #[test]
    fn content_decision_checks_exact_identity_and_offset_before_publication() {
        let offer = offer();
        for (hash, offset) in [([2; 32], 0), ([1; 32], 5)] {
            assert!(
                ContentDecision::decode(
                    &offer,
                    NetOutbound::ContentRequest(ContentRequest {
                        full_mod_sha256: hash,
                        resume_offset: offset,
                    })
                )
                .is_err()
            );
        }
        let request = ContentDecision::decode(
            &offer,
            NetOutbound::ContentRequest(ContentRequest {
                full_mod_sha256: [1; 32],
                resume_offset: 4,
            }),
        )
        .unwrap();
        assert!(matches!(
            request.message(&offer),
            NetMsg::ContentRequest(ContentRequest {
                resume_offset: 4,
                ..
            })
        ));
        assert_eq!(request.resume_offset().unwrap(), 4);
        let rejected = ContentDecision::decode(
            &offer,
            NetOutbound::ContentReject(ContentReject {
                full_mod_sha256: [1; 32],
                reason: "declined".into(),
            }),
        )
        .unwrap();
        assert!(
            matches!(rejected.message(&offer), NetMsg::ContentReject(ContentReject { reason, .. }) if reason == "declined")
        );
        assert_eq!(
            rejected.resume_offset().unwrap_err().to_string(),
            "local player declined exact host content: declined"
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
