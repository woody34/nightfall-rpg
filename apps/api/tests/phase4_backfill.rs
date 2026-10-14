//! E3.3 / B5 backfill matrix (D114 gate 3) on owned PostgreSQL and `JetStream`.
//!
//! Existing characters: every admissible profession (base, first and second class; third
//! classes are not admissible characters, `ClassState` tier ≤ 2) × alive (V5-admitted, retained
//! skill checkpoint) / dead (V5-admitted, dead) / `never_admitted` (Phase 2-era row: class history and
//! learned metadata, never admitted to a V5 or V6 zone). Their rows predate Phase 4: the
//! creation-time starter request is removed and migration 000011 runs over them, twice.
//! First V6 admission (real WS login) materializes the base-class kit before the inventory
//! baseline; the first item checkpoint of every owner fails once, before the PostgreSQL commit
//! or after it with a lost acknowledgement, and resumes exactly once; reconnects, a second
//! epoch and a migration rerun add nothing; item ids stay inside disjoint reserved ranges.
//! With `PHASE4_BACKFILL_RESULTS=<path>` the per-case gates are written for the harness
//! (`phase4-regression.py --backfill-results`).
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::arithmetic_side_effects
)]
mod common;
use api::application::checkpoint::CheckpointService;
use api::application::ports::*;
use api::application::replay_log::{
    restore_snapshot, start_epoch, AppliedTickRecord, DurableTickGate, EventLog, GateConfig,
    NoReplayMetrics,
};
use api::application::use_cases::RegistryItemCatalogue;
use api::application::zone_actor::{IntervalTicks, ZoneActor};
use api::domain::character_progression::{
    auto_get_metadata, ClassState, FrozenTransferResult, SuccessfulTransferReceipt,
};
use api::domain::class::{ClassDef, ClassRegistry};
use api::domain::subclass::LearnedSkill;
use api::domain::zone::effects::RetainedModifierSource;
use api::domain::zone::skill::SkillRef;
use api::domain::zone::*;
use api::domain::{AccountId, Character, CharacterId, CharacterName, DomainEvent, Position};
use api::infrastructure::eventlog::JetStreamEventLog;
use api::infrastructure::memory::InMemorySessionAudit;
use api::infrastructure::postgres::{connection_from_pool, PgCharacterRepository};
use api::infrastructure::telemetry::Metrics;
use api::infrastructure::{class_data, item_data, rules_data, skill_data};
use api::interface::grpc::pb::{self, server_message, world_event};
use common::ws;
use nightfall_api as api;
use parking_lot::Mutex;
use sqlx::{Executor, PgPool};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const BACKFILL: &str = include_str!(
    "../src/infrastructure/postgres/migrations/m20261012_000011_pending_item_backfill.sql"
);
const STATES: [&str; 3] = ["alive", "dead", "never_admitted"];
const GATES: [&str; 5] = [
    "migration_rerun",
    "first_v6_admission",
    "crash_resume",
    "kit_once",
    "allocator_no_collision",
];

/// The first item checkpoint of every owner fails once: even owners before the commit, odd
/// owners after the real commit with the acknowledgement lost. The actor must resume both.
struct OnceFailingItems {
    inner: Arc<PgCharacterRepository>,
    failed: Mutex<BTreeMap<CharacterId, bool>>,
}
#[async_trait::async_trait]
impl ItemReceiptRepository for OnceFailingItems {
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
impl ItemCheckpointRepository for OnceFailingItems {
    async fn checkpoint_items(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
        work: &ItemCheckpointWork,
        learning: Option<&SuccessfulLearnReceipt>,
    ) -> Result<CheckpointOutcome, CheckpointError> {
        let before_commit = cp.character_id.as_uuid().as_u128().is_multiple_of(2);
        let first = match self.failed.lock().entry(cp.character_id) {
            std::collections::btree_map::Entry::Occupied(_) => false,
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(before_commit);
                true
            },
        };
        if first && before_commit {
            return Err(anyhow::anyhow!("injected failure before the PostgreSQL commit").into());
        }
        let outcome = self
            .inner
            .checkpoint_items(cp, events, work, learning)
            .await?;
        if first {
            return Err(anyhow::anyhow!("lost actual PostgreSQL commit acknowledgement").into());
        }
        Ok(outcome)
    }
}

struct Seeded {
    case: String,
    class_id: u32,
    state: &'static str,
    player: ws::Player,
    base: u32,
}

fn inventory_sync(m: &pb::ServerMessage) -> Option<&pb::InventorySync> {
    match &m.payload {
        Some(server_message::Payload::Event(pb::WorldEvent {
            event: Some(world_event::Event::InventorySync(sync)),
        })) => Some(sync),
        _ => None,
    }
}

/// The levelled V5-era retained skill checkpoint: every learned executable passive installed in
/// skill-id order (a V5 fresh admission's order), no effects or timers.
fn v5_checkpoint(rules: &Phase3Rules, learned: &[LearnedSkill]) -> Phase3Checkpoint {
    let mut checkpoint = Phase3Checkpoint::new();
    checkpoint.timer_policy_version = rules.timer_policy_version();
    for skill in rules.skills().skills().values().filter(|skill| {
        skill.operate_type == "P"
            && skill.runtime_status == api::domain::class::RuntimeSkillStatus::Executable
    }) {
        if let Some(rank) = learned.iter().find(|entry| entry.key == skill.key) {
            checkpoint
                .modifier_install_order
                .push(RetainedModifierSource::Passive {
                    skill: SkillRef {
                        id: skill.id,
                        rank: rank.level,
                    },
                });
        }
    }
    checkpoint
}

async fn seed(
    pool: &PgPool,
    legacy: &PgCharacterRepository,
    registry: &ClassRegistry,
    xp_rules: &api::domain::zone::StatRules,
    v5: &Phase3Rules,
    class: &ClassDef,
    state: &'static str,
    ordinal: u128,
) -> Seeded {
    let account = Uuid::from_u128(0xb5_0000_0000 + ordinal);
    common::pg::insert_account(pool, account).await;
    let base = registry.get(class.base_class_id).unwrap();
    let race = registry.race(class.race).unwrap();
    // Names are letters only: the ordinal in base 26.
    let mut name = String::from("Backfill");
    let mut rest = ordinal;
    for _ in 0..3 {
        name.push(char::from(b'a' + u8::try_from(rest % 26).unwrap()));
        rest /= 26;
    }
    // Created exactly as CreateCharacter creates it.
    let mut c = Character::create(
        AccountId::from_uuid(account),
        CharacterName::new(&name).unwrap(),
        class.race,
    );
    c.stats = base.base_stats;
    c.class_state = ClassState::new(base.id);
    c.class_state
        .merge_learned_skills_checked(
            registry,
            class.race,
            auto_get_metadata(registry, base.id, 1)
                .unwrap()
                .into_iter()
                .chain(race.passive_skill_keys.iter().map(|key| LearnedSkill {
                    key: key.clone(),
                    level: 1,
                })),
        )
        .unwrap();
    legacy
        .create_idempotent(&IdempotencyKey::new(), &name, &c)
        .await
        .unwrap();
    // Phase 2 history: one real transfer receipt per tier, levelled to the target's minimum.
    let level = class.min_level.max(if class.tier == 0 { 10 } else { 1 });
    let mut ledger = c.class_state.clone();
    let mut path = registry.ancestors(class.id);
    path.reverse();
    path.push(class.id);
    for target in path.into_iter().filter(|id| *id != base.id) {
        let definition = registry.get(target).unwrap();
        ledger
            .record_success(SuccessfulTransferReceipt {
                key: Uuid::now_v7(),
                target_class_id: target,
                result: FrozenTransferResult {
                    character_id: c.id,
                    identity: c.identity(),
                    name: c.name.clone(),
                    current_class_id: target,
                    level: definition.min_level,
                    xp: xp_rules.xp_table()[usize::try_from(definition.min_level - 1).unwrap()],
                    sp: 0,
                    stats: c.stats,
                    position_millitiles: [0, 0],
                    hp: 100,
                    mp: 50,
                    cp: 0,
                    max_hp: 100,
                    max_mp: 50,
                    max_cp: 0,
                    token_tier_1_count: 0,
                    token_tier_2_count: 0,
                    granted_skill_keys: Vec::new(),
                },
            })
            .unwrap();
        ledger.current_class_id = target;
    }
    ledger
        .merge_learned_skills_checked(
            registry,
            class.race,
            auto_get_metadata(registry, class.id, level).unwrap(),
        )
        .unwrap();
    let alive = state != "dead";
    let phase3 = (state != "never_admitted").then(|| v5_checkpoint(v5, &ledger.learned_skills));
    let checkpoint = CharacterCheckpoint {
        character_id: c.id,
        revision_seen: 0,
        level,
        xp: xp_rules.xp_table()[usize::try_from(level - 1).unwrap()],
        hp: if alive { 100 } else { 0 },
        mp: 50,
        alive,
        position: Position { x: 10.0, y: 10.0 },
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        class_state: Some(ledger),
        phase4: None,
        phase3,
    };
    let saved = legacy.checkpoint(&checkpoint, &[]).await;
    assert!(
        matches!(saved, Ok(CheckpointOutcome::Applied(1))),
        "{} {state}: {saved:?}",
        class.key
    );
    Seeded {
        case: format!("{}-{state}", class.key),
        class_id: class.id.0,
        state,
        player: ws::Player {
            account,
            character: c.id,
            name,
        },
        base: base.id.0,
    }
}

async fn zone_for_epoch(
    repo: &Arc<PgCharacterRepository>,
    writer: Arc<OnceFailingItems>,
    log: Arc<JetStreamEventLog>,
    bundle: &item_data::Phase4RuleBundle,
    classes: &class_data::ResolvedClasses,
    zone: ZoneId,
    epoch: u64,
) -> (common::TestApp, ZoneSnapshot) {
    let reservation = repo.reserve_item_range(zone, epoch).await.unwrap();
    let state = ZoneState::new_phase4(
        ZoneSeed { zone, epoch },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        rules_data::load_rules(&rules_data::RulesSource::embedded())
            .unwrap()
            .rules,
        classes.registry.clone(),
        (*bundle.skills).clone(),
        bundle.items.clone(),
        ItemInstanceRange::new(
            reservation.range_start,
            reservation.next_reserved,
            reservation.range_end_exclusive,
        )
        .unwrap(),
    )
    .unwrap();
    let baseline = state.snapshot();
    let started = start_epoch(log.as_ref(), &baseline).await.unwrap();
    let gate = DurableTickGate::new(
        &started,
        log,
        Arc::new(NoReplayMetrics),
        GateConfig::default(),
        CancellationToken::new(),
    );
    let handle = ZoneActor::spawn_gated(state, IntervalTicks::new(), gate);
    handle.checkpoints.install(
        CheckpointService::new(
            repo.clone(),
            Arc::new(InMemorySessionAudit::default()),
            Arc::new(Metrics::detached()),
        )
        .with_items(writer)
        .with_item_admission(repo.clone())
        .with_learning(repo.clone()),
    );
    let catalogue =
        Arc::new(RegistryItemCatalogue::new(bundle.items.clone(), &bundle.skills).unwrap());
    let ports = repo.clone();
    let classes = classes.clone();
    let app = common::TestApp::spawn_with_handle(
        move |deps| {
            deps.characters = ports.clone();
            deps.item_receipts = Some(ports.clone());
            deps.item_inventory = Some(ports.clone());
            deps.item_checkpoints = Some(ports.clone());
            deps.item_admission = Some(ports.clone());
            deps.learning = Some(ports);
            deps.item_catalogue = Some(catalogue);
            deps.classes = Some(classes);
        },
        handle,
    )
    .await;
    (app, baseline)
}

/// Replays every recorded tick of the epoch byte-exact from its baseline and returns the
/// recorded V6 admission loads (owner -> `PlayerLoad`).
async fn replay_epoch(
    log: &JetStreamEventLog,
    zone: ZoneId,
    epoch: u64,
    baseline: ZoneSnapshot,
) -> BTreeMap<EntityId, PlayerLoad> {
    use futures_util::StreamExt as _;
    let mut replay = restore_snapshot(baseline).unwrap();
    let mut records = log.read_epoch(zone, epoch).await.unwrap();
    let mut loads = BTreeMap::new();
    while let Some(record) = records.next().await {
        let record = record.unwrap();
        assert_eq!(AppliedTickRecord::decode(&record.encode()).unwrap(), record);
        let tick = replay
            .run_tick(AppliedTickDraft {
                epoch,
                tick: record.tick,
                commands: record.commands.clone(),
            })
            .unwrap();
        assert!(
            record.reproduced_by(&AppliedTickRecord::from_applied(zone, &tick)),
            "epoch {epoch} tick {:?} does not replay byte-exact",
            record.tick
        );
        for command in &record.commands {
            if let ZoneCommand::SpawnPlayer {
                entity,
                load: Some(load),
                ..
            } = &command.command
            {
                loads.entry(*entity).or_insert_with(|| (**load).clone());
            }
        }
    }
    loads
}

/// Log in, take the inventory baseline, log out. `Err` says how the login ended without it.
async fn login(app: &common::TestApp, player: &ws::Player) -> Result<pb::InventorySync, String> {
    // The per-address WS handshake limiter (429) applies to this single test client: back off.
    let mut attempt = 0;
    let mut socket = loop {
        let ticket = ws::ticket(app, player).await;
        match ws::connect(app, &ticket).await {
            Ok(socket) => break socket,
            Err(429) if attempt < 40 => {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            },
            Err(code) => return Err(format!("handshake refused {code}")),
        }
    };
    let mut outcome = Err("no message".to_owned());
    for _ in 0..400 {
        // Generous: a first admission waits for the injected checkpoint failure's retry.
        match socket.recv(std::time::Duration::from_mins(1)).await {
            Some(ws::Received::Message(m, _)) => {
                if let Some(found) = inventory_sync(&m).filter(|s| s.baseline) {
                    outcome = Ok(found.clone());
                    break;
                }
                if let Some(rejected) = ws::rejected(&m) {
                    outcome = Err(format!("rejected {rejected:?}"));
                }
            },
            Some(ws::Received::Closed(code)) => {
                outcome = Err(format!("closed {code:?}"));
                break;
            },
            None => {
                outcome = Err("timed out".into());
                break;
            },
        }
    }
    socket.close().await;
    // The session's detach must settle before the next admission of the same owner.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    outcome
}

async fn scalar(pool: &PgPool, sql: &str, id: CharacterId) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .bind(id.as_uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn existing_characters_receive_their_base_kit_once_at_first_v6_admission() {
    let pool = common::pg::migrated_pool()
        .await
        .expect("owned DATABASE_URL required");
    let nats = std::env::var("NATS_URL").expect("owned NATS_URL required");
    let classes = class_data::load_classes(&class_data::ClassSource::embedded()).unwrap();
    let registry = classes.registry.clone();
    let xp_rules = rules_data::load_rules(&rules_data::RulesSource::embedded())
        .unwrap()
        .rules;
    let v5 = Arc::new(
        skill_data::load_phase3_rules(&skill_data::Phase3DataSource::embedded(), &[]).unwrap(),
    );
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let legacy = PgCharacterRepository::new(connection_from_pool(&pool))
        .with_classes(registry.clone())
        .with_phase3_rules(v5.clone());
    let repo = Arc::new(
        PgCharacterRepository::new(connection_from_pool(&pool))
            .with_classes(registry.clone())
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    // Seed every admissible profession × state as a pre-Phase 4 character.
    let mut professions: Vec<&ClassDef> = registry.classes().iter().collect();
    professions.sort_by(|a, b| a.key.cmp(&b.key));
    let mut seeded = Vec::new();
    let mut unsupported = Vec::new();
    let mut ordinal = 0_u128;
    for class in &professions {
        for state in STATES {
            ordinal += 1;
            if class.tier > 2 {
                unsupported.push((format!("{}-{state}", class.key), class.id.0, state));
                continue;
            }
            seeded
                .push(seed(&pool, &legacy, &registry, &xp_rules, &v5, class, state, ordinal).await);
        }
    }
    // Pre-Phase 4 rows carry no kit request; migration 000011 runs over them, then reruns.
    pool.execute("DELETE FROM pending_item_requests WHERE kind='starter'")
        .await
        .unwrap();
    pool.execute(sqlx::raw_sql(BACKFILL)).await.unwrap();
    let manifests = |pool: PgPool| async move {
        sqlx::query_as::<_, (Uuid, String, String, String, serde_json::Value)>(
            "SELECT character_id,kind,status,manifest_hash,payload FROM pending_item_requests ORDER BY character_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap()
    };
    let first = manifests(pool.clone()).await;
    pool.execute(sqlx::raw_sql(BACKFILL)).await.unwrap();
    let rerun = manifests(pool.clone()).await;
    let migration_rerun = first == rerun
        && first.len() == seeded.len()
        && first
            .iter()
            .all(|(_, kind, status, _, _)| kind == "backfill" && status == "pending");
    assert!(migration_rerun, "000011 inserts one pending backfill per character, rerun-safe");
    let kit_of: BTreeMap<Uuid, serde_json::Value> = first
        .iter()
        .map(|(id, _, _, _, payload)| (*id, payload.clone()))
        .collect();

    // Every existing owner must load for V6 admission before any login is attempted.
    let mut problems: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for owner in &seeded {
        let id = owner.player.character;
        if let Err(error) = repo.get(id).await {
            problems
                .entry(owner.case.clone())
                .or_default()
                .push(format!("character read: {error:#}"));
        }
        if let Err(error) = repo.load_for_admission(id).await {
            problems
                .entry(owner.case.clone())
                .or_default()
                .push(format!("V6 admission load: {error:#}"));
        }
    }
    assert!(problems.is_empty(), "existing owners do not load on V6: {problems:#?}");
    // Two epochs of one zone: the first admits half the owners, the second the rest and
    // reconnects everyone.
    let log = Arc::new(
        JetStreamEventLog::connect(async_nats::connect(&nats).await.unwrap(), Metrics::detached())
            .await
            .unwrap(),
    );
    let writer = Arc::new(OnceFailingItems {
        inner: repo.clone(),
        failed: Mutex::new(BTreeMap::new()),
    });
    let zone = ZoneId(u32::try_from(Uuid::now_v7().as_u128() & 0x7fff_ffff).unwrap());
    let half = seeded.len() / 2;
    let mut first_sync = BTreeMap::new();
    let mut baselines = Vec::new();
    for (epoch, owners) in [(1_u64, &seeded[..half]), (2, &seeded[half..])] {
        let (app, baseline) =
            zone_for_epoch(&repo, writer.clone(), log.clone(), &bundle, &classes, zone, epoch)
                .await;
        for chunk in owners.chunks(8) {
            let results = futures_util::future::join_all(
                chunk.iter().map(|owner| login(&app, &owner.player)),
            )
            .await;
            for (owner, sync) in chunk.iter().zip(results) {
                match sync {
                    Ok(sync) => {
                        first_sync.insert(owner.player.character, sync);
                    },
                    Err(why) => problems
                        .entry(owner.case.clone())
                        .or_default()
                        .push(format!("first V6 admission produced no inventory baseline: {why}")),
                }
            }
        }
        baselines.push((epoch, baseline));
        if epoch == 2 {
            // Reconnect every owner, including the first epoch's, in the second epoch.
            for chunk in seeded.chunks(8) {
                let results = futures_util::future::join_all(
                    chunk.iter().map(|owner| login(&app, &owner.player)),
                )
                .await;
                for (owner, again) in chunk.iter().zip(results) {
                    let before = first_sync.get(&owner.player.character);
                    let same = again
                        .as_ref()
                        .ok()
                        .zip(before)
                        .is_some_and(|(again, before)| {
                            again.inventory_revision == before.inventory_revision
                                && again.items == before.items
                                && again.paperdoll == before.paperdoll
                        });
                    if !same {
                        let detail = match (&again, before) {
                            (Err(why), _) => format!("reconnect produced no inventory baseline: {why}"),
                            (Ok(again), Some(before)) => format!(
                                "reconnect changed the inventory: revision {} -> {}, items {} -> {}, paperdoll {} -> {}",
                                before.inventory_revision,
                                again.inventory_revision,
                                before.items.len(),
                                again.items.len(),
                                before.paperdoll.len(),
                                again.paperdoll.len()
                            ),
                            (Ok(_), None) => "reconnect without a first baseline".to_owned(),
                        };
                        problems.entry(owner.case.clone()).or_default().push(detail);
                    }
                }
            }
        }
    }
    // Both recorded epochs replay byte-exact. Every V6 admission load carries the pending
    // backfill kit; a V5-admitted owner's skill checkpoint carries the D116 upgrade (mystics:
    // the V6-promoted passives 118/163/214 they learned), the never-admitted none.
    let mut recorded = BTreeMap::new();
    for (epoch, baseline) in baselines {
        for (entity, load) in replay_epoch(&log, zone, epoch, baseline).await {
            recorded.entry(entity).or_insert(load);
        }
    }
    let promoted = [118_u32, 163, 214];
    let mut upgraded_mystics = 0_usize;
    for owner in &seeded {
        let entity = EntityId::from_uuid(owner.player.character.as_uuid());
        let Some(load) = recorded.get(&entity) else {
            problems
                .entry(owner.case.clone())
                .or_default()
                .push("no recorded V6 SpawnPlayer".into());
            continue;
        };
        let pending = load
            .phase4
            .as_ref()
            .is_some_and(|items| items.checkpoint.is_none() && items.pending.len() == 1);
        let learned = repo
            .get(owner.player.character)
            .await
            .unwrap()
            .unwrap()
            .class_state
            .learned_skills;
        // Complete under V6: exactly the learned passives V6 executes.
        let executable: BTreeSet<u32> = bundle
            .skills
            .skills()
            .skills()
            .values()
            .filter(|skill| {
                skill.operate_type == "P"
                    && skill.runtime_status == api::domain::class::RuntimeSkillStatus::Executable
                    && learned.iter().any(|entry| entry.key == skill.key)
            })
            .map(|skill| skill.id.0)
            .collect();
        let skills_ok = match (owner.state, &load.phase3) {
            ("never_admitted", None) => true,
            ("alive" | "dead", Some(phase3)) => {
                let installed: BTreeSet<u32> = phase3
                    .modifier_install_order
                    .iter()
                    .filter_map(|source| match source {
                        RetainedModifierSource::Passive { skill } => Some(skill.id.0),
                        RetainedModifierSource::Effect { .. }
                        | RetainedModifierSource::DeathPenalty => None,
                    })
                    .collect();
                if promoted.iter().any(|id| installed.contains(id)) {
                    upgraded_mystics += 1;
                }
                installed == executable
            },
            _ => false,
        };
        if !(pending && skills_ok) {
            problems
                .entry(owner.case.clone())
                .or_default()
                .push("recorded V6 admission load is not the canonical backfill load".into());
        }
    }
    // V5 never executed 118/163/214, so each owner holding them was upgraded by D116.
    assert!(upgraded_mystics > 0, "no V5-admitted owner exercised the D116 upgrade");
    // A migration rerun after materialization changes nothing.
    pool.execute(sqlx::raw_sql(BACKFILL)).await.unwrap();
    let after = manifests(pool.clone()).await;
    let ranges: Vec<(i64, i64, i64)> = sqlx::query_as(
        "SELECT epoch,range_start,range_end_exclusive FROM item_id_ranges WHERE zone_id=$1 ORDER BY epoch",
    )
    .bind(i32::try_from(zone.0).unwrap())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(ranges.len(), 2, "one reserved range per epoch");
    let disjoint = ranges[0].2 <= ranges[1].1 || ranges[1].2 <= ranges[0].1;
    let failed = writer.failed.lock().clone();
    let mut cases = Vec::new();
    for owner in &seeded {
        let id = owner.player.character;
        let mut gates = BTreeMap::new();
        gates.insert("migration_rerun", migration_rerun);
        let sync = first_sync.get(&id);
        let kit = &kit_of[&id.as_uuid()];
        // The baseline holds exactly the base-class kit: per-template counts and equipped rows.
        let expected: BTreeMap<u32, u64> =
            kit["items"]
                .as_array()
                .unwrap()
                .iter()
                .fold(BTreeMap::new(), |mut totals, item| {
                    *totals
                        .entry(u32::try_from(item["template_id"].as_u64().unwrap()).unwrap())
                        .or_default() += item["count"].as_u64().unwrap();
                    totals
                });
        let equipped: BTreeSet<u32> = kit["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["equipped"].as_bool().unwrap())
            .map(|item| u32::try_from(item["template_id"].as_u64().unwrap()).unwrap())
            .collect();
        let admitted = sync.is_some_and(|sync| {
            let held = sync.items.iter().fold(BTreeMap::new(), |mut totals, item| {
                *totals.entry(item.template_id).or_insert(0_u64) += item.count;
                totals
            });
            let worn: BTreeSet<u32> = sync
                .paperdoll
                .iter()
                .filter_map(|entry| {
                    sync.items
                        .iter()
                        .find(|item| item.instance_id == entry.instance_id)
                        .map(|item| item.template_id)
                })
                .collect();
            held == expected && worn == equipped && kit["source_class_id"] == owner.base
        });
        gates.insert("first_v6_admission", admitted);
        let instances =
            scalar(&pool, "SELECT count(*) FROM item_instances WHERE character_id=$1", id).await;
        let birth = scalar(
            &pool,
            "SELECT count(*) FROM item_ledger WHERE character_id=$1 AND op='backfill'",
            id,
        )
        .await;
        let other = scalar(
            &pool,
            "SELECT count(*) FROM item_ledger WHERE character_id=$1 AND op<>'backfill'",
            id,
        )
        .await;
        let receipts = scalar(
            &pool,
            "SELECT count(*) FROM pending_item_requests WHERE character_id=$1 AND status='materialized' AND kind='backfill'",
            id,
        )
        .await;
        let still_one = after
            .iter()
            .filter(|(character, ..)| *character == id.as_uuid())
            .count()
            == 1;
        let reconnect_ok = !problems
            .get(&owner.case)
            .is_some_and(|p| p.iter().any(|p| p.starts_with("reconnect")));
        let kit_once = admitted
            && instances > 0
            && birth == instances
            && other == 0
            && receipts == 1
            && still_one
            && reconnect_ok;
        gates.insert("kit_once", kit_once);
        // The injected first-checkpoint failure happened for this owner and resolved once.
        gates.insert("crash_resume", failed.contains_key(&id) && kit_once);
        let ids: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM item_instances WHERE character_id=$1")
                .bind(id.as_uuid())
                .fetch_all(&pool)
                .await
                .unwrap();
        let in_range = !ids.is_empty()
            && ids.iter().all(|item| {
                ranges
                    .iter()
                    .any(|(_, start, end)| (start..end).contains(&item))
            });
        gates.insert("allocator_no_collision", disjoint && in_range);
        for (gate, ok) in &gates {
            if !ok {
                problems
                    .entry(owner.case.clone())
                    .or_default()
                    .push(format!("gate {gate} failed"));
            }
        }
        cases.push(serde_json::json!({
            "id": owner.case,
            "class_id": owner.class_id,
            "state": owner.state,
            "gates": gates,
            "before_commit_failure": failed.get(&id).copied(),
        }));
    }
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM item_instances")
        .fetch_one(&pool)
        .await
        .unwrap();
    let distinct: i64 = sqlx::query_scalar("SELECT count(DISTINCT id) FROM item_instances")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, distinct);
    for (case, class_id, state) in &unsupported {
        cases.push(serde_json::json!({
            "id": case,
            "class_id": class_id,
            "state": state,
            "gates": GATES.iter().map(|gate| (*gate, false)).collect::<BTreeMap<_, _>>(),
            "note": "third-class profession: not an admissible character (ClassState tier <= 2)",
        }));
    }
    if let Ok(path) = std::env::var("PHASE4_BACKFILL_RESULTS") {
        let commit = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .map_or_else(|| "unknown".into(), |sha| sha.trim().to_owned());
        let results = serde_json::json!({
            "schema_version": 1,
            "source": "apps/api/tests/phase4_backfill.rs",
            "commit": commit,
            "cases": cases,
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    assert!(problems.is_empty(), "backfill matrix failures: {problems:#?}");
    assert_eq!(seeded.len() + unsupported.len(), professions.len() * 3);
}
