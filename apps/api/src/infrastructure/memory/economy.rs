//! In-memory economy adapter (stories 4.3/4.4): the same lease, checkpoint, delivery and
//! counter contracts as the Postgres adapter, over maps under one mutex, so every call is atomic
//! like a transaction. Tests inject a transient failure to exercise the retry classification.

use parking_lot::Mutex;
use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use uuid::Uuid;

use crate::application::economy::{
    canonical_keys, ContainerDelta, DeliveryCompletion, DeliveryRepository, DeliveryRow,
    DeltaChange, EconomyCheckpointRepository, EconomyCheckpointWrite, EconomyCounterRepository,
    EconomyError, LeaseGrant, LeaseKey, LeaseRepository, OutboxFact, OwnerLedgerRow,
};

/// A stored container row: its revision and columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRow {
    /// The current `revision`.
    pub revision: u64,
    /// The columns as last written.
    pub columns: serde_json::Map<String, serde_json::Value>,
}

#[derive(Default)]
struct State {
    leases: BTreeMap<LeaseKey, LeaseGrant>,
    rows: BTreeMap<(String, String), StoredRow>,
    deliveries: BTreeMap<Uuid, (DeliveryRow, Option<DeliveryCompletion>, u64)>,
    ledger: Vec<OwnerLedgerRow>,
    outbox: Vec<OutboxFact>,
    counters: BTreeMap<(chrono::NaiveDate, String), i128>,
    facts: BTreeSet<Uuid>,
    fail_next_transient: u32,
    /// DB time (epoch ms) of this adapter: the stand-in for Postgres `now()`.
    now_ms: i64,
    /// `mail_messages.created_at` / `expires_at` (epoch ms), kept out of the row columns so that
    /// a pre-image never carries a timestamp (E12).
    mail_times: BTreeMap<Uuid, (i64, i64)>,
    /// `characters.name` for mail headers.
    names: BTreeMap<Uuid, String>,
}

/// The default DB time of a fresh memory store (2026-10-31T00:00:00Z).
pub const MEMORY_EPOCH_MS: i64 = 1_793_404_800_000;

/// Map-backed economy store.
pub struct InMemoryEconomyStore {
    state: Mutex<State>,
}

impl Default for InMemoryEconomyStore {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                now_ms: MEMORY_EPOCH_MS,
                ..State::default()
            }),
        }
    }
}

impl InMemoryEconomyStore {
    /// Set the adapter's DB time (tests: expiry and auction transitions).
    pub fn set_now_ms(&self, now_ms: i64) {
        self.lock().now_ms = now_ms;
    }

    /// Record a character name (mail headers read it as Postgres joins `characters`).
    pub fn set_character_name(&self, character: Uuid, name: &str) {
        self.lock().names.insert(character, name.to_owned());
    }

    /// Override a message's DB times (seeding an old message).
    pub fn set_mail_times(&self, message: Uuid, created_ms: i64, expires_ms: i64) {
        self.lock()
            .mail_times
            .insert(message, (created_ms, expires_ms));
    }

    fn lock(&self) -> parking_lot::MutexGuard<'_, State> {
        self.state.lock()
    }

    /// The next `count` lease/checkpoint calls fail `Transient` (the forced `40P01` of story 4.4).
    pub fn fail_next_transient(&self, count: u32) {
        self.lock().fail_next_transient = count;
    }

    fn take_transient(state: &mut State) -> Result<(), EconomyError> {
        if let Some(left) = state.fail_next_transient.checked_sub(1) {
            state.fail_next_transient = left;
            return Err(EconomyError::Transient("40P01 (injected)".into()));
        }
        Ok(())
    }

    /// Seed a container row (tests and seeders).
    pub fn put_row(&self, row_kind: &str, row_key: &str, row: StoredRow) {
        self.lock()
            .rows
            .insert((row_kind.to_owned(), row_key.to_owned()), row);
    }

    /// Read a container row.
    #[must_use]
    pub fn row(&self, row_kind: &str, row_key: &str) -> Option<StoredRow> {
        self.lock()
            .rows
            .get(&(row_kind.to_owned(), row_key.to_owned()))
            .cloned()
    }

    /// Every ledger row written so far, in write order.
    #[must_use]
    pub fn ledger(&self) -> Vec<OwnerLedgerRow> {
        self.lock().ledger.clone()
    }

    /// Every outbox fact written so far, in write order.
    #[must_use]
    pub fn outbox(&self) -> Vec<OutboxFact> {
        self.lock().outbox.clone()
    }

    /// A delivery's completion (when delivered).
    #[must_use]
    pub fn completion(&self, id: Uuid) -> Option<DeliveryCompletion> {
        self.lock()
            .deliveries
            .get(&id)
            .and_then(|(_, done, _)| *done)
    }

    /// Insert a pending delivery outside a checkpoint (seeding).
    pub fn put_delivery(&self, row: DeliveryRow) {
        self.lock().deliveries.insert(row.id, (row, None, 0));
    }

    fn apply_delta(state: &mut State, delta: &ContainerDelta) -> Result<(), EconomyError> {
        let key = (delta.row_kind.clone(), delta.row_key.clone());
        match delta.change {
            DeltaChange::Insert => {
                if state.rows.contains_key(&key) {
                    return Err(EconomyError::LeaseViolation(delta.lease));
                }
                if delta.row_kind == "mail_message" {
                    // expires_at is computed in the insert, as the Postgres writer does: +12 h
                    // with COD, +360 h otherwise (D §11.4).
                    let id: Uuid = delta
                        .row_key
                        .parse()
                        .map_err(|_| EconomyError::Constraint("economy_delta_row_key".into()))?;
                    let cod = delta
                        .columns
                        .get("cod_adena")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0);
                    let hours: i64 = if cod > 0 { 12 } else { 360 };
                    let expires = hours
                        .checked_mul(3_600_000)
                        .and_then(|span| state.now_ms.checked_add(span))
                        .ok_or_else(|| EconomyError::Constraint("mail expiry overflow".into()))?;
                    state.mail_times.insert(id, (state.now_ms, expires));
                }
                state.rows.insert(
                    key,
                    StoredRow {
                        revision: 0,
                        columns: delta.columns.clone(),
                    },
                );
            },
            DeltaChange::Update => {
                let Some(row) = state.rows.get_mut(&key) else {
                    return Err(EconomyError::LeaseViolation(delta.lease));
                };
                if Some(row.revision) != delta.expected_revision {
                    return Err(EconomyError::LeaseViolation(delta.lease));
                }
                row.revision = row
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| EconomyError::Constraint("revision overflow".into()))?;
                for (column, value) in &delta.columns {
                    row.columns.insert(column.clone(), value.clone());
                }
            },
            DeltaChange::Delete => {
                let Some(row) = state.rows.get(&key) else {
                    return Err(EconomyError::LeaseViolation(delta.lease));
                };
                if Some(row.revision) != delta.expected_revision {
                    return Err(EconomyError::LeaseViolation(delta.lease));
                }
                if delta.row_kind == "mail_message" {
                    if let Ok(id) = delta.row_key.parse::<Uuid>() {
                        state.mail_times.remove(&id);
                    }
                }
                state.rows.remove(&key);
            },
        }
        Ok(())
    }
}

#[async_trait]
impl LeaseRepository for InMemoryEconomyStore {
    async fn acquire(&self, grant: &LeaseGrant) -> Result<(), EconomyError> {
        canonical_keys(&grant.keys)?;
        let mut state = self.lock();
        Self::take_transient(&mut state)?;
        let held: Vec<LeaseKey> = grant
            .keys
            .iter()
            .filter(|key| state.leases.contains_key(key))
            .copied()
            .collect();
        if !held.is_empty() {
            return Err(EconomyError::Busy(held));
        }
        for key in &grant.keys {
            state.leases.insert(*key, grant.clone());
        }
        Ok(())
    }

    async fn release(
        &self,
        zone_id: u32,
        epoch: u64,
        keys: &[LeaseKey],
    ) -> Result<(), EconomyError> {
        let mut state = self.lock();
        for key in keys {
            if state
                .leases
                .get(key)
                .is_some_and(|g| g.zone_id == zone_id && g.epoch == epoch)
            {
                state.leases.remove(key);
            }
        }
        Ok(())
    }

    async fn release_epoch(
        &self,
        zone_id: u32,
        epoch: u64,
    ) -> Result<Vec<LeaseGrant>, EconomyError> {
        let mut state = self.lock();
        let released: Vec<LeaseKey> = state
            .leases
            .iter()
            .filter(|(_, g)| g.zone_id == zone_id && g.epoch == epoch)
            .map(|(k, _)| *k)
            .collect();
        let mut grants = Vec::new();
        for key in released {
            if let Some(grant) = state.leases.remove(&key) {
                grants.push(grant);
            }
        }
        grants.dedup_by(|a, b| a.request_key == b.request_key);
        Ok(grants)
    }

    async fn held(&self, keys: &[LeaseKey]) -> Result<Vec<LeaseGrant>, EconomyError> {
        let state = self.lock();
        // One grant per holder (the whole stored set), like the Postgres adapter.
        let mut grants: Vec<LeaseGrant> = Vec::new();
        for grant in keys.iter().filter_map(|k| state.leases.get(k)) {
            if !grants.iter().any(|g| g.request_key == grant.request_key) {
                grants.push(grant.clone());
            }
        }
        Ok(grants)
    }
}

#[async_trait]
impl EconomyCheckpointRepository for InMemoryEconomyStore {
    async fn checkpoint_economy(&self, write: &EconomyCheckpointWrite) -> Result<(), EconomyError> {
        write.validate()?;
        if !write.receipts.is_empty() {
            return Err(EconomyError::Constraint(
                "economy_receipts_need_character_checkpoint".into(),
            ));
        }
        self.apply_write(write)
    }
}

impl InMemoryEconomyStore {
    /// The whole write, atomically (dry run over copies, then one assignment). The memory
    /// character repository calls it inside its combined checkpoint, which holds the receipts.
    pub(crate) fn apply_write(&self, write: &EconomyCheckpointWrite) -> Result<(), EconomyError> {
        let derived = crate::application::economy_facts::derive_write_facts(write)?;
        let mut state = self.lock();
        Self::take_transient(&mut state)?;
        // c-review-4 P2-1: every released lease must still be held by this (zone, epoch); a
        // writer whose leases recovery released is fenced, with nothing written.
        for key in &write.lease_releases {
            if !state
                .leases
                .get(key)
                .is_some_and(|g| g.zone_id == write.zone_id && g.epoch == write.epoch)
            {
                return Err(EconomyError::LeaseViolation(*key));
            }
        }
        // Dry-run the CAS over a copy so a violation leaves the store untouched (one transaction).
        let mut rows = state.rows.clone();
        let mut deliveries = state.deliveries.clone();
        let mail_times;
        {
            let mut scratch = State {
                rows: std::mem::take(&mut rows),
                now_ms: state.now_ms,
                mail_times: state.mail_times.clone(),
                ..State::default()
            };
            for delta in write.ordered_deltas() {
                Self::apply_delta(&mut scratch, delta)?;
            }
            rows = scratch.rows;
            mail_times = scratch.mail_times;
        }
        for completion in &write.completions {
            let Some((_, done, revision)) = deliveries.get_mut(&completion.id) else {
                return Err(EconomyError::LeaseViolation(LeaseKey::of_uuid(
                    crate::application::economy::LeaseKind::Delivery,
                    completion.id,
                )));
            };
            if done.is_some() || *revision != completion.expected_revision {
                return Err(EconomyError::LeaseViolation(LeaseKey::of_uuid(
                    crate::application::economy::LeaseKind::Delivery,
                    completion.id,
                )));
            }
            *done = Some(*completion);
            *revision = revision
                .checked_add(1)
                .ok_or_else(|| EconomyError::Constraint("revision overflow".into()))?;
        }
        for delivery in &write.deliveries {
            if deliveries.contains_key(&delivery.id) {
                return Err(EconomyError::Constraint("economy_deliveries_pkey".into()));
            }
            deliveries.insert(delivery.id, (delivery.clone(), None, 0));
        }
        state.rows = rows;
        state.mail_times = mail_times;
        state.deliveries = deliveries;
        state.ledger.extend(write.ledger.iter().cloned());
        state.outbox.extend(write.outbox.iter().cloned());
        // P5-D18: the economy facts are derived from the ledger rows; the zone emits none.
        state.outbox.extend(derived);
        for key in &write.lease_releases {
            state.leases.remove(key);
        }
        Ok(())
    }
}

#[async_trait]
impl DeliveryRepository for InMemoryEconomyStore {
    async fn pending(&self, target: Uuid) -> Result<Vec<DeliveryRow>, EconomyError> {
        let state = self.lock();
        Ok(state
            .deliveries
            .values()
            .filter(|(row, done, _)| row.target_character == target && done.is_none())
            .map(|(row, _, _)| row.clone())
            .collect())
    }
}

#[async_trait]
impl EconomyCounterRepository for InMemoryEconomyStore {
    async fn apply_fact(
        &self,
        fact_id: Uuid,
        day: chrono::NaiveDate,
        counter: &str,
        value: i128,
    ) -> Result<bool, EconomyError> {
        let mut state = self.lock();
        if !state.facts.insert(fact_id) {
            return Ok(false);
        }
        let slot = state.counters.entry((day, counter.to_owned())).or_insert(0);
        *slot = slot
            .checked_add(value)
            .ok_or_else(|| EconomyError::Constraint("counter overflow".into()))?;
        Ok(true)
    }

    async fn counter(&self, day: chrono::NaiveDate, counter: &str) -> Result<i128, EconomyError> {
        Ok(self
            .lock()
            .counters
            .get(&(day, counter.to_owned()))
            .copied()
            .unwrap_or(0))
    }
}

// --- Read side (stories 4.5 / 4.6): the same views the Postgres adapter reads ----------------

fn col_u64(row: &StoredRow, name: &str) -> u64 {
    row.columns
        .get(name)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}
fn col_i64(row: &StoredRow, name: &str) -> i64 {
    row.columns
        .get(name)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0)
}
fn col_str<'a>(row: &'a StoredRow, name: &str) -> &'a str {
    row.columns
        .get(name)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}
fn col_bool(row: &StoredRow, name: &str) -> bool {
    row.columns
        .get(name)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}
fn col_uuid(row: &StoredRow, name: &str) -> Option<Uuid> {
    row.columns
        .get(name)
        .and_then(serde_json::Value::as_str)
        .and_then(|s| s.parse().ok())
}
fn narrow<T: TryFrom<u64>>(value: u64) -> Result<T, EconomyError> {
    T::try_from(value).map_err(|_| EconomyError::Constraint("economy_read_out_of_range".into()))
}

fn item_of(
    key: &str,
    row: &StoredRow,
) -> Result<crate::application::economy_reads::ContainerItem, EconomyError> {
    Ok(crate::application::economy_reads::ContainerItem {
        instance_id: key
            .parse()
            .map_err(|_| EconomyError::Constraint("economy_read_row_key".into()))?,
        template_id: narrow(col_u64(row, "template_id"))?,
        count: col_u64(row, "count"),
        enchant: narrow(col_u64(row, "enchant"))?,
        augment_id: row
            .columns
            .get("augment_id")
            .and_then(serde_json::Value::as_u64)
            .map(narrow)
            .transpose()?,
        charge_bits: narrow(col_u64(row, "charge_bits"))?,
        revision: row.revision,
    })
}

impl State {
    fn mail_row(
        &self,
        id: Uuid,
        row: &StoredRow,
    ) -> Result<crate::application::economy_reads::MailRow, EconomyError> {
        let (created, expires) = self
            .mail_times
            .get(&id)
            .copied()
            .unwrap_or((self.now_ms, i64::MAX));
        let sender = col_uuid(row, "sender_id").unwrap_or_default();
        let receiver = col_uuid(row, "receiver_id").unwrap_or_default();
        Ok(crate::application::economy_reads::MailRow {
            id,
            sender_id: sender,
            receiver_id: receiver,
            sender_name: self.names.get(&sender).cloned().unwrap_or_default(),
            receiver_name: self.names.get(&receiver).cloned().unwrap_or_default(),
            subject: col_str(row, "subject").to_owned(),
            body: col_str(row, "body").to_owned(),
            cod_adena: col_u64(row, "cod_adena"),
            has_attachments: col_bool(row, "has_attachments"),
            returned: col_bool(row, "returned"),
            system_kind: narrow(col_u64(row, "system_kind"))?,
            read: col_bool(row, "read"),
            deleted_by_sender: col_bool(row, "deleted_by_sender"),
            deleted_by_receiver: col_bool(row, "deleted_by_receiver"),
            created_unix_ms: created,
            expires_unix_ms: expires,
            revision: row.revision,
        })
    }

    fn mails(&self) -> Result<Vec<crate::application::economy_reads::MailRow>, EconomyError> {
        self.rows
            .iter()
            .filter(|((kind, _), _)| kind == "mail_message")
            .map(|((_, key), row)| {
                let id = key
                    .parse()
                    .map_err(|_| EconomyError::Constraint("economy_read_row_key".into()))?;
                self.mail_row(id, row)
            })
            .collect()
    }

    fn items_where(
        &self,
        keep: impl Fn(&StoredRow) -> bool,
    ) -> Result<Vec<(String, StoredRow)>, EconomyError> {
        let mut out: Vec<(u64, String, StoredRow)> = Vec::new();
        for ((kind, key), row) in &self.rows {
            if kind == "item_instance" && keep(row) {
                let id = key
                    .parse()
                    .map_err(|_| EconomyError::Constraint("economy_read_row_key".into()))?;
                out.push((id, key.clone(), row.clone()));
            }
        }
        out.sort_by_key(|(id, _, _)| *id);
        Ok(out.into_iter().map(|(_, key, row)| (key, row)).collect())
    }
}

fn frozen(
    lease: LeaseKey,
    kind: &str,
    key: &str,
    row: &StoredRow,
) -> crate::application::economy::FrozenRow {
    crate::application::economy::FrozenRow {
        lease,
        row_kind: kind.to_owned(),
        row_key: key.to_owned(),
        revision: row.revision,
        columns: row.columns.clone(),
    }
}

#[async_trait]
impl crate::application::economy_reads::EconomyReadRepository for InMemoryEconomyStore {
    async fn db_now_ms(&self) -> Result<i64, EconomyError> {
        Ok(self.lock().now_ms)
    }

    async fn container_items(
        &self,
        owner_kind: &str,
        character: Uuid,
    ) -> Result<Vec<crate::application::economy_reads::ContainerItem>, EconomyError> {
        let state = self.lock();
        let owner = character.to_string();
        state
            .items_where(|row| {
                col_str(row, "owner_kind") == owner_kind && col_str(row, "character_id") == owner
            })?
            .iter()
            .map(|(key, row)| item_of(key, row))
            .collect()
    }

    async fn mail_list(
        &self,
        character: Uuid,
        outbox: bool,
    ) -> Result<Vec<crate::application::economy_reads::MailRow>, EconomyError> {
        let state = self.lock();
        let now = state.now_ms;
        let mut rows: Vec<_> = state
            .mails()?
            .into_iter()
            .filter(|m| m.expires_unix_ms > now)
            .filter(|m| {
                if outbox {
                    m.sender_id == character && !m.deleted_by_sender
                } else {
                    m.receiver_id == character && !m.deleted_by_receiver
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            b.created_unix_ms
                .cmp(&a.created_unix_ms)
                .then(a.id.cmp(&b.id))
        });
        Ok(rows)
    }

    async fn mail(
        &self,
        character: Uuid,
        message: Uuid,
    ) -> Result<Option<crate::application::economy_reads::MailRow>, EconomyError> {
        let state = self.lock();
        let now = state.now_ms;
        let Some(row) = state
            .rows
            .get(&("mail_message".to_owned(), message.to_string()))
        else {
            return Ok(None);
        };
        let mail = state.mail_row(message, row)?;
        let visible = (mail.receiver_id == character && !mail.deleted_by_receiver)
            || (mail.sender_id == character && !mail.deleted_by_sender);
        Ok((visible && mail.expires_unix_ms > now).then_some(mail))
    }

    async fn mail_attachments(
        &self,
        message: Uuid,
    ) -> Result<Vec<crate::application::economy_reads::ContainerItem>, EconomyError> {
        let state = self.lock();
        let id = message.to_string();
        state
            .items_where(|row| col_str(row, "mail_id") == id)?
            .iter()
            .map(|(key, row)| item_of(key, row))
            .collect()
    }

    async fn expired_mail(
        &self,
        limit: u32,
    ) -> Result<Vec<crate::application::economy_reads::MailRow>, EconomyError> {
        let state = self.lock();
        let now = state.now_ms;
        let mut rows: Vec<_> = state
            .mails()?
            .into_iter()
            .filter(|m| m.expires_unix_ms <= now)
            .collect();
        rows.sort_by(|a, b| {
            a.expires_unix_ms
                .cmp(&b.expires_unix_ms)
                .then(a.id.cmp(&b.id))
        });
        rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        Ok(rows)
    }

    async fn mail_exists(&self, message: Uuid) -> Result<bool, EconomyError> {
        Ok(self
            .lock()
            .rows
            .contains_key(&("mail_message".to_owned(), message.to_string())))
    }

    async fn auctions(
        &self,
        instance_npc: u32,
    ) -> Result<Vec<crate::application::economy_reads::AuctionRow>, EconomyError> {
        let state = self.lock();
        let mut out = Vec::new();
        for ((kind, key), row) in &state.rows {
            if kind != "item_auction" || col_u64(row, "instance_npc") != u64::from(instance_npc) {
                continue;
            }
            out.push(crate::application::economy_reads::AuctionRow {
                id: key
                    .parse()
                    .map_err(|_| EconomyError::Constraint("economy_read_row_key".into()))?,
                instance_npc,
                item_index: narrow(col_u64(row, "item_index"))?,
                state: narrow(col_u64(row, "state"))?,
                highest_bidder: col_uuid(row, "highest_bidder"),
                highest_bid: row
                    .columns
                    .get("highest_bid")
                    .and_then(serde_json::Value::as_u64),
                start_unix_ms: col_i64(row, "start_unix_ms"),
                end_unix_ms: col_i64(row, "end_unix_ms"),
                revision: row.revision,
            });
        }
        out.sort_by_key(|a| (a.start_unix_ms, a.id));
        Ok(out)
    }

    async fn bids(
        &self,
        auction: u64,
    ) -> Result<Vec<crate::application::economy_reads::BidRow>, EconomyError> {
        let state = self.lock();
        let mut out: Vec<_> = state
            .rows
            .iter()
            .filter(|((kind, _), row)| {
                kind == "item_auction_bid" && col_u64(row, "auction_id") == auction
            })
            .filter_map(|(_, row)| {
                Some(crate::application::economy_reads::BidRow {
                    auction_id: auction,
                    character_id: col_uuid(row, "character_id")?,
                    last_bid: col_i64(row, "last_bid"),
                })
            })
            .collect();
        out.sort_by_key(|b| b.character_id);
        Ok(out)
    }

    async fn manor(
        &self,
        castle: u32,
    ) -> Result<crate::application::economy_reads::ManorRows, EconomyError> {
        let state = self.lock();
        let mut rows = crate::application::economy_reads::ManorRows::default();
        for ((kind, _), row) in &state.rows {
            if col_u64(row, "castle_id") != u64::from(castle) {
                continue;
            }
            match kind.as_str() {
                "manor_seed_production" => rows.seeds.push((
                    narrow(col_u64(row, "seed_id"))?,
                    col_u64(row, "price"),
                    col_u64(row, "start_amount"),
                    col_u64(row, "amount"),
                )),
                "manor_crop_procure" => rows.crops.push((
                    narrow(col_u64(row, "crop_id"))?,
                    col_u64(row, "price"),
                    col_u64(row, "start_amount"),
                    col_u64(row, "amount"),
                    narrow(col_u64(row, "reward_type"))?,
                )),
                _ => {},
            }
        }
        rows.seeds.sort_unstable();
        rows.crops.sort_unstable();
        Ok(rows)
    }

    async fn delivery(
        &self,
        id: Uuid,
    ) -> Result<Option<crate::application::economy_reads::DeliveryState>, EconomyError> {
        Ok(self.lock().deliveries.get(&id).map(|(row, done, _)| {
            crate::application::economy_reads::DeliveryState {
                row: row.clone(),
                pending: done.is_none(),
            }
        }))
    }

    async fn delivery_targets(&self) -> Result<Vec<Uuid>, EconomyError> {
        let state = self.lock();
        let targets: BTreeSet<Uuid> = state
            .deliveries
            .values()
            .filter(|(_, done, _)| done.is_none())
            .map(|(row, _, _)| row.target_character)
            .collect();
        Ok(targets.into_iter().collect())
    }

    async fn mailbox_counts(&self, character: Uuid) -> Result<(u32, u32), EconomyError> {
        let state = self.lock();
        let mails = state.mails()?;
        let inbox = mails
            .iter()
            .filter(|m| m.receiver_id == character && !m.deleted_by_receiver)
            .count();
        let outbox = mails
            .iter()
            .filter(|m| m.sender_id == character && !m.deleted_by_sender)
            .count();
        Ok((
            u32::try_from(inbox).map_err(|_| EconomyError::Constraint("mailbox count".into()))?,
            u32::try_from(outbox).map_err(|_| EconomyError::Constraint("mailbox count".into()))?,
        ))
    }

    async fn open_ground_refund(
        &self,
        zone: u32,
        epoch: u64,
    ) -> Result<Vec<crate::application::economy_reads::OpenChain>, EconomyError> {
        let state = self.lock();
        let mut last: BTreeMap<(String, Uuid, u64), &OwnerLedgerRow> = BTreeMap::new();
        for row in state
            .ledger
            .iter()
            .filter(|r| r.zone_id == zone && matches!(r.owner_kind.as_str(), "ground" | "refund"))
        {
            last.insert((row.owner_kind.clone(), row.owner_id, row.instance_id), row);
        }
        let mut open: Vec<_> = last
            .into_values()
            .filter(|row| row.epoch < epoch && row.delta.get("after").is_some_and(|a| !a.is_null()))
            .map(|row| crate::application::economy_reads::OpenChain {
                epoch: row.epoch,
                owner_kind: row.owner_kind.clone(),
                owner: row.owner_id,
                instance: row.instance_id,
                after: row.delta.get("after").cloned().unwrap_or_default(),
            })
            .collect();
        open.sort_by(|a, b| {
            (a.epoch, &a.owner_kind, a.owner, a.instance).cmp(&(
                b.epoch,
                &b.owner_kind,
                b.owner,
                b.instance,
            ))
        });
        Ok(open)
    }

    async fn ground_refund_chains(
        &self,
        zone: u32,
    ) -> Result<Vec<crate::application::economy_reads::ChainEnd>, EconomyError> {
        let state = self.lock();
        let mut last: BTreeMap<(String, Uuid, u64), &OwnerLedgerRow> = BTreeMap::new();
        for row in state
            .ledger
            .iter()
            .filter(|r| r.zone_id == zone && matches!(r.owner_kind.as_str(), "ground" | "refund"))
        {
            last.insert((row.owner_kind.clone(), row.owner_id, row.instance_id), row);
        }
        Ok(last
            .into_values()
            .map(|row| crate::application::economy_reads::ChainEnd {
                epoch: row.epoch,
                tick: row.tick,
                owner_kind: row.owner_kind.clone(),
                owner: row.owner_id,
                instance: row.instance_id,
                terminal: row
                    .delta
                    .get("after")
                    .is_none_or(serde_json::Value::is_null),
            })
            .collect())
    }

    async fn lease_epochs(&self, zone: u32, epoch: u64) -> Result<Vec<u64>, EconomyError> {
        let state = self.lock();
        let epochs: BTreeSet<u64> = state
            .leases
            .values()
            .filter(|g| g.zone_id == zone && g.epoch < epoch)
            .map(|g| g.epoch)
            .collect();
        Ok(epochs.into_iter().collect())
    }

    async fn pre_image(
        &self,
        keys: &[LeaseKey],
    ) -> Result<Vec<crate::application::economy::FrozenRow>, EconomyError> {
        use crate::application::economy::LeaseKind;
        let state = self.lock();
        let mut out = Vec::new();
        for key in keys {
            let uuid = Uuid::from_u128(key.owner).to_string();
            match key.kind {
                LeaseKind::Warehouse | LeaseKind::Freight => {
                    let owner_kind = if key.kind == LeaseKind::Warehouse {
                        "warehouse"
                    } else {
                        "freight"
                    };
                    for (k, row) in state.items_where(|r| {
                        col_str(r, "owner_kind") == owner_kind && col_str(r, "character_id") == uuid
                    })? {
                        out.push(frozen(*key, "item_instance", &k, &row));
                    }
                },
                LeaseKind::Mail => {
                    if let Some(row) = state.rows.get(&("mail_message".to_owned(), uuid.clone())) {
                        out.push(frozen(*key, "mail_message", &uuid, row));
                    }
                    for (k, row) in state.items_where(|r| col_str(r, "mail_id") == uuid)? {
                        out.push(frozen(*key, "item_instance", &k, &row));
                    }
                },
                LeaseKind::Delivery => {
                    let id = Uuid::from_u128(key.owner);
                    if let Some((row, done, revision)) = state.deliveries.get(&id) {
                        let mut columns = serde_json::Map::new();
                        columns.insert(
                            "target_character".into(),
                            row.target_character.to_string().into(),
                        );
                        columns.insert("kind".into(), row.kind.name().into());
                        columns.insert("payload".into(), row.payload.clone());
                        columns.insert(
                            "status".into(),
                            if done.is_some() {
                                "delivered"
                            } else {
                                "pending"
                            }
                            .into(),
                        );
                        out.push(frozen(
                            *key,
                            "economy_delivery",
                            &uuid,
                            &StoredRow {
                                revision: *revision,
                                columns,
                            },
                        ));
                    }
                    for (k, row) in state.items_where(|r| col_str(r, "delivery_id") == uuid)? {
                        out.push(frozen(*key, "item_instance", &k, &row));
                    }
                },
                LeaseKind::Auction => {
                    let auction = u64::try_from(key.owner)
                        .map_err(|_| EconomyError::Constraint("auction id".into()))?;
                    if let Some(row) = state
                        .rows
                        .get(&("item_auction".to_owned(), auction.to_string()))
                    {
                        out.push(frozen(*key, "item_auction", &auction.to_string(), row));
                    }
                    for ((kind, k), row) in &state.rows {
                        if kind == "item_auction_bid" && col_u64(row, "auction_id") == auction {
                            out.push(frozen(*key, "item_auction_bid", k, row));
                        }
                    }
                    let owners: BTreeSet<Uuid> = state
                        .rows
                        .iter()
                        .filter(|((kind, _), row)| {
                            kind == "item_auction_bid" && col_u64(row, "auction_id") == auction
                        })
                        .filter_map(|(_, row)| col_uuid(row, "character_id"))
                        .map(|c| crate::application::economy::auction_bid_owner(auction, c))
                        .collect();
                    // Per escrow owner in owner order, then by id (the Postgres adapter's order).
                    for owner in &owners {
                        for (k, row) in state.items_where(|r| {
                            col_str(r, "owner_kind") == "auction_bid"
                                && col_uuid(r, "container_id") == Some(*owner)
                        })? {
                            out.push(frozen(*key, "item_instance", &k, &row));
                        }
                    }
                },
                LeaseKind::Manor => {
                    // Seeds then crops, each by id (the Postgres adapter's order).
                    let castle = key.owner;
                    for (kind, id) in [
                        ("manor_seed_production", "seed_id"),
                        ("manor_crop_procure", "crop_id"),
                    ] {
                        let mut rows: Vec<_> = state
                            .rows
                            .iter()
                            .filter(|((k, _), row)| {
                                k == kind && u128::from(col_u64(row, "castle_id")) == castle
                            })
                            .collect();
                        rows.sort_by_key(|(_, row)| col_u64(row, id));
                        for ((_, k), row) in rows {
                            out.push(frozen(*key, kind, k, row));
                        }
                    }
                },
                LeaseKind::Mailbox => {},
            }
        }
        Ok(out)
    }
}
