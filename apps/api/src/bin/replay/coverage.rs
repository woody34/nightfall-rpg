//! Recording coverage: durable facts and the server-observable attack request lifecycle.
#[path = "coverage_phase3.rs"]
mod phase3;

use std::collections::BTreeMap;

use nightfall_api::application::replay_log::{decode_events, AppliedTickRecord, OutputForm};
use nightfall_api::domain::zone::{AppliedTickDraft, EntityId, EntityKind, ZoneCommand, ZoneEvent};
use nightfall_api::infrastructure::eventlog::Recording;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct Transition {
    from: String,
    to: String,
    reachable: bool,
    count: u64,
}

#[derive(Debug, Serialize)]
pub(super) struct Coverage {
    schema_version: u32,
    npc_intentions: Vec<Transition>,
    player_attack_states: Vec<Transition>,
    life_incarnations: BTreeMap<String, u64>,
    deaths: BTreeMap<String, u64>,
    respawns: BTreeMap<String, u64>,
    intent_rejected: BTreeMap<String, u64>,
    // Actor commands are not ClientMessage intents; a non-rejected retry need not create an effect.
    class_transfer_commands: BTreeMap<String, u64>,
    class_changes: BTreeMap<String, u64>,
    class_transfer_effects: BTreeMap<String, u64>,
    owner_stats_updates: u64,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    phase3: Option<phase3::Stats>,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    items: Option<ItemStats>,
}

/// T48: schema-four V6 item coverage (sim-gates.py `V4_COUNTERS`/`V4_SCALARS`/`V4_LABELS`),
/// from the verified final per-owner `ItemCheckpoint` of every tick.
#[derive(Debug, Default, Serialize)]
pub(super) struct ItemStats {
    /// One per ledger row by its operation, plus one per `ToggleAutoShot` receipt.
    item_operations: BTreeMap<String, u64>,
    enchant_outcomes: BTreeMap<String, u64>,
    refine_outcomes: BTreeMap<String, u64>,
    /// One per charge bit a Use row adds to a weapon (soul 1, spirit 2, blessed spirit 4).
    shot_payments: BTreeMap<String, u64>,
    item_ledger_rows: u64,
    /// Ledger rows that create an instance (no `before`).
    item_allocations: u64,
    /// Retained/initial inventories at the boundary plus every V6 player admission.
    inventory_baselines: u64,
}

fn snake(value: &impl Serialize) -> anyhow::Result<String> {
    let value = serde_json::to_value(value)?;
    if let serde_json::Value::String(name) = value {
        Ok(name)
    } else {
        anyhow::bail!("non-symbolic coverage label {value}")
    }
}

fn item_facts(items: &mut ItemStats, events: &[ZoneEvent]) -> anyhow::Result<()> {
    use nightfall_api::domain::zone::phase4_projection::{ItemOperation, ItemResultDetail};
    for event in events {
        if let ZoneEvent::EntitySpawn {
            kind: EntityKind::Player,
            ..
        } = event
        {
            items.inventory_baselines = items.inventory_baselines.saturating_add(1);
        }
        if let ZoneEvent::ItemCheckpoint(projection) = event {
            {
                for row in &projection.ledger {
                    items.item_ledger_rows = items.item_ledger_rows.saturating_add(1);
                    if row.before.is_none() && row.after.is_some() {
                        items.item_allocations = items.item_allocations.saturating_add(1);
                    }
                    increment(&mut items.item_operations, snake(&row.operation)?);
                    if row.operation == ItemOperation::Use {
                        if let (Some(before), Some(after)) = (&row.before, &row.after) {
                            let added = after.charges & !before.charges;
                            for (bit, label) in
                                [(1_u8, "soul"), (2, "spirit"), (4, "blessed_spirit")]
                            {
                                if added & bit != 0 {
                                    increment(&mut items.shot_payments, label.into());
                                }
                            }
                        }
                    }
                }
                for receipt in &projection.receipts {
                    if let ItemResultDetail::Operation {
                        operation: ItemOperation::ToggleAutoShot,
                        ..
                    } = receipt.result.detail
                    {
                        increment(&mut items.item_operations, "toggle_auto_shot".into());
                    }
                }
                for attempt in &projection.enchant_attempts {
                    increment(&mut items.enchant_outcomes, snake(&attempt.outcome)?);
                }
                for refine in &projection.refines {
                    increment(&mut items.refine_outcomes, snake(&refine.outcome)?);
                }
            }
        }
    }
    Ok(())
}

fn table(states: &[&str], reachable: &[(&str, &str)]) -> Vec<Transition> {
    states
        .iter()
        .flat_map(|from| {
            states.iter().map(move |to| Transition {
                from: (*from).to_owned(),
                to: (*to).to_owned(),
                reachable: reachable.contains(&(*from, *to)),
                count: 0,
            })
        })
        .collect()
}

fn increment(counts: &mut BTreeMap<String, u64>, key: String) {
    let count = counts.entry(key).or_default();
    *count = count.saturating_add(1);
}

fn transition(rows: &mut [Transition], from: &str, to: &str) {
    if from == to {
        return;
    }
    if let Some(row) = rows.iter_mut().find(|row| row.from == from && row.to == to) {
        row.count = row.count.saturating_add(1);
    }
}

impl Coverage {
    fn new() -> Self {
        Self {
            schema_version: 2,
            npc_intentions: table(
                &["Idle", "Active", "Attack", "ReturnHome", "Dead"],
                &[
                    ("Idle", "Active"),
                    ("Active", "Idle"),
                    ("Idle", "Attack"),
                    ("Active", "Attack"),
                    ("Attack", "ReturnHome"),
                    ("ReturnHome", "Active"),
                    ("Idle", "Dead"),
                    ("Active", "Dead"),
                    ("Attack", "Dead"),
                    ("Dead", "Idle"),
                ],
            ),
            player_attack_states: table(
                &["idle", "pending", "active"],
                &[
                    ("idle", "pending"),
                    ("pending", "active"),
                    ("pending", "idle"),
                    ("active", "idle"),
                ],
            ),
            life_incarnations: BTreeMap::new(),
            deaths: BTreeMap::from([("player".into(), 0), ("npc".into(), 0)]),
            respawns: BTreeMap::from([("player".into(), 0), ("npc".into(), 0)]),
            intent_rejected: BTreeMap::new(),
            class_transfer_commands: BTreeMap::new(),
            class_changes: BTreeMap::new(),
            class_transfer_effects: BTreeMap::new(),
            owner_stats_updates: 0,
            phase3: None,
            items: None,
        }
    }

    pub(super) fn print(&self) {
        let mut tables = vec![
            ("NPC intentions", &self.npc_intentions),
            ("Player attack states", &self.player_attack_states),
        ];
        if let Some(phase) = &self.phase3 {
            tables.extend([
                ("Cast states", &phase.cast_states),
                ("Effect states", &phase.effect_states),
            ]);
        }
        for (name, rows) in tables {
            let covered = rows.iter().filter(|r| r.reachable && r.count > 0).count();
            let total = rows.iter().filter(|r| r.reachable).count();
            println!("{name}: {covered}/{total} reachable pairs covered\nfrom -> to                 count  status");
            for row in rows {
                let status = if row.count > 0 {
                    "observed"
                } else if row.reachable {
                    "never seen"
                } else {
                    "unreachable"
                };
                println!("{} -> {}\t{}\t{status}", row.from, row.to, row.count);
            }
        }
        let mut counters = vec![
            ("Life incarnations", &self.life_incarnations),
            ("Deaths", &self.deaths),
            ("Respawns", &self.respawns),
            ("IntentRejected", &self.intent_rejected),
            (
                "Class transfer actor commands (not WebSocket intents)",
                &self.class_transfer_commands,
            ),
            ("Public class changes by destination", &self.class_changes),
            ("Recorded class transfer effects", &self.class_transfer_effects),
        ];
        if let Some(phase) = &self.phase3 {
            counters.extend([
                ("Cast endings", &phase.cast_end_reasons),
                ("Effect removals", &phase.effect_remove_reasons),
                ("Learning actor commands", &phase.learning_commands),
                ("CP absorption", &phase.cp_absorption),
            ]);
            println!("Committed learn effects\t{}", phase.learning_committed);
        }
        for (name, counts) in counters {
            println!("{name}");
            for (key, count) in counts {
                println!("{key}\t{count}");
            }
        }
        println!("Owner stats updates\t{}", self.owner_stats_updates);
        if let Some(items) = &self.items {
            for (name, counts) in [
                ("Item operations", &items.item_operations),
                ("Enchant outcomes", &items.enchant_outcomes),
                ("Refine outcomes", &items.refine_outcomes),
                ("Shot payments", &items.shot_payments),
            ] {
                println!("{name}");
                for (key, count) in counts {
                    println!("{key}\t{count}");
                }
            }
            println!(
                "Item ledger rows\t{}\nItem allocations\t{}\nInventory baselines\t{}",
                items.item_ledger_rows, items.item_allocations, items.inventory_baselines
            );
        }
    }
}

fn kind(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Player => "player",
        EntityKind::Npc => "npc",
    }
}

/// Pending means an attack request awaiting its disposition, active means auto-attack
/// enabled (including chase/cooldown), not an individual swing. Repeats are not transitions.
pub(super) fn collect(rec: &Recording) -> anyhow::Result<Coverage> {
    let mut report = Coverage::new();
    if rec.snapshot.phase3.is_some() {
        report.schema_version = 3;
        report.phase3 = Some(phase3::Stats::new());
    }
    // T48: V6 recordings (with the item lane) carry schema four; V5 stays schema three.
    if let Some(items) = rec.snapshot.phase4.as_ref() {
        report.schema_version = 4;
        report.items = Some(ItemStats {
            inventory_baselines: u64::try_from(items.players.len())?,
            ..ItemStats::default()
        });
    }
    let mut phase3 = phase3::Tracker::baseline(&rec.snapshot);
    let mut zone = nightfall_api::application::replay_log::restore_snapshot(rec.snapshot.clone())?;
    let mut kinds = BTreeMap::new();
    let mut lives = BTreeMap::new();
    let mut targets = BTreeMap::new();
    for member in &rec.snapshot.spawn_members {
        if let Some(entity) = member.entity {
            kinds.insert(entity, EntityKind::Npc);
            lives.insert(entity, member.incarnation);
        }
    }
    let mut attacks = BTreeMap::<EntityId, &'static str>::new();
    for e in &rec.snapshot.entities {
        kinds.insert(e.id, e.kind);
        targets.insert(e.id, e.targeting.target);
        if let Some(c) = &e.combat {
            lives.insert(e.id, c.incarnation);
        }
        if e.kind == EntityKind::Player {
            attacks.insert(
                e.id,
                if e.combat.as_ref().is_some_and(|c| c.auto_attack) {
                    "active"
                } else {
                    "idle"
                },
            );
        }
    }
    for r in &rec.records {
        let applied = zone.run_tick(AppliedTickDraft {
            epoch: r.epoch,
            tick: r.tick,
            commands: r.commands.clone(),
        })?;
        if report.phase3.is_some()
            && !r.reproduced_by(&AppliedTickRecord::from_applied(r.zone, &applied))
        {
            anyhow::bail!(
                "Phase3 coverage requires verified replay: divergence at tick {}",
                r.tick.0
            );
        }
        for d in &r.dispositions {
            let transfer = r.commands.iter().any(|c| {
                c.ordinal == d.ordinal && matches!(c.command, ZoneCommand::ChangeClass { .. })
            });
            if !transfer {
                increment(&mut report.intent_rejected, format!("{:?}", d.reason));
            }
        }
        commands(&mut report, &mut attacks, &mut kinds, &mut targets, r);
        if let Some(items) = report.items.as_mut() {
            item_facts(items, &applied.events)?;
        }
        let events = match r.output_form {
            OutputForm::Encoded => decode_events(&r.events)?,
            OutputForm::Sha256 => applied.events,
        };
        phase3.facts(&mut report, &events);
        facts(&mut report, &mut lives, &mut kinds, events);
        // Admissions can restore retained containers without EffectApplied zone facts.
        // Seed their identities without counting an installation, just like the initial
        // snapshot; later real removals are still counted once.
        let snapshot = zone.snapshot();
        phase3 = phase3::Tracker::baseline(&snapshot);
        for e in &snapshot.entities {
            targets.insert(e.id, e.targeting.target);
            if e.kind == EntityKind::Player {
                let to = if e.combat.as_ref().is_some_and(|c| c.auto_attack) {
                    "active"
                } else {
                    "idle"
                };
                let from = attacks.entry(e.id).or_insert("idle");
                transition(&mut report.player_attack_states, from, to);
                *from = to;
            }
        }
    }
    Ok(report)
}

fn note_life(
    report: &mut Coverage,
    lives: &mut BTreeMap<EntityId, u32>,
    entity: EntityId,
    incarnation: u32,
    k: EntityKind,
) {
    if let Some(before) = lives.insert(entity, incarnation) {
        if before != incarnation {
            increment(
                &mut report.life_incarnations,
                format!("{}:{before}->{incarnation}", kind(k)),
            );
        }
    }
}

fn commands(
    report: &mut Coverage,
    attacks: &mut BTreeMap<EntityId, &'static str>,
    kinds: &mut BTreeMap<EntityId, EntityKind>,
    targets: &mut BTreeMap<EntityId, Option<EntityId>>,
    r: &AppliedTickRecord,
) {
    for command in &r.commands {
        let rejected = r.dispositions.iter().any(|d| d.ordinal == command.ordinal);
        if !rejected {
            if let ZoneCommand::SpawnPlayer { entity, .. } = command.command {
                kinds.insert(entity, EntityKind::Player);
                attacks.entry(entity).or_insert("idle");
            }
        }
        if let ZoneCommand::SetTarget { entity, target } = command.command {
            if !rejected && targets.insert(entity, target).flatten() != target {
                if let Some(from) = attacks.get_mut(&entity) {
                    transition(&mut report.player_attack_states, from, "idle");
                    *from = "idle";
                }
            }
        }
        if let ZoneCommand::Despawn { entity } = command.command {
            if !rejected {
                for (owner, target) in targets.iter_mut() {
                    if *target == Some(entity) {
                        *target = None;
                        if let Some(from) = attacks.get_mut(owner) {
                            transition(&mut report.player_attack_states, from, "idle");
                            *from = "idle";
                        }
                    }
                }
            }
        }
        if let ZoneCommand::LearnSkill { .. } = command.command {
            let outcome = r
                .dispositions
                .iter()
                .find(|d| d.ordinal == command.ordinal)
                .map_or_else(
                    || "applied_without_rejection".to_owned(),
                    |d| format!("rejected:{:?}", d.reason),
                );
            if let Some(phase) = &mut report.phase3 {
                increment(&mut phase.learning_commands, outcome);
            }
        }
        if let ZoneCommand::ChangeClass { .. } = command.command {
            let outcome = r
                .dispositions
                .iter()
                .find(|d| d.ordinal == command.ordinal)
                .map_or_else(
                    || "applied_without_rejection".to_owned(),
                    |d| format!("rejected:{:?}", d.reason),
                );
            increment(&mut report.class_transfer_commands, outcome);
        }
        match command.command {
            ZoneCommand::Attack { entity } if attacks.get(&entity) == Some(&"idle") => {
                transition(&mut report.player_attack_states, "idle", "pending");
                let to = if rejected { "idle" } else { "active" };
                transition(&mut report.player_attack_states, "pending", to);
                attacks.insert(entity, to);
            },
            ZoneCommand::StopAttack { entity }
            | ZoneCommand::StopMove { entity }
            | ZoneCommand::MoveTo { entity, .. }
            | ZoneCommand::Despawn { entity }
                if !rejected =>
            {
                if let Some(from) = attacks.get_mut(&entity) {
                    transition(&mut report.player_attack_states, from, "idle");
                    *from = "idle";
                }
            },
            ZoneCommand::ChangeClass { .. }
            | ZoneCommand::CastSkill { .. }
            | ZoneCommand::CancelCast { .. }
            | ZoneCommand::AcceptResurrect { .. }
            | ZoneCommand::LearnSkill { .. }
            | ZoneCommand::MaterializePendingItems(_)
            | ZoneCommand::ExtendItemRange { .. }
            | ZoneCommand::Item(_)
            | ZoneCommand::SpawnPlayer { .. }
            | ZoneCommand::SpawnNpc { .. }
            | ZoneCommand::Despawn { .. }
            | ZoneCommand::ReplaceSession { .. }
            | ZoneCommand::MoveTo { .. }
            | ZoneCommand::SetTarget { .. }
            | ZoneCommand::Attack { .. }
            | ZoneCommand::StopAttack { .. }
            | ZoneCommand::Respawn { .. }
            | ZoneCommand::StopMove { .. }
            | ZoneCommand::AddAggro { .. } => {},
        }
    }
}

fn facts(
    report: &mut Coverage,
    lives: &mut BTreeMap<EntityId, u32>,
    kinds: &mut BTreeMap<EntityId, EntityKind>,
    events: Vec<ZoneEvent>,
) {
    for event in events {
        match event {
            ZoneEvent::ClassChanged { class_id, .. } => {
                increment(&mut report.class_changes, class_id.0.to_string());
            },
            ZoneEvent::ClassTransfer {
                old_class_id,
                receipt,
                ..
            } => {
                increment(
                    &mut report.class_transfer_effects,
                    format!("{}->{}", old_class_id.0, receipt.target_class_id.0),
                );
            },
            ZoneEvent::StatsChanged { .. } => {
                report.owner_stats_updates = report.owner_stats_updates.saturating_add(1);
            },
            ZoneEvent::NpcIntentionChanged { from, to, .. } => {
                transition(&mut report.npc_intentions, &format!("{from:?}"), &format!("{to:?}"));
            },
            ZoneEvent::EntitySpawn {
                entity,
                kind: k,
                combat,
                ..
            } => {
                kinds.insert(entity, k);
                if let Some(c) = combat {
                    note_life(report, lives, entity, c.incarnation, k);
                }
            },
            ZoneEvent::EntityDied { entity, .. } => {
                if let Some(k) = kinds.get(&entity) {
                    increment(&mut report.deaths, kind(*k).into());
                }
            },
            ZoneEvent::EntityRespawned {
                entity,
                incarnation,
                ..
            } => {
                if let Some(k) = kinds.get(&entity) {
                    increment(&mut report.respawns, kind(*k).into());
                    note_life(report, lives, entity, incarnation, *k);
                }
            },
            ZoneEvent::EntityDespawn { entity, .. } => {
                // These are zone-wide facts, not observer AOI diffs. Player admission starts
                // a fresh incarnation counter; NPC slot members retain theirs across decay.
                // Consume the boundary in event order so earlier same-tick respawns count.
                if kinds.get(&entity) == Some(&EntityKind::Player) {
                    lives.remove(&entity);
                }
            },
            ZoneEvent::EffectApplied { .. }
            | ZoneEvent::EffectRemoved { .. }
            | ZoneEvent::SkillCastStarted { .. }
            | ZoneEvent::SkillCastResult { .. }
            | ZoneEvent::SkillCastCancelled { .. }
            | ZoneEvent::SkillLearned { .. }
            | ZoneEvent::PvpStateChanged { .. }
            | ZoneEvent::ResurrectOffer { .. }
            | ZoneEvent::SkillStateSync { .. }
            | ZoneEvent::SpGained { .. }
            | ZoneEvent::SkillReuse { .. }
            | ZoneEvent::VitalsChanged { .. }
            | ZoneEvent::TokensReconciled { .. }
            | ZoneEvent::AttackResult { .. }
            | ZoneEvent::AttackStarted { .. }
            | ZoneEvent::AttackCancelled { .. }
            | ZoneEvent::HateChanged { .. }
            | ZoneEvent::XpGained { .. }
            | ZoneEvent::LevelUp { .. }
            | ZoneEvent::TargetChanged { .. }
            | ZoneEvent::EntityMove { .. }
            | ZoneEvent::ItemView(_)
            | ZoneEvent::ItemResult(_)
            | ZoneEvent::ItemCheckpoint(_)
            | ZoneEvent::Progression(_) => {},
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use nightfall_api::domain::zone::{
        AppliedCommand, CommandSource, Disposition, Ordinal, RejectReason,
    };

    /// T48: a V6 recording reports schema four with the sim-gates `V4_COUNTERS`/`V4_SCALARS`
    /// (toggle-steps: two pure toggles and one charging toggle that pays a soulshot), while a
    /// V5 recording keeps schema three with no item keys.
    #[test]
    fn v6_recordings_report_schema_four_item_coverage() {
        let rec = Recording::from_bytes(include_bytes!("../../../fixtures/items/toggle-steps.nfr"))
            .unwrap();
        let report = serde_json::to_value(collect(&rec).unwrap()).unwrap();
        assert_eq!(report["schema_version"], 4);
        for key in [
            "item_operations",
            "enchant_outcomes",
            "refine_outcomes",
            "shot_payments",
        ] {
            assert!(report[key].is_object(), "{key}");
        }
        for key in [
            "item_ledger_rows",
            "item_allocations",
            "inventory_baselines",
        ] {
            assert!(report[key].is_u64(), "{key}");
        }
        assert_eq!(report["item_operations"]["toggle_auto_shot"], 3);
        assert_eq!(report["item_operations"]["use"], 1);
        assert_eq!(report["item_operations"]["consume"], 1);
        assert_eq!(report["shot_payments"], serde_json::json!({"soul": 1}));
        assert_eq!(report["item_ledger_rows"], 2);
        assert_eq!(report["item_allocations"], 0);
        assert_eq!(report["inventory_baselines"], 2);
        let legacy =
            Recording::from_bytes(include_bytes!("../../../fixtures/sessions/phase3-casts-v5.nfr"))
                .unwrap();
        let report = serde_json::to_value(collect(&legacy).unwrap()).unwrap();
        assert_eq!(report["schema_version"], 3);
        assert!(
            report.get("item_operations").is_none() && report.get("item_ledger_rows").is_none()
        );
    }

    #[test]
    fn rejected_player_spawn_cannot_reclassify_an_npc_or_clear_its_life_on_decay() {
        let rec = Recording::from_bytes(include_bytes!(
            "../../../fixtures/sessions/two-players-fight-v2.nfr"
        ))
        .unwrap();
        let spawn = rec.records.iter().flat_map(|r| decode_events(&r.events).unwrap()).find(|e| matches!(e, ZoneEvent::EntitySpawn { kind: EntityKind::Npc, combat: Some(c), .. } if c.incarnation == 2)).unwrap();
        let entity = spawn.entity();
        let mut record = rec.records[0].clone();
        record.commands = vec![AppliedCommand {
            ordinal: Ordinal(0),
            source: CommandSource::System,
            seq: None,
            command: ZoneCommand::SpawnPlayer {
                entity,
                name: "refused".into(),
                pos: nightfall_api::domain::zone::Vec2Fixed::default(),
                speed: nightfall_api::domain::zone::Speed::DEFAULT,
                generation: nightfall_api::domain::zone::SessionGeneration(1),
                load: None,
            },
        }];
        record.dispositions = vec![Disposition {
            ordinal: Ordinal(0),
            source: CommandSource::System,
            seq: None,
            tick_seen: record.tick,
            reason: RejectReason::AlreadyExists,
        }];
        let mut report = Coverage::new();
        let mut kinds = BTreeMap::from([(entity, EntityKind::Npc)]);
        let mut lives = BTreeMap::from([(entity, 1)]);
        commands(&mut report, &mut BTreeMap::new(), &mut kinds, &mut BTreeMap::new(), &record);
        assert_eq!(kinds[&entity], EntityKind::Npc);
        let respawn = rec.records.iter().flat_map(|r| decode_events(&r.events).unwrap()).find(|e| matches!(e, ZoneEvent::EntityRespawned { entity: id, incarnation: 2, .. } if *id == entity)).unwrap();
        facts(
            &mut report,
            &mut lives,
            &mut kinds,
            vec![
                ZoneEvent::EntityDespawn {
                    tick: record.tick,
                    entity,
                },
                spawn,
                respawn,
            ],
        );
        assert_eq!(lives[&entity], 2);
        assert_eq!(report.life_incarnations, BTreeMap::from([("npc:1->2".into(), 1)]));
        assert_eq!(report.respawns["npc"], 1);
    }

    #[test]
    fn class_actor_commands_count_separately_from_public_facts_and_owner_stats() {
        use nightfall_api::domain::zone::{SessionGeneration, Tick};
        use nightfall_api::domain::{class::ClassId, AccountId};
        let rec =
            Recording::from_bytes(include_bytes!("../../../fixtures/sessions/two-players-v4.nfr"))
                .unwrap();
        let mut record = rec.records[0].clone();
        let entity = EntityId::from_uuid(uuid::Uuid::from_u128(1));
        record.commands = (0..3)
            .map(|ordinal| AppliedCommand {
                ordinal: Ordinal(ordinal),
                source: CommandSource::System,
                seq: None,
                command: ZoneCommand::ChangeClass {
                    entity,
                    account: AccountId::from_uuid(uuid::Uuid::nil()),
                    request_key: uuid::Uuid::from_u128(10),
                    target: ClassId(1),
                },
            })
            .collect();
        record.dispositions = vec![Disposition {
            ordinal: Ordinal(2),
            source: CommandSource::System,
            seq: None,
            tick_seen: record.tick,
            reason: RejectReason::UnknownEntity,
        }];
        let mut report = Coverage::new();
        let mut attacks = BTreeMap::from([(entity, "idle")]);
        commands(&mut report, &mut attacks, &mut BTreeMap::new(), &mut BTreeMap::new(), &record);
        facts(
            &mut report,
            &mut BTreeMap::new(),
            &mut BTreeMap::new(),
            vec![
                crate::test_support::transfer_effect(entity, Tick(1)),
                ZoneEvent::ClassChanged {
                    entity,
                    tick: Tick(1),
                    class_id: ClassId(1),
                    generation: SessionGeneration(u64::MAX),
                },
                ZoneEvent::StatsChanged {
                    phase4: None,
                    entity,
                    tick: Tick(1),
                    class: None,
                    hp: 1,
                    max_hp: 2,
                    mp: 0,
                    max_mp: 2,
                    level: 20,
                    xp: 0,
                },
            ],
        );
        assert_eq!(report.class_transfer_commands["applied_without_rejection"], 2);
        assert_eq!(report.class_transfer_commands["rejected:UnknownEntity"], 1);
        assert_eq!(report.class_changes["1"], 1);
        assert_eq!(report.class_transfer_effects["0->1"], 1);
        assert_eq!(report.owner_stats_updates, 1);
        assert_eq!(attacks[&entity], "idle");
        assert!(report.player_attack_states.iter().all(|r| r.count == 0));
    }

    #[test]
    fn request_projection_counts_rejections_and_same_tick_stop_without_repeat_inflation() {
        let rec =
            Recording::from_bytes(include_bytes!("../../../fixtures/sessions/two-players-v4.nfr"))
                .unwrap();
        let mut record = rec.records[0].clone();
        let entity = EntityId::from_uuid(uuid::Uuid::from_u128(1));
        record.commands = [
            ZoneCommand::Attack { entity },
            ZoneCommand::Attack { entity },
            ZoneCommand::Attack { entity },
            ZoneCommand::StopAttack { entity },
        ]
        .into_iter()
        .enumerate()
        .map(|(index, command)| AppliedCommand {
            ordinal: Ordinal(u64::try_from(index).unwrap()),
            source: CommandSource::System,
            seq: None,
            command,
        })
        .collect();
        record.dispositions = vec![Disposition {
            ordinal: Ordinal(0),
            source: CommandSource::System,
            seq: None,
            tick_seen: record.tick,
            reason: RejectReason::UnknownEntity,
        }];
        let mut report = Coverage::new();
        let mut attacks = BTreeMap::from([(entity, "idle")]);
        commands(&mut report, &mut attacks, &mut BTreeMap::new(), &mut BTreeMap::new(), &record);
        let counts: Vec<_> = report
            .player_attack_states
            .iter()
            .filter(|r| r.count > 0)
            .map(|r| (r.from.as_str(), r.to.as_str(), r.count))
            .collect();
        assert_eq!(
            counts,
            [
                ("idle", "pending", 2),
                ("pending", "idle", 1),
                ("pending", "active", 1),
                ("active", "idle", 1)
            ]
        );
        assert_eq!(attacks[&entity], "idle");

        // Target removal precedes a later rejected Attack in this same tick.
        let target = EntityId::from_uuid(uuid::Uuid::from_u128(2));
        record.commands.truncate(2);
        record.commands[0].command = ZoneCommand::Despawn { entity: target };
        record.dispositions[0].ordinal = record.commands[1].ordinal;
        let mut report = Coverage::new();
        attacks.insert(entity, "active");
        commands(
            &mut report,
            &mut attacks,
            &mut BTreeMap::new(),
            &mut BTreeMap::from([(entity, Some(target))]),
            &record,
        );
        let counts: Vec<_> = report
            .player_attack_states
            .iter()
            .filter(|r| r.count > 0)
            .map(|r| (r.from.as_str(), r.to.as_str(), r.count))
            .collect();
        assert_eq!(
            counts,
            [
                ("idle", "pending", 1),
                ("pending", "idle", 1),
                ("active", "idle", 1)
            ]
        );
    }
}
