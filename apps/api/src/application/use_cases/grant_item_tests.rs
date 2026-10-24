//! Unit routing contracts; a fake actor port does not prove live materialization.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::*;
use crate::application::ports::*;
use crate::domain::zone::phase4_checkpoint::StarterKitReceipt;
use crate::domain::zone::phase4_projection::{FrozenItemResult, ItemOperation, ItemResultDetail};
use crate::domain::zone::{
    EntityId, ItemInstance, ItemInstanceId, ItemLocation, Phase4Checkpoint, Tick,
};
use crate::domain::{Character, CharacterName, Race};
use crate::infrastructure::{
    item_data::embedded_phase4_bundle, memory::InMemoryCharacterRepository,
};
use parking_lot::Mutex;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Clone, Copy)]
enum Behavior {
    Success,
    Full,
    Deferred,
    Stopped,
    Conflict,
    BadReply,
    Pending,
    Infrastructure,
}
struct Actor {
    available: AtomicBool,
    calls: AtomicUsize,
    behavior: Mutex<Behavior>,
    receipts: Mutex<BTreeMap<(AccountId, IdempotencyKey), ItemMutationReceipt>>,
    lookup_failure: AtomicBool,
}
impl Default for Actor {
    fn default() -> Self {
        Self {
            available: AtomicBool::new(true),
            calls: AtomicUsize::new(0),
            behavior: Mutex::new(Behavior::Success),
            receipts: Mutex::default(),
            lookup_failure: AtomicBool::new(false),
        }
    }
}
#[async_trait::async_trait]
impl ItemReceiptRepository for Actor {
    async fn item_receipt_lookup(
        &self,
        account: AccountId,
        operation: ItemMutationOperation,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<ItemReceiptLookup> {
        anyhow::ensure!(!self.lookup_failure.load(Ordering::SeqCst), "lookup failure");
        assert_eq!(operation, ItemMutationOperation::GrantItem);
        Ok(match self.receipts.lock().get(&(account, *key)) {
            None => ItemReceiptLookup::Unknown,
            Some(receipt) if receipt.fingerprint != fingerprint => ItemReceiptLookup::Conflict {
                stored_fingerprint: receipt.fingerprint.clone(),
            },
            Some(receipt) => ItemReceiptLookup::Known(Box::new(receipt.clone())),
        })
    }
}
#[async_trait::async_trait]
impl ItemGrantRuntime for Actor {
    async fn available(&self) -> Result<(), AppError> {
        if self.available.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(AppError::Unavailable("stopped".into()))
        }
    }
    async fn enqueue_grant_item(
        &self,
        account: AccountId,
        character: CharacterId,
        request: ItemGrantRequest,
    ) -> Result<ItemMutationReceipt, AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let behavior = *self.behavior.lock();
        match behavior {
            Behavior::Full => return Err(AppError::ResourceExhausted("full".into())),
            Behavior::Deferred => {
                return Err(AppError::FailedPrecondition("lifecycle conflict".into()))
            },
            Behavior::Stopped => return Err(AppError::Unavailable("stopped".into())),
            Behavior::Conflict => return Err(AppError::IdempotencyConflict),
            Behavior::Infrastructure => return Err(anyhow::anyhow!("actor port failure").into()),
            Behavior::Pending => return std::future::pending().await,
            Behavior::Success | Behavior::BadReply => {},
        }
        let mut receipts = self.receipts.lock();
        if let Some(receipt) = receipts.get(&(account, request.key)) {
            return if receipt.fingerprint == request.fingerprint {
                Ok(receipt.clone())
            } else {
                Err(AppError::IdempotencyConflict)
            };
        }
        let mut receipt = frozen(account, character, &request);
        receipts.insert((account, request.key), receipt.clone());
        if matches!(behavior, Behavior::BadReply) {
            receipt.fingerprint = "corrupt".into();
        }
        Ok(receipt)
    }
}
fn frozen(
    account: AccountId,
    character: CharacterId,
    request: &ItemGrantRequest,
) -> ItemMutationReceipt {
    let id = ItemInstanceId::new(1).unwrap();
    ItemMutationReceipt {
        account_id: account,
        character_id: character,
        operation: ItemMutationOperation::GrantItem,
        key: request.key.as_uuid(),
        fingerprint: request.fingerprint.clone(),
        result: FrozenItemResult {
            accepted_sequence: None,
            request_fingerprint: request.fingerprint.clone(),
            entity: EntityId::from_uuid(character.as_uuid()),
            incarnation: 0,
            tick: Tick(71),
            inventory_revision: 2,
            receipt: request.key.as_uuid(),
            detail: ItemResultDetail::Operation {
                operation: ItemOperation::Grant,
                affected: vec![id],
                recipe: None,
            },
        },
        inventory: Some(FrozenItemInventory {
            character_revision: 4,
            phase3_source_count: 0,
            ordinary_capacity: 83,
            quest_capacity: 101,
            checkpoint: Phase4Checkpoint {
                schema_version: 1,
                timer_policy_version: 1,
                rules_hash: request.rules_hash,
                inventory_revision: 3,
                instances: vec![ItemInstance {
                    creation_origin: None,
                    id,
                    template_id: request.template_id,
                    count: request.count,
                    location: ItemLocation::Inventory,
                    slot: None,
                    enchant: u32::MAX,
                    augment_id: Some(u32::MAX),
                    charges: 0,
                    created_tick: Tick(1),
                    revision: 2,
                }],
                paperdoll: BTreeMap::new(),
                auto_shots: BTreeSet::default(),
                item_reuse_remainders: vec![],
                equipment_install_order: vec![],
                active_sets: BTreeMap::new(),
                weight: 1,
                max_load: 100_000,
                ordinary_capacity: 83,
                quest_capacity: 101,
                weight_penalty_bonus: 1,
                overload_level: 0,
                overloaded: false,
                expertise: 0,
                weapon_penalty: 0,
                armor_penalty: 0,
                starter_kit_receipt: Some(StarterKitReceipt {
                    manifest_version: 1,
                    manifest_hash: [1; 32],
                    source_class: crate::domain::class::ClassId(0),
                    request_id: uuid::Uuid::from_u128(7),
                    materialized_revision: 1,
                }),
            },
        }),
    }
}
struct Fixture {
    use_case: GrantItem,
    actor: Arc<Actor>,
    repo: Arc<InMemoryCharacterRepository>,
    input: GrantItemInput,
}
async fn fixture() -> Fixture {
    let repo = Arc::new(InMemoryCharacterRepository::default());
    let character = Character::create(
        AccountId::from_uuid(uuid::Uuid::from_u128(1)),
        CharacterName::new("GrantUnit").unwrap(),
        Race::Human,
    );
    repo.create_idempotent(&IdempotencyKey::new(), "unit", &character)
        .await
        .unwrap();
    let bundle = embedded_phase4_bundle().unwrap();
    let catalogue = Arc::new(
        crate::application::use_cases::RegistryItemCatalogue::new(bundle.items, &bundle.skills)
            .unwrap(),
    );
    let actor = Arc::new(Actor::default());
    let use_case = GrantItem::new(
        true,
        repo.clone(),
        Some(actor.clone()),
        Some(catalogue),
        Some(actor.clone()),
    );
    Fixture {
        use_case,
        actor,
        repo,
        input: GrantItemInput {
            caller: character.account_id,
            character_id: character.id.as_uuid().as_bytes().to_vec(),
            template_id: 10,
            count: 2,
            idempotency_key: IdempotencyKey::new().to_string(),
            enchant_level: None,
        },
    }
}
#[tokio::test]
async fn frozen_success_replays_before_changed_eligibility_and_body_conflicts() {
    let f = fixture().await;
    let before = f.repo.staged_events();
    let first = f.use_case.execute(f.input.clone()).await.unwrap();
    assert_eq!(first.inventory.as_ref().unwrap().ordinary_capacity, 83);
    *f.actor.behavior.lock() = Behavior::Full;
    assert_eq!(f.use_case.execute(f.input.clone()).await.unwrap(), first);
    let mut changed = f.input.clone();
    changed.count = 3;
    assert!(matches!(f.use_case.execute(changed).await, Err(AppError::IdempotencyConflict)));
    let mut changed = f.input.clone();
    changed.character_id = uuid::Uuid::from_u128(9).as_bytes().to_vec();
    assert!(matches!(f.use_case.execute(changed).await, Err(AppError::IdempotencyConflict)));
    assert_eq!(f.actor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.repo.staged_events(), before);
}
#[tokio::test]
async fn invalid_owner_missing_character_and_storage_inputs_have_no_actor_write() {
    let f = fixture().await;
    let before = f.repo.staged_events();
    for (id, template, count, key) in [
        (vec![0; 15], 10, 1, f.input.idempotency_key.clone()),
        (f.input.character_id.clone(), 0, 1, f.input.idempotency_key.clone()),
        (f.input.character_id.clone(), u32::MAX, 1, f.input.idempotency_key.clone()),
        (f.input.character_id.clone(), 10, 0, f.input.idempotency_key.clone()),
        (f.input.character_id.clone(), 10, u64::MAX, f.input.idempotency_key.clone()),
        (f.input.character_id.clone(), 10, 1, "bad".into()),
        (f.input.character_id.clone(), 10, 1, uuid::Uuid::nil().to_string()),
        (f.input.character_id.clone(), 2_000_000_000, 1, f.input.idempotency_key.clone()),
    ] {
        let mut input = f.input.clone();
        input.character_id = id;
        input.template_id = template;
        input.count = count;
        input.idempotency_key = key;
        assert!(matches!(f.use_case.execute(input).await, Err(AppError::InvalidArgument(_))));
    }
    let mut input = f.input.clone();
    input.caller = AccountId::from_uuid(uuid::Uuid::from_u128(8));
    assert!(matches!(f.use_case.execute(input).await, Err(AppError::PermissionDenied(_))));
    let mut input = f.input.clone();
    input.character_id = uuid::Uuid::from_u128(9).as_bytes().to_vec();
    assert!(matches!(f.use_case.execute(input).await, Err(AppError::NotFound { .. })));
    let rules = f.use_case.catalogue.as_ref().unwrap().catalogue().unwrap();
    let denied = rules
        .parts()
        .capabilities
        .values()
        .find(|capability| !capability.storable)
        .unwrap()
        .template_id;
    let mut input = f.input.clone();
    input.template_id = denied;
    assert!(matches!(f.use_case.execute(input).await, Err(AppError::FailedPrecondition(_))));
    assert_eq!(f.actor.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.repo.staged_events(), before);
}
#[tokio::test]
async fn gate_port_failure_actor_statuses_and_corrupt_reply_fail_closed() {
    let mut f = fixture().await;
    f.use_case.enabled = false;
    assert!(matches!(
        f.use_case.execute(f.input.clone()).await,
        Err(AppError::FailedPrecondition(_))
    ));
    f.use_case.enabled = true;
    f.actor.available.store(false, Ordering::SeqCst);
    assert!(matches!(
        f.use_case.execute(f.input.clone()).await,
        Err(AppError::Unavailable(_))
    ));
    f.actor.available.store(true, Ordering::SeqCst);
    f.actor.lookup_failure.store(true, Ordering::SeqCst);
    assert!(matches!(
        f.use_case.execute(f.input.clone()).await,
        Err(AppError::Infrastructure(_))
    ));
    f.actor.lookup_failure.store(false, Ordering::SeqCst);
    for behavior in [
        Behavior::Full,
        Behavior::Deferred,
        Behavior::Stopped,
        Behavior::Conflict,
        Behavior::Infrastructure,
        Behavior::BadReply,
    ] {
        *f.actor.behavior.lock() = behavior;
        let result = f.use_case.execute(f.input.clone()).await;
        match behavior {
            Behavior::Full => assert!(matches!(result, Err(AppError::ResourceExhausted(_)))),
            Behavior::Deferred => assert!(matches!(result, Err(AppError::FailedPrecondition(_)))),
            Behavior::Stopped => assert!(matches!(result, Err(AppError::Unavailable(_)))),
            Behavior::Conflict => assert!(matches!(result, Err(AppError::IdempotencyConflict))),
            Behavior::Infrastructure | Behavior::BadReply => {
                assert!(matches!(result, Err(AppError::Infrastructure(_))));
            },
            Behavior::Success | Behavior::Pending => {},
        }
    }
    f.use_case.runtime = None;
    assert!(matches!(
        f.use_case.execute(f.input.clone()).await,
        Err(AppError::Unavailable(_))
    ));
    f.use_case.runtime = Some(f.actor.clone());
    f.use_case.receipts = None;
    assert!(matches!(
        f.use_case.execute(f.input.clone()).await,
        Err(AppError::Unavailable(_))
    ));
    f.use_case.receipts = Some(f.actor.clone());
    f.use_case.catalogue = None;
    assert!(matches!(f.use_case.execute(f.input).await, Err(AppError::Unavailable(_))));
}
#[tokio::test]
async fn bounded_ack_timeout_returns_unavailable_without_a_frozen_success() {
    let f = fixture().await;
    *f.actor.behavior.lock() = Behavior::Pending;
    assert!(matches!(f.use_case.execute(f.input).await, Err(AppError::Unavailable(_))));
    assert!(f.actor.receipts.lock().is_empty());
}
