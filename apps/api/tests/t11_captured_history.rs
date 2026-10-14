//! T11 captured history (owned stack, zone 1 epoch 23, `nightfall-replay export --live`): a
//! `CreateCharacter` owner logs into a default-configuration V6 zone, receives online dev grants
//! (1060 x5, 1835 x200) and uses the potion; the item skill's effect lands the next tick and
//! extends the provider install order. The live checkpoint lane fenced the zone there and the
//! next boot refused the unresolved history. The recording resolves by hash with the default
//! composition, replays byte-exact, and checkpoint recovery now settles the whole history.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::wildcard_enum_match_arm
)]
use api::application::checkpoint::CheckpointService;
use api::application::ports::*;
use api::application::replay_log::{restore_snapshot, AppliedTickRecord, EventLog};
use api::domain::zone::{AppliedTickDraft, ZoneCommand};
use api::domain::{AccountId, Character, CharacterId, DomainEvent};
use api::infrastructure::eventlog::{InMemoryEventLog, Recording};
use api::infrastructure::memory::InMemorySessionAudit;
use api::infrastructure::telemetry::Metrics;
use nightfall_api as api;
use parking_lot::Mutex;
use std::sync::Arc;

const CAPTURE: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/sessions/t11-v6-zone1-epoch23.nfr");

fn capture() -> Recording {
    Recording::read(std::path::Path::new(CAPTURE)).expect("default-composition V6 bundle by hash")
}

#[test]
fn captured_t11_epoch_replays_byte_exact_with_the_default_composition() {
    let recording = capture();
    let bundle = api::infrastructure::item_data::embedded_phase4_bundle().unwrap();
    let items = recording.snapshot.phase4.as_ref().expect("V6 epoch");
    assert_eq!(items.rules_hash, bundle.items.hash());
    assert_eq!(recording.snapshot.phase3.as_ref().unwrap().rules_hash, bundle.skills.hash());
    assert_eq!(recording.records.len(), 673);
    let zone = recording.snapshot.seed.zone;
    let epoch = recording.snapshot.seed.epoch;
    let mut state = restore_snapshot(recording.snapshot.clone()).unwrap();
    let mut uses = 0;
    for record in &recording.records {
        let tick = state
            .run_tick(AppliedTickDraft {
                epoch,
                tick: record.tick,
                commands: record.commands.clone(),
            })
            .unwrap();
        assert!(record.reproduced_by(&AppliedTickRecord::from_applied(zone, &tick)));
        uses += record
            .commands
            .iter()
            .filter(|c| matches!(&c.command, ZoneCommand::Item(_)))
            .count();
    }
    assert!(uses >= 1, "the captured history carries the fatal UseItem");
}

/// Accepts every checkpoint at the next revision and records what was written.
#[derive(Default)]
struct Accepting {
    plain: Mutex<Vec<CharacterCheckpoint>>,
    items: Mutex<Vec<(CharacterCheckpoint, ItemCheckpointWork)>>,
}
#[async_trait::async_trait]
impl CharacterRepository for Accepting {
    async fn get(&self, _: CharacterId) -> anyhow::Result<Option<Character>> {
        Ok(None)
    }
    async fn list_by_account(&self, _: AccountId) -> anyhow::Result<Vec<Character>> {
        Ok(Vec::new())
    }
    async fn create_idempotent(
        &self,
        _: &IdempotencyKey,
        _: &str,
        _: &Character,
    ) -> Result<CreateOutcome, RepositoryError> {
        Err(RepositoryError::Other(anyhow::anyhow!("not used")))
    }
    async fn load_for_admission(&self, _: CharacterId) -> anyhow::Result<Option<ProgressionState>> {
        Ok(None)
    }
    async fn checkpoint(
        &self,
        cp: &CharacterCheckpoint,
        _: &[DomainEvent],
    ) -> Result<CheckpointOutcome, CheckpointError> {
        self.plain.lock().push(cp.clone());
        Ok(CheckpointOutcome::Applied(cp.revision_seen.saturating_add(1)))
    }
}
#[async_trait::async_trait]
impl ItemReceiptRepository for Accepting {
    async fn item_receipt_lookup(
        &self,
        _: AccountId,
        _: ItemMutationOperation,
        _: &IdempotencyKey,
        _: &str,
    ) -> anyhow::Result<ItemReceiptLookup> {
        Ok(ItemReceiptLookup::Unknown)
    }
}
#[async_trait::async_trait]
impl ItemCheckpointRepository for Accepting {
    async fn checkpoint_items(
        &self,
        cp: &CharacterCheckpoint,
        _: &[DomainEvent],
        work: &ItemCheckpointWork,
        _: Option<&SuccessfulLearnReceipt>,
    ) -> Result<CheckpointOutcome, CheckpointError> {
        self.items.lock().push((cp.clone(), work.clone()));
        Ok(CheckpointOutcome::Applied(cp.revision_seen.saturating_add(1)))
    }
}

#[tokio::test]
async fn checkpoint_recovery_settles_the_captured_t11_history() {
    let recording = capture();
    let zone = recording.snapshot.seed.zone;
    let epoch = recording.snapshot.seed.epoch;
    let log = InMemoryEventLog::default();
    log.write_snapshot(&recording.snapshot).await.unwrap();
    log.write_recovery_snapshot(&recording.snapshot)
        .await
        .unwrap();
    for record in &recording.records {
        log.append_applied(record).await.unwrap();
    }
    let repo = Arc::new(Accepting::default());
    let mut service = CheckpointService::new(
        repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    )
    .with_items(repo.clone());
    service
        .recover(&log, zone, epoch)
        .await
        .expect("the captured history no longer fences (no quarantine either)");
    let items = repo.items.lock();
    assert!(
        items.iter().any(|(_, work)| work
            .projection
            .receipts
            .iter()
            .any(|r| r.result.operation()
                == api::domain::zone::phase4_projection::ItemOperation::Use)),
        "the paid UseItem settles"
    );
    // The post-use effect tick rides an ordinary checkpoint with the extended install order.
    let fatal = items
        .iter()
        .rev()
        .find(|(_, work)| {
            work.projection.receipts.iter().any(|r| {
                r.result.operation() == api::domain::zone::phase4_projection::ItemOperation::Use
            })
        })
        .map(|(cp, _)| cp.clone())
        .unwrap();
    let plain = repo.plain.lock();
    assert!(plain.iter().any(|cp| {
        cp.character_id == fatal.character_id
            && cp.phase4.as_ref().is_some_and(|after| {
                let before = fatal.phase4.as_ref().unwrap();
                after.instances == before.instances
                    && after.equipment_install_order.len() > before.equipment_install_order.len()
            })
    }));
}

/// The adopted `nightfall-replay frames` subcommand (client lane, D26 evidence): on the
/// captured V6 epoch it verifies the replay, writes the owner's live-equivalent frames, marks
/// the tag-41 enriched spawn/stats frames, and refuses to overwrite its output.
#[test]
fn replay_frames_writes_the_sessions_verified_wire_frames() {
    use prost::Message as _;
    let recording = capture();
    let session = recording
        .records
        .iter()
        .flat_map(|record| &record.commands)
        .find_map(|command| match &command.command {
            ZoneCommand::SpawnPlayer { entity, .. } => Some(*entity),
            _ => None,
        })
        .unwrap();
    let out = std::env::temp_dir().join(format!("t11-frames-{}.jsonl", uuid::Uuid::now_v7()));
    let run = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_nightfall-replay"))
            .args(["frames", "--file", CAPTURE, "--session"])
            .arg(session.as_uuid().to_string())
            .arg("--out")
            .arg(&out)
            .output()
            .unwrap()
    };
    let first = run();
    assert_eq!(first.status.code(), Some(0), "{first:?}");
    let text = std::fs::read_to_string(&out).unwrap();
    let mut enriched = 0;
    let mut frames = 0;
    for line in text.lines() {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        let hex = value["frame_hex"].as_str().unwrap();
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        api::interface::grpc::pb::ServerMessage::decode(bytes.as_slice()).unwrap();
        frames += 1;
        if value["enriched"].as_bool().unwrap() {
            enriched += 1;
            assert!(matches!(value["kind"].as_str(), Some("spawn" | "stats")));
        }
    }
    assert!(frames > 0 && enriched > 0, "{frames} frames, {enriched} enriched");
    // A second run never overwrites evidence.
    assert_eq!(run().status.code(), Some(2));
    std::fs::remove_file(&out).unwrap();
}
