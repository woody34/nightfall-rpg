//! `CreateCharacter` records every playable pinned kit as input, never speculative ownership.
#![allow(missing_docs, clippy::unwrap_used)]
mod common;
use common::TestApp;
use nightfall_api::application::ports::{ItemAdmissionRepository, PendingItemManifest};
use nightfall_api::domain::CharacterId;
use nightfall_api::interface::grpc::pb;
use uuid::Uuid;
#[tokio::test]
async fn every_base_lineage_records_one_kit_and_wire_retry_adds_none() {
    let app = TestApp::spawn().await;
    for (race, base, first_template, name) in [
        (pb::Race::Human, 0, 2369, "HumanBlade"),
        (pb::Race::Human, 10, 6, "HumanMage"),
        (pb::Race::Elf, 18, 2369, "ElvenBlade"),
        (pb::Race::Elf, 25, 6, "ElvenMage"),
        (pb::Race::DarkElf, 31, 2369, "DarkBlade"),
        (pb::Race::DarkElf, 38, 6, "DarkMage"),
        (pb::Race::Orc, 44, 2369, "OrcBlade"),
        (pb::Race::Orc, 49, 2368, "OrcMage"),
        (pb::Race::Dwarf, 53, 2370, "DwarfBlade"),
    ] {
        let mut client = app.game_as(Uuid::from_u128(u128::from(base) + 1));
        let request = pb::CreateCharacterRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            name: name.into(),
            race: race.into(),
            base_class_id: Some(base),
            ..Default::default()
        };
        let created = client
            .create_character(request.clone())
            .await
            .unwrap()
            .into_inner();
        let id = CharacterId::from_uuid(Uuid::parse_str(&created.id).unwrap());
        let pending = app.characters.pending_item_requests(id).await.unwrap();
        assert_eq!(pending.len(), 1);
        let kit = match &pending.first().unwrap().manifest {
            PendingItemManifest::Kit(kit) => Some(kit),
        }
        .unwrap();
        assert_eq!(kit.source_class_id.0, base);
        assert_eq!(kit.items.first().unwrap().template_id, first_template);
        assert!(kit
            .items
            .iter()
            .any(|item| item.template_id == 10650 && item.count == 5));
        assert!(kit
            .items
            .iter()
            .any(|item| item.template_id == 12753 && item.count == 10));
        assert_eq!(client.create_character(request).await.unwrap().into_inner(), created);
        assert_eq!(app.characters.pending_item_requests(id).await.unwrap(), pending);
    }
    assert_eq!(app.characters.staged_events().len(), 9);
    assert!(app.bus.published().is_empty());
}
