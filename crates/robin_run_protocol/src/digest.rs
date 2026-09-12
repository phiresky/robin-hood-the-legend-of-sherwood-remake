use std::fmt;
use std::io::Read;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HexError {
    #[error("hex value has length {actual}; expected {expected} lowercase characters")]
    Length { expected: usize, actual: usize },
    #[error("hex value contains non-lowercase-hex byte at character {index}")]
    Character { index: usize },
}

fn encode_lower_hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

fn decode_lower_hex<const N: usize>(value: &str) -> Result<[u8; N], HexError> {
    if value.len() != N * 2 {
        return Err(HexError::Length {
            expected: N * 2,
            actual: value.len(),
        });
    }
    if let Some(index) = value
        .bytes()
        .position(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(HexError::Character { index });
    }
    let mut decoded = [0u8; N];
    hex::decode_to_slice(value, &mut decoded).expect("length and lowercase alphabet checked above");
    Ok(decoded)
}

macro_rules! fixed_hex_type {
    ($name:ident, $length:expr, $description:literal) => {
        #[doc = $description]
        #[derive(
            Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, bitcode::Encode, bitcode::Decode,
        )]
        pub struct $name([u8; $length]);

        impl Default for $name {
            fn default() -> Self {
                Self([0; $length])
            }
        }

        impl crate::validation::IsZero for $name {
            fn is_zero(&self) -> bool {
                self.is_zero()
            }
        }

        impl $name {
            pub const LENGTH: usize = $length;

            pub const fn from_bytes(bytes: [u8; $length]) -> Self {
                Self(bytes)
            }

            pub const fn as_bytes(&self) -> &[u8; $length] {
                &self.0
            }

            pub const fn into_bytes(self) -> [u8; $length] {
                self.0
            }

            /// Whether every byte is zero.
            ///
            /// Zero remains representable so these wire primitives can also
            /// describe hashes and test vectors. Protocol documents reject a
            /// zero nonce, identity, or signature where it would be invalid.
            pub fn is_zero(&self) -> bool {
                self.0.iter().all(|byte| *byte == 0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&self.to_string())
                    .finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&encode_lower_hex(&self.0))
            }
        }

        impl FromStr for $name {
            type Err = HexError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                decode_lower_hex(value).map(Self)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                value.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

fixed_hex_type!(
    Digest32,
    32,
    "A SHA-256 digest, encoded as strict lowercase hexadecimal on the wire."
);
fixed_hex_type!(
    PublicKey32,
    32,
    "Raw Ed25519 public-key bytes, compatible with an iroh EndpointId."
);
fixed_hex_type!(
    Signature64,
    64,
    "Raw Ed25519 signature bytes, compatible with an iroh Signature."
);
fixed_hex_type!(
    ChallengeNonce32,
    32,
    "A server-issued 32-byte anti-replay nonce."
);

/// Exact unsigned 64-bit simulation seed serialized as canonical decimal
/// text. JavaScript JSON numbers cannot represent every `u64` exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SimulationSeed64(u64);

impl SimulationSeed64 {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SimulationSeed64 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for SimulationSeed64 {
    type Err = SimulationSeedError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || (value.len() > 1 && value.starts_with('0'))
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(SimulationSeedError::NonCanonical);
        }
        value
            .parse::<u64>()
            .map(Self)
            .map_err(|_| SimulationSeedError::OutOfRange)
    }
}

impl Serialize for SimulationSeed64 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SimulationSeed64 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SimulationSeedError {
    #[error("simulation seed is not canonical unsigned decimal text")]
    NonCanonical,
    #[error("simulation seed exceeds the unsigned 64-bit range")]
    OutOfRange,
}

/// A bounded opaque identifier assigned outside this protocol crate.
///
/// IDs are deliberately syntax-neutral: the API may use UUIDs, UUIDv7, or
/// another stable representation without changing the signed document.  The
/// bound and control-character rules keep them safe to log and compare.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct OpaqueId(String);

impl OpaqueId {
    pub const MAX_LENGTH: usize = 128;

    pub fn new(value: impl Into<String>) -> Result<Self, crate::ValidationError> {
        let value = value.into();
        crate::validation::text("opaque_id", &value, Self::MAX_LENGTH)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for OpaqueId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for OpaqueId {
    type Err = crate::ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for OpaqueId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl Digest32 {
    pub fn digest_bytes(bytes: impl AsRef<[u8]>) -> Self {
        Self(Sha256::digest(bytes.as_ref()).into())
    }

    pub fn digest_reader(mut reader: impl Read) -> std::io::Result<Self> {
        let mut hasher = Sha256::new();
        std::io::copy(&mut reader, &mut DigestWriter(&mut hasher))?;
        Ok(Self(hasher.finalize().into()))
    }
}

impl PublicKey32 {
    /// Canonical short display fingerprint used only to disambiguate mutable,
    /// non-unique usernames. Authorization always uses the complete key.
    pub fn short_fingerprint(&self) -> String {
        const DOMAIN: &[u8] = b"robinhood-run-key-fingerprint-v1\0";
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN);
        hasher.update(self.0);
        encode_lower_hex(&hasher.finalize()[..16])
    }
}

struct DigestWriter<'a>(&'a mut Sha256);

impl std::io::Write for DigestWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct SeedFixture {
        simulation_seed: SimulationSeed64,
    }

    #[test]
    fn sha256_known_vector_and_reader_match() {
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(Digest32::digest_bytes(b"abc").to_string(), expected);
        assert_eq!(
            Digest32::digest_reader(&b"abc"[..]).unwrap(),
            expected.parse().unwrap()
        );
    }

    #[test]
    fn human_wire_format_is_strict_lowercase_hex() {
        let digest = Digest32::from_bytes([0xab; 32]);
        let json = serde_json::to_string(&digest).unwrap();
        assert_eq!(json, format!("\"{}\"", "ab".repeat(32)));
        assert_eq!(serde_json::from_str::<Digest32>(&json).unwrap(), digest);
        assert!("AB".repeat(32).parse::<Digest32>().is_err());
        assert!("ab".repeat(31).parse::<Digest32>().is_err());
    }

    #[test]
    fn simulation_seed_uses_exact_canonical_decimal_string() {
        let maximum = SimulationSeed64::new(u64::MAX);
        assert_eq!(
            serde_json::to_string(&maximum).unwrap(),
            "\"18446744073709551615\""
        );
        assert_eq!(
            serde_json::from_str::<SimulationSeed64>("\"18446744073709551615\"").unwrap(),
            maximum
        );
        for invalid in [
            "0",
            "\"\"",
            "\"00\"",
            "\"01\"",
            "\"-1\"",
            "\"+1\"",
            "\"18446744073709551616\"",
        ] {
            assert!(
                serde_json::from_str::<SimulationSeed64>(invalid).is_err(),
                "accepted noncanonical seed {invalid}"
            );
        }
    }

    #[test]
    fn max_seed_cross_language_canonical_fixture_is_pinned() {
        let fixture: SeedFixture =
            serde_json::from_str(include_str!("../fixtures/simulation_seed64_max.json")).unwrap();
        assert_eq!(fixture.simulation_seed, SimulationSeed64::new(u64::MAX));
        let canonical = crate::canonical_json_bytes(&fixture).unwrap();
        assert_eq!(
            String::from_utf8(canonical.clone()).unwrap(),
            include_str!("../fixtures/simulation_seed64_max.json").trim_end()
        );
        assert_eq!(
            Digest32::digest_bytes(canonical).to_string(),
            include_str!("../fixtures/simulation_seed64_max.sha256").trim()
        );
    }

    #[test]
    fn iroh_compatible_key_and_signature_lengths_are_pinned() {
        assert_eq!(PublicKey32::LENGTH, 32);
        assert_eq!(Signature64::LENGTH, 64);
        assert_eq!(ChallengeNonce32::LENGTH, 32);
        assert!(PublicKey32::default().is_zero());
        assert!(!PublicKey32::from_bytes([1; 32]).is_zero());
        assert_eq!(
            PublicKey32::from_bytes([0xab; 32]).short_fingerprint(),
            "4386e85fa8fe41e53c9be90d18458bbd"
        );
        let mut colliding_prefix = [0xab; 32];
        colliding_prefix[31] = 0xac;
        let other = PublicKey32::from_bytes(colliding_prefix).short_fingerprint();
        assert_ne!(other, "4386e85fa8fe41e53c9be90d18458bbd");
        assert_eq!(other.len(), 32);
        assert!(
            other
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
    }

    #[test]
    fn opaque_ids_are_bounded_at_deserialization() {
        let id = OpaqueId::new("018f0f65-9c2a-7e36-a9ea-4c11ea5be125").unwrap();
        assert_eq!(
            serde_json::from_str::<OpaqueId>(&serde_json::to_string(&id).unwrap()).unwrap(),
            id
        );
        assert!(OpaqueId::new(" bad").is_err());
        assert!(OpaqueId::new("x".repeat(OpaqueId::MAX_LENGTH + 1)).is_err());
    }
}
