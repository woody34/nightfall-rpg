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
}

/// Map-backed economy store.
#[derive(Default)]
pub struct InMemoryEconomyStore {
    state: Mutex<State>,
}

impl InMemoryEconomyStore {
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
        let mut state = self.lock();
        Self::take_transient(&mut state)?;
        // Dry-run the CAS over a copy so a violation leaves the store untouched (one transaction).
        let mut rows = state.rows.clone();
        let mut deliveries = state.deliveries.clone();
        {
            let mut scratch = State {
                rows: std::mem::take(&mut rows),
                ..State::default()
            };
            for delta in write.ordered_deltas() {
                Self::apply_delta(&mut scratch, delta)?;
            }
            rows = scratch.rows;
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
        state.deliveries = deliveries;
        state.ledger.extend(write.ledger.iter().cloned());
        state.outbox.extend(write.outbox.iter().cloned());
        for key in &write.lease_releases {
            if state
                .leases
                .get(key)
                .is_some_and(|g| g.zone_id == write.zone_id && g.epoch == write.epoch)
            {
                state.leases.remove(key);
            }
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
