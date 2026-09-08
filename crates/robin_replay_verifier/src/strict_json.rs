//! Duplicate-aware JSON admission for signed worker documents.
//!
//! `serde_json::Value` normally implements last-key-wins semantics. That is
//! unsafe for signed requests: an auditor, canonicalizer, and typed decoder
//! must never be able to observe different values for the same object member.
//! This decoder rejects duplicates recursively before deserializing the typed
//! document and rejects trailing JSON values as well.

use serde::de::DeserializeSeed as _;

#[derive(Debug, thiserror::Error)]
pub enum StrictJsonError {
    #[error("duplicate JSON object key `{0}`")]
    DuplicateKey(String),
    #[error("invalid or ambiguous JSON: {0}")]
    Syntax(#[from] serde_json::Error),
    #[error("JSON does not match the required document: {0}")]
    Document(serde_json::Error),
}

/// Deserialize one JSON document after recursively rejecting duplicate keys.
pub fn from_slice<T>(bytes: &[u8]) -> Result<T, StrictJsonError>
where
    T: serde::de::DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let mut duplicate = None;
    let decoded = UniqueValueSeed(&mut duplicate).deserialize(&mut deserializer);
    if let Some(key) = duplicate {
        return Err(StrictJsonError::DuplicateKey(key));
    }
    let UniqueValue(value) = decoded?;
    deserializer.end()?;
    serde_json::from_value(value).map_err(StrictJsonError::Document)
}

struct UniqueValue(serde_json::Value);

struct UniqueValueSeed<'a>(&'a mut Option<String>);

impl<'de> serde::de::DeserializeSeed<'de> for UniqueValueSeed<'_> {
    type Value = UniqueValue;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueValueVisitor(self.0))
    }
}

struct UniqueValueVisitor<'a>(&'a mut Option<String>);

impl<'de> serde::de::Visitor<'de> for UniqueValueVisitor<'_> {
    type Value = UniqueValue;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("one JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .map(UniqueValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(UniqueValue(value)) = sequence.next_element_seed(UniqueValueSeed(self.0))? {
            values.push(value);
        }
        Ok(UniqueValue(serde_json::Value::Array(values)))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let mut values = serde_json::Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                *self.0 = Some(key.clone());
                return Err(serde::de::Error::custom(format!(
                    "duplicate JSON object key `{key}`"
                )));
            }
            let UniqueValue(value) = object.next_value_seed(UniqueValueSeed(self.0))?;
            values.insert(key, value);
        }
        Ok(UniqueValue(serde_json::Value::Object(values)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Fixture {
        outer: Vec<Inner>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Inner {
        value: u32,
    }

    #[test]
    fn accepts_one_exact_document() {
        assert_eq!(
            from_slice::<Fixture>(br#"{"outer":[{"value":7}]}"#).unwrap(),
            Fixture {
                outer: vec![Inner { value: 7 }]
            }
        );
    }

    #[test]
    fn rejects_duplicates_at_every_recursive_depth() {
        for document in [
            br#"{"outer":[],"outer":[]}"#.as_slice(),
            br#"{"outer":[{"value":1,"value":2}]}"#.as_slice(),
        ] {
            let error = from_slice::<Fixture>(document).unwrap_err();
            assert!(matches!(error, StrictJsonError::DuplicateKey(_)));
        }
    }

    #[test]
    fn rejects_trailing_values_and_unknown_members() {
        assert!(from_slice::<Fixture>(br#"{"outer":[]} {}"#).is_err());
        assert!(from_slice::<Fixture>(br#"{"outer":[],"future":true}"#).is_err());
    }
}
