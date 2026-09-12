//! Private, bounded crash and bug reports. Never used as ranked evidence.
use serde::{Deserialize, Serialize};
pub const MAX_DIAGNOSTIC_BODY_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_DIAGNOSTIC_ATTACHMENT_BYTES: usize = 1024 * 1024;

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
        if self.recent_log.len() > 256 * 1024
            || self
                .backtrace
                .as_ref()
                .is_some_and(|s| s.len() > 128 * 1024)
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
        report.recent_log = "x".repeat(256 * 1024 + 1);
        assert!(report.validate().is_err());
    }
}
