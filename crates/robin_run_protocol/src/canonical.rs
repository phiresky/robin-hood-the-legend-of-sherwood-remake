use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Digest32, Validate, ValidationError};

/// JSON-shaped deterministic configuration data.
///
/// Floating-point numbers are deliberately absent.  Gameplay configuration
/// uses exact booleans, integers, strings, arrays, and objects; admitting
/// binary floating-point values would require a separate canonical numeric
/// contract and create cross-language edge cases around `-0`, exponent form,
/// and precision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CanonicalValue {
    Null,
    Bool(bool),
    Signed(i64),
    Unsigned(u64),
    String(String),
    Array(Vec<CanonicalValue>),
    Object(BTreeMap<String, CanonicalValue>),
}

impl CanonicalValue {
    pub fn from_serializable(value: &impl Serialize) -> Result<Self, serde_json::Error> {
        serde_json::from_value(serde_json::to_value(value)?)
    }

    pub fn validate_depth(&self, maximum: usize) -> Result<(), ValidationError> {
        fn visit(
            value: &CanonicalValue,
            depth: usize,
            maximum: usize,
        ) -> Result<(), ValidationError> {
            if depth > maximum {
                return Err(ValidationError::ConfigTooDeep { maximum });
            }
            match value {
                CanonicalValue::String(value) => {
                    if value.len() > 16 * 1024 {
                        return Err(ValidationError::TooLong {
                            field: "canonical_value.string",
                            maximum: 16 * 1024,
                        });
                    }
                }
                CanonicalValue::Array(values) => {
                    for value in values {
                        visit(value, depth + 1, maximum)?;
                    }
                }
                CanonicalValue::Object(values) => {
                    for (key, value) in values {
                        crate::validation::text("canonical_value.key", key, 256)?;
                        visit(value, depth + 1, maximum)?;
                    }
                }
                CanonicalValue::Null
                | CanonicalValue::Bool(_)
                | CanonicalValue::Signed(_)
                | CanonicalValue::Unsigned(_) => {}
            }
            Ok(())
        }
        visit(self, 0, maximum)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CanonicalError {
    #[error("serialize canonical document: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("floating-point JSON numbers are forbidden in canonical documents")]
    FloatingPoint,
}

/// Serialize to compact JSON with object keys sorted recursively.
///
/// This is the canonical byte contract used for document fingerprints and
/// signatures. Arrays retain their order. Strings use serde_json's stable JSON
/// escaping. Integers retain their exact decimal representation.
pub fn canonical_json_bytes(value: &(impl Serialize + ?Sized)) -> Result<Vec<u8>, CanonicalError> {
    let value = serde_json::to_value(value)?;
    let mut output = Vec::new();
    write_value(&value, &mut output)?;
    Ok(output)
}

fn write_value(value: &serde_json::Value, output: &mut Vec<u8>) -> Result<(), CanonicalError> {
    match value {
        serde_json::Value::Null => output.extend_from_slice(b"null"),
        serde_json::Value::Bool(value) => {
            output.extend_from_slice(if *value { b"true" } else { b"false" })
        }
        serde_json::Value::Number(value) => {
            if value.as_i64().is_none() && value.as_u64().is_none() {
                return Err(CanonicalError::FloatingPoint);
            }
            output.extend_from_slice(value.to_string().as_bytes());
        }
        serde_json::Value::String(value) => {
            serde_json::to_writer(output, value)?;
        }
        serde_json::Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_value(value, output)?;
            }
            output.push(b']');
        }
        serde_json::Value::Object(values) => {
            output.push(b'{');
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key)?;
                output.push(b':');
                write_value(&values[key], output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

/// A validated, canonical, content-addressed protocol document.
pub trait CanonicalDocument: Validate + Serialize {
    fn canonical_bytes(&self) -> Result<Vec<u8>, CanonicalDocumentError> {
        self.validate()?;
        Ok(canonical_json_bytes(self)?)
    }

    fn canonical_digest(&self) -> Result<Digest32, CanonicalDocumentError> {
        Ok(Digest32::digest_bytes(self.canonical_bytes()?))
    }
}

impl<T> CanonicalDocument for T where T: Validate + Serialize {}

#[derive(Debug, thiserror::Error)]
pub enum CanonicalDocumentError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    Canonical(#[from] CanonicalError),
}

/// Prefix canonical JSON with an exact protocol domain before signing.
pub(crate) fn domain_separated_bytes(
    domain: &'static [u8],
    value: &impl Serialize,
) -> Result<Vec<u8>, CanonicalError> {
    debug_assert_eq!(domain.last(), Some(&0), "signature domain must end in NUL");
    let canonical = canonical_json_bytes(value)?;
    let mut bytes = Vec::with_capacity(domain.len() + canonical.len());
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&canonical);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recursively_sorts_object_keys_and_keeps_array_order() {
        let value = serde_json::json!({"z": 1, "a": [{"b": 2, "a": 3}, 4]});
        assert_eq!(
            canonical_json_bytes(&value).unwrap(),
            br#"{"a":[{"a":3,"b":2},4],"z":1}"#
        );
    }

    #[test]
    fn canonical_contract_rejects_floating_point_numbers() {
        assert!(matches!(
            canonical_json_bytes(&serde_json::json!({"n": 1.5})),
            Err(CanonicalError::FloatingPoint)
        ));
    }
}
