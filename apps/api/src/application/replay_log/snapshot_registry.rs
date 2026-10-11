//! Content-deduplicated typed registry tree. Nodes use backward references only, in
//! deterministic first-use/postorder. Expansion is charged before cloning referenced data.
use super::record::{CodecError, MAX_SNAPSHOT_JSON};
use super::snapshot_value::error;
use prost::Message;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, PartialEq, Message)]
pub(super) struct Registry {
    #[prost(message, repeated, tag = "1")]
    nodes: Vec<Node>,
    #[prost(uint32, tag = "2")]
    root: u32,
}
#[derive(Clone, PartialEq, Message)]
struct Node {
    #[prost(oneof = "Kind", tags = "1,2,3,4,5,6,7")]
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
    Array(References),
    #[prost(message, tag = "7")]
    Object(References),
}
#[derive(Clone, PartialEq, Message)]
struct References {
    // Objects alternate key-string/value IDs. Arrays contain ordered value IDs.
    #[prost(uint32, repeated, packed = "true", tag = "1")]
    ids: Vec<u32>,
}
impl Registry {
    pub(super) fn pack<T: Serialize>(rules: &T) -> Result<Self, CodecError> {
        let mut nodes = Vec::new();
        let root = intern(
            serde_json::to_value(rules).map_err(error)?,
            &mut nodes,
            &mut BTreeMap::new(),
            0,
        )?;
        Ok(Self { nodes, root })
    }
    pub(super) fn unpack<T: DeserializeOwned>(self) -> Result<T, CodecError> {
        // Charge each expanded subtree before recursively materializing only the root.
        // No cache of cloned expanded subtrees: memory is linear in the final DTO.
        let mut sizes = Vec::<usize>::new();
        let mut depths = Vec::<u32>::new();
        for node in &self.nodes {
            let size = match node
                .kind
                .as_ref()
                .ok_or_else(|| error("registry node kind"))?
            {
                Kind::Null(_) | Kind::Bool(_) => 8,
                Kind::Signed(_) | Kind::Unsigned(_) => 24,
                Kind::String(s) => serde_json::to_vec(s)
                    .map_err(error)?
                    .len()
                    .saturating_add(8),
                Kind::Array(r) | Kind::Object(r) => {
                    r.ids.iter().try_fold(8_usize, |size, id| {
                        size.checked_add(
                            *sizes
                                .get(usize::try_from(*id).map_err(error)?)
                                .ok_or_else(|| error("non-backward registry reference"))?,
                        )
                        .and_then(|n| n.checked_add(8))
                        .ok_or_else(|| error("registry expansion overflow"))
                    })?
                },
            };
            let depth = match node.kind.as_ref() {
                Some(Kind::Array(r) | Kind::Object(r)) => {
                    r.ids.iter().try_fold(0_u32, |depth, id| {
                        let child = *depths
                            .get(usize::try_from(*id).map_err(error)?)
                            .ok_or_else(|| error("registry depth reference"))?;
                        Ok::<_, CodecError>(depth.max(child.saturating_add(1)))
                    })?
                },
                _ => 0,
            };
            if depth > 32 {
                return Err(error("registry nesting exceeds 32"));
            }
            depths.push(depth);
            if size > MAX_SNAPSHOT_JSON {
                return Err(error("registry expansion exceeds 32 MiB"));
            }
            sizes.push(size);
        }
        serde_json::from_value(expand(self.root, &self.nodes)?).map_err(error)
    }
}
fn expand(id: u32, nodes: &[Node]) -> Result<Value, CodecError> {
    let node = nodes
        .get(usize::try_from(id).map_err(error)?)
        .ok_or_else(|| error("registry reference"))?;
    Ok(
        match node
            .kind
            .as_ref()
            .ok_or_else(|| error("registry node kind"))?
        {
            Kind::Null(true) => Value::Null,
            Kind::Null(false) => return Err(error("registry null marker")),
            Kind::Bool(v) => Value::Bool(*v),
            Kind::Signed(v) if *v < 0 => Value::from(*v),
            Kind::Signed(_) => return Err(error("registry signed integer")),
            Kind::Unsigned(v) => Value::from(*v),
            Kind::String(v) => Value::String(v.clone()),
            Kind::Array(r) => Value::Array(
                r.ids
                    .iter()
                    .map(|id| expand(*id, nodes))
                    .collect::<Result<_, _>>()?,
            ),
            Kind::Object(r) => {
                let (pairs, tail) = r.ids.as_chunks::<2>();
                if !tail.is_empty() {
                    return Err(error("registry object pair"));
                }
                let mut fields = serde_json::Map::new();
                let mut previous = None;
                for [key, value] in pairs {
                    let Value::String(key) = expand(*key, nodes)? else {
                        return Err(error("registry object key"));
                    };
                    if previous.as_ref().is_some_and(|p| p >= &key) {
                        return Err(error("registry key order"));
                    }
                    previous = Some(key.clone());
                    fields.insert(key, expand(*value, nodes)?);
                }
                Value::Object(fields)
            },
        },
    )
}

fn intern(
    value: Value,
    nodes: &mut Vec<Node>,
    index: &mut BTreeMap<Vec<u8>, u32>,
    depth: usize,
) -> Result<u32, CodecError> {
    if depth > 32 {
        return Err(error("registry nesting exceeds 32"));
    }
    let kind = match value {
        Value::Null => Kind::Null(true),
        Value::Bool(v) => Kind::Bool(v),
        Value::Number(v) => {
            if let Some(v) = v.as_u64() {
                Kind::Unsigned(v)
            } else {
                Kind::Signed(v.as_i64().ok_or_else(|| error("noninteger registry"))?)
            }
        },
        Value::String(v) => Kind::String(v),
        Value::Array(values) => Kind::Array(References {
            ids: values
                .into_iter()
                .map(|v| intern(v, nodes, index, depth.saturating_add(1)))
                .collect::<Result<_, _>>()?,
        }),
        Value::Object(fields) => {
            let mut ids = Vec::new();
            for (key, value) in fields {
                ids.push(intern(Value::String(key), nodes, index, depth.saturating_add(1))?);
                ids.push(intern(value, nodes, index, depth.saturating_add(1))?);
            }
            Kind::Object(References { ids })
        },
    };
    let node = Node { kind: Some(kind) };
    let key = node.encode_to_vec();
    if let Some(id) = index.get(&key) {
        return Ok(*id);
    }
    let id = u32::try_from(nodes.len()).map_err(error)?;
    nodes.push(node);
    index.insert(key, id);
    Ok(id)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn rejects_forward_cycles_and_exponential_registry_expansion() {
        let cycle = Registry {
            nodes: vec![Node {
                kind: Some(Kind::Array(References { ids: vec![0] })),
            }],
            root: 0,
        };
        assert!(cycle
            .unpack::<Value>()
            .unwrap_err()
            .0
            .contains("non-backward"));
        let mut nodes = vec![Node {
            kind: Some(Kind::String("coefficient".into())),
        }];
        for previous in 0..30 {
            nodes.push(Node {
                kind: Some(Kind::Array(References {
                    ids: vec![previous, previous],
                })),
            });
        }
        assert!(Registry { nodes, root: 30 }
            .unpack::<Value>()
            .unwrap_err()
            .0
            .contains("expansion exceeds"));
    }
}
