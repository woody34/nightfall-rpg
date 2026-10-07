//! Runs only when `NATS_URL` is set (docker compose up -d).
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use futures_util::StreamExt;
use nightfall_api::application::EventBus;
use nightfall_api::domain::{CharacterId, DomainEvent, Race};
use nightfall_api::infrastructure::nats::NatsEventBus;
use uuid::Uuid;

#[tokio::test]
async fn published_event_arrives_on_its_subject_as_json() {
    let Ok(url) = std::env::var("NATS_URL") else {
        return;
    };
    let bus = NatsEventBus::connect(&url).await.unwrap();
    let mut sub = bus
        .client()
        .subscribe("nightfall.character.>")
        .await
        .unwrap();

    let event = DomainEvent::CharacterCreated {
        character_id: CharacterId::new(),
        account_id: Uuid::nil(),
        race: Race::DarkElf,
    };
    bus.publish(&event).await.unwrap();

    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), sub.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(msg.subject.as_str(), "nightfall.character.created");
    let decoded: DomainEvent = serde_json::from_slice(&msg.payload).unwrap();
    assert_eq!(decoded, event);
}
