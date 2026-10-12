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
use std::sync::atomic::{AtomicUsize, Ordering};
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
