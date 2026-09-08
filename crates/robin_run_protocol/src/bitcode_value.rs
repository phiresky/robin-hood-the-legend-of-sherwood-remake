//! Native bitcode representation of deterministic projection values.
//!
//! A flat preorder stream avoids recursive encoder types. Object keys retain
//! BTreeMap order; integer signedness is normalized just as in canonical JSON.
use crate::{CanonicalValue, ValidationError};
use bitcode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
enum Node {
    Null,
    Bool(bool),
    Negative(i64),
    Unsigned(u64),
    String(String),
    Array(u64),
    Object(u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
pub struct BitcodeValue(Vec<Node>);

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
        fn append(value: &CanonicalValue, nodes: &mut Vec<Node>) {
            match value {
                CanonicalValue::Null => nodes.push(Node::Null),
                CanonicalValue::Bool(v) => nodes.push(Node::Bool(*v)),
                CanonicalValue::Signed(v) if *v < 0 => nodes.push(Node::Negative(*v)),
                CanonicalValue::Signed(v) => nodes.push(Node::Unsigned(*v as u64)),
                CanonicalValue::Unsigned(v) => nodes.push(Node::Unsigned(*v)),
                CanonicalValue::String(v) => nodes.push(Node::String(v.clone())),
                CanonicalValue::Array(values) => {
                    nodes.push(Node::Array(values.len() as u64));
                    for value in values {
                        append(value, nodes);
                    }
                }
                CanonicalValue::Object(values) => {
                    nodes.push(Node::Object(values.len() as u64));
                    for (key, value) in values {
                        nodes.push(Node::String(key.clone()));
                        append(value, nodes);
                    }
                }
            }
        }
        let mut nodes = Vec::new();
        append(value, &mut nodes);
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
