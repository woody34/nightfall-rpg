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
//! The ledger is the manifest source: ground and refund owners have no `item_instances` rows
//! (E17, B6), and a chain is live exactly when its last row is not terminal. That is the content
//! of the last fenced snapshot as far as the committed ledger goes.

use std::collections::BTreeMap;
use std::sync::Arc;

use uuid::Uuid;

use super::economy::{
    EconomyError, FrozenInputs, LeaseGrant, LeaseRepository, OwnerLedgerRow, SystemJob,
};
use super::economy_reads::{EconomyReadRepository, OpenChain};
use super::economy_scheduler::{ScheduledJob, SystemJobPort};
use crate::domain::zone::phase5_command::Phase5Command;

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

/// Epoch close and abandoned-epoch reconciliation over the lease, read and system-job ports.
pub struct EpochReconciler {
    leases: Arc<dyn LeaseRepository>,
    reads: Arc<dyn EconomyReadRepository>,
    zones: Arc<dyn SystemJobPort>,
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
        }
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
        for older in self.reads.lease_epochs(zone, epoch).await? {
            report
                .released
                .extend(self.leases.release_epoch(zone, older).await?);
        }
        let mut by_epoch: BTreeMap<u64, Vec<OpenChain>> = BTreeMap::new();
        for chain in self.reads.open_ground_refund(zone, epoch).await? {
            by_epoch.entry(chain.epoch).or_default().push(chain);
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
