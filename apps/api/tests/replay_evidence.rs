//! CLI evidence creation must be fail closed and must not overwrite its recording.
use anyhow::Context as _;
use nightfall_api::infrastructure::eventlog::Recording;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> anyhow::Result<Self> {
        let path = std::env::temp_dir()
            .join(format!("nightfall-phase3-zone-evidence-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn export(input: &Path, out: &Path) -> anyhow::Result<std::process::Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_nightfall-replay"))
        .arg("export")
        .arg("--file")
        .arg(input)
        .arg("--evidence-out")
        .arg(out)
        .output()?)
}

#[test]
fn cli_creates_bound_json_lines_and_refuses_existing_destinations() -> anyhow::Result<()> {
    let work = Workspace::new()?;
    let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/sessions/phase3-casts-v5.nfr");
    let original = std::fs::read(&input)?;
    let out = work.0.join("evidence.jsonl");
    let result = export(&input, &out)?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let evidence = std::fs::read(&out)?;
    let text = std::str::from_utf8(&evidence)?;
    let last: serde_json::Value =
        serde_json::from_str(text.lines().last().context("complete row")?)?;
    assert_eq!(last.get("kind"), Some(&serde_json::json!("complete")));
    assert!(!export(&input, &out)?.status.success());
    assert_eq!(std::fs::read(&out)?, evidence);
    assert!(!export(&input, &input)?.status.success());
    assert_eq!(std::fs::read(&input)?, original);
    Ok(())
}

#[test]
fn cli_divergence_creates_no_evidence_even_after_valid_earlier_ticks() -> anyhow::Result<()> {
    let work = Workspace::new()?;
    let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/sessions/phase3-casts-v5.nfr");
    let mut recording = Recording::read(&input)?;
    let last = recording.records.last_mut().context("last tick")?;
    last.server_time_ms = last.server_time_ms.saturating_add(1);
    let corrupt = work.0.join("corrupt.nfr");
    recording.write(&corrupt)?;
    let out = work.0.join("evidence.jsonl");
    let result = export(&corrupt, &out)?;
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("divergence"));
    assert!(!out.exists());
    Ok(())
}

#[test]
fn evidence_honors_the_existing_recording_input_budget() -> anyhow::Result<()> {
    let work = Workspace::new()?;
    let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/sessions/phase3-casts-v5.nfr");
    let out = work.0.join("evidence.jsonl");
    let result = Command::new(env!("CARGO_BIN_EXE_nightfall-replay"))
        .arg("export")
        .arg("--file")
        .arg(input)
        .arg("--evidence-out")
        .arg(&out)
        .env("NIGHTFALL_RECORDING_MAX_BODY_BYTES", "1")
        .output()?;
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("budget"));
    assert!(!out.exists());
    Ok(())
}
