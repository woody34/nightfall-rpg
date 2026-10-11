//! Lossless protobuf scalar/container vocabulary for immutable registry and small sidecar
//! DTOs. No JSON bytes, floating point, numeric coercion or application data lookup. The
//! domain's strict serde decoders still own validation of these versioned DTOs.
use prost::Message;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

use super::record::CodecError;

#[derive(Clone, PartialEq, Message)]
pub(super) struct PbValue {
    #[prost(oneof = "Kind", tags = "1, 2, 3, 4, 5, 6, 7, 8")]
    kind: Option<Kind>,
}
#[derive(Clone, PartialEq, prost::Oneof)]
enum Kind {
    #[prost(bool, tag = "1")]
    Null(bool),
    #[prost(bool, tag = "2")]
    Bool(bool),
    #[prost(sint64, tag = "3")]
    Signed(i64),
    #[prost(uint64, tag = "4")]
    Unsigned(u64),
    #[prost(string, tag = "5")]
    String(String),
    #[prost(message, tag = "6")]
    Array(Array),
    #[prost(message, tag = "7")]
    Object(Object),
    #[prost(bytes, tag = "8")]
    Uuid(Vec<u8>),
}
#[derive(Clone, PartialEq, Message)]
struct Array {
    #[prost(message, repeated, tag = "1")]
    values: Vec<PbValue>,
}
#[derive(Clone, PartialEq, Message)]
struct Object {
    #[prost(message, repeated, tag = "1")]
    fields: Vec<Field>,
}
#[derive(Clone, PartialEq, Message)]
struct Field {
    #[prost(string, tag = "1")]
    key: String,
    #[prost(message, optional, tag = "2")]
    value: Option<PbValue>,
}

pub(super) fn error(message: impl std::fmt::Display) -> CodecError {
    CodecError(format!("Snapshot9 binary: {message}"))
}
pub(super) fn pack<T: Serialize>(value: &T) -> Result<PbValue, CodecError> {
    from_value(serde_json::to_value(value).map_err(error)?, 0)
}
pub(super) fn unpack<T: DeserializeOwned>(value: PbValue) -> Result<T, CodecError> {
    serde_json::from_value(into_value(value, 0)?).map_err(error)
}
fn from_value(value: Value, depth: usize) -> Result<PbValue, CodecError> {
    if depth > 32 {
        return Err(error("DTO nesting exceeds 32"));
    }
    let kind = match value {
        Value::Null => Kind::Null(true),
        Value::Bool(v) => Kind::Bool(v),
        Value::Number(v) => {
            if let Some(v) = v.as_u64() {
                Kind::Unsigned(v)
            } else {
                Kind::Signed(v.as_i64().ok_or_else(|| error("non-integer DTO"))?)
            }
        },
        Value::String(v) => match uuid::Uuid::parse_str(&v) {
            Ok(id) if id.to_string() == v => Kind::Uuid(id.as_bytes().to_vec()),
            _ => Kind::String(v),
        },
        Value::Array(values) => Kind::Array(Array {
            values: values
                .into_iter()
                .map(|v| from_value(v, depth.saturating_add(1)))
                .collect::<Result<_, _>>()?,
        }),
        Value::Object(fields) => Kind::Object(Object {
            fields: fields
                .into_iter()
                .map(|(key, v)| {
                    Ok(Field {
                        key,
                        value: Some(from_value(v, depth.saturating_add(1))?),
                    })
                })
                .collect::<Result<_, CodecError>>()?,
        }),
    };
    Ok(PbValue { kind: Some(kind) })
}
fn into_value(value: PbValue, depth: usize) -> Result<Value, CodecError> {
    if depth > 32 {
        return Err(error("DTO nesting exceeds 32"));
    }
    Ok(match value.kind.ok_or_else(|| error("missing value kind"))? {
        Kind::Null(true) => Value::Null,
        Kind::Null(false) => return Err(error("invalid null marker")),
        Kind::Bool(v) => Value::Bool(v),
        Kind::Signed(v) if v < 0 => Value::from(v),
        Kind::Signed(_) => return Err(error("noncanonical signed integer")),
        Kind::Unsigned(v) => Value::from(v),
        Kind::String(v) => Value::String(v),
        Kind::Uuid(v) => Value::String(uuid::Uuid::from_slice(&v).map_err(error)?.to_string()),
        Kind::Array(v) => Value::Array(
            v.values
                .into_iter()
                .map(|v| into_value(v, depth.saturating_add(1)))
                .collect::<Result<_, _>>()?,
        ),
        Kind::Object(v) => {
            let mut fields = serde_json::Map::new();
            let mut previous = None;
            for field in v.fields {
                if previous.as_ref().is_some_and(|key| key >= &field.key) {
                    return Err(error("unordered/duplicate object key"));
                }
                previous = Some(field.key.clone());
                fields.insert(
                    field.key,
                    into_value(
                        field.value.ok_or_else(|| error("missing field value"))?,
                        depth.saturating_add(1),
                    )?,
                );
            }
            Value::Object(fields)
        },
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn exact_integer_extremes_and_unknown_numeric_types() {
        let value = serde_json::json!({"minimum": i64::MIN, "maximum": u64::MAX, "null": null,
            "array": [true, false, "1/3", "", [], {}]});
        assert_eq!(unpack::<Value>(pack(&value).unwrap()).unwrap(), value);
        assert!(pack(&serde_json::json!(0.5)).is_err());
        assert!(unpack::<Value>(PbValue {
            kind: Some(Kind::Signed(0))
        })
        .is_err());
        assert!(unpack::<Value>(PbValue { kind: None }).is_err());
    }
}
