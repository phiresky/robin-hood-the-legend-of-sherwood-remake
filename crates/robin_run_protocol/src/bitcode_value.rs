//! Native bitcode representation of deterministic projection values.
//!
//! A flat preorder stream avoids recursive encoder types. Object keys retain
//! BTreeMap order; integer signedness is normalized just as in canonical JSON.
use crate::{CanonicalValue, ValidationError};
use bitcode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
enum Node<S = String> {
    Null,
    Bool(bool),
    Negative(i64),
    Unsigned(u64),
    String(S),
    Array(u64),
    Object(u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
pub struct BitcodeValue(Vec<Node>);

/// Encoding-only view with the exact owned value's wire layout. String payloads
/// borrow the diagnostic document instead of being copied on every hash.
#[derive(Debug, Serialize, Encode)]
pub struct BitcodeValueRef<'a>(Vec<Node<&'a str>>);

impl<'a> BitcodeValueRef<'a> {
    pub fn from_value(value: &'a CanonicalValue) -> Result<Self, ProjectionBitcodeError> {
        value.validate_depth(128)?;
        let mut nodes = Vec::new();
        append(value, &mut nodes, |string| string);
        Ok(Self(nodes))
    }
}

fn append<'a, S>(
    value: &'a CanonicalValue,
    nodes: &mut Vec<Node<S>>,
    string: impl Fn(&'a str) -> S + Copy,
) {
    match value {
        CanonicalValue::Null => nodes.push(Node::Null),
        CanonicalValue::Bool(v) => nodes.push(Node::Bool(*v)),
        CanonicalValue::Signed(v) if *v < 0 => nodes.push(Node::Negative(*v)),
        CanonicalValue::Signed(v) => nodes.push(Node::Unsigned(*v as u64)),
        CanonicalValue::Unsigned(v) => nodes.push(Node::Unsigned(*v)),
        CanonicalValue::String(v) => nodes.push(Node::String(string(v))),
        CanonicalValue::Array(values) => {
            nodes.push(Node::Array(values.len() as u64));
            for value in values {
                append(value, nodes, string);
            }
        }
        CanonicalValue::Object(values) => {
            nodes.push(Node::Object(values.len() as u64));
            for (key, value) in values {
                nodes.push(Node::String(string(key)));
                append(value, nodes, string);
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectionBitcodeError {
    #[error(transparent)]
    Decode(#[from] bitcode::Error),
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("invalid native projection: {0}")]
    Invalid(&'static str),
}

impl BitcodeValue {
    pub fn from_value(value: &CanonicalValue) -> Result<Self, ProjectionBitcodeError> {
        value.validate_depth(128)?;
        let mut nodes = Vec::new();
        append(value, &mut nodes, str::to_owned);
        Ok(Self(nodes))
    }

    pub fn into_value(self) -> Result<CanonicalValue, ProjectionBitcodeError> {
        use ProjectionBitcodeError::Invalid;
        fn read(
            nodes: &mut std::vec::IntoIter<Node>,
            depth: usize,
        ) -> Result<CanonicalValue, ProjectionBitcodeError> {
            if depth > 128 {
                return Err(Invalid("value nesting exceeds 128"));
            }
            Ok(match nodes.next().ok_or(Invalid("missing value"))? {
                Node::Null => CanonicalValue::Null,
                Node::Bool(v) => CanonicalValue::Bool(v),
                Node::Negative(v) if v < 0 => CanonicalValue::Signed(v),
                Node::Negative(_) => return Err(Invalid("noncanonical signed integer")),
                Node::Unsigned(v) => CanonicalValue::Unsigned(v),
                Node::String(v) => CanonicalValue::String(v),
                Node::Array(count) => {
                    if count > nodes.len() as u64 {
                        return Err(Invalid("array length exceeds remaining values"));
                    }
                    let mut values = Vec::new();
                    for _ in 0..count {
                        values.push(read(nodes, depth + 1)?);
                    }
                    CanonicalValue::Array(values)
                }
                Node::Object(count) => {
                    if count > nodes.len() as u64 / 2 {
                        return Err(Invalid("object length exceeds remaining values"));
                    }
                    let mut values = BTreeMap::new();
                    for _ in 0..count {
                        let Some(Node::String(key)) = nodes.next() else {
                            return Err(Invalid("missing object key"));
                        };
                        if values
                            .last_key_value()
                            .is_some_and(|(last, _)| last >= &key)
                        {
                            return Err(Invalid("object keys must be strictly sorted"));
                        }
                        values.insert(key, read(nodes, depth + 1)?);
                    }
                    CanonicalValue::Object(values)
                }
            })
        }
        let mut nodes = self.0.into_iter();
        let value = read(&mut nodes, 0)?;
        if nodes.next().is_some() {
            return Err(Invalid("trailing values"));
        }
        value.validate_depth(128)?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_nodes_preserve_original_owned_wire_layout() {
        // Independent pre-refactor schema: protects variant indices and the
        // equivalence of String and &str encoders, including mixed variants.
        #[derive(Encode)]
        enum OriginalNode {
            Null,
            Bool(bool),
            Negative(i64),
            Unsigned(u64),
            String(String),
            Array(u64),
            Object(u64),
        }
        #[derive(Encode)]
        struct OriginalValue(Vec<OriginalNode>);
        let value = CanonicalValue::Object(BTreeMap::from([(
            "values".into(),
            CanonicalValue::Array(vec![
                CanonicalValue::Null,
                CanonicalValue::Bool(true),
                CanonicalValue::Signed(i64::MIN),
                CanonicalValue::Signed(7),
                CanonicalValue::Unsigned(u64::MAX),
                CanonicalValue::String("雪".into()),
            ]),
        )]));
        let original = OriginalValue(vec![
            OriginalNode::Object(1),
            OriginalNode::String("values".into()),
            OriginalNode::Array(6),
            OriginalNode::Null,
            OriginalNode::Bool(true),
            OriginalNode::Negative(i64::MIN),
            OriginalNode::Unsigned(7),
            OriginalNode::Unsigned(u64::MAX),
            OriginalNode::String("雪".into()),
        ]);
        assert_eq!(
            bitcode::encode(&original),
            bitcode::encode(&BitcodeValueRef::from_value(&value).unwrap())
        );
        assert_eq!(
            bitcode::encode(&original),
            bitcode::encode(&BitcodeValue::from_value(&value).unwrap())
        );
        let too_long = CanonicalValue::String("a".repeat(16 * 1024 + 1));
        assert!(BitcodeValueRef::from_value(&too_long).is_err());
    }

    #[test]
    fn native_values_round_trip_and_normalize_integer_signedness() {
        let value = CanonicalValue::Object(BTreeMap::from([
            (
                "array".into(),
                CanonicalValue::Array(vec![
                    CanonicalValue::Null,
                    CanonicalValue::Signed(-7),
                    CanonicalValue::Unsigned(u64::MAX),
                ]),
            ),
            ("text".into(), CanonicalValue::String("雪\n\"".into())),
        ]));
        let encoded = bitcode::encode(&BitcodeValue::from_value(&value).unwrap());
        assert_eq!(
            encoded,
            bitcode::encode(&BitcodeValueRef::from_value(&value).unwrap())
        );
        let decoded: BitcodeValue = bitcode::decode(&encoded).unwrap();
        assert_eq!(decoded.into_value().unwrap(), value);
        assert_eq!(
            bitcode::encode(&BitcodeValue::from_value(&CanonicalValue::Signed(7)).unwrap()),
            bitcode::encode(&BitcodeValue::from_value(&CanonicalValue::Unsigned(7)).unwrap())
        );
    }

    #[test]
    fn rejects_malformed_structure_without_allocating_claimed_lengths() {
        for nodes in [
            vec![],
            vec![Node::Null, Node::Null],
            vec![Node::Array(u64::MAX)],
            vec![Node::Object(u64::MAX)],
            vec![Node::Negative(0)],
            vec![
                Node::Object(2),
                Node::String("a".into()),
                Node::Null,
                Node::String("a".into()),
                Node::Null,
            ],
            vec![
                Node::Object(2),
                Node::String("z".into()),
                Node::Null,
                Node::String("a".into()),
                Node::Null,
            ],
        ] {
            assert!(BitcodeValue(nodes).into_value().is_err());
        }
        let mut nested = vec![Node::Array(1); 130];
        nested.push(Node::Null);
        assert!(BitcodeValue(nested).into_value().is_err());
    }

    #[test]
    fn component_format_rejects_json_truncation_and_trailing_bytes() {
        use crate::{SimulationContentComponentDocumentV1, SimulationContentComponentKindV1};
        let document = SimulationContentComponentDocumentV1 {
            schema_version: 1,
            kind: SimulationContentComponentKindV1::Profiles,
            component_schema_version: 1,
            payload: CanonicalValue::Null,
        };
        let mut bytes = document.bitcode_bytes().unwrap();
        assert_eq!(
            SimulationContentComponentDocumentV1::from_bitcode(&bytes).unwrap(),
            document
        );
        for end in 0..bytes.len() {
            assert!(SimulationContentComponentDocumentV1::from_bitcode(&bytes[..end]).is_err());
        }
        bytes.push(0);
        assert!(SimulationContentComponentDocumentV1::from_bitcode(&bytes).is_err());
        assert!(
            SimulationContentComponentDocumentV1::from_bitcode(
                &serde_json::to_vec(&document).unwrap()
            )
            .is_err()
        );
    }
}
