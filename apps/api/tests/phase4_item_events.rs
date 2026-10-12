//! API-owned item subject/serialization contracts; operation semantics stay in the actor.
#![allow(missing_docs, clippy::unwrap_used)]
use futures_util::StreamExt as _;
use nightfall_api::application::EventBus;
use nightfall_api::domain::zone::{ItemInstance, ItemInstanceId, ItemLocation, Tick};
use nightfall_api::domain::{CharacterId, DomainEvent, EventMetadata, ItemEventFact};
use nightfall_api::infrastructure::nats::NatsEventBus;
mod common;

use std::time::Duration;
use uuid::Uuid;

fn facts() -> Vec<DomainEvent> {
    let item = ItemInstance {
        id: ItemInstanceId::new(1).unwrap(),
        template_id: 57,
        count: 1,
        location: ItemLocation::Inventory,
        slot: None,
        enchant: 0,
        augment_id: None,
        charges: 0,
        created_tick: Tick(1),
        revision: 1,
    };
    let fact = ItemEventFact {
        metadata: EventMetadata {
            event_id: Uuid::now_v7(),
            sequence: (1, 0),
        },
        character_id: CharacterId::new(),
        instance_id: item.id,
        template_id: item.template_id,
        count_delta: 0,
        before: Some(item.clone()),
        after: Some(item),
        causes: vec![serde_json::json!({"ordinal":0,"fixture":"subject_contract"})],
    };
    vec![
        DomainEvent::ItemGranted { fact: fact.clone() },
        DomainEvent::ItemEquipped { fact: fact.clone() },
        DomainEvent::ItemUnequipped { fact: fact.clone() },
        DomainEvent::ItemConsumed { fact: fact.clone() },
        DomainEvent::ItemEnchanted { fact: fact.clone() },
        DomainEvent::ItemDestroyed { fact: fact.clone() },
        DomainEvent::ItemRefined { fact: fact.clone() },
        DomainEvent::ItemExchanged { fact },
    ]
}

#[test]
fn all_item_subjects_are_exact_and_scalar_audit_round_trips() {
    for (event, subject) in facts().into_iter().zip([
        "granted",
        "equipped",
        "unequipped",
        "consumed",
        "enchanted",
        "destroyed",
        "refined",
        "exchanged",
    ]) {
        assert_eq!(event.subject(), format!("nightfall.item.{subject}"));
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(serde_json::from_value::<DomainEvent>(json).unwrap(), event);
    }
}

#[tokio::test]
async fn every_item_variant_arrives_over_the_owned_nats_subject() {
    let Ok(url) = std::env::var("NATS_URL") else {
        return;
    };
    let bus = NatsEventBus::connect(&url).await.unwrap();
    let mut subscription = bus.client().subscribe("nightfall.item.*").await.unwrap();
    bus.client().flush().await.unwrap();
    for event in facts() {
        bus.publish(&event).await.unwrap();
        bus.client().flush().await.unwrap();
        let received = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(message) = subscription.next().await {
                let decoded: DomainEvent = serde_json::from_slice(&message.payload).unwrap();
                if decoded == event {
                    return Some((message.subject.to_string(), decoded));
                }
            }
            None
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(received.0, event.subject());
        assert_eq!(received.1, event);
    }
}

#[test]
fn v5_snapshot_grammar_rejects_every_item_audit_subject() {
    use nightfall_api::domain::zone::{CheckpointSnapshot, EntityId, ZoneState};
    use nightfall_api::infrastructure::{
        class_data::{load_classes, ClassSource},
        skill_data::{load_phase3_rules, Phase3DataSource},
    };
    let base = common::fixture_zone().snapshot();
    let zone = ZoneState::new_phase3(
        base.seed,
        base.bounds,
        base.time_origin_ms,
        common::rules(),
        load_classes(&ClassSource::embedded()).unwrap().registry,
        load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap(),
    )
    .unwrap();
    let original = zone.snapshot();
    assert!(original.validate_version().is_ok());
    for event in facts() {
        let mut snapshot = original.clone();
        snapshot.checkpoints.push(CheckpointSnapshot {
            entity: EntityId::from_uuid(Uuid::from_u128(1)),
            revision: 0,
            last_saved: Tick(0),
            dirty: false,
            fenced: false,
            events: vec![event],
            latest: None,
        });
        assert!(snapshot.validate_version().is_err());
    }
}

#[test]
fn exchanged_fact_preserves_distinct_before_after_templates_and_selects_current_template() {
    let item = ItemInstance {
        id: ItemInstanceId::new(1).unwrap(),
        template_id: 72,
        count: 1,
        location: ItemLocation::Inventory,
        slot: None,
        enchant: 3,
        augment_id: None,
        charges: 0,
        created_tick: Tick(1),
        revision: 1,
    };
    let mut after = item.clone();
    after.template_id = 4681;
    after.revision = 2;
    let fact = ItemEventFact {
        metadata: EventMetadata::default(),
        character_id: CharacterId::new(),
        instance_id: item.id,
        template_id: after.template_id,
        count_delta: 0,
        before: Some(item.clone()),
        after: Some(after.clone()),
        causes: Vec::new(),
    };
    fact.validate().unwrap();
    let event = DomainEvent::ItemExchanged { fact: fact.clone() };
    assert_eq!(
        serde_json::from_slice::<DomainEvent>(&serde_json::to_vec(&event).unwrap()).unwrap(),
        event
    );
    let mut invalid = fact;
    invalid.template_id = item.template_id;
    assert!(invalid.validate().is_err());
}
