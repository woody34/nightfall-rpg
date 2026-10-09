//! Offline trace CLI contracts over the committed recordings.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use std::collections::BTreeSet;
use std::process::Command;

use nightfall_api::application::replay_log::decode_events;
use nightfall_api::domain::zone::ZoneEvent;
use nightfall_api::infrastructure::eventlog::Recording;

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/sessions")
        .join(name)
}

fn page(name: &str, tick: u64) -> (String, Recording) {
    let input = fixture(name);
    let rec = Recording::read(&input).unwrap();
    let output =
        std::env::temp_dir().join(format!("trace-{}-{name}-{tick}.html", std::process::id()));
    let session = rec
        .records
        .iter()
        .flat_map(|r| &r.outputs)
        .next()
        .unwrap()
        .entity;
    let result = Command::new(env!("CARGO_BIN_EXE_nightfall-replay"))
        .args(["trace", "--file"])
        .arg(input)
        .args([
            "--session",
            &session.to_string(),
            "--fail-tick",
            &tick.to_string(),
            "--out",
        ])
        .arg(&output)
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let html = std::fs::read_to_string(&output).unwrap();
    std::fs::remove_file(output).unwrap();
    let has_own = rec
        .records
        .iter()
        .any(|r| r.tick.0.abs_diff(tick) <= 20 && r.outputs.iter().any(|o| o.entity == session));
    assert_eq!(html.contains("class=\"own\""), has_own);
    (html, rec)
}

#[test]
fn both_fixtures_have_complete_entity_and_event_counts_and_bounded_pages() {
    for name in ["two-players-v4.nfr", "two-players-fight-v2.nfr"] {
        let rec = Recording::read(&fixture(name)).unwrap();
        let tick = rec.records[rec.records.len() / 2].tick.0;
        let (html, rec) = page(name, tick);
        let facts: Vec<_> = rec
            .records
            .iter()
            .flat_map(|r| decode_events(&r.events).unwrap())
            .collect();
        let entities: BTreeSet<_> = rec
            .snapshot
            .entities
            .iter()
            .map(|e| e.id)
            .chain(facts.iter().filter_map(|e| {
                if let ZoneEvent::EntitySpawn { entity, .. } = e {
                    Some(*entity)
                } else {
                    None
                }
            }))
            .collect();
        assert_eq!(entities.len(), 7);
        assert!(html.contains(&format!("{} entities", entities.len())));
        for (kind, count) in [
            (
                "death",
                facts
                    .iter()
                    .filter(|e| matches!(e, ZoneEvent::EntityDied { .. }))
                    .count(),
            ),
            (
                "respawn",
                facts
                    .iter()
                    .filter(|e| matches!(e, ZoneEvent::EntityRespawned { .. }))
                    .count(),
            ),
            (
                "intention",
                facts
                    .iter()
                    .filter(|e| matches!(e, ZoneEvent::NpcIntentionChanged { .. }))
                    .count(),
            ),
            (
                "target",
                facts
                    .iter()
                    .filter(|e| matches!(e, ZoneEvent::TargetChanged { .. }))
                    .count(),
            ),
            ("rejection", rec.records.iter().map(|r| r.dispositions.len()).sum()),
        ] {
            assert_eq!(
                html.matches(&format!("data-kind=\"{kind}\"")).count(),
                count,
                "{name}: {kind}"
            );
        }
        for outcome in ["Hit", "Miss", "Crit"] {
            let count = facts.iter().filter(|e| matches!(e, ZoneEvent::AttackResult {outcome: o,..} if format!("{o:?}")==outcome)).count();
            assert_eq!(
                html.matches(&format!("data-kind=\"{}\"", outcome.to_lowercase()))
                    .count(),
                count
            );
        }
        assert_eq!(html.matches("<tr id=\"tick-").count(), 41);
        for t in tick.saturating_sub(20)..=tick.saturating_add(20) {
            assert!(html.contains(&format!("id=\"tick-{t}\"")));
        }
        assert!(html.contains(&format!("id=\"tick-{tick}\" class=\"failure\"")));
        assert!(!html.contains(&format!("id=\"tick-{}\"", tick.saturating_sub(21))));
        assert!(html.len() < 2_000_000, "{name}: {} bytes", html.len());
        assert!(html.len() < 5_000_000);
        assert!(!html.contains("<script src="));
        assert!(!html.contains("<link "));
        assert!(!html.contains("@@"));
    }
}

#[test]
fn failure_windows_clip_to_recording_edges() {
    let name = "two-players-v4.nfr";
    let rec = Recording::read(&fixture(name)).unwrap();
    for tick in [rec.records[0].tick.0, rec.records.last().unwrap().tick.0] {
        let (html, _) = page(name, tick);
        assert_eq!(html.matches("<tr id=\"tick-").count(), 21);
    }
}

#[test]
fn trace_rejects_missing_output_and_out_of_range_failure() {
    for flags in [
        vec![],
        vec![
            "--out",
            "/tmp/invalid-trace.html",
            "--fail-tick",
            "18446744073709551615",
        ],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_nightfall-replay"))
            .args(["trace", "--file"])
            .arg(fixture("two-players-v4.nfr"))
            .args(flags)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2));
    }
}

#[test]
fn active_window_decodes_and_highlights_observer_messages() {
    let name = "two-players-fight-v2.nfr";
    let rec = Recording::read(&fixture(name)).unwrap();
    let record = rec.records.iter().find(|r| !r.outputs.is_empty()).unwrap();
    let (html, _) = page(name, record.tick.0);
    assert!(html.contains("class=\"own\""));
    assert!(html.contains("EntitySpawn"));
}
