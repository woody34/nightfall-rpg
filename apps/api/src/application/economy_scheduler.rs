//! Story 4.5: the API schedulers and system-job injection (design §10.0, §10.3–10.5; E31, E33,
//! E38, E41).
//!
//! The scheduler only **reads** (E31): it detects due jobs on DB time, takes the job's lease set
//! all-or-nothing, freezes the pre-image and enqueues the system command into the job's host
//! zone through [`SystemJobPort`]. The zone's checkpoint transaction is the only writer of the
//! rows. A `CONTAINER_BUSY` (or transient) preflight holds nothing and re-queues the job to the
//! next tick; a job whose host zone is not running waits. After an epoch-fenced lease release,
//! [`EconomyScheduler::recover`] checks each system job's commit marker and re-enqueues the
//! uncommitted ones.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use super::economy::{
    derived_row_id, sets, DeliveryRepository, DerivedRow, EconomyError, FrozenInputs, LeaseGrant,
    LeaseKey, LeaseRepository, SystemJob, NS_DELIVERY,
};
use super::economy_reads::{AuctionRow, EconomyReadRepository};
use crate::domain::zone::phase5_command::{AuctionTransition, Phase5Command};
use crate::domain::zone::EntityId;

/// Where a system job executes (design §10.0 "Executing zone").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostZone {
    /// The zone where the target character is admitted (`DeliverPending`).
    Character(Uuid),
    /// The configured economy host zone (`MailExpire`).
    EconomyHost,
    /// The zone hosting the auctioneer NPC of the instance (`AuctionSchedule`).
    Auctioneer(u32),
}

/// A preflighted system job ready for its host zone.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledJob {
    /// The job identity (its request key is deterministic).
    pub job: SystemJob,
    /// The zone command.
    pub command: Phase5Command,
    /// The frozen leases, pre-images and DB-time decisions (part of the command bytes).
    pub frozen: FrozenInputs,
    /// The executing `(zone, epoch)` the leases were taken for.
    pub zone: (u32, u64),
}

/// Why the host zone did not take a job.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("system job refused by the host zone: {0}")]
pub struct JobRefused(pub String);

/// The runtime port into the zones (the D30 injection port): host lookup and enqueue.
#[async_trait]
pub trait SystemJobPort: Send + Sync {
    /// The running `(zone, epoch)` that hosts the job, or `None` (the job waits).
    async fn host(&self, host: &HostZone) -> Option<(u32, u64)>;
    /// Enqueue the command; the zone executes it on a later tick.
    async fn enqueue(&self, job: ScheduledJob) -> Result<(), JobRefused>;
}

/// The API-only `MailNotification` gateway sink (design §12.3): sends to the receiver's
/// session when the receiver is admitted somewhere; returns whether it was delivered.
#[async_trait]
pub trait MailNotifier: Send + Sync {
    /// Deliver the notification to an admitted receiver.
    async fn notify(&self, receiver: Uuid, notification: MailNotificationView) -> bool;
}

/// The `MailNotification` world event body (`WorldEvent` 42), mapped to protobuf by the interface.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct MailNotificationView {
    pub message_id: Uuid,
    pub sender_name: String,
    pub subject: String,
    pub has_attachments: bool,
    pub cod_adena: u64,
}

/// One fixture auction (B4: the fixture schedule replaces the wall-clock generator).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct FixtureAuction {
    pub auction_id: u64,
    pub instance: u32,
    pub round: u32,
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
    /// A pinned item index; `None` draws at create (draw #14).
    pub item_index: Option<u32>,
}

impl FixtureAuction {
    /// The hashed fixture schedule of the epoch's `Phase5Rules`, in (instance, round) order.
    #[must_use]
    pub fn from_rules(rules: &crate::domain::zone::Phase5Rules) -> Vec<Self> {
        let mut out: Vec<Self> = rules
            .parts()
            .auction
            .fixture_schedule
            .auctions
            .iter()
            .map(|row| Self {
                auction_id: row.auction_id,
                instance: row.instance,
                round: row.round,
                start_unix_ms: row.start_epoch_ms,
                end_unix_ms: row.end_epoch_ms,
                item_index: row.item.parse().ok(),
            })
            .collect();
        out.sort_by_key(|a| (a.instance, a.round, a.auction_id));
        out
    }
}

/// The due `AuctionSchedule` transitions on DB time (design §10.5), at most one per auction:
/// - `Create`: a fixture auction without a row whose earlier rounds all have rows, before its end;
/// - `Start`: CREATED and `now ≥ start`;
/// - `Finish`: STARTED and `now ≥ end` (the row's end, extensions included, P21);
/// - `Purge`: FINISHED and `now ≥ start + expired_after` (14 days; unclaimed escrow is destroyed).
#[must_use]
pub fn auction_transitions(
    schedule: &[FixtureAuction],
    rows: &[AuctionRow],
    now_ms: i64,
    expired_after_ms: i64,
) -> Vec<(FixtureAuction, AuctionTransition)> {
    let by_id: BTreeMap<u64, &AuctionRow> = rows.iter().map(|row| (row.id, row)).collect();
    let mut out = Vec::new();
    let mut earlier_missing: BTreeSet<u32> = BTreeSet::new();
    for entry in schedule {
        match by_id.get(&entry.auction_id) {
            None => {
                if !earlier_missing.contains(&entry.instance) && now_ms < entry.end_unix_ms {
                    out.push((entry.clone(), AuctionTransition::Create));
                }
                earlier_missing.insert(entry.instance);
            },
            Some(row) => {
                let due = match row.state {
                    0 if now_ms >= row.start_unix_ms => Some(AuctionTransition::Start),
                    1 if now_ms >= row.end_unix_ms => Some(AuctionTransition::Finish),
                    2 if row
                        .start_unix_ms
                        .checked_add(expired_after_ms)
                        .is_some_and(|at| now_ms >= at) =>
                    {
                        Some(AuctionTransition::Purge)
                    },
                    _ => None,
                };
                if let Some(transition) = due {
                    out.push((entry.clone(), transition));
                }
            },
        }
    }
    out
}

/// What one scheduler tick did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TickReport {
    /// Jobs preflighted and enqueued this tick.
    pub enqueued: Vec<SystemJob>,
    /// Jobs refused `CONTAINER_BUSY` (or transient) at preflight: nothing held, retried next tick.
    pub busy: Vec<SystemJob>,
    /// Jobs whose host zone is not running (or refused the enqueue): nothing held.
    pub waiting: Vec<SystemJob>,
}

/// What recovery found for the released system-job leases of a fenced epoch.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Jobs whose commit marker is present.
    pub committed: Vec<SystemJob>,
    /// Jobs whose marker is absent: re-enqueued by the next tick.
    pub requeued: Vec<SystemJob>,
}

/// The scheduler (one per API process; it runs beside the zone actors, design §10.0).
pub struct EconomyScheduler {
    leases: Arc<dyn LeaseRepository>,
    reads: Arc<dyn EconomyReadRepository>,
    deliveries: Arc<dyn DeliveryRepository>,
    zones: Arc<dyn SystemJobPort>,
    schedule: Vec<FixtureAuction>,
    expired_after_ms: i64,
    mail_batch: u32,
    in_flight: BTreeMap<Uuid, SystemJob>,
    delivery_targets: BTreeMap<Uuid, Uuid>,
}

impl EconomyScheduler {
    /// A scheduler over the ports, with the epoch's fixture schedule and auction expiry.
    #[must_use]
    pub fn new(
        leases: Arc<dyn LeaseRepository>,
        reads: Arc<dyn EconomyReadRepository>,
        deliveries: Arc<dyn DeliveryRepository>,
        zones: Arc<dyn SystemJobPort>,
        schedule: Vec<FixtureAuction>,
        expired_after_days: u64,
    ) -> Self {
        let expired_after_ms = i64::try_from(expired_after_days)
            .ok()
            .and_then(|days| days.checked_mul(86_400_000))
            .unwrap_or(i64::MAX);
        Self {
            leases,
            reads,
            deliveries,
            zones,
            schedule,
            expired_after_ms,
            mail_batch: 64,
            in_flight: BTreeMap::new(),
            delivery_targets: BTreeMap::new(),
        }
    }

    /// Jobs enqueued and not yet reported committed or rejected.
    #[must_use]
    pub fn in_flight(&self) -> Vec<SystemJob> {
        self.in_flight.values().copied().collect()
    }

    /// The zone reported the job's checkpoint commit (or its rejection, which released the
    /// whole lease set in the draft): the job leaves the in-flight set.
    pub fn completed(&mut self, job: SystemJob) {
        self.in_flight.remove(&job.request_key());
    }

    /// One scheduler pass at the zone tick cadence: deliveries, mail expiry, auctions.
    pub async fn tick(&mut self) -> Result<TickReport, EconomyError> {
        let mut report = TickReport::default();
        self.deliveries_pass(&mut report).await?;
        self.mail_pass(&mut report).await?;
        self.auction_pass(&mut report).await?;
        Ok(report)
    }

    /// The D7 admission materialization slot: only the lowest-id pending delivery of the
    /// target runs there (E38); a busy one follows on a later tick and never blocks admission.
    pub async fn admission_delivery(
        &mut self,
        target: Uuid,
        zone: (u32, u64),
    ) -> Result<Option<ScheduledJob>, EconomyError> {
        let Some(head) = self.deliveries.head(target).await? else {
            return Ok(None);
        };
        let job = SystemJob::DeliverPending(head.id);
        if self.in_flight.contains_key(&job.request_key()) {
            return Ok(None);
        }
        let keys = sets::deliver_pending(head.id, head.kind, target);
        let command = Phase5Command::DeliverPending {
            request_key: job.request_key(),
            target: EntityId::from_uuid(target),
            delivery_id: head.id,
        };
        match self
            .preflight(job, command, keys, zone, serde_json::Map::new())
            .await?
        {
            Some(scheduled) => {
                self.in_flight.insert(job.request_key(), job);
                Ok(Some(scheduled))
            },
            None => Ok(None),
        }
    }

    async fn deliveries_pass(&mut self, report: &mut TickReport) -> Result<(), EconomyError> {
        for target in self.reads.delivery_targets().await? {
            // E41: at most one delivery per target per tick, and the queue head only.
            if self.in_flight.values().any(|job| matches!(job, SystemJob::DeliverPending(d) if self.target_of(*d) == Some(target))) {
                continue;
            }
            let Some(head) = self.deliveries.head(target).await? else {
                continue;
            };
            let job = SystemJob::DeliverPending(head.id);
            let keys = sets::deliver_pending(head.id, head.kind, target);
            let command = Phase5Command::DeliverPending {
                request_key: job.request_key(),
                target: EntityId::from_uuid(target),
                delivery_id: head.id,
            };
            self.dispatch(
                report,
                job,
                command,
                keys,
                HostZone::Character(target),
                serde_json::Map::new(),
            )
            .await?;
        }
        Ok(())
    }

    fn target_of(&self, delivery: Uuid) -> Option<Uuid> {
        self.delivery_targets.get(&delivery).copied()
    }

    async fn mail_pass(&mut self, report: &mut TickReport) -> Result<(), EconomyError> {
        for mail in self.reads.expired_mail(self.mail_batch).await? {
            let job = SystemJob::MailExpire(mail.id);
            if self.in_flight.contains_key(&job.request_key()) {
                continue;
            }
            // design §10.3: delivery_id = UUIDv5(NS_DELIVERY, message_id ‖ "expiry").
            let delivery = mail
                .has_attachments
                .then(|| Uuid::new_v5(&NS_DELIVERY, format!("{}:expiry", mail.id).as_bytes()));
            let keys = sets::mail_expire(mail.id, mail.sender_id, mail.receiver_id, delivery);
            let mut decisions = serde_json::Map::new();
            decisions.insert("expired".into(), true.into());
            if let Some(delivery) = delivery {
                decisions.insert("delivery_id".into(), delivery.to_string().into());
            }
            let command = Phase5Command::MailExpire {
                request_key: job.request_key(),
                message_id: mail.id,
            };
            self.dispatch(report, job, command, keys, HostZone::EconomyHost, decisions)
                .await?;
        }
        Ok(())
    }

    async fn auction_pass(&mut self, report: &mut TickReport) -> Result<(), EconomyError> {
        let now = self.reads.db_now_ms().await?;
        let instances: BTreeSet<u32> = self.schedule.iter().map(|a| a.instance).collect();
        for instance in instances {
            let schedule: Vec<FixtureAuction> = self
                .schedule
                .iter()
                .filter(|a| a.instance == instance)
                .cloned()
                .collect();
            let rows = self.reads.auctions(instance).await?;
            for (entry, transition) in
                auction_transitions(&schedule, &rows, now, self.expired_after_ms)
            {
                let job = SystemJob::AuctionSchedule(entry.auction_id, transition);
                if self.in_flight.contains_key(&job.request_key()) {
                    continue;
                }
                let mut decisions = serde_json::Map::new();
                decisions.insert("transition".into(), transition.name().into());
                let keys = if transition == AuctionTransition::Finish {
                    let delivery = derived_row_id(DerivedRow::Delivery, job.request_key(), 0);
                    decisions.insert("delivery_id".into(), delivery.to_string().into());
                    sets::auction_finish(entry.auction_id, delivery)
                } else {
                    if transition == AuctionTransition::Create {
                        decisions.insert("instance_npc".into(), entry.instance.into());
                        decisions.insert("start_unix_ms".into(), entry.start_unix_ms.into());
                        decisions.insert("end_unix_ms".into(), entry.end_unix_ms.into());
                        if let Some(item) = entry.item_index {
                            decisions.insert("item_index".into(), item.into());
                        }
                    }
                    sets::auction(entry.auction_id)
                };
                let command = Phase5Command::AuctionSchedule {
                    request_key: job.request_key(),
                    auction_id: entry.auction_id,
                    transition,
                };
                self.dispatch(
                    report,
                    job,
                    command,
                    keys,
                    HostZone::Auctioneer(instance),
                    decisions,
                )
                .await?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // One job's frozen parts, passed whole to preflight.
    async fn dispatch(
        &mut self,
        report: &mut TickReport,
        job: SystemJob,
        command: Phase5Command,
        keys: Vec<LeaseKey>,
        host: HostZone,
        decisions: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), EconomyError> {
        let Some(zone) = self.zones.host(&host).await else {
            report.waiting.push(job);
            return Ok(());
        };
        if let Some(scheduled) = self
            .preflight(job, command, keys.clone(), zone, decisions)
            .await?
        {
            if let SystemJob::DeliverPending(d) = job {
                if let HostZone::Character(target) = host {
                    self.delivery_targets.insert(d, target);
                }
            }
            if self.zones.enqueue(scheduled).await.is_ok() {
                self.in_flight.insert(job.request_key(), job);
                report.enqueued.push(job);
            } else {
                // Nothing reached a zone: release the whole set at once.
                self.leases.release(zone.0, zone.1, &keys).await?;
                report.waiting.push(job);
            }
        } else {
            report.busy.push(job);
        }
        Ok(())
    }

    /// Preflight (design §10.0 step 1): acquire the set all-or-nothing, then freeze the
    /// pre-image. `None` = `CONTAINER_BUSY` or a transient failure: nothing is held.
    async fn preflight(
        &self,
        job: SystemJob,
        command: Phase5Command,
        keys: Vec<LeaseKey>,
        zone: (u32, u64),
        decisions: serde_json::Map<String, serde_json::Value>,
    ) -> Result<Option<ScheduledJob>, EconomyError> {
        let grant = LeaseGrant {
            zone_id: zone.0,
            epoch: zone.1,
            account_id: None,
            operation: job.kind().name().to_owned(),
            request_key: job.request_key(),
            keys: keys.clone(),
        };
        match self.leases.acquire(&grant).await {
            Ok(()) => {},
            Err(EconomyError::Busy(_) | EconomyError::Transient(_)) => return Ok(None),
            Err(other) => return Err(other),
        }
        let rows = match self.reads.pre_image(&keys).await {
            Ok(rows) => rows,
            Err(error) => {
                self.leases.release(zone.0, zone.1, &keys).await?;
                return Err(error);
            },
        };
        let frozen = FrozenInputs {
            leases: keys,
            rows,
            decisions,
        };
        frozen.validate()?;
        Ok(Some(ScheduledJob {
            job,
            command,
            frozen,
            zone,
        }))
    }

    /// Lease recovery (design §10.0 step 4): for each released system-job grant, check the
    /// job's commit marker; an uncommitted job is re-enqueued by the next tick.
    pub async fn recover(
        &mut self,
        released: &[LeaseGrant],
    ) -> Result<RecoveryReport, EconomyError> {
        let mut report = RecoveryReport::default();
        for grant in released.iter().filter(|g| g.account_id.is_none()) {
            let Some(job) = self.in_flight.remove(&grant.request_key) else {
                continue;
            };
            if self.marker(job).await? {
                report.committed.push(job);
            } else {
                report.requeued.push(job);
            }
        }
        Ok(report)
    }

    /// The committed marker of a system job (design §10.0 step 4, DR6 nit 10).
    pub async fn marker(&self, job: SystemJob) -> Result<bool, EconomyError> {
        Ok(match job {
            SystemJob::DeliverPending(d) => self
                .reads
                .delivery(d)
                .await?
                .is_some_and(|state| !state.pending),
            SystemJob::MailExpire(m) => !self.reads.mail_exists(m).await?,
            SystemJob::AuctionSchedule(a, transition) => {
                let instance = self
                    .schedule
                    .iter()
                    .find(|e| e.auction_id == a)
                    .map(|e| e.instance);
                let row = match instance {
                    Some(instance) => self
                        .reads
                        .auctions(instance)
                        .await?
                        .into_iter()
                        .find(|r| r.id == a),
                    None => None,
                };
                match (transition, row) {
                    (AuctionTransition::Purge, row) => row.is_none(),
                    (AuctionTransition::Create, row) => row.is_some(),
                    (AuctionTransition::Start, Some(row)) => row.state >= 1,
                    (AuctionTransition::Finish, Some(row)) => row.state == 2,
                    (AuctionTransition::Start | AuctionTransition::Finish, None) => false,
                }
            },
            SystemJob::EpochReconcile(..) => false,
        })
    }
}

/// The API-only `MailNotification` gateway (design §10.3 commit, §12.3): after a checkpoint
/// that inserted message rows commits, notify each admitted receiver from the committed row.
pub async fn notify_new_mail(
    write: &super::economy::EconomyCheckpointWrite,
    reads: &dyn EconomyReadRepository,
    notifier: &dyn MailNotifier,
) -> Result<Vec<Uuid>, EconomyError> {
    let mut sent = Vec::new();
    for delta in write
        .deltas
        .iter()
        .filter(|d| d.row_kind == "mail_message" && d.change == super::economy::DeltaChange::Insert)
    {
        let Ok(message) = delta.row_key.parse::<Uuid>() else {
            continue;
        };
        let Some(receiver) = delta
            .columns
            .get("receiver_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|s| s.parse::<Uuid>().ok())
        else {
            continue;
        };
        let Some(row) = reads.mail(receiver, message).await? else {
            continue;
        };
        let notification = MailNotificationView {
            message_id: row.id,
            sender_name: row.sender_name,
            subject: row.subject,
            has_attachments: row.has_attachments,
            cod_adena: row.cod_adena,
        };
        if notifier.notify(receiver, notification).await {
            sent.push(message);
        }
    }
    Ok(sent)
}
