//! Frozen pending kit inputs: l2j-dp @3ca488dd data/stats/initialEquipment.xml:4-109.
//! Runtime catalogue validation/materialization is owned by the zone, never this manifest.
use crate::application::ports::{
    PendingItemKind, PendingItemManifest, PendingItemRequest, PendingItemStatus, StarterKitManifest,
};
use crate::domain::{class::ClassId, Character};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Select immutable XML rows using the base creation class, never the current transfer class.
pub(super) fn starter_manifest(class: ClassId) -> anyhow::Result<StarterKitManifest> {
    static KITS: OnceLock<Result<BTreeMap<ClassId, StarterKitManifest>, String>> = OnceLock::new();
    let kits = KITS
        .get_or_init(|| load().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    kits.get(&class)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing pinned starter class {}", class.0))
}

fn load() -> anyhow::Result<BTreeMap<ClassId, StarterKitManifest>> {
    let kits: Vec<StarterKitManifest> =
        serde_json::from_str(include_str!("starter-kit-manifests-v1.json"))?;
    let mut result = BTreeMap::new();
    for kit in kits {
        kit.validate()?;
        anyhow::ensure!(
            result.insert(kit.source_class_id, kit).is_none(),
            "duplicate pinned starter class"
        );
    }
    anyhow::ensure!(result.len() == 9, "incomplete playable starter manifests");
    Ok(result)
}

/// Creation writes only this durable input; no item ID/ownership is allocated in an API RPC.
pub(super) fn starter_request(character: &Character) -> anyhow::Result<PendingItemRequest> {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let kit = starter_manifest(character.class_state.base_class_id)?;
    let manifest_hash = kit.hash()?;
    let mut request = Sha256::new();
    request.update(b"nightfall.phase4.starter.1\0");
    request.update(character.id.as_uuid().as_bytes());
    let digest = request.finalize();
    let mut identity = [0; 16];
    for (output, input) in identity.iter_mut().zip(digest) {
        *output = input;
    }
    let mut hash = String::with_capacity(64);
    for byte in manifest_hash {
        write!(hash, "{byte:02x}")?;
    }
    Ok(PendingItemRequest {
        account_id: character.account_id,
        operation: PendingItemKind::Starter.operation(),
        request_id: uuid::Uuid::from_bytes(identity),
        character_id: character.id,
        kind: PendingItemKind::Starter,
        fingerprint: format!("v1|starter|{}|{hash}", character.id),
        manifest_version: 1,
        manifest_hash,
        manifest: PendingItemManifest::Kit(kit),
        status: PendingItemStatus::Pending,
    })
}

/// Freeze all pending contents under the same parent lock as progression and its revision.
pub(super) fn admission_load(
    character: &Character,
    checkpoint: Option<crate::domain::zone::Phase4Checkpoint>,
    requests: &[PendingItemRequest],
    phase3_sources: usize,
    rules: &crate::domain::zone::Phase4Rules,
) -> anyhow::Result<crate::domain::zone::Phase4PlayerLoad> {
    use crate::domain::zone::phase4_admission::{
        ItemKitManifest, ItemMaterialization, PendingItemMaterialization, StarterKitSeed,
    };
    use crate::domain::zone::Phase4PlayerLoad;
    if let Some(cp) = &checkpoint {
        anyhow::ensure!(cp.rules_hash == rules.hash(), "retained inventory rules hash mismatch");
    }
    let mut pending = Vec::new();
    for request in requests {
        anyhow::ensure!(
            request.character_id == character.id
                && request.account_id == character.account_id
                && request.operation == request.kind.operation()
                && request.manifest_version == 1
                && !request.fingerprint.is_empty(),
            "invalid pending item identity"
        );
        let materialization = match (&request.kind, &request.manifest) {
            (
                kind @ (PendingItemKind::Starter | PendingItemKind::Backfill),
                PendingItemManifest::Kit(kit),
            ) => {
                anyhow::ensure!(
                    *kit == starter_manifest(character.class_state.base_class_id)?,
                    "pending kit does not match creation class"
                );
                let kit = ItemKitManifest {
                    source_class_id: kit.source_class_id,
                    items: kit
                        .items
                        .iter()
                        .map(|item| StarterKitSeed {
                            template_id: item.template_id,
                            count: item.count,
                            equipped: item.equipped,
                        })
                        .collect(),
                };
                if *kind == PendingItemKind::Starter {
                    ItemMaterialization::Starter(kit)
                } else {
                    ItemMaterialization::Backfill(kit)
                }
            },
        };
        anyhow::ensure!(
            materialization.manifest_hash()? == request.manifest_hash,
            "pending item content hash mismatch"
        );
        match request.status {
            PendingItemStatus::Pending => pending.push(PendingItemMaterialization {
                request_id: request.request_id,
                request_fingerprint: request.fingerprint.clone(),
                manifest_version: request.manifest_version,
                manifest_hash: request.manifest_hash,
                materialization,
            }),
            PendingItemStatus::Materialized { revision, receipt } => {
                let cp = checkpoint.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("completed request without inventory checkpoint")
                })?;
                anyhow::ensure!(
                    receipt == request.request_id && revision <= cp.inventory_revision,
                    "pending item completion mismatch"
                );
                {
                    let kit = cp
                        .starter_kit_receipt
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("completed kit without retained receipt"))?;
                    anyhow::ensure!(
                        kit.request_id == receipt
                            && kit.materialized_revision == revision
                            && kit.manifest_version == request.manifest_version
                            && kit.manifest_hash == request.manifest_hash
                            && kit.source_class == character.class_state.base_class_id,
                        "completed kit receipt mismatch"
                    );
                }
            },
        }
    }
    pending.sort_by_key(|request| request.request_id);
    let load = Phase4PlayerLoad {
        checkpoint,
        pending,
    };
    load.validate_structure(phase3_sources)?;
    Ok(load)
}
