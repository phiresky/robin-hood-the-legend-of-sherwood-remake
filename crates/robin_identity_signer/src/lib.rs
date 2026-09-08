//! Isolated browser identity signer. No game, engine, renderer or asset dependencies.

#[cfg(target_arch = "wasm32")]
pub mod browser;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LeaderboardSigningError {
    #[error("durable game identity is unavailable: {0}")]
    Identity(String),
    #[error("leaderboard signing claim is invalid: {0}")]
    InvalidClaim(String),
    #[error("leaderboard signing claim names a different public key")]
    WrongIdentity,
    #[error("leaderboard submission does not claim this public key")]
    IdentityNotClaimed,
    #[error("canonical leaderboard signing failed: {0}")]
    Canonical(String),
    #[error("leaderboard bridge document exceeds {maximum} bytes")]
    DocumentTooLarge { maximum: usize },
    #[error("leaderboard bridge document is not valid JSON: {0}")]
    InvalidJson(String),
    #[error("leaderboard bridge caller origin does not match this deployment")]
    OriginNotAuthorized,
    #[error("the leaderboard identity signer must run in its isolated embedded document")]
    SignerContext,
}

#[cfg(any(test, target_arch = "wasm32"))]
fn decode_json<T: serde::de::DeserializeOwned>(
    json: &str,
    maximum: usize,
) -> Result<T, LeaderboardSigningError> {
    if json.len() > maximum {
        return Err(LeaderboardSigningError::DocumentTooLarge { maximum });
    }
    serde_json::from_str(json)
        .map_err(|error| LeaderboardSigningError::InvalidJson(error.to_string()))
}

#[cfg(any(test, target_arch = "wasm32"))]
fn authorize_context(
    parent: &str,
    expected_parent: &str,
    actual_origin: &str,
    expected_signer: &str,
    embedded: bool,
) -> Result<(), LeaderboardSigningError> {
    if parent != expected_parent {
        return Err(LeaderboardSigningError::OriginNotAuthorized);
    }
    if actual_origin != expected_signer || actual_origin == parent || !embedded {
        return Err(LeaderboardSigningError::SignerContext);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Document {
        text: String,
    }

    #[test]
    fn bounded_decoder_counts_utf8_bytes_and_accepts_exact_boundary() {
        let json = r#"{"text":"é"}"#;
        assert_eq!(decode_json::<Document>(json, json.len()).unwrap().text, "é");
        assert_eq!(
            decode_json::<Document>(json, json.len() - 1),
            Err(LeaderboardSigningError::DocumentTooLarge {
                maximum: json.len() - 1
            }),
        );
    }

    #[test]
    fn bounded_decoder_rejects_malformed_unknown_and_missing_fields() {
        for json in ["{", "{}", r#"{"text":"ok","extra":true}"#] {
            assert!(matches!(
                decode_json::<Document>(json, 1024),
                Err(LeaderboardSigningError::InvalidJson(_))
            ));
        }
    }

    #[test]
    fn only_the_exact_separate_embedded_origin_is_authorized() {
        const GAME: &str = "https://game.example";
        const SIGNER: &str = "https://identity.example";
        assert_eq!(authorize_context(GAME, GAME, SIGNER, SIGNER, true), Ok(()));
        for parent in [
            "https://game.example.evil",
            "http://game.example",
            "https://game.example/",
        ] {
            assert_eq!(
                authorize_context(parent, GAME, SIGNER, SIGNER, true),
                Err(LeaderboardSigningError::OriginNotAuthorized)
            );
        }
        for (actual, expected, embedded) in [
            (GAME, SIGNER, true),
            (GAME, GAME, true),
            (SIGNER, SIGNER, false),
        ] {
            assert_eq!(
                authorize_context(GAME, GAME, actual, expected, embedded),
                Err(LeaderboardSigningError::SignerContext)
            );
        }
    }
}
