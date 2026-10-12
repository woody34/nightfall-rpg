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
