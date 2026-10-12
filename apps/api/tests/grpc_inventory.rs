//! Owner/lifecycle/zero-origin retained inventory contracts through real sockets.
//! Retained scalar fixtures test transport; catalogue/fold/gameplay validation is separate.
#![allow(missing_docs, clippy::unwrap_used)]
mod common;
use common::TestApp;
use nightfall_api::application::{
    ports::{ItemCataloguePort, ItemInventoryRepository, ItemInventoryRuntime, RetainedInventory},
    use_cases::GetInventory,
    AppError,
};
use nightfall_api::domain::zone::{
    phase4_checkpoint::{ItemReuseKey, RetainedItemReuse, StarterKitReceipt},
    EquipSlot, ItemInstance, ItemInstanceId, ItemLocation, Phase4Checkpoint, Phase4Rules, Tick,
};
use nightfall_api::domain::{AccountId, Character, CharacterId, CharacterName, Race};
use nightfall_api::infrastructure::memory::InMemoryCharacterRepository;
use nightfall_api::interface::grpc::pb;
use parking_lot::Mutex;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, OnceLock,
};
use tonic::Code;
use uuid::Uuid;

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
struct Catalogue(Arc<Phase4Rules>);
impl ItemCataloguePort for Catalogue {
    fn catalogue(&self) -> Result<Arc<Phase4Rules>, AppError> {
        Ok(self.0.clone())
    }
}
struct Inventory {
    value: Mutex<Option<RetainedInventory>>,
    failure: AtomicBool,
    reads: AtomicUsize,
}
#[async_trait::async_trait]
impl ItemInventoryRepository for Inventory {
    async fn retained_inventory(
        &self,
        _: CharacterId,
    ) -> anyhow::Result<Option<RetainedInventory>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(
            !self.failure.load(Ordering::SeqCst),
            "sentinel inventory infrastructure failure"
        );
        Ok(self.value.lock().clone())
    }
}
struct Runtime(u8);
#[async_trait::async_trait]
impl ItemInventoryRuntime for Runtime {
    async fn detached_inventory(
        &self,
        character: CharacterId,
        repo: &dyn ItemInventoryRepository,
    ) -> Result<Option<RetainedInventory>, AppError> {
        if self.0 == 2 {
            return Err(AppError::Unavailable("zone stopped".into()));
        }
        nightfall_api::application::use_cases::get_inventory::require_out_of_world(
            true,
            true,
            self.0 == 1,
        )?;
        Ok(repo.retained_inventory(character).await?)
    }
}
fn cp(c: &Character) -> Phase4Checkpoint {
    Phase4Checkpoint {
        schema_version: 1,
        timer_policy_version: 1,
        rules_hash: rules().hash(),
        inventory_revision: 9,
        instances: vec![
            ItemInstance {
                id: ItemInstanceId::new(77).unwrap(),
                template_id: 57,
                count: 5,
                location: ItemLocation::Inventory,
                slot: None,
                enchant: u32::MAX,
                augment_id: Some(u32::MAX),
                charges: 0,
                created_tick: Tick(3),
                revision: 9,
            },
            ItemInstance {
                id: ItemInstanceId::new(78).unwrap(),
                template_id: 2369,
                count: 1,
                location: ItemLocation::Paperdoll,
                slot: Some(EquipSlot::Rhand),
                enchant: 3,
                augment_id: None,
                charges: 3,
                created_tick: Tick(3),
                revision: 9,
            },
        ],
        paperdoll: BTreeMap::from([(EquipSlot::Rhand, ItemInstanceId::new(78).unwrap())]),
        auto_shots: BTreeSet::from([1835]),
        item_reuse_remainders: vec![
            RetainedItemReuse {
                key: ItemReuseKey::Template(736),
                remaining_ticks: 12,
            },
            RetainedItemReuse {
                key: ItemReuseKey::SharedGroup(2),
                remaining_ticks: 3,
            },
        ],
        equipment_install_order: Vec::new(),
        active_sets: BTreeMap::new(),
        weight: 1000,
        max_load: 5000,
        weight_penalty_bonus: 42,
        overload_level: 1,
        overloaded: true,
        expertise: 3,
        weapon_penalty: 2,
        armor_penalty: 1,
        starter_kit_receipt: Some(StarterKitReceipt {
            manifest_version: 1,
            manifest_hash: [1; 32],
            source_class: c.class_state.base_class_id,
            request_id: Uuid::from_u128(1),
            materialized_revision: 1,
        }),
    }
}
fn fixture() -> (Arc<InMemoryCharacterRepository>, Character, Arc<Inventory>) {
    let repo = Arc::new(InMemoryCharacterRepository::default());
    let c = Character::create(
        AccountId::from_uuid(common::account()),
        CharacterName::new("InventoryOwner").unwrap(),
        Race::Dwarf,
    );
    repo.insert_for_test(c.clone());
    let checkpoint = cp(&c);
    checkpoint.validate_structure(0).unwrap();
    let inv = Arc::new(Inventory {
        value: Mutex::new(Some(RetainedInventory {
            character_id: c.id,
            character_revision: 13,
            checkpoint: Some(checkpoint),
        })),
        failure: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
    });
    (repo, c, inv)
}
fn request(c: &Character) -> pb::GetInventoryRequest {
    pb::GetInventoryRequest {
        character_id: c.id.as_uuid().as_bytes().to_vec(),
    }
}
fn catalogue() -> Arc<dyn ItemCataloguePort> {
    Arc::new(Catalogue(rules()))
}

#[tokio::test]
async fn unit_owner_read_and_every_reachable_error_leave_inventory_unchanged() {
    let (repo, c, inv) = fixture();
    let read = GetInventory::new(
        repo.clone(),
        Some(inv.clone()),
        Some(Arc::new(Runtime(0))),
        Some(catalogue()),
    );
    let out = read
        .execute(c.account_id, &request(&c).character_id)
        .await
        .unwrap();
    assert_eq!(out.character_revision, 13);
    assert_eq!(out.checkpoint, cp(&c));
    assert!(matches!(
        read.execute(c.account_id, &[]).await,
        Err(AppError::InvalidArgument(_))
    ));
    assert!(matches!(
        read.execute(AccountId::new(), &request(&c).character_id)
            .await,
        Err(AppError::PermissionDenied(_))
    ));
    assert!(matches!(
        read.execute(c.account_id, Uuid::now_v7().as_bytes()).await,
        Err(AppError::NotFound { .. })
    ));
    let live = GetInventory::new(
        repo.clone(),
        Some(inv.clone()),
        Some(Arc::new(Runtime(1))),
        Some(catalogue()),
    );
    let before = inv.reads.load(Ordering::SeqCst);
    assert!(matches!(
        live.execute(c.account_id, &request(&c).character_id).await,
        Err(AppError::FailedPrecondition(_))
    ));
    assert_eq!(inv.reads.load(Ordering::SeqCst), before);
    inv.failure.store(true, Ordering::SeqCst);
    assert!(matches!(
        read.execute(c.account_id, &request(&c).character_id).await,
        Err(AppError::Infrastructure(_))
    ));
    inv.failure.store(false, Ordering::SeqCst);
    let stopped = GetInventory::new(
        repo.clone(),
        Some(inv.clone()),
        Some(Arc::new(Runtime(2))),
        Some(catalogue()),
    );
    assert!(matches!(
        stopped
            .execute(c.account_id, &request(&c).character_id)
            .await,
        Err(AppError::Unavailable(_))
    ));
    assert!(repo.staged_events().is_empty());
}

#[tokio::test]
async fn detached_owner_baseline_has_exact_fields_and_cannot_be_a_live_ws_baseline() {
    let (repo, c, inv) = fixture();
    let configured = repo.clone();
    let configured_inv = inv.clone();
    let app = TestApp::spawn_with(move |deps| {
        deps.characters = configured;
        deps.item_inventory = Some(configured_inv);
        deps.item_catalogue = Some(catalogue());
    })
    .await;
    let response = app
        .grpc
        .clone()
        .get_inventory(request(&c))
        .await
        .unwrap()
        .into_inner()
        .inventory
        .unwrap();
    assert_eq!(response.entity_id, c.id.as_uuid().as_bytes());
    assert_eq!((response.tick, response.baseline_tick, response.incarnation), (0, 0, 0));
    assert!(response.baseline);
    assert_eq!(response.inventory_revision, 9);
    assert_eq!(response.rules_hash, rules().hash());
    assert_eq!(response.ordinary_capacity, rules().parts().capacities.dwarf_inventory);
    assert_eq!(response.quest_capacity, rules().parts().capacities.quest_inventory);
    assert_eq!(
        response.items,
        vec![
            pb::OwnedItem {
                instance_id: 77,
                template_id: 57,
                count: 5,
                enchant: u32::MAX,
                augment_id: Some(u32::MAX),
                location: pb::ItemLocation::Inventory.into(),
                slot: 0,
                charge_bits: 0,
                revision: 9
            },
            pb::OwnedItem {
                instance_id: 78,
                template_id: 2369,
                count: 1,
                enchant: 3,
                augment_id: None,
                location: pb::ItemLocation::Paperdoll.into(),
                slot: pb::EquipSlot::Rhand.into(),
                charge_bits: 3,
                revision: 9,
            }
        ]
    );
    assert_eq!(
        response.paperdoll,
        [pb::PaperdollEntry {
            slot: pb::EquipSlot::Rhand.into(),
            instance_id: 78
        }]
    );
    assert_eq!(response.auto_shot_templates, [1835]);
    assert_eq!(
        response.weight,
        Some(pb::ItemWeightState {
            weight: 1000,
            max_load: 5000,
            penalty_bonus: 42,
            per_mille: 200
        })
    );
    assert_eq!(
        response.penalties,
        Some(pb::ItemPenaltyState {
            expertise: 3,
            weapon_level: 2,
            armor_level: 1,
            overload_level: 1,
            overloaded: true
        })
    );
    assert_eq!(
        response.reuse,
        vec![
            pb::ItemReuseState {
                kind: pb::ItemReuseKind::Template.into(),
                key: 736,
                until_tick: 12
            },
            pb::ItemReuseState {
                kind: pb::ItemReuseKind::SharedGroup.into(),
                key: 2,
                until_tick: 3
            }
        ]
    );
    // The C2 owner hydration fence requires the live identity incarnation (always positive).
    let live_incarnation = 1;
    assert_ne!(response.incarnation, live_incarnation);
    assert_eq!(
        app.grpc
            .clone()
            .get_inventory(request(&c))
            .await
            .unwrap()
            .into_inner()
            .inventory
            .unwrap(),
        response
    );
    assert!(repo.staged_events().is_empty());
    assert!(app.bus.published().is_empty());
}

#[tokio::test]
async fn real_admitted_owner_is_refused_before_any_retained_database_read() {
    let (_, _, inv) = fixture();
    let data = inv.clone();
    let app = TestApp::spawn_with(move |deps| {
        deps.item_inventory = Some(data);
        deps.item_catalogue = Some(catalogue());
    })
    .await;
    let player = common::ws::seed_player(&app, "LiveInventory", 10.0, 10.0);
    let mut socket = common::ws::join(&app, &player).await;
    socket
        .until(|message| {
            common::ws::spawn_of(message).is_some_and(|spawn| spawn.entity_id == player.entity_id())
        })
        .await;
    let before = inv.reads.load(Ordering::SeqCst);
    let error = app
        .game_as(player.account)
        .get_inventory(pb::GetInventoryRequest {
            character_id: player.character.as_uuid().as_bytes().to_vec(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
    assert_eq!(inv.reads.load(Ordering::SeqCst), before);
}

#[tokio::test]
async fn wire_errors_auth_owner_ids_pending_corruption_and_each_absent_port() {
    let (repo, c, inv) = fixture();
    let data = inv.clone();
    let configured = repo.clone();
    let app = TestApp::spawn_with(move |deps| {
        deps.characters = configured;
        deps.item_inventory = Some(data);
        deps.item_catalogue = Some(catalogue());
    })
    .await;
    assert_eq!(
        app.anon_grpc()
            .get_inventory(request(&c))
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    assert_eq!(
        app.game_as(Uuid::from_u128(99))
            .get_inventory(request(&c))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    assert_eq!(
        app.grpc
            .clone()
            .get_inventory(pb::GetInventoryRequest::default())
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        app.grpc
            .clone()
            .get_inventory(pb::GetInventoryRequest {
                character_id: Uuid::now_v7().as_bytes().to_vec()
            })
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    inv.failure.store(true, Ordering::SeqCst);
    let error = app
        .grpc
        .clone()
        .get_inventory(request(&c))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Internal);
    assert_eq!(error.message(), "internal error");
    inv.failure.store(false, Ordering::SeqCst);
    inv.value.lock().as_mut().unwrap().checkpoint = None;
    assert_eq!(
        app.grpc
            .clone()
            .get_inventory(request(&c))
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    inv.value.lock().as_mut().unwrap().checkpoint = Some(cp(&c));
    inv.value
        .lock()
        .as_mut()
        .unwrap()
        .checkpoint
        .as_mut()
        .unwrap()
        .rules_hash = [0; 32];
    assert_eq!(
        app.grpc
            .clone()
            .get_inventory(request(&c))
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    for (zone, cat) in [(false, true), (true, false)] {
        let runtime = zone.then(|| Arc::new(Runtime(0)) as Arc<dyn ItemInventoryRuntime>);
        let cat = cat.then(catalogue);
        let use_case = GetInventory::new(repo.clone(), Some(inv.clone()), runtime, cat.clone());
        let configured = TestApp::spawn_with_get_inventory(repo.clone(), cat, use_case).await;
        assert_eq!(
            configured
                .grpc
                .clone()
                .get_inventory(request(&c))
                .await
                .unwrap_err()
                .code(),
            Code::Unavailable
        );
    }
    assert!(repo.staged_events().is_empty());
    assert!(app.bus.published().is_empty());
}

#[tokio::test]
async fn postgres_detached_transport_reads_consistent_revision_and_rejects_row_drift() {
    use nightfall_api::application::{CharacterRepository, IdempotencyKey};
    use nightfall_api::infrastructure::postgres::PgCharacterRepository;
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let repo = Arc::new(PgCharacterRepository::new(pool.clone()));
    let c = Character::create(
        AccountId::from_uuid(common::account()),
        CharacterName::new("StoredInventory").unwrap(),
        Race::Dwarf,
    );
    repo.create_idempotent(&IdempotencyKey::new(), "detached-db-seed", &c)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE characters SET revision=13,phase3_timer_policy_version=1,phase4_checkpoint=$2 WHERE id=$1").bind(c.id.as_uuid()).bind(serde_json::to_value(cp(&c)).unwrap()).execute(&mut *tx).await.unwrap();
    for item in &cp(&c).instances {
        sqlx::query("INSERT INTO item_instances(id,character_id,location,slot,template_id,count,enchant,augment_id,charge_bits,created_tick,revision) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
            .bind(i64::try_from(item.id.get()).unwrap()).bind(c.id.as_uuid()).bind(if item.location==ItemLocation::Inventory{"inventory"}else{"paperdoll"})
            .bind(item.slot.map(|slot|i16::from(slot as u8))).bind(i32::try_from(item.template_id).unwrap()).bind(i64::try_from(item.count).unwrap())
            .bind(i64::from(item.enchant)).bind(item.augment_id.map(i64::from)).bind(i16::from(item.charges)).bind(i64::try_from(item.created_tick.0).unwrap()).bind(i64::try_from(item.revision).unwrap()).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
    let configured = repo.clone();
    let app = TestApp::spawn_with(move |deps| {
        deps.characters = configured.clone();
        deps.item_inventory = Some(configured);
        deps.item_catalogue = Some(catalogue());
    })
    .await;
    let first = app
        .grpc
        .clone()
        .get_inventory(request(&c))
        .await
        .unwrap()
        .into_inner()
        .inventory
        .unwrap();
    assert_eq!(first.inventory_revision, 9);
    assert_character_header(&app, &c, 9, &first.rules_hash).await;
    assert_eq!(
        repo.retained_inventory(c.id)
            .await
            .unwrap()
            .unwrap()
            .character_revision,
        13
    );
    let mut newer = cp(&c);
    newer.inventory_revision = 10;
    newer.instances.first_mut().unwrap().count = 6;
    newer.instances.first_mut().unwrap().revision = 10;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE characters SET revision=14,phase4_checkpoint=$2 WHERE id=$1")
        .bind(c.id.as_uuid())
        .bind(serde_json::to_value(newer).unwrap())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE item_instances SET count=6,revision=10 WHERE id=77")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let latest = app
        .grpc
        .clone()
        .get_inventory(request(&c))
        .await
        .unwrap()
        .into_inner()
        .inventory
        .unwrap();
    assert_eq!((latest.inventory_revision, latest.items.first().unwrap().count), (10, 6));
    assert_character_header(&app, &c, 10, &latest.rules_hash).await;
    assert_eq!(
        repo.retained_inventory(c.id)
            .await
            .unwrap()
            .unwrap()
            .character_revision,
        14
    );
    sqlx::query("UPDATE item_instances SET count=7 WHERE id=77")
        .execute(&pool)
        .await
        .unwrap();
    let error = app
        .grpc
        .clone()
        .get_inventory(request(&c))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Internal);
    assert_eq!(error.message(), "internal error");
    assert!(app.bus.published().is_empty());
}

async fn assert_character_header(
    app: &TestApp,
    character: &Character,
    revision: u64,
    rules_hash: &[u8],
) {
    let header = app
        .grpc
        .clone()
        .get_character(pb::GetCharacterRequest {
            character_id: character.id.to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(header.inventory_revision, revision);
    assert_eq!(header.item_rules_hash, rules_hash);
    let listed = app
        .grpc
        .clone()
        .list_my_characters(pb::ListMyCharactersRequest {})
        .await
        .unwrap()
        .into_inner();
    let header = listed.characters.first().unwrap();
    assert_eq!(header.inventory_revision, revision);
    assert_eq!(header.item_rules_hash, rules_hash);
}
