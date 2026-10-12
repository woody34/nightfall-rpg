//! The dev gate is authenticated and precedes every durable input or actor mutation.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]
mod common;
use common::TestApp;
use nightfall_api::interface::grpc::pb;
use tonic::Code;

#[tokio::test]
async fn grant_requires_explicit_dev_gate_for_every_authenticated_account() {
    let app = TestApp::spawn().await;
    let before = app.characters.staged_events();
    let request = pb::GrantItemRequest::default();
    for mut client in [app.anon_grpc(), app.game_with_token("invalid")] {
        assert_eq!(
            client.grant_item(request.clone()).await.unwrap_err().code(),
            Code::Unauthenticated
        );
    }
    for account in [common::account(), uuid::Uuid::from_u128(77)] {
        let status = app
            .game_as(account)
            .grant_item(request.clone())
            .await
            .unwrap_err();
        assert_eq!(status.code(), Code::FailedPrecondition);
        assert!(status.message().contains("AUTH_DEV_TOKENS=1"));
    }
    assert_eq!(app.characters.staged_events(), before);
    assert!(app.bus.published().is_empty());
    assert_eq!(app.characters.len(), 0);
}

#[tokio::test]
async fn enabled_dev_gate_requires_the_item_runtime_and_authentication() {
    let app = TestApp::spawn_with(|deps| deps.dev_item_grants = true).await;
    assert_eq!(
        app.anon_grpc()
            .grant_item(pb::GrantItemRequest::default())
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    for _ in 0..2 {
        assert_eq!(
            app.game_as(common::account())
                .grant_item(pb::GrantItemRequest::default())
                .await
                .unwrap_err()
                .code(),
            Code::Unavailable
        );
    }
    assert_eq!(app.characters.len(), 0);
    assert!(app.characters.staged_events().is_empty());
    assert!(app.bus.published().is_empty());
}

use nightfall_api::application::{ports::*, AppError, CharacterRepository, IdempotencyKey};
use nightfall_api::domain::zone::phase4_checkpoint::StarterKitReceipt;
use nightfall_api::domain::zone::phase4_projection::{
    FrozenItemResult, ItemOperation, ItemResultDetail,
};
use nightfall_api::domain::zone::{
    EntityId, ItemInstance, ItemInstanceId, ItemLocation, Phase4Checkpoint, Phase4Rules, Tick,
};
use nightfall_api::domain::{AccountId, Character, CharacterId, CharacterName, Race};
use parking_lot::Mutex;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, OnceLock,
    },
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
            Some(receipt) if receipt.fingerprint != fingerprint => ItemReceiptLookup::Conflict,
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
                    source_class: nightfall_api::domain::class::ClassId(0),
                    request_id: uuid::Uuid::from_u128(7),
                    materialized_revision: 1,
                }),
            },
        }),
    }
}

struct Catalogue(Arc<Phase4Rules>);
impl ItemCataloguePort for Catalogue {
    fn catalogue(&self) -> Result<Arc<Phase4Rules>, AppError> {
        Ok(self.0.clone())
    }
}
fn rules() -> Arc<Phase4Rules> {
    static RULES: OnceLock<Arc<Phase4Rules>> = OnceLock::new();
    RULES
        .get_or_init(|| {
            Arc::new(
                nightfall_api::infrastructure::item_data::load_embedded_phase4_rules([7; 32])
                    .unwrap(),
            )
        })
        .clone()
}
async fn configured() -> (TestApp, Arc<Actor>, Character) {
    let actor = Arc::new(Actor::default());
    let catalogue = Arc::new(Catalogue(rules()));
    let injected = actor.clone();
    let app = TestApp::spawn_with(move |deps| {
        deps.dev_item_grants = true;
        deps.item_grants = Some(injected.clone());
        deps.item_receipts = Some(injected);
        deps.item_catalogue = Some(catalogue);
    })
    .await;
    let character = Character::create(
        AccountId::from_uuid(common::account()),
        CharacterName::new("GrantSocket").unwrap(),
        Race::Human,
    );
    app.characters
        .create_idempotent(&IdempotencyKey::new(), "socket", &character)
        .await
        .unwrap();
    (app, actor, character)
}
fn request(character: &Character) -> pb::GrantItemRequest {
    pb::GrantItemRequest {
        character_id: character.id.as_uuid().as_bytes().to_vec(),
        template_id: 10,
        count: 2,
        idempotency_key: IdempotencyKey::new().to_string(),
    }
}
#[tokio::test]
async fn typed_grant_routing_full_response_frozen_retry_later_grant_and_concurrent_keys() {
    let (app, actor, character) = configured().await;
    let body = request(&character);
    let first = app
        .game_as(common::account())
        .grant_item(body.clone())
        .await
        .unwrap()
        .into_inner();
    let result = first.result.as_ref().unwrap();
    assert_eq!(
        *result,
        pb::ItemOperationResult {
            entity_id: body.character_id.clone(),
            incarnation: 0,
            tick: 71,
            inventory_revision: 2,
            receipt_id: body.idempotency_key.clone(),
            operation: pb::ItemOperation::Grant.into(),
            affected_instance_ids: vec![1],
            recipe_id: None
        }
    );
    let inventory = first.inventory.as_ref().unwrap();
    assert_eq!(inventory.entity_id, body.character_id);
    assert_eq!(inventory.incarnation, 0);
    assert_eq!(inventory.tick, 0);
    assert_eq!(inventory.baseline_tick, 0);
    assert!(inventory.baseline);
    assert_eq!(inventory.inventory_revision, 3);
    assert_eq!(
        inventory.items,
        vec![pb::OwnedItem {
            instance_id: 1,
            template_id: 10,
            count: 2,
            enchant: u32::MAX,
            augment_id: Some(u32::MAX),
            location: pb::ItemLocation::Inventory.into(),
            slot: 0,
            charge_bits: 0,
            revision: 2
        }]
    );
    assert!(inventory.paperdoll.is_empty());
    assert!(inventory.auto_shot_templates.is_empty());
    assert!(inventory.reuse.is_empty());
    assert_eq!(
        inventory.weight,
        Some(pb::ItemWeightState {
            weight: 1,
            max_load: 100_000,
            penalty_bonus: 1,
            per_mille: 0
        })
    );
    assert_eq!(
        inventory.penalties,
        Some(pb::ItemPenaltyState {
            expertise: 0,
            weapon_level: 0,
            armor_level: 0,
            overload_level: 0,
            overloaded: false
        })
    );
    assert_eq!(inventory.rules_hash, rules().hash());
    assert_eq!(inventory.ordinary_capacity, 83);
    assert_eq!(inventory.quest_capacity, 101);
    let mut later = request(&character);
    later.count = 5;
    assert_eq!(
        app.game_as(common::account())
            .grant_item(later)
            .await
            .unwrap()
            .into_inner()
            .inventory
            .unwrap()
            .items[0]
            .count,
        5
    );
    *actor.behavior.lock() = Behavior::Full;
    assert_eq!(
        app.game_as(common::account())
            .grant_item(body.clone())
            .await
            .unwrap()
            .into_inner(),
        first
    );
    let mut changed = body.clone();
    changed.count = 3;
    assert_eq!(
        app.game_as(common::account())
            .grant_item(changed)
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    *actor.behavior.lock() = Behavior::Success;
    let concurrent = request(&character);
    let mut left = app.game_as(common::account());
    let mut right = app.game_as(common::account());
    let (left, right) =
        tokio::join!(left.grant_item(concurrent.clone()), right.grant_item(concurrent));
    assert_eq!(left.unwrap().into_inner(), right.unwrap().into_inner());
    assert_eq!(actor.receipts.lock().len(), 3);
}
#[tokio::test]
async fn typed_grant_auth_validation_ownership_capability_and_runtime_status_matrix() {
    let (app, actor, character) = configured().await;
    let body = request(&character);
    let before = app.characters.staged_events();
    assert_eq!(
        app.anon_grpc()
            .grant_item(body.clone())
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    assert_eq!(
        app.game_with_token("invalid")
            .grant_item(body.clone())
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    assert_eq!(
        app.game_as(uuid::Uuid::from_u128(99))
            .grant_item(body.clone())
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    let mut missing = body.clone();
    missing.character_id = uuid::Uuid::from_u128(99).as_bytes().to_vec();
    assert_eq!(
        app.game_as(common::account())
            .grant_item(missing)
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    for (id, template, count, key) in [
        (vec![0; 15], 10, 1, body.idempotency_key.clone()),
        (body.character_id.clone(), 0, 1, body.idempotency_key.clone()),
        (body.character_id.clone(), u32::MAX, 1, body.idempotency_key.clone()),
        (body.character_id.clone(), 10, 0, body.idempotency_key.clone()),
        (body.character_id.clone(), 10, u64::MAX, body.idempotency_key.clone()),
        (body.character_id.clone(), 10, 1, "bad".into()),
        (body.character_id.clone(), 10, 1, uuid::Uuid::nil().to_string()),
        (body.character_id.clone(), 2_000_000_000, 1, body.idempotency_key.clone()),
    ] {
        let invalid = pb::GrantItemRequest {
            character_id: id,
            template_id: template,
            count,
            idempotency_key: key,
        };
        assert_eq!(
            app.game_as(common::account())
                .grant_item(invalid)
                .await
                .unwrap_err()
                .code(),
            Code::InvalidArgument
        );
    }
    let denied = rules()
        .parts()
        .capabilities
        .values()
        .find(|capability| !capability.storable)
        .unwrap()
        .template_id;
    let mut unsupported = body.clone();
    unsupported.template_id = denied;
    assert_eq!(
        app.game_as(common::account())
            .grant_item(unsupported)
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    assert_eq!(actor.calls.load(Ordering::SeqCst), 0);
    assert!(actor.receipts.lock().is_empty());
    assert_eq!(app.characters.staged_events(), before);
    assert!(app.bus.published().is_empty());
    for (behavior, expected) in [
        (Behavior::Full, Code::ResourceExhausted),
        (Behavior::Deferred, Code::FailedPrecondition),
        (Behavior::Stopped, Code::Unavailable),
        (Behavior::Conflict, Code::FailedPrecondition),
        (Behavior::Infrastructure, Code::Internal),
        (Behavior::BadReply, Code::Internal),
    ] {
        *actor.behavior.lock() = behavior;
        let status = app
            .game_as(common::account())
            .grant_item(request(&character))
            .await
            .unwrap_err();
        assert_eq!(status.code(), expected);
        if expected == Code::Internal {
            assert_eq!(status.message(), "internal error");
        }
    }
    actor.lookup_failure.store(true, Ordering::SeqCst);
    assert_eq!(
        app.game_as(common::account())
            .grant_item(request(&character))
            .await
            .unwrap_err()
            .code(),
        Code::Internal
    );
    actor.lookup_failure.store(false, Ordering::SeqCst);
    *actor.behavior.lock() = Behavior::Pending;
    assert_eq!(
        app.game_as(common::account())
            .grant_item(request(&character))
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
}
#[tokio::test]
async fn every_required_grant_port_absent_or_zone_stopped_is_unavailable_without_writes() {
    let catalogue = Arc::new(Catalogue(rules()));
    for missing in 0..4 {
        let actor = Arc::new(Actor::default());
        let injected = actor.clone();
        let catalogue = catalogue.clone();
        if missing == 3 {
            actor.available.store(false, Ordering::SeqCst);
        }
        let app = TestApp::spawn_with(move |deps| {
            deps.dev_item_grants = true;
            deps.item_grants =
                (missing != 0).then(|| injected.clone() as Arc<dyn ItemGrantRuntime>);
            deps.item_receipts =
                (missing != 1).then_some(injected as Arc<dyn ItemReceiptRepository>);
            deps.item_catalogue = (missing != 2).then_some(catalogue as Arc<dyn ItemCataloguePort>);
        })
        .await;
        assert_eq!(
            app.game_as(common::account())
                .grant_item(pb::GrantItemRequest::default())
                .await
                .unwrap_err()
                .code(),
            Code::Unavailable
        );
        assert!(actor.receipts.lock().is_empty());
        assert_eq!(actor.calls.load(Ordering::SeqCst), 0);
        assert!(app.characters.staged_events().is_empty());
    }
}
