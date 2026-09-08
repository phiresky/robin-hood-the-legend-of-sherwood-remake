use std::collections::BTreeSet;

/// Structural validation shared by protocol documents.
pub trait Validate {
    fn validate(&self) -> Result<(), ValidationError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error("{document} has schema version {actual}; expected {expected}")]
    SchemaVersion {
        document: &'static str,
        expected: u32,
        actual: u32,
    },
    #[error("{field} must not be empty")]
    Empty { field: &'static str },
    #[error("{field} exceeds its {maximum}-byte limit")]
    TooLong { field: &'static str, maximum: usize },
    #[error("{field} contains a control character")]
    ControlCharacter { field: &'static str },
    #[error("{field} must not have surrounding whitespace")]
    SurroundingWhitespace { field: &'static str },
    #[error("{field} must be strictly sorted and contain no duplicates")]
    NotCanonicalOrder { field: &'static str },
    #[error("{field} contains duplicate value `{value}`")]
    Duplicate { field: &'static str, value: String },
    #[error("{field} is not a canonical relative path: {reason}")]
    InvalidRelativePath {
        field: &'static str,
        reason: &'static str,
    },
    #[error("{field} must be greater than zero")]
    Zero { field: &'static str },
    #[error("canonical config nesting exceeds {maximum} levels")]
    ConfigTooDeep { maximum: usize },
    #[error("{field} must be a JSON object")]
    NotObject { field: &'static str },
    #[error("{field} must match the value carried by the signed submission")]
    ClaimMismatch { field: &'static str },
    #[error("campaign-chain verification requires a predecessor result")]
    MissingPredecessor,
    #[error("a non-chain initial-state expectation must not name a predecessor result")]
    UnexpectedPredecessor,
    #[error("expected_player_count must be greater than zero")]
    EmptyPlayerCount,
    #[error("participant claims must include the host in seat 0")]
    MissingHostClaim,
    #[error("participant claims exceed expected_player_count")]
    TooManyParticipantClaims,
    #[error("participant claims must be ordered by unique seat and public key")]
    InvalidParticipantClaims,
    #[error("participant signatures must exactly match the claimed public keys")]
    InvalidParticipantSignatures,
    #[error("{field} must contain at least one metric in canonical order")]
    InvalidMetrics { field: &'static str },
    #[error("requested metrics are not a subset of the signed offer")]
    MetricsNotOffered,
    #[error("tainted input provenance must name at least one unique taint in canonical order")]
    InvalidInputTaints,
    #[error("a verified public run must have rankable input provenance")]
    VerifiedRunNotRankable,
    #[error("input-provenance rejection must preserve an ineligible provenance status")]
    InvalidInputProvenanceRejection,
    #[error("{field} exceeds its related count")]
    CountOutOfRange { field: &'static str },
    #[error("{field} must agree with the selected metric")]
    MetricValueMismatch { field: &'static str },
    #[error("verified original score must fit the canonical unsigned 32-bit mission score")]
    InvalidOriginalScore,
}

pub(crate) fn schema(document: &'static str, actual: u32) -> Result<(), ValidationError> {
    schema_exact(document, crate::SCHEMA_VERSION_V1, actual)
}

pub(crate) fn schema_exact(
    document: &'static str,
    expected: u32,
    actual: u32,
) -> Result<(), ValidationError> {
    if actual == expected {
        Ok(())
    } else {
        Err(ValidationError::SchemaVersion {
            document,
            expected,
            actual,
        })
    }
}

pub(crate) fn text(
    field: &'static str,
    value: &str,
    maximum: usize,
) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::Empty { field });
    }
    if value.len() > maximum {
        return Err(ValidationError::TooLong { field, maximum });
    }
    if value.trim() != value {
        return Err(ValidationError::SurroundingWhitespace { field });
    }
    if value.chars().any(char::is_control) {
        return Err(ValidationError::ControlCharacter { field });
    }
    if value.chars().any(|character| {
        matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
    }) {
        return Err(ValidationError::ControlCharacter { field });
    }
    Ok(())
}

/// URL-path fragment below an immutable allowlisted viewer origin.
///
/// The deliberately small unescaped ASCII grammar makes both filesystem joins
/// and WHATWG URL resolution stay below the configured base path. Callers must
/// not percent-decode or otherwise reinterpret this value before validation.
pub(crate) fn artifact_relative_url_path(
    field: &'static str,
    value: &str,
) -> Result<(), ValidationError> {
    canonical_relative_path(field, value)?;
    if value.starts_with("//")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
    {
        return Err(ValidationError::InvalidRelativePath {
            field,
            reason: "viewer paths permit only unescaped ASCII letters, digits, `/`, `.`, `_`, and `-`",
        });
    }
    Ok(())
}

pub(crate) fn canonical_relative_path(
    field: &'static str,
    value: &str,
) -> Result<(), ValidationError> {
    text(field, value, 1024)?;
    if value.starts_with('/') {
        return Err(ValidationError::InvalidRelativePath {
            field,
            reason: "absolute paths are forbidden",
        });
    }
    if value.contains('\\') {
        return Err(ValidationError::InvalidRelativePath {
            field,
            reason: "use forward slashes",
        });
    }
    if value
        .split('/')
        .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(ValidationError::InvalidRelativePath {
            field,
            reason: "empty, `.` and `..` components are forbidden",
        });
    }
    Ok(())
}

pub(crate) fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

pub(crate) fn unique_text<'a>(
    field: &'static str,
    values: impl IntoIterator<Item = &'a str>,
) -> Result<(), ValidationError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(ValidationError::Duplicate {
                field,
                value: value.to_owned(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_text_rejects_bidi_controls() {
        for control in ['\u{061c}', '\u{200e}', '\u{202e}', '\u{2066}', '\u{2069}'] {
            assert!(text("test", &format!("Robin{control}Hood"), 100).is_err());
        }
    }

    #[test]
    fn viewer_path_grammar_cannot_escape_url_base() {
        assert!(artifact_relative_url_path("test", "viewer/robin-1.wasm").is_ok());
        for path in [
            "../x",
            "/x",
            "//evil/x",
            "javascript:x",
            "data:text/javascript,x",
            "%2e%2e/x",
            "x?y",
            "x#y",
            "x\\y",
        ] {
            assert!(artifact_relative_url_path("test", path).is_err(), "{path}");
        }
    }
}
