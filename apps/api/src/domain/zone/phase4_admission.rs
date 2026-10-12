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

/// Normalized owner item intent. Every scalar is client input until zone preflight has
/// checked ownership, capability and state; no client supplies costs, stats or RNG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum ItemCommandKind {
    Equip {
        instance: super::ItemInstanceId,
        preferred_slot: Option<super::EquipSlot>,
    },
    Unequip {
        slot: super::EquipSlot,
    },
    Use {
        instance: super::ItemInstanceId,
        target: Option<super::effects::TargetBinding>,
    },
    Destroy {
        instance: super::ItemInstanceId,
        count: u64,
    },
    Enchant {
        scroll: super::ItemInstanceId,
        target: super::ItemInstanceId,
        support: Option<super::ItemInstanceId>,
    },
    ToggleAutoShot {
        template: u32,
        enabled: bool,
    },
    Refine {
        npc: super::effects::TargetBinding,
        target: super::ItemInstanceId,
        stone: super::ItemInstanceId,
    },
    CancelRefine {
        npc: super::effects::TargetBinding,
        target: super::ItemInstanceId,
    },
    ExchangeSpecialAbility {
        npc: super::effects::TargetBinding,
        target: super::ItemInstanceId,
        recipe: u64,
    },
    Crystallize {
        instance: super::ItemInstanceId,
    },
}

/// Actor preflight supplies account identity after checking the session generation. The
/// wire decoder creates an unauthenticated intent; only an enriched command is recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemCommand {
    pub entity: super::EntityId,
    pub request_id: uuid::Uuid,
    pub kind: ItemCommandKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account: Option<crate::domain::AccountId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    known: Option<RecordedItemReceipt>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedItemReceipt {
    fingerprint: String,
    payload: super::phase4_projection::ItemResultPayload,
}

impl ItemCommand {
    /// Transport-independent shape validation. Gameplay checks still run in the zone.
    pub fn new(
        entity: super::EntityId,
        request_id: uuid::Uuid,
        kind: ItemCommandKind,
    ) -> Result<Self, ItemError> {
        let command = Self {
            entity,
            request_id,
            kind,
            account: None,
            known: None,
        };
        command.validate_shape()?;
        Ok(command)
    }

    /// Enrich once with the authenticated owner; retries may only retain the same account.
    pub fn with_account(mut self, account: crate::domain::AccountId) -> Result<Self, ItemError> {
        self.validate_shape()?;
        if account.as_uuid().is_nil() || self.account.is_some_and(|old| old != account) {
            return Err(ItemError("item command account mismatch"));
        }
        self.account = Some(account);
        Ok(self)
    }

    /// Durable command validation requires authenticated context, unlike WS decoding.
    pub fn authenticated_account(&self) -> Result<crate::domain::AccountId, ItemError> {
        self.validate_shape()?;
        self.account
            .ok_or(ItemError("item command lacks authenticated account"))
    }

    /// Hash the complete normalized body and owner, excluding receipt key/transport seq.
    /// Account scope belongs to the receipt repository and is retained in the envelope.
    pub fn fingerprint(&self) -> Result<[u8; 32], ItemError> {
        self.authenticated_account()?;
        self.body_fingerprint()
    }
    fn body_fingerprint(&self) -> Result<[u8; 32], ItemError> {
        let value = serde_json::to_value((self.entity, &self.kind))
            .map_err(|_| ItemError("item command encoding failed"))?;
        let bytes = serde_json::to_vec(&sort_manifest(value))
            .map_err(|_| ItemError("item command encoding failed"))?;
        let mut hash = Sha256::new();
        hash.update(b"nightfall.item.request.1\0");
        hash.update(bytes);
        Ok(hash.finalize().into())
    }

    /// Repository fingerprint representation; identical for fresh and Known preflight.
    pub fn fingerprint_hex(&self) -> Result<String, ItemError> {
        self.authenticated_account()?;
        self.body_fingerprint_hex()
    }
    fn body_fingerprint_hex(&self) -> Result<String, ItemError> {
        use std::fmt::Write as _;
        let mut value = String::with_capacity(64);
        for byte in self.body_fingerprint()? {
            write!(&mut value, "{byte:02x}")
                .map_err(|_| ItemError("item fingerprint encoding failed"))?;
        }
        Ok(value)
    }
    /// Actor-only enrichment after an authenticated scoped receipt lookup. The stored
    /// result is bound to the normalized body/key/owner before it can enter a replay record.
    pub fn with_known_receipt(
        mut self,
        fingerprint: String,
        payload: super::phase4_projection::ItemResultPayload,
    ) -> Result<Self, ItemError> {
        self.authenticated_account()?;
        let known = RecordedItemReceipt {
            fingerprint,
            payload,
        };
        if self.known.as_ref().is_some_and(|old| old != &known) {
            return Err(ItemError("item preflight result changed"));
        }
        self.known = Some(known);
        self.validate_shape()?;
        Ok(self)
    }
    /// Known receipts bypass current gameplay eligibility; session/account checks remain.
    pub const fn has_known_receipt(&self) -> bool {
        self.known.is_some()
    }
    /// No repository call or mutation is needed to recover the recorded result.
    pub fn known_result(&self) -> Result<Option<super::FrozenItemResult>, ItemError> {
        self.validate_shape()?;
        self.known
            .as_ref()
            .map(|known| known.payload.result())
            .transpose()
    }
    /// Logical receipt scope, independent of automatic ledger changes.
    pub const fn operation(&self) -> super::phase4_projection::ItemOperation {
        use super::phase4_projection::ItemOperation as O;
        match self.kind {
            ItemCommandKind::Equip { .. } => O::Equip,
            ItemCommandKind::Unequip { .. } => O::Unequip,
            ItemCommandKind::Use { .. } => O::Use,
            ItemCommandKind::Destroy { .. } => O::Destroy,
            ItemCommandKind::Enchant { .. } => O::Enchant,
            ItemCommandKind::ToggleAutoShot { .. } => O::ToggleAutoShot,
            ItemCommandKind::Refine { .. } => O::Refine,
            ItemCommandKind::CancelRefine { .. } => O::CancelRefine,
            ItemCommandKind::ExchangeSpecialAbility { .. } => O::Exchange,
            ItemCommandKind::Crystallize { .. } => O::Crystallize,
        }
    }

    /// Also called after typed replay decoding, so derived serde cannot bypass shape guards.
    pub fn validate_shape(&self) -> Result<(), ItemError> {
        if self.request_id.is_nil()
            || self.entity.as_uuid().is_nil()
            || self
                .account
                .is_some_and(|account| account.as_uuid().is_nil())
        {
            return Err(ItemError("empty item request identity"));
        }
        let binding = match &self.kind {
            ItemCommandKind::Use { target, .. } => *target,
            ItemCommandKind::Refine { npc, .. }
            | ItemCommandKind::CancelRefine { npc, .. }
            | ItemCommandKind::ExchangeSpecialAbility { npc, .. } => Some(*npc),
            ItemCommandKind::Equip { .. }
            | ItemCommandKind::Unequip { .. }
            | ItemCommandKind::Destroy { .. }
            | ItemCommandKind::Enchant { .. }
            | ItemCommandKind::ToggleAutoShot { .. }
            | ItemCommandKind::Crystallize { .. } => None,
        };
        if binding
            .is_some_and(|binding| binding.entity.as_uuid().is_nil() || binding.incarnation == 0)
        {
            return Err(ItemError("invalid item target binding"));
        }
        match &self.kind {
            ItemCommandKind::Destroy { count, .. }
                if *count == 0 || i64::try_from(*count).is_err() =>
            {
                Err(ItemError("item count outside positive bigint"))
            },
            ItemCommandKind::ToggleAutoShot { template, .. }
                if *template == 0 || i32::try_from(*template).is_err() =>
            {
                Err(ItemError("invalid item template"))
            },
            ItemCommandKind::ExchangeSpecialAbility { recipe, .. }
                if *recipe == 0 || i64::try_from(*recipe).is_err() =>
            {
                Err(ItemError("invalid item recipe"))
            },
            ItemCommandKind::Enchant {
                scroll,
                target,
                support,
            } if scroll == target || support.is_some_and(|id| id == *scroll || id == *target) => {
                Err(ItemError("item roles must be distinct"))
            },
            ItemCommandKind::Refine { target, stone, .. } if target == stone => {
                Err(ItemError("item roles must be distinct"))
            },
            ItemCommandKind::Equip { .. }
            | ItemCommandKind::Unequip { .. }
            | ItemCommandKind::Use { .. }
            | ItemCommandKind::Destroy { .. }
            | ItemCommandKind::Enchant { .. }
            | ItemCommandKind::ToggleAutoShot { .. }
            | ItemCommandKind::Refine { .. }
            | ItemCommandKind::CancelRefine { .. }
            | ItemCommandKind::ExchangeSpecialAbility { .. }
            | ItemCommandKind::Crystallize { .. } => Ok(()),
        }?;
        if let Some(known) = &self.known {
            let result = known.payload.result()?;
            if self.account.is_none()
                || known.fingerprint != self.body_fingerprint_hex()?
                || result.entity != self.entity
                || result.receipt != self.request_id
                || result.operation() != self.operation()
                || result.incarnation == 0
            {
                return Err(ItemError("item preflight result scope mismatch"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod command_tests {
    use super::super::ItemInstanceId;
    use super::*;
    use crate::domain::AccountId;

    #[test]
    fn known_receipt_is_actor_only_and_bound_to_body_owner_key_and_operation() {
        use super::super::phase4_projection::{ItemOperation, ItemResultDetail};
        let fresh = command(1, 22)
            .with_account(AccountId::from_uuid(uuid::Uuid::from_u128(3)))
            .unwrap();
        let mut result = super::super::phase4_projection::tests::projection()
            .receipts
            .remove(0)
            .result;
        result.entity = fresh.entity;
        result.receipt = fresh.request_id;
        result.detail = ItemResultDetail::Operation {
            operation: ItemOperation::Destroy,
            affected: vec![ItemInstanceId::new(7).unwrap()],
            recipe: None,
        };
        let payload = result.payload().unwrap();
        let fingerprint = fresh.fingerprint_hex().unwrap();
        let known = fresh
            .clone()
            .with_known_receipt(fingerprint.clone(), payload.clone())
            .unwrap();
        assert_eq!(known.fingerprint().unwrap(), fresh.fingerprint().unwrap());
        assert!(known.has_known_receipt());
        assert_eq!(known.known_result().unwrap(), Some(result.clone()));
        let bytes = serde_json::to_vec(&known).unwrap();
        let decoded: ItemCommand = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, known);
        decoded.authenticated_account().unwrap();
        assert!(command(1, 22)
            .with_known_receipt(fingerprint.clone(), payload.clone())
            .is_err());
        assert!(fresh
            .clone()
            .with_known_receipt("wrong".into(), payload.clone())
            .is_err());
        let wrong_body = command(2, 22)
            .with_account(AccountId::from_uuid(uuid::Uuid::from_u128(3)))
            .unwrap();
        assert!(wrong_body
            .with_known_receipt(fingerprint.clone(), payload)
            .is_err());
        for field in ["owner", "key", "operation", "incarnation"] {
            let mut wrong = result.clone();
            match field {
                "owner" => {
                    wrong.entity = super::super::EntityId::from_uuid(uuid::Uuid::from_u128(99));
                },
                "key" => wrong.receipt = uuid::Uuid::from_u128(99),
                "operation" => {
                    wrong.detail = ItemResultDetail::Operation {
                        operation: ItemOperation::Equip,
                        affected: Vec::new(),
                        recipe: None,
                    }
                },
                _ => wrong.incarnation = 0,
            }
            assert!(fresh
                .clone()
                .with_known_receipt(fingerprint.clone(), wrong.payload().unwrap())
                .is_err());
        }
        let mut corrupt = known.clone();
        corrupt.kind = ItemCommandKind::Destroy {
            instance: ItemInstanceId::new(7).unwrap(),
            count: 2,
        };
        assert!(corrupt.validate_shape().is_err());
        result.inventory_revision = result.inventory_revision.checked_add(1).unwrap();
        assert!(known
            .with_known_receipt(fingerprint, result.payload().unwrap())
            .is_err());
    }

    fn command(count: u64, key: u128) -> ItemCommand {
        ItemCommand::new(
            super::super::EntityId::from_uuid(uuid::Uuid::from_u128(1)),
            uuid::Uuid::from_u128(key),
            ItemCommandKind::Destroy {
                instance: ItemInstanceId::new(7).unwrap(),
                count,
            },
        )
        .unwrap()
    }
    #[test]
    fn account_is_injected_once_and_request_keys_do_not_change_body_fingerprint() {
        let account = AccountId::from_uuid(uuid::Uuid::from_u128(2));
        let input = command(2, 3);
        assert!(input.authenticated_account().is_err());
        assert!(input.fingerprint().is_err());
        assert!(input
            .clone()
            .with_account(AccountId::from_uuid(uuid::Uuid::nil()))
            .is_err());
        let authenticated = input.with_account(account).unwrap();
        let retry = command(2, 4).with_account(account).unwrap();
        assert_eq!(authenticated.fingerprint().unwrap(), retry.fingerprint().unwrap());
        assert_ne!(
            authenticated.fingerprint().unwrap(),
            command(3, 3)
                .with_account(account)
                .unwrap()
                .fingerprint()
                .unwrap()
        );
        assert_eq!(authenticated.clone().with_account(account).unwrap(), authenticated);
        assert!(authenticated
            .clone()
            .with_account(AccountId::from_uuid(uuid::Uuid::from_u128(9)))
            .is_err());
        let decoded: ItemCommand =
            serde_json::from_slice(&serde_json::to_vec(&authenticated).unwrap()).unwrap();
        assert_eq!(decoded, authenticated);
        assert_eq!(decoded.authenticated_account().unwrap(), account);
    }
    #[test]
    fn decoded_invalid_roles_counts_and_keys_are_rejected_before_recording() {
        let instance = ItemInstanceId::new(7).unwrap();
        let cases = [
            ItemCommandKind::Use {
                instance,
                target: Some(super::super::effects::TargetBinding {
                    entity: super::super::EntityId::from_uuid(uuid::Uuid::from_u128(8)),
                    incarnation: 0,
                }),
            },
            ItemCommandKind::Destroy { instance, count: 0 },
            ItemCommandKind::Destroy {
                instance,
                count: u64::MAX,
            },
            ItemCommandKind::Enchant {
                scroll: instance,
                target: instance,
                support: None,
            },
            ItemCommandKind::ToggleAutoShot {
                template: 0,
                enabled: true,
            },
            ItemCommandKind::ExchangeSpecialAbility {
                npc: super::super::effects::TargetBinding {
                    entity: super::super::EntityId::from_uuid(uuid::Uuid::from_u128(8)),
                    incarnation: 1,
                },
                target: instance,
                recipe: 0,
            },
        ];
        for kind in cases {
            let mut invalid = command(1, 3);
            invalid.kind = kind;
            let decoded: ItemCommand =
                serde_json::from_value(serde_json::to_value(&invalid).unwrap()).unwrap();
            assert!(decoded.validate_shape().is_err());
        }
        let mut invalid = command(1, 3);
        invalid.request_id = uuid::Uuid::nil();
        assert!(invalid.validate_shape().is_err());
    }
}

/// Server-only out-of-world materialization input (D30). The repository supplies a full
/// fenced load; the zone settles privately without creating a session or public entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct MaterializePendingItems {
    pub account: crate::domain::AccountId,
    pub character: crate::domain::CharacterId,
    pub position: super::Vec2Fixed,
    pub load: Box<super::PlayerLoad>,
    pub items: Phase4PlayerLoad,
}
impl MaterializePendingItems {
    /// Preflight requires the authoritative parent revision and both skill/item payloads.
    /// Runtime additionally checks rule hashes, lineage, bounds, inventory and source fold.
    pub fn validate_structure(&self) -> Result<(), ItemError> {
        if self
            .load
            .checkpoint_revision
            .is_none_or(|revision| i64::try_from(revision).is_err())
            || self
                .load
                .progression
                .as_ref()
                .is_none_or(|p| p.identity.account_id != self.account)
            || self.items.pending.is_empty()
        {
            return Err(ItemError("unfenced private materialization load"));
        }
        let phase3 = self
            .load
            .phase3
            .as_ref()
            .ok_or(ItemError("private materialization requires skills"))?;
        self.items
            .validate_structure(phase3.modifier_install_order.len())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod materialization_tests {
    use super::*;
    #[test]
    fn private_input_requires_owner_revision_skills_and_pending_contents() {
        let account = crate::domain::AccountId::from_uuid(uuid::Uuid::from_u128(1));
        let materialization = ItemMaterialization::Starter(ItemKitManifest {
            source_class_id: ClassId(0),
            items: vec![StarterKitSeed {
                template_id: 1,
                count: 1,
                equipped: true,
            }],
        });
        let mut load = super::super::PlayerLoad::fresh("human_fighter");
        load.phase3 = Some(super::super::Phase3Checkpoint::new());
        load.checkpoint_revision = Some(1);
        load.progression = Some(Box::new(super::super::PlayerProgression {
            identity: crate::domain::character_progression::CharacterIdentity {
                account_id: account,
                race: crate::domain::Race::Human,
                base_class_id: ClassId(0),
                appearance: crate::domain::character_progression::CharacterAppearance::default(),
            },
            class_state: crate::domain::character_progression::ClassState::new(ClassId(0)),
            max_cp: 1,
        }));
        let command = MaterializePendingItems {
            account,
            character: crate::domain::CharacterId::from_uuid(uuid::Uuid::from_u128(2)),
            position: super::super::Vec2Fixed::from_tiles(128, 128),
            load: Box::new(load),
            items: Phase4PlayerLoad {
                checkpoint: None,
                pending: vec![PendingItemMaterialization {
                    request_id: uuid::Uuid::from_u128(3),
                    request_fingerprint: "source-pending-body".into(),
                    manifest_version: 1,
                    manifest_hash: materialization.manifest_hash().unwrap(),
                    materialization,
                }],
            },
        };
        command.validate_structure().unwrap();
        let mut wrong = command.clone();
        wrong.account = crate::domain::AccountId::from_uuid(uuid::Uuid::from_u128(9));
        assert!(wrong.validate_structure().is_err());
        let mut wrong = command.clone();
        wrong.load.checkpoint_revision = None;
        assert!(wrong.validate_structure().is_err());
        let mut wrong = command.clone();
        wrong.load.phase3 = None;
        assert!(wrong.validate_structure().is_err());
        let mut wrong = command;
        wrong.items.pending.clear();
        assert!(wrong.validate_structure().is_err());
    }
}
