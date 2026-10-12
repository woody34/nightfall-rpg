//! Retained projection consistency and D19/D20 scalar/history storage, before live integration.
#![allow(missing_docs, clippy::unwrap_used)]
mod common;
use nightfall_api::application::ports::ItemInventoryRepository;
use nightfall_api::application::{CharacterCheckpoint, CharacterRepository, IdempotencyKey};
use nightfall_api::domain::zone::phase4_checkpoint::{
    EquipmentSourceKey, ItemReuseKey, RetainedEquipmentSource, RetainedItemReuse, SetActivation,
    SetSkillKind,
};
use nightfall_api::domain::zone::{
    EquipSlot, ItemInstance, ItemInstanceId, ItemLocation, Phase3Checkpoint, Phase4Checkpoint, Tick,
};
use nightfall_api::domain::{AccountId, Character, CharacterName, Position, Race};
use nightfall_api::infrastructure::{
    memory::InMemoryCharacterRepository, postgres::PgCharacterRepository,
};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

fn retained() -> Phase4Checkpoint {
    // Structural storage fixture; catalogue/fold validation remains the admission gate.
    let id = ItemInstanceId::new(1).unwrap();
    Phase4Checkpoint {
        schema_version: 1,
        timer_policy_version: 1,
        rules_hash: [1; 32],
        inventory_revision: 1,
        instances: vec![ItemInstance {
            creation_origin: None,
            id,
            template_id: 1146,
            count: 1,
            location: ItemLocation::Paperdoll,
            slot: Some(EquipSlot::Chest),
            enchant: u32::MAX,
            augment_id: Some(u32::MAX),
            charges: 0,
            created_tick: Tick(0),
            revision: 1,
        }],
        paperdoll: BTreeMap::from([(EquipSlot::Chest, id)]),
        auto_shots: BTreeSet::from([1835]),
        item_reuse_remainders: vec![RetainedItemReuse {
            key: ItemReuseKey::Template(736),
            remaining_ticks: 10,
        }],
        equipment_install_order: vec![RetainedEquipmentSource::Equipment(
            EquipmentSourceKey::ArmorSet {
                chest: id,
                set: 1,
                skill_kind: SetSkillKind::Shield,
                ordinal: 0,
            },
        )],
        active_sets: BTreeMap::from([(
            1,
            SetActivation {
                chest: id,
                full_set_active: false,
                shield_skill_active: true,
                enchant6_active: false,
            },
        )]),
        weight: 1000,
        max_load: 1000,
        ordinary_capacity: 80,
        quest_capacity: 100,
        weight_penalty_bonus: 1,
        overload_level: 0,
        overloaded: true,
        expertise: 0,
        weapon_penalty: 0,
        armor_penalty: 0,
        starter_kit_receipt: None,
    }
}
async fn seed<R: CharacterRepository>(repo: &R) -> Character {
    let c = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("RetainedItems").unwrap(),
        Race::Human,
    );
    repo.create_idempotent(&IdempotencyKey::new(), "retained-inventory", &c)
        .await
        .unwrap();
    repo.checkpoint(
        &CharacterCheckpoint {
            phase4: None,
            character_id: c.id,
            revision_seen: 0,
            level: 1,
            xp: 0,
            hp: 20,
            mp: 10,
            alive: true,
            position: Position::default(),
            idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
            class_state: Some(c.class_state.clone()),
            phase3: Some(Phase3Checkpoint::new()),
        },
        &[],
    )
    .await
    .unwrap();
    c
}

#[tokio::test]
async fn memory_retains_exact_history_and_legacy_none_without_synthesizing_items() {
    let repo = InMemoryCharacterRepository::default();
    let c = seed(&repo).await;
    assert!(repo
        .retained_inventory(c.id)
        .await
        .unwrap()
        .unwrap()
        .checkpoint
        .is_none());
    repo.insert_item_checkpoint_for_test(c.id, retained())
        .unwrap();
    let view = repo.retained_inventory(c.id).await.unwrap().unwrap();
    assert_eq!(view.character_revision, 1);
    assert_eq!(view.checkpoint, Some(retained()));
    let header = repo
        .get(c.id)
        .await
        .unwrap()
        .unwrap()
        .item_metadata
        .unwrap();
    assert_eq!(header.inventory_revision, retained().inventory_revision);
    assert_eq!(header.rules_hash, retained().rules_hash);
    assert_eq!(
        repo.list_by_account(c.account_id)
            .await
            .unwrap()
            .first()
            .unwrap()
            .item_metadata,
        Some(header)
    );

    assert!(repo
        .retained_inventory(nightfall_api::domain::CharacterId::new())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn postgres_retains_full_u32_widths_history_and_rejects_projection_corruption() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let c = seed(&repo).await;
    assert!(repo
        .retained_inventory(c.id)
        .await
        .unwrap()
        .unwrap()
        .checkpoint
        .is_none());
    // Explicit pre-admission fixture seed, never an RPC mutation or a second ownership lane.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE characters SET phase4_checkpoint=$2 WHERE id=$1")
        .bind(c.id.as_uuid())
        .bind(serde_json::to_value(retained()).unwrap())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO item_instances(id,character_id,location,slot,template_id,count,enchant,augment_id,created_tick,revision) VALUES(1,$1,'paperdoll',6,1146,1,4294967295,4294967295,0,1)").bind(c.id.as_uuid()).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let view = repo.retained_inventory(c.id).await.unwrap().unwrap();
    assert_eq!(view.character_revision, 1);
    assert_eq!(view.checkpoint, Some(retained()));
    let header = repo
        .get(c.id)
        .await
        .unwrap()
        .unwrap()
        .item_metadata
        .unwrap();
    assert_eq!(header.inventory_revision, retained().inventory_revision);
    assert_eq!(header.rules_hash, retained().rules_hash);
    assert_eq!(
        repo.list_by_account(c.account_id)
            .await
            .unwrap()
            .first()
            .unwrap()
            .item_metadata,
        Some(header)
    );

    sqlx::query("UPDATE item_instances SET enchant=0 WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.retained_inventory(c.id).await.is_err());
    sqlx::query("UPDATE item_instances SET enchant=4294967295 WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE characters SET phase4_checkpoint=phase4_checkpoint || '{\"unexpected\":true}'::jsonb WHERE id=$1").bind(c.id.as_uuid()).execute(&pool).await.unwrap();
    assert!(repo.retained_inventory(c.id).await.is_err());
    sqlx::query("UPDATE characters SET phase4_checkpoint=NULL WHERE id=$1")
        .bind(c.id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.retained_inventory(c.id).await.is_err());
    sqlx::query("DELETE FROM item_instances WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo
        .retained_inventory(c.id)
        .await
        .unwrap()
        .unwrap()
        .checkpoint
        .is_none());
}

fn periodic(c: &Character, seen: u64, checkpoint: Option<Phase4Checkpoint>) -> CharacterCheckpoint {
    CharacterCheckpoint {
        character_id: c.id,
        revision_seen: seen,
        level: 1,
        xp: 0,
        hp: 20,
        mp: 10,
        alive: true,
        position: Position::default(),
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        class_state: Some(c.class_state.clone()),
        phase3: Some(Phase3Checkpoint::new()),
        phase4: checkpoint,
    }
}

async fn periodic_contract<R: CharacterRepository + ItemInventoryRepository>(
    repo: &R,
    c: &Character,
) {
    use nightfall_api::application::{CheckpointError, CheckpointOutcome};
    let mut paused = retained();
    paused
        .item_reuse_remainders
        .first_mut()
        .unwrap()
        .remaining_ticks = 5;
    let cp = periodic(c, 1, Some(paused.clone()));
    assert_eq!(repo.checkpoint(&cp, &[]).await.unwrap(), CheckpointOutcome::Applied(2));
    assert_eq!(
        repo.retained_inventory(c.id)
            .await
            .unwrap()
            .unwrap()
            .checkpoint,
        Some(paused.clone())
    );
    let mut next = cp.clone();
    next.revision_seen = 2;
    next.idempotency.1 = IdempotencyKey::new();
    next.phase4.as_mut().unwrap().item_reuse_remainders.clear();
    assert_eq!(repo.checkpoint(&next, &[]).await.unwrap(), CheckpointOutcome::Applied(3));
    assert_eq!(repo.checkpoint(&cp, &[]).await.unwrap(), CheckpointOutcome::Replayed(2));
    let downgrade = periodic(c, 3, None);
    assert!(
        matches!(repo.checkpoint(&downgrade, &[]).await, Err(CheckpointError::Constraint(name)) if name == "characters_phase4_downgrade")
    );
    for altered in 0..3 {
        let mut mutation = periodic(c, 3, next.phase4.clone());
        let items = mutation.phase4.as_mut().unwrap();
        match altered {
            0 => items.instances.first_mut().unwrap().enchant = 0,
            1 => items.auto_shots.clear(),
            _ => items.inventory_revision = items.inventory_revision.checked_add(1).unwrap(),
        }
        assert!(
            matches!(repo.checkpoint(&mutation, &[]).await, Err(CheckpointError::Constraint(name)) if name == "item_projection_required")
        );
    }
    let retained = repo.retained_inventory(c.id).await.unwrap().unwrap();
    assert_eq!(retained.character_revision, 3);
    assert_eq!(retained.checkpoint, next.phase4);
}

#[tokio::test]
async fn memory_periodic_checkpoint_preserves_items_retries_and_refuses_unledgered_mutation() {
    let repo = InMemoryCharacterRepository::default();
    let c = seed(&repo).await;
    repo.insert_item_checkpoint_for_test(c.id, retained())
        .unwrap();
    periodic_contract(&repo, &c).await;
}

#[tokio::test]
async fn postgres_periodic_checkpoint_is_atomic_and_fingerprint_compatible() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let c = seed(&repo).await;
    sqlx::query("UPDATE characters SET phase4_checkpoint=$2 WHERE id=$1")
        .bind(c.id.as_uuid())
        .bind(serde_json::to_value(retained()).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO item_instances(id,character_id,location,slot,template_id,count,enchant,augment_id,created_tick,revision) VALUES(1,$1,'paperdoll',6,1146,1,4294967295,4294967295,0,1)")
        .bind(c.id.as_uuid()).execute(&pool).await.unwrap();
    periodic_contract(&repo, &c).await;
    let claims: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM idempotency_keys WHERE operation='save_checkpoint'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(claims, 3, "refused mutations leave no checkpoint claim");
}

#[test]
fn absent_phase4_keeps_legacy_checkpoint_fingerprint_bytes() {
    use sha2::{Digest, Sha256};
    let c = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("Fingerprint").unwrap(),
        Race::Human,
    );
    let cp = periodic(&c, 3, None);
    let body = serde_json::json!({
        "character_id": cp.character_id.as_uuid(), "revision_seen": cp.revision_seen,
        "level":cp.level,"xp":cp.xp,"hp":cp.hp,"mp":cp.mp,"alive":cp.alive,
        "position":[cp.position.x,cp.position.y],"events":Vec::<nightfall_api::domain::DomainEvent>::new(),
        "class_state":cp.class_state,"phase3":cp.phase3,
    });
    assert_eq!(
        cp.fingerprint(&[]),
        format!("{:x}", Sha256::digest(body.to_string().as_bytes()))
    );
    let mut with_items = cp.clone();
    with_items.phase4 = Some(retained());
    assert_ne!(with_items.fingerprint(&[]), cp.fingerprint(&[]));
}

#[tokio::test]
async fn configured_v6_admission_freezes_pending_contents_with_the_parent_revision() {
    let bundle = nightfall_api::infrastructure::item_data::embedded_phase4_bundle().unwrap();
    let memory = InMemoryCharacterRepository::default()
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    let c = seed(&memory).await;
    let loaded = memory.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(loaded.revision, 1);
    let items = loaded.phase4.unwrap();
    assert!(items.checkpoint.is_none());
    assert_eq!(items.pending.len(), 1);
    assert_eq!(
        items
            .pending
            .first()
            .unwrap()
            .materialization
            .manifest_hash()
            .unwrap(),
        items.pending.first().unwrap().manifest_hash
    );
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let postgres = PgCharacterRepository::new(pool.clone())
        .with_phase4_rules(bundle.items, bundle.skills)
        .unwrap();
    let c = seed(&postgres).await;
    let loaded = postgres.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(loaded.revision, 1);
    let stored = loaded.phase4.unwrap();
    assert_eq!(
        stored.pending.first().unwrap().materialization,
        items.pending.first().unwrap().materialization
    );
    assert!(stored.checkpoint.is_none());
    sqlx::query("UPDATE pending_item_requests SET status='materialized', materialized_revision=1, receipt_ref=request_id WHERE character_id=$1")
        .bind(c.id.as_uuid()).execute(&pool).await.unwrap();
    assert!(
        postgres.load_for_admission(c.id).await.is_err(),
        "completed request without ownership cannot be silently discarded"
    );
}
