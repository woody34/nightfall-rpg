//! Recorded item admission inputs. Pending manifests carry contents, never database pointers.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{ItemError, Phase4Checkpoint};
use crate::domain::class::ClassId;

/// The only item grants permitted during admission; equipment follows XML row order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct StarterKitSeed {
    pub template_id: u32,
    pub count: u64,
    pub equipped: bool,
}

/// Exact pending kit contents shared with the API's immutable pending-request projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemKitManifest {
    pub source_class_id: ClassId,
    pub items: Vec<StarterKitSeed>,
}

/// Dev grant contents; the client cannot inject enchantment, augmentation or identities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemGrantManifest {
    pub template_id: u32,
    pub count: u64,
}

/// Creation/backfill differ in their durable operation, but share kit materialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "manifest",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ItemMaterialization {
    /// First kit for a newly created character.
    Starter(ItemKitManifest),
    /// First kit for a character predating Phase4.
    Backfill(ItemKitManifest),
    /// Authenticated dev-only grant, already gated by the application.
    Grant(ItemGrantManifest),
}

impl ItemMaterialization {
    /// API manifest hash: compact sorted-key JSON of the bare typed manifest. The kind is
    /// separately bound by the request fingerprint and recorded enum, never inferred.
    pub fn manifest_hash(&self) -> Result<[u8; 32], ItemError> {
        let value = match self {
            Self::Starter(kit) | Self::Backfill(kit) => serde_json::to_value(kit),
            Self::Grant(grant) => serde_json::to_value(grant),
        }
        .map_err(|_| ItemError("manifest encoding failed"))?;
        let bytes = serde_json::to_vec(&sort_manifest(value))
            .map_err(|_| ItemError("manifest encoding failed"))?;
        Ok(Sha256::digest(bytes).into())
    }

    fn validate(&self) -> Result<(), ItemError> {
        let valid = |template: u32, count: u64| {
            template > 0
                && i32::try_from(template).is_ok()
                && count > 0
                && i64::try_from(count).is_ok()
        };
        match self {
            Self::Starter(kit) | Self::Backfill(kit) => {
                if kit.items.is_empty()
                    || kit
                        .items
                        .iter()
                        .any(|item| !valid(item.template_id, item.count))
                {
                    return Err(ItemError("invalid pending kit contents"));
                }
            },
            Self::Grant(grant) => {
                if !valid(grant.template_id, grant.count) {
                    return Err(ItemError("invalid pending grant contents"));
                }
            },
        }
        Ok(())
    }
}

fn sort_manifest(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(fields) => {
            let ordered: std::collections::BTreeMap<_, _> = fields.into_iter().collect();
            serde_json::Value::Object(
                ordered
                    .into_iter()
                    .map(|(key, value)| (key, sort_manifest(value)))
                    .collect(),
            )
        },
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(sort_manifest).collect())
        },
        value @ (serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_)) => value,
    }
}

/// One immutable request, bound to the enclosing authenticated character load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct PendingItemMaterialization {
    pub request_id: uuid::Uuid,
    pub request_fingerprint: String,
    pub manifest_version: u32,
    pub manifest_hash: [u8; 32],
    pub materialization: ItemMaterialization,
}

/// `PlayerLoad` tag10. None checkpoint means a pending first materialization, never legacy
/// equipment or an automatically synthesized empty inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Phase4PlayerLoad {
    pub checkpoint: Option<Phase4Checkpoint>,
    pub pending: Vec<PendingItemMaterialization>,
}
impl Phase4PlayerLoad {
    /// Validate the recorded input without allocating IDs or reading a repository. Kit
    /// lineage, template eligibility and capacity additionally require the epoch rules.
    pub fn validate_structure(&self, phase3_sources: usize) -> Result<(), ItemError> {
        if let Some(checkpoint) = &self.checkpoint {
            checkpoint.validate_structure(phase3_sources)?;
        }
        let mut previous = None;
        let mut kit_count = 0_u8;
        for request in &self.pending {
            if request.request_id.is_nil()
                || previous.is_some_and(|id| id >= request.request_id)
                || request.request_fingerprint.is_empty()
                || request.manifest_version != 1
                || request.materialization.manifest_hash()? != request.manifest_hash
            {
                return Err(ItemError("invalid or unordered pending item manifest"));
            }
            previous = Some(request.request_id);
            request.materialization.validate()?;
            if matches!(
                request.materialization,
                ItemMaterialization::Starter(_) | ItemMaterialization::Backfill(_)
            ) {
                kit_count = kit_count
                    .checked_add(1)
                    .ok_or(ItemError("duplicate kit materialization"))?;
            }
        }
        let materialized = self
            .checkpoint
            .as_ref()
            .is_some_and(|cp| cp.starter_kit_receipt.is_some());
        if (materialized && kit_count != 0) || (!materialized && kit_count != 1) {
            return Err(ItemError("missing or repeated starter materialization"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn source_row_order_hash_and_once_only_kit_are_recorded() {
        let kit = ItemMaterialization::Starter(ItemKitManifest {
            source_class_id: ClassId(0),
            items: vec![
                StarterKitSeed {
                    template_id: 2369,
                    count: 1,
                    equipped: true,
                },
                StarterKitSeed {
                    template_id: 10,
                    count: 1,
                    equipped: false,
                },
            ],
        });
        let mut load = Phase4PlayerLoad {
            checkpoint: None,
            pending: vec![PendingItemMaterialization {
                request_id: uuid::Uuid::from_u128(1),
                request_fingerprint: "fixture-request".into(),
                manifest_version: 1,
                manifest_hash: kit.manifest_hash().unwrap(),
                materialization: kit,
            }],
        };
        load.validate_structure(0).unwrap();
        let bytes = serde_json::to_vec(&load).unwrap();
        assert_eq!(serde_json::from_slice::<Phase4PlayerLoad>(&bytes).unwrap(), load);
        if let ItemMaterialization::Starter(kit) =
            &mut load.pending.first_mut().unwrap().materialization
        {
            kit.items.reverse();
        }
        assert!(load.validate_structure(0).is_err());
        load.pending.clear();
        assert!(load.validate_structure(0).is_err());
    }
}
