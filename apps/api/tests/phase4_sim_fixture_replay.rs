//! T18: a V6 epoch recorded over `ZONE_SIM_FIXTURE` (the phase1a-social-aggro NPC speed
//! override changes the Phase 3 profiles, so the V6 bundle hashes differ from the embedded
//! ones) resolves in a fresh `nightfall-replay` process only from the same configuration,
//! exactly as the API startup composes it. Without it the tool refuses by name.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]
use anyhow::{ensure, Context as _};
use nightfall_api::application::replay_log::{
    restore_snapshot, AppliedTickRecord, Watermark, WatermarkReason,
};
use nightfall_api::infrastructure::eventlog::Recording;
use nightfall_api::infrastructure::item_data::{embedded_phase4_bundle, resolve_phase4_bundle};

fn replay(
    args: &[&std::ffi::OsStr],
    rules_dir: Option<&std::path::Path>,
    fixture: Option<&str>,
) -> std::process::Output {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_nightfall-replay"));
    command
        .args(args)
        .env_remove("RULES_DIR")
        .env_remove("ZONE_SIM_FIXTURE")
        .env_remove("ZONE_FILE");
    if let Some(dir) = rules_dir {
        command.env("RULES_DIR", dir);
    }
    if let Some(name) = fixture {
        command.env("ZONE_SIM_FIXTURE", name);
    }
    command.output().unwrap()
}

#[tokio::test]
async fn a_zone_sim_fixture_v6_epoch_replays_and_exports_with_its_configuration(
) -> anyhow::Result<()> {
    // The harness overlay for this unit (sim-lib.sh sim_unit_prepare: RULES_DIR=phase1a-data,
    // ZONE_SIM_FIXTURE=phase1a-social-aggro, ZONE_FILE unset).
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .context("repository root")?;
    let dir = std::env::temp_dir().join(format!("nightfall-t18-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir(&dir)?;
    let root = dir.join("phase1a-data");
    let generated = std::process::Command::new("python3")
        .arg(repo.join("apps/client-unreal/Scripts/phase3-fixtures.py"))
        .arg(&root)
        .arg(repo)
        .arg("phase1a-social-aggro")
        .status()?;
    ensure!(generated.success(), "phase1a overlay generation failed");
    let config = nightfall_api::zone_runtime::ZoneRuntimeConfig {
        rules_dir: Some(root.clone()),
        zone_file: None,
        simulation_fixture: Some("phase1a-social-aggro".into()),
        auth_dev_tokens: true,
        phase3_enabled: Some("1".into()),
        phase4_enabled: Some("1".into()),
    };
    let mut deps = nightfall_api::Dependencies::in_memory();
    deps.classes = Some(nightfall_api::infrastructure::class_data::load_classes(
        &nightfall_api::infrastructure::class_data::ClassSource::from_dir(&root)?,
    )?);
    let running = nightfall_api::zone_runtime::start(&config, None, None, &deps).await?;
    let snapshot = running
        .handle()
        .snapshot()
        .await
        .map_err(|_| anyhow::anyhow!("zone stopped"))?;
    running.shutdown(WatermarkReason::Shutdown).await?;
    let items = snapshot.phase4.as_ref().context("V6 epoch expected")?;
    let skills = snapshot.phase3.as_ref().context("V5 sidecar")?;
    let embedded = embedded_phase4_bundle().map_err(anyhow::Error::msg)?;
    ensure!(
        skills.rules_hash != embedded.skills.hash() && items.rules_hash != embedded.items.hash(),
        "the fixture override composes its own V6 bundle"
    );
    // The API process that recorded the epoch resolves it (startup installed the composition).
    resolve_phase4_bundle(items.rules_hash, skills.rules_hash).map_err(anyhow::Error::msg)?;
    // A short recorded epoch from that snapshot (the fixture NPCs tick).
    let mut state = restore_snapshot(snapshot.clone())?;
    let mut recording = Recording {
        watermark: Watermark {
            zone: snapshot.seed.zone,
            epoch: snapshot.seed.epoch,
            last_tick: None,
            records: 0,
            reason: WatermarkReason::Capture,
        },
        snapshot,
        records: Vec::new(),
    };
    for _ in 0..20 {
        let applied = state.run_tick(state.draft(Vec::new()))?;
        recording
            .records
            .push(AppliedTickRecord::from_applied(recording.snapshot.seed.zone, &applied));
        recording.watermark.last_tick = Some(applied.tick);
    }
    recording.watermark.records = u64::try_from(recording.records.len())?;
    let file = dir.join("sim-fixture-v6.nfr");
    recording.write(&file)?;
    let result = (|| {
        // A fresh process without the full configuration refuses the epoch by name: neither
        // the embedded bundle nor the plain RULES_DIR overlay (the T18 capture shell before the
        // fix) stands in for the fixture composition.
        for rules_dir in [None, Some(root.as_path())] {
            let refused =
                replay(&["check".as_ref(), "--file".as_ref(), file.as_os_str()], rules_dir, None);
            ensure!(!refused.status.success(), "{refused:?}");
            ensure!(
                String::from_utf8_lossy(&refused.stderr)
                    .contains("recorded V6 rules are unavailable"),
                "named refusal expected: {refused:?}"
            );
        }
        // With the recording configuration it replays to a match and exports.
        let checked = replay(
            &["check".as_ref(), "--file".as_ref(), file.as_os_str()],
            Some(root.as_path()),
            Some("phase1a-social-aggro"),
        );
        ensure!(checked.status.success(), "check: {checked:?}");
        ensure!(String::from_utf8_lossy(&checked.stdout).contains("match"), "{checked:?}");
        let evidence = dir.join("sim-fixture-v6.jsonl");
        let exported = replay(
            &[
                "export".as_ref(),
                "--file".as_ref(),
                file.as_os_str(),
                "--evidence-out".as_ref(),
                evidence.as_os_str(),
            ],
            Some(root.as_path()),
            Some("phase1a-social-aggro"),
        );
        ensure!(exported.status.success(), "export: {exported:?}");
        ensure!(std::fs::metadata(&evidence)?.len() > 0, "evidence written");
        // An unknown fixture name is a configuration error, never a silent default.
        let unknown = replay(
            &["check".as_ref(), "--file".as_ref(), file.as_os_str()],
            Some(root.as_path()),
            Some("other-fixture"),
        );
        ensure!(!unknown.status.success(), "{unknown:?}");
        Ok(())
    })();
    std::fs::remove_dir_all(&dir)?;
    result
}
