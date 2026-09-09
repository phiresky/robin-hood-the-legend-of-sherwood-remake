//! JSON-compatible conversion for serde values with non-string map keys.

/// Preserve diagnostic map-key spelling while rejecting lossy key collisions.
pub fn to_json_value<T>(value: &T) -> Result<serde_json::Value, serde_value::SerializerError>
where
    T: serde::Serialize + ?Sized,
{
    serde_value::to_value(value).and_then(serde_value_to_json)
}

fn serde_value_to_json(
    value: serde_value::Value,
) -> Result<serde_json::Value, serde_value::SerializerError> {
    Ok(match value {
        serde_value::Value::Bool(v) => serde_json::Value::Bool(v),
        serde_value::Value::I8(v) => serde_json::json!(v),
        serde_value::Value::I16(v) => serde_json::json!(v),
        serde_value::Value::I32(v) => serde_json::json!(v),
        serde_value::Value::I64(v) => serde_json::json!(v),
        serde_value::Value::U8(v) => serde_json::json!(v),
        serde_value::Value::U16(v) => serde_json::json!(v),
        serde_value::Value::U32(v) => serde_json::json!(v),
        serde_value::Value::U64(v) => serde_json::json!(v),
        serde_value::Value::F32(v) => serde_json::json!(v),
        serde_value::Value::F64(v) => serde_json::json!(v),
        serde_value::Value::Char(v) => serde_json::json!(v.to_string()),
        serde_value::Value::String(v) => serde_json::Value::String(v),
        serde_value::Value::Bytes(v) => serde_json::json!(v),
        serde_value::Value::Unit => serde_json::Value::Null,
        serde_value::Value::Option(v) => v
            .map(|v| serde_value_to_json(*v))
            .transpose()?
            .unwrap_or(serde_json::Value::Null),
        serde_value::Value::Newtype(v) => serde_value_to_json(*v)?,
        serde_value::Value::Seq(values) => serde_json::Value::Array(
            values
                .into_iter()
                .map(serde_value_to_json)
                .collect::<Result<_, _>>()?,
        ),
        serde_value::Value::Map(entries) => {
            let mut object = serde_json::Map::new();
            for (key, value) in entries {
                let key = serde_value_key_to_string(key);
                if object.contains_key(&key) {
                    return Err(serde::ser::Error::custom(format!(
                        "distinct map keys encode to the same JSON key {key:?}"
                    )));
                }
                object.insert(key, serde_value_to_json(value)?);
            }
            serde_json::Value::Object(object)
        }
    })
}

fn serde_value_key_to_string(key: serde_value::Value) -> String {
    match key {
        serde_value::Value::String(v) => v,
        serde_value::Value::Char(v) => v.to_string(),
        serde_value::Value::Bool(v) => v.to_string(),
        serde_value::Value::I8(v) => v.to_string(),
        serde_value::Value::I16(v) => v.to_string(),
        serde_value::Value::I32(v) => v.to_string(),
        serde_value::Value::I64(v) => v.to_string(),
        serde_value::Value::U8(v) => v.to_string(),
        serde_value::Value::U16(v) => v.to_string(),
        serde_value::Value::U32(v) => v.to_string(),
        serde_value::Value::U64(v) => v.to_string(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_value::Value;
    use std::collections::BTreeMap;

    #[test]
    fn unambiguous_scalar_and_compound_keys_keep_the_existing_representation() {
        let value = Value::Map(BTreeMap::from([
            (Value::Bool(true), Value::U64(u64::MAX)),
            (
                Value::I32(-1),
                Value::Option(Some(Box::new(Value::String("雪".into())))),
            ),
            (
                Value::String("nested".into()),
                Value::Seq(vec![Value::Unit, Value::I64(i64::MIN)]),
            ),
        ]));
        assert_eq!(
            to_json_value(&value).unwrap(),
            serde_json::json!({
                "true": u64::MAX, "-1": "雪", "nested": [null, i64::MIN]
            })
        );
        let pairs = BTreeMap::from([((1i32, 2i32), "pair")]);
        assert_eq!(
            to_json_value(&pairs).unwrap(),
            serde_json::json!({"Seq([I32(1), I32(2)])": "pair"})
        );
    }

    #[test]
    fn distinct_map_keys_must_not_silently_overwrite_each_other() {
        for keys in [
            [Value::Bool(true), Value::String("true".into())],
            [Value::U8(1), Value::I32(1)],
            [Value::Char('x'), Value::String("x".into())],
            [Value::Unit, Value::String("Unit".into())],
        ] {
            let values = Value::Map(BTreeMap::from([
                (keys[0].clone(), Value::String("first".into())),
                (keys[1].clone(), Value::String("second".into())),
            ]));
            let error = to_json_value(&values).unwrap_err();
            assert!(error.to_string().contains("same JSON key"));
            // Collisions inside containers must propagate to the root.
            let nested = Value::Map(BTreeMap::from([(
                Value::String("outer".into()),
                Value::Seq(vec![Value::Option(Some(Box::new(Value::Newtype(
                    Box::new(values),
                ))))]),
            )]));
            assert!(to_json_value(&nested).is_err());
        }
    }
}
