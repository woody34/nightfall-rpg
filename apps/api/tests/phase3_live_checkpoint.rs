//! Real experimental actor, repository admission, paused logout and recovery barriers.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::expect_used,
    clippy::arithmetic_side_effects
)]
mod common;
use nightfall_api::application::checkpoint::CheckpointService;
use nightfall_api::application::ports::RepositoryError;
use nightfall_api::application::replay_log::{
    start_epoch, DurableTickGate, EventLog, GateConfig, NoReplayMetrics,
};
use nightfall_api::application::session::{AdmitError, PlayerSpawn, SessionRegistry};
use nightfall_api::application::use_cases::{CreateCharacter, CreateCharacterInput};
use nightfall_api::application::zone_actor::{
    manual_ticks, ManualTickDriver, TickOutcome, ZoneActor, ZoneHandle,
};
use nightfall_api::application::{
    CharacterCheckpoint, CharacterRepository, CheckpointError, CheckpointOutcome, CreateOutcome,
    IdempotencyKey, ProgressionState,
};
use nightfall_api::domain::character_progression::CharacterAppearance;
use nightfall_api::domain::class::{ClassId, ClassRegistry};
use nightfall_api::domain::zone::effects::{
    continuous_handler, ActiveEffect, ActiveHandler, EffectFlags, EffectId, EffectPool,
    EffectSource, EffectSourceKind, RetainedEffect, RetainedModifierSource,
};
use nightfall_api::domain::zone::skill::{RankValue, SkillId, SkillRef};
use nightfall_api::domain::zone::*;
use nightfall_api::domain::{
    AccountId, Character, CharacterId, DomainEvent, Position, Race, SessionId,
};
use nightfall_api::infrastructure::class_data::{load_classes, ClassSource};
use nightfall_api::infrastructure::eventlog::InMemoryEventLog;
use nightfall_api::infrastructure::memory::{InMemoryCharacterRepository, InMemorySessionAudit};
use nightfall_api::infrastructure::postgres::PgCharacterRepository;
use nightfall_api::infrastructure::rules_data::{load_rules, RulesSource};
use nightfall_api::infrastructure::skill_data::{load_phase3_rules, Phase3DataSource};
use nightfall_api::infrastructure::telemetry::Metrics;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn classes() -> Arc<ClassRegistry> {
    load_classes(&ClassSource::embedded()).unwrap().registry
}
fn rules() -> Arc<Phase3Rules> {
    Arc::new(load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap())
}
fn shield(rules: &Phase3Rules) -> RetainedEffect {
    retained_effect(rules, 1040)
}
fn retained_effect(rules: &Phase3Rules, id: u32) -> RetainedEffect {
    let skill = rules.skills().get(SkillId(id)).unwrap();
    let abnormal = skill.abnormal.as_ref().unwrap();
    let flag = |v: &RankValue<bool>| *v.at(1, skill.max_level).unwrap();
    ActiveEffect {
        id: EffectId(77),
        skill: SkillRef {
            id: skill.id,
            rank: 1,
        },
        source: EffectSource {
            binding: None,
            kind: EffectSourceKind::System,
        },
        target_incarnation: 9,
        applied_at: Tick(0),
        expires_at: Some(Tick(600.min(u64::try_from(abnormal.time_s[0]).unwrap() * 10))),
        pool: EffectPool::Buff,
        abnormal_type: abnormal.r#type.clone(),
        abnormal_level: abnormal.level[0],
        flags: EffectFlags {
            stay_after_death: flag(&skill.stay_after_death),
            stay_on_subclass_change: flag(&skill.stay_on_subclass_change),
            removed_on_damage: flag(&skill.removed_on_damage),
            removed_on_action_except_move: flag(&skill.removed_on_action_except_move),
            irreplaceable: flag(&skill.irreplaceable_buff),
        },
        handlers: skill
            .effects
            .iter()
            .enumerate()
            .filter(|(_, h)| continuous_handler(&h.name))
            .map(|(i, _)| ActiveHandler {
                xml_ordinal: u32::try_from(i).unwrap(),
                periodic: None,
            })
            .collect(),
    }
    .retain(Tick(0))
    .unwrap()
    .unwrap()
}
async fn seed(repo: Arc<dyn CharacterRepository>) -> Character {
    seed_named(repo, "DormantMage").await
}
async fn seed_named(repo: Arc<dyn CharacterRepository>, name: &str) -> Character {
    let c = CreateCharacter::new(repo.clone(), classes())
        .execute(CreateCharacterInput {
            idempotency_key: IdempotencyKey::new(),
            account_id: AccountId::from_uuid(Uuid::nil()),
            name: name.into(),
            race: Race::Human,
            base_class_id: Some(ClassId(10)),
            appearance: CharacterAppearance::default(),
        })
        .await
        .unwrap();
    let mut ledger = c.class_state.clone();
    ledger.sp = 90;
    let mut p = Phase3Checkpoint::new();
    p.resource_fractions.hp = 1;
    p.resource_fractions.mp = 321;
    p.reuse_remainders.insert(SkillId(1177), 51);
    p.effect_remainders.push(shield(&rules()));
    p.modifier_install_order
        .push(RetainedModifierSource::Effect { effect_ordinal: 0 });
    let cp = CharacterCheckpoint {
        character_id: c.id,
        revision_seen: 0,
        level: c.level,
        xp: c.xp,
        hp: 20,
        mp: 10,
        alive: true,
        position: Position { x: 126.0, y: 128.0 },
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        class_state: Some(ledger),
        phase3: Some(p),
    };
    assert_eq!(repo.checkpoint(&cp, &[]).await.unwrap(), CheckpointOutcome::Applied(1));
    c
}
async fn actor(
    repo: Arc<dyn CharacterRepository>,
) -> (ZoneHandle, ManualTickDriver, Arc<InMemoryEventLog>) {
    actor_metrics(repo, Arc::new(Metrics::detached()), None).await
}
async fn actor_metrics(
    repo: Arc<dyn CharacterRepository>,
    metrics: Arc<Metrics>,
    index: Option<Arc<dyn nightfall_api::application::replay_log::ZoneSnapshotStore>>,
) -> (ZoneHandle, ManualTickDriver, Arc<InMemoryEventLog>) {
    let state = ZoneState::new_phase3(
        ZoneSeed {
            zone: ZoneId(77),
            epoch: 1,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        load_rules(&RulesSource::embedded()).unwrap().rules,
        classes(),
        (*rules()).clone(),
    )
    .unwrap();
    let log = Arc::new(InMemoryEventLog::default());
    let started = start_epoch(log.as_ref(), &state.snapshot()).await.unwrap();
    let gate = DurableTickGate::new(
        &started,
        log.clone(),
        Arc::new(NoReplayMetrics),
        GateConfig::default(),
        CancellationToken::new(),
    );
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn_gated(state, ticks, gate);
    let mut service =
        CheckpointService::new(repo, Arc::new(InMemorySessionAudit::default()), metrics);
    if let Some(index) = index {
        service = service.with_durability(log.clone(), index);
    }
    zone.checkpoints.install(service);
    (zone, driver, log)
}
fn spawn(c: &Character) -> PlayerSpawn {
    PlayerSpawn {
        entity: EntityId::from_uuid(c.id.as_uuid()),
        name: "UntrustedName".into(),
        pos: Vec2Fixed::from_tiles(0, 0),
        speed: Speed::DEFAULT,
        load: None,
    }
}
async fn lifecycle(repo: Arc<dyn CharacterRepository>) {
    let c = seed(repo.clone()).await;
    let (zone, driver, log) = actor(repo.clone()).await;
    let registry = SessionRegistry::with_characters(repo.clone());
    let input = spawn(&c);
    let session = SessionId::new();
    registry
        .admit(&zone, session, SessionGeneration(1), &input)
        .await
        .unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(Tick(0)));
    let initial = zone.snapshot().await.unwrap();
    let side = &initial.phase3.as_ref().unwrap().entities[&input.entity];
    assert_eq!(side.reuse[&SkillId(1177)], Tick(51));
    assert_eq!(side.effects.entries()[&EffectId(1)].expires_at, Some(Tick(600)));
    assert_eq!(side.fractions.mp, 321);
    let first = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(first.revision, 2);
    assert_eq!(first.class_state.sp, 90);
    assert_eq!(first.name, c.name);
    for _ in 0..15 {
        driver.step().await.unwrap();
    }
    assert_eq!(
        repo.load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        2,
        "countdown projections alone do not trigger writes"
    );
    let leave = registry.leave(&zone, session, input.entity, SessionGeneration(1));
    tokio::pin!(leave);
    assert!(tokio::time::timeout(Duration::from_millis(1), &mut leave)
        .await
        .is_err());
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(Tick(16)));
    leave.await.unwrap();
    let paused = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(paused.revision, 3);
    let p = paused.phase3.as_ref().unwrap();
    assert!(p.reuse_remainders.is_empty());
    assert_eq!(p.effect_remainders[0].remaining_ticks, 585);
    registry
        .admit(&zone, SessionId::new(), SessionGeneration(2), &input)
        .await
        .unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(Tick(17)));
    let readmitted = zone.snapshot().await.unwrap();
    let side = &readmitted.phase3.as_ref().unwrap().entities[&input.entity];
    assert!(side.reuse.is_empty());
    assert_eq!(side.effects.entries()[&EffectId(2)].expires_at, Some(Tick(602)));
    assert_eq!(side.fractions.mp, 321);
    let frozen = &readmitted.checkpoints[0].latest.as_ref().unwrap().phase3;
    assert!(frozen.is_some());
    let before = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let mut recovery = CheckpointService::new(
        repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    );
    recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
    assert_eq!(
        repo.load_for_admission(c.id).await.unwrap().unwrap(),
        before,
        "acknowledged replay must return immutable checkpoint receipts"
    );
    drop(driver);
    zone.stopped().await;
}
#[tokio::test]
async fn memory_real_admission_logout_rebase_and_recovery() {
    lifecycle(Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules()))).await;
}
#[tokio::test]
async fn postgres_real_admission_logout_rebase_and_recovery() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    lifecycle(Arc::new(PgCharacterRepository::new(pool).with_phase3_rules(rules()))).await;
}

#[tokio::test]
async fn retained_character_is_refused_before_legacy_actor_queue() {
    let repo: Arc<dyn CharacterRepository> =
        Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules()));
    let c = seed(repo.clone()).await;
    let before = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let state = ZoneState::new(
        ZoneSeed {
            zone: ZoneId(78),
            epoch: 1,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
    );
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn(state, ticks);
    assert!(!zone.phase3_enabled());
    assert!(matches!(
        SessionRegistry::with_characters(repo.clone())
            .admit(&zone, SessionId::new(), SessionGeneration(1), &spawn(&c))
            .await,
        Err(AdmitError::LoadFailed)
    ));
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(Tick(0)));
    assert!(zone.snapshot().await.unwrap().entities.is_empty());
    assert!(!zone.persistence_failed());
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), before);
    drop(driver);
    zone.stopped().await;
}

#[tokio::test]
async fn retained_admission_stale_revision_fences_before_output() {
    let repo: Arc<dyn CharacterRepository> =
        Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules()));
    let c = seed(repo.clone()).await;
    let (zone, driver, log) = actor(repo.clone()).await;
    let mut output = zone.subscribe();
    SessionRegistry::with_characters(repo.clone())
        .admit(&zone, SessionId::new(), SessionGeneration(1), &spawn(&c))
        .await
        .unwrap();
    // Admission captured revision 1. A newer writer wins before this tick checkpoints.
    let saved = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let newer = CharacterCheckpoint {
        character_id: c.id,
        revision_seen: saved.revision,
        level: saved.level,
        xp: saved.xp,
        hp: saved.hp.unwrap(),
        mp: saved.mp.unwrap(),
        alive: saved.alive,
        position: saved.position,
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        class_state: Some(saved.class_state),
        phase3: saved.phase3,
    };
    assert_eq!(repo.checkpoint(&newer, &[]).await.unwrap(), CheckpointOutcome::Applied(2));
    let expected = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(Tick(0)));
    zone.stopped().await;
    assert!(zone.persistence_failed());
    assert!(output.try_recv().is_err());
    assert!(zone.snapshot().await.is_err());
    assert!(log.subjects().iter().any(|(_, s)| s.ends_with(".applied")));
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), expected);
}

#[tokio::test]
async fn dirty_retained_baseline_without_outbox_facts_commits_and_replays_once() {
    let repo: Arc<dyn CharacterRepository> =
        Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules()));
    let c = seed(repo.clone()).await;
    let (zone, driver, log) = actor(repo.clone()).await;
    let entity = spawn(&c).entity;
    SessionRegistry::with_characters(repo.clone())
        .admit(&zone, SessionId::new(), SessionGeneration(1), &spawn(&c))
        .await
        .unwrap();
    driver.step().await.unwrap();
    let mut baseline = zone.snapshot().await.unwrap();
    drop(driver);
    zone.stopped().await;
    // Model a durable baseline carrying a frozen critical resource-fraction save.
    let lane = &mut baseline.checkpoints[0];
    lane.dirty = true;
    assert!(lane.events.is_empty());
    let cp = lane.latest.as_mut().unwrap();
    cp.revision_seen = lane.revision;
    cp.key = Uuid::now_v7();
    cp.phase3.as_mut().unwrap().resource_fractions.mp = 322;
    baseline
        .phase3
        .as_mut()
        .unwrap()
        .entities
        .get_mut(&entity)
        .unwrap()
        .fractions
        .mp = 322;
    log.write_recovery_snapshot(&baseline).await.unwrap();
    let mut recovery = CheckpointService::new(
        repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    );
    recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
    let committed = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(committed.revision, 3);
    assert_eq!(committed.phase3.as_ref().unwrap().resource_fractions.mp, 322);
    recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), committed);
}

struct Probe {
    inner: Arc<dyn CharacterRepository>,
    hold: AtomicBool,
    lose_ack: AtomicBool,
    entered: Notify,
    release: Notify,
    requests: parking_lot::Mutex<Vec<CharacterCheckpoint>>,
}
#[async_trait::async_trait]
impl CharacterRepository for Probe {
    async fn get(&self, id: CharacterId) -> anyhow::Result<Option<Character>> {
        self.inner.get(id).await
    }
    async fn list_by_account(&self, id: AccountId) -> anyhow::Result<Vec<Character>> {
        self.inner.list_by_account(id).await
    }
    async fn create_idempotent(
        &self,
        key: &IdempotencyKey,
        fp: &str,
        c: &Character,
    ) -> Result<CreateOutcome, RepositoryError> {
        self.inner.create_idempotent(key, fp, c).await
    }
    async fn load_for_admission(
        &self,
        id: CharacterId,
    ) -> anyhow::Result<Option<ProgressionState>> {
        self.inner.load_for_admission(id).await
    }
    async fn checkpoint(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
    ) -> Result<CheckpointOutcome, CheckpointError> {
        self.requests.lock().push(cp.clone());
        if self.hold.load(Ordering::Acquire) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        let result = self.inner.checkpoint(cp, events).await?;
        if self.lose_ack.swap(false, Ordering::AcqRel) {
            return Err(anyhow::anyhow!("injected lost commit acknowledgement").into());
        }
        Ok(result)
    }
}
#[tokio::test]
async fn retained_admission_waits_for_ack_and_retry_freezes_paused_request() {
    let inner: Arc<dyn CharacterRepository> =
        Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules()));
    let c = seed(inner.clone()).await;
    let probe = Arc::new(Probe {
        inner,
        hold: AtomicBool::new(true),
        lose_ack: AtomicBool::new(true),
        entered: Notify::new(),
        release: Notify::new(),
        requests: parking_lot::Mutex::default(),
    });
    let (zone, driver, log) = actor(probe.clone()).await;
    let mut output = zone.subscribe();
    let registry = SessionRegistry::with_characters(probe.clone());
    registry
        .admit(&zone, SessionId::new(), SessionGeneration(1), &spawn(&c))
        .await
        .unwrap();
    let step = tokio::spawn(async move { driver.step().await });
    probe.entered.notified().await;
    assert!(log
        .subjects()
        .iter()
        .any(|(_, subject)| subject.ends_with(".applied")));
    assert!(output.try_recv().is_err());
    assert!(tokio::time::timeout(Duration::from_millis(10), zone.snapshot())
        .await
        .is_err());
    probe.hold.store(false, Ordering::Release);
    probe.release.notify_one();
    assert_eq!(step.await.unwrap().unwrap(), TickOutcome::Ran(Tick(0)));
    {
        let requests = probe.requests.lock();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0], requests[1]);
        assert_eq!(requests[0].phase3.as_ref().unwrap().effect_remainders[0].remaining_ticks, 600);
    }
    assert!(!output.recv().await.unwrap().outputs.is_empty());
    assert_eq!(
        probe
            .load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        2
    );
}

#[tokio::test]
async fn durable_tail_rebuilds_fractions_reuse_and_effects_before_fresh_admission() {
    use nightfall_api::application::replay_log::AppliedTickRecord;
    for head in [35, 55, 605] {
        let repo = Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules()));
        let c = seed(repo.clone()).await;
        let (zone, driver, log) = actor(repo.clone()).await;
        let entity = spawn(&c).entity;
        SessionRegistry::with_characters(repo.clone())
            .admit(&zone, SessionId::new(), SessionGeneration(1), &spawn(&c))
            .await
            .unwrap();
        driver.step().await.unwrap();
        let baseline = zone.snapshot().await.unwrap();
        log.write_recovery_snapshot(&baseline).await.unwrap();
        drop(driver);
        zone.stopped().await;
        let before = repo.load_for_admission(c.id).await.unwrap().unwrap();
        let facts = repo.staged_events();
        let mut state = ZoneState::from_snapshot(baseline.clone()).unwrap();
        // All these ticks have durable ACKs, but no corresponding DB checkpoint.
        while state.next_tick() <= Tick(head) {
            let applied = state.run_tick(state.draft(vec![])).unwrap();
            log.append_applied(&AppliedTickRecord::from_applied(ZoneId(77), &applied))
                .await
                .unwrap();
        }
        let expected = state.snapshot();
        assert_ne!(baseline.phase3, expected.phase3);
        let expected_cp = expected
            .phase3
            .as_ref()
            .unwrap()
            .checkpoint(entity, Tick(head))
            .unwrap();
        if head == 35 {
            assert_ne!(
                expected_cp.resource_fractions,
                before.phase3.as_ref().unwrap().resource_fractions
            );
        }
        if head >= 55 {
            assert!(expected_cp.reuse_remainders.is_empty());
        }
        if head >= 605 {
            assert!(expected_cp.effect_remainders.is_empty());
        }
        let mut recovery = CheckpointService::new(
            repo.clone(),
            Arc::new(InMemorySessionAudit::default()),
            Arc::new(Metrics::detached()),
        );
        recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
        let restored = repo.load_for_admission(c.id).await.unwrap().unwrap();
        assert_eq!(restored.phase3, Some(expected_cp));
        let combat = expected
            .entities
            .iter()
            .find(|e| e.id == entity)
            .unwrap()
            .combat
            .as_ref()
            .unwrap();
        assert_eq!((restored.hp, restored.mp), (Some(combat.hp), Some(combat.mp)));
        recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
        assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), restored);
        assert_eq!(repo.staged_events(), facts, "recovery creates no duplicate outbox facts");
    }
}

struct BatchProbe {
    inner: Arc<dyn CharacterRepository>,
    armed: AtomicBool,
    failure: std::sync::atomic::AtomicU8,
    failed_id: CharacterId,
    lose_ack: AtomicBool,
    entered: tokio::sync::Semaphore,
    gates: std::collections::BTreeMap<CharacterId, tokio::sync::Semaphore>,
    requests: parking_lot::Mutex<Vec<CharacterCheckpoint>>,
}
#[async_trait::async_trait]
impl CharacterRepository for BatchProbe {
    async fn get(&self, id: CharacterId) -> anyhow::Result<Option<Character>> {
        self.inner.get(id).await
    }
    async fn list_by_account(&self, id: AccountId) -> anyhow::Result<Vec<Character>> {
        self.inner.list_by_account(id).await
    }
    async fn create_idempotent(
        &self,
        k: &IdempotencyKey,
        f: &str,
        c: &Character,
    ) -> Result<CreateOutcome, RepositoryError> {
        self.inner.create_idempotent(k, f, c).await
    }
    async fn load_for_admission(
        &self,
        id: CharacterId,
    ) -> anyhow::Result<Option<ProgressionState>> {
        self.inner.load_for_admission(id).await
    }
    async fn checkpoint(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
    ) -> Result<CheckpointOutcome, CheckpointError> {
        if self.armed.load(Ordering::Acquire) {
            assert!(events.is_empty(), "critical resource changes have no outbox facts");
            self.requests.lock().push(cp.clone());
            self.entered.add_permits(1);
            self.gates[&cp.character_id]
                .acquire()
                .await
                .unwrap()
                .forget();
            if cp.character_id == self.failed_id {
                match self.failure.load(Ordering::Acquire) {
                    1 => return Ok(CheckpointOutcome::Stale),
                    2 => {
                        return Err(CheckpointError::Constraint(
                            "injected permanent failure".into(),
                        ))
                    },
                    _ => {},
                }
            }
        }
        let result = self.inner.checkpoint(cp, events).await?;
        if self.armed.load(Ordering::Acquire)
            && cp.character_id == self.failed_id
            && self.lose_ack.swap(false, Ordering::AcqRel)
        {
            return Err(anyhow::anyhow!("lost batch acknowledgement").into());
        }
        Ok(result)
    }
}
fn lag(metrics: &Metrics) -> f64 {
    metrics
        .render()
        .unwrap()
        .lines()
        .find(|l| l.starts_with("nightfall_checkpoint_lag_seconds"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .parse()
        .unwrap()
}
async fn batch_case(failure: u8, release_second_first: bool, lose_ack: bool) {
    use futures_util::StreamExt as _;
    let inner = Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules()));
    let a = seed_named(inner.clone(), "BatchFirst").await;
    let b = seed_named(inner.clone(), "BatchSecond").await;
    let probe = Arc::new(BatchProbe {
        inner: inner.clone(),
        armed: AtomicBool::new(false),
        failure: std::sync::atomic::AtomicU8::new(failure),
        failed_id: a.id,
        lose_ack: AtomicBool::new(lose_ack),
        entered: tokio::sync::Semaphore::new(0),
        gates: [a.id, b.id]
            .into_iter()
            .map(|id| (id, tokio::sync::Semaphore::new(0)))
            .collect(),
        requests: parking_lot::Mutex::default(),
    });
    let metrics = Arc::new(Metrics::detached());
    let (zone, driver, log) = actor_metrics(probe.clone(), metrics.clone(), None).await;
    let registry = SessionRegistry::with_characters(probe.clone());
    for c in [&a, &b] {
        registry
            .admit(&zone, SessionId::new(), SessionGeneration(1), &spawn(c))
            .await
            .unwrap();
    }
    for _ in 0..30 {
        driver.step().await.unwrap();
    }
    let before = zone.snapshot().await.unwrap();
    assert_eq!(before.tick, Tick(30));
    assert!(before.checkpoints.iter().all(|p| p.last_saved == Tick(0)));
    let facts = inner.staged_events();
    probe.armed.store(true, Ordering::Release);
    let mut output = zone.subscribe();
    let step = tokio::spawn(async move { driver.step().await });
    tokio::time::timeout(Duration::from_secs(10), probe.entered.acquire_many(2))
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert!(output.try_recv().is_err());
    assert_eq!(log.read_epoch(ZoneId(77), 1).await.unwrap().count().await, 31);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(lag(&metrics) > 0.0);
    let first = if release_second_first { b.id } else { a.id };
    let second = if release_second_first { a.id } else { b.id };
    probe.gates[&first].add_permits(1);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!step.is_finished(), "the blocked peer must settle even after a failure");
    assert!(output.try_recv().is_err());
    assert!(lag(&metrics) > 0.0, "one acknowledgement must not clear the shared lag gauge");
    probe.gates[&second].add_permits(1);
    if lose_ack {
        tokio::time::timeout(Duration::from_secs(10), probe.entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        probe.gates[&a.id].add_permits(1);
    }
    assert_eq!(
        step.await.unwrap().unwrap(),
        if failure == 0 {
            TickOutcome::Ran(Tick(30))
        } else {
            TickOutcome::Held(Tick(30))
        }
    );
    assert!(lag(&metrics) <= f64::EPSILON);
    if failure == 0 {
        assert_eq!(output.recv().await.unwrap().tick, Tick(30));
    } else {
        zone.stopped().await;
        assert!(zone.persistence_failed());
        assert!(output.try_recv().is_err());
    }
    let requests = probe.requests.lock().clone();
    assert!(requests
        .iter()
        .all(|cp| cp.class_state.as_ref().unwrap().cp > 0));
    if lose_ack {
        let a_requests: Vec<_> = requests.iter().filter(|c| c.character_id == a.id).collect();
        assert_eq!(a_requests.len(), 2);
        assert_eq!(a_requests[0], a_requests[1]);
    }
    // A failed tick has one committed player and one missing checkpoint. Replay both twice.
    let mut recovery = CheckpointService::new(
        inner.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    );
    recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
    let mut restored = Vec::new();
    for id in [a.id, b.id] {
        let state = inner.load_for_admission(id).await.unwrap().unwrap();
        let cp = requests.iter().find(|cp| cp.character_id == id).unwrap();
        assert_eq!(state.revision, cp.revision_seen + 1);
        assert_eq!(state.phase3, cp.phase3);
        assert_eq!(state.class_state, cp.class_state.clone().unwrap());
        restored.push(state);
    }
    recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
    for (id, expected) in [a.id, b.id].into_iter().zip(restored) {
        assert_eq!(inner.load_for_admission(id).await.unwrap().unwrap(), expected);
    }
    assert_eq!(inner.staged_events(), facts);
}
#[tokio::test]
async fn v5_critical_saves_overlap_and_either_single_ack_keeps_output_and_lag_pending() {
    batch_case(0, false, false).await;
    batch_case(0, true, true).await;
}
#[tokio::test]
async fn v5_stale_and_permanent_failures_settle_blocked_peer_then_recover_partial_batch_twice() {
    batch_case(1, false, false).await;
    batch_case(2, false, false).await;
}

#[tokio::test]
async fn zero_write_ticks_leave_completion_marker_older_and_indexed_recovery_replays_tail() {
    use nightfall_api::application::replay_log::ZoneSnapshotStore;
    use nightfall_api::infrastructure::postgres::PgZoneSnapshotStore;
    let pool = common::pg::migrated_pool()
        .await
        .expect("owned DATABASE_URL required");
    sqlx::query("INSERT INTO zone_epochs(zone_id,epoch) VALUES(77,1)")
        .execute(&pool)
        .await
        .unwrap();
    let index = Arc::new(PgZoneSnapshotStore::new(pool.clone()));
    let repo = Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules()));
    let c = seed(repo.clone()).await;
    let (zone, driver, log) =
        actor_metrics(repo.clone(), Arc::new(Metrics::detached()), Some(index.clone())).await;
    SessionRegistry::with_characters(repo.clone())
        .admit(&zone, SessionId::new(), SessionGeneration(1), &spawn(&c))
        .await
        .unwrap();
    for _ in 0..36 {
        driver.step().await.unwrap();
    }
    let boundary = zone.snapshot().await.unwrap();
    let markers: (i64, i64) = sqlx::query_as("SELECT last_recorded_tick,last_checkpointed_tick FROM zone_epochs WHERE zone_id=77 AND epoch=1").fetch_one(&pool).await.unwrap();
    assert_eq!(markers, (35, 30));
    drop(driver);
    zone.stopped().await;
    let mut recovery = CheckpointService::new(
        repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    );
    recovery
        .recover_indexed(log.as_ref(), index.as_ref(), ZoneId(77))
        .await
        .unwrap();
    let saved = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(
        saved.phase3,
        Some(
            boundary
                .phase3
                .unwrap()
                .checkpoint(spawn(&c).entity, Tick(35))
                .unwrap()
        )
    );
    assert!(index.unresolved(ZoneId(77)).await.unwrap().is_empty());
    recovery
        .recover_indexed(log.as_ref(), index.as_ref(), ZoneId(77))
        .await
        .unwrap();
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), saved);
}

#[tokio::test]
async fn internal_retained_state_waits_for_cadence_and_survives_both_crash_windows() {
    use nightfall_api::application::replay_log::AppliedTickRecord;
    for head in [35, 50] {
        let repo = Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules()));
        let c = seed_full_cp(repo.clone()).await;
        let (zone, driver, log) = actor(repo.clone()).await;
        SessionRegistry::with_characters(repo.clone())
            .admit(&zone, SessionId::new(), SessionGeneration(1), &spawn(&c))
            .await
            .unwrap();
        driver.step().await.unwrap();
        let baseline = zone.snapshot().await.unwrap();
        log.write_recovery_snapshot(&baseline).await.unwrap();
        let at_admission = repo.load_for_admission(c.id).await.unwrap().unwrap();
        for _ in 1..=head.min(49) {
            driver.step().await.unwrap();
        }
        let boundary = zone.snapshot().await.unwrap();
        assert!(boundary.checkpoints[0].dirty, "fractional changes must remain pending");
        assert_eq!(
            repo.load_for_admission(c.id).await.unwrap().unwrap(),
            at_admission,
            "no internal-state save before cadence"
        );
        drop(driver);
        zone.stopped().await;
        let mut state = ZoneState::from_snapshot(boundary).unwrap();
        if head == 50 {
            // Crash after the cadence tick's durable ACK, before its checkpoint.
            let applied = state.run_tick(state.draft(vec![])).unwrap();
            assert_eq!(applied.tick, Tick(50));
            log.append_applied(&AppliedTickRecord::from_applied(ZoneId(77), &applied))
                .await
                .unwrap();
        }
        let expected = state
            .snapshot()
            .phase3
            .unwrap()
            .checkpoint(spawn(&c).entity, Tick(head))
            .unwrap();
        let facts = repo.staged_events();
        let mut recovery = CheckpointService::new(
            repo.clone(),
            Arc::new(InMemorySessionAudit::default()),
            Arc::new(Metrics::detached()),
        );
        recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
        let restored = repo.load_for_admission(c.id).await.unwrap().unwrap();
        assert_eq!(restored.revision, at_admission.revision + 1);
        assert_eq!(restored.phase3, Some(expected));
        recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
        assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), restored);
        assert_eq!(repo.staged_events(), facts);
    }
}

async fn seed_full_cp(repo: Arc<dyn CharacterRepository>) -> Character {
    seed_full_cp_named(repo, "DormantMage").await
}
async fn seed_full_cp_named(repo: Arc<dyn CharacterRepository>, name: &str) -> Character {
    let c = seed_named(repo.clone(), name).await;
    let saved = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let (_, max_cp, _, _) = profession_stats(
        &load_rules(&RulesSource::embedded()).unwrap().rules,
        &classes(),
        &c.identity(),
        &saved.class_state,
        c.level,
    )
    .unwrap();
    let mut ledger = saved.class_state;
    ledger.cp = max_cp;
    let cp = CharacterCheckpoint {
        character_id: c.id,
        revision_seen: saved.revision,
        level: saved.level,
        xp: saved.xp,
        hp: saved.hp.unwrap(),
        mp: saved.mp.unwrap(),
        alive: saved.alive,
        position: saved.position,
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        class_state: Some(ledger),
        phase3: saved.phase3,
    };
    repo.checkpoint(&cp, &[]).await.unwrap();
    c
}

#[tokio::test]
async fn historical_checkpoint_policy_recovers_existing_keys_and_survives_snapshot_refresh() {
    let repo = Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules()));
    let c = seed_full_cp(repo.clone()).await;
    let (zone, driver, log) = actor(repo.clone()).await;
    SessionRegistry::with_characters(repo.clone())
        .admit(&zone, SessionId::new(), SessionGeneration(1), &spawn(&c))
        .await
        .unwrap();
    driver.step().await.unwrap();
    let mut baseline = zone.snapshot().await.unwrap();
    baseline.meta.checkpoint_internal_cadence = false;
    let json = serde_json::to_string(&baseline).unwrap();
    assert!(!json.contains("checkpoint_internal_cadence"));
    zone.checkpoints
        .service()
        .unwrap()
        .lock()
        .await
        .restore(&baseline);
    log.write_recovery_snapshot(&baseline).await.unwrap();
    let initial_revision = repo
        .load_for_admission(c.id)
        .await
        .unwrap()
        .unwrap()
        .revision;
    for _ in 1..=65 {
        driver.step().await.unwrap();
    }
    let boundary = zone.snapshot().await.unwrap();
    assert!(!boundary.meta.checkpoint_internal_cadence);
    let before = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert!(
        before.revision > initial_revision + 1,
        "historical per-change saves remain enabled"
    );
    drop(driver);
    zone.stopped().await;
    let mut recovery = CheckpointService::new(
        repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    );
    recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
    let restored = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(
        restored.phase3,
        Some(
            boundary
                .phase3
                .unwrap()
                .checkpoint(spawn(&c).entity, Tick(65))
                .unwrap()
        )
    );
    assert_eq!(
        restored.revision,
        before.revision + 1,
        "historical receipts replay, only the final head flush is new"
    );
    recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), restored);
}

#[path = "phase3_live_checkpoint/startup.rs"]
mod startup;
