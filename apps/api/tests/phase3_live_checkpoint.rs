//! Real experimental actor, repository admission, paused logout and recovery barriers.
#![allow(missing_docs, clippy::unwrap_used, clippy::indexing_slicing)]
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
    let skill = rules.skills().get(SkillId(1040)).unwrap();
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
        expires_at: Some(Tick(600)),
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
    let c = CreateCharacter::new(repo.clone(), classes())
        .execute(CreateCharacterInput {
            idempotency_key: IdempotencyKey::new(),
            account_id: AccountId::from_uuid(Uuid::nil()),
            name: "DormantMage".into(),
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
    zone.checkpoints.install(CheckpointService::new(
        repo,
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    ));
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
