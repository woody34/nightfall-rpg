//! Phase 5 container economy ports (stories 4.3 and 4.4; design §10.0, §10.4, §11.5, B8).
//!
//! The zone never reads the database (E12). The API preflight derives a command's **lease set**,
//! acquires it all-or-nothing in bytewise key order (the lease rows are the only lock, E39), reads
//! the pre-images, freezes them into the command, and hands the command to the zone. The zone's
//! checkpoint transaction then applies every `ContainerDelta` as a compare-and-set on `revision` in
//! the same key order, deletes the leases in that order, writes the owner-kind ledger rows and the
//! outbox facts, all in one transaction (§11.5). The critical-mutation order of D134 is the zone
//! actor's: the recording index is written after the log ack and before this checkpoint; these ports
//! only persist what the acknowledged draft says.
//!
//! Error classes (design §10.0 step 3, E39): a held key is `Busy` (retryable, nothing inserted);
//! Postgres `40P01`/`40001` are `Transient` (preflight → `CONTAINER_BUSY`, checkpoint → retry at
//! the next cadence); a CAS miss under a held lease is the permanent `economy_lease_violation`
//! fence; database integrity violations are permanent constraints.

use std::collections::BTreeSet;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use crate::domain::zone::phase5_event::{LeaseKey, LeaseKind};
use crate::domain::zone::phase5_wire::Phase5CommandKind;

/// `UUIDv5` namespace of system-job request keys (design §11.4 `economy_leases.request_key`).
pub const NS_SYSJOB: Uuid = Uuid::from_u128(0x6e69_6768_7466_616c_6c2d_7379_736a_6f62);
/// `UUIDv5` namespace of `auction_bid` owner ids (design §11.1).
pub const NS_AUCTION: Uuid = Uuid::from_u128(0x6e69_6768_7466_616c_6c2d_6175_6374_696f);
/// `UUIDv5` namespace of derived mail message ids (design §10.3 `NS_MAIL`).
pub const NS_MAIL: Uuid = Uuid::from_u128(0x6e69_6768_7466_616c_6c2d_6d61_696c_0000);
/// `UUIDv5` namespace of derived delivery ids (design §10.4 `NS_DELIVERY`).
pub const NS_DELIVERY: Uuid = Uuid::from_u128(0x6e69_6768_7466_616c_6c2d_6465_6c69_7665);
/// `UUIDv5` namespace of ledger transfer links (design §11.2 `NS_XFER`).
pub const NS_XFER: Uuid = Uuid::from_u128(0x6e69_6768_7466_616c_6c2d_7866_6572_0000);

/// A row whose id the API derives from the request key (design §10.0, E12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivedRow {
    /// A mail message (`NS_MAIL`).
    Message,
    /// A delivery (`NS_DELIVERY`).
    Delivery,
    /// A ledger transfer link (`NS_XFER`).
    Transfer,
}

impl DerivedRow {
    fn namespace(self) -> &'static Uuid {
        match self {
            Self::Message => &NS_MAIL,
            Self::Delivery => &NS_DELIVERY,
            Self::Transfer => &NS_XFER,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Delivery => "delivery",
            Self::Transfer => "transfer",
        }
    }
}

/// Refuses a lease key list that is not strictly ascending in the canonical bytewise order
/// (design §10.0): all-or-nothing acquisition is deadlock-free only in that order, and
/// `LeaseGrant.keys` is a plain `Vec` any caller can fill (c-review-4 P2-2).
pub fn canonical_keys(keys: &[LeaseKey]) -> Result<(), EconomyError> {
    if keys
        .windows(2)
        .any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a >= b))
    {
        return Err(EconomyError::Constraint("lease set out of canonical order".into()));
    }
    Ok(())
}

/// Why a lease or checkpoint call failed.
#[derive(Debug, thiserror::Error)]
pub enum EconomyError {
    /// At least one key of the set is held; nothing was inserted (`CONTAINER_BUSY`, retryable).
    #[error("container busy: {0:?}")]
    Busy(Vec<LeaseKey>),
    /// Postgres `40P01` (deadlock) or `40001` (serialization): retry, never a fence (E39).
    #[error("transient database failure {0}")]
    Transient(String),
    /// A compare-and-set missed under a held lease: the permanent `economy_lease_violation` fence.
    #[error("economy lease violation on {0:?}")]
    LeaseViolation(LeaseKey),
    /// A named database constraint refused the write (permanent).
    #[error("economy checkpoint violates constraint {0}")]
    Constraint(String),
    /// Anything else.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl EconomyError {
    /// `true` for the retryable classes (busy and transient).
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Busy(_) | Self::Transient(_))
    }
}

/// One acquired lease set: who holds which keys for which command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseGrant {
    /// Zone and epoch of the executing zone (recovery releases per `(zone, epoch)`).
    pub zone_id: u32,
    /// The epoch.
    pub epoch: u64,
    /// The authenticated account of a client intent; `None` for system jobs.
    pub account_id: Option<Uuid>,
    /// The command kind name (`economy_leases.operation`).
    pub operation: String,
    /// The client request key, or the derived system-job key.
    pub request_key: Uuid,
    /// The whole set, ascending and deduplicated.
    pub keys: Vec<LeaseKey>,
}

/// The lease set of one command: deduplicated and in bytewise key order (design §10.0 "Key order").
#[must_use]
pub fn lease_set(keys: impl IntoIterator<Item = LeaseKey>) -> Vec<LeaseKey> {
    keys.into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The pinned mailbox-size guard of E34 (`GC/character.properties` `MailInboxSize` = 240).
pub const MAILBOX_LIMIT: u32 = 240;

/// Per-command lease sets of design §10.0 (E38 table). Roles never decide order: every set is
/// sorted bytewise, so `MailSend(sender, receiver)` and `MailSend(receiver, sender)` acquire in the
/// same order.
pub mod sets {
    use super::{lease_set, LeaseKey, LeaseKind, Uuid};

    fn c(kind: LeaseKind, character: Uuid) -> LeaseKey {
        LeaseKey::of_uuid(kind, character)
    }

    /// `warehouse:c_actor` for deposit and withdraw.
    #[must_use]
    pub fn warehouse(actor: Uuid) -> Vec<LeaseKey> {
        lease_set([c(LeaseKind::Warehouse, actor)])
    }
    /// `freight:c_target` (`FreightSend`).
    #[must_use]
    pub fn freight_send(target: Uuid) -> Vec<LeaseKey> {
        lease_set([c(LeaseKind::Freight, target)])
    }
    /// `freight:c_actor` (`FreightWithdraw`).
    #[must_use]
    pub fn freight_withdraw(actor: Uuid) -> Vec<LeaseKey> {
        lease_set([c(LeaseKind::Freight, actor)])
    }
    /// `mailbox:{c_sender, c_receiver}` (`MailSend`); the new message is inserted under the receiver's key.
    #[must_use]
    pub fn mail_send(sender: Uuid, receiver: Uuid) -> Vec<LeaseKey> {
        lease_set([
            c(LeaseKind::Mailbox, sender),
            c(LeaseKind::Mailbox, receiver),
        ])
    }
    /// `delivery:d` (COD only) and `mail:m` (`MailTakeAttachments`).
    #[must_use]
    pub fn mail_take(message: Uuid, cod_delivery: Option<Uuid>) -> Vec<LeaseKey> {
        lease_set(
            std::iter::once(c(LeaseKind::Mail, message))
                .chain(cod_delivery.map(|d| c(LeaseKind::Delivery, d))),
        )
    }
    /// `mail:m` (`MailMarkRead`).
    #[must_use]
    pub fn mail_mark_read(message: Uuid) -> Vec<LeaseKey> {
        lease_set([c(LeaseKind::Mail, message)])
    }
    /// `mail:m` and `mailbox:c_original_sender` (`MailReject`); the return message is inserted under the sender's key.
    #[must_use]
    pub fn mail_reject(message: Uuid, original_sender: Uuid) -> Vec<LeaseKey> {
        lease_set([
            c(LeaseKind::Mail, message),
            c(LeaseKind::Mailbox, original_sender),
        ])
    }
    /// `mail:m` and `mailbox:{c_sender, c_receiver}` (`MailCancel`).
    #[must_use]
    pub fn mail_cancel(message: Uuid, sender: Uuid, receiver: Uuid) -> Vec<LeaseKey> {
        lease_set([
            c(LeaseKind::Mail, message),
            c(LeaseKind::Mailbox, sender),
            c(LeaseKind::Mailbox, receiver),
        ])
    }
    /// `mail:m_i` for every message and `mailbox:c_actor` (`MailDelete`).
    #[must_use]
    pub fn mail_delete(messages: &[Uuid], actor: Uuid) -> Vec<LeaseKey> {
        lease_set(
            messages
                .iter()
                .map(|m| c(LeaseKind::Mail, *m))
                .chain(std::iter::once(c(LeaseKind::Mailbox, actor))),
        )
    }
    /// `delivery:d` (when attachments exist), `mail:m`, `mailbox:{c_sender, c_receiver}` (`MailExpire`, system).
    #[must_use]
    pub fn mail_expire(
        message: Uuid,
        sender: Uuid,
        receiver: Uuid,
        delivery: Option<Uuid>,
    ) -> Vec<LeaseKey> {
        lease_set(
            [
                c(LeaseKind::Mail, message),
                c(LeaseKind::Mailbox, sender),
                c(LeaseKind::Mailbox, receiver),
            ]
            .into_iter()
            .chain(delivery.map(|d| c(LeaseKind::Delivery, d))),
        )
    }
    /// `delivery:d`, then `warehouse:c_target` for `mail_expiry_return` / `auction_award` (`DeliverPending`, E33).
    #[must_use]
    pub fn deliver_pending(delivery: Uuid, kind: DeliveryKind, target: Uuid) -> Vec<LeaseKey> {
        lease_set(
            std::iter::once(c(LeaseKind::Delivery, delivery)).chain(
                matches!(kind, DeliveryKind::MailExpiryReturn | DeliveryKind::AuctionAward)
                    .then(|| c(LeaseKind::Warehouse, target)),
            ),
        )
    }
    /// `auction:a` (`ItemAuctionBid`; `AuctionSchedule` Create/Start/Purge).
    #[must_use]
    pub fn auction(auction: u64) -> Vec<LeaseKey> {
        lease_set([LeaseKey {
            kind: LeaseKind::Auction,
            owner: u128::from(auction),
        }])
    }
    /// `auction:a` for each non-CREATED auction of the instance holding the actor's bid (`ItemAuctionCancelBid`).
    #[must_use]
    pub fn auction_cancel_bid(auctions: &[u64]) -> Vec<LeaseKey> {
        lease_set(auctions.iter().map(|a| LeaseKey {
            kind: LeaseKind::Auction,
            owner: u128::from(*a),
        }))
    }
    /// `auction:a` and `delivery:d` (`AuctionSchedule` Finish; the award delivery is inserted under `delivery:d`).
    #[must_use]
    pub fn auction_finish(auction: u64, delivery: Uuid) -> Vec<LeaseKey> {
        lease_set([
            LeaseKey {
                kind: LeaseKind::Auction,
                owner: u128::from(auction),
            },
            c(LeaseKind::Delivery, delivery),
        ])
    }
    /// `manor:manor_id` (`ManorBuySeed`).
    #[must_use]
    pub fn manor_buy_seed(manor: u32) -> Vec<LeaseKey> {
        lease_set([LeaseKey {
            kind: LeaseKind::Manor,
            owner: u128::from(manor),
        }])
    }
    /// `manor:<each distinct entry manor_id>` (`ManorSellCrop`).
    #[must_use]
    pub fn manor_sell_crop(manors: &[u32]) -> Vec<LeaseKey> {
        lease_set(manors.iter().map(|m| LeaseKey {
            kind: LeaseKind::Manor,
            owner: u128::from(*m),
        }))
    }
    /// `EpochReconcile` holds nothing (ledger only).
    #[must_use]
    pub fn epoch_reconcile() -> Vec<LeaseKey> {
        Vec::new()
    }

    pub use super::DeliveryKind;
}

/// `economy_deliveries.kind` (design §10.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum DeliveryKind {
    CodPayment,
    MailExpiryReturn,
    AuctionAward,
}

impl DeliveryKind {
    /// The column text.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::CodPayment => "cod_payment",
            Self::MailExpiryReturn => "mail_expiry_return",
            Self::AuctionAward => "auction_award",
        }
    }
}

/// `AuctionSchedule` transitions (design §10.5, §12.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum AuctionTransition {
    Create,
    Start,
    Finish,
    Purge,
}

impl AuctionTransition {
    const fn name(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Start => "start",
            Self::Finish => "finish",
            Self::Purge => "purge",
        }
    }
}

/// The system jobs' derived `request_key` (design §11.4; DR5 nit 10):
/// `UUIDv5(NS_SYSJOB, kind ‖ delivery_id | auction_id ‖ transition | message_id)`. Recovery keys on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemJob {
    /// `DeliverPending{delivery}`.
    DeliverPending(Uuid),
    /// `AuctionSchedule{auction, transition}`.
    AuctionSchedule(u64, AuctionTransition),
    /// `MailExpire{message}`.
    MailExpire(Uuid),
    /// `EpochReconcile{zone, epoch}`.
    EpochReconcile(u32, u64),
}

impl SystemJob {
    /// The `PbCommandKind` of the job.
    #[must_use]
    pub const fn kind(self) -> Phase5CommandKind {
        match self {
            Self::DeliverPending(_) => Phase5CommandKind::DeliverPending,
            Self::AuctionSchedule(..) => Phase5CommandKind::AuctionSchedule,
            Self::MailExpire(_) => Phase5CommandKind::MailExpire,
            Self::EpochReconcile(..) => Phase5CommandKind::EpochReconcile,
        }
    }

    /// The deterministic request key.
    #[must_use]
    pub fn request_key(self) -> Uuid {
        let name = match self {
            Self::DeliverPending(d) => format!("deliver_pending:{d}"),
            Self::AuctionSchedule(a, t) => format!("auction_schedule:{a}:{}", t.name()),
            Self::MailExpire(m) => format!("mail_expire:{m}"),
            Self::EpochReconcile(z, e) => format!("epoch_reconcile:{z}:{e}"),
        };
        Uuid::new_v5(&NS_SYSJOB, name.as_bytes())
    }
}

/// `auction_bid` owner id: `UUIDv5(NS_AUCTION, auction_id ‖ character)` (design §11.1).
#[must_use]
pub fn auction_bid_owner(auction: u64, character: Uuid) -> Uuid {
    Uuid::new_v5(&NS_AUCTION, format!("{auction}:{character}").as_bytes())
}

/// A derived row id (message or delivery) from request identity (E12): never a `BIGSERIAL`.
#[must_use]
pub fn derived_row_id(row: DerivedRow, request_key: Uuid, ordinal: u32) -> Uuid {
    Uuid::new_v5(row.namespace(), format!("{}:{request_key}:{ordinal}", row.label()).as_bytes())
}

/// One durable container row as the preflight froze it (design §10.0 step 1, §12.2). Integers
/// only; canonical JSON (sorted keys) is the fingerprint input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenRow {
    /// The lease key covering the row.
    pub lease: LeaseKey,
    /// The row kind (`item_instance`, `mail_message`, `item_auction`, `item_auction_bid`,
    /// `manor_seed_production`, `manor_crop_procure`, `economy_delivery`).
    pub row_kind: String,
    /// The row key as text (uuid, bigint or `castle:id`).
    pub row_key: String,
    /// The row's `revision` at preflight.
    pub revision: u64,
    /// The row columns the zone needs (integers and strings only; no timestamps).
    pub columns: serde_json::Map<String, serde_json::Value>,
}

/// The frozen preflight result carried inside the command bytes (design §10.0, §12.2).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenInputs {
    /// The acquired lease set (ascending).
    pub leases: Vec<LeaseKey>,
    /// Pre-images of every covered row the command touches.
    pub rows: Vec<FrozenRow>,
    /// DB-time decisions (`expired`, `remaining_le_10min`, `older_than_14d`, counts, resolutions),
    /// as integers/booleans/strings only.
    pub decisions: serde_json::Map<String, serde_json::Value>,
}

impl FrozenInputs {
    /// Canonical JSON (sorted keys, no whitespace): the fingerprint input of D43.
    pub fn canonical_json(&self) -> anyhow::Result<String> {
        let value = serde_json::to_value(self)?;
        Ok(canonical(&value))
    }

    /// Refuses floats and timestamps anywhere in the payload (E12: integers only).
    pub fn validate(&self) -> Result<(), EconomyError> {
        fn walk(value: &serde_json::Value) -> Result<(), EconomyError> {
            match value {
                serde_json::Value::Number(n) if !n.is_u64() && !n.is_i64() => {
                    Err(EconomyError::Constraint("frozen inputs carry a non-integer number".into()))
                },
                serde_json::Value::Array(items) => items.iter().try_for_each(walk),
                serde_json::Value::Object(map) => map.values().try_for_each(walk),
                serde_json::Value::Null
                | serde_json::Value::Bool(_)
                | serde_json::Value::Number(_)
                | serde_json::Value::String(_) => Ok(()),
            }
        }
        if self
            .leases
            .windows(2)
            .any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a >= b))
        {
            return Err(EconomyError::Constraint("frozen lease set out of canonical order".into()));
        }
        for row in &self.rows {
            if !self.leases.contains(&row.lease) {
                return Err(EconomyError::Constraint("frozen row outside the lease set".into()));
            }
            walk(&serde_json::Value::Object(row.columns.clone()))?;
        }
        walk(&serde_json::Value::Object(self.decisions.clone()))
    }
}

fn canonical(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let body: Vec<String> = keys
                .into_iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::Value::String(k.clone()),
                        canonical(map.get(k).unwrap_or(&serde_json::Value::Null))
                    )
                })
                .collect();
            format!("{{{}}}", body.join(","))
        },
        serde_json::Value::Array(items) => {
            let body: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", body.join(","))
        },
        other @ (serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_)) => other.to_string(),
    }
}

/// How a container delta changes its row (design §10.0 step 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum DeltaChange {
    Insert,
    Update,
    Delete,
}

/// One container row delta: CAS on `revision` for updates and deletes, insert for new rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerDelta {
    /// The lease key the row is covered by (fresh rows: the key they are inserted under).
    pub lease: LeaseKey,
    /// Row kind (see [`FrozenRow::row_kind`]).
    pub row_kind: String,
    /// Row key text.
    pub row_key: String,
    /// The change.
    pub change: DeltaChange,
    /// The revision the pre-image carried (`None` for inserts).
    pub expected_revision: Option<u64>,
    /// Column values to write (inserts and updates), as the adapter understands them per row kind.
    pub columns: serde_json::Map<String, serde_json::Value>,
}

/// One owner-kind ledger row (design §11.1–11.3), ordered by T1 inside the writer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerLedgerRow {
    /// `item_ledger.owner_kind`.
    pub owner_kind: String,
    /// The owner id (character id for `character`; the owner uuid otherwise).
    pub owner_id: Uuid,
    /// Zone, epoch, tick, revision.
    pub zone_id: u32,
    /// The epoch.
    pub epoch: u64,
    /// The tick.
    pub tick: u64,
    /// The owner's revision after the row.
    pub revision: u64,
    /// `item_ledger.op`.
    pub op: String,
    /// The instance.
    pub instance_id: u64,
    /// Row position 0..n-1 (T1).
    pub operation_ordinal: u32,
    /// `delta` JSON (`before`/`after`, transfer link, causes).
    pub delta: serde_json::Value,
    /// Receipt reference.
    pub receipt_ref: Option<Uuid>,
}

/// A pending delivery row (design §10.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryRow {
    /// `economy_deliveries.id` (derived `UUIDv5`).
    pub id: Uuid,
    /// The target character.
    pub target_character: Uuid,
    /// The kind.
    pub kind: DeliveryKind,
    /// Integer-only payload.
    pub payload: serde_json::Value,
}

/// A delivery completion inside a checkpoint (status `delivered` with zone/epoch/tick).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryCompletion {
    /// The delivery.
    pub id: Uuid,
    /// Where it executed.
    pub zone_id: u32,
    /// The epoch.
    pub epoch: u64,
    /// The tick.
    pub tick: u64,
    /// The delivery row's pre-image revision (CAS).
    pub expected_revision: u64,
}

/// One outbox fact of design §11.5 (`nightfall.item.<subject>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboxFact {
    /// The subject suffix (one of [`OUTBOX_SUBJECTS`]).
    pub subject: String,
    /// The fact id (B8 dedupe for `economy_counters`).
    pub fact_id: Uuid,
    /// Integer-only payload.
    pub payload: serde_json::Value,
}

/// The `nightfall.item.*` subject suffixes of design §11.5 (plus the existing `destroyed` and `exchanged`).
pub const OUTBOX_SUBJECTS: [&str; 26] = [
    "looted",
    "picked_up",
    "dropped_pk",
    "swept",
    "harvested",
    "purchased",
    "sold",
    "refunded",
    "crafted",
    "traded",
    "deposited",
    "withdrawn",
    "freighted",
    "mailed",
    "mail_taken",
    "mail_returned",
    "mail_expired",
    "delivered",
    "delivery_blocked",
    "auction_bid",
    "auction_refunded",
    "auction_awarded",
    "manor_traded",
    "decayed",
    "destroyed",
    "exchanged",
];

/// The economy part of one tick's checkpoint transaction (design §11.5, `EconomyCheckpoint` `PbOutput` 55).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EconomyCheckpointWrite {
    /// Zone and epoch of the writing zone.
    pub zone_id: u32,
    /// The epoch.
    pub epoch: u64,
    /// The tick.
    pub tick: u64,
    /// Container deltas; applied in ascending lease-key order.
    pub deltas: Vec<ContainerDelta>,
    /// New deliveries.
    pub deliveries: Vec<DeliveryRow>,
    /// Deliveries completed this tick.
    pub completions: Vec<DeliveryCompletion>,
    /// Owner-kind ledger rows (T1 order per owner/instance).
    pub ledger: Vec<OwnerLedgerRow>,
    /// Leases released; deleted in ascending key order after the deltas.
    pub lease_releases: Vec<LeaseKey>,
    /// Outbox facts.
    pub outbox: Vec<OutboxFact>,
}

impl EconomyCheckpointWrite {
    /// Structural checks before any write: canonical lease order, known subjects, every delta's
    /// key inside the release set, integer-only payloads.
    pub fn validate(&self) -> Result<(), EconomyError> {
        if self
            .lease_releases
            .windows(2)
            .any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a >= b))
        {
            return Err(EconomyError::Constraint("lease releases out of canonical order".into()));
        }
        for delta in &self.deltas {
            if !self.lease_releases.contains(&delta.lease) {
                return Err(EconomyError::Constraint(
                    "container delta outside the released lease set".into(),
                ));
            }
            if matches!(delta.change, DeltaChange::Insert) != delta.expected_revision.is_none() {
                return Err(EconomyError::Constraint(
                    "insert deltas carry no expected revision; updates and deletes must".into(),
                ));
            }
        }
        for fact in &self.outbox {
            if !OUTBOX_SUBJECTS.contains(&fact.subject.as_str()) {
                return Err(EconomyError::Constraint(format!(
                    "unknown outbox subject {}",
                    fact.subject
                )));
            }
        }
        Ok(())
    }

    /// The deltas in the order the writer applies them (ascending lease key, then row key).
    #[must_use]
    pub fn ordered_deltas(&self) -> Vec<&ContainerDelta> {
        let mut deltas: Vec<&ContainerDelta> = self.deltas.iter().collect();
        deltas.sort_by(|a, b| {
            a.lease
                .cmp(&b.lease)
                .then_with(|| a.row_key.cmp(&b.row_key))
        });
        deltas
    }
}

/// Lease acquisition and release (design §10.0 steps 1 and 4; story 4.3).
#[async_trait]
pub trait LeaseRepository: Send + Sync {
    /// Acquire the whole set all-or-nothing in ascending key order. Returns `Busy(held)` with
    /// nothing inserted when any key is held; `Transient` on `40P01`/`40001`.
    async fn acquire(&self, grant: &LeaseGrant) -> Result<(), EconomyError>;
    /// Release the given keys of `(zone, epoch)` (preflight rollback paths).
    async fn release(
        &self,
        zone_id: u32,
        epoch: u64,
        keys: &[LeaseKey],
    ) -> Result<(), EconomyError>;
    /// Epoch-fenced recovery: release every lease of `(zone, epoch)`; returns what was held.
    async fn release_epoch(
        &self,
        zone_id: u32,
        epoch: u64,
    ) -> Result<Vec<LeaseGrant>, EconomyError>;
    /// The current holders of `keys` (empty when free).
    async fn held(&self, keys: &[LeaseKey]) -> Result<Vec<LeaseGrant>, EconomyError>;
}

/// The economy checkpoint writer (design §11.5; story 4.4).
#[async_trait]
pub trait EconomyCheckpointRepository: Send + Sync {
    /// Apply the whole write in one transaction: deltas (CAS) and lease deletes in key order,
    /// delivery inserts/completions, ledger rows, outbox facts. `LeaseViolation` is permanent;
    /// `Transient` means retry at the next cadence with the same acknowledged draft.
    async fn checkpoint_economy(&self, write: &EconomyCheckpointWrite) -> Result<(), EconomyError>;
}

/// Pending deliveries (design §10.4; E38/E41 queue: ascending id, one per target per tick).
#[async_trait]
pub trait DeliveryRepository: Send + Sync {
    /// Pending deliveries of a target in ascending id order.
    async fn pending(&self, target: Uuid) -> Result<Vec<DeliveryRow>, EconomyError>;
    /// The head of a target's queue, if any.
    async fn head(&self, target: Uuid) -> Result<Option<DeliveryRow>, EconomyError> {
        Ok(self.pending(target).await?.into_iter().next())
    }
}

/// `economy_counters` rollups fed asynchronously from outbox facts, idempotent by `fact_id` (B8).
#[async_trait]
pub trait EconomyCounterRepository: Send + Sync {
    /// Apply one fact: adds `value` to `(day, counter)` once; a repeated `fact_id` changes nothing.
    /// Returns `true` when the fact was new.
    async fn apply_fact(
        &self,
        fact_id: Uuid,
        day: chrono::NaiveDate,
        counter: &str,
        value: i128,
    ) -> Result<bool, EconomyError>;
    /// The current value of `(day, counter)`.
    async fn counter(&self, day: chrono::NaiveDate, counter: &str) -> Result<i128, EconomyError>;
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests {
    use super::*;

    const A: Uuid = Uuid::from_u128(0x50);
    const B: Uuid = Uuid::from_u128(0x51);
    const M: Uuid = Uuid::from_u128(0x7100);
    const D: Uuid = Uuid::from_u128(0x7200);

    #[test]
    fn mail_send_acquires_in_the_same_order_for_both_roles() {
        assert_eq!(sets::mail_send(A, B), sets::mail_send(B, A));
        assert_eq!(sets::mail_send(A, B)[0].owner, A.as_u128());
        assert_eq!(sets::mail_send(A, A).len(), 1, "self-addressed key appears once");
    }

    #[test]
    fn key_order_is_bytewise_on_the_canonical_text() {
        let keys = lease_set([
            LeaseKey::of_uuid(LeaseKind::Warehouse, A),
            LeaseKey::of_uuid(LeaseKind::Mailbox, M),
            LeaseKey::of_uuid(LeaseKind::Mail, M),
            LeaseKey {
                kind: LeaseKind::Auction,
                owner: 1,
            },
            LeaseKey::of_uuid(LeaseKind::Delivery, D),
            LeaseKey {
                kind: LeaseKind::Manor,
                owner: 1,
            },
            LeaseKey::of_uuid(LeaseKind::Freight, A),
        ]);
        let kinds: Vec<LeaseKind> = keys.iter().map(|k| k.kind).collect();
        assert_eq!(
            kinds,
            [
                LeaseKind::Auction,
                LeaseKind::Delivery,
                LeaseKind::Freight,
                LeaseKind::Mail,
                LeaseKind::Mailbox,
                LeaseKind::Manor,
                LeaseKind::Warehouse
            ]
        );
        let canonical: Vec<String> = keys.iter().map(|k| k.canonical()).collect();
        let mut sorted = canonical.clone();
        sorted.sort();
        assert_eq!(canonical, sorted);
        assert_eq!(keys[0].canonical().len(), "auction:".len() + 39);
    }

    #[test]
    fn every_command_set_matches_design_10_0() {
        assert_eq!(sets::warehouse(A), vec![LeaseKey::of_uuid(LeaseKind::Warehouse, A)]);
        assert_eq!(sets::freight_send(B), vec![LeaseKey::of_uuid(LeaseKind::Freight, B)]);
        assert_eq!(sets::mail_take(M, None), vec![LeaseKey::of_uuid(LeaseKind::Mail, M)]);
        assert_eq!(
            sets::mail_take(M, Some(D)),
            vec![
                LeaseKey::of_uuid(LeaseKind::Delivery, D),
                LeaseKey::of_uuid(LeaseKind::Mail, M)
            ]
        );
        assert_eq!(sets::mail_cancel(M, A, B).len(), 3);
        assert_eq!(sets::mail_delete(&[M, D, M], A).len(), 3, "duplicates collapse");
        assert_eq!(sets::mail_expire(M, A, B, None).len(), 3);
        assert_eq!(sets::mail_expire(M, A, B, Some(D)).len(), 4);
        assert_eq!(sets::deliver_pending(D, DeliveryKind::CodPayment, A).len(), 1);
        assert_eq!(sets::deliver_pending(D, DeliveryKind::AuctionAward, A).len(), 2);
        assert_eq!(
            sets::deliver_pending(D, DeliveryKind::MailExpiryReturn, A)[1].kind,
            LeaseKind::Warehouse
        );
        assert_eq!(
            sets::auction_cancel_bid(&[3, 1, 2, 1])
                .iter()
                .map(|k| k.owner)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(sets::auction_finish(7, D)[0].kind, LeaseKind::Auction);
        assert_eq!(sets::manor_sell_crop(&[2, 1, 2]).len(), 2);
        assert!(sets::epoch_reconcile().is_empty());
    }

    #[test]
    fn system_job_keys_are_deterministic_and_distinct() {
        let a = SystemJob::AuctionSchedule(1, AuctionTransition::Finish);
        assert_eq!(a.request_key(), a.request_key());
        assert_ne!(
            a.request_key(),
            SystemJob::AuctionSchedule(1, AuctionTransition::Purge).request_key()
        );
        assert_ne!(
            SystemJob::DeliverPending(D).request_key(),
            SystemJob::MailExpire(D).request_key()
        );
        assert_eq!(SystemJob::MailExpire(M).kind(), Phase5CommandKind::MailExpire);
        assert_eq!(auction_bid_owner(1, A), auction_bid_owner(1, A));
        assert_ne!(auction_bid_owner(1, A), auction_bid_owner(2, A));
        assert_ne!(
            derived_row_id(DerivedRow::Message, D, 0),
            derived_row_id(DerivedRow::Message, D, 1)
        );
        assert_ne!(
            derived_row_id(DerivedRow::Message, D, 0),
            derived_row_id(DerivedRow::Delivery, D, 0)
        );
        assert_ne!(
            derived_row_id(DerivedRow::Delivery, D, 0),
            derived_row_id(DerivedRow::Transfer, D, 0)
        );
        assert_ne!(NS_MAIL, NS_DELIVERY);
        assert_ne!(NS_DELIVERY, NS_XFER);
        let mut descending = sets::mail_send(A, B);
        descending.reverse();
        assert!(matches!(canonical_keys(&descending), Err(EconomyError::Constraint(_))));
        assert!(canonical_keys(&sets::mail_send(A, B)).is_ok());
        assert!(matches!(
            canonical_keys(&[descending[0], descending[0]]),
            Err(EconomyError::Constraint(_))
        ));
    }

    #[test]
    fn frozen_inputs_are_canonical_and_integer_only() {
        let lease = LeaseKey::of_uuid(LeaseKind::Warehouse, A);
        let mut columns = serde_json::Map::new();
        columns.insert("count".into(), serde_json::json!(5));
        columns.insert("template_id".into(), serde_json::json!(1864));
        let frozen = FrozenInputs {
            leases: vec![lease],
            rows: vec![FrozenRow {
                lease,
                row_kind: "item_instance".into(),
                row_key: "900001".into(),
                revision: 0,
                columns,
            }],
            decisions: serde_json::Map::new(),
        };
        frozen.validate().unwrap();
        let json = frozen.canonical_json().unwrap();
        assert!(json.contains("{\"count\":5,\"template_id\":1864}"), "{json}");
        assert_eq!(json, frozen.canonical_json().unwrap());
        let mut floaty = frozen.clone();
        floaty
            .decisions
            .insert("rate".into(), serde_json::json!(1.5));
        assert!(matches!(floaty.validate(), Err(EconomyError::Constraint(_))));
        let mut outside = frozen;
        outside.rows[0].lease = LeaseKey::of_uuid(LeaseKind::Freight, A);
        assert!(matches!(outside.validate(), Err(EconomyError::Constraint(_))));
    }

    #[test]
    fn checkpoint_write_validation() {
        let lease = LeaseKey::of_uuid(LeaseKind::Warehouse, A);
        let mut write = EconomyCheckpointWrite {
            lease_releases: vec![lease],
            deltas: vec![ContainerDelta {
                lease,
                row_kind: "item_instance".into(),
                row_key: "900001".into(),
                change: DeltaChange::Update,
                expected_revision: Some(0),
                columns: serde_json::Map::new(),
            }],
            outbox: vec![OutboxFact {
                subject: "deposited".into(),
                fact_id: D,
                payload: serde_json::json!({}),
            }],
            ..Default::default()
        };
        write.validate().unwrap();
        write.outbox[0].subject = "teleported".into();
        assert!(write.validate().is_err());
        write.outbox.clear();
        write.deltas[0].expected_revision = None;
        assert!(write.validate().is_err());
        write.deltas[0].change = DeltaChange::Insert;
        write.validate().unwrap();
        write.deltas[0].lease = LeaseKey::of_uuid(LeaseKind::Freight, A);
        assert!(write.validate().is_err());
    }
}
