//! Story 4.2: `characters.phase5_checkpoint`, the `PbPlayerLoad` 11 producer and the economy
//! write inside the revision-fenced character checkpoint (c-review-4 P2-1). Memory cases always
//! run; Postgres cases run only with `DATABASE_URL`.
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value
)]

mod common;

use nightfall_api::application::economy::{
    ContainerDelta, DeltaChange, EconomyCheckpointRepository, EconomyCheckpointWrite, EconomyError,
    LeaseGrant, LeaseKey, LeaseKind, LeaseRepository, OwnerLedgerRow,
};
use nightfall_api::application::ports::{phase5_admission_load, phase5_checkpoint_from_column};
use nightfall_api::application::{
    CharacterCheckpoint, CharacterRepository, CheckpointError, CheckpointOutcome, IdempotencyKey,
};
use nightfall_api::domain::zone::{Phase5Checkpoint, RecipeBook};
use nightfall_api::domain::{AccountId, Character, CharacterName, Position, Race};
use nightfall_api::infrastructure::memory::{
    InMemoryCharacterRepository, InMemoryEconomyStore, StoredRow,
};
use nightfall_api::infrastructure::postgres::{PgCharacterRepository, PgEconomyRepository};
use sqlx::{PgPool, Row};
use uuid::Uuid;

const RULES: &str = "237e23b23f28a631cc068d974571d0ee1eea5670b5397fd3b27cb66b91be2eca";
const OTHER_RULES: &str = "0000000000000000000000000000000000000000000000000000000000000001";

fn key(n: u128) -> IdempotencyKey {
    IdempotencyKey::from_uuid(Uuid::from_u128(0x0190_a7e2_0000_7000_8000_0000_0000_0000 | n))
}

fn book(dwarven: &[u32]) -> Phase5Checkpoint {
    Phase5Checkpoint {
        schema_version: Phase5Checkpoint::SCHEMA_VERSION,
        rules_hash: RULES.into(),
        recipes: RecipeBook {
            dwarven: dwarven.iter().copied().collect(),
            common: [1].into_iter().collect(),
        },
    }
}

fn checkpoint(
    c: &Character,
    seen: u64,
    k: u128,
    phase5: Option<Phase5Checkpoint>,
) -> CharacterCheckpoint {
    CharacterCheckpoint {
        phase5,
        phase4: None,
        phase3: None,
        class_state: None,
        character_id: c.id,
        revision_seen: seen,
        level: 3,
        xp: 1_234,
        hp: 77,
        mp: 31,
        alive: true,
        position: Position { x: 12.5, y: -3.25 },
        idempotency: ("save_checkpoint".to_owned(), key(k)),
    }
}

fn character() -> Character {
    Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("Durin").unwrap(),
        Race::Dwarf,
    )
}

/// The shared adapter contract: round trip, fingerprint replay/conflict, downgrade fence and the
/// structural constraint. Both adapters must give the same answers (fingerprint parity).
async fn contract<R: CharacterRepository>(repo: &R, c: &Character) -> Vec<String> {
    let mut trace = Vec::new();
    assert_eq!(
        repo.load_for_admission(c.id).await.unwrap().unwrap().phase5,
        None,
        "NULL column"
    );
    let first = checkpoint(c, 0, 1, Some(book(&[322, 1])));
    let applied = repo.checkpoint(&first, &[]).await.unwrap();
    trace.push(format!("{applied:?}"));
    let loaded = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(loaded.phase5, first.phase5, "round trip");
    // Same key, same body (book included): replay. Same key, other book: KeyReused.
    trace.push(format!("{:?}", repo.checkpoint(&first, &[]).await.unwrap()));
    let other_book = checkpoint(c, 0, 1, Some(book(&[322])));
    assert_ne!(
        first.fingerprint(&[]),
        other_book.fingerprint(&[]),
        "the book is in the fingerprint"
    );
    trace.push(format!("{:?}", repo.checkpoint(&other_book, &[]).await.unwrap_err()));
    // A pre-V7 writer can never drop the stored book.
    let downgrade = repo
        .checkpoint(&checkpoint(c, 1, 2, None), &[])
        .await
        .unwrap_err();
    assert!(
        matches!(&downgrade, CheckpointError::Constraint(n) if n == "characters_phase5_downgrade"),
        "{downgrade:?}"
    );
    trace.push(format!("{downgrade:?}"));
    // A malformed book is refused before any write.
    let mut bad = book(&[]);
    bad.rules_hash = "ABC".into();
    let refused = repo
        .checkpoint(&checkpoint(c, 1, 3, Some(bad)), &[])
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, CheckpointError::Constraint(n) if n == "characters_phase5_checkpoint_valid"),
        "{refused:?}"
    );
    trace.push(format!("{refused:?}"));
    // The next book replaces the first.
    let second = checkpoint(c, 1, 4, Some(book(&[322, 1, 9])));
    trace.push(format!("{:?}", repo.checkpoint(&second, &[]).await.unwrap()));
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap().phase5, second.phase5);
    trace
}

fn bad_columns() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        (
            "unknown field",
            serde_json::json!({"schema_version": 1, "rules_hash": RULES, "recipes": {"dwarven": [], "common": []}, "extra": 1}),
        ),
        (
            "unknown recipe-book field",
            serde_json::json!({"schema_version": 1, "rules_hash": RULES, "recipes": {"dwarven": [], "common": [], "shadow": []}}),
        ),
        (
            "other schema version",
            serde_json::json!({"schema_version": 2, "rules_hash": RULES, "recipes": {"dwarven": [], "common": []}}),
        ),
        (
            "uppercase hash",
            serde_json::json!({"schema_version": 1, "rules_hash": RULES.to_uppercase(), "recipes": {"dwarven": [], "common": []}}),
        ),
        ("missing book", serde_json::json!({"schema_version": 1, "rules_hash": RULES})),
        ("not an object", serde_json::json!([1, 2])),
    ]
}

#[test]
fn producer_binds_the_book_to_the_epoch_rules() {
    // Pre-V7 epochs receive no book; a stored one offered to them fails closed.
    assert_eq!(phase5_admission_load(None, None).unwrap(), None);
    assert!(phase5_admission_load(Some(book(&[1])), None).is_err());
    // V7: a fresh character starts with an empty book under the epoch's rules.
    let fresh = phase5_admission_load(None, Some(RULES)).unwrap().unwrap();
    assert_eq!(fresh.rules_hash, RULES);
    assert_eq!(fresh.recipes, RecipeBook::default());
    assert_eq!(fresh.schema_version, 1);
    // A stored book under the same rules passes unchanged; under other rules it fails (D27).
    assert_eq!(
        phase5_admission_load(Some(book(&[322])), Some(RULES)).unwrap(),
        Some(book(&[322]))
    );
    assert!(phase5_admission_load(Some(book(&[322])), Some(OTHER_RULES)).is_err());
    assert!(phase5_admission_load(None, Some("not-hex")).is_err());
}

#[test]
fn column_reader_fails_closed_by_name() {
    assert_eq!(phase5_checkpoint_from_column(None).unwrap(), None);
    let good = serde_json::to_value(book(&[322])).unwrap();
    assert_eq!(phase5_checkpoint_from_column(Some(good)).unwrap(), Some(book(&[322])));
    for (label, value) in bad_columns() {
        let error = phase5_checkpoint_from_column(Some(value))
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("characters_phase5_checkpoint_invalid"), "{label}: {error}");
    }
}

#[test]
fn legacy_fingerprints_do_not_change() {
    let c = character();
    let legacy = checkpoint(&c, 0, 1, None);
    let mut body = serde_json::json!({
        "character_id": c.id.as_uuid(), "revision_seen": 0, "level": 3, "xp": 1_234, "hp": 77,
        "mp": 31, "alive": true, "position": [12.5_f32, -3.25_f32], "events": [],
    });
    let digest = |value: &serde_json::Value| {
        use sha2::Digest as _;
        sha2::Sha256::digest(value.to_string().as_bytes())
            .iter()
            .fold(String::new(), |mut out, b| {
                use std::fmt::Write as _;
                let _ = write!(out, "{b:02x}");
                out
            })
    };
    assert_eq!(legacy.fingerprint(&[]), digest(&body), "no phase5 key when absent");
    body.as_object_mut()
        .unwrap()
        .insert("phase5".into(), serde_json::json!(book(&[1])));
    assert_eq!(checkpoint(&c, 0, 1, Some(book(&[1]))).fingerprint(&[]), digest(&body));
}

#[tokio::test]
async fn memory_adapter_contract_and_strict_column() {
    let repo = InMemoryCharacterRepository::default();
    let c = character();
    repo.create_idempotent(&key(0x90), "fp", &c).await.unwrap();
    let trace = contract(&repo, &c).await;
    assert_eq!(trace[0], "Applied(1)");
    assert_eq!(trace[1], "Replayed(1)");
    for (label, value) in bad_columns() {
        repo.set_phase5_column_for_testing(c.id, value);
        let error = format!("{:#}", repo.load_for_admission(c.id).await.unwrap_err());
        assert!(error.contains("characters_phase5_checkpoint_invalid"), "{label}: {error}");
    }
}

async fn pg_seeded(pool: &PgPool) -> (PgCharacterRepository, Character) {
    let repo = PgCharacterRepository::new(pool.clone());
    let c = character();
    repo.create_idempotent(&key(0x90), "fp", &c).await.unwrap();
    (repo, c)
}

#[tokio::test]
async fn postgres_adapter_contract_matches_memory_and_strict_column() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let (repo, c) = pg_seeded(&pool).await;
    let pg_trace = contract(&repo, &c).await;
    let memory = InMemoryCharacterRepository::default();
    memory
        .create_idempotent(&key(0x90), "fp", &c)
        .await
        .unwrap();
    assert_eq!(pg_trace, contract(&memory, &c).await, "fingerprint and outcome parity");
    let stored: serde_json::Value =
        sqlx::query("SELECT phase5_checkpoint FROM characters WHERE id = $1")
            .bind(c.id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
    assert_eq!(stored, serde_json::to_value(book(&[322, 1, 9])).unwrap());
    for (label, value) in bad_columns() {
        sqlx::query("UPDATE characters SET phase5_checkpoint = $2 WHERE id = $1")
            .bind(c.id.as_uuid())
            .bind(value)
            .execute(&pool)
            .await
            .unwrap();
        let error = format!("{:#}", repo.load_for_admission(c.id).await.unwrap_err());
        assert!(error.contains("characters_phase5_checkpoint_invalid"), "{label}: {error}");
    }
}

fn grant(keys: Vec<LeaseKey>) -> LeaseGrant {
    LeaseGrant {
        zone_id: 1,
        epoch: 1,
        account_id: None,
        operation: "warehouse_deposit".into(),
        request_key: Uuid::from_u128(0x4200),
        keys,
    }
}

fn ledger_row(owner: Uuid, op: &str, ordinal: u32, after: serde_json::Value) -> OwnerLedgerRow {
    OwnerLedgerRow {
        owner_kind: "warehouse".into(),
        owner_id: owner,
        zone_id: 1,
        epoch: 1,
        tick: 30,
        revision: 1,
        op: op.into(),
        instance_id: 900_001,
        operation_ordinal: ordinal,
        delta: serde_json::json!({"before": {"count": 5}, "after": after}),
        receipt_ref: None,
    }
}

fn deposit(owner: Uuid) -> EconomyCheckpointWrite {
    let warehouse = LeaseKey::of_uuid(LeaseKind::Warehouse, owner);
    EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 1,
        tick: 30,
        deltas: vec![ContainerDelta {
            lease: warehouse,
            row_kind: "item_instance".into(),
            row_key: "900001".into(),
            change: DeltaChange::Update,
            expected_revision: Some(0),
            columns: [
                ("character_id".to_owned(), serde_json::json!(owner.to_string())),
                ("location".to_owned(), serde_json::json!("warehouse")),
                ("count".to_owned(), serde_json::json!(4)),
                ("owner_kind".to_owned(), serde_json::json!("warehouse")),
            ]
            .into_iter()
            .collect(),
        }],
        ledger: vec![
            ledger_row(owner, "transfer_in", 0, serde_json::json!({"count": 5})),
            ledger_row(owner, "consume", 1, serde_json::json!({"count": 4})),
        ],
        lease_releases: vec![warehouse],
        ..Default::default()
    }
}

async fn warehouse_row(pool: &PgPool) -> (i64, i64) {
    let row = sqlx::query("SELECT count, revision FROM item_instances WHERE id = 900001")
        .fetch_one(pool)
        .await
        .unwrap();
    (row.get("count"), row.get("revision"))
}

async fn revision(pool: &PgPool, c: &Character) -> i64 {
    sqlx::query("SELECT revision FROM characters WHERE id = $1")
        .bind(c.id.as_uuid())
        .fetch_one(pool)
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
async fn postgres_economy_write_commits_inside_the_fenced_character_checkpoint() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let (repo, c) = pg_seeded(&pool).await;
    let owner = c.id.as_uuid();
    sqlx::query("INSERT INTO item_instances(id,character_id,location,slot,template_id,count,created_tick,revision,owner_kind) VALUES(900001,$1,'warehouse',NULL,1864,5,0,0,'warehouse')")
        .bind(owner).execute(&pool).await.unwrap();
    let leases = PgEconomyRepository::new(
        nightfall_api::infrastructure::postgres::connection_from_pool(&pool),
    );
    let write = deposit(owner);
    leases
        .acquire(&grant(write.lease_releases.clone()))
        .await
        .unwrap();
    // A stale character revision fences the whole transaction: the economy write is not applied.
    let stale = repo
        .checkpoint_with_economy(&checkpoint(&c, 7, 1, Some(book(&[1]))), &[], None, &write)
        .await
        .unwrap();
    assert_eq!(stale, CheckpointOutcome::Stale);
    assert_eq!(warehouse_row(&pool).await, (5, 0));
    assert_eq!(leases.held(&write.lease_releases).await.unwrap().len(), 1, "lease still held");
    // Recovery released the epoch's leases: the writer is fenced and nothing is written, the
    // character revision and its book included.
    leases.release_epoch(1, 1).await.unwrap();
    let fenced = repo
        .checkpoint_with_economy(&checkpoint(&c, 0, 2, Some(book(&[1]))), &[], None, &write)
        .await
        .unwrap_err();
    assert!(
        matches!(&fenced, CheckpointError::Constraint(n) if n.starts_with("economy_lease_violation:warehouse:")),
        "{fenced:?}"
    );
    assert_eq!(warehouse_row(&pool).await, (5, 0));
    assert_eq!(revision(&pool, &c).await, 0);
    let book_column: Option<serde_json::Value> =
        sqlx::query("SELECT phase5_checkpoint FROM characters WHERE id = $1")
            .bind(owner)
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
    assert_eq!(book_column, None, "the book rolled back with the economy write");
    // Held leases: character, book, delta, ledger and lease delete commit together.
    leases
        .acquire(&grant(write.lease_releases.clone()))
        .await
        .unwrap();
    let applied = repo
        .checkpoint_with_economy(&checkpoint(&c, 0, 3, Some(book(&[1]))), &[], None, &write)
        .await
        .unwrap();
    assert_eq!(applied, CheckpointOutcome::Applied(1));
    assert_eq!(warehouse_row(&pool).await, (4, 1));
    assert_eq!(revision(&pool, &c).await, 1);
    assert!(leases.held(&write.lease_releases).await.unwrap().is_empty());
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap().phase5, Some(book(&[1])));
    // The standalone writer (tests and tools only) has the same lease fence.
    let again = leases.checkpoint_economy(&write).await.unwrap_err();
    assert!(matches!(again, EconomyError::LeaseViolation(_)), "{again:?}");
}

#[tokio::test]
async fn memory_economy_release_epoch_before_checkpoint_is_a_lease_violation() {
    let store = InMemoryEconomyStore::default();
    let owner = Uuid::from_u128(0x5000);
    let write = deposit(owner);
    store.put_row(
        "item_instance",
        "900001",
        StoredRow {
            revision: 0,
            columns: [("count".to_owned(), serde_json::json!(5))]
                .into_iter()
                .collect(),
        },
    );
    store
        .acquire(&grant(write.lease_releases.clone()))
        .await
        .unwrap();
    store.release_epoch(1, 1).await.unwrap();
    let fenced = store.checkpoint_economy(&write).await.unwrap_err();
    assert!(
        matches!(fenced, EconomyError::LeaseViolation(k) if k == write.lease_releases[0]),
        "{fenced:?}"
    );
    assert_eq!(store.row("item_instance", "900001").unwrap().revision, 0, "nothing written");
    assert!(store.ledger().is_empty());
}
