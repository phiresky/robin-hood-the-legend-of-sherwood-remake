//! Allocation-free validation of an explicitly projected, JSON-persisted value.
//!
//! This visitor neither selects state nor reconstructs a runtime. Owners must
//! first construct their typed persisted projection. Validation then preserves
//! the old JSON boundary's finite-number, map-key and nesting requirements,
//! including custom serializers' errors and deliberate string encodings.

use serde::{Deserialize, Serialize, ser};

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{0}")]
pub struct PersistenceValidationError(String);

impl ser::Error for PersistenceValidationError {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self(message.to_string())
    }
}

/// Validate without creating JSON, a value tree, or a second runtime.
pub fn validate<T: Serialize + ?Sized>(value: &T) -> Result<(), PersistenceValidationError> {
    value.serialize(Validator {
        key: false,
        depth: 0,
    })
}

#[derive(Clone, Copy)]
struct Validator {
    key: bool,
    depth: u8,
}

impl Validator {
    fn value(self) -> Result<(), PersistenceValidationError> {
        if self.key {
            Err(ser::Error::custom(
                "persisted map key must be a string-compatible scalar",
            ))
        } else {
            Ok(())
        }
    }

    fn compound(self) -> Result<Self, PersistenceValidationError> {
        self.value()?;
        // serde_json's default decoder rejects the 128th nested array/object.
        if self.depth >= 127 {
            return Err(ser::Error::custom(
                "persisted value exceeds JSON nesting limit",
            ));
        }
        Ok(Self {
            key: false,
            depth: self.depth + 1,
        })
    }
}

macro_rules! scalar {
    ($($method:ident($type:ty)),* $(,)?) => {$ (
        fn $method(self, _: $type) -> Result<(), Self::Error> { Ok(()) }
    )*};
}

impl ser::Serializer for Validator {
    type Ok = ();
    type Error = PersistenceValidationError;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    scalar!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_i128(i128),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_u128(u128),
        serialize_char(char),
        serialize_str(&str)
    );

    fn serialize_f32(self, value: f32) -> Result<(), Self::Error> {
        if value.is_finite() {
            Ok(())
        } else {
            Err(ser::Error::custom("nonfinite f32 in persisted state"))
        }
    }
    fn serialize_f64(self, value: f64) -> Result<(), Self::Error> {
        if value.is_finite() {
            Ok(())
        } else {
            Err(ser::Error::custom("nonfinite f64 in persisted state"))
        }
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<(), Self::Error> {
        self.compound().map(|_| ())
    }
    fn serialize_none(self) -> Result<(), Self::Error> {
        self.value()
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), Self::Error> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Self::Error> {
        self.value()
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Self::Error> {
        self.value()
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(self.compound()?)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self, Self::Error> {
        self.compound()
    }
    fn serialize_tuple(self, _: usize) -> Result<Self, Self::Error> {
        self.compound()
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self, Self::Error> {
        self.compound()
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self, Self::Error> {
        self.compound()?.compound()
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self, Self::Error> {
        self.compound()
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self, Self::Error> {
        self.compound()
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self, Self::Error> {
        self.compound()?.compound()
    }
    fn collect_str<T: std::fmt::Display + ?Sized>(self, value: &T) -> Result<(), Self::Error> {
        // A lawful sentinel encoded as a string is not a serialized float.
        use std::fmt::Write;
        DisplaySink
            .write_fmt(format_args!("{value}"))
            .map_err(ser::Error::custom)
    }
    fn is_human_readable(&self) -> bool {
        true
    }
}

struct DisplaySink;
impl std::fmt::Write for DisplaySink {
    fn write_str(&mut self, _: &str) -> std::fmt::Result {
        Ok(())
    }
}

macro_rules! compound {
    ($trait:ident, $method:ident $(, $key:ident)?) => {
        impl ser::$trait for Validator {
            type Ok = ();
            type Error = PersistenceValidationError;
            fn $method<T: Serialize + ?Sized>(&mut self, $($key: &'static str,)? value: &T) -> Result<(), Self::Error> {
                $(let _ = $key;)?
                value.serialize(*self)
            }
            fn end(self) -> Result<(), Self::Error> { Ok(()) }
        }
    };
}

compound!(SerializeSeq, serialize_element);
compound!(SerializeTuple, serialize_element);
compound!(SerializeTupleStruct, serialize_field);
compound!(SerializeTupleVariant, serialize_field);
compound!(SerializeStruct, serialize_field, key);
compound!(SerializeStructVariant, serialize_field, key);

impl ser::SerializeMap for Validator {
    type Ok = ();
    type Error = PersistenceValidationError;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Self::Error> {
        key.serialize(Self {
            key: true,
            depth: self.depth,
        })
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(*self)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn finite_numbers_and_json_map_keys_pass() {
        validate(&(f32::MAX, f64::MIN, -0.0f32, Some(7u128))).unwrap();
        validate(&BTreeMap::from([(7u32, vec![0.25f32])])).unwrap();
        validate(&BTreeMap::from([(false, "no"), (true, "yes")])).unwrap();
        assert!(validate(&BTreeMap::from([((1u8, 2u8), 3u8)])).is_err());
    }

    #[test]
    fn nonfinite_persisted_fields_fail_without_reconstructing() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                validate(&vec![value])
                    .unwrap_err()
                    .to_string()
                    .contains("nonfinite f32")
            );
        }
        assert!(
            validate(&Some(f64::NAN))
                .unwrap_err()
                .to_string()
                .contains("nonfinite f64")
        );
    }

    #[derive(Deserialize)]
    struct StringSentinel(f64);
    impl Serialize for StringSentinel {
        fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(&self.0)
        }
    }

    #[test]
    fn deliberate_string_sentinels_remain_valid() {
        validate(&StringSentinel(f64::INFINITY)).unwrap();
        assert_eq!(
            serde_json::to_string(&StringSentinel(f64::INFINITY)).unwrap(),
            "\"inf\""
        );
    }

    #[derive(Deserialize)]
    struct Rejected;
    impl Serialize for Rejected {
        fn serialize<S: ser::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(ser::Error::custom("active callback cannot be persisted"))
        }
    }

    #[test]
    fn custom_boundary_errors_are_not_ignored() {
        assert_eq!(
            validate(&Rejected).unwrap_err().to_string(),
            "active callback cannot be persisted"
        );
    }

    #[test]
    fn nesting_limit_matches_the_default_json_decoder() {
        for depth in [126usize, 127, 128, 129] {
            let mut value = serde_json::Value::Null;
            for _ in 0..depth {
                value = serde_json::Value::Array(vec![value]);
            }
            let bytes = serde_json::to_vec(&value).unwrap();
            assert_eq!(
                validate(&value).is_ok(),
                serde_json::from_slice::<serde_json::Value>(&bytes).is_ok(),
                "depth {depth}"
            );
        }
    }
}
