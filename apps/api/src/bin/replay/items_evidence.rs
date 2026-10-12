//! Frozen D46 JSONL, released only after whole-recording byte/digest verification.
use anyhow::{bail, Context as _};
use nightfall_api::application::replay_log::{AppliedTickRecord, WatermarkReason};
use nightfall_api::domain::zone::{AppliedTickDraft, StateDigestVersion};
use nightfall_api::infrastructure::eventlog::Recording;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

fn line(out: &mut Vec<u8>, value: &impl Serialize) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *out, value)?;
    out.push(b'\n');
    Ok(())
}

#[derive(Serialize)]
struct SequencedFact<'a> {
    sequence: u64,
    #[serde(flatten)]
    fact: &'a nightfall_api::domain::zone::evidence::ItemEvidenceFact,
}

#[allow(clippy::too_many_lines)] // Whole-recording validation precedes any artifact publication.
pub(super) fn collect(rec: &Recording, recording_bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let seed = rec.snapshot.seed;
    let phase4 = rec
        .snapshot
        .phase4
        .as_ref()
        .context("item evidence requires V6 items")?;
    let records = u64::try_from(rec.records.len())?;
    let last_tick = rec
        .snapshot
        .tick
        .0
        .checked_add(records)
        .and_then(|n| n.checked_sub(1))
        .context("item evidence tick overflow")?;
    if rec.snapshot.meta.schema_version != 10
        || rec.snapshot.meta.digest_version != StateDigestVersion::BinaryV6
        || seed.zone.0 == 0
        || seed.epoch == 0
        || records == 0
        || rec.watermark.zone != seed.zone
        || rec.watermark.epoch != seed.epoch
        || rec.watermark.records != records
        || rec.watermark.last_tick.map(|tick| tick.0) != Some(last_tick)
        || rec.records.last().map(|record| record.tick.0) != Some(last_tick)
    {
        bail!("item evidence requires a complete, consistent V6 recording");
    }
    let reason = match rec.watermark.reason {
        WatermarkReason::Shutdown => "shutdown",
        WatermarkReason::EpochEnd => "epoch_closed",
        WatermarkReason::Capture => bail!("item evidence refuses partial/live captures"),
    };
    let mut rules_hash = String::with_capacity(64);
    for byte in phase4.rules_hash {
        use std::fmt::Write as _;
        write!(&mut rules_hash, "{byte:02x}")?;
    }
    let mut state = nightfall_api::application::replay_log::restore_snapshot(rec.snapshot.clone())?;
    state.enable_items_replay_evidence()?;
    let mut out = Vec::new();
    line(
        &mut out,
        &json!({"kind":"manifest","schema_version":1,"scope":"whole_recording",
        "recording_sha256":format!("{:x}",Sha256::digest(recording_bytes)),
        "zone":seed.zone.0,"epoch":seed.epoch,"snapshot_tick":rec.snapshot.tick,
        "watermark":{"zone":seed.zone.0,"epoch":seed.epoch,"records":records,"last_tick":last_tick,"reason":reason},
        "phase4_rules_hash":rules_hash,"snapshot_schema":10,"record_schema":8,"digest_version":"BinaryV6"}),
    )?;
    let mut sequence = 0_u64;
    let mut cursor = rec.snapshot.rng.word_pos;
    for record in &rec.records {
        if record.zone != seed.zone
            || record.epoch != seed.epoch
            || record.digest_version != StateDigestVersion::BinaryV6
        {
            bail!("item evidence record policy mismatch at tick {}", record.tick.0);
        }
        let applied = state.run_tick(AppliedTickDraft {
            epoch: record.epoch,
            tick: record.tick,
            commands: record.commands.clone(),
        })?;
        if !record.reproduced_by(&AppliedTickRecord::from_applied(seed.zone, &applied)) {
            bail!("item evidence replay divergence at tick {}", record.tick.0);
        }
        for fact in state.take_items_replay_evidence() {
            if let nightfall_api::domain::zone::evidence::ItemEvidenceData::Rng {
                word_before,
                word_after,
                bound,
                value,
                ..
            } = &fact.fact
            {
                if word_after <= word_before
                    || *bound == 0
                    || value >= bound
                    || cursor != *word_before
                {
                    bail!("incomplete item RNG trace at tick {}", record.tick.0);
                }
                cursor = *word_after;
            }
            if (fact.incarnation == 0)
                != (fact.origin
                    == nightfall_api::domain::zone::evidence::ItemEvidenceOrigin::Offline)
                || fact.tick > record.tick
            {
                bail!("invalid item evidence subject/tick");
            }
            line(
                &mut out,
                &SequencedFact {
                    sequence,
                    fact: &fact,
                },
            )?;
            sequence = sequence
                .checked_add(1)
                .context("item evidence sequence overflow")?;
        }
        if cursor != state.replay_rng_word_position() {
            bail!("missing item RNG tail at tick {}", record.tick.0);
        }
    }
    if sequence == 0 {
        bail!("item evidence requires observed facts");
    }
    line(
        &mut out,
        &json!({"kind":"complete","facts":sequence,"ticks":records,"last_tick":last_tick}),
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_api::domain::zone::evidence::{ItemEvidenceData, ItemEvidenceFact};

    #[test]
    fn item_export_requires_export_mode_and_distinct_fresh_destination() {
        for args in [
            vec![
                "check",
                "--file",
                "record.nfr",
                "--items-evidence-out",
                "proof.jsonl",
            ],
            vec![
                "export",
                "--file",
                "record.nfr",
                "--items-evidence-out",
                "record.nfr",
            ],
            vec![
                "export",
                "--file",
                "record.nfr",
                "--items-evidence-out",
                "proof.jsonl",
                "--evidence-out",
                "old.jsonl",
            ],
        ] {
            assert!(crate::parse_args(args.into_iter().map(str::to_owned)).is_err());
        }
        assert!(crate::parse_args(
            [
                "export",
                "--file",
                "record.nfr",
                "--items-evidence-out",
                "proof.jsonl"
            ]
            .into_iter()
            .map(str::to_owned)
        )
        .is_ok());
    }

    #[test]
    fn historical_recording_cannot_be_relabelled_as_item_evidence() -> anyhow::Result<()> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/sessions/phase3-gameplay-v5.nfr");
        let (recording, bytes) = Recording::read_with_bytes(&path)?;
        assert!(collect(&recording, &bytes).is_err());
        Ok(())
    }

    #[test]
    fn cursor_is_an_exact_json_integer_above_u64() -> anyhow::Result<()> {
        let before = u128::from(u64::MAX).saturating_add(1);
        let fact = ItemEvidenceFact {
            entity: "00000000-0000-0000-0000-000000000001".parse()?,
            incarnation: 1,
            origin: nightfall_api::domain::zone::evidence::ItemEvidenceOrigin::Live,
            tick: nightfall_api::domain::zone::Tick(1),
            fact: ItemEvidenceData::Rng {
                callsite: "enchant".into(),
                word_before: before,
                word_after: before.saturating_add(1),
                bound: 100_000,
                value: 1,
                resolution_id: "enchant:wide-cursor".into(),
            },
        };
        let mut out = Vec::new();
        line(
            &mut out,
            &SequencedFact {
                sequence: 0,
                fact: &fact,
            },
        )?;
        let text = String::from_utf8(out)?;
        assert!(text.contains("\"word_before\":18446744073709551616"));
        assert!(text.contains("\"word_after\":18446744073709551617"));
        assert!(text.contains("\"kind\":\"item_rng\""));
        Ok(())
    }
}
