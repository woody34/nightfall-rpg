#![allow(clippy::indexing_slicing)] // JSON Value indexing returns Null; typed checks below fail closed.
use super::*;
use std::collections::BTreeSet;

fn fixture(name: &str) -> anyhow::Result<(Recording, Vec<u8>)> {
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/sessions")
            .join(name),
    )?;
    Ok((Recording::from_bytes(&bytes)?, bytes))
}
fn rows(bytes: &[u8]) -> anyhow::Result<Vec<serde_json::Value>> {
    std::str::from_utf8(bytes)?
        .lines()
        .map(|s| serde_json::from_str(s).map_err(Into::into))
        .collect()
}

#[test]
fn gameplay_evidence_is_byte_identical_across_two_verified_replays() -> anyhow::Result<()> {
    let (rec, bytes) = fixture("phase3-gameplay-v5.nfr")?;
    let first = collect(&rec, &bytes)?;
    assert_eq!(first, collect(&rec, &bytes)?);
    let rows = rows(&first)?;
    assert_eq!(
        rows.first().context("manifest")?["recording_sha256"],
        format!("{:x}", Sha256::digest(&bytes))
    );
    assert_eq!(rows.last().context("completion")?["ticks"], rec.records.len());
    let kinds: BTreeSet<_> = rows.iter().filter_map(|r| r["kind"].as_str()).collect();
    for kind in [
        "regen_pulse",
        "formula_resolution",
        "rng_draw",
        "modifier_reconstruction",
        "hate_delta",
        "death_record",
        "resurrection_offer",
        "resurrection_refund",
    ] {
        assert!(kinds.contains(kind), "missing {kind}: {kinds:?}");
    }
    let facts: Vec<_> = rows
        .iter()
        .filter(|row| row.get("sequence").is_some())
        .collect();
    for (index, row) in facts.iter().enumerate() {
        assert_eq!(row["sequence"], index);
    }
    assert_eq!(rows.last().context("completion")?["facts"], facts.len());
    for row in rows.iter().filter(|r| r["kind"] == "rng_draw") {
        let data = &row["data"];
        assert!(
            data["value"].as_u64().context("value")? < data["bound"].as_u64().context("bound")?
        );
        let before: u128 = data["word_before"].as_str().context("cursor")?.parse()?;
        let after: u128 = data["word_after"].as_str().context("cursor")?.parse()?;
        assert!(after > before);
    }
    for row in rows.iter().filter(|r| r["kind"] == "regen_pulse") {
        let data = &row["data"];
        for resource in ["hp", "mp"] {
            let before = data["before"][resource]
                .as_u64()
                .context("resource before")?;
            let actual = data["actual"][resource]
                .as_u64()
                .context("resource after")?;
            let granted = data["actual"][format!("{resource}_granted")]
                .as_u64()
                .context("credit")?;
            assert_eq!(actual.checked_sub(before), Some(granted));
            assert!(
                data["actual"]["fractions"][resource]
                    .as_i64()
                    .context("Q remainder")?
                    < 1_000_000
            );
        }
    }
    for row in rows
        .iter()
        .filter(|r| r["kind"] == "modifier_reconstruction")
    {
        let data = &row["data"];
        assert_eq!(data["before"]["word_cursor"], data["word_cursor_after"]);
        let sources = data["ordered_sources"]
            .as_array()
            .context("ordered source array")?;
        let sequences = sources
            .iter()
            .map(|s| {
                s.get(0)
                    .and_then(serde_json::Value::as_u64)
                    .context("install sequence")
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        assert!(sequences.windows(2).all(|w| w.first() < w.get(1)));
        for resource in ["hp", "mp"] {
            assert!(
                data["after"]["entity"]["combat"][resource]
                    .as_u64()
                    .context("after")?
                    <= data["before"]["entity"]["combat"][resource]
                        .as_u64()
                        .context("before")?
            );
        }
    }
    for row in rows.iter().filter(|r| r["kind"] == "resurrection_refund") {
        let data = &row["data"];
        assert_eq!(
            data["xp_after"]
                .as_u64()
                .context("xp_after")?
                .checked_sub(data["xp_before"].as_u64().context("xp_before")?),
            data["expected_xp_refund"].as_u64()
        );
        if data["accepted"] == true {
            assert!(data["after"]["last_death"].is_null());
        }
    }
    Ok(())
}

#[test]
fn cast_resolution_preserves_full_ordered_source_rng_tail() -> anyhow::Result<()> {
    let (rec, bytes) = fixture("phase3-gameplay-v5.nfr")?;
    let rows = rows(&collect(&rec, &bytes)?)?;
    let mut magic = 0;
    for row in rows.iter().filter(|r| r["kind"] == "formula_resolution") {
        let draws = row["data"]["rng_draws"].as_array().context("draws")?;
        let sites: Vec<_> = draws
            .iter()
            .filter_map(|r| r["callsite"].as_str())
            .collect();
        if sites.first() == Some(&"MagicCritical") {
            magic += 1;
            assert_eq!(sites.get(1), Some(&"MagicSuccess"));
            assert!(sites.contains(&"Spread"));
            // Positive magic damage retains the zero-chance reflection draw.
            let positive = row["data"]["outcomes"]
                .as_array()
                .context("outcomes")?
                .iter()
                .any(|r| r["rolled"].as_u64().is_some_and(|v| v > 0));
            if positive {
                assert!(sites.contains(&"MagicVengeance"));
            }
        }
    }
    assert!(magic > 0);
    Ok(())
}

#[test]
fn digest_only_records_produce_identical_raw_facts() -> anyhow::Result<()> {
    let (mut rec, bytes) = fixture("phase3-casts-v5.nfr")?;
    let full = rows(&collect(&rec, &bytes)?)?;
    rec.records = rec
        .records
        .iter()
        .map(AppliedTickRecord::with_output_digests)
        .collect();
    let digests = rows(&collect(&rec, &rec.to_bytes()?)?)?;
    assert_eq!(full.get(1..), digests.get(1..));
    Ok(())
}

#[test]
fn divergence_and_inconsistent_watermark_fail_closed() -> anyhow::Result<()> {
    let (mut rec, bytes) = fixture("phase3-casts-v5.nfr")?;
    let first = rec.records.first_mut().context("record")?;
    first.server_time_ms = first.server_time_ms.saturating_add(1);
    assert!(collect(&rec, &bytes).is_err());
    let (mut rec, bytes) = fixture("phase3-casts-v5.nfr")?;
    rec.watermark.records = rec.watermark.records.saturating_add(1);
    assert!(collect(&rec, &bytes).is_err());
    let (mut rec, bytes) = fixture("phase3-casts-v5.nfr")?;
    rec.records.remove(1);
    rec.watermark.records = u64::try_from(rec.records.len())?;
    assert!(collect(&rec, &bytes).is_err());
    Ok(())
}

#[test]
fn legacy_replay_does_not_invent_phase3_facts() -> anyhow::Result<()> {
    for name in ["two-players-v4.nfr", "two-players-fight-v2.nfr"] {
        let (rec, bytes) = fixture(name)?;
        let rows = rows(&collect(&rec, &bytes)?)?;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.last().context("completion")?["facts"], 0);
    }
    Ok(())
}

#[test]
fn recorded_admission_and_learning_emit_actual_restored_state_and_frozen_commit(
) -> anyhow::Result<()> {
    use nightfall_api::application::replay_log::{Watermark, WatermarkReason};
    use nightfall_api::domain::character_progression::{
        CharacterAppearance, CharacterIdentity, ClassState,
    };
    use nightfall_api::domain::class::ClassId;
    use nightfall_api::domain::zone::*;
    use nightfall_api::domain::{AccountId, Race};
    use nightfall_api::infrastructure::{
        class_data::{load_classes, ClassSource},
        rules_data::{load_rules, RulesSource},
        skill_data::{load_phase3_rules, Phase3DataSource},
    };
    let entity = EntityId::from_uuid(uuid::Uuid::from_u128(1));
    let account = AccountId::from_uuid(uuid::Uuid::from_u128(2));
    let rules = load_rules(&RulesSource::embedded())?.rules;
    let mut load = PlayerLoad::fresh("human_fighter");
    load.level = 10;
    load.xp = rules.xp_to_level(load.level)?;
    let mut class_state = ClassState::new(ClassId(0));
    class_state.sp = 2000;
    load.progression = Some(Box::new(PlayerProgression {
        identity: CharacterIdentity {
            account_id: account,
            race: Race::Human,
            base_class_id: ClassId(0),
            appearance: CharacterAppearance::default(),
        },
        class_state,
        max_cp: 0,
    }));
    let mut state = ZoneState::new_phase3(
        ZoneSeed {
            zone: ZoneId(99),
            epoch: 7,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256))?,
        0,
        rules,
        load_classes(&ClassSource::embedded())?.registry,
        load_phase3_rules(&Phase3DataSource::embedded(), &[])?,
    )?;
    let snapshot = state.snapshot();
    let mut records = Vec::new();
    for input in [
        ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity,
            name: "Learner".into(),
            pos: Vec2Fixed::from_tiles(126, 128),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(1),
            load: Some(Box::new(load)),
        }),
        ZoneInput::session(
            entity,
            SessionGeneration(1),
            1,
            ZoneCommand::LearnSkill {
                entity,
                account,
                request_key: uuid::Uuid::from_u128(3),
                skill_id: skill::SkillId(141),
                skill_level: 1,
            },
        ),
    ] {
        let applied = state.run_tick(state.draft(vec![input]))?;
        assert!(applied.dispositions.is_empty());
        records.push(AppliedTickRecord::from_applied(snapshot.seed.zone, &applied));
    }
    let rec = Recording {
        watermark: Watermark {
            zone: snapshot.seed.zone,
            epoch: snapshot.seed.epoch,
            last_tick: records.last().map(|r| r.tick),
            records: u64::try_from(records.len())?,
            reason: WatermarkReason::Capture,
        },
        snapshot,
        records,
    };
    let rows = rows(&collect(&rec, &rec.to_bytes()?)?)?;
    let admission = rows
        .iter()
        .find(|r| r["kind"] == "admission_reconstruction")
        .context("admission")?;
    assert_eq!(admission["entity"], entity.to_string());
    let learns: Vec<_> = rows
        .iter()
        .filter(|r| r["kind"] == "learning_commit")
        .collect();
    assert_eq!(learns.len(), 1);
    let event = rec.records.last().context("learn record")?;
    let learned = nightfall_api::application::replay_log::decode_events(&event.events)?;
    let actual = learned
        .iter()
        .find_map(|e| {
            if let ZoneEvent::SkillLearned { result, .. } = e {
                Some(result)
            } else {
                None
            }
        })
        .context("frozen result")?;
    assert_eq!(
        learns.first().context("learning fact")?["data"]["result"],
        serde_json::to_value(actual)?
    );
    Ok(())
}

/// T17/D114: V6 recordings carry the Phase 3 `observer_combat` facts the C11 gates read.
/// Fixture: a V6 (snapshot 10, `BinaryV6`) Prophet self-cast of 1388 through completion,
/// written by the lib test `v6_learned_item_cost_is_atomic_paid_once_and_not_refunded_on_cancel`
/// (`NIGHTFALL_TEST_CAST_COST_EVIDENCE_RECORDING`, embedded rules).
#[test]
fn v6_recordings_export_observer_combat_facts() -> anyhow::Result<()> {
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/phase3-v6/v6-cast-observers.nfr"),
    )?;
    let rec = Recording::from_bytes(&bytes)?;
    assert_eq!(
        rec.snapshot.meta.digest_version,
        nightfall_api::domain::zone::StateDigestVersion::BinaryV6
    );
    let first = collect(&rec, &bytes)?;
    assert_eq!(first, collect(&rec, &bytes)?);
    let rows = rows(&first)?;
    let observed: Vec<_> = rows
        .iter()
        .filter(|row| row["kind"] == "observer_combat")
        .collect();
    assert!(!observed.is_empty(), "V6 export lacks observer_combat");
    let events: BTreeSet<_> = observed
        .iter()
        .flat_map(|row| row["data"]["events"].as_array().into_iter().flatten())
        .filter_map(|event| event["type"].as_str())
        .collect();
    for kind in ["skill_cast_started", "skill_cast_result"] {
        assert!(events.contains(kind), "missing {kind}: {events:?}");
    }
    for row in &observed {
        assert!(row["entity"].is_string() && row["tick"].is_u64(), "{row}");
    }
    Ok(())
}
