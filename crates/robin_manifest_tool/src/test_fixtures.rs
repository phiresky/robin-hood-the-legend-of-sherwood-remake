//! Byte-exact artifact fixtures shared by publication and VPS tests.
pub(crate) fn fact(bytes: &[u8]) -> robin_run_protocol::ArtifactRefV1 {
    robin_run_protocol::ArtifactRefV1 {
        sha256: robin_run_protocol::Digest32::digest_bytes(bytes),
        byte_length: bytes.len() as u64,
        media_type: "application/octet-stream".into(),
    }
}
