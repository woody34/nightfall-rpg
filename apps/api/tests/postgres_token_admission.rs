//! Real transaction failures and lost commit acknowledgements must fence admission visibility.
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]
mod common;

use common::token::{balances, character, grants, spawned, Harness};
use common::ws;
use nightfall_api::application::ports::RepositoryError;
use nightfall_api::application::replay_log::{WatermarkReason, ZoneSnapshotStore};
use nightfall_api::application::{
    CharacterCheckpoint, CharacterRepository, CheckpointError, CheckpointOutcome, CreateOutcome,
    IdempotencyKey, ProgressionState,
};
use nightfall_api::domain::{AccountId, Character, CharacterId, DomainEvent};
use nightfall_api::infrastructure::postgres::PgCharacterRepository;
use sqlx::Executor;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Default)]
struct LostAck {
    once: AtomicBool,
    hold_retry: AtomicBool,
    committed: Notify,
    entered: Notify,
    release: Notify,
    requests: parking_lot::Mutex<Vec<(CharacterCheckpoint, Vec<DomainEvent>)>>,
}
struct Repository {
    inner: Arc<PgCharacterRepository>,
    fault: Arc<LostAck>,
}
#[async_trait::async_trait]
impl CharacterRepository for Repository {
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
        let retry = {
            let mut requests = self.fault.requests.lock();
            let retry = !requests.is_empty();
            requests.push((cp.clone(), events.to_vec()));
            retry
        };
        if retry {
            self.fault.entered.notify_one();
            while self.fault.hold_retry.load(Ordering::Acquire) {
                self.fault.release.notified().await;
            }
        }
        let result = self.inner.checkpoint(cp, events).await?;
        if self.fault.once.swap(false, Ordering::AcqRel) {
            assert!(matches!(result, CheckpointOutcome::Applied(_)));
            self.fault.committed.notify_one();
            return Err(anyhow::anyhow!("injected loss after real Postgres commit").into());
        }
        Ok(result)
    }
}

#[tokio::test]
async fn lost_real_postgres_commit_ack_retries_identical_checkpoint_before_first_spawn_with_one_durable_grant(
) {
    let fault = Arc::new(LostAck::default());
    fault.once.store(true, Ordering::Release);
    fault.hold_retry.store(true, Ordering::Release);
    let controls = fault.clone();
    let h = Harness::with_adapter(move |inner| {
        Arc::new(Repository {
            inner,
            fault: controls,
        })
    })
    .await;
    let c = character("Lostack", 40, [0, 0], 0);
    let p = h.seed(&c).await;
    let mut socket = ws::join(&h.app, &p).await;
    tokio::time::timeout(ws::WAIT, fault.committed.notified())
        .await
        .unwrap();
    tokio::time::timeout(ws::WAIT, fault.entered.notified())
        .await
        .unwrap();
    assert!(socket.recv(Duration::from_millis(100)).await.is_none());
    assert_eq!(balances(&h.repo.get(c.id).await.unwrap().unwrap()), ([1, 1], 3));
    assert_eq!(
        h.repo
            .load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    let first_rows = grants(&h.pool, &c).await;
    assert_eq!(first_rows.len(), 2);
    {
        let attempts = fault.requests.lock();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0], attempts[1]);
    }
    fault.hold_retry.store(false, Ordering::Release);
    fault.release.notify_one();
    socket.until(|m| spawned(m, &p)).await;
    assert_eq!(
        h.repo
            .load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    assert_eq!(grants(&h.pool, &c).await, first_rows);
    assert_eq!(h.store.unresolved(h.running.zone()).await.unwrap().len(), 1);
    socket.close().await;
    h.running.shutdown(WatermarkReason::Shutdown).await.unwrap();
}

#[tokio::test]
async fn real_postgres_constraint_failure_rolls_back_grant_and_key_and_stops_admission_before_visibility(
) {
    let h = Harness::new().await;
    let c = character("Databasefault", 40, [0, 0], 0);
    let p = h.seed(&c).await;
    let keys_before: i64 = sqlx::query_scalar("SELECT count(*) FROM idempotency_keys")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    let outbox_before: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    h.pool.execute("ALTER TABLE characters ADD CONSTRAINT reject_token_supply CHECK (token_tier_1_count=0 AND token_tier_2_count=0) NOT VALID").await.unwrap();
    let mut socket = ws::join(&h.app, &p).await;
    tokio::time::timeout(ws::WAIT, h.app.zone.stopped())
        .await
        .unwrap();
    assert!(h.app.zone.persistence_failed());
    // A close frame is allowed; no entity/stats/world frame may escape the failed tick.
    assert!(!matches!(
        socket.recv(Duration::from_millis(100)).await,
        Some(ws::Received::Message(..))
    ));
    assert_eq!(balances(&h.repo.get(c.id).await.unwrap().unwrap()), ([0, 0], 0));
    assert_eq!(
        h.repo
            .load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        0
    );
    assert!(grants(&h.pool, &c).await.is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM idempotency_keys")
            .fetch_one(&h.pool)
            .await
            .unwrap(),
        keys_before
    ); // play tickets use the harness memory adapter
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM outbox")
            .fetch_one(&h.pool)
            .await
            .unwrap(),
        outbox_before
    );
    let indexed = h.store.unresolved(h.running.zone()).await.unwrap();
    assert_eq!(indexed.len(), 1);
    assert!(indexed[0].last_recorded_tick.is_some());
    assert!(h.running.shutdown(WatermarkReason::Shutdown).await.is_err());
}

#[tokio::test]
async fn concurrent_replacement_during_lost_checkpoint_ack_waits_for_commit_barrier_and_adds_no_supply(
) {
    let fault = Arc::new(LostAck::default());
    fault.once.store(true, Ordering::Release);
    fault.hold_retry.store(true, Ordering::Release);
    let controls = fault.clone();
    let h = Harness::with_adapter(move |inner| {
        Arc::new(Repository {
            inner,
            fault: controls,
        })
    })
    .await;
    let c = character("Concurrentjoin", 40, [0, 0], 0);
    let p = h.seed(&c).await;
    let mut first = ws::join(&h.app, &p).await;
    tokio::time::timeout(ws::WAIT, fault.committed.notified())
        .await
        .unwrap();
    tokio::time::timeout(ws::WAIT, fault.entered.notified())
        .await
        .unwrap();
    let facts = grants(&h.pool, &c).await;
    // The second real HTTP/WebSocket upgrade arrives while the initial recorded
    // admission's real commit ACK is unresolved. Both generations remain invisible.
    let mut replacement = ws::join(&h.app, &p).await;
    assert!(first.recv(Duration::from_millis(100)).await.is_none());
    assert!(replacement.recv(Duration::from_millis(100)).await.is_none());
    assert_eq!(facts.len(), 2);
    fault.hold_retry.store(false, Ordering::Release);
    fault.release.notify_one();
    replacement.until(|m| spawned(m, &p)).await;
    assert_eq!(first.closed(ws::WAIT).await, Some(4409));
    assert_eq!(balances(&h.repo.get(c.id).await.unwrap().unwrap()), ([1, 1], 3));
    assert_eq!(grants(&h.pool, &c).await, facts);
    assert_eq!(
        h.repo
            .load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    replacement.close().await;
    h.running.shutdown(WatermarkReason::Shutdown).await.unwrap();
}
