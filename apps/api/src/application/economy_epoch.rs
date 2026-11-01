//! Story 4.7 (design §11.6): epoch close and `EpochReconcile`, API side.
//!
//! - **Graceful close.** The zone writes `destroy` rows for every ground item and refund entry
//!   in its final tick (cause `epoch_close:<epoch>`). The API then releases every lease of that
//!   epoch ([`EpochReconciler::close`]).
//! - **Abandoned epoch.** Before the next epoch of the zone admits a player,
//!   [`EpochReconciler::before_admission`] reads the ground and refund chains that are still live
//!   in the ledger. It releases the leases of every older epoch and injects one
//!   `EpochReconcile` system job per abandoned epoch, with that manifest frozen
//!   (`decisions.manifest`, E12). The host zone writes one `destroy` row per manifest entry
//!   (cause `epoch_void:<epoch>`, [`void_rows`]), so every live instance in the ledger gets its
//!   terminal fact.
//!
//! The ledger is the manifest source (P5-D23): ground and refund owners have no `item_instances`
//! rows (E17, B6), and a chain is live exactly when its last row is not terminal. When a fenced
//! snapshot of an abandoned epoch exists, its ground and refund instances must agree with the
//! ledger: each one is live in the ledger, or its chain ended after the snapshot tick. Any other
//! instance is an invariant failure, and no reconcile runs (`epoch_reconcile_snapshot_mismatch`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use uuid::Uuid;

use super::economy::{
    EconomyError, FrozenInputs, LeaseGrant, LeaseRepository, OwnerLedgerRow, SystemJob,
};
use super::economy_reads::{ChainEnd, EconomyReadRepository, OpenChain};
use super::economy_scheduler::{ScheduledJob, SystemJobPort};
use super::replay_log::ZoneSnapshotStore;
use crate::domain::zone::phase5_command::Phase5Command;
use crate::domain::zone::{ZoneId, ZoneSnapshot};

/// What [`EpochReconciler::before_admission`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconcileReport {
    /// `(abandoned epoch, live ground/refund instances)` per injected job, ascending.
    pub reconciled: Vec<(u64, usize)>,
    /// Every lease grant of an older epoch, released.
    pub released: Vec<LeaseGrant>,
}

/// The manifest entry JSON of one live chain (integers and strings only).
fn manifest_entry(chain: &OpenChain) -> serde_json::Value {
    serde_json::json!({
        "owner_kind": chain.owner_kind,
        "owner": chain.owner.to_string(),
        "instance": chain.instance,
        "item": chain.after,
    })
}

/// The `destroy` rows an `EpochReconcile` of `abandoned` writes in `(zone, epoch, tick)`: one
/// terminal row per live chain, ordinal 0, cause `epoch_void:<abandoned>` (design §11.3).
#[must_use]
pub fn void_rows(
    zone: u32,
    epoch: u64,
    tick: u64,
    abandoned: u64,
    manifest: &[OpenChain],
) -> Vec<OwnerLedgerRow> {
    manifest
        .iter()
        .map(|chain| {
            let count = chain
                .after
                .get("count")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            OwnerLedgerRow {
                owner_kind: chain.owner_kind.clone(),
                owner_id: chain.owner,
                zone_id: zone,
                epoch,
                tick,
                revision: 0,
                op: "destroy".into(),
                instance_id: chain.instance,
                operation_ordinal: 0,
                delta: serde_json::json!({
                    "before": chain.after,
                    "after": null,
                    "count_delta": count.checked_neg().unwrap_or(0),
                    "causes": [format!("epoch_void:{abandoned}")],
                }),
                receipt_ref: None,
            }
        })
        .collect()
}

/// The ground and refund instances of a fenced snapshot: `(snapshot tick, {(owner kind, owner,
/// instance)})`. A pre-V7 snapshot has none.
#[must_use]
pub fn fenced_instances(snapshot: &ZoneSnapshot) -> (u64, BTreeSet<(String, Uuid, u64)>) {
    let mut out = BTreeSet::new();
    if let Some(block) = &snapshot.phase5 {
        for ground in block.ground.values() {
            out.insert(("ground".to_owned(), ground.entity.as_uuid(), ground.item.id.get()));
        }
        for (player, state) in &block.players {
            for item in &state.refund {
                out.insert(("refund".to_owned(), player.as_uuid(), item.id.get()));
            }
        }
    }
    (snapshot.tick.0, out)
}

/// P5-D23: every fenced instance is live in the ledger, or its chain ended after the snapshot.
pub fn check_fenced(
    epoch: u64,
    tick: u64,
    fenced: &BTreeSet<(String, Uuid, u64)>,
    chains: &[ChainEnd],
) -> Result<(), EconomyError> {
    let ends: BTreeMap<(&str, Uuid, u64), &ChainEnd> = chains
        .iter()
        .map(|c| ((c.owner_kind.as_str(), c.owner, c.instance), c))
        .collect();
    for (kind, owner, instance) in fenced {
        let agrees = ends
            .get(&(kind.as_str(), *owner, *instance))
            .is_some_and(|end| {
                !end.terminal || end.epoch > epoch || (end.epoch == epoch && end.tick > tick)
            });
        if !agrees {
            return Err(EconomyError::Constraint(format!(
                "epoch_reconcile_snapshot_mismatch:{epoch}:{kind}:{owner}:{instance}"
            )));
        }
    }
    Ok(())
}

/// Epoch close and abandoned-epoch reconciliation over the lease, read and system-job ports.
pub struct EpochReconciler {
    leases: Arc<dyn LeaseRepository>,
    reads: Arc<dyn EconomyReadRepository>,
    zones: Arc<dyn SystemJobPort>,
    snapshots: Option<Arc<dyn ZoneSnapshotStore>>,
}

impl EpochReconciler {
    /// The reconciler over its ports.
    #[must_use]
    pub fn new(
        leases: Arc<dyn LeaseRepository>,
        reads: Arc<dyn EconomyReadRepository>,
        zones: Arc<dyn SystemJobPort>,
    ) -> Self {
        Self {
            leases,
            reads,
            zones,
            snapshots: None,
        }
    }

    /// Checks each abandoned epoch's fenced snapshot against the ledger manifest (P5-D23).
    #[must_use]
    pub fn with_snapshots(mut self, snapshots: Arc<dyn ZoneSnapshotStore>) -> Self {
        self.snapshots = Some(snapshots);
        self
    }

    /// P5-D23 for every older epoch with a fenced snapshot, before anything is released or
    /// queued.
    async fn guard(
        &self,
        zone: u32,
        epoch: u64,
        abandoned: &BTreeSet<u64>,
    ) -> Result<(), EconomyError> {
        let Some(store) = &self.snapshots else {
            return Ok(());
        };
        let mut epochs = abandoned.clone();
        epochs.extend(
            store
                .unresolved(ZoneId(zone))
                .await
                .map_err(EconomyError::Other)?
                .into_iter()
                .map(|e| e.epoch)
                .filter(|e| *e < epoch),
        );
        let chains = self.reads.ground_refund_chains(zone).await?;
        for older in epochs {
            let Some(row) = store
                .get(ZoneId(zone), older)
                .await
                .map_err(EconomyError::Other)?
            else {
                continue;
            };
            let snapshot = super::replay_log::decode_snapshot(&row.snapshot)
                .map_err(|e| EconomyError::Other(e.into()))?;
            let (tick, fenced) = fenced_instances(&snapshot);
            check_fenced(older, tick, &fenced, &chains)?;
        }
        Ok(())
    }

    /// Graceful close: the closing epoch's final tick is committed; release all its leases.
    pub async fn close(&self, zone: u32, epoch: u64) -> Result<Vec<LeaseGrant>, EconomyError> {
        self.leases.release_epoch(zone, epoch).await
    }

    /// Before epoch `epoch` of `zone` admits a player: release the leases of every older epoch
    /// and inject one `EpochReconcile` per abandoned epoch with live ground/refund chains. A
    /// refused job is an error: admission waits until the reconcile is queued.
    pub async fn before_admission(
        &self,
        zone: u32,
        epoch: u64,
    ) -> Result<ReconcileReport, EconomyError> {
        let mut report = ReconcileReport::default();
        let mut by_epoch: BTreeMap<u64, Vec<OpenChain>> = BTreeMap::new();
        for chain in self.reads.open_ground_refund(zone, epoch).await? {
            by_epoch.entry(chain.epoch).or_default().push(chain);
        }
        // P5-D23: a fenced snapshot that disagrees with the ledger stops everything here.
        self.guard(zone, epoch, &by_epoch.keys().copied().collect())
            .await?;
        for older in self.reads.lease_epochs(zone, epoch).await? {
            report
                .released
                .extend(self.leases.release_epoch(zone, older).await?);
        }
        for (abandoned, manifest) in by_epoch {
            let job = SystemJob::EpochReconcile(zone, abandoned);
            let mut decisions = serde_json::Map::new();
            decisions.insert("abandoned_epoch".into(), abandoned.into());
            decisions.insert(
                "manifest".into(),
                manifest
                    .iter()
                    .map(manifest_entry)
                    .collect::<Vec<_>>()
                    .into(),
            );
            let scheduled = ScheduledJob {
                job,
                command: Phase5Command::EpochReconcile {
                    request_key: job.request_key(),
                    abandoned_epoch: abandoned,
                },
                frozen: FrozenInputs {
                    leases: Vec::new(),
                    rows: Vec::new(),
                    decisions,
                },
                zone: (zone, epoch),
            };
            self.zones.enqueue(scheduled).await.map_err(|refused| {
                EconomyError::Transient(format!("epoch reconcile not queued: {}", refused.0))
            })?;
            report.reconciled.push((abandoned, manifest.len()));
        }
        Ok(report)
    }
}

/// The manifest of a frozen `EpochReconcile` (what the host zone reads back).
pub fn frozen_manifest(frozen: &FrozenInputs) -> Result<Vec<OpenChain>, EconomyError> {
    let bad = || EconomyError::Constraint("epoch_reconcile_manifest".into());
    let epoch = frozen
        .decisions
        .get("abandoned_epoch")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(bad)?;
    frozen
        .decisions
        .get("manifest")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(bad)?
        .iter()
        .map(|entry| {
            Ok(OpenChain {
                epoch,
                owner_kind: entry
                    .get("owner_kind")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(bad)?
                    .to_owned(),
                owner: entry
                    .get("owner")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|s| s.parse::<Uuid>().ok())
                    .ok_or_else(bad)?,
                instance: entry
                    .get("instance")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(bad)?,
                after: entry.get("item").cloned().ok_or_else(bad)?,
            })
        })
        .collect()
}
