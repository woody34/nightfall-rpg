//! Frozen Phase 5 tag table (plan §2.1 == design §13): intents 29..69, events 34..45,
//! `RejectReason` 38..88, `ItemOperation` 14..42, `ItemLocation` 3..8, `EntityKind` 3,
//! `EntitySpawn.ground_item` 30, `EconomyResult` outcomes 20..31; legacy bytes unchanged.
#![allow(missing_docs)]
use nightfall_api::interface::grpc::pb;
use prost::encoding::{encode_key, WireType};
use prost::Message;

/// The protobuf key bytes of a length-delimited field `tag`, followed by a zero length.
fn empty_field(tag: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    encode_key(tag, WireType::LengthDelimited, &mut bytes);
    bytes.push(0);
    bytes
}

#[test]
fn economy_intents_hold_tags_29_to_69_and_round_trip() -> Result<(), prost::DecodeError> {
    use pb::client_message::Intent;
    let intents = [
        (Intent::PickUpItem(pb::PickUpItemRequest::default()), 29),
        (Intent::VendorBuy(pb::VendorBuyRequest::default()), 30),
        (Intent::VendorSell(pb::VendorSellRequest::default()), 31),
        (Intent::VendorRefund(pb::VendorRefundRequest::default()), 32),
        (Intent::MultisellChoose(pb::MultisellChooseRequest::default()), 33),
        (Intent::RegisterRecipe(pb::RegisterRecipeRequest::default()), 34),
        (Intent::DeleteRecipe(pb::DeleteRecipeRequest::default()), 35),
        (Intent::CraftItem(pb::CraftItemRequest::default()), 36),
        (Intent::TradeRequest(pb::TradeRequestRequest::default()), 37),
        (Intent::TradeAnswer(pb::TradeAnswerRequest::default()), 38),
        (Intent::TradeAddItem(pb::TradeAddItemRequest::default()), 39),
        (Intent::TradeConfirm(pb::TradeConfirmRequest::default()), 40),
        (Intent::TradeCancel(pb::TradeCancelRequest::default()), 41),
        (Intent::SetPrivateStoreSell(pb::SetPrivateStoreSellRequest::default()), 42),
        (Intent::SetPrivateStoreBuy(pb::SetPrivateStoreBuyRequest::default()), 43),
        (Intent::SetPrivateManufacture(pb::SetPrivateManufactureRequest::default()), 44),
        (Intent::ClosePrivateStore(pb::ClosePrivateStoreRequest::default()), 45),
        (Intent::PrivateStoreBuy(pb::PrivateStoreBuyRequest::default()), 46),
        (Intent::PrivateStoreSell(pb::PrivateStoreSellRequest::default()), 47),
        (
            Intent::PrivateManufactureOrder(pb::PrivateManufactureOrderRequest::default()),
            48,
        ),
        (Intent::PartyInvite(pb::PartyInviteRequest::default()), 49),
        (Intent::PartyAnswer(pb::PartyAnswerRequest::default()), 50),
        (Intent::PartyLeave(pb::PartyLeaveRequest::default()), 51),
        (Intent::PartyKick(pb::PartyKickRequest::default()), 52),
        (Intent::PartyChangeLeader(pb::PartyChangeLeaderRequest::default()), 53),
        (Intent::PartyLootChangeRequest(pb::PartyLootChangeRequest::default()), 54),
        (Intent::PartyLootChangeAnswer(pb::PartyLootChangeAnswer::default()), 55),
        (Intent::WarehouseDeposit(pb::WarehouseDepositRequest::default()), 56),
        (Intent::WarehouseWithdraw(pb::WarehouseWithdrawRequest::default()), 57),
        (Intent::FreightSend(pb::FreightSendRequest::default()), 58),
        (Intent::FreightWithdraw(pb::FreightWithdrawRequest::default()), 59),
        (Intent::MailSend(pb::MailSendRequest::default()), 60),
        (Intent::MailTakeAttachments(pb::MailTakeAttachmentsRequest::default()), 61),
        (Intent::MailReject(pb::MailRejectRequest::default()), 62),
        (Intent::MailCancel(pb::MailCancelRequest::default()), 63),
        (Intent::MailDelete(pb::MailDeleteRequest::default()), 64),
        (Intent::ManorBuySeed(pb::ManorBuySeedRequest::default()), 65),
        (Intent::ManorSellCrop(pb::ManorSellCropRequest::default()), 66),
        (Intent::ItemAuctionBid(pb::ItemAuctionBidRequest::default()), 67),
        (Intent::ItemAuctionCancelBid(pb::ItemAuctionCancelBidRequest::default()), 68),
        (Intent::MailMarkRead(pb::MailMarkReadRequest::default()), 69),
    ];
    assert_eq!(intents.len(), 41);
    for (intent, tag) in intents {
        let message = pb::ClientMessage {
            seq: 0,
            intent: Some(intent),
        };
        let bytes = message.encode_to_vec();
        assert_eq!(bytes, empty_field(tag), "intent tag {tag}");
        assert_eq!(pb::ClientMessage::decode(bytes.as_slice())?, message);
    }
    Ok(())
}

#[test]
fn economy_events_hold_tags_34_to_45_and_round_trip() -> Result<(), prost::DecodeError> {
    use pb::world_event::Event;
    let events = [
        (Event::EconomyResult(pb::EconomyResult::default()), 34),
        (Event::CorpseStateChanged(pb::CorpseStateChanged::default()), 35),
        (Event::PartyStateChanged(pb::PartyStateChanged::default()), 36),
        (Event::PartyLootVote(pb::PartyLootVote::default()), 37),
        (Event::PartyInviteReceived(pb::PartyInviteReceived::default()), 38),
        (Event::TradeRequestReceived(pb::TradeRequestReceived::default()), 39),
        (Event::TradeSessionChanged(pb::TradeSessionChanged::default()), 40),
        (Event::PrivateStoreChanged(pb::PrivateStoreChanged::default()), 41),
        (Event::MailNotification(pb::MailNotification::default()), 42),
        (Event::RecipeBookSync(pb::RecipeBookSync::default()), 43),
        (Event::ItemAuctionStatusChanged(pb::ItemAuctionStatusChanged::default()), 44),
        (Event::LootDistributed(pb::LootDistributed::default()), 45),
    ];
    assert_eq!(events.len(), 12);
    for (event, tag) in events {
        let message = pb::WorldEvent { event: Some(event) };
        let bytes = message.encode_to_vec();
        assert_eq!(bytes, empty_field(tag), "event tag {tag}");
        assert_eq!(pb::WorldEvent::decode(bytes.as_slice())?, message);
    }
    Ok(())
}

#[test]
fn economy_result_outcomes_hold_tags_20_to_31() -> Result<(), prost::DecodeError> {
    use pb::economy_result::Outcome;
    let outcomes = [
        (Outcome::Pickup(pb::PickupOutcome::default()), 20),
        (Outcome::Vendor(pb::VendorOutcome::default()), 21),
        (Outcome::Craft(pb::CraftOutcome::default()), 22),
        (Outcome::Multisell(pb::ExchangeOutcome::default()), 23),
        (Outcome::Trade(pb::TradeOutcome::default()), 24),
        (Outcome::Store(pb::StoreOutcome::default()), 25),
        (Outcome::Container(pb::ContainerOutcome::default()), 26),
        (Outcome::Mail(pb::MailOutcome::default()), 27),
        (Outcome::Manor(pb::ManorOutcome::default()), 28),
        (Outcome::Auction(pb::AuctionOutcome::default()), 29),
        (Outcome::Party(pb::PartyOutcome::default()), 30),
        (Outcome::Recipe(pb::RecipeOutcome::default()), 31),
    ];
    assert_eq!(outcomes.len(), 12);
    for (outcome, tag) in outcomes {
        let message = pb::EconomyResult {
            outcome: Some(outcome),
            ..Default::default()
        };
        let bytes = message.encode_to_vec();
        assert_eq!(bytes, empty_field(tag), "outcome tag {tag}");
        assert_eq!(pb::EconomyResult::decode(bytes.as_slice())?, message);
    }
    // The D53 receipt fields keep ItemOperationResult's tags 1..7; recipe_id (8) is omitted,
    // so request_key / fingerprint / accepted_client_seq sit at 8..10.
    let receipt = pb::EconomyResult {
        request_key: vec![7],
        request_fingerprint: "f".into(),
        accepted_client_seq: Some(1),
        ..Default::default()
    };
    assert_eq!(receipt.encode_to_vec(), [66, 1, 7, 74, 1, b'f', 80, 1]);
    Ok(())
}

#[test]
fn ground_item_spawn_detail_is_tag_30_and_kind_3() -> Result<(), prost::DecodeError> {
    let spawn = pb::EntitySpawn {
        ground_item: Some(pb::GroundItemDetail::default()),
        ..Default::default()
    };
    let bytes = spawn.encode_to_vec();
    assert_eq!(bytes, empty_field(30));
    assert_eq!(pb::EntitySpawn::decode(bytes.as_slice())?, spawn);
    assert_eq!(i32::from(pb::EntityKind::GroundItem), 3);
    assert_eq!(pb::EntityKind::try_from(3), Ok(pb::EntityKind::GroundItem));
    assert!(pb::EntityKind::try_from(4).is_err());
    Ok(())
}

#[test]
fn reject_reasons_38_to_88_extend_the_existing_enum() {
    use pb::RejectReason as R;
    let table = [
        (R::GroundItemNotFound, 38),
        (R::GroundItemProtected, 39),
        (R::InvalidMerchant, 40),
        (R::InvalidQuantity, 41),
        (R::VendorOutOfStock, 42),
        (R::AdenaLimit, 43),
        (R::WeightLimit, 44),
        (R::InsufficientAdena, 45),
        (R::ItemNotSellable, 46),
        (R::ItemCountCap, 47),
        (R::RecipeAlreadyRegistered, 48),
        (R::RecipeBookFull, 49),
        (R::CraftSkillMissing, 50),
        (R::CraftLevelTooLow, 51),
        (R::MissingMaterials, 52),
        (R::InCombat, 53),
        (R::NoPendingRequest, 54),
        (R::AlreadyTrading, 55),
        (R::TargetBusy, 56),
        (R::CannotTradeInStore, 57),
        (R::CannotAdjustAfterConfirm, 58),
        (R::ItemNotTradable, 59),
        (R::StoreSlotLimit, 60),
        (R::StoreNotOpen, 61),
        (R::PackageIncomplete, 62),
        (R::PriceMismatch, 63),
        (R::PartyFull, 64),
        (R::PartyAlreadyInParty, 65),
        (R::NotPartyLeader, 66),
        (R::NotPartyMember, 67),
        (R::PartyLootVoteInProgress, 68),
        (R::ContainerBusy, 69),
        (R::WarehouseFull, 70),
        (R::ItemNotDepositable, 71),
        (R::FreightTargetInvalid, 72),
        (R::ItemNotFreightable, 73),
        (R::FreightFull, 74),
        (R::PeaceZoneRequired, 75),
        (R::MailAttachmentsExceeded, 76),
        (R::MailReceiverNotFound, 77),
        (R::MailboxFull, 78),
        (R::MailNotFound, 79),
        (R::MailAlreadyTaken, 80),
        (R::SweepNotAllowed, 81),
        (R::NotSeedable, 82),
        (R::HarvestNotAllowed, 83),
        (R::ManorUnavailable, 84),
        (R::AuctionNotActive, 85),
        (R::BidTooLow, 86),
        (R::BidTooHigh, 87),
        (R::NoBidToCancel, 88),
    ];
    assert_eq!(table.len(), 51);
    for (reason, value) in table {
        assert_eq!(i32::from(reason), value);
        assert_eq!(R::try_from(value), Ok(reason));
    }
    // Reused existing reasons keep their numbers; nothing sits past 88.
    assert_eq!(i32::from(R::RateLimited), 5);
    assert_eq!(i32::from(R::DeadActor), 7);
    assert_eq!(i32::from(R::OutOfRange), 10);
    assert_eq!(i32::from(R::InsufficientItems), 28);
    assert_eq!(i32::from(R::InventoryFull), 29);
    assert_eq!(i32::from(R::ItemCapabilityDeferred), 36);
    assert_eq!(i32::from(R::CrystallizeLevelTooLow), 37);
    assert!(R::try_from(89).is_err());
}

#[test]
fn item_operations_14_to_42_follow_the_receipt_scope_order() {
    use pb::ItemOperation as O;
    let table = [
        (O::PickUpItem, 14),
        (O::VendorBuy, 15),
        (O::VendorSell, 16),
        (O::VendorRefund, 17),
        (O::MultisellChoose, 18),
        (O::RegisterRecipe, 19),
        (O::DeleteRecipe, 20),
        (O::CraftItem, 21),
        (O::TradeConfirm, 22),
        (O::StoreBuy, 23),
        (O::StoreSell, 24),
        (O::PrivateManufacture, 25),
        (O::WarehouseDeposit, 26),
        (O::WarehouseWithdraw, 27),
        (O::FreightSend, 28),
        (O::FreightWithdraw, 29),
        (O::MailSend, 30),
        (O::MailTake, 31),
        (O::MailReject, 32),
        (O::MailCancel, 33),
        (O::MailDelete, 34),
        (O::ManorBuySeed, 35),
        (O::ManorSellCrop, 36),
        (O::AuctionBid, 37),
        (O::AuctionCancelBid, 38),
        (O::Party, 39),
        (O::TradeSession, 40),
        (O::StoreSetup, 41),
        (O::MailMarkRead, 42),
    ];
    assert_eq!(table.len(), 29);
    for (operation, value) in table {
        assert_eq!(i32::from(operation), value);
        assert_eq!(O::try_from(value), Ok(operation));
    }
    assert_eq!(i32::from(O::Backfill), 13);
    assert!(O::try_from(43).is_err());
}

#[test]
fn item_locations_3_to_8_name_the_containers() {
    use pb::ItemLocation as L;
    let table = [
        (L::Warehouse, 3),
        (L::Freight, 4),
        (L::Mail, 5),
        (L::AuctionBid, 6),
        (L::Refund, 7),
        (L::Delivery, 8),
    ];
    for (location, value) in table {
        assert_eq!(i32::from(location), value);
        assert_eq!(L::try_from(value), Ok(location));
    }
    assert_eq!(i32::from(L::Paperdoll), 2);
    assert!(L::try_from(9).is_err());
}

#[test]
fn economy_enums_start_at_unspecified_and_hold_the_design_values() {
    assert_eq!(i32::from(pb::LootMode::FindersKeepers), 1);
    assert_eq!(i32::from(pb::LootMode::ByTurnIncludingSpoil), 5);
    assert_eq!(i32::from(pb::LootVotePhase::Cancelled), 3);
    assert_eq!(i32::from(pb::TradePhase::Cancelled), 5);
    assert_eq!(i32::from(pb::StoreKind::Manufacture), 5);
    assert_eq!(i32::from(pb::AuctionState::Finished), 3);
    assert_eq!(i32::from(pb::MailBox::Outbox), 2);
    assert_eq!(pb::LootMode::default(), pb::LootMode::Unspecified);
    assert_eq!(pb::MailBox::default(), pb::MailBox::Unspecified);
}

#[test]
fn additions_leave_legacy_bytes_unchanged() -> Result<(), prost::DecodeError> {
    let spawn = [10, 1, b'p', 18, 1, b'n', 32, 1];
    assert_eq!(pb::EntitySpawn::decode(spawn.as_slice())?.encode_to_vec(), spawn);
    assert_eq!(pb::EntitySpawn::default().encode_to_vec(), Vec::<u8>::new());
    let stats = [10, 1, b'p', 16, 3, 24, 5, 32, 7];
    assert_eq!(pb::StatsChanged::decode(stats.as_slice())?.encode_to_vec(), stats);
    let item = pb::OwnedItem {
        instance_id: 1,
        location: pb::ItemLocation::Inventory.into(),
        ..Default::default()
    };
    assert_eq!(item.encode_to_vec(), [8, 1, 48, 1]);
    let result = pb::ItemOperationResult {
        recipe_id: Some(2),
        ..Default::default()
    };
    assert_eq!(result.encode_to_vec(), [64, 2]);
    Ok(())
}

#[test]
fn container_views_reuse_owned_item_with_the_new_locations() -> Result<(), prost::DecodeError> {
    let response = pb::GetWarehouseResponse {
        items: vec![pb::OwnedItem {
            instance_id: u64::MAX,
            location: pb::ItemLocation::Warehouse.into(),
            ..Default::default()
        }],
        capacity: 100,
        revision: u64::MAX,
    };
    assert_eq!(pb::GetWarehouseResponse::decode(response.encode_to_vec().as_slice())?, response);
    let mail = pb::ListMailRequest {
        character_id: vec![0; 16],
        r#box: pb::MailBox::Inbox.into(),
    };
    assert_eq!(pb::ListMailRequest::decode(mail.encode_to_vec().as_slice())?, mail);
    Ok(())
}
