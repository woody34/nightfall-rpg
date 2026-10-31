//! Phase 5 (V7) event, enrichment and checkpoint contracts: the eleven recorded world
//! events of design §13.2 (`PbOutput` 43..54), the `GroundItemDetail` enrichment of a
//! ground-item `EntitySpawn` (`PbOutput` 42, D26 pattern), the private `EconomyCheckpoint`
//! (`PbOutput` 55) and the `Phase5Checkpoint` player load (`PbPlayerLoad` 11).
//!
//! Every recorded body carries `tick` and an `entity` subject so observer routing has the
//! same handles as every older event; the wire mapper drops what design §13.2 omits.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::phase5_command::{
    ItemCount, LootMode, ManufactureEntry, StoreBuyEntry, StoreSellEntry, TemplateCount,
};
use super::{EntityId, ItemError, ItemInstanceId, Phase5OutputKind, Tick};

/// One protection pair of design §1.2 as the client sees it (`GroundItemDetail`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct GroundProtectionView {
    pub owner: EntityId,
    pub until: Tick,
}

/// `GroundItemDetail` of design §13.2, present on a ground-item `EntitySpawn` only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct GroundItemDetail {
    pub item_instance_id: ItemInstanceId,
    pub template_id: u32,
    pub count: u64,
    pub enchant: u32,
    /// The `ItemTable` loot-owner mechanism (design §3.7); `None` when unprotected.
    pub loot_owner: Option<GroundProtectionView>,
    /// The `DropProtection` mechanism (design §3.7); `None` when unprotected.
    pub drop_protection: Option<GroundProtectionView>,
    pub dropped_tick: Tick,
    pub herb: bool,
}

impl GroundItemDetail {
    /// Structural checks: catalogue template id, positive bigint count, non-nil owners.
    pub fn validate(&self) -> Result<(), ItemError> {
        if self.template_id == 0 || self.template_id > i32::MAX as u32 {
            return Err(ItemError("ground item template outside the catalogue range"));
        }
        if self.count == 0 || self.count > i64::MAX as u64 {
            return Err(ItemError("ground item count outside positive bigint"));
        }
        for protection in [self.loot_owner, self.drop_protection]
            .into_iter()
            .flatten()
        {
            if protection.owner.as_uuid().is_nil() {
                return Err(ItemError("ground protection without an owner"));
            }
        }
        Ok(())
    }
}

/// The 29 receipt scopes of design §11.3 (wire `ItemOperation` 14..42, same order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum EconomyScope {
    PickUpItem,
    VendorBuy,
    VendorSell,
    VendorRefund,
    MultisellChoose,
    RegisterRecipe,
    DeleteRecipe,
    CraftItem,
    TradeConfirm,
    StoreBuy,
    StoreSell,
    PrivateManufacture,
    WarehouseDeposit,
    WarehouseWithdraw,
    FreightSend,
    FreightWithdraw,
    MailSend,
    MailTake,
    MailReject,
    MailCancel,
    MailDelete,
    ManorBuySeed,
    ManorSellCrop,
    AuctionBid,
    AuctionCancelBid,
    Party,
    TradeSession,
    StoreSetup,
    MailMarkRead,
}

impl EconomyScope {
    /// Every scope in wire order.
    pub const ALL: [Self; 29] = [
        Self::PickUpItem,
        Self::VendorBuy,
        Self::VendorSell,
        Self::VendorRefund,
        Self::MultisellChoose,
        Self::RegisterRecipe,
        Self::DeleteRecipe,
        Self::CraftItem,
        Self::TradeConfirm,
        Self::StoreBuy,
        Self::StoreSell,
        Self::PrivateManufacture,
        Self::WarehouseDeposit,
        Self::WarehouseWithdraw,
        Self::FreightSend,
        Self::FreightWithdraw,
        Self::MailSend,
        Self::MailTake,
        Self::MailReject,
        Self::MailCancel,
        Self::MailDelete,
        Self::ManorBuySeed,
        Self::ManorSellCrop,
        Self::AuctionBid,
        Self::AuctionCancelBid,
        Self::Party,
        Self::TradeSession,
        Self::StoreSetup,
        Self::MailMarkRead,
    ];

    /// The wire `ItemOperation` value, 14..42.
    #[must_use]
    pub const fn wire(self) -> u32 {
        (self as u32).saturating_add(14)
    }

    /// Scopes whose receipts freeze a result but never write ledger rows (D28).
    #[must_use]
    pub const fn writes_ledger(self) -> bool {
        !matches!(
            self,
            Self::Party
                | Self::TradeSession
                | Self::StoreSetup
                | Self::DeleteRecipe
                | Self::MailDelete
                | Self::MailMarkRead
        )
    }
}

/// `ContainerOutcome.container` of design §13.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum EconomyContainer {
    Warehouse,
    Freight,
}

/// The `EconomyResult.outcome` oneof of design §13.2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum EconomyOutcome {
    Pickup {
        template_id: u32,
        count: u64,
        recipient: EntityId,
        herb: bool,
        split: bool,
    },
    Vendor {
        adena_delta_abs: u64,
        adena_paid: bool,
        refund_order: Vec<u32>,
    },
    Craft {
        success: bool,
        masterwork: bool,
        product_template: u32,
        count: u64,
        fee: u64,
    },
    Multisell {
        list_id: u32,
        entry_id: u32,
        amount: u64,
    },
    Trade {
        partner: EntityId,
        given: Vec<ItemCount>,
        received: Vec<ItemCount>,
    },
    Store {
        counterparty: EntityId,
        items: Vec<ItemCount>,
        adena: u64,
        skipped: Vec<u32>,
    },
    Container {
        container: EconomyContainer,
        fee: u64,
        moved: Vec<ItemCount>,
    },
    Mail {
        message_id: uuid::Uuid,
        fee: u64,
        cod_paid: u64,
        processed: u32,
    },
    Manor {
        granted: Vec<TemplateCount>,
        adena: u64,
        fee: u64,
    },
    Auction {
        auction_id: u64,
        bid: u64,
        paid: u64,
        refunded: u64,
        reserve_not_met: bool,
    },
    Party {
        party_id: u64,
    },
    Recipe {
        recipe_id: u32,
        dwarven: bool,
    },
}

/// `PartyMember` of design §13.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct PartyMemberView {
    pub entity: EntityId,
    pub level: u32,
}

/// `LootVotePhase` of design §13.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum LootVotePhase {
    Requested,
    Applied,
    Cancelled,
}

/// `TradeEntry` of design §13.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct TradeEntry {
    pub instance: ItemInstanceId,
    pub template: u32,
    pub count: u64,
    pub enchant: u32,
}

/// `TradePhase` of design §13.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum TradePhase {
    Started,
    ItemAdded,
    PartnerConfirmed,
    Completed,
    Cancelled,
}

/// `StoreKind` of design §13.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum StoreKind {
    None,
    Sell,
    PackageSell,
    Buy,
    Manufacture,
}

/// `AuctionState` of design §13.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum AuctionState {
    Created,
    Started,
    Finished,
}

/// The recorded body of one Phase 5 world event (design §13.2), by `PbOutput` carrier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum EconomyBody {
    /// Owner-only frozen receipt; always an accept (rejections are `IntentRejected`).
    EconomyResult {
        incarnation: u32,
        inventory_revision: u64,
        receipt_id: String,
        operation: EconomyScope,
        affected_instance_ids: Vec<ItemInstanceId>,
        request_key: uuid::Uuid,
        request_fingerprint: String,
        accepted_client_seq: Option<u64>,
        outcome: EconomyOutcome,
    },
    /// Public; the subject is the NPC.
    CorpseStateChanged {
        spoiled: bool,
        seeded: bool,
        sweepable: bool,
        swept: bool,
    },
    /// Party members; the subject is the leader (`members[0]`, design §1.11).
    PartyStateChanged {
        party_id: u64,
        members: Vec<PartyMemberView>,
        mode: LootMode,
        baseline: bool,
        disbanded: bool,
    },
    /// Party members; the subject is the leader.
    PartyLootVote {
        party_id: u64,
        mode: LootMode,
        phase: LootVotePhase,
        expires_tick: Tick,
    },
    /// The subject is the invited target.
    PartyInviteReceived {
        requester: EntityId,
        mode: LootMode,
        expires_tick: Tick,
    },
    /// The subject is the requested target.
    TradeRequestReceived {
        requester: EntityId,
        expires_tick: Tick,
    },
    /// One copy per participant; the subject is the participant it addresses.
    TradeSessionChanged {
        partner: EntityId,
        phase: TradePhase,
        own: Vec<TradeEntry>,
        partner_items: Vec<TradeEntry>,
        own_confirmed: bool,
        partner_confirmed: bool,
        cancel_reason: String,
    },
    /// AOI; the subject is the store owner.
    PrivateStoreChanged {
        kind: StoreKind,
        title: String,
        sell: Vec<StoreSellEntry>,
        buy: Vec<StoreBuyEntry>,
        manufacture: Vec<ManufactureEntry>,
    },
    /// Owner baseline or delta; the subject is the owner.
    RecipeBookSync {
        dwarven: Vec<u32>,
        common: Vec<u32>,
        dwarven_limit: u32,
        common_limit: u32,
        baseline: bool,
        incarnation: u32,
    },
    /// Bidders and the auctioneer's AOI; the subject is the auctioneer NPC.
    ItemAuctionStatusChanged {
        instance_npc_id: u32,
        auction_id: u64,
        template_id: u32,
        highest_bid: u64,
        state: AuctionState,
        init_bid: u64,
        extended: bool,
    },
    /// Party members except the recipient; the subject is the picker.
    LootDistributed {
        recipient: EntityId,
        template_id: u32,
        count: u64,
        mode: LootMode,
        spoil: bool,
    },
}

impl EconomyBody {
    /// The `PbOutput` carrier of this body.
    #[must_use]
    pub const fn output_kind(&self) -> Phase5OutputKind {
        match self {
            Self::EconomyResult { .. } => Phase5OutputKind::EconomyResult,
            Self::CorpseStateChanged { .. } => Phase5OutputKind::CorpseStateChanged,
            Self::PartyStateChanged { .. } => Phase5OutputKind::PartyStateChanged,
            Self::PartyLootVote { .. } => Phase5OutputKind::PartyLootVote,
            Self::PartyInviteReceived { .. } => Phase5OutputKind::PartyInviteReceived,
            Self::TradeRequestReceived { .. } => Phase5OutputKind::TradeRequestReceived,
            Self::TradeSessionChanged { .. } => Phase5OutputKind::TradeSessionChanged,
            Self::PrivateStoreChanged { .. } => Phase5OutputKind::PrivateStoreChanged,
            Self::RecipeBookSync { .. } => Phase5OutputKind::RecipeBookSync,
            Self::ItemAuctionStatusChanged { .. } => Phase5OutputKind::ItemAuctionStatusChanged,
            Self::LootDistributed { .. } => Phase5OutputKind::LootDistributed,
        }
    }
}

/// One recorded Phase 5 world event: when, whom it is about, and the design §13.2 body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EconomyEvent {
    /// When.
    pub tick: Tick,
    /// The subject, as documented per [`EconomyBody`] variant.
    pub entity: EntityId,
    /// The body.
    pub body: EconomyBody,
}

impl EconomyEvent {
    /// Structural checks: a live subject and non-nil references, positive counts.
    pub fn validate(&self) -> Result<(), ItemError> {
        if self.entity.as_uuid().is_nil() {
            return Err(ItemError("economy event without a subject"));
        }
        match &self.body {
            EconomyBody::EconomyResult {
                incarnation,
                request_key,
                receipt_id,
                ..
            } => {
                if *incarnation == 0 || request_key.is_nil() || receipt_id.is_empty() {
                    return Err(ItemError("economy result without a live receipt identity"));
                }
                Ok(())
            },
            EconomyBody::PartyStateChanged {
                members, disbanded, ..
            } => {
                if !*disbanded
                    && (members.len() < 2
                        || members.len() > 9
                        || members
                            .first()
                            .is_none_or(|leader| leader.entity != self.entity))
                {
                    return Err(ItemError("party state outside the design §1.11 invariants"));
                }
                Ok(())
            },
            EconomyBody::PartyInviteReceived { requester, .. }
            | EconomyBody::TradeRequestReceived { requester, .. } => {
                if requester.as_uuid().is_nil() || *requester == self.entity {
                    return Err(ItemError("request without a distinct requester"));
                }
                Ok(())
            },
            EconomyBody::TradeSessionChanged { partner, .. } => {
                if partner.as_uuid().is_nil() || *partner == self.entity {
                    return Err(ItemError("trade without a distinct partner"));
                }
                Ok(())
            },
            EconomyBody::RecipeBookSync { incarnation, .. } => {
                if *incarnation == 0 {
                    return Err(ItemError("recipe book sync without a live incarnation"));
                }
                Ok(())
            },
            EconomyBody::LootDistributed {
                recipient, count, ..
            } => {
                if recipient.as_uuid().is_nil() || *count == 0 {
                    return Err(ItemError("loot distribution without a recipient or count"));
                }
                Ok(())
            },
            EconomyBody::CorpseStateChanged { .. }
            | EconomyBody::PartyLootVote { .. }
            | EconomyBody::PrivateStoreChanged { .. }
            | EconomyBody::ItemAuctionStatusChanged { .. } => Ok(()),
        }
    }
}

/// The lease key kinds of design §10.0, named as the `economy_leases.owner_kind` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum LeaseKind {
    Auction,
    Delivery,
    Freight,
    Mail,
    Mailbox,
    Manor,
    Warehouse,
}

impl LeaseKind {
    /// The `owner_kind` column text.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Auction => "auction",
            Self::Delivery => "delivery",
            Self::Freight => "freight",
            Self::Mail => "mail",
            Self::Mailbox => "mailbox",
            Self::Manor => "manor",
            Self::Warehouse => "warehouse",
        }
    }
}

/// One container lease key (design §10.0, E42): `<owner_kind>:<39-digit owner id>`,
/// compared bytewise on that text. UUID owners use their u128 value; castle and auction
/// ids use the same 39-digit form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseKey {
    /// The container kind.
    pub kind: LeaseKind,
    /// The owner id as an unsigned 128-bit value.
    pub owner: u128,
}

impl LeaseKey {
    /// A key over a UUID-identified owner (character, message, delivery).
    #[must_use]
    pub const fn of_uuid(kind: LeaseKind, id: uuid::Uuid) -> Self {
        Self {
            kind,
            owner: id.as_u128(),
        }
    }

    /// The canonical text both Rust and `COLLATE "C"` order bytewise.
    #[must_use]
    pub fn canonical(self) -> String {
        format!("{}:{:039}", self.kind.name(), self.owner)
    }
}

impl PartialOrd for LeaseKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LeaseKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Bytewise on the canonical text: `mail:` sorts before `mailbox:` because ':' < 'b'.
        self.canonical()
            .as_bytes()
            .cmp(other.canonical().as_bytes())
    }
}

/// Private durable economy settlement of one tick (`PbOutput` 55; design §12.3). The
/// container deltas and delivery changes (story 3.11) and the owner-kind ledger rows
/// (story 3.12) extend this record; the lease release of every executed command is the
/// recovery contract of design §10.0 step 2 and is present from the boundary on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EconomyCheckpoint {
    /// The tick whose checkpoint transaction carries these rows.
    pub tick: Tick,
    /// Every lease the tick's commands release, in bytewise key order and without
    /// duplicates (one whole set per command, design §10.0).
    pub lease_releases: Vec<LeaseKey>,
}

impl EconomyCheckpoint {
    /// Structural checks: releases are strictly ascending (sorted, deduplicated).
    pub fn validate(&self) -> Result<(), ItemError> {
        if self
            .lease_releases
            .windows(2)
            .any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a >= b))
        {
            return Err(ItemError("lease releases out of canonical order"));
        }
        Ok(())
    }
}

/// The recipe book: one home (the zone plus `characters.phase5_checkpoint`), design §1.8.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeBook {
    /// Registered Dwarven recipe ids.
    pub dwarven: BTreeSet<u32>,
    /// Registered Common recipe ids.
    pub common: BTreeSet<u32>,
}

/// `characters.phase5_checkpoint` and the `PbPlayerLoad` tag 11 payload (design §1.11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phase5Checkpoint {
    /// Always [`Self::SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Lowercase hex SHA-256 of the `Phase5Rules` the book was validated against.
    pub rules_hash: String,
    /// The recipe book.
    pub recipes: RecipeBook,
}

impl Phase5Checkpoint {
    /// The only checkpoint schema.
    pub const SCHEMA_VERSION: u32 = 1;

    /// Structural checks: the schema and a well-formed rules hash. Rules agreement is
    /// checked against the epoch's `Phase5Rules` at admission.
    pub fn validate(&self) -> Result<(), ItemError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(ItemError("unsupported Phase 5 checkpoint schema"));
        }
        if self.rules_hash.len() != 64
            || !self
                .rules_hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ItemError("Phase 5 checkpoint rules hash is not lowercase sha256 hex"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod samples {
    //! One valid sample per carrier, shared with the codec round-trip tests.
    use super::*;

    fn id(n: u128) -> EntityId {
        EntityId::from_uuid(uuid::Uuid::from_u128(n))
    }

    /// One event per world-event carrier, in [`Phase5OutputKind::ALL`] order (43..54).
    #[allow(clippy::too_many_lines)]
    pub(crate) fn events() -> Vec<EconomyEvent> {
        let tick = Tick(40);
        let entity = id(1);
        let item = ItemCount {
            instance: ItemInstanceId::new(7).unwrap(),
            count: 2,
        };
        let bodies = vec![
            EconomyBody::EconomyResult {
                incarnation: 1,
                inventory_revision: 9,
                receipt_id: "receipt:pick_up_item:00000000-0000-0000-0000-000000000002".into(),
                operation: EconomyScope::PickUpItem,
                affected_instance_ids: vec![ItemInstanceId::new(7).unwrap()],
                request_key: uuid::Uuid::from_u128(2),
                request_fingerprint: "fp".into(),
                accepted_client_seq: Some(3),
                outcome: EconomyOutcome::Pickup {
                    template_id: 57,
                    count: 2,
                    recipient: entity,
                    herb: false,
                    split: false,
                },
            },
            EconomyBody::CorpseStateChanged {
                spoiled: true,
                seeded: false,
                sweepable: true,
                swept: false,
            },
            EconomyBody::PartyStateChanged {
                party_id: 1,
                members: vec![
                    PartyMemberView { entity, level: 20 },
                    PartyMemberView {
                        entity: id(5),
                        level: 21,
                    },
                ],
                mode: LootMode::FindersKeepers,
                baseline: true,
                disbanded: false,
            },
            EconomyBody::PartyLootVote {
                party_id: 1,
                mode: LootMode::Random,
                phase: LootVotePhase::Requested,
                expires_tick: Tick(340),
            },
            EconomyBody::PartyInviteReceived {
                requester: id(5),
                mode: LootMode::ByTurn,
                expires_tick: Tick(340),
            },
            EconomyBody::TradeRequestReceived {
                requester: id(5),
                expires_tick: Tick(340),
            },
            EconomyBody::TradeSessionChanged {
                partner: id(5),
                phase: TradePhase::ItemAdded,
                own: vec![TradeEntry {
                    instance: ItemInstanceId::new(7).unwrap(),
                    template: 57,
                    count: 2,
                    enchant: 0,
                }],
                partner_items: Vec::new(),
                own_confirmed: false,
                partner_confirmed: false,
                cancel_reason: String::new(),
            },
            EconomyBody::PrivateStoreChanged {
                kind: StoreKind::Sell,
                title: "wares".into(),
                sell: vec![StoreSellEntry {
                    instance: ItemInstanceId::new(7).unwrap(),
                    count: 1,
                    price: 100,
                }],
                buy: Vec::new(),
                manufacture: Vec::new(),
            },
            EconomyBody::RecipeBookSync {
                dwarven: vec![9],
                common: Vec::new(),
                dwarven_limit: 50,
                common_limit: 50,
                baseline: true,
                incarnation: 1,
            },
            EconomyBody::ItemAuctionStatusChanged {
                instance_npc_id: 32_320,
                auction_id: 1,
                template_id: 9575,
                highest_bid: 10_000,
                state: AuctionState::Started,
                init_bid: 5_000,
                extended: false,
            },
            EconomyBody::LootDistributed {
                recipient: id(5),
                template_id: 57,
                count: item.count,
                mode: LootMode::Random,
                spoil: false,
            },
        ];
        bodies
            .into_iter()
            .map(|body| EconomyEvent { tick, entity, body })
            .collect()
    }

    pub(crate) fn ground_item() -> GroundItemDetail {
        GroundItemDetail {
            item_instance_id: ItemInstanceId::new(7).unwrap(),
            template_id: 57,
            count: 120,
            enchant: 0,
            loot_owner: Some(GroundProtectionView {
                owner: id(1),
                until: Tick(190),
            }),
            drop_protection: Some(GroundProtectionView {
                owner: id(1),
                until: Tick(190),
            }),
            dropped_tick: Tick(40),
            herb: false,
        }
    }

    pub(crate) fn checkpoint() -> EconomyCheckpoint {
        EconomyCheckpoint {
            tick: Tick(40),
            lease_releases: vec![
                LeaseKey::of_uuid(LeaseKind::Mail, uuid::Uuid::from_u128(10)),
                LeaseKey::of_uuid(LeaseKind::Mailbox, uuid::Uuid::from_u128(1)),
                LeaseKey::of_uuid(LeaseKind::Mailbox, uuid::Uuid::from_u128(5)),
            ],
        }
    }

    pub(crate) fn player_load() -> Phase5Checkpoint {
        Phase5Checkpoint {
            schema_version: 1,
            rules_hash: "4386eed8ca269e85885dabaf9bae02c5327cf425013221dfbf7df962136201f6".into(),
            recipes: RecipeBook {
                dwarven: [9].into_iter().collect(),
                common: BTreeSet::new(),
            },
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn samples_cover_every_world_event_carrier_in_order_and_validate() {
        let events = samples::events();
        let carriers: Vec<_> = Phase5OutputKind::ALL
            .iter()
            .copied()
            .filter(|k| k.world_event_tag().is_some())
            .collect();
        assert_eq!(events.len(), carriers.len());
        for (event, kind) in events.iter().zip(carriers) {
            assert_eq!(event.body.output_kind(), kind);
            event.validate().unwrap();
            let json = serde_json::to_value(event).unwrap();
            let back: EconomyEvent = serde_json::from_value(json).unwrap();
            assert_eq!(&back, event);
        }
        samples::ground_item().validate().unwrap();
        samples::checkpoint().validate().unwrap();
        samples::player_load().validate().unwrap();
    }

    #[test]
    fn economy_scopes_take_wire_values_14_to_42() {
        let wire: Vec<u32> = EconomyScope::ALL.iter().map(|s| s.wire()).collect();
        assert_eq!(wire, (14..=42).collect::<Vec<_>>());
        assert_eq!(
            EconomyScope::ALL
                .iter()
                .filter(|s| !s.writes_ledger())
                .count(),
            6,
            "D28: party, trade_session, store_setup, delete_recipe, mail_delete, mail_mark_read"
        );
    }

    #[test]
    fn lease_keys_order_bytewise_on_their_canonical_text() {
        let a = LeaseKey {
            kind: LeaseKind::Mail,
            owner: u128::MAX,
        };
        let b = LeaseKey {
            kind: LeaseKind::Mailbox,
            owner: 0,
        };
        assert_eq!(a.canonical().len(), "mail:".len() + 39);
        assert_eq!(b.canonical(), format!("mailbox:{}", "0".repeat(39)));
        assert!(a < b, "mail: sorts before mailbox: because ':' < 'b'");
        let kinds = [
            LeaseKind::Auction,
            LeaseKind::Delivery,
            LeaseKind::Freight,
            LeaseKind::Mail,
            LeaseKind::Mailbox,
            LeaseKind::Manor,
            LeaseKind::Warehouse,
        ];
        for pair in kinds.windows(2) {
            assert!(
                LeaseKey {
                    kind: pair[0],
                    owner: u128::MAX
                } < LeaseKey {
                    kind: pair[1],
                    owner: 0
                }
            );
        }
        // Within a kind, numeric order equals text order thanks to the zero padding.
        assert!(
            LeaseKey {
                kind: LeaseKind::Manor,
                owner: 9
            } < LeaseKey {
                kind: LeaseKind::Manor,
                owner: 10
            }
        );
        let mut unsorted = EconomyCheckpoint {
            tick: Tick(1),
            lease_releases: vec![b, a],
        };
        assert!(unsorted.validate().is_err());
        unsorted.lease_releases.sort();
        unsorted.validate().unwrap();
        unsorted.lease_releases.push(b);
        assert!(unsorted.validate().is_err(), "duplicates are refused");
    }

    #[test]
    fn structural_bounds_fail_closed() {
        let mut events = samples::events();
        events[0].entity = EntityId::from_uuid(uuid::Uuid::nil());
        assert!(events[0].validate().is_err());
        let mut party = samples::events()[2].clone();
        if let EconomyBody::PartyStateChanged { members, .. } = &mut party.body {
            members.truncate(1);
        }
        assert!(party.validate().is_err());
        let mut invite = samples::events()[4].clone();
        if let EconomyBody::PartyInviteReceived { requester, .. } = &mut invite.body {
            *requester = invite.entity;
        }
        assert!(invite.validate().is_err());
        let mut ground = samples::ground_item();
        ground.count = 0;
        assert!(ground.validate().is_err());
        let mut load = samples::player_load();
        load.schema_version = 2;
        assert!(load.validate().is_err());
        let mut load = samples::player_load();
        load.rules_hash.make_ascii_uppercase();
        assert!(load.validate().is_err());
        let json = serde_json::json!({
            "tick": 1,
            "entity": uuid::Uuid::from_u128(1),
            "body": {
                "event": "corpse_state_changed",
                "spoiled": false, "seeded": false, "sweepable": false, "swept": false,
                "frozen": 1
            }
        });
        assert!(serde_json::from_value::<EconomyEvent>(json).is_err());
    }
}
