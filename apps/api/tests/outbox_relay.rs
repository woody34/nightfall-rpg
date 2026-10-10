//! Relay tests. Need `DATABASE_URL`; the `JetStream` tests also need `NATS_URL`.
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use nightfall_api::infrastructure::outbox::{JetStreamPublisher, OutboxPublisher, OutboxRelay};
use nightfall_api::infrastructure::postgres::{connection_from_pool, Migrator};
use nightfall_api::infrastructure::telemetry::Metrics;
use parking_lot::Mutex;
use sea_orm::DatabaseConnection;
use sea_orm_migration::MigratorTrait;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

async fn migrated() -> Option<(PgPool, DatabaseConnection)> {
    let pool = common::pg::empty_schema_pool().await?;
    let db = connection_from_pool(&pool);
    Migrator::up(&db, None).await.unwrap();
    Some((pool, db))
}

/// Stages a row with an explicit, globally unique id: tests share one `JetStream` stream, and
/// `Nats-Msg-Id` dedupe would otherwise collide across per-test schemas that all start at 1.
async fn stage(pool: &PgPool, subject: &str) -> i64 {
    let id = i64::from_ne_bytes(
        Uuid::now_v7().as_u128().to_ne_bytes()[..8]
            .try_into()
            .unwrap(),
    )
    .abs();
    sqlx::query(
        "INSERT INTO outbox (id, subject, payload) OVERRIDING SYSTEM VALUE VALUES ($1, $2, $3)",
    )
    .bind(id)
    .bind(subject)
    .bind(serde_json::json!({"hello": "world"}))
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn pending(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM outbox WHERE published_at IS NULL")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn wait_until_drained(pool: &PgPool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while pending(pool).await > 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("outbox did not drain");
}

/// Records publishes; fails the first `fail_first` calls.
#[derive(Default)]
struct FakePublisher {
    fail_first: usize,
    calls: AtomicUsize,
    published: Mutex<Vec<i64>>,
    delay: Duration,
}

#[async_trait]
impl OutboxPublisher for FakePublisher {
    async fn publish(&self, _subject: &str, msg_id: i64, _payload: Bytes) -> anyhow::Result<()> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        if n < self.fail_first {
            anyhow::bail!("broker unavailable");
        }
        self.published.lock().push(msg_id);
        Ok(())
    }
}

async fn stop(relay: OutboxRelay, token: CancellationToken) {
    token.cancel();
    relay.join().await;
}

#[tokio::test]
async fn failed_publish_leaves_row_unpublished_then_retries() {
    let Some((pool, db)) = migrated().await else {
        return;
    };
    let id = stage(&pool, "nightfall.test.retry").await;
    let publisher = Arc::new(FakePublisher {
        fail_first: 1,
        ..Default::default()
    });
    let token = CancellationToken::new();
    let relay = OutboxRelay::spawn(db, publisher.clone(), &Metrics::detached(), token.clone());

    // First attempt fails: the row must still be pending.
    tokio::time::timeout(Duration::from_secs(5), async {
        while publisher.calls.load(Ordering::SeqCst) < 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(publisher.published.lock().is_empty());

    // Backoff elapses, retry succeeds, row is marked.
    wait_until_drained(&pool).await;
    assert_eq!(*publisher.published.lock(), vec![id]);
    assert!(relay.stats().oldest_pending_age_seconds().abs() < f64::EPSILON);
    assert_eq!(relay.stats().pending(), 0);
    stop(relay, token).await;
}

#[tokio::test]
async fn concurrent_relays_never_double_publish() {
    let Some((pool, db)) = migrated().await else {
        return;
    };
    let mut ids = Vec::new();
    for _ in 0..250 {
        ids.push(stage(&pool, "nightfall.test.concurrent").await);
    }
    // The delay keeps each batch's locks held long enough for the two relays to overlap.
    let publisher = Arc::new(FakePublisher {
        delay: Duration::from_millis(2),
        ..Default::default()
    });
    let token = CancellationToken::new();
    let a = OutboxRelay::spawn(db.clone(), publisher.clone(), &Metrics::detached(), token.clone());
    let b = OutboxRelay::spawn(db, publisher.clone(), &Metrics::detached(), token.clone());
    wait_until_drained(&pool).await;
    stop(a, token.clone()).await;
    stop(b, token).await;

    let sent = publisher.published.lock().clone();
    let unique: HashSet<i64> = sent.iter().copied().collect();
    assert_eq!(sent.len(), ids.len(), "every row published exactly once");
    assert_eq!(unique.len(), ids.len());
}

#[tokio::test]
async fn staged_row_is_published_to_jetstream_and_marked() {
    let Some((pool, db)) = migrated().await else {
        return;
    };
    let Ok(url) = std::env::var("NATS_URL") else {
        return;
    };
    let client = async_nats::connect(&url).await.unwrap();
    let subject = format!("nightfall.test.{}", Uuid::now_v7().simple());
    let mut sub = client.subscribe(subject.clone()).await.unwrap();
    stage(&pool, &subject).await;

    let publisher = Arc::new(JetStreamPublisher::connect(client).await.unwrap());
    let token = CancellationToken::new();
    let relay = OutboxRelay::spawn(db, publisher, &Metrics::detached(), token.clone());
    let msg = tokio::time::timeout(Duration::from_secs(10), sub.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(msg.payload.as_ref(), br#"{"hello":"world"}"#);
    wait_until_drained(&pool).await;
    stop(relay, token).await;
}

#[tokio::test]
async fn crash_after_publish_before_mark_is_deduped_and_row_ends_marked() {
    let Some((pool, db)) = migrated().await else {
        return;
    };
    let Ok(url) = std::env::var("NATS_URL") else {
        return;
    };
    let client = async_nats::connect(&url).await.unwrap();
    let publisher = Arc::new(JetStreamPublisher::connect(client.clone()).await.unwrap());
    let subject = format!("nightfall.test.{}", Uuid::now_v7().simple());
    let id = stage(&pool, &subject).await;

    // "Crash": the first relay publishes and is killed before it marks the row.
    publisher
        .publish(&subject, id, Bytes::from_static(br#"{"hello":"world"}"#))
        .await
        .unwrap();
    assert_eq!(pending(&pool).await, 1);

    // Restart: the retry carries the same Nats-Msg-Id, so the broker drops it.
    let token = CancellationToken::new();
    let relay = OutboxRelay::spawn(db, publisher, &Metrics::detached(), token.clone());
    wait_until_drained(&pool).await;
    stop(relay, token).await;

    // Core subscribers see every publish the server receives; dedupe is a property of the
    // stream, so count what the stream stored.
    let stream = async_nats::jetstream::new(client)
        .get_stream("NF_EVENTS")
        .await
        .unwrap();
    let mut subjects = stream.info_with_subjects(&subject).await.unwrap();
    let (_, stored) = subjects.next().await.unwrap().unwrap();
    assert_eq!(stored, 1, "the retried publish must be deduplicated by the broker");
}

/// Value of the unlabelled-by-us gauge `name` in a Prometheus exposition.
fn gauge(text: &str, name: &str) -> f64 {
    text.lines()
        .find(|l| l.starts_with(name))
        .and_then(|l| l.rsplit(' ').next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(f64::NAN)
}

async fn wait_for_gauge(metrics: &Metrics, name: &str, ok: impl Fn(f64) -> bool) -> f64 {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let v = gauge(&metrics.render().unwrap(), name);
            if ok(v) {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("gauge never reached the expected value")
}

/// Production wiring: `OutboxRelay::spawn` registers its stats on the exported `Metrics`, so
/// the Prometheus exposition (the same instruments OTLP exports) shows the real outbox state.
#[tokio::test]
async fn gauges_track_a_stalled_relay_even_while_publishes_fail_then_reset_when_drained() {
    let Some((pool, db)) = migrated().await else {
        return;
    };
    for _ in 0..2 {
        stage(&pool, "nightfall.test.gauges").await;
    }
    // Backdate so the expected lag is far above scheduling noise.
    sqlx::query("UPDATE outbox SET created_at = now() - interval '40 seconds'")
        .execute(&pool)
        .await
        .unwrap();

    let metrics = Metrics::detached();
    let publisher = Arc::new(FakePublisher {
        fail_first: 4,
        ..Default::default()
    });
    let token = CancellationToken::new();
    let relay = OutboxRelay::spawn(db, publisher.clone(), &metrics, token.clone());

    // Broker is failing: rows stay pending and the lag is the oldest row's age, not zero.
    let pending = wait_for_gauge(&metrics, "nightfall_outbox_pending", |v| v >= 2.0).await;
    assert!((pending - 2.0).abs() < f64::EPSILON);
    let lag = gauge(&metrics.render().unwrap(), "nightfall_outbox_lag_seconds");
    assert!((40.0..70.0).contains(&lag), "lag {lag}");
    assert!(publisher.published.lock().is_empty());

    // Broker recovers: the outbox drains and both gauges return to zero.
    wait_until_drained(&pool).await;
    wait_for_gauge(&metrics, "nightfall_outbox_pending", |v| v == 0.0).await;
    wait_for_gauge(&metrics, "nightfall_outbox_lag_seconds", |v| v == 0.0).await;
    stop(relay, token).await;
}

#[tokio::test]
async fn progression_failed_commit_publishes_nothing_and_committed_retry_is_broker_deduped() {
    use nightfall_api::application::checkpoint::domain_events;
    use nightfall_api::application::{CharacterCheckpoint, CharacterRepository, IdempotencyKey};
    use nightfall_api::domain::zone::{
        DeathFact, EntityId, ProgressionDelta, Tick, Vec2Fixed, ZoneId,
    };
    use nightfall_api::domain::{AccountId, Character, CharacterName, Position, Race};
    use nightfall_api::infrastructure::postgres::PgCharacterRepository;
    let Some((pool, db)) = migrated().await else {
        return;
    };
    let Ok(url) = std::env::var("NATS_URL") else {
        return;
    };
    let client = async_nats::connect(url).await.unwrap();
    let publisher = Arc::new(JetStreamPublisher::connect(client.clone()).await.unwrap());
    let repo = PgCharacterRepository::new(db.clone());
    let c = Character::create(
        AccountId::from_uuid(Uuid::now_v7()),
        CharacterName::new("Relayer").unwrap(),
        Race::Human,
    );
    repo.create_idempotent(&IdempotencyKey::from_uuid(Uuid::now_v7()), "create", &c)
        .await
        .unwrap();
    sqlx::query("UPDATE outbox SET published_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    let delta = ProgressionDelta {
        tick: Tick(9),
        entity: EntityId::from_uuid(c.id.as_uuid()),
        level_before: 1,
        xp_before: 60,
        level: 2,
        xp: 340,
        hp: 0,
        mp: 12,
        alive: false,
        pos: Vec2Fixed::from_tiles(1, 2),
        xp_gained: 330,
        levels_gained: vec![2, 3],
        died: Some(DeathFact {
            killer: None,
            killer_template: Some("keltir".into()),
            xp_lost: 50,
        }),
        respawned: false,
    };
    let events = domain_events(ZoneId(1), 2, 1, &delta);
    let cp = CharacterCheckpoint {
        character_id: c.id,
        revision_seen: 0,
        level: 2,
        xp: 340,
        hp: 0,
        mp: 12,
        alive: false,
        position: Position { x: 1.0, y: 2.0 },
        idempotency: ("save_checkpoint".into(), IdempotencyKey::from_uuid(Uuid::now_v7())),
        class_state: None,
    };
    sqlx::query("ALTER TABLE outbox ADD CONSTRAINT reject_checkpoint CHECK (false) NOT VALID")
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.checkpoint(&cp, &events).await.is_err());
    assert_eq!(pending(&pool).await, 0);
    assert_eq!(
        repo.load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        0
    );
    sqlx::query("ALTER TABLE outbox DROP CONSTRAINT reject_checkpoint")
        .execute(&pool)
        .await
        .unwrap();
    // Independent test schemas share a broker: use globally distinct row ids in this fixture.
    let base = i64::try_from(Uuid::now_v7().as_u128() >> 80).unwrap();
    sqlx::query("SELECT setval(pg_get_serial_sequence('outbox', 'id'), $1)")
        .bind(base)
        .execute(&pool)
        .await
        .unwrap();
    repo.checkpoint(&cp, &events).await.unwrap();
    let rows: Vec<(i64, String, serde_json::Value)> = sqlx::query_as(
        "SELECT id,subject,payload FROM outbox WHERE published_at IS NULL ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 3);
    let stream = async_nats::jetstream::new(client)
        .get_stream("NF_EVENTS")
        .await
        .unwrap();
    let before = progression_count(&stream).await;
    // Crash after each broker ack, before any database publish mark.
    for (id, subject, payload) in &rows {
        publisher
            .publish(subject, *id, Bytes::from(serde_json::to_vec(payload).unwrap()))
            .await
            .unwrap();
    }
    let stop_token = CancellationToken::new();
    let relay = OutboxRelay::spawn(db, publisher, &Metrics::detached(), stop_token.clone());
    wait_until_drained(&pool).await;
    stop(relay, stop_token).await;
    assert_eq!(progression_count(&stream).await, before + 3);
    assert_eq!(
        repo.checkpoint(&cp, &events).await.unwrap(),
        nightfall_api::application::CheckpointOutcome::Replayed(1)
    );
    assert_eq!(pending(&pool).await, 0);
}

async fn progression_count(stream: &async_nats::jetstream::stream::Stream) -> usize {
    let mut count = 0_usize;
    for subject in ["nightfall.character.leveled", "nightfall.character.died"] {
        let mut info = stream.info_with_subjects(subject).await.unwrap();
        while let Some(row) = info.next().await {
            count = count.checked_add(row.unwrap().1).unwrap();
        }
    }
    count
}
