//! C14 evidence: verified replay observations, bound to the exact recording bytes.
use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context as _};
use nightfall_api::application::replay_log::AppliedTickRecord;
use nightfall_api::domain::zone::{AppliedTick, AppliedTickDraft, ObserverOutput, ZoneEvent};
use nightfall_api::infrastructure::eventlog::Recording;
use serde_json::json;
use sha2::{Digest, Sha256};

fn line(out: &mut Vec<u8>, value: &impl serde::Serialize) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *out, value)?;
    out.push(b'\n');
    Ok(())
}

// Observer-specific identity and combat carriers are evidence, never simulation state.
fn observer_combat(
    out: &mut Vec<u8>,
    applied: &AppliedTick,
    count: &mut u64,
) -> anyhow::Result<()> {
    if applied.digest_version != nightfall_api::domain::zone::StateDigestVersion::BinaryV5 {
        return Ok(());
    }
    for (observer, outputs) in &applied.outputs {
        let events: Vec<_> = outputs
            .iter()
            .filter_map(|output| match output {
                ObserverOutput::Event(event)
                    if matches!(
                        event,
                        ZoneEvent::EntitySpawn {
                            combat: Some(_),
                            ..
                        } | ZoneEvent::EntityDespawn { .. }
                            | ZoneEvent::AttackStarted { .. }
                            | ZoneEvent::AttackResult { .. }
                            | ZoneEvent::StatsChanged { .. }
                            | ZoneEvent::VitalsChanged { .. }
                            | ZoneEvent::SkillCastStarted { .. }
                            | ZoneEvent::SkillCastResult { .. }
                            | ZoneEvent::SkillCastCancelled { .. }
                    ) =>
                {
                    Some(event)
                },
                ObserverOutput::Event(_)
                | ObserverOutput::Accepted { .. }
                | ObserverOutput::Rejected(_) => None,
            })
            .collect();
        if events.is_empty() {
            continue;
        }
        line(
            out,
            &json!({"kind": "observer_combat", "entity": observer,
            "tick": applied.tick, "sequence": count, "data": {"events": events}}),
        )?;
        *count = count
            .checked_add(1)
            .context("evidence sequence exhausted")?;
    }
    Ok(())
}

/// No evidence leaves this function until every record and the completion watermark match.
pub(super) fn collect(rec: &Recording, recording_bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let seed = rec.snapshot.seed;
    if rec.watermark.zone != seed.zone
        || rec.watermark.epoch != seed.epoch
        || rec.watermark.records != u64::try_from(rec.records.len())?
        || rec.watermark.last_tick != rec.records.last().map(|r| r.tick)
    {
        bail!("evidence requires a consistent completion watermark");
    }
    let mut state = nightfall_api::application::replay_log::restore_snapshot(rec.snapshot.clone())?;
    state.enable_replay_evidence();
    let mut out = Vec::new();
    line(
        &mut out,
        &json!({
            "kind": "manifest", "schema_version": 1,
            "recording_sha256": format!("{:x}", Sha256::digest(recording_bytes)),
            "zone": seed.zone.0, "epoch": seed.epoch,
            "snapshot_tick": rec.snapshot.tick, "watermark": rec.watermark,
            "snapshot_meta": rec.snapshot.meta,
            "rules": rec.snapshot.rules,
            "classes": rec.snapshot.classes,
            "phase3_rules": rec.snapshot.phase3.as_ref().map(|p| p.rules.as_ref()),
            "scope": "whole_recording"
        }),
    )?;
    let mut count = 0_u64;
    for record in &rec.records {
        if record.zone != seed.zone || record.digest_version != rec.snapshot.meta.digest_version {
            bail!("evidence record policy/zone mismatch at tick {}", record.tick.0);
        }
        let applied = state.run_tick(AppliedTickDraft {
            epoch: record.epoch,
            tick: record.tick,
            commands: record.commands.clone(),
        })?;
        if !record.reproduced_by(&AppliedTickRecord::from_applied(seed.zone, &applied)) {
            bail!("evidence replay divergence at tick {}", record.tick.0);
        }
        for fact in state.take_replay_evidence() {
            line(
                &mut out,
                &json!({"kind": fact.kind, "entity": fact.entity,
                "tick": fact.tick, "sequence": count, "data": fact.data}),
            )?;
            count = count
                .checked_add(1)
                .context("evidence sequence exhausted")?;
        }
        observer_combat(&mut out, &applied, &mut count)?;
        for event in &applied.events {
            if let ZoneEvent::SkillLearned {
                tick,
                entity,
                account,
                request_key,
                result,
            } = event
            {
                line(
                    &mut out,
                    &json!({"kind": "learning_commit", "entity": entity,
                    "tick": tick, "sequence": count,
                    "data": {"account": account, "request_key": request_key, "result": result}}),
                )?;
                count = count
                    .checked_add(1)
                    .context("evidence sequence exhausted")?;
            }
        }
    }
    line(
        &mut out,
        &json!({"kind": "complete", "facts": count,
        "ticks": rec.records.len(), "last_tick": rec.watermark.last_tick}),
    )?;
    Ok(out)
}

/// Fresh destination prevents stale-success reuse and accidental input clobbering.
pub(super) fn write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create fresh evidence {}", path.display()))?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(error).with_context(|| format!("write evidence {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
#[path = "evidence_tests.rs"]
mod tests;
