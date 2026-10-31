//! Phase 5 (V7) replay-codec allocations: the one place the Phase 5 command kinds, output
//! carriers and player-load slot are numbered (plan §4 story 2.2; design §12.2, §12.3).
//!
//! Replay tags are independent of the WebSocket intent numbers of design §13.1, but the
//! design fixes the relation `command = client_tag + 3` for the 41 intents, so both are
//! kept here and checked by unit tests. System kinds 73..76 have no client tag.
//!
//! The vocabulary lists at the end name every V7 value that a Record ≤ 8 or Snapshot ≤ 10
//! decoder must refuse recursively (design §12.1 "Legacy"), before permissive decoding.

/// The 45 Phase 5 command kinds of design §12.2, in `PbCommandKind` tag order 32..76.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(missing_docs)]
pub enum Phase5CommandKind {
    PickUpItem,
    VendorBuy,
    VendorSell,
    VendorRefund,
    MultisellChoose,
    RegisterRecipe,
    DeleteRecipe,
    CraftItem,
    TradeRequest,
    TradeAnswer,
    TradeAddItem,
    TradeConfirm,
    TradeCancel,
    SetPrivateStoreSell,
    SetPrivateStoreBuy,
    SetPrivateManufacture,
    ClosePrivateStore,
    PrivateStoreBuy,
    PrivateStoreSell,
    PrivateManufactureOrder,
    PartyInvite,
    PartyAnswer,
    PartyLeave,
    PartyKick,
    PartyChangeLeader,
    PartyLootChangeRequest,
    PartyLootChangeAnswer,
    WarehouseDeposit,
    WarehouseWithdraw,
    FreightSend,
    FreightWithdraw,
    MailSend,
    MailTakeAttachments,
    MailReject,
    MailCancel,
    MailDelete,
    ManorBuySeed,
    ManorSellCrop,
    ItemAuctionBid,
    ItemAuctionCancelBid,
    MailMarkRead,
    /// System job (design §10.4); no client tag.
    DeliverPending,
    /// System job (design §10.5); no client tag.
    AuctionSchedule,
    /// System job (design §11.6); no client tag.
    EpochReconcile,
    /// System job (design §10.3); no client tag.
    MailExpire,
}

impl Phase5CommandKind {
    /// Every kind, in tag order.
    pub const ALL: [Self; 45] = [
        Self::PickUpItem,
        Self::VendorBuy,
        Self::VendorSell,
        Self::VendorRefund,
        Self::MultisellChoose,
        Self::RegisterRecipe,
        Self::DeleteRecipe,
        Self::CraftItem,
        Self::TradeRequest,
        Self::TradeAnswer,
        Self::TradeAddItem,
        Self::TradeConfirm,
        Self::TradeCancel,
        Self::SetPrivateStoreSell,
        Self::SetPrivateStoreBuy,
        Self::SetPrivateManufacture,
        Self::ClosePrivateStore,
        Self::PrivateStoreBuy,
        Self::PrivateStoreSell,
        Self::PrivateManufactureOrder,
        Self::PartyInvite,
        Self::PartyAnswer,
        Self::PartyLeave,
        Self::PartyKick,
        Self::PartyChangeLeader,
        Self::PartyLootChangeRequest,
        Self::PartyLootChangeAnswer,
        Self::WarehouseDeposit,
        Self::WarehouseWithdraw,
        Self::FreightSend,
        Self::FreightWithdraw,
        Self::MailSend,
        Self::MailTakeAttachments,
        Self::MailReject,
        Self::MailCancel,
        Self::MailDelete,
        Self::ManorBuySeed,
        Self::ManorSellCrop,
        Self::ItemAuctionBid,
        Self::ItemAuctionCancelBid,
        Self::MailMarkRead,
        Self::DeliverPending,
        Self::AuctionSchedule,
        Self::EpochReconcile,
        Self::MailExpire,
    ];

    /// First Phase 5 `PbCommandKind` tag (design §12.1: the Phase 4 high-water mark is 31).
    pub const FIRST_CODEC_TAG: u32 = 32;
    /// Last Phase 5 `PbCommandKind` tag.
    pub const LAST_CODEC_TAG: u32 = 76;
    /// First Phase 5 `ClientMessage.intent` tag (design §13.1).
    pub const FIRST_CLIENT_TAG: u32 = 29;
    /// Last Phase 5 `ClientMessage.intent` tag (`MailMarkRead`).
    pub const LAST_CLIENT_TAG: u32 = 69;
    /// Design §12.2: `command = client_tag + 3` for every WebSocket intent.
    pub const CLIENT_TO_CODEC_OFFSET: u32 = 3;

    /// Position in [`Self::ALL`] (0..45), which is also the tag offset from 32.
    #[must_use]
    pub const fn ordinal(self) -> u32 {
        self as u32
    }

    /// The `PbCommandKind` field tag, 32..76.
    #[must_use]
    pub const fn codec_tag(self) -> u32 {
        Self::FIRST_CODEC_TAG.saturating_add(self.ordinal())
    }

    /// The `ClientMessage.intent` tag, 29..69; `None` for the four system kinds.
    #[must_use]
    pub const fn client_tag(self) -> Option<u32> {
        if self.is_system() {
            None
        } else {
            Some(
                self.codec_tag()
                    .saturating_sub(Self::CLIENT_TO_CODEC_OFFSET),
            )
        }
    }

    /// System jobs are enqueued by the API scheduler; they carry no client body.
    #[must_use]
    pub const fn is_system(self) -> bool {
        matches!(
            self,
            Self::DeliverPending | Self::AuctionSchedule | Self::EpochReconcile | Self::MailExpire
        )
    }

    /// Inverse of [`Self::codec_tag`].
    #[must_use]
    pub fn from_codec_tag(tag: u32) -> Option<Self> {
        tag.checked_sub(Self::FIRST_CODEC_TAG)
            .and_then(|offset| usize::try_from(offset).ok())
            .and_then(|offset| Self::ALL.get(offset).copied())
    }

    /// The `snake_case` serde kind tag of the matching [`super::Phase5Command`] variant.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::PickUpItem => "pick_up_item",
            Self::VendorBuy => "vendor_buy",
            Self::VendorSell => "vendor_sell",
            Self::VendorRefund => "vendor_refund",
            Self::MultisellChoose => "multisell_choose",
            Self::RegisterRecipe => "register_recipe",
            Self::DeleteRecipe => "delete_recipe",
            Self::CraftItem => "craft_item",
            Self::TradeRequest => "trade_request",
            Self::TradeAnswer => "trade_answer",
            Self::TradeAddItem => "trade_add_item",
            Self::TradeConfirm => "trade_confirm",
            Self::TradeCancel => "trade_cancel",
            Self::SetPrivateStoreSell => "set_private_store_sell",
            Self::SetPrivateStoreBuy => "set_private_store_buy",
            Self::SetPrivateManufacture => "set_private_manufacture",
            Self::ClosePrivateStore => "close_private_store",
            Self::PrivateStoreBuy => "private_store_buy",
            Self::PrivateStoreSell => "private_store_sell",
            Self::PrivateManufactureOrder => "private_manufacture_order",
            Self::PartyInvite => "party_invite",
            Self::PartyAnswer => "party_answer",
            Self::PartyLeave => "party_leave",
            Self::PartyKick => "party_kick",
            Self::PartyChangeLeader => "party_change_leader",
            Self::PartyLootChangeRequest => "party_loot_change_request",
            Self::PartyLootChangeAnswer => "party_loot_change_answer",
            Self::WarehouseDeposit => "warehouse_deposit",
            Self::WarehouseWithdraw => "warehouse_withdraw",
            Self::FreightSend => "freight_send",
            Self::FreightWithdraw => "freight_withdraw",
            Self::MailSend => "mail_send",
            Self::MailTakeAttachments => "mail_take_attachments",
            Self::MailReject => "mail_reject",
            Self::MailCancel => "mail_cancel",
            Self::MailDelete => "mail_delete",
            Self::ManorBuySeed => "manor_buy_seed",
            Self::ManorSellCrop => "manor_sell_crop",
            Self::ItemAuctionBid => "item_auction_bid",
            Self::ItemAuctionCancelBid => "item_auction_cancel_bid",
            Self::MailMarkRead => "mail_mark_read",
            Self::DeliverPending => "deliver_pending",
            Self::AuctionSchedule => "auction_schedule",
            Self::EpochReconcile => "epoch_reconcile",
            Self::MailExpire => "mail_expire",
        }
    }
}

/// The 13 Phase 5 `PbOutput` carriers of design §12.3 (tags 42..55; 51 is reserved).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(missing_docs)]
pub enum Phase5OutputKind {
    /// D26 pattern: `EntitySpawn` with its `GroundItemDetail` sidecar (design §12.3).
    Phase5Enriched,
    EconomyResult,
    CorpseStateChanged,
    PartyStateChanged,
    PartyLootVote,
    PartyInviteReceived,
    TradeRequestReceived,
    TradeSessionChanged,
    PrivateStoreChanged,
    RecipeBookSync,
    ItemAuctionStatusChanged,
    LootDistributed,
    /// Private durable economy settlement of a tick (counterpart of `ItemCheckpoint` 40).
    EconomyCheckpoint,
}

impl Phase5OutputKind {
    /// Every carrier, in tag order.
    pub const ALL: [Self; 13] = [
        Self::Phase5Enriched,
        Self::EconomyResult,
        Self::CorpseStateChanged,
        Self::PartyStateChanged,
        Self::PartyLootVote,
        Self::PartyInviteReceived,
        Self::TradeRequestReceived,
        Self::TradeSessionChanged,
        Self::PrivateStoreChanged,
        Self::RecipeBookSync,
        Self::ItemAuctionStatusChanged,
        Self::LootDistributed,
        Self::EconomyCheckpoint,
    ];

    /// First Phase 5 `PbOutput` tag (design §12.1: the Phase 4 high-water mark is 41).
    pub const FIRST_CODEC_TAG: u32 = 42;
    /// Last Phase 5 `PbOutput` tag.
    pub const LAST_CODEC_TAG: u32 = 55;
    /// `MailNotification` (`WorldEvent` 42) is API-only and never recorded: its `PbOutput`
    /// slot stays reserved and every decoder refuses it (design §12.3).
    pub const RESERVED_CODEC_TAG: u32 = 51;

    /// The `PbOutput` field tag.
    #[must_use]
    pub const fn codec_tag(self) -> u32 {
        match self {
            Self::Phase5Enriched => 42,
            Self::EconomyResult => 43,
            Self::CorpseStateChanged => 44,
            Self::PartyStateChanged => 45,
            Self::PartyLootVote => 46,
            Self::PartyInviteReceived => 47,
            Self::TradeRequestReceived => 48,
            Self::TradeSessionChanged => 49,
            Self::PrivateStoreChanged => 50,
            Self::RecipeBookSync => 52,
            Self::ItemAuctionStatusChanged => 53,
            Self::LootDistributed => 54,
            Self::EconomyCheckpoint => 55,
        }
    }

    /// The `WorldEvent.event` tag of design §13.1 (34..45), where the carrier is a world
    /// event: `output = world_event + 9`. The enrichment and the checkpoint have none.
    #[must_use]
    pub const fn world_event_tag(self) -> Option<u32> {
        match self {
            Self::Phase5Enriched | Self::EconomyCheckpoint => None,
            Self::EconomyResult
            | Self::CorpseStateChanged
            | Self::PartyStateChanged
            | Self::PartyLootVote
            | Self::PartyInviteReceived
            | Self::TradeRequestReceived
            | Self::TradeSessionChanged
            | Self::PrivateStoreChanged
            | Self::RecipeBookSync
            | Self::ItemAuctionStatusChanged
            | Self::LootDistributed => Some(self.codec_tag().saturating_sub(9)),
        }
    }

    /// Inverse of [`Self::codec_tag`]; `None` for 51 and for every non-Phase-5 tag.
    #[must_use]
    pub fn from_codec_tag(tag: u32) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.codec_tag() == tag)
    }
}

/// `PbPlayerLoad` tag of the `Phase5Checkpoint` bytes (design §12.3).
pub const PHASE5_PLAYER_LOAD_TAG: u32 = 11;

/// First replay-codec `PbDisposition.reason` value reserved for the 51 Phase 5 rejection
/// reasons of design §13.1 (`REJECT_REASON` 38..88 on the wire). The Phase 4 codec values end
/// at 51 (`CrystallizeLevelTooLow`), so Phase 5 takes 52..102 in design order. The domain
/// variants land with the proto (story 2.1); Record ≤ 8 already refuses the whole range.
pub const PHASE5_FIRST_REJECT_REASON: i32 = 52;
/// Last replay-codec rejection value of the Phase 5 range.
pub const PHASE5_LAST_REJECT_REASON: i32 = 102;

/// Replay-codec `EntityKind` value of `ENTITY_KIND_GROUND_ITEM` (design §13.1).
pub const GROUND_ITEM_ENTITY_KIND: i32 = 3;

/// Canonical-JSON keys that only V7 payloads may carry (snapshot, command, load or event).
pub const V7_JSON_KEYS: [&str; 5] = [
    "phase5",
    "ground_item",
    "economy",
    "economy_checkpoint",
    "lease_releases",
];

/// `ItemLocation` values that exist only under V7 (design §11.2).
pub const V7_ITEM_LOCATIONS: [&str; 7] = [
    "ground",
    "refund",
    "warehouse",
    "freight",
    "mail",
    "delivery",
    "auction_bid",
];

/// `item_ledger.op` values that exist only under V7 (design §11.3).
pub const V7_LEDGER_OPS: [&str; 12] = [
    "loot",
    "transfer_out",
    "transfer_in",
    "party_split",
    "sweep",
    "harvest",
    "purchase",
    "sale_proceeds",
    "craft",
    "cod_credit",
    "auction_award",
    "manor_reward",
];

/// The 29 receipt scopes of design §11.3 (wire `ItemOperation` 14..42, same order).
pub const V7_RECEIPT_SCOPES: [&str; 29] = [
    "pick_up_item",
    "vendor_buy",
    "vendor_sell",
    "vendor_refund",
    "multisell_choose",
    "register_recipe",
    "delete_recipe",
    "craft_item",
    "trade_confirm",
    "store_buy",
    "store_sell",
    "private_manufacture",
    "warehouse_deposit",
    "warehouse_withdraw",
    "freight_send",
    "freight_withdraw",
    "mail_send",
    "mail_take",
    "mail_reject",
    "mail_cancel",
    "mail_delete",
    "manor_buy_seed",
    "manor_sell_crop",
    "auction_bid",
    "auction_cancel_bid",
    "party",
    "trade_session",
    "store_setup",
    "mail_mark_read",
];

/// Serde name of the V7 `EntityKind::GroundItem` value.
pub const V7_ENTITY_KIND: &str = "ground_item";

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn command_tags_are_32_to_76_in_order_with_client_tags_three_below() {
        let mut expected = Phase5CommandKind::FIRST_CODEC_TAG;
        for (index, kind) in Phase5CommandKind::ALL.iter().enumerate() {
            assert_eq!(kind.ordinal() as usize, index);
            assert_eq!(kind.codec_tag(), expected);
            assert_eq!(Phase5CommandKind::from_codec_tag(expected), Some(*kind));
            match kind.client_tag() {
                Some(client) => {
                    assert!(!kind.is_system());
                    assert_eq!(client + 3, expected);
                    assert!((29..=69).contains(&client));
                },
                None => assert!(kind.is_system()),
            }
            expected += 1;
        }
        assert_eq!(expected - 1, Phase5CommandKind::LAST_CODEC_TAG);
        assert_eq!(
            Phase5CommandKind::ALL
                .iter()
                .filter(|k| k.is_system())
                .count(),
            4
        );
        assert_eq!(Phase5CommandKind::PickUpItem.client_tag(), Some(29));
        assert_eq!(Phase5CommandKind::MailMarkRead.client_tag(), Some(69));
        assert_eq!(Phase5CommandKind::MailMarkRead.codec_tag(), 72);
        assert_eq!(Phase5CommandKind::DeliverPending.codec_tag(), 73);
        assert_eq!(Phase5CommandKind::MailExpire.codec_tag(), 76);
        assert_eq!(Phase5CommandKind::from_codec_tag(31), None);
        assert_eq!(Phase5CommandKind::from_codec_tag(77), None);
        let mut names: Vec<_> = Phase5CommandKind::ALL.iter().map(|k| k.name()).collect();
        names.dedup();
        assert_eq!(names.len(), 45);
    }

    #[test]
    fn output_tags_are_42_to_55_skipping_the_reserved_51() {
        let tags: Vec<u32> = Phase5OutputKind::ALL
            .iter()
            .map(|k| k.codec_tag())
            .collect();
        assert_eq!(tags, [42, 43, 44, 45, 46, 47, 48, 49, 50, 52, 53, 54, 55]);
        assert_eq!(Phase5OutputKind::from_codec_tag(51), None);
        assert_eq!(Phase5OutputKind::from_codec_tag(41), None);
        assert_eq!(Phase5OutputKind::from_codec_tag(56), None);
        for kind in Phase5OutputKind::ALL {
            assert_eq!(Phase5OutputKind::from_codec_tag(kind.codec_tag()), Some(kind));
        }
        // Design §13.1 world events 34..45 minus MailNotification (42) map to output + 9.
        let world: Vec<u32> = Phase5OutputKind::ALL
            .iter()
            .filter_map(|k| k.world_event_tag())
            .collect();
        assert_eq!(world, [34, 35, 36, 37, 38, 39, 40, 41, 43, 44, 45]);
        assert_eq!(Phase5OutputKind::EconomyResult.world_event_tag(), Some(34));
        assert_eq!(Phase5OutputKind::LootDistributed.world_event_tag(), Some(45));
    }

    #[test]
    fn vocabularies_are_distinct_and_match_the_design_counts() {
        let mut scopes = V7_RECEIPT_SCOPES.to_vec();
        scopes.sort_unstable();
        scopes.dedup();
        assert_eq!(scopes.len(), 29);
        let mut ops = V7_LEDGER_OPS.to_vec();
        ops.sort_unstable();
        ops.dedup();
        assert_eq!(ops.len(), 12);
        assert_eq!(PHASE5_LAST_REJECT_REASON - PHASE5_FIRST_REJECT_REASON + 1, 51);
        assert_eq!(PHASE5_PLAYER_LOAD_TAG, 11);
        assert_eq!(GROUND_ITEM_ENTITY_KIND, 3);
    }
}
