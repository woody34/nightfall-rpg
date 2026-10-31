//! Read-only economy port (stories 4.5 / 4.6; design §10.0 step 1, §10.3–10.5, §13.4).
//!
//! Every method reads; none writes (E31). The schedulers use it to detect due system jobs on DB
//! time and to freeze pre-images; the read RPCs use it for their views. DB time (`now()`) never
//! enters zone state: callers turn it into frozen decisions (D65, E12).

use async_trait::async_trait;
use uuid::Uuid;

use super::economy::{DeliveryRow, EconomyError, FrozenRow, LeaseKey};

/// One item row of a durable container (warehouse, freight, mail, delivery, auction escrow).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerItem {
    /// `item_instances.id`.
    pub instance_id: u64,
    /// Template.
    pub template_id: u32,
    /// Stack count.
    pub count: u64,
    /// Enchant level.
    pub enchant: u32,
    /// Augment id, if any.
    pub augment_id: Option<u32>,
    /// Charge bits.
    pub charge_bits: u32,
    /// Row revision.
    pub revision: u64,
}

/// One `mail_messages` row with the DB-time fields as epoch milliseconds (read RPCs only).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs, clippy::struct_excessive_bools)]
pub struct MailRow {
    pub id: Uuid,
    pub sender_id: Uuid,
    pub receiver_id: Uuid,
    pub sender_name: String,
    pub receiver_name: String,
    pub subject: String,
    pub body: String,
    pub cod_adena: u64,
    pub has_attachments: bool,
    pub returned: bool,
    pub system_kind: u32,
    pub read: bool,
    pub deleted_by_sender: bool,
    pub deleted_by_receiver: bool,
    pub created_unix_ms: i64,
    pub expires_unix_ms: i64,
    pub revision: u64,
}

/// One `item_auctions` row.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct AuctionRow {
    pub id: u64,
    pub instance_npc: u32,
    pub item_index: u32,
    /// 0 CREATED, 1 STARTED, 2 FINISHED (`item_auctions.state`).
    pub state: u8,
    pub highest_bidder: Option<Uuid>,
    pub highest_bid: Option<u64>,
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
    pub revision: u64,
}

/// One `item_auction_bids` row; `last_bid = -1` is a cancelled bid.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct BidRow {
    pub auction_id: u64,
    pub character_id: Uuid,
    pub last_bid: i64,
}

/// The manor rows of one castle.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ManorRows {
    /// `(seed_id, price, start_amount, amount)`.
    pub seeds: Vec<(u32, u64, u64, u64)>,
    /// `(crop_id, price, start_amount, amount, reward_type)`.
    pub crops: Vec<(u32, u64, u64, u64, u32)>,
}

/// A delivery row and whether it is still pending (the `DeliverPending` commit marker).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryState {
    /// The row.
    pub row: DeliveryRow,
    /// `status = 'pending'`.
    pub pending: bool,
}

/// The read side of the DB-only economy (E31: no method writes).
#[async_trait]
pub trait EconomyReadRepository: Send + Sync {
    /// DB time now, epoch milliseconds (Postgres `now()`; the memory adapter's clock).
    async fn db_now_ms(&self) -> Result<i64, EconomyError>;
    /// Items of `owner_kind` (`warehouse` / `freight`) owned by the character, ascending id.
    async fn container_items(
        &self,
        owner_kind: &str,
        character: Uuid,
    ) -> Result<Vec<ContainerItem>, EconomyError>;
    /// The character's inbox (`outbox = false`) or outbox, newest first; expired messages
    /// (`expires_at <= now()`) and messages deleted by that side are hidden (§10.3).
    async fn mail_list(&self, character: Uuid, outbox: bool) -> Result<Vec<MailRow>, EconomyError>;
    /// One message if it is visible to the character on either side and not expired.
    async fn mail(&self, character: Uuid, message: Uuid) -> Result<Option<MailRow>, EconomyError>;
    /// Attachment rows of a message (`location = 'mail'`), ascending id.
    async fn mail_attachments(&self, message: Uuid) -> Result<Vec<ContainerItem>, EconomyError>;
    /// Messages with `expires_at <= now()`, oldest first, at most `limit` (`MailExpire` detection).
    async fn expired_mail(&self, limit: u32) -> Result<Vec<MailRow>, EconomyError>;
    /// Whether the message row still exists (the `MailExpire` commit marker is its absence).
    async fn mail_exists(&self, message: Uuid) -> Result<bool, EconomyError>;
    /// Every auction row of the instance, ascending start.
    async fn auctions(&self, instance_npc: u32) -> Result<Vec<AuctionRow>, EconomyError>;
    /// Every bid row of the auction, ascending character.
    async fn bids(&self, auction: u64) -> Result<Vec<BidRow>, EconomyError>;
    /// The manor rows of a castle (empty for a castle without fixture rows).
    async fn manor(&self, castle: u32) -> Result<ManorRows, EconomyError>;
    /// A delivery row and its status.
    async fn delivery(&self, id: Uuid) -> Result<Option<DeliveryState>, EconomyError>;
    /// Targets with at least one pending delivery, ascending.
    async fn delivery_targets(&self) -> Result<Vec<Uuid>, EconomyError>;
    /// `(inbox, outbox)` message counts of a character (deleted sides excluded).
    async fn mailbox_counts(&self, character: Uuid) -> Result<(u32, u32), EconomyError>;
    /// Pre-images of every row covered by the keys (design §10.0 step 1): integer/string
    /// columns only, no timestamps. Read after the leases are held (READ COMMITTED, E39).
    async fn pre_image(&self, keys: &[LeaseKey]) -> Result<Vec<FrozenRow>, EconomyError>;
}
