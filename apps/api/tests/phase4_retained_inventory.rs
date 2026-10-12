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
