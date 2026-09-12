//! Private, bounded crash and bug reports. Never used as ranked evidence.
use serde::{Deserialize, Serialize};
pub const MAX_DIAGNOSTIC_BODY_BYTES: usize = 20 * 1024 * 1024;
// Separate expansion guard: this is not the compressed upload budget.
pub const MAX_DIAGNOSTIC_DECODED_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_DIAGNOSTIC_LOG_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_DIAGNOSTIC_ATTACHMENT_BYTES: usize = 224 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticAttachmentV1 {
    pub filename: String,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticReportV1 {
    pub schema_version: u32,
    pub kind: DiagnosticKindV1,
    pub description: String,
    pub engine_commit: String,
    pub platform: String,
    pub occurred_at_unix_ms: u64,
    pub backtrace: Option<String>,
    pub recent_log: String,
    pub attachments: Vec<DiagnosticAttachmentV1>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticKindV1 {
    Bug,
    Panic,
    FatalError,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticReceiptV1 {
    pub schema_version: u32,
    pub report_id: String,
}
impl DiagnosticReportV1 {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1 {
            return Err("unsupported diagnostics schema");
        }
        if self.description.trim().is_empty() || self.description.len() > 16384 {
            return Err("invalid description length");
        }
        if self.engine_commit.is_empty()
            || self.engine_commit.len() > 128
            || self.platform.is_empty()
            || self.platform.len() > 128
        {
            return Err("invalid build/platform");
        }
        if self.recent_log.len() > MAX_DIAGNOSTIC_LOG_BYTES
            || self
                .backtrace
                .as_ref()
                .is_some_and(|s| s.len() > MAX_DIAGNOSTIC_LOG_BYTES)
        {
            return Err("diagnostic text too large");
        }
        if self.attachments.len() > 64
            || self.warnings.len() > 64
            || self.warnings.iter().any(|s| s.len() > 1024)
        {
            return Err("too many attachments or warnings");
        }
        let mut names = std::collections::BTreeSet::new();
        let mut total = 0;
        for attachment in &self.attachments {
            if attachment.filename.is_empty()
                || attachment.filename.len() > 128
                || !attachment
                    .filename
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                || !names.insert(&attachment.filename)
            {
                return Err("invalid or duplicate attachment name");
            }
            total += attachment.content.len();
            if total > MAX_DIAGNOSTIC_ATTACHMENT_BYTES {
                return Err("attachments too large");
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unbounded_text_and_attachment_paths() {
        let mut report = DiagnosticReportV1 {
            schema_version: 1,
            kind: DiagnosticKindV1::Bug,
            description: "stuck".into(),
            engine_commit: "test".into(),
            platform: "test".into(),
            occurred_at_unix_ms: 0,
            backtrace: None,
            recent_log: String::new(),
            attachments: vec![],
            warnings: vec![],
        };
        assert!(report.validate().is_ok());
        report.attachments.push(DiagnosticAttachmentV1 {
            filename: "../secret".into(),
            content: String::new(),
        });
        assert!(report.validate().is_err());
        report.attachments.clear();
        report.recent_log = "x".repeat(MAX_DIAGNOSTIC_LOG_BYTES + 1);
        assert!(report.validate().is_err());
    }
}

/// Compress the entire JSON document with zstd before testing the upload budget.
pub fn compress_report(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    if bytes.len() > MAX_DIAGNOSTIC_DECODED_BYTES {
        return Err(std::io::Error::other("report exceeds decoded safety limit"));
    }
    let compressed = zstd::stream::encode_all(bytes, 3)?;
    if compressed.len() > MAX_DIAGNOSTIC_BODY_BYTES {
        return Err(std::io::Error::other(
            "report exceeds 20 MiB compressed upload limit",
        ));
    }
    Ok(compressed)
}

/// Bound expansion independently of the compressed transport budget.
pub fn decompress_report(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    if bytes.len() > MAX_DIAGNOSTIC_BODY_BYTES {
        return Err(std::io::Error::other(
            "compressed report exceeds upload limit",
        ));
    }
    decompress_bounded(bytes, MAX_DIAGNOSTIC_DECODED_BYTES)
}

fn decompress_bounded(bytes: &[u8], limit: usize) -> std::io::Result<Vec<u8>> {
    let mut decoder = zstd::stream::read::Decoder::new(bytes)?;
    // Bound decoder history as well as emitted bytes for untrusted uploads.
    decoder.window_log_max(28)?;
    decode_limited(decoder, limit)
}

/// Browsers use their built-in gzip CompressionStream instead of a wasm codec.
pub fn decompress_gzip_report(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    if bytes.len() > MAX_DIAGNOSTIC_BODY_BYTES {
        return Err(std::io::Error::other(
            "compressed report exceeds upload limit",
        ));
    }
    decode_limited(
        flate2::read::MultiGzDecoder::new(bytes),
        MAX_DIAGNOSTIC_DECODED_BYTES,
    )
}

fn decode_limited(reader: impl std::io::Read, limit: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let mut decoded = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut decoded)?;
    if decoded.len() > limit {
        return Err(std::io::Error::other("report exceeds decoded safety limit"));
    }
    Ok(decoded)
}

#[cfg(test)]
mod compression_tests {
    use super::*;
    #[test]
    fn accepts_large_compressible_report_and_checks_corruption_and_expansion() {
        let raw = vec![b'x'; 24 * 1024 * 1024];
        let compressed = compress_report(&raw).unwrap();
        assert!(compressed.len() < MAX_DIAGNOSTIC_BODY_BYTES);
        assert_eq!(decompress_report(&compressed).unwrap(), raw);
        assert!(decompress_bounded(&compressed, 1024).is_err());
        assert!(decompress_report(&compressed[..compressed.len() - 4]).is_err());
        assert!(decompress_report(b"not zstd").is_err());
        assert!(decompress_report(&vec![0; MAX_DIAGNOSTIC_BODY_BYTES + 1]).is_err());
    }

    #[test]
    fn browser_gzip_is_bounded_and_requires_a_complete_stream() {
        use std::io::Write as _;
        let raw = vec![b'x'; 24 * 1024 * 1024];
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).unwrap();
        let compressed = encoder.finish().unwrap();
        assert_eq!(decompress_gzip_report(&compressed).unwrap(), raw);
        assert!(
            decode_limited(
                flate2::read::MultiGzDecoder::new(compressed.as_slice()),
                1024
            )
            .is_err()
        );
        assert!(decompress_gzip_report(&compressed[..compressed.len() - 4]).is_err());
    }
}
