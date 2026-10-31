//! Phase 5 (V7) replay carriers: `PbCommandKind` 32..76, `PbPlayerLoad` 11 and `PbOutput`
//! 42..55 (51 reserved), each a canonical integer-only JSON payload of the domain contract
//! (design §12.2, §12.3). Replay tags are independent of the WebSocket intent numbers.
use super::{canonical_json, err, CodecError, PbCommandKind, PbOutputItem};
use crate::domain::zone::{
    EconomyCheckpoint, EconomyEvent, Phase5Checkpoint, Phase5Command, Phase5CommandKind,
    Phase5OutputKind, ZoneCommand, ZoneEvent,
};

pub(super) fn command_to_pb(command: &Phase5Command) -> PbCommandKind {
    let bytes = canonical_json(command);
    match command.kind() {
        Phase5CommandKind::PickUpItem => PbCommandKind::PickUpItem(bytes),
        Phase5CommandKind::VendorBuy => PbCommandKind::VendorBuy(bytes),
        Phase5CommandKind::VendorSell => PbCommandKind::VendorSell(bytes),
        Phase5CommandKind::VendorRefund => PbCommandKind::VendorRefund(bytes),
        Phase5CommandKind::MultisellChoose => PbCommandKind::MultisellChoose(bytes),
        Phase5CommandKind::RegisterRecipe => PbCommandKind::RegisterRecipe(bytes),
        Phase5CommandKind::DeleteRecipe => PbCommandKind::DeleteRecipe(bytes),
        Phase5CommandKind::CraftItem => PbCommandKind::CraftItem(bytes),
        Phase5CommandKind::TradeRequest => PbCommandKind::TradeRequest(bytes),
        Phase5CommandKind::TradeAnswer => PbCommandKind::TradeAnswer(bytes),
        Phase5CommandKind::TradeAddItem => PbCommandKind::TradeAddItem(bytes),
        Phase5CommandKind::TradeConfirm => PbCommandKind::TradeConfirm(bytes),
        Phase5CommandKind::TradeCancel => PbCommandKind::TradeCancel(bytes),
        Phase5CommandKind::SetPrivateStoreSell => PbCommandKind::SetPrivateStoreSell(bytes),
        Phase5CommandKind::SetPrivateStoreBuy => PbCommandKind::SetPrivateStoreBuy(bytes),
        Phase5CommandKind::SetPrivateManufacture => PbCommandKind::SetPrivateManufacture(bytes),
        Phase5CommandKind::ClosePrivateStore => PbCommandKind::ClosePrivateStore(bytes),
        Phase5CommandKind::PrivateStoreBuy => PbCommandKind::PrivateStoreBuy(bytes),
        Phase5CommandKind::PrivateStoreSell => PbCommandKind::PrivateStoreSell(bytes),
        Phase5CommandKind::PrivateManufactureOrder => PbCommandKind::PrivateManufactureOrder(bytes),
        Phase5CommandKind::PartyInvite => PbCommandKind::PartyInvite(bytes),
        Phase5CommandKind::PartyAnswer => PbCommandKind::PartyAnswer(bytes),
        Phase5CommandKind::PartyLeave => PbCommandKind::PartyLeave(bytes),
        Phase5CommandKind::PartyKick => PbCommandKind::PartyKick(bytes),
        Phase5CommandKind::PartyChangeLeader => PbCommandKind::PartyChangeLeader(bytes),
        Phase5CommandKind::PartyLootChangeRequest => PbCommandKind::PartyLootChangeRequest(bytes),
        Phase5CommandKind::PartyLootChangeAnswer => PbCommandKind::PartyLootChangeAnswer(bytes),
        Phase5CommandKind::WarehouseDeposit => PbCommandKind::WarehouseDeposit(bytes),
        Phase5CommandKind::WarehouseWithdraw => PbCommandKind::WarehouseWithdraw(bytes),
        Phase5CommandKind::FreightSend => PbCommandKind::FreightSend(bytes),
        Phase5CommandKind::FreightWithdraw => PbCommandKind::FreightWithdraw(bytes),
        Phase5CommandKind::MailSend => PbCommandKind::MailSend(bytes),
        Phase5CommandKind::MailTakeAttachments => PbCommandKind::MailTakeAttachments(bytes),
        Phase5CommandKind::MailReject => PbCommandKind::MailReject(bytes),
        Phase5CommandKind::MailCancel => PbCommandKind::MailCancel(bytes),
        Phase5CommandKind::MailDelete => PbCommandKind::MailDelete(bytes),
        Phase5CommandKind::ManorBuySeed => PbCommandKind::ManorBuySeed(bytes),
        Phase5CommandKind::ManorSellCrop => PbCommandKind::ManorSellCrop(bytes),
        Phase5CommandKind::ItemAuctionBid => PbCommandKind::ItemAuctionBid(bytes),
        Phase5CommandKind::ItemAuctionCancelBid => PbCommandKind::ItemAuctionCancelBid(bytes),
        Phase5CommandKind::MailMarkRead => PbCommandKind::MailMarkRead(bytes),
        Phase5CommandKind::DeliverPending => PbCommandKind::DeliverPending(bytes),
        Phase5CommandKind::AuctionSchedule => PbCommandKind::AuctionSchedule(bytes),
        Phase5CommandKind::EpochReconcile => PbCommandKind::EpochReconcile(bytes),
        Phase5CommandKind::MailExpire => PbCommandKind::MailExpire(bytes),
    }
}

/// Strict body identity: canonical bytes, the carrier's kind, structural validity.
pub(super) fn command_from_json(
    bytes: &[u8],
    expected: Phase5CommandKind,
) -> Result<ZoneCommand, CodecError> {
    let command: Phase5Command = serde_json::from_slice(bytes).map_err(err)?;
    command.validate().map_err(err)?;
    if command.kind() != expected || canonical_json(&command) != bytes {
        return Err(CodecError("noncanonical or mismatched V7 economy command".into()));
    }
    Ok(ZoneCommand::Phase5(Box::new(command)))
}

pub(super) fn event_to_pb(event: &EconomyEvent) -> PbOutputItem {
    let bytes = canonical_json(event);
    match event.body.output_kind() {
        Phase5OutputKind::EconomyResult => PbOutputItem::EconomyResult(bytes),
        Phase5OutputKind::CorpseStateChanged => PbOutputItem::CorpseStateChanged(bytes),
        Phase5OutputKind::PartyStateChanged => PbOutputItem::PartyStateChanged(bytes),
        Phase5OutputKind::PartyLootVote => PbOutputItem::PartyLootVote(bytes),
        Phase5OutputKind::PartyInviteReceived => PbOutputItem::PartyInviteReceived(bytes),
        Phase5OutputKind::TradeRequestReceived => PbOutputItem::TradeRequestReceived(bytes),
        Phase5OutputKind::TradeSessionChanged => PbOutputItem::TradeSessionChanged(bytes),
        Phase5OutputKind::PrivateStoreChanged => PbOutputItem::PrivateStoreChanged(bytes),
        Phase5OutputKind::RecipeBookSync => PbOutputItem::RecipeBookSync(bytes),
        Phase5OutputKind::ItemAuctionStatusChanged => PbOutputItem::ItemAuctionStatusChanged(bytes),
        Phase5OutputKind::LootDistributed => PbOutputItem::LootDistributed(bytes),
        // Never produced by `EconomyBody::output_kind`; kept for an exhaustive dispatch.
        Phase5OutputKind::Phase5Enriched | Phase5OutputKind::EconomyCheckpoint => {
            PbOutputItem::EconomyResult(bytes)
        },
    }
}

pub(super) fn event_from_json(
    bytes: &[u8],
    expected: Phase5OutputKind,
) -> Result<ZoneEvent, CodecError> {
    let event: EconomyEvent = serde_json::from_slice(bytes).map_err(err)?;
    event.validate().map_err(err)?;
    if event.body.output_kind() != expected || canonical_json(&event) != bytes {
        return Err(CodecError("noncanonical or mismatched V7 economy event".into()));
    }
    Ok(ZoneEvent::Economy(Box::new(event)))
}

pub(super) fn checkpoint_from_json(bytes: &[u8]) -> Result<ZoneEvent, CodecError> {
    let checkpoint: EconomyCheckpoint = serde_json::from_slice(bytes).map_err(err)?;
    checkpoint.validate().map_err(err)?;
    if canonical_json(&checkpoint) != bytes {
        return Err(CodecError("noncanonical private economy checkpoint".into()));
    }
    Ok(ZoneEvent::EconomyCheckpoint(Box::new(checkpoint)))
}

/// Design §12.3 tag 42: exactly a ground-item `EntitySpawn` carrying its detail. Players
/// and NPCs never take this carrier, so their spawns stay byte-identical to V6.
pub(super) fn enriched(event: &ZoneEvent) -> bool {
    matches!(
        event,
        ZoneEvent::EntitySpawn {
            phase5: Some(_),
            ..
        }
    )
}

pub(super) fn enriched_from_json(bytes: &[u8]) -> Result<ZoneEvent, CodecError> {
    let event: ZoneEvent = serde_json::from_slice(bytes).map_err(err)?;
    let ZoneEvent::EntitySpawn {
        phase5: Some(detail),
        phase4: None,
        identity: None,
        combat: None,
        ..
    } = &event
    else {
        return Err(err("invalid Phase5 enriched event"));
    };
    detail.validate().map_err(err)?;
    if canonical_json(&event) != bytes {
        return Err(err("noncanonical Phase5 enriched event"));
    }
    Ok(event)
}

pub(super) fn load_from_json(bytes: &[u8]) -> Result<Phase5Checkpoint, CodecError> {
    let load: Phase5Checkpoint = serde_json::from_slice(bytes).map_err(err)?;
    load.validate().map_err(err)?;
    if canonical_json(&load) != bytes {
        return Err(err("noncanonical Phase 5 admission load"));
    }
    Ok(load)
}
