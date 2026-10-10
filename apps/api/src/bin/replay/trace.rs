//! Offline trace projection. Recorded facts remain authoritative; digest-only facts are labelled.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{bail, Context as _};
use nightfall_api::application::replay_log::{
    decode_events, decode_outputs, AppliedTickRecord, OutputForm,
};
use nightfall_api::domain::zone::{
    AppliedTickDraft, AttackOutcome, EntityId, ObserverOutput, Vec2Fixed, ZoneCommand, ZoneEvent,
    ZoneState,
};
use nightfall_api::infrastructure::eventlog::Recording;
use serde::Serialize;

#[derive(Serialize)]
struct Track {
    id: EntityId,
    name: String,
    // Tick, milli-tile x/y, segment (break on despawn or teleport).
    points: Vec<(u64, i32, i32, u64)>,
    segment: u64,
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn glyph(e: &ZoneEvent) -> Option<(&'static str, &'static str)> {
    match e {
        ZoneEvent::AttackResult { outcome, .. } => Some(match outcome {
            AttackOutcome::Hit => ("hit", "●"),
            AttackOutcome::Miss => ("miss", "○"),
            AttackOutcome::Crit => ("crit", "★"),
        }),
        ZoneEvent::ClassChanged { .. } => Some(("class", "◆")),
        ZoneEvent::EntityDied { .. } => Some(("death", "×")),
        ZoneEvent::EntityRespawned { .. } => Some(("respawn", "↥")),
        ZoneEvent::NpcIntentionChanged { .. } => Some(("intention", "◇")),
        ZoneEvent::TargetChanged { .. } => Some(("target", "◎")),
        ZoneEvent::TokensReconciled { .. }
        | ZoneEvent::ClassTransfer { .. }
        | ZoneEvent::AttackStarted { .. }
        | ZoneEvent::AttackCancelled { .. }
        | ZoneEvent::HateChanged { .. }
        | ZoneEvent::StatsChanged { .. }
        | ZoneEvent::XpGained { .. }
        | ZoneEvent::LevelUp { .. }
        | ZoneEvent::EntitySpawn { .. }
        | ZoneEvent::EntityMove { .. }
        | ZoneEvent::Progression(_)
        | ZoneEvent::EntityDespawn { .. } => None,
    }
}

struct Projection {
    tracks: BTreeMap<EntityId, Track>,
    positions: BTreeMap<EntityId, Vec2Fixed>,
    markers: String,
    counts: BTreeMap<&'static str, usize>,
}

impl Projection {
    fn new(rec: &Recording) -> Self {
        let mut p = Self {
            tracks: BTreeMap::new(),
            positions: BTreeMap::new(),
            markers: String::new(),
            counts: BTreeMap::new(),
        };
        for e in &rec.snapshot.entities {
            p.positions.insert(e.id, e.pos);
            p.tracks.insert(
                e.id,
                Track {
                    id: e.id,
                    name: e.name.clone(),
                    points: Vec::new(),
                    segment: 0,
                },
            );
        }
        p
    }

    fn event(&mut self, e: &ZoneEvent) {
        let entity = e.entity();
        match e {
            ZoneEvent::EntitySpawn { name, pos, .. } => {
                self.tracks.entry(entity).or_insert_with(|| Track {
                    id: entity,
                    name: name.clone(),
                    points: Vec::new(),
                    segment: 0,
                });
                self.positions.insert(entity, *pos);
            },
            ZoneEvent::EntityMove { pos, .. } => {
                self.positions.insert(entity, *pos);
            },
            ZoneEvent::EntityRespawned { position, .. } => {
                self.positions.insert(entity, *position);
                self.break_path(entity);
            },
            ZoneEvent::EntityDespawn { .. } => {
                self.positions.remove(&entity);
                self.break_path(entity);
            },
            ZoneEvent::ClassChanged { .. }
            | ZoneEvent::TokensReconciled { .. }
            | ZoneEvent::ClassTransfer { .. }
            | ZoneEvent::AttackResult { .. }
            | ZoneEvent::EntityDied { .. }
            | ZoneEvent::AttackStarted { .. }
            | ZoneEvent::AttackCancelled { .. }
            | ZoneEvent::HateChanged { .. }
            | ZoneEvent::NpcIntentionChanged { .. }
            | ZoneEvent::StatsChanged { .. }
            | ZoneEvent::XpGained { .. }
            | ZoneEvent::LevelUp { .. }
            | ZoneEvent::TargetChanged { .. }
            | ZoneEvent::Progression(_) => {},
        }
    }

    fn break_path(&mut self, entity: EntityId) {
        if let Some(track) = self.tracks.get_mut(&entity) {
            track.segment = track.segment.saturating_add(1);
        }
    }

    fn marker(
        &mut self,
        tick: u64,
        entity: Option<EntityId>,
        kind: &'static str,
        symbol: &str,
        detail: &str,
    ) -> anyhow::Result<()> {
        let pos = entity.and_then(|id| self.positions.get(&id));
        let (x, y) = pos.map_or((String::new(), String::new()), |p| {
            (p.x.raw().to_string(), p.y.raw().to_string())
        });
        // Missing positions are displayed in the map's unlocated-event rail, never at a fabricated world position.
        write!(self.markers, "<text class=\"event\" data-kind=\"{kind}\" data-tick=\"{tick}\" data-x=\"{x}\" data-y=\"{y}\">{symbol}<title>{}</title></text>", escape(&format!("tick {tick}: {detail}")))?;
        let count = self.counts.entry(kind).or_default();
        *count = count.saturating_add(1);
        Ok(())
    }
}

// Private state is shown only in the selected owner's recorded output, never in off-AOI facts.
fn event_detail(event: &ZoneEvent, owner: Option<EntityId>) -> String {
    match event {
        ZoneEvent::ClassChanged {
            entity,
            class_id,
            tick,
            generation,
        } => format!(
            "ClassChanged: {entity} → class {} · tick {} · generation {}",
            class_id.0, tick.0, generation.0
        ),
        ZoneEvent::TokensReconciled {
            entity,
            adjustment,
            source,
            ..
        } => format!(
            "Tokens reconciled: {entity} · {source:?} · claimed {} · granted {}",
            adjustment.claimed_mask, adjustment.granted_mask
        ),
        ZoneEvent::ClassTransfer {
            entity,
            tick,
            old_class_id,
            receipt,
        } => format!(
            "ClassTransfer effect: {entity} · {} → {} · tick {} (private receipt hidden)",
            old_class_id.0, receipt.target_class_id.0, tick.0
        ),
        ZoneEvent::StatsChanged { entity, tick, .. } | ZoneEvent::XpGained { entity, tick, .. }
            if owner != Some(*entity) =>
        {
            format!("Owner update: {entity} · tick {} (owner-private fields hidden)", tick.0)
        },
        ZoneEvent::Progression(delta) => format!(
            "Progression checkpoint: {} · tick {} (private state hidden)",
            delta.entity, delta.tick.0
        ),
        ZoneEvent::AttackResult { .. }
        | ZoneEvent::EntityDied { .. }
        | ZoneEvent::AttackStarted { .. }
        | ZoneEvent::AttackCancelled { .. }
        | ZoneEvent::HateChanged { .. }
        | ZoneEvent::NpcIntentionChanged { .. }
        | ZoneEvent::EntityRespawned { .. }
        | ZoneEvent::StatsChanged { .. }
        | ZoneEvent::XpGained { .. }
        | ZoneEvent::LevelUp { .. }
        | ZoneEvent::TargetChanged { .. }
        | ZoneEvent::EntitySpawn { .. }
        | ZoneEvent::EntityMove { .. }
        | ZoneEvent::EntityDespawn { .. } => format!("{event:#?}"),
    }
}

fn command_details(record: &AppliedTickRecord) -> String {
    let mut lines = Vec::new();
    for applied in &record.commands {
        let detail = match &applied.command {
            ZoneCommand::SpawnPlayer { .. } => {
                let mut command = applied.command.clone();
                if let ZoneCommand::SpawnPlayer { load, .. } = &mut command {
                    *load = None;
                }
                format!("{command:#?} (private admission load hidden)")
            },
            ZoneCommand::ChangeClass { entity, target, .. } => format!(
                "ChangeClass actor command: {entity} → class {} (gRPC mutation; not a WebSocket intent)",
                target.0
            ),
            ZoneCommand::SpawnNpc { .. }
            | ZoneCommand::Despawn { .. }
            | ZoneCommand::ReplaceSession { .. }
            | ZoneCommand::MoveTo { .. }
            | ZoneCommand::SetTarget { .. }
            | ZoneCommand::Attack { .. }
            | ZoneCommand::StopAttack { .. }
            | ZoneCommand::Respawn { .. }
            | ZoneCommand::StopMove { .. }
            | ZoneCommand::AddAggro { .. } => format!("{:#?}", applied.command),
        };
        lines.push(format!(
            "ordinal {} · source {:?} · seq {:?}\n{detail}",
            applied.ordinal.0, applied.source, applied.seq
        ));
    }
    format!("{}\n{:#?}", lines.join("\n"), record.dispositions)
}

fn output_details(
    bytes: &[u8],
    recipient: EntityId,
    session: Option<EntityId>,
) -> anyhow::Result<String> {
    let owner = session.filter(|id| *id == recipient);
    Ok(decode_outputs(bytes)?
        .iter()
        .map(|item| match item {
            ObserverOutput::Event(event) => event_detail(event, owner),
            ObserverOutput::Accepted { .. } | ObserverOutput::Rejected(_) => format!("{item:#?}"),
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn row(
    record: &AppliedTickRecord,
    session: Option<EntityId>,
    fail: Option<u64>,
    events: &[ZoneEvent],
) -> anyhow::Result<String> {
    let tick = record.tick.0;
    let mut html = format!(
        "<tr id=\"tick-{tick}\" class=\"{}\"><th>{tick}</th><td><pre>{}</pre></td><td>",
        if fail == Some(tick) { "failure" } else { "" },
        escape(&command_details(record))
    );
    for output in &record.outputs {
        let decoded = match record.output_form {
            OutputForm::Encoded => output_details(&output.bytes, output.entity, session)?,
            OutputForm::Sha256 => {
                format!("SHA-256 only (message bytes unavailable): {:02x?}", output.bytes)
            },
        };
        write!(
            html,
            "<details class=\"{}\" open><summary>Output to {}</summary><pre>{}</pre></details>",
            if session == Some(output.entity) {
                "own"
            } else {
                ""
            },
            output.entity,
            escape(&decoded)
        )?;
    }
    let facts = events
        .iter()
        .map(|event| event_detail(event, None))
        .collect::<Vec<_>>()
        .join("\n");
    write!(html, "</td><td><pre>{}</pre></td></tr>", escape(&facts))?;
    Ok(html)
}

pub(super) fn render(
    rec: &Recording,
    session: Option<EntityId>,
    fail: Option<u64>,
) -> anyhow::Result<String> {
    let first = rec
        .records
        .first()
        .context("trace requires at least one tick")?
        .tick
        .0;
    let last = rec
        .records
        .last()
        .context("trace requires at least one tick")?
        .tick
        .0;
    if fail.is_some_and(|t| t < first || t > last) {
        bail!("--fail-tick must be within {first}..={last}");
    }
    if session.is_some_and(|id| {
        !rec.records
            .iter()
            .any(|r| r.outputs.iter().any(|o| o.entity == id))
    }) {
        bail!("selected session has no outputs in this recording");
    }
    let focus = fail.unwrap_or(first);
    let low = focus.saturating_sub(20).max(first);
    let high = focus.saturating_add(20).min(last);
    let mut p = Projection::new(rec);
    // Re-run only when digest-only records require reconstruction. Never silently replace recorded facts.
    let mut zone = if rec
        .records
        .iter()
        .any(|r| r.output_form == OutputForm::Sha256)
    {
        Some(ZoneState::from_snapshot(rec.snapshot.clone())?)
    } else {
        None
    };
    let mut rows = String::new();
    let stride = rec.records.len().div_ceil(600).max(1);
    for (index, r) in rec.records.iter().enumerate() {
        let rerun = zone
            .as_mut()
            .map(|z| {
                z.run_tick(AppliedTickDraft {
                    epoch: r.epoch,
                    tick: r.tick,
                    commands: r.commands.clone(),
                })
            })
            .transpose()?;
        let events = match r.output_form {
            OutputForm::Encoded => decode_events(&r.events)?,
            OutputForm::Sha256 => rerun.context("digest reconstruction unavailable")?.events,
        };
        let mut important = BTreeSet::new();
        for e in &events {
            p.event(e);
            if matches!(e, ZoneEvent::EntitySpawn { .. } | ZoneEvent::EntityRespawned { .. }) {
                important.insert(e.entity());
            }
            if let Some((kind, symbol)) = glyph(e) {
                important.insert(e.entity());
                p.marker(r.tick.0, Some(e.entity()), kind, symbol, &event_detail(e, None))?;
            }
        }
        for d in &r.dispositions {
            let entity = r
                .commands
                .iter()
                .find(|c| c.ordinal == d.ordinal)
                .and_then(|c| c.command.entity());
            if let Some(id) = entity {
                important.insert(id);
            }
            p.marker(r.tick.0, entity, "rejection", "!", &format!("{d:?}"))?;
        }
        for (id, pos) in &p.positions {
            if index.is_multiple_of(stride)
                || r.tick.0 == last
                || (low..=high).contains(&r.tick.0)
                || important.contains(id)
            {
                if let Some(track) = p.tracks.get_mut(id) {
                    track
                        .points
                        .push((r.tick.0, pos.x.raw(), pos.y.raw(), track.segment));
                }
            }
        }
        if (low..=high).contains(&r.tick.0) {
            rows.push_str(&row(r, session, fail, &events)?);
        }
    }
    let bounds = rec.snapshot.bounds;
    let data = serde_json::json!({ "tracks": p.tracks.values().collect::<Vec<_>>(), "bounds": [bounds.min().x.raw(), bounds.min().y.raw(), bounds.max().x.raw(), bounds.max().y.raw()], "first": first, "last": last, "fail": fail, "session": session });
    // JSON is in an HTML raw-text element: escape '<' even inside entity names (</script>).
    let data = serde_json::to_string(&data)?
        .replace('<', "\\u003c")
        .replace('&', "\\u0026");
    let mut summary = format!("{} entities · ticks {first}–{last} · snapshot schema {} · state digest {:?} · path sampling every {stride} ticks (event ticks and table window retained).", p.tracks.len(), rec.snapshot.meta.schema_version, rec.snapshot.meta.digest_version);
    for (kind, count) in p.counts {
        write!(summary, " {kind}: {count}.")?;
    }
    Ok(include_str!("trace.html")
        .replace("@@SUMMARY@@", &summary)
        .replace("@@DATA@@", &data)
        .replace("@@EVENTS@@", &p.markers)
        .replace("@@ROWS@@", &rows)
        .replace("@@WINDOW@@", &format!("{low}–{high}")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn class_change_is_concise_and_private_stats_require_the_selected_recipient() {
        use nightfall_api::application::replay_log::{encode_events, encode_outputs, PlayerOutput};
        use nightfall_api::domain::class::ClassId;
        use nightfall_api::domain::zone::{ClassStatsView, SessionGeneration, Tick};
        let mut rec =
            Recording::from_bytes(include_bytes!("../../../fixtures/sessions/two-players-v4.nfr"))
                .unwrap();
        rec.records.truncate(1);
        let owner = EntityId::from_uuid(uuid::Uuid::from_u128(1));
        let observer = EntityId::from_uuid(uuid::Uuid::from_u128(2));
        let tick = rec.records[0].tick;
        let changed = ZoneEvent::ClassChanged {
            entity: owner,
            class_id: ClassId(1),
            tick,
            generation: SessionGeneration(u64::MAX),
        };
        let stats = ZoneEvent::StatsChanged {
            entity: owner,
            tick,
            hp: 1,
            max_hp: 2,
            mp: 0,
            max_mp: 2,
            level: 20,
            xp: 0,
            class: Some(ClassStatsView {
                class_id: ClassId(1),
                sp: 765_432,
                cp: 987_654,
                max_cp: 999_999,
                token_tier_1_count: 17,
                token_tier_2_count: 19,
            }),
        };
        rec.records[0].events = encode_events(&[
            changed.clone(),
            stats.clone(),
            crate::test_support::transfer_effect(owner, tick),
        ]);
        // A malformed observer stream must not make another player's resource values visible.
        let bytes = encode_outputs(&[ObserverOutput::Event(changed), ObserverOutput::Event(stats)]);
        rec.records[0].outputs = vec![
            PlayerOutput {
                entity: owner,
                bytes: bytes.clone(),
            },
            PlayerOutput {
                entity: observer,
                bytes,
            },
        ];
        let owner_page = render(&rec, Some(owner), None).unwrap();
        assert_eq!(owner_page.matches("cp: 987654").count(), 1);
        assert_eq!(owner_page.matches("sp: 765432").count(), 1);
        for selected in [None, Some(observer)] {
            let html = render(&rec, selected, None).unwrap();
            assert!(!html.contains("987654"));
            assert!(!html.contains("765432"));
            assert!(!html.contains("PRIVATE_GRANTED_SKILL_SENTINEL"));
            assert!(html.contains("ClassTransfer effect:"));
            assert!(html.contains("private receipt hidden"));
            assert!(html.contains("owner-private fields hidden"));
            assert!(html.contains("ClassChanged:"));
            assert!(html.contains("→ class 1"));
            assert!(html.contains("generation 18446744073709551615"));
            assert_eq!(html.matches("data-kind=\"class\"").count(), 1);
        }
        assert_eq!(
            event_detail(
                &ZoneEvent::ClassChanged {
                    entity: owner,
                    class_id: ClassId(0),
                    tick: Tick(0),
                    generation: SessionGeneration(0)
                },
                None
            ),
            format!("ClassChanged: {owner} → class 0 · tick 0 · generation 0")
        );
    }

    #[test]
    fn names_cannot_escape_html_or_script_and_digests_are_not_decoded_as_messages() {
        let mut rec =
            Recording::from_bytes(include_bytes!("../../../fixtures/sessions/two-players-v4.nfr"))
                .unwrap();
        for record in &mut rec.records {
            let mut events = decode_events(&record.events).unwrap();
            for event in &mut events {
                if let ZoneEvent::EntitySpawn { name, .. } = event {
                    *name = "</script><img src=x onerror=alert(1)>".into();
                }
            }
            record.events = nightfall_api::application::replay_log::encode_events(&events);
        }
        let html = render(&rec, None, None).unwrap();
        assert!(!html.contains("<img src=x"));
        assert!(html.contains("\\u003c/script>"));
        let mut rec =
            Recording::from_bytes(include_bytes!("../../../fixtures/sessions/two-players-v4.nfr"))
                .unwrap();
        let index = rec
            .records
            .iter()
            .position(|r| !r.outputs.is_empty())
            .unwrap();
        let tick = rec.records[index].tick.0;
        rec.records[index] = rec.records[index].with_output_digests();
        let html = render(&rec, None, Some(tick)).unwrap();
        assert!(html.contains("SHA-256 only (message bytes unavailable)"));
    }
}
