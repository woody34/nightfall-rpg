//! Phase 5 (V7) command contract: the 41 WebSocket intents of design §13.2 and the four
//! system jobs of design §12.2, as the zone records them (canonical integer-only JSON in
//! the `PbCommandKind` 32..76 bytes). The owner binding is the authenticated session
//! entity; the request key is the client idempotency key (D41–D43), or the system job's
//! derived key for kinds 73..76. Lease-bound kinds gain their frozen pre-images (design
//! §10.0, §12.2 "Frozen payloads") with the container story (3.11) and its API counterpart
//! (4.3); until then they record only the client body.

use serde::{Deserialize, Serialize};

use super::{EntityId, ItemError, ItemInstanceId, Phase5CommandKind};

/// `ItemCount` of design §13.2: one owned instance and a positive count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemCount {
    pub instance: ItemInstanceId,
    pub count: u64,
}

/// `TemplateCount` of design §13.2: a catalogue template and a positive count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct TemplateCount {
    pub template: u32,
    pub count: u64,
}

/// `StoreSellEntry` of design §13.2 (also the body of a store purchase).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct StoreSellEntry {
    pub instance: ItemInstanceId,
    pub count: u64,
    pub price: u64,
}

/// `StoreBuyEntry` of design §13.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct StoreBuyEntry {
    pub template: u32,
    pub enchant: u32,
    pub count: u64,
    pub price: u64,
}

/// `ManufactureEntry` of design §13.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ManufactureEntry {
    pub recipe: u32,
    pub cost: u64,
}

/// `PrivateStoreSellEntry` of design §13.2: what a seller offers a buy store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct PrivateStoreSellEntry {
    pub instance: ItemInstanceId,
    pub template: u32,
    pub count: u64,
    pub price: u64,
}

/// `CropSale` of design §13.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct CropSale {
    pub instance: ItemInstanceId,
    pub crop: u32,
    pub manor: u32,
    pub count: u64,
}

/// Party loot modes (design §13.2 `LootMode`): the pinned `PartyDistributionType` ids in
/// order; the wire value is `pinned + 1` and `UNSPECIFIED` rejects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum LootMode {
    FindersKeepers,
    Random,
    RandomIncludingSpoil,
    ByTurn,
    ByTurnIncludingSpoil,
}

impl LootMode {
    /// Every mode in pinned id order.
    pub const ALL: [Self; 5] = [
        Self::FindersKeepers,
        Self::Random,
        Self::RandomIncludingSpoil,
        Self::ByTurn,
        Self::ByTurnIncludingSpoil,
    ];

    /// The pinned `PartyDistributionType` id (0..4).
    #[must_use]
    pub const fn pinned_id(self) -> u32 {
        self as u32
    }

    /// The wire `LootMode` value (`pinned_id + 1`; 0 is `UNSPECIFIED`).
    #[must_use]
    pub const fn wire(self) -> u32 {
        self.pinned_id().saturating_add(1)
    }

    /// Inverse of [`Self::wire`]; `None` for `UNSPECIFIED` and out-of-range values.
    #[must_use]
    pub fn from_wire(value: u32) -> Option<Self> {
        value
            .checked_sub(1)
            .and_then(|id| usize::try_from(id).ok())
            .and_then(|id| Self::ALL.get(id).copied())
    }
}

/// `AuctionSchedule` transitions of design §10.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum AuctionTransition {
    Create,
    Start,
    Finish,
    Purge,
}

/// The pinned `PartyAnswer` responses (design §13.2: `int32 answer`, pinned −1 / 0 / 1).
pub const PARTY_ANSWERS: [i32; 3] = [-1, 0, 1];
/// `mail_messages.subject` bound (design §11.4).
pub const MAIL_SUBJECT_MAX_CHARS: usize = 128;
/// `mail_messages.body` bound (design §11.4).
pub const MAIL_BODY_MAX_CHARS: usize = 512;
/// `mail_messages.cod_adena` bound (design §11.4).
pub const MAIL_COD_MAX: u64 = 99_900_000_000;

/// One recorded Phase 5 command. Field names are the design §13.2 proto names in
/// `snake_case`; entity references are domain ids, never client-supplied strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum Phase5Command {
    PickUpItem {
        entity: EntityId,
        request_key: uuid::Uuid,
        ground: EntityId,
    },
    VendorBuy {
        entity: EntityId,
        request_key: uuid::Uuid,
        merchant: EntityId,
        list_id: u64,
        items: Vec<TemplateCount>,
    },
    VendorSell {
        entity: EntityId,
        request_key: uuid::Uuid,
        merchant: EntityId,
        list_id: u64,
        items: Vec<ItemCount>,
    },
    VendorRefund {
        entity: EntityId,
        request_key: uuid::Uuid,
        merchant: EntityId,
        list_id: u64,
        refund_indices: Vec<u32>,
    },
    MultisellChoose {
        entity: EntityId,
        request_key: uuid::Uuid,
        npc: EntityId,
        list_id: u32,
        entry_id: u32,
        amount: u64,
    },
    RegisterRecipe {
        entity: EntityId,
        request_key: uuid::Uuid,
        recipe_instance: ItemInstanceId,
    },
    DeleteRecipe {
        entity: EntityId,
        request_key: uuid::Uuid,
        recipe_id: u32,
    },
    CraftItem {
        entity: EntityId,
        request_key: uuid::Uuid,
        recipe_id: u32,
    },
    TradeRequest {
        entity: EntityId,
        request_key: uuid::Uuid,
        target: EntityId,
    },
    TradeAnswer {
        entity: EntityId,
        request_key: uuid::Uuid,
        accept: bool,
    },
    TradeAddItem {
        entity: EntityId,
        request_key: uuid::Uuid,
        instance: ItemInstanceId,
        count: u64,
    },
    TradeConfirm {
        entity: EntityId,
        request_key: uuid::Uuid,
    },
    TradeCancel {
        entity: EntityId,
        request_key: uuid::Uuid,
    },
    SetPrivateStoreSell {
        entity: EntityId,
        request_key: uuid::Uuid,
        title: String,
        package_sale: bool,
        entries: Vec<StoreSellEntry>,
    },
    SetPrivateStoreBuy {
        entity: EntityId,
        request_key: uuid::Uuid,
        title: String,
        entries: Vec<StoreBuyEntry>,
    },
    SetPrivateManufacture {
        entity: EntityId,
        request_key: uuid::Uuid,
        title: String,
        entries: Vec<ManufactureEntry>,
    },
    ClosePrivateStore {
        entity: EntityId,
        request_key: uuid::Uuid,
    },
    PrivateStoreBuy {
        entity: EntityId,
        request_key: uuid::Uuid,
        store_owner: EntityId,
        entries: Vec<StoreSellEntry>,
    },
    PrivateStoreSell {
        entity: EntityId,
        request_key: uuid::Uuid,
        store_owner: EntityId,
        entries: Vec<PrivateStoreSellEntry>,
    },
    PrivateManufactureOrder {
        entity: EntityId,
        request_key: uuid::Uuid,
        store_owner: EntityId,
        recipe_id: u32,
    },
    PartyInvite {
        entity: EntityId,
        request_key: uuid::Uuid,
        target: EntityId,
        mode: LootMode,
    },
    PartyAnswer {
        entity: EntityId,
        request_key: uuid::Uuid,
        /// Pinned −1 / 0 / 1 (`PARTY_ANSWERS`).
        answer: i32,
    },
    PartyLeave {
        entity: EntityId,
        request_key: uuid::Uuid,
    },
    PartyKick {
        entity: EntityId,
        request_key: uuid::Uuid,
        member: EntityId,
    },
    PartyChangeLeader {
        entity: EntityId,
        request_key: uuid::Uuid,
        member: EntityId,
    },
    PartyLootChangeRequest {
        entity: EntityId,
        request_key: uuid::Uuid,
        mode: LootMode,
    },
    PartyLootChangeAnswer {
        entity: EntityId,
        request_key: uuid::Uuid,
        accept: bool,
    },
    WarehouseDeposit {
        entity: EntityId,
        request_key: uuid::Uuid,
        keeper: EntityId,
        entries: Vec<ItemCount>,
    },
    WarehouseWithdraw {
        entity: EntityId,
        request_key: uuid::Uuid,
        keeper: EntityId,
        entries: Vec<ItemCount>,
    },
    FreightSend {
        entity: EntityId,
        request_key: uuid::Uuid,
        keeper: EntityId,
        target_character: EntityId,
        entries: Vec<ItemCount>,
    },
    FreightWithdraw {
        entity: EntityId,
        request_key: uuid::Uuid,
        keeper: EntityId,
        entries: Vec<ItemCount>,
    },
    MailSend {
        entity: EntityId,
        request_key: uuid::Uuid,
        receiver_name: String,
        subject: String,
        body: String,
        cod_adena: u64,
        attachments: Vec<ItemCount>,
    },
    MailTakeAttachments {
        entity: EntityId,
        request_key: uuid::Uuid,
        message_id: uuid::Uuid,
    },
    MailReject {
        entity: EntityId,
        request_key: uuid::Uuid,
        message_id: uuid::Uuid,
    },
    MailCancel {
        entity: EntityId,
        request_key: uuid::Uuid,
        message_id: uuid::Uuid,
    },
    MailDelete {
        entity: EntityId,
        request_key: uuid::Uuid,
        message_ids: Vec<uuid::Uuid>,
        sent_box: bool,
    },
    ManorBuySeed {
        entity: EntityId,
        request_key: uuid::Uuid,
        manager: EntityId,
        manor_id: u32,
        seeds: Vec<TemplateCount>,
    },
    ManorSellCrop {
        entity: EntityId,
        request_key: uuid::Uuid,
        manager: EntityId,
        entries: Vec<CropSale>,
    },
    ItemAuctionBid {
        entity: EntityId,
        request_key: uuid::Uuid,
        instance_npc_id: u32,
        bid: u64,
    },
    ItemAuctionCancelBid {
        entity: EntityId,
        request_key: uuid::Uuid,
        auctioneer: EntityId,
        instance_npc_id: u32,
    },
    MailMarkRead {
        entity: EntityId,
        request_key: uuid::Uuid,
        message_id: uuid::Uuid,
    },
    /// System job: the lowest-id pending delivery of `target` (design §10.4).
    DeliverPending {
        request_key: uuid::Uuid,
        target: EntityId,
        delivery_id: uuid::Uuid,
    },
    /// System job: one fixture-scheduled auction transition (design §10.5).
    AuctionSchedule {
        request_key: uuid::Uuid,
        auction_id: u64,
        transition: AuctionTransition,
    },
    /// System job: terminal rows for an abandoned epoch (design §11.6).
    EpochReconcile {
        request_key: uuid::Uuid,
        abandoned_epoch: u64,
    },
    /// System job: expiry of one mail message (design §10.3).
    MailExpire {
        request_key: uuid::Uuid,
        message_id: uuid::Uuid,
    },
}

impl Phase5Command {
    /// The codec kind of this command.
    #[must_use]
    pub const fn kind(&self) -> Phase5CommandKind {
        match self {
            Self::PickUpItem { .. } => Phase5CommandKind::PickUpItem,
            Self::VendorBuy { .. } => Phase5CommandKind::VendorBuy,
            Self::VendorSell { .. } => Phase5CommandKind::VendorSell,
            Self::VendorRefund { .. } => Phase5CommandKind::VendorRefund,
            Self::MultisellChoose { .. } => Phase5CommandKind::MultisellChoose,
            Self::RegisterRecipe { .. } => Phase5CommandKind::RegisterRecipe,
            Self::DeleteRecipe { .. } => Phase5CommandKind::DeleteRecipe,
            Self::CraftItem { .. } => Phase5CommandKind::CraftItem,
            Self::TradeRequest { .. } => Phase5CommandKind::TradeRequest,
            Self::TradeAnswer { .. } => Phase5CommandKind::TradeAnswer,
            Self::TradeAddItem { .. } => Phase5CommandKind::TradeAddItem,
            Self::TradeConfirm { .. } => Phase5CommandKind::TradeConfirm,
            Self::TradeCancel { .. } => Phase5CommandKind::TradeCancel,
            Self::SetPrivateStoreSell { .. } => Phase5CommandKind::SetPrivateStoreSell,
            Self::SetPrivateStoreBuy { .. } => Phase5CommandKind::SetPrivateStoreBuy,
            Self::SetPrivateManufacture { .. } => Phase5CommandKind::SetPrivateManufacture,
            Self::ClosePrivateStore { .. } => Phase5CommandKind::ClosePrivateStore,
            Self::PrivateStoreBuy { .. } => Phase5CommandKind::PrivateStoreBuy,
            Self::PrivateStoreSell { .. } => Phase5CommandKind::PrivateStoreSell,
            Self::PrivateManufactureOrder { .. } => Phase5CommandKind::PrivateManufactureOrder,
            Self::PartyInvite { .. } => Phase5CommandKind::PartyInvite,
            Self::PartyAnswer { .. } => Phase5CommandKind::PartyAnswer,
            Self::PartyLeave { .. } => Phase5CommandKind::PartyLeave,
            Self::PartyKick { .. } => Phase5CommandKind::PartyKick,
            Self::PartyChangeLeader { .. } => Phase5CommandKind::PartyChangeLeader,
            Self::PartyLootChangeRequest { .. } => Phase5CommandKind::PartyLootChangeRequest,
            Self::PartyLootChangeAnswer { .. } => Phase5CommandKind::PartyLootChangeAnswer,
            Self::WarehouseDeposit { .. } => Phase5CommandKind::WarehouseDeposit,
            Self::WarehouseWithdraw { .. } => Phase5CommandKind::WarehouseWithdraw,
            Self::FreightSend { .. } => Phase5CommandKind::FreightSend,
            Self::FreightWithdraw { .. } => Phase5CommandKind::FreightWithdraw,
            Self::MailSend { .. } => Phase5CommandKind::MailSend,
            Self::MailTakeAttachments { .. } => Phase5CommandKind::MailTakeAttachments,
            Self::MailReject { .. } => Phase5CommandKind::MailReject,
            Self::MailCancel { .. } => Phase5CommandKind::MailCancel,
            Self::MailDelete { .. } => Phase5CommandKind::MailDelete,
            Self::ManorBuySeed { .. } => Phase5CommandKind::ManorBuySeed,
            Self::ManorSellCrop { .. } => Phase5CommandKind::ManorSellCrop,
            Self::ItemAuctionBid { .. } => Phase5CommandKind::ItemAuctionBid,
            Self::ItemAuctionCancelBid { .. } => Phase5CommandKind::ItemAuctionCancelBid,
            Self::MailMarkRead { .. } => Phase5CommandKind::MailMarkRead,
            Self::DeliverPending { .. } => Phase5CommandKind::DeliverPending,
            Self::AuctionSchedule { .. } => Phase5CommandKind::AuctionSchedule,
            Self::EpochReconcile { .. } => Phase5CommandKind::EpochReconcile,
            Self::MailExpire { .. } => Phase5CommandKind::MailExpire,
        }
    }

    /// The client key, or the system job's derived key.
    #[must_use]
    pub const fn request_key(&self) -> uuid::Uuid {
        match self {
            Self::PickUpItem { request_key, .. }
            | Self::VendorBuy { request_key, .. }
            | Self::VendorSell { request_key, .. }
            | Self::VendorRefund { request_key, .. }
            | Self::MultisellChoose { request_key, .. }
            | Self::RegisterRecipe { request_key, .. }
            | Self::DeleteRecipe { request_key, .. }
            | Self::CraftItem { request_key, .. }
            | Self::TradeRequest { request_key, .. }
            | Self::TradeAnswer { request_key, .. }
            | Self::TradeAddItem { request_key, .. }
            | Self::TradeConfirm { request_key, .. }
            | Self::TradeCancel { request_key, .. }
            | Self::SetPrivateStoreSell { request_key, .. }
            | Self::SetPrivateStoreBuy { request_key, .. }
            | Self::SetPrivateManufacture { request_key, .. }
            | Self::ClosePrivateStore { request_key, .. }
            | Self::PrivateStoreBuy { request_key, .. }
            | Self::PrivateStoreSell { request_key, .. }
            | Self::PrivateManufactureOrder { request_key, .. }
            | Self::PartyInvite { request_key, .. }
            | Self::PartyAnswer { request_key, .. }
            | Self::PartyLeave { request_key, .. }
            | Self::PartyKick { request_key, .. }
            | Self::PartyChangeLeader { request_key, .. }
            | Self::PartyLootChangeRequest { request_key, .. }
            | Self::PartyLootChangeAnswer { request_key, .. }
            | Self::WarehouseDeposit { request_key, .. }
            | Self::WarehouseWithdraw { request_key, .. }
            | Self::FreightSend { request_key, .. }
            | Self::FreightWithdraw { request_key, .. }
            | Self::MailSend { request_key, .. }
            | Self::MailTakeAttachments { request_key, .. }
            | Self::MailReject { request_key, .. }
            | Self::MailCancel { request_key, .. }
            | Self::MailDelete { request_key, .. }
            | Self::ManorBuySeed { request_key, .. }
            | Self::ManorSellCrop { request_key, .. }
            | Self::ItemAuctionBid { request_key, .. }
            | Self::ItemAuctionCancelBid { request_key, .. }
            | Self::MailMarkRead { request_key, .. }
            | Self::DeliverPending { request_key, .. }
            | Self::AuctionSchedule { request_key, .. }
            | Self::EpochReconcile { request_key, .. }
            | Self::MailExpire { request_key, .. } => *request_key,
        }
    }

    /// The actor for client intents, the target character for `DeliverPending`; `None`
    /// for the zone-scoped system jobs.
    #[must_use]
    pub const fn entity(&self) -> Option<EntityId> {
        match self {
            Self::PickUpItem { entity, .. }
            | Self::VendorBuy { entity, .. }
            | Self::VendorSell { entity, .. }
            | Self::VendorRefund { entity, .. }
            | Self::MultisellChoose { entity, .. }
            | Self::RegisterRecipe { entity, .. }
            | Self::DeleteRecipe { entity, .. }
            | Self::CraftItem { entity, .. }
            | Self::TradeRequest { entity, .. }
            | Self::TradeAnswer { entity, .. }
            | Self::TradeAddItem { entity, .. }
            | Self::TradeConfirm { entity, .. }
            | Self::TradeCancel { entity, .. }
            | Self::SetPrivateStoreSell { entity, .. }
            | Self::SetPrivateStoreBuy { entity, .. }
            | Self::SetPrivateManufacture { entity, .. }
            | Self::ClosePrivateStore { entity, .. }
            | Self::PrivateStoreBuy { entity, .. }
            | Self::PrivateStoreSell { entity, .. }
            | Self::PrivateManufactureOrder { entity, .. }
            | Self::PartyInvite { entity, .. }
            | Self::PartyAnswer { entity, .. }
            | Self::PartyLeave { entity, .. }
            | Self::PartyKick { entity, .. }
            | Self::PartyChangeLeader { entity, .. }
            | Self::PartyLootChangeRequest { entity, .. }
            | Self::PartyLootChangeAnswer { entity, .. }
            | Self::WarehouseDeposit { entity, .. }
            | Self::WarehouseWithdraw { entity, .. }
            | Self::FreightSend { entity, .. }
            | Self::FreightWithdraw { entity, .. }
            | Self::MailSend { entity, .. }
            | Self::MailTakeAttachments { entity, .. }
            | Self::MailReject { entity, .. }
            | Self::MailCancel { entity, .. }
            | Self::MailDelete { entity, .. }
            | Self::ManorBuySeed { entity, .. }
            | Self::ManorSellCrop { entity, .. }
            | Self::ItemAuctionBid { entity, .. }
            | Self::ItemAuctionCancelBid { entity, .. }
            | Self::MailMarkRead { entity, .. } => Some(*entity),
            Self::DeliverPending { target, .. } => Some(*target),
            Self::AuctionSchedule { .. }
            | Self::EpochReconcile { .. }
            | Self::MailExpire { .. } => None,
        }
    }

    /// Structural checks shared by the codec and admission: non-nil keys and references,
    /// positive signed-bigint counts and prices, and the design §11.4 column bounds.
    /// Gameplay checks (ownership, ranges, stock, caps) run in the zone.
    #[allow(clippy::too_many_lines)] // One arm per contract variant keeps the checks visible.
    pub fn validate(&self) -> Result<(), ItemError> {
        if self.request_key().is_nil() {
            return Err(ItemError("Phase 5 command without a request key"));
        }
        if self.entity().is_some_and(|id| id.as_uuid().is_nil()) {
            return Err(ItemError("Phase 5 command without an owner"));
        }
        match self {
            Self::PickUpItem { ground, .. } => reference(*ground),
            Self::VendorBuy {
                merchant, items, ..
            } => {
                reference(*merchant)?;
                items.iter().try_for_each(|item| count(item.count))
            },
            Self::VendorSell {
                merchant, items, ..
            } => {
                reference(*merchant)?;
                items.iter().try_for_each(|item| count(item.count))
            },
            Self::VendorRefund { merchant, .. }
            | Self::MultisellChoose { npc: merchant, .. }
            | Self::ItemAuctionCancelBid {
                auctioneer: merchant,
                ..
            } => reference(*merchant),
            Self::TradeRequest { target, .. }
            | Self::PartyInvite { target, .. }
            | Self::PartyKick { member: target, .. }
            | Self::PartyChangeLeader { member: target, .. } => reference(*target),
            Self::TradeAddItem { count: n, .. } => count(*n),
            Self::SetPrivateStoreSell { entries, .. } => entries
                .iter()
                .try_for_each(|entry| count(entry.count).and_then(|()| price(entry.price))),
            Self::SetPrivateStoreBuy { entries, .. } => entries
                .iter()
                .try_for_each(|entry| count(entry.count).and_then(|()| price(entry.price))),
            Self::SetPrivateManufacture { entries, .. } => {
                entries.iter().try_for_each(|entry| price(entry.cost))
            },
            Self::PrivateStoreBuy {
                store_owner,
                entries,
                ..
            } => {
                reference(*store_owner)?;
                entries
                    .iter()
                    .try_for_each(|entry| count(entry.count).and_then(|()| price(entry.price)))
            },
            Self::PrivateStoreSell {
                store_owner,
                entries,
                ..
            } => {
                reference(*store_owner)?;
                entries
                    .iter()
                    .try_for_each(|entry| count(entry.count).and_then(|()| price(entry.price)))
            },
            Self::PrivateManufactureOrder { store_owner, .. } => reference(*store_owner),
            Self::PartyAnswer { answer, .. } => {
                if PARTY_ANSWERS.contains(answer) {
                    Ok(())
                } else {
                    Err(ItemError("party answer outside the pinned -1/0/1"))
                }
            },
            Self::WarehouseDeposit {
                keeper, entries, ..
            }
            | Self::WarehouseWithdraw {
                keeper, entries, ..
            }
            | Self::FreightWithdraw {
                keeper, entries, ..
            } => {
                reference(*keeper)?;
                entries.iter().try_for_each(|entry| count(entry.count))
            },
            Self::FreightSend {
                keeper,
                target_character,
                entries,
                ..
            } => {
                reference(*keeper)?;
                reference(*target_character)?;
                entries.iter().try_for_each(|entry| count(entry.count))
            },
            Self::MailSend {
                receiver_name,
                subject,
                body,
                cod_adena,
                attachments,
                ..
            } => {
                if receiver_name.is_empty()
                    || subject.chars().count() > MAIL_SUBJECT_MAX_CHARS
                    || body.chars().count() > MAIL_BODY_MAX_CHARS
                    || *cod_adena > MAIL_COD_MAX
                {
                    return Err(ItemError("mail outside the design §11.4 column bounds"));
                }
                attachments.iter().try_for_each(|entry| count(entry.count))
            },
            Self::MailTakeAttachments { message_id, .. }
            | Self::MailReject { message_id, .. }
            | Self::MailCancel { message_id, .. }
            | Self::MailMarkRead { message_id, .. }
            | Self::MailExpire { message_id, .. } => message(*message_id),
            Self::MailDelete { message_ids, .. } => {
                if message_ids.is_empty() {
                    return Err(ItemError("mail delete without messages"));
                }
                message_ids.iter().try_for_each(|id| message(*id))
            },
            Self::ManorBuySeed { manager, seeds, .. } => {
                reference(*manager)?;
                seeds.iter().try_for_each(|seed| count(seed.count))
            },
            Self::ManorSellCrop {
                manager, entries, ..
            } => {
                reference(*manager)?;
                entries.iter().try_for_each(|entry| count(entry.count))
            },
            Self::ItemAuctionBid { bid, .. } => price(*bid),
            Self::DeliverPending { delivery_id, .. } => message(*delivery_id),
            Self::AuctionSchedule { auction_id, .. } => {
                if *auction_id == 0 || *auction_id > i64::MAX as u64 {
                    return Err(ItemError("auction id outside positive bigint"));
                }
                Ok(())
            },
            Self::EpochReconcile {
                abandoned_epoch, ..
            } => {
                if *abandoned_epoch > i64::MAX as u64 {
                    return Err(ItemError("abandoned epoch outside bigint"));
                }
                Ok(())
            },
            Self::RegisterRecipe { .. }
            | Self::DeleteRecipe { .. }
            | Self::CraftItem { .. }
            | Self::TradeAnswer { .. }
            | Self::TradeConfirm { .. }
            | Self::TradeCancel { .. }
            | Self::ClosePrivateStore { .. }
            | Self::PartyLeave { .. }
            | Self::PartyLootChangeRequest { .. }
            | Self::PartyLootChangeAnswer { .. } => Ok(()),
        }
    }
}

fn reference(id: EntityId) -> Result<(), ItemError> {
    if id.as_uuid().is_nil() {
        Err(ItemError("Phase 5 command names a nil entity"))
    } else {
        Ok(())
    }
}

fn message(id: uuid::Uuid) -> Result<(), ItemError> {
    if id.is_nil() {
        Err(ItemError("Phase 5 command names a nil row id"))
    } else {
        Ok(())
    }
}

const fn count(value: u64) -> Result<(), ItemError> {
    if value == 0 || value > i64::MAX as u64 {
        Err(ItemError("Phase 5 count outside positive bigint"))
    } else {
        Ok(())
    }
}

const fn price(value: u64) -> Result<(), ItemError> {
    if value > i64::MAX as u64 {
        Err(ItemError("Phase 5 price outside bigint"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod samples {
    //! One valid sample per kind, shared with the codec round-trip tests.
    use super::*;

    fn id(n: u128) -> EntityId {
        EntityId::from_uuid(uuid::Uuid::from_u128(n))
    }
    fn key(n: u128) -> uuid::Uuid {
        uuid::Uuid::from_u128(n)
    }
    fn instance(n: u64) -> ItemInstanceId {
        ItemInstanceId::new(n).unwrap()
    }

    /// Every kind of [`Phase5CommandKind::ALL`], in the same order.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn all() -> Vec<Phase5Command> {
        let entity = id(1);
        let request_key = key(2);
        let item = ItemCount {
            instance: instance(7),
            count: 3,
        };
        let template = TemplateCount {
            template: 57,
            count: 10,
        };
        vec![
            Phase5Command::PickUpItem {
                entity,
                request_key,
                ground: id(3),
            },
            Phase5Command::VendorBuy {
                entity,
                request_key,
                merchant: id(4),
                list_id: 3_000_100,
                items: vec![template],
            },
            Phase5Command::VendorSell {
                entity,
                request_key,
                merchant: id(4),
                list_id: 3_000_100,
                items: vec![item],
            },
            Phase5Command::VendorRefund {
                entity,
                request_key,
                merchant: id(4),
                list_id: 3_000_100,
                refund_indices: vec![0, 2],
            },
            Phase5Command::MultisellChoose {
                entity,
                request_key,
                npc: id(4),
                list_id: 1,
                entry_id: 2,
                amount: 5,
            },
            Phase5Command::RegisterRecipe {
                entity,
                request_key,
                recipe_instance: instance(8),
            },
            Phase5Command::DeleteRecipe {
                entity,
                request_key,
                recipe_id: 9,
            },
            Phase5Command::CraftItem {
                entity,
                request_key,
                recipe_id: 9,
            },
            Phase5Command::TradeRequest {
                entity,
                request_key,
                target: id(5),
            },
            Phase5Command::TradeAnswer {
                entity,
                request_key,
                accept: true,
            },
            Phase5Command::TradeAddItem {
                entity,
                request_key,
                instance: instance(7),
                count: 1,
            },
            Phase5Command::TradeConfirm {
                entity,
                request_key,
            },
            Phase5Command::TradeCancel {
                entity,
                request_key,
            },
            Phase5Command::SetPrivateStoreSell {
                entity,
                request_key,
                title: "wares".into(),
                package_sale: true,
                entries: vec![StoreSellEntry {
                    instance: instance(7),
                    count: 1,
                    price: 100,
                }],
            },
            Phase5Command::SetPrivateStoreBuy {
                entity,
                request_key,
                title: "buying".into(),
                entries: vec![StoreBuyEntry {
                    template: 57,
                    enchant: 0,
                    count: 1,
                    price: 100,
                }],
            },
            Phase5Command::SetPrivateManufacture {
                entity,
                request_key,
                title: "crafting".into(),
                entries: vec![ManufactureEntry {
                    recipe: 9,
                    cost: 100,
                }],
            },
            Phase5Command::ClosePrivateStore {
                entity,
                request_key,
            },
            Phase5Command::PrivateStoreBuy {
                entity,
                request_key,
                store_owner: id(5),
                entries: vec![StoreSellEntry {
                    instance: instance(7),
                    count: 1,
                    price: 100,
                }],
            },
            Phase5Command::PrivateStoreSell {
                entity,
                request_key,
                store_owner: id(5),
                entries: vec![PrivateStoreSellEntry {
                    instance: instance(7),
                    template: 57,
                    count: 1,
                    price: 100,
                }],
            },
            Phase5Command::PrivateManufactureOrder {
                entity,
                request_key,
                store_owner: id(5),
                recipe_id: 9,
            },
            Phase5Command::PartyInvite {
                entity,
                request_key,
                target: id(5),
                mode: LootMode::Random,
            },
            Phase5Command::PartyAnswer {
                entity,
                request_key,
                answer: 1,
            },
            Phase5Command::PartyLeave {
                entity,
                request_key,
            },
            Phase5Command::PartyKick {
                entity,
                request_key,
                member: id(5),
            },
            Phase5Command::PartyChangeLeader {
                entity,
                request_key,
                member: id(5),
            },
            Phase5Command::PartyLootChangeRequest {
                entity,
                request_key,
                mode: LootMode::ByTurnIncludingSpoil,
            },
            Phase5Command::PartyLootChangeAnswer {
                entity,
                request_key,
                accept: false,
            },
            Phase5Command::WarehouseDeposit {
                entity,
                request_key,
                keeper: id(6),
                entries: vec![item],
            },
            Phase5Command::WarehouseWithdraw {
                entity,
                request_key,
                keeper: id(6),
                entries: vec![item],
            },
            Phase5Command::FreightSend {
                entity,
                request_key,
                keeper: id(6),
                target_character: id(5),
                entries: vec![item],
            },
            Phase5Command::FreightWithdraw {
                entity,
                request_key,
                keeper: id(6),
                entries: vec![item],
            },
            Phase5Command::MailSend {
                entity,
                request_key,
                receiver_name: "Partner".into(),
                subject: "hello".into(),
                body: "body".into(),
                cod_adena: 1_000,
                attachments: vec![item],
            },
            Phase5Command::MailTakeAttachments {
                entity,
                request_key,
                message_id: key(10),
            },
            Phase5Command::MailReject {
                entity,
                request_key,
                message_id: key(10),
            },
            Phase5Command::MailCancel {
                entity,
                request_key,
                message_id: key(10),
            },
            Phase5Command::MailDelete {
                entity,
                request_key,
                message_ids: vec![key(10), key(11)],
                sent_box: false,
            },
            Phase5Command::ManorBuySeed {
                entity,
                request_key,
                manager: id(6),
                manor_id: 1,
                seeds: vec![template],
            },
            Phase5Command::ManorSellCrop {
                entity,
                request_key,
                manager: id(6),
                entries: vec![CropSale {
                    instance: instance(7),
                    crop: 5_000,
                    manor: 1,
                    count: 2,
                }],
            },
            Phase5Command::ItemAuctionBid {
                entity,
                request_key,
                instance_npc_id: 32_320,
                bid: 10_000,
            },
            Phase5Command::ItemAuctionCancelBid {
                entity,
                request_key,
                auctioneer: id(6),
                instance_npc_id: 32_320,
            },
            Phase5Command::MailMarkRead {
                entity,
                request_key,
                message_id: key(10),
            },
            Phase5Command::DeliverPending {
                request_key,
                target: id(5),
                delivery_id: key(12),
            },
            Phase5Command::AuctionSchedule {
                request_key,
                auction_id: 1,
                transition: AuctionTransition::Finish,
            },
            Phase5Command::EpochReconcile {
                request_key,
                abandoned_epoch: 3,
            },
            Phase5Command::MailExpire {
                request_key,
                message_id: key(10),
            },
        ]
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn samples_cover_every_kind_in_tag_order_and_validate() {
        let all = samples::all();
        assert_eq!(all.len(), Phase5CommandKind::ALL.len());
        for (command, kind) in all.iter().zip(Phase5CommandKind::ALL) {
            assert_eq!(command.kind(), kind, "{command:?}");
            command.validate().unwrap();
            let json = serde_json::to_value(command).unwrap();
            assert_eq!(json["kind"], kind.name());
            assert_eq!(
                command.entity().is_none(),
                kind.is_system() && kind != Phase5CommandKind::DeliverPending
            );
            let back: Phase5Command = serde_json::from_value(json).unwrap();
            assert_eq!(&back, command);
        }
    }

    #[test]
    fn structural_bounds_fail_closed() {
        let entity = EntityId::from_uuid(uuid::Uuid::from_u128(1));
        let request_key = uuid::Uuid::from_u128(2);
        let nil = EntityId::from_uuid(uuid::Uuid::nil());
        for bad in [
            Phase5Command::TradeConfirm {
                entity,
                request_key: uuid::Uuid::nil(),
            },
            Phase5Command::TradeConfirm {
                entity: nil,
                request_key,
            },
            Phase5Command::PickUpItem {
                entity,
                request_key,
                ground: nil,
            },
            Phase5Command::TradeAddItem {
                entity,
                request_key,
                instance: ItemInstanceId::new(1).unwrap(),
                count: 0,
            },
            Phase5Command::TradeAddItem {
                entity,
                request_key,
                instance: ItemInstanceId::new(1).unwrap(),
                count: 1 << 63,
            },
            Phase5Command::ItemAuctionBid {
                entity,
                request_key,
                instance_npc_id: 1,
                bid: 1 << 63,
            },
            Phase5Command::PartyAnswer {
                entity,
                request_key,
                answer: 2,
            },
            Phase5Command::MailSend {
                entity,
                request_key,
                receiver_name: String::new(),
                subject: String::new(),
                body: String::new(),
                cod_adena: 0,
                attachments: Vec::new(),
            },
            Phase5Command::MailSend {
                entity,
                request_key,
                receiver_name: "x".into(),
                subject: "s".repeat(129),
                body: String::new(),
                cod_adena: 0,
                attachments: Vec::new(),
            },
            Phase5Command::MailSend {
                entity,
                request_key,
                receiver_name: "x".into(),
                subject: String::new(),
                body: "b".repeat(513),
                cod_adena: 0,
                attachments: Vec::new(),
            },
            Phase5Command::MailSend {
                entity,
                request_key,
                receiver_name: "x".into(),
                subject: String::new(),
                body: String::new(),
                cod_adena: MAIL_COD_MAX + 1,
                attachments: Vec::new(),
            },
            Phase5Command::MailDelete {
                entity,
                request_key,
                message_ids: Vec::new(),
                sent_box: true,
            },
            Phase5Command::MailExpire {
                request_key,
                message_id: uuid::Uuid::nil(),
            },
            Phase5Command::AuctionSchedule {
                request_key,
                auction_id: 0,
                transition: AuctionTransition::Create,
            },
            Phase5Command::DeliverPending {
                request_key,
                target: nil,
                delivery_id: request_key,
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn loot_modes_follow_the_pinned_ids_plus_one_on_the_wire() {
        for (index, mode) in LootMode::ALL.iter().enumerate() {
            assert_eq!(mode.pinned_id() as usize, index);
            assert_eq!(mode.wire() as usize, index + 1);
            assert_eq!(LootMode::from_wire(mode.wire()), Some(*mode));
        }
        assert_eq!(LootMode::from_wire(0), None);
        assert_eq!(LootMode::from_wire(6), None);
    }

    #[test]
    fn unknown_fields_and_wrong_kinds_are_refused() {
        let json = serde_json::json!({
            "kind": "trade_confirm",
            "entity": uuid::Uuid::from_u128(1),
            "request_key": uuid::Uuid::from_u128(2),
            "frozen": {}
        });
        assert!(serde_json::from_value::<Phase5Command>(json).is_err());
        let json = serde_json::json!({
            "kind": "mail_notification",
            "entity": uuid::Uuid::from_u128(1),
            "request_key": uuid::Uuid::from_u128(2)
        });
        assert!(serde_json::from_value::<Phase5Command>(json).is_err());
    }
}
