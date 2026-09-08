use unicode_normalization::UnicodeNormalization as _;

pub const MAX_USERNAME_CHARS: usize = 32;

pub fn validate_username(value: &str) -> Result<String, &'static str> {
    if value != value.trim() {
        return Err("username must not have leading or trailing whitespace");
    }
    let normalized = value.nfc().collect::<String>();
    let chars = normalized.chars().count();
    if !(1..=MAX_USERNAME_CHARS).contains(&chars) {
        return Err("username must contain between 1 and 32 characters");
    }
    if normalized.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
            )
    })
    {
        return Err("username contains a control or bidirectional override character");
    }
    let search_key = normalized_username(&normalized);
    if search_key.is_empty() || search_key.chars().count() > 48 {
        return Err("username expands beyond the normalized storage limit");
    }
    Ok(normalized)
}

pub fn normalized_username(value: &str) -> String {
    value.nfkc().collect::<String>().to_lowercase()
}

pub fn verify_signature(
    public_key: &[u8; 32],
    signature: &[u8; 64],
    message: &[u8],
) -> Result<(), &'static str> {
    robin_run_protocol::verify_ed25519_strict(public_key, signature, message).map_err(|error| {
        match error {
            robin_run_protocol::SignatureVerificationError::InvalidPublicKey => {
                "invalid Ed25519 public key"
            }
            robin_run_protocol::SignatureVerificationError::InvalidSignature => {
                "invalid Ed25519 signature"
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};

    #[test]
    fn signatures_are_checked_against_exact_bytes() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let signature = key.sign(b"exact").to_bytes();
        assert!(verify_signature(&key.verifying_key().to_bytes(), &signature, b"exact").is_ok());
        assert!(verify_signature(&key.verifying_key().to_bytes(), &signature, b"changed").is_err());
    }

    #[test]
    fn usernames_are_normalized_but_not_silently_trimmed() {
        assert_eq!(validate_username("Cafe\u{301}").unwrap(), "Caf\u{e9}");
        assert!(validate_username(" Robin").is_err());
        assert!(validate_username("x\u{202e}y").is_err());
        for bidi in [
            '\u{061c}', '\u{200e}', '\u{200f}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ] {
            assert!(validate_username(&format!("x{bidi}y")).is_err());
        }
        assert!(validate_username(&"\u{fdfa}".repeat(3)).is_err());
    }
}
