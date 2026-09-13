//! Codec-safe JSON snapshot values.
use bitcode_parity as bitcode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Recursive JSON tree used for high-volume trace snapshots.
///
/// `serde_json::Value` deliberately has no native binary-codec derives. This
/// equivalent tree keeps frame parsing strict; it is the serde
/// (JSONL) view of [`TraceJsonValue`], which stores the same data flat.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum TraceJsonTree {
    Null(()),
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    String(String),
    Array(Vec<TraceJsonTree>),
    Object(BTreeMap<String, TraceJsonTree>),
}

impl TraceJsonTree {
    pub(crate) fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Null(()) => serde_json::Value::Null,
            Self::Bool(value) => (*value).into(),
            Self::Unsigned(value) => (*value).into(),
            Self::Signed(value) => (*value).into(),
            Self::Float(value) => (*value).into(),
            Self::String(value) => value.clone().into(),
            Self::Array(values) => values.iter().map(Self::to_json).collect(),
            Self::Object(values) => values
                .iter()
                .map(|(key, value)| (key.clone(), value.to_json()))
                .collect(),
        }
    }

    pub(crate) fn flatten_into(&self, tokens: &mut Vec<TraceJsonToken>) {
        match self {
            Self::Null(()) => tokens.push(TraceJsonToken::Null),
            Self::Bool(value) => tokens.push(TraceJsonToken::Bool(*value)),
            Self::Unsigned(value) => tokens.push(TraceJsonToken::Unsigned(*value)),
            Self::Signed(value) => tokens.push(TraceJsonToken::Signed(*value)),
            Self::Float(value) => tokens.push(TraceJsonToken::Float(*value)),
            Self::String(value) => tokens.push(TraceJsonToken::String(value.clone())),
            Self::Array(values) => {
                tokens.push(TraceJsonToken::Array(
                    u32::try_from(values.len()).expect("JSON array length exceeds u32"),
                ));
                for value in values {
                    value.flatten_into(tokens);
                }
            }
            Self::Object(values) => {
                tokens.push(TraceJsonToken::Object(
                    u32::try_from(values.len()).expect("JSON object length exceeds u32"),
                ));
                for (key, value) in values {
                    tokens.push(TraceJsonToken::Key(key.clone()));
                    value.flatten_into(tokens);
                }
            }
        }
    }

    pub(crate) fn unflatten(tokens: &mut std::slice::Iter<'_, TraceJsonToken>) -> Self {
        match tokens.next().expect("flat JSON token stream ended early") {
            TraceJsonToken::Null => Self::Null(()),
            TraceJsonToken::Bool(value) => Self::Bool(*value),
            TraceJsonToken::Unsigned(value) => Self::Unsigned(*value),
            TraceJsonToken::Signed(value) => Self::Signed(*value),
            TraceJsonToken::Float(value) => Self::Float(*value),
            TraceJsonToken::String(value) => Self::String(value.clone()),
            TraceJsonToken::Array(len) => {
                Self::Array((0..*len).map(|_| Self::unflatten(tokens)).collect())
            }
            TraceJsonToken::Object(len) => Self::Object(
                (0..*len)
                    .map(|_| {
                        let TraceJsonToken::Key(key) = tokens
                            .next()
                            .expect("flat JSON object ended before its key")
                        else {
                            panic!("flat JSON object entry does not start with a key")
                        };
                        (key.clone(), Self::unflatten(tokens))
                    })
                    .collect(),
            ),
            TraceJsonToken::Key(key) => {
                panic!("unexpected flat JSON key {key:?} in value position")
            }
        }
    }
}

/// One pre-order token of a flattened [`TraceJsonTree`]. `Array`/`Object`
/// carry their child count; object entries are `Key` followed by a value.
#[derive(Clone, Debug, PartialEq, bitcode::Decode, bitcode::Encode)]
pub(crate) enum TraceJsonToken {
    Null,
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    String(String),
    Array(u32),
    Object(u32),
    Key(String),
}

/// Cache-safe JSON value: a [`TraceJsonTree`] stored as a flat pre-order
/// token list. bitcode's derives cannot encode recursive types (the derived
/// encoder would be infinitely sized), so the binary codecs see a plain
/// `Vec<TraceJsonToken>` while serde still reads and writes the JSON shape.
#[derive(Clone, Debug, PartialEq, bitcode::Decode, bitcode::Encode)]
pub(crate) struct TraceJsonValue {
    pub(crate) tokens: Vec<TraceJsonToken>,
}

impl TraceJsonValue {
    pub(crate) fn tree(&self) -> TraceJsonTree {
        let mut tokens = self.tokens.iter();
        let tree = TraceJsonTree::unflatten(&mut tokens);
        assert!(
            tokens.next().is_none(),
            "flat JSON token stream has trailing tokens"
        );
        tree
    }

    pub(crate) fn to_json(&self) -> serde_json::Value {
        self.tree().to_json()
    }
}

impl From<TraceJsonTree> for TraceJsonValue {
    fn from(tree: TraceJsonTree) -> Self {
        let mut tokens = Vec::new();
        tree.flatten_into(&mut tokens);
        Self { tokens }
    }
}

pub(crate) fn missing_legacy_trace_json_value() -> TraceJsonValue {
    TraceJsonTree::Null(()).into()
}

impl Serialize for TraceJsonValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.tree().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TraceJsonValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        TraceJsonTree::deserialize(deserializer).map(Self::from)
    }
}
