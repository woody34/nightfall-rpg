//! Real equipment simulation, atomic acknowledgement and exact pending-work recovery.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects
)]
use super::*;
use crate::application::ports::{
    CheckpointError, ItemCheckpointRepository, ItemCheckpointWork, ItemInventoryRepository,
    ItemMutationOperation, ItemReceiptLookup, ItemReceiptRepository, SuccessfulLearnReceipt,
};
use crate::domain::zone::{
    EquipSlot, ItemCommand, ItemCommandKind, ObserverOutput, SessionGeneration, ZoneInput,
    ZoneState,
};
use crate::domain::{AccountId, Character, CharacterName};
use crate::infrastructure::memory::{
    AuditedFrame, InMemoryCharacterRepository, InMemorySessionAudit,
};
use crate::infrastructure::{item_data, telemetry::Metrics};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Notify;

type Attempt = (CharacterCheckpoint, Vec<DomainEvent>, ItemCheckpointWork);
struct Harness {
    repo: Arc<InMemoryCharacterRepository>,
    audit: Arc<InMemorySessionAudit>,
    service: CheckpointService,
    state: ZoneState,
    entity: EntityId,
    account: AccountId,
    session: SessionId,
}
impl Harness {
    async fn new() -> Self {
        let bundle = item_data::embedded_phase4_bundle().unwrap();
        let repo = Arc::new(
            InMemoryCharacterRepository::default()
                .with_phase4_rules(bundle.items, bundle.skills)
                .unwrap(),
        );
        let (mut state, target, _) = ZoneState::item_checkpoint_fixture();
        state.run_tick(state.draft(Vec::new())).unwrap();
        let snapshot = state.snapshot();
        let audit = Arc::new(InMemorySessionAudit::default());
        let mut service =
            CheckpointService::new(repo.clone(), audit.clone(), Arc::new(Metrics::detached()))
                .with_items(repo.clone());
        let mut account = None;
        for entity in &snapshot.entities {
            let Some(combat) = &entity.combat else {
                continue;
            };
            let CombatRole::Player {
                progression: Some(progression),
                xp,
                ..
            } = &combat.role
            else {
                continue;
            };
            let mut c = Character::create(
                progression.identity.account_id,
                CharacterName::new(&entity.name).unwrap(),
                progression.identity.race,
            );
            c.id = CharacterId::from_uuid(entity.id.as_uuid());
            c.class_state = progression.class_state.clone();
            c.appearance = progression.identity.appearance;
            c.level = combat.sheet.level();
            c.xp = *xp;
            repo.insert_for_test(c.clone());
            let phase = snapshot.phase3.as_ref().unwrap();
            let mut cp = CharacterCheckpoint {
                character_id: c.id,
                revision_seen: 0,
                level: c.level,
                xp: c.xp,
                hp: combat.hp,
                mp: combat.mp,
                alive: !entity.targeting.dead,
                position: position(entity.pos),
                idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
                class_state: Some(c.class_state),
                phase3: Some(phase.checkpoint(entity.id, Tick(1)).unwrap()),
                phase4: None,
            };
            assert_eq!(repo.checkpoint(&cp, &[]).await.unwrap(), CheckpointOutcome::Applied(1));
            cp.phase4 = Some(
                snapshot
                    .phase4
                    .as_ref()
                    .unwrap()
                    .checkpoint(entity.id, Tick(1), phase)
                    .unwrap(),
            );
            repo.insert_item_checkpoint_for_test(c.id, cp.phase4.clone().unwrap())
                .unwrap();
            cp.revision_seen = 1;
            service.players.insert(
                entity.id,
                Player {
                    revision: 1,
                    last_saved: Tick(1),
                    latest: Some(cp),
                    last_phase3: phase.entities.get(&entity.id).cloned(),
                    events: Vec::new(),
                    learning: None,
                    items: None,
                    dirty: false,
                    fenced: false,
                },
            );
            if entity.id == target.entity {
                account = Some(c.account_id);
            }
        }
        let session = SessionId::new();
        service.bind_session(target.entity, session);
        Self {
            repo,
            audit,
            service,
            state,
            entity: target.entity,
            account: account.unwrap(),
            session,
        }
    }
    fn equipment_inputs(&self) -> Vec<ZoneInput> {
        let weapon = self.state.snapshot().phase4.unwrap().players[&self.entity].paperdoll
            [&EquipSlot::Rhand];
        [
            ItemCommandKind::Unequip {
                slot: EquipSlot::Rhand,
            },
            ItemCommandKind::Equip {
                instance: weapon,
                preferred_slot: None,
            },
            ItemCommandKind::Equip {
                instance: weapon,
                preferred_slot: None,
            },
        ]
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            ZoneInput::session(
                self.entity,
                SessionGeneration(1),
                u32::try_from(index.checked_add(1).unwrap()).unwrap(),
                ZoneCommand::Item(Box::new(
                    ItemCommand::new(self.entity, Uuid::from_u128(991), kind)
                        .unwrap()
                        .with_account(self.account)
                        .unwrap(),
                )),
            )
        })
        .collect()
    }
    fn equipment_tick(&mut self) -> AppliedTick {
        self.state
            .run_tick(self.state.draft(self.equipment_inputs()))
            .unwrap()
    }
    fn acknowledgements(&self) -> usize {
        self.audit
            .frames(self.session)
            .iter()
            .filter(|frame| matches!(frame, AuditedFrame::Checkpoint(_)))
            .count()
    }
}

struct FlakyItems {
    inner: Arc<InMemoryCharacterRepository>,
    attempts: parking_lot::Mutex<Vec<Attempt>>,
    entered: Notify,
    released: Notify,
    calls: AtomicUsize,
    lookup_calls: AtomicUsize,
    lookup_failure: AtomicBool,
    after_commit: bool,
    permanent: bool,
}
impl FlakyItems {
    fn new(inner: Arc<InMemoryCharacterRepository>, after_commit: bool, permanent: bool) -> Self {
        Self {
            inner,
            attempts: parking_lot::Mutex::new(Vec::new()),
            entered: Notify::new(),
            released: Notify::new(),
            calls: AtomicUsize::new(0),
            lookup_calls: AtomicUsize::new(0),
            lookup_failure: AtomicBool::new(false),
            after_commit,
            permanent,
        }
    }
}
#[async_trait::async_trait]
impl ItemReceiptRepository for FlakyItems {
    async fn item_receipt_lookup(
        &self,
        account: AccountId,
        operation: ItemMutationOperation,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<ItemReceiptLookup> {
        self.lookup_calls.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(!self.lookup_failure.load(Ordering::SeqCst), "injected receipt failure");
        self.inner
            .item_receipt_lookup(account, operation, key, fingerprint)
            .await
    }
}
#[async_trait::async_trait]
impl ItemCheckpointRepository for FlakyItems {
    async fn checkpoint_items(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
        work: &ItemCheckpointWork,
        learning: Option<&SuccessfulLearnReceipt>,
    ) -> Result<CheckpointOutcome, CheckpointError> {
        self.attempts
            .lock()
            .push((cp.clone(), events.to_vec(), work.clone()));
        if self.permanent {
            return Err(CheckpointError::Constraint("injected".into()));
        }
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        if first {
            self.entered.notify_one();
            self.released.notified().await;
            if !self.after_commit {
                return Err(anyhow::anyhow!("before transaction").into());
            }
        }
        let result = self
            .inner
            .checkpoint_items(cp, events, work, learning)
            .await;
        if first {
            return Err(anyhow::anyhow!("lost commit response").into());
        }
        result
    }
}

#[tokio::test]
async fn item_batch_ack_waits_for_atomic_write_and_retries_exact_work() {
    for after_commit in [false, true] {
        let mut h = Harness::new().await;
        let tick = h.equipment_tick();
        assert!(tick.dispositions.is_empty());
        let writer = Arc::new(FlakyItems::new(h.repo.clone(), after_commit, false));
        h.service.item_repo = Some(writer.clone());
        let snapshot = h.state.snapshot();
        let audit = h.audit.clone();
        let session = h.session;
        let task = tokio::spawn(async move {
            h.service.admitted(&tick, &snapshot).await.unwrap();
            (h, tick)
        });
        tokio::time::timeout(Duration::from_secs(10), writer.entered.notified())
            .await
            .unwrap();
        assert!(audit
            .frames(session)
            .iter()
            .all(|frame| !matches!(frame, AuditedFrame::Checkpoint(_))));
        assert!(writer.inner.item_ledger_for_test().is_empty());
        writer.released.notify_one();
        let (mut h, tick) = task.await.unwrap();
        let attempts = writer.attempts.lock().clone();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0], attempts[1]);
        assert_eq!(attempts[0].2.projection.receipts.len(), 2);
        assert_eq!(attempts[0].2.projection.ledger.len(), 2);
        assert_eq!(h.acknowledgements(), 1);
        assert_eq!(h.repo.item_ledger_for_test().len(), 2);
        let stored = h
            .repo
            .retained_inventory(h.entity.as_uuid().into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.character_revision, 2);
        assert_eq!(stored.checkpoint.as_ref(), Some(&attempts[0].2.projection.checkpoint));
        assert!(matches!(
            h.repo.staged_events().as_slice(),
            [
                DomainEvent::ItemUnequipped { .. },
                DomainEvent::ItemEquipped { .. }
            ]
        ));
        assert!(h.service.players[&h.entity].items.is_none());
        h.service
            .admitted(&tick, &h.state.snapshot())
            .await
            .unwrap();
        assert_eq!(h.acknowledgements(), 1);
        assert_eq!(writer.calls.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn pending_item_snapshot_roundtrip_restores_exact_transaction_without_simulation() {
    use crate::application::replay_log::{decode_snapshot, encode_snapshot, restore_snapshot};
    let mut h = Harness::new().await;
    let tick = h.equipment_tick();
    let writer = Arc::new(FlakyItems::new(h.repo.clone(), false, true));
    h.service.item_repo = Some(writer.clone());
    assert!(h
        .service
        .admitted(&tick, &h.state.snapshot())
        .await
        .is_err());
    assert!(h.service.players[&h.entity].fenced);
    assert_eq!(h.acknowledgements(), 0);
    assert!(h.repo.item_ledger_for_test().is_empty());
    // The captured immutable request is exactly the pending state at a process interruption
    // before dispatch; the injected permanent response above also separately proves fencing.
    h.service.players.get_mut(&h.entity).unwrap().fenced = false;
    let mut snapshot = h.state.snapshot();
    h.service.snapshot(&mut snapshot);
    let digest = restore_snapshot(snapshot.clone()).unwrap().state_digest();
    let bytes = encode_snapshot(&snapshot).unwrap();
    let restored = decode_snapshot(&bytes).unwrap();
    assert_eq!(restored, snapshot);
    let mut no_lanes = snapshot.clone();
    no_lanes.checkpoints.clear();
    assert_eq!(restore_snapshot(no_lanes).unwrap().state_digest(), digest);
    let attempt = writer.attempts.lock()[0].clone();
    let mut recovery =
        CheckpointService::new(h.repo.clone(), h.audit.clone(), Arc::new(Metrics::detached()))
            .with_items(h.repo.clone());
    recovery.bind_session(h.entity, h.session);
    recovery.restore(&restored);
    let player = &recovery.players[&h.entity];
    assert_eq!(player.latest.as_ref().unwrap(), &attempt.0);
    assert_eq!(player.events, attempt.1);
    assert_eq!(player.items.as_ref().unwrap(), &attempt.2);
    recovery.flush(h.entity).await.unwrap();
    assert_eq!(h.repo.item_ledger_for_test().len(), 2);
    assert_eq!(h.acknowledgements(), 1);
    recovery.flush(h.entity).await.unwrap();
    assert_eq!(h.acknowledgements(), 1);
    for field in 0..5 {
        let mut bad = restored.clone();
        let lane = bad
            .checkpoints
            .iter_mut()
            .find(|lane| lane.entity == h.entity)
            .unwrap();
        let work = lane.item_work.as_mut().unwrap();
        match field {
            0 => work.projection.account = AccountId::from_uuid(Uuid::from_u128(900)),
            1 => work.projection.entity = EntityId::from_uuid(Uuid::from_u128(901)),
            2 => work.projection.tick = bad.tick,
            3 => lane.latest.as_mut().unwrap().phase3 = None,
            _ => {
                lane.latest
                    .as_mut()
                    .unwrap()
                    .phase4
                    .as_mut()
                    .unwrap()
                    .inventory_revision += 1;
            },
        }
        assert!(encode_snapshot(&bad).is_err(), "corrupt field {field}");
    }
}

#[tokio::test]
async fn item_checkpoint_missing_writer_or_projection_refuses_before_ack() {
    for missing_writer in [false, true] {
        let mut h = Harness::new().await;
        let mut tick = h.equipment_tick();
        if missing_writer {
            h.service.item_repo = None;
        } else {
            tick.events.retain(|event| {
                !matches!(event, crate::domain::zone::ZoneEvent::ItemCheckpoint(_))
            });
        }
        assert!(h
            .service
            .admitted(&tick, &h.state.snapshot())
            .await
            .is_err());
        assert_eq!(h.acknowledgements(), 0);
        assert!(h.repo.item_ledger_for_test().is_empty());
        assert!(tick.outputs.values().flatten().all(|output| !matches!(
            output,
            ObserverOutput::Event(crate::domain::zone::ZoneEvent::ItemCheckpoint(_))
        )));
    }
}

async fn actor_log_then_checkpoint_then_output(log: Arc<dyn EventLog>) {
    use crate::application::replay_log::{
        start_epoch, DurableTickGate, GateConfig, NoReplayMetrics,
    };
    use crate::application::zone_actor::{manual_ticks, TickOutcome, ZoneActor};
    use tokio_stream::StreamExt as _;
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::ERROR)
        .try_init();
    let mut h = Harness::new().await;
    let mut snapshot = h.state.snapshot();
    let nonce = Uuid::now_v7().as_u128();
    snapshot.seed.zone = ZoneId(u32::try_from(nonce & 0x7fff_ffff).unwrap());
    snapshot.seed.epoch = u64::try_from(nonce & 0x7fff_ffff_ffff_ffff).unwrap();
    snapshot.rng.key = snapshot.seed.key();
    h.state = crate::application::replay_log::restore_snapshot(snapshot).unwrap();
    let inputs = h.equipment_inputs();
    let mut baseline = h.state.snapshot();
    h.service.snapshot(&mut baseline);
    let started = start_epoch(log.as_ref(), &baseline).await.unwrap();
    let writer = Arc::new(FlakyItems::new(h.repo.clone(), true, false));
    h.service.item_repo = Some(writer.clone());
    let gate = DurableTickGate::new(
        &started,
        log.clone(),
        Arc::new(NoReplayMetrics),
        GateConfig::default(),
        tokio_util::sync::CancellationToken::new(),
    );
    let (ticks, driver) = manual_ticks();
    let handle = ZoneActor::spawn_gated(h.state, ticks, gate);
    handle.checkpoints.install(h.service);
    let mut outputs = handle.subscribe();
    for input in inputs {
        handle.send(input).unwrap();
    }
    let stepping = tokio::spawn(async move { (driver.step().await.unwrap(), driver) });
    tokio::time::timeout(Duration::from_secs(10), writer.entered.notified())
        .await
        .unwrap();
    assert!(matches!(
        outputs.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    assert!(h.repo.item_ledger_for_test().is_empty());
    assert!(h
        .audit
        .frames(h.session)
        .iter()
        .all(|frame| !matches!(frame, AuditedFrame::Checkpoint(_))));
    // The log contains the complete private projection before the item repository is released.
    let mut recorded = log
        .read_epoch(baseline.seed.zone, baseline.seed.epoch)
        .await
        .unwrap();
    let first = recorded.next().await.unwrap().unwrap();
    assert_eq!(first.tick, baseline.tick);
    assert!(recorded.next().await.is_none());
    assert_eq!(writer.attempts.lock()[0].2.projection.tick, first.tick);
    writer.released.notify_one();
    let (outcome, driver) = stepping.await.unwrap();
    assert_eq!(outcome, TickOutcome::Ran(first.tick));
    let applied = outputs.recv().await.unwrap();
    assert!(first.reproduced_by(&AppliedTickRecord::from_applied(baseline.seed.zone, &applied)));
    assert_eq!(h.repo.item_ledger_for_test().len(), 2);
    assert_eq!(writer.attempts.lock().len(), 2);
    let acks = h
        .audit
        .frames(h.session)
        .iter()
        .filter(|frame| matches!(frame, AuditedFrame::Checkpoint(_)))
        .count();
    assert_eq!(acks, 1);
    drop(driver);
    handle.stopped().await;
    assert!(!handle.persistence_failed());
    // Recovery of that acknowledged tick sees the exact committed checkpoint key and
    // neither duplicates ledger/outbox nor executes gameplay against database state.
    let mut recovery =
        CheckpointService::new(h.repo.clone(), h.audit.clone(), Arc::new(Metrics::detached()))
            .with_items(h.repo.clone());
    let before = h.repo.staged_events();
    recovery
        .recover(log.as_ref(), baseline.seed.zone, baseline.seed.epoch)
        .await
        .unwrap();
    assert_eq!(h.repo.item_ledger_for_test().len(), 2);
    assert_eq!(h.repo.staged_events(), before);
}

#[tokio::test]
async fn actual_item_actor_orders_log_database_ack_and_output_then_recovers() {
    actor_log_then_checkpoint_then_output(Arc::new(
        crate::infrastructure::eventlog::InMemoryEventLog::default(),
    ))
    .await;
}

#[tokio::test]
async fn owned_jetstream_item_actor_acknowledges_log_before_item_checkpoint_and_output() {
    let Ok(url) = std::env::var("NATS_URL") else {
        return;
    };
    let client = async_nats::connect(url).await.unwrap();
    crate::infrastructure::outbox::JetStreamPublisher::connect(client.clone())
        .await
        .unwrap();
    let log =
        crate::infrastructure::eventlog::JetStreamEventLog::connect(client, Metrics::detached())
            .await
            .unwrap();
    actor_log_then_checkpoint_then_output(Arc::new(log)).await;
}

#[tokio::test]
async fn unified_learning_context_survives_pending_item_snapshot_and_settles_once() {
    use crate::application::ports::{
        learn_skill_fingerprint, FrozenLearnResult, LearnReceiptLookup, LearnReceiptRepository,
        LearnedEffectiveStats,
    };
    use crate::domain::subclass::LearnedSkill;
    use crate::domain::zone::skill::{SkillId, SkillRef};
    let mut h = Harness::new().await;
    let tick = h.equipment_tick();
    h.service.item_repo = Some(Arc::new(FlakyItems::new(h.repo.clone(), false, true)));
    assert!(h
        .service
        .admitted(&tick, &h.state.snapshot())
        .await
        .is_err());
    let lane = h.service.players.get_mut(&h.entity).unwrap();
    lane.fenced = false;
    let cp = lane.latest.as_mut().unwrap();
    let skill = SkillRef {
        id: SkillId(1177),
        rank: 1,
    };
    cp.class_state
        .as_mut()
        .unwrap()
        .merge_learned_skills([LearnedSkill {
            key: "l2.skill.1177".into(),
            level: 1,
        }]);
    // Persistence-context fixture: active Wind Strike is a legal retained mage rank.
    // Manual learning eligibility and source book/SP costs are separate gameplay tests.
    let receipt = SuccessfulLearnReceipt {
        account_id: h.account,
        key: IdempotencyKey::new(),
        request_fingerprint: learn_skill_fingerprint(cp.character_id, skill),
        result: FrozenLearnResult {
            character_id: cp.character_id,
            skill,
            sp_spent: 0,
            sp_remaining: 0,
            tick: tick.tick,
            hp: cp.hp,
            max_hp: cp.hp,
            mp: cp.mp,
            max_mp: cp.mp,
            cp: 0,
            max_cp: 0,
            effective_stats: LearnedEffectiveStats::default(),
            incarnation: 1,
        },
    };
    lane.events = vec![DomainEvent::CharacterSkillLearned {
        metadata: EventMetadata {
            event_id: stable_id(h.entity, h.state.seed().zone, tick.epoch, tick.tick, Some(0)),
            sequence: (cp.revision_seen.checked_add(1).unwrap(), 0),
        },
        character_id: cp.character_id,
        skill_id: skill.id.0,
        skill_level: skill.rank,
        sp_spent: 0,
        sp_remaining: 0,
        tick: tick.tick.0,
        request_key: receipt.key.as_uuid(),
    }];
    lane.events
        .extend(item_events(cp, lane.items.as_ref().unwrap(), 1).unwrap());
    lane.learning = Some(receipt.clone());
    let expected = (cp.clone(), lane.events.clone(), lane.items.clone(), lane.learning.clone());
    let mut snapshot = h.state.snapshot();
    h.service.snapshot(&mut snapshot);
    let bytes = crate::application::replay_log::encode_snapshot(&snapshot).unwrap();
    let restored = crate::application::replay_log::decode_snapshot(&bytes).unwrap();
    let mut recovery =
        CheckpointService::new(h.repo.clone(), h.audit.clone(), Arc::new(Metrics::detached()))
            .with_items(h.repo.clone());
    recovery.restore(&restored);
    let lane = &recovery.players[&h.entity];
    assert_eq!(
        (
            lane.latest.clone().unwrap(),
            lane.events.clone(),
            lane.items.clone(),
            lane.learning.clone()
        ),
        expected
    );
    recovery.flush(h.entity).await.unwrap();
    assert_eq!(h.repo.item_ledger_for_test().len(), 2);
    assert_eq!(h.repo.staged_events().len(), 3);
    assert_eq!(
        h.repo
            .learn_receipt_lookup(h.account, &receipt.key, &receipt.request_fingerprint)
            .await
            .unwrap(),
        LearnReceiptLookup::Known(receipt.result)
    );
    recovery.flush(h.entity).await.unwrap();
    assert_eq!(h.repo.staged_events().len(), 3);
    for field in 0..6 {
        let mut bad = restored.clone();
        let lane = bad
            .checkpoints
            .iter_mut()
            .find(|lane| lane.entity == h.entity)
            .unwrap();
        match field {
            0 => lane.learning = None,
            1 => {
                lane.learning.as_mut().unwrap().account =
                    AccountId::from_uuid(Uuid::from_u128(902));
            },
            2 => lane.learning.as_mut().unwrap().key = Uuid::nil(),
            3 => lane.learning.as_mut().unwrap().fingerprint.push('!'),
            4 => lane.learning.as_mut().unwrap().result.sp_spent = 1,
            _ => lane
                .events
                .retain(|event| !matches!(event, DomainEvent::CharacterSkillLearned { .. })),
        }
        assert!(
            crate::application::replay_log::encode_snapshot(&bad).is_err(),
            "learning field {field}"
        );
    }
}

#[tokio::test]
async fn refuses_item_actor_without_durable_gate_before_database_and_visibility() {
    use crate::application::zone_actor::{manual_ticks, TickOutcome, ZoneActor};
    let h = Harness::new().await;
    let inputs = h.equipment_inputs();
    let tick = h.state.next_tick();
    let (ticks, driver) = manual_ticks();
    let handle = ZoneActor::spawn(h.state, ticks);
    handle.checkpoints.install(h.service);
    let mut outputs = handle.subscribe();
    for input in inputs {
        handle.send(input).unwrap();
    }
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(tick));
    handle.stopped().await;
    assert!(handle.persistence_failed());
    assert!(h.repo.item_ledger_for_test().is_empty());
    assert!(h.repo.staged_events().is_empty());
    assert!(h
        .audit
        .frames(h.session)
        .iter()
        .all(|frame| !matches!(frame, AuditedFrame::Checkpoint(_))));
    assert!(outputs.try_recv().is_err());
}

async fn receipt_actor(
    state: ZoneState,
    service: CheckpointService,
) -> (
    crate::application::zone_actor::ZoneHandle,
    crate::application::zone_actor::ManualTickDriver,
    ZoneSnapshot,
    Arc<dyn EventLog>,
) {
    use crate::application::replay_log::{
        start_epoch, DurableTickGate, GateConfig, NoReplayMetrics,
    };
    use crate::application::zone_actor::{manual_ticks, ZoneActor};
    let mut baseline = state.snapshot();
    service.snapshot(&mut baseline);
    let log: Arc<dyn EventLog> =
        Arc::new(crate::infrastructure::eventlog::InMemoryEventLog::default());
    let started = start_epoch(log.as_ref(), &baseline).await.unwrap();
    let gate = DurableTickGate::new(
        &started,
        log.clone(),
        Arc::new(NoReplayMetrics),
        GateConfig::default(),
        tokio_util::sync::CancellationToken::new(),
    );
    let (ticks, driver) = manual_ticks();
    let handle = ZoneActor::spawn_gated(state, ticks, gate);
    handle.checkpoints.install(service);
    (handle, driver, baseline, log)
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Keep real log/DB/Known/cold-replay checks and original-sequence proof together.
async fn item_receipt_preflight_records_frozen_known_and_conflict_without_reexecution() {
    use crate::application::zone_actor::TickOutcome;
    use crate::domain::zone::{AppliedTickDraft, ItemInstanceId, RejectReason, ZoneEvent};
    let mut h = Harness::new().await;
    let initial = h.equipment_inputs();
    let tick = h.equipment_tick();
    h.service
        .admitted(&tick, &h.state.snapshot())
        .await
        .unwrap();
    let original: Vec<_> = tick
        .events
        .iter()
        .filter_map(|event| {
            if let ZoneEvent::ItemResult(result) = event {
                Some(result.as_ref().clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(original[0].accepted_sequence, Some(1));
    assert_eq!(original[1].accepted_sequence, Some(2));
    // A later committed unequip makes replaying the old Equip command observably wrong.
    let later = ZoneInput::session(
        h.entity,
        SessionGeneration(1),
        4,
        ZoneCommand::Item(Box::new(
            ItemCommand::new(
                h.entity,
                Uuid::from_u128(992),
                ItemCommandKind::Unequip {
                    slot: EquipSlot::Rhand,
                },
            )
            .unwrap()
            .with_account(h.account)
            .unwrap(),
        )),
    );
    let tick = h.state.run_tick(h.state.draft(vec![later])).unwrap();
    h.service
        .admitted(&tick, &h.state.snapshot())
        .await
        .unwrap();
    let before = h.state.snapshot();
    let rows = h.repo.item_ledger_for_test();
    let events = h.repo.staged_events();
    let writer = Arc::new(FlakyItems::new(h.repo.clone(), false, false));
    h.service.item_repo = Some(writer.clone());
    let (handle, driver, baseline, _) = receipt_actor(h.state, h.service).await;
    let mut outputs = handle.subscribe();
    for (index, input) in initial.into_iter().enumerate() {
        let ZoneCommand::Item(command) = input.command else {
            unreachable!()
        };
        let mut kind = command.kind;
        // The middle body conflicts in the Equip scope; first/third replay both scopes.
        if index == 1 {
            kind = ItemCommandKind::Equip {
                instance: ItemInstanceId::new(99999).unwrap(),
                preferred_slot: None,
            };
        }
        handle
            .send(ZoneInput::session(
                h.entity,
                SessionGeneration(1),
                u32::try_from(index + 11).unwrap(),
                ZoneCommand::Item(Box::new(
                    ItemCommand::new(h.entity, command.request_id, kind).unwrap(),
                )),
            ))
            .unwrap();
    }
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(baseline.tick));
    let applied = outputs.recv().await.unwrap();
    assert_eq!(applied.commands.len(), 3);
    assert_eq!(applied.dispositions.len(), 1);
    assert_eq!(applied.dispositions[0].reason, RejectReason::ItemOperationConflict);
    let results: Vec<_> = applied
        .events
        .iter()
        .filter_map(|event| {
            if let ZoneEvent::ItemResult(result) = event {
                Some(result.as_ref().clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(results, vec![original[0].clone(), original[1].clone()]);
    for (index, command) in applied.commands.iter().enumerate() {
        let ZoneCommand::Item(command) = &command.command else {
            unreachable!()
        };
        assert_eq!(command.authenticated_account().unwrap(), h.account);
        assert_eq!(command.has_known_receipt(), index != 1);
        assert_eq!(command.receipt_conflict().unwrap().is_some(), index == 1);
    }
    assert!(!applied
        .events
        .iter()
        .any(|event| matches!(event, ZoneEvent::ItemCheckpoint(_))));
    assert_eq!(h.repo.item_ledger_for_test(), rows);
    assert_eq!(h.repo.staged_events(), events);
    assert!(writer.attempts.lock().is_empty());
    assert_eq!(writer.lookup_calls.load(Ordering::SeqCst), 6);
    let recorded = AppliedTickRecord::from_applied(baseline.seed.zone, &applied);
    let decoded = AppliedTickRecord::decode(&recorded.encode()).unwrap();
    let mut cold = crate::application::replay_log::restore_snapshot(baseline).unwrap();
    let rerun = cold
        .run_tick(AppliedTickDraft {
            epoch: decoded.epoch,
            tick: decoded.tick,
            commands: decoded.commands,
        })
        .unwrap();
    assert!(recorded.reproduced_by(&AppliedTickRecord::from_applied(before.seed.zone, &rerun)));
    assert_eq!(cold.snapshot().phase4, before.phase4);
    assert_eq!(cold.snapshot().rng, before.rng);
    drop(driver);
    handle.stopped().await;
    assert!(!handle.persistence_failed());
}

#[tokio::test]
async fn item_receipt_preflight_remeasures_enriched_known_before_drafting() {
    use crate::application::replay_log::command_admission_size;
    use crate::application::zone_actor::TickOutcome;
    let mut h = Harness::new().await;
    let mut input = h.equipment_inputs().remove(1);
    let tick = h.equipment_tick();
    h.service
        .admitted(&tick, &h.state.snapshot())
        .await
        .unwrap();
    let limit = command_admission_size(&input).unwrap().normalized_bytes();
    let ZoneCommand::Item(command) = &input.command else {
        unreachable!()
    };
    input.command = ZoneCommand::Item(Box::new(
        ItemCommand::new(command.entity, command.request_id, command.kind.clone()).unwrap(),
    ));
    let mut snapshot = h.state.snapshot();
    snapshot
        .meta
        .phase3_admission_policy
        .as_mut()
        .unwrap()
        .max_command_bytes = u64::try_from(limit).unwrap();
    h.state = crate::application::replay_log::restore_snapshot(snapshot).unwrap();
    let rows = h.repo.item_ledger_for_test();
    let events = h.repo.staged_events();
    let writer = Arc::new(FlakyItems::new(h.repo.clone(), false, false));
    h.service.item_repo = Some(writer.clone());
    let (handle, driver, baseline, _) = receipt_actor(h.state, h.service).await;
    let mut failures = handle.subscribe_admission_failures();
    handle.send(input).unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(baseline.tick));
    let failure = failures.try_recv().unwrap();
    assert_eq!(
        failure.reason,
        crate::application::zone_admission::AdmissionFailureReason::CommandTooLarge
    );
    assert!(failure.normalized_bytes > limit);
    assert_eq!(handle.stats().borrow().commands_applied, 0);
    assert_eq!(writer.lookup_calls.load(Ordering::SeqCst), 1);
    assert!(writer.attempts.lock().is_empty());
    assert_eq!(h.repo.item_ledger_for_test(), rows);
    assert_eq!(h.repo.staged_events(), events);
    drop(driver);
    handle.stopped().await;
    assert!(!handle.persistence_failed());
}

#[tokio::test]
async fn item_receipt_preflight_lookup_failure_fences_before_log_database_or_output() {
    use crate::application::zone_actor::TickOutcome;
    use tokio_stream::StreamExt as _;
    let mut h = Harness::new().await;
    let input = h.equipment_inputs().remove(0);
    let writer = Arc::new(FlakyItems::new(h.repo.clone(), false, false));
    writer.lookup_failure.store(true, Ordering::SeqCst);
    h.service.item_repo = Some(writer.clone());
    let (handle, driver, baseline, log) = receipt_actor(h.state, h.service).await;
    let mut outputs = handle.subscribe();
    handle.send(input).unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(baseline.tick));
    handle.stopped().await;
    assert!(handle.persistence_failed());
    assert!(log
        .read_epoch(baseline.seed.zone, baseline.seed.epoch)
        .await
        .unwrap()
        .next()
        .await
        .is_none());
    assert_eq!(writer.lookup_calls.load(Ordering::SeqCst), 1);
    assert!(writer.attempts.lock().is_empty());
    assert!(h.repo.item_ledger_for_test().is_empty());
    assert!(h.repo.staged_events().is_empty());
    assert!(outputs.try_recv().is_err());
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Five outcome branches share one exact captured-audit contract.
async fn captured_enchant_outbox_preserves_all_outcomes_before_later_destruction() {
    use crate::domain::zone::phase4_projection::{
        ItemAuditEvidence, ItemDeltaPlan, ItemEnchantOutcome, ItemOperation, ItemResultDetail,
    };
    use crate::domain::zone::{EnchantAttempt, ItemInstanceId};
    let mut h = Harness::new().await;
    let tick = h.equipment_tick();
    let projection = tick
        .events
        .iter()
        .find_map(|event| {
            if let crate::domain::zone::ZoneEvent::ItemCheckpoint(p) = event {
                Some(p.as_ref())
            } else {
                None
            }
        })
        .unwrap();
    let mut cp = h
        .service
        .players
        .get(&h.entity)
        .unwrap()
        .latest
        .clone()
        .unwrap();
    cp.phase4 = Some(projection.checkpoint.clone());
    let before = projection
        .checkpoint
        .instances
        .iter()
        .find(|item| item.count == 1)
        .unwrap()
        .clone();
    for outcome in [
        ItemEnchantOutcome::Success,
        ItemEnchantOutcome::SafeFailed,
        ItemEnchantOutcome::BlessedReset,
        ItemEnchantOutcome::Destroyed,
        ItemEnchantOutcome::SuccessNoIncrement,
    ] {
        let mut work = ItemCheckpointWork {
            zone: h.state.snapshot().seed.zone,
            epoch: tick.epoch,
            allocation: h.state.snapshot().phase4.unwrap().allocator,
            projection: projection.clone(),
        };
        work.projection.ledger.clear();
        work.projection.receipts.truncate(1);
        let mut after = before.clone();
        after.enchant = match outcome {
            ItemEnchantOutcome::Success => before.enchant + 1,
            ItemEnchantOutcome::SafeFailed | ItemEnchantOutcome::SuccessNoIncrement => {
                before.enchant
            },
            ItemEnchantOutcome::BlessedReset | ItemEnchantOutcome::Destroyed => 0,
        };
        let receipt = work.projection.receipts.first_mut().unwrap();
        receipt.result.detail = ItemResultDetail::Enchant {
            target: before.id,
            old_enchant: before.enchant,
            new_enchant: after.enchant,
            outcome,
            refunds: Vec::new(),
            announcement_tier: 0,
        };
        let success =
            matches!(outcome, ItemEnchantOutcome::Success | ItemEnchantOutcome::SuccessNoIncrement);
        let attempt = EnchantAttempt {
            operation_ordinal: 1,
            receipt: receipt.reference(),
            target: before.id,
            template: before.template_id,
            old_enchant: before.enchant,
            new_enchant: after.enchant,
            outcome,
            before: before.clone(),
            after: (outcome != ItemEnchantOutcome::Destroyed).then_some(after.clone()),
            scroll: ItemInstanceId::new(9_999_999).unwrap(),
            scroll_template: 951,
            chance_1e5: if success { 100_000 } else { 0 },
            raw_draw: 199_999,
            draw: 99_999,
            cause: ItemAuditEvidence {
                family: "enchant".into(),
                fields: BTreeMap::new(),
            },
        };
        attempt.validate(receipt).unwrap();
        work.projection.enchant_attempts = vec![attempt.clone()];
        // The final inventory is intentionally different from the captured attempt.
        work.projection.checkpoint.instances.clear();
        work.projection.ledger.push(ItemDeltaPlan {
            operation: if outcome == ItemEnchantOutcome::Destroyed {
                ItemOperation::Enchant
            } else {
                ItemOperation::Destroy
            },
            operation_ordinal: if outcome == ItemEnchantOutcome::Destroyed {
                1
            } else {
                2
            },
            instance: before.id,
            before: Some(after),
            after: None,
            count_delta: -1,
            receipt: Some(attempt.receipt),
            causes: Vec::new(),
        });
        if outcome == ItemEnchantOutcome::Success {
            work.projection.ledger.push(ItemDeltaPlan {
                operation: ItemOperation::Enchant,
                operation_ordinal: 1,
                instance: before.id,
                before: Some(before.clone()),
                after: attempt.after.clone(),
                count_delta: 0,
                receipt: Some(attempt.receipt),
                causes: Vec::new(),
            });
        }
        let events = item_events(&cp, &work, 4).unwrap();
        assert_eq!(events.len(), 2);
        let DomainEvent::ItemEnchanted { fact } = &events[0] else {
            panic!("missing enchant attempt");
        };
        assert_eq!(fact.before.as_ref(), Some(&before));
        assert_eq!(fact.after, attempt.after);
        assert_eq!(fact.causes, vec![serde_json::to_value(&attempt).unwrap()]);
        assert_eq!(fact.metadata.sequence, (cp.revision_seen + 1, 4));
        assert!(matches!(events[1], DomainEvent::ItemDestroyed { .. }));
        work.validate_events(&cp, &events).unwrap();
        assert_eq!(events, item_events(&cp, &work, 4).unwrap());
        assert!(work.validate_events(&cp, &events[1..]).is_err());
        let mut altered = events.clone();
        let DomainEvent::ItemEnchanted { fact } = &mut altered[0] else {
            panic!();
        };
        fact.causes.clear();
        assert!(work.validate_events(&cp, &altered).is_err());
        let mut duplicate = events.clone();
        duplicate.push(events[0].clone());
        assert!(work.validate_events(&cp, &duplicate).is_err());
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Real log/write barrier, lost acknowledgement, subsequent mutation and scoped retry.
async fn recorded_grant_reply_waits_for_log_checkpoint_and_returns_original_baseline() {
    use crate::application::zone_actor::TickOutcome;
    use crate::domain::zone::{ItemGrantCommand, RejectReason, ZoneEvent};
    let mut h = Harness::new().await;
    let command = ItemGrantCommand {
        entity: h.entity,
        account: h.account,
        request_id: Uuid::from_u128(5001),
        template_id: 57,
        count: 10,
        fingerprint: crate::application::ports::grant_item_fingerprint(
            CharacterId::from_uuid(h.entity.as_uuid()),
            57,
            10,
        ),
        rules_hash: h.state.snapshot().phase4.unwrap().rules_hash,
        paused: None,
        known: None,
        conflict: None,
    };
    let writer = Arc::new(FlakyItems::new(h.repo.clone(), true, false));
    h.service.item_repo = Some(writer.clone());
    let (handle, driver, baseline, log) = receipt_actor(h.state, h.service).await;
    let mut outputs = handle.subscribe();
    let mut reply = handle.enqueue_grant_item(command.clone()).await.unwrap();
    let mut duplicate = handle.enqueue_grant_item(command.clone()).await.unwrap();
    let stepping = tokio::spawn(async move { (driver.step().await.unwrap(), driver) });
    tokio::time::timeout(Duration::from_secs(10), writer.entered.notified())
        .await
        .unwrap();
    assert!(matches!(
        reply.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        duplicate.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert!(outputs.try_recv().is_err());
    assert!(h.repo.item_ledger_for_test().is_empty());
    let mut records = log
        .read_epoch(baseline.seed.zone, baseline.seed.epoch)
        .await
        .unwrap();
    let first = records.next().await.unwrap().unwrap();
    assert!(matches!(first.commands[0].command, ZoneCommand::MaterializePendingItems(_)));
    writer.released.notify_one();
    let (outcome, driver) = stepping.await.unwrap();
    assert_eq!(outcome, TickOutcome::Ran(baseline.tick));
    let original = reply.await.unwrap().unwrap();
    assert_eq!(duplicate.await.unwrap().unwrap(), original);
    assert_eq!(original.inventory.as_ref().unwrap().character_revision, 2);
    assert_eq!(
        original
            .inventory
            .as_ref()
            .unwrap()
            .checkpoint
            .instances
            .iter()
            .find(|i| i.template_id == 57)
            .unwrap()
            .count,
        10
    );
    outputs.recv().await.unwrap();
    {
        let attempts = writer.attempts.lock();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0], attempts[1]);
    }
    assert_eq!(h.repo.item_ledger_for_test().len(), 1);
    let mut next = command.clone();
    next.request_id = Uuid::from_u128(5002);
    next.count = 3;
    next.fingerprint = crate::application::ports::grant_item_fingerprint(
        CharacterId::from_uuid(h.entity.as_uuid()),
        57,
        3,
    );
    let later = handle.enqueue_grant_item(next).await.unwrap();
    driver.step().await.unwrap();
    let later = later.await.unwrap().unwrap();
    assert_eq!(
        later
            .inventory
            .unwrap()
            .checkpoint
            .instances
            .iter()
            .find(|i| i.template_id == 57)
            .unwrap()
            .count,
        13
    );
    outputs.recv().await.unwrap();
    let known = handle.enqueue_grant_item(command.clone()).await.unwrap();
    driver.step().await.unwrap();
    assert_eq!(known.await.unwrap().unwrap(), original);
    let recorded = outputs.recv().await.unwrap();
    let ZoneCommand::MaterializePendingItems(known) = &recorded.commands[0].command else {
        unreachable!()
    };
    assert_eq!(known.known.as_ref().unwrap().result, original.result);
    assert!(!recorded
        .events
        .iter()
        .any(|event| matches!(event, ZoneEvent::ItemCheckpoint(_))));
    let mut conflict = command;
    conflict.count = 11;
    conflict.fingerprint = crate::application::ports::grant_item_fingerprint(
        CharacterId::from_uuid(h.entity.as_uuid()),
        57,
        11,
    );
    let refused = handle.enqueue_grant_item(conflict).await.unwrap();
    driver.step().await.unwrap();
    assert!(matches!(
        refused.await.unwrap(),
        Err(crate::application::AppError::IdempotencyConflict)
    ));
    let recorded = outputs.recv().await.unwrap();
    assert_eq!(recorded.dispositions[0].reason, RejectReason::ItemOperationConflict);
    let ZoneCommand::MaterializePendingItems(conflict) = &recorded.commands[0].command else {
        unreachable!()
    };
    assert_eq!(conflict.conflict.as_ref(), Some(&original.fingerprint));
    assert_eq!(h.repo.item_ledger_for_test().len(), 2);
    drop(driver);
    handle.stopped().await;
    assert!(!handle.persistence_failed());
}

#[tokio::test]
async fn recorded_grant_unavailable_before_receipt_reads_without_durable_gate() {
    use crate::application::zone_actor::{manual_ticks, ZoneActor};
    let mut h = Harness::new().await;
    let writer = Arc::new(FlakyItems::new(h.repo.clone(), false, false));
    h.service.item_repo = Some(writer.clone());
    let (ticks, driver) = manual_ticks();
    let handle = ZoneActor::spawn(h.state, ticks);
    handle.checkpoints.install(h.service);
    assert!(matches!(
        handle.item_grants_available().await,
        Err(crate::application::AppError::Unavailable(_))
    ));
    assert_eq!(writer.lookup_calls.load(Ordering::SeqCst), 0);
    assert!(h.repo.item_ledger_for_test().is_empty());
    drop(driver);
    handle.stopped().await;
}

#[path = "checkpoint_paused_tests.rs"]
mod paused_tests;
