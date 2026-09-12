//! Shared HMAC-SHA256 authority. Domains and canonical payloads belong to callers.

use hmac::{Hmac, KeyInit as _, Mac as _};
use sha2::Sha256;

fn keyed(key: &[u8]) -> Hmac<Sha256> {
    Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts arbitrary key lengths")
}

pub(crate) fn sign(key: &[u8], message: &[u8]) -> [u8; 32] {
    keyed(key)
        .chain_update(message)
        .finalize()
        .into_bytes()
        .into()
}

pub(crate) fn verify(
    key: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), hmac::digest::MacError> {
    keyed(key).chain_update(message).verify_slice(signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_preserve_the_independent_legacy_implementation() {
        for key in [b"".as_slice(), b"short", &[0x55; 131]] {
            for message in [b"".as_slice(), b"exact canonical authority", &[0xa1; 256]] {
                let legacy =
                    ring::hmac::sign(&ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key), message);
                assert_eq!(sign(key, message).as_slice(), legacy.as_ref());
                verify(key, message, legacy.as_ref()).unwrap();
                assert!(verify(key, message, &legacy.as_ref()[..31]).is_err());
                let mut changed = sign(key, message);
                changed[0] ^= 1;
                assert!(verify(key, message, &changed).is_err());
            }
        }
    }
}
