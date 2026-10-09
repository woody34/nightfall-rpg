//! Recording coverage: durable facts and the server-observable attack request lifecycle.
use std::collections::BTreeMap;

use nightfall_api::application::replay_log::{decode_events, AppliedTickRecord, OutputForm};
use nightfall_api::domain::zone::{
    AppliedTickDraft, EntityId, EntityKind, ZoneCommand, ZoneEvent, ZoneState,
};
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
            schema_version: 1,
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
        }
    }

    pub(super) fn print(&self) {
        for (name, rows) in [
            ("NPC intentions", &self.npc_intentions),
            ("Player attack states", &self.player_attack_states),
        ] {
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
        for (name, counts) in [
            ("Life incarnations", &self.life_incarnations),
            ("Deaths", &self.deaths),
            ("Respawns", &self.respawns),
            ("IntentRejected", &self.intent_rejected),
        ] {
            println!("{name}");
            for (key, count) in counts {
                println!("{key}\t{count}");
            }
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
    let mut zone = ZoneState::from_snapshot(rec.snapshot.clone())?;
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
        for d in &r.dispositions {
            increment(&mut report.intent_rejected, format!("{:?}", d.reason));
        }
        commands(&mut report, &mut attacks, &mut kinds, &mut targets, r);
        let events = match r.output_form {
            OutputForm::Encoded => decode_events(&r.events)?,
            OutputForm::Sha256 => applied.events,
        };
        facts(&mut report, &mut lives, &mut kinds, events);
        for e in &zone.snapshot().entities {
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
        if let ZoneCommand::SpawnPlayer { entity, .. } = command.command {
            kinds.insert(entity, EntityKind::Player);
            attacks.entry(entity).or_insert("idle");
        }
        let rejected = r.dispositions.iter().any(|d| d.ordinal == command.ordinal);
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
            ZoneCommand::SpawnPlayer { .. }
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
            ZoneEvent::AttackResult { .. }
            | ZoneEvent::AttackStarted { .. }
            | ZoneEvent::AttackCancelled { .. }
            | ZoneEvent::HateChanged { .. }
            | ZoneEvent::StatsChanged { .. }
            | ZoneEvent::XpGained { .. }
            | ZoneEvent::LevelUp { .. }
            | ZoneEvent::TargetChanged { .. }
            | ZoneEvent::EntityMove { .. }
            | ZoneEvent::Progression(_)
            | ZoneEvent::EntityDespawn { .. } => {},
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
