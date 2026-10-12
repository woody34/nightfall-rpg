//! Frozen Phase 4 tag map, integer widths, privacy and legacy byte compatibility.
use nightfall_api::interface::grpc::pb;
use prost::Message;

#[test]
fn item_intents_are_additive_and_round_trip() -> Result<(), prost::DecodeError> {
    use pb::client_message::Intent;
    let intents = [
        Intent::EquipItem(pb::EquipItemRequest::default()),
        Intent::UnequipItem(pb::UnequipItemRequest::default()),
        Intent::UseItem(pb::UseItemRequest::default()),
        Intent::DestroyItem(pb::DestroyItemRequest::default()),
        Intent::EnchantItem(pb::EnchantItemRequest::default()),
        Intent::ToggleAutoShot(pb::ToggleAutoShotRequest::default()),
        Intent::RefineItem(pb::RefineItemRequest::default()),
        Intent::CancelRefine(pb::CancelRefineRequest::default()),
        Intent::ExchangeSpecialAbility(pb::ExchangeSpecialAbilityRequest::default()),
        Intent::CrystallizeItem(pb::CrystallizeItemRequest::default()),
    ];
    for (intent, key) in intents.into_iter().zip((154_u8..=226).step_by(8)) {
        let message = pb::ClientMessage {
            seq: 0,
            intent: Some(intent),
        };
        let bytes = message.encode_to_vec();
        assert_eq!(bytes, [key, 1, 0]);
        assert_eq!(pb::ClientMessage::decode(bytes.as_slice())?, message);
    }
    Ok(())
}

#[test]
fn item_events_are_additive_and_round_trip() -> Result<(), prost::DecodeError> {
    use pb::world_event::Event;
    let events = [
        Event::InventorySync(pb::InventorySync::default()),
        Event::InventoryChanged(pb::InventoryChanged::default()),
        Event::EquipmentChanged(pb::EquipmentChanged::default()),
        Event::WeightChanged(pb::WeightChanged::default()),
        Event::PenaltyChanged(pb::PenaltyChanged::default()),
        Event::EnchantResult(pb::EnchantResult::default()),
        Event::RefineResult(pb::RefineResult::default()),
        Event::ItemOperationResult(pb::ItemOperationResult::default()),
    ];
    for (event, key) in events
        .into_iter()
        .zip([210, 218, 226, 234, 242, 250, 130, 138])
    {
        let message = pb::WorldEvent { event: Some(event) };
        let bytes = message.encode_to_vec();
        assert_eq!(bytes, [key, if key < 210 { 2 } else { 1 }, 0]);
        assert_eq!(pb::WorldEvent::decode(bytes.as_slice())?, message);
    }
    Ok(())
}

#[test]
fn additions_leave_legacy_bytes_unchanged() -> Result<(), prost::DecodeError> {
    let stats = [10, 1, b'p', 16, 3, 24, 5, 32, 7];
    assert_eq!(pb::StatsChanged::decode(stats.as_slice())?.encode_to_vec(), stats);
    let spawn = [10, 1, b'p', 18, 1, b'n', 32, 1];
    assert_eq!(pb::EntitySpawn::decode(spawn.as_slice())?.encode_to_vec(), spawn);
    let character = [10, 1, b'p', 18, 1, b'n', 24, 1];
    assert_eq!(pb::Character::decode(character.as_slice())?.encode_to_vec(), character);
    let effective = [8, 1, 16, 2, 80, 3];
    assert_eq!(pb::EffectiveStats::decode(effective.as_slice())?.encode_to_vec(), effective);
    Ok(())
}

#[test]
fn full_width_owner_values_and_presence_survive_wire() -> Result<(), prost::DecodeError> {
    let item = pb::OwnedItem {
        instance_id: i64::MAX.unsigned_abs(),
        template_id: u32::MAX,
        count: u64::MAX,
        enchant: u32::MAX,
        augment_id: Some(0),
        location: pb::ItemLocation::Paperdoll.into(),
        slot: pb::EquipSlot::Rhand.into(),
        charge_bits: 7,
        revision: u64::MAX,
    };
    assert_eq!(pb::OwnedItem::decode(item.encode_to_vec().as_slice())?, item);
    let result = pb::RefineResult {
        augment_id: Some(0),
        ..Default::default()
    };
    assert_eq!(pb::RefineResult::decode(result.encode_to_vec().as_slice())?, result);
    let public = pb::EquipmentChanged {
        tick: 1,
        ..Default::default()
    };
    assert_eq!(public.encode_to_vec(), [24, 1]); // reserved tag4 never emitted
    assert_eq!(i32::from(pb::EquipSlot::Under), 1);
    assert_eq!(i32::from(pb::EquipSlot::Belt), 25);
    assert_eq!(i32::from(pb::ItemGrade::Ng), 1);
    assert_eq!(i32::from(pb::ItemGrade::S84), 8);
    assert_eq!(i32::from(pb::RejectReason::ItemNotOwned), 25);
    assert_eq!(i32::from(pb::RejectReason::CrystallizeLevelTooLow), 37);
    Ok(())
}
