//! Replays a recorded zone epoch and checks every player's output byte for byte (Story 3.3,
//! architecture.md §2.5 "Replay tool").
//!
//! ```text
//! nightfall-replay [verify] --zone ID (--epoch N | --latest) [--session ENTITY] [--out DIR]
//! nightfall-replay [verify] --source file --file PATH [--session ENTITY] [--out DIR]
//! nightfall-replay export --zone ID (--epoch N | --latest) [--live] [--session ENTITY] --out FILE
//! nightfall-replay trace --file PATH [--session ENTITY] [--fail-tick N] --out trace.html
//! nightfall-replay frames --file PATH --session ENTITY --out FRAMES.jsonl
//! ```
//!
//! `--source jetstream` (default) reads `NATS_URL` (or `--nats URL`). `--session` selects whose
//! outputs are compared; every command is still applied. `--out DIR` (verify) dumps the first
//! divergence there.
//!
//! Exit codes: 0 match, 1 divergence, 2 usage or I/O error, 3 epoch incomplete (no watermark).

#![allow(
    clippy::print_stdout, // a CLI report is the point of this binary
    clippy::print_stderr
)]

#[path = "replay/coverage.rs"]
mod coverage;
#[path = "replay/evidence.rs"]
mod evidence;
#[path = "replay/frames.rs"]
mod frames;
#[path = "replay/items_evidence.rs"]
mod items_evidence;
#[path = "replay/trace.rs"]
mod trace;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{bail, Context as _};
use nightfall_api::application::replay::{verify_epoch, VerifyError, VerifyOptions};
use nightfall_api::application::replay_log::{
    encode_snapshot, open_epoch, EventLog, RecordedEpoch, ReplayError,
};
use nightfall_api::domain::zone::{EntityId, SnapshotMeta, ZoneId};
use nightfall_api::infrastructure::eventlog::{JetStreamEventLog, Recording};
use nightfall_api::infrastructure::telemetry::Metrics;

const USAGE: &str = "usage:
  nightfall-replay [verify] --zone ID (--epoch N | --latest) [--session ENTITY] [--out DIR] [--nats URL]
  nightfall-replay [verify] --source file --file PATH [--session ENTITY] [--out DIR]
  nightfall-replay export --zone ID (--epoch N | --latest) [--live] [--session ENTITY] --out FILE [--evidence-out FILE] [--nats URL]
  nightfall-replay export --file PATH (--evidence-out FILE | --items-evidence-out FILE)
  nightfall-replay check --file PATH [--session ENTITY] [--out DIR]
  nightfall-replay coverage --file PATH [--out coverage.transitions.json]
  nightfall-replay trace --file PATH [--session ENTITY] [--fail-tick N] --out trace.html
  nightfall-replay frames --file PATH --session ENTITY --out FRAMES.jsonl
exit: 0 match, 1 divergence, 2 error, 3 epoch incomplete";

const MATCH: u8 = 0;
const DIVERGED: u8 = 1;
const ERROR: u8 = 2;
const INCOMPLETE: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Verify,
    Export,
    Coverage,
    Trace,
    Frames,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    JetStream,
    File,
}

#[derive(Debug)]
struct Args {
    command: Command,
    source: Source,
    zone: Option<ZoneId>,
    epoch: Option<u64>,
    latest: bool,
    live: bool,
    session: Option<EntityId>,
    file: Option<PathBuf>,
    out: Option<PathBuf>,
    nats: String,
    fail_tick: Option<u64>,
    evidence_out: Option<PathBuf>,
    items_evidence_out: Option<PathBuf>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> anyhow::Result<Args> {
    let mut it = args.into_iter().peekable();
    let command = match it.peek().map(String::as_str) {
        Some("export") => {
            it.next();
            Command::Export
        },
        Some("trace") => {
            it.next();
            Command::Trace
        },
        Some("coverage") => {
            it.next();
            Command::Coverage
        },
        Some("frames") => {
            it.next();
            Command::Frames
        },
        Some("verify" | "check") => {
            it.next();
            Command::Verify
        },
        _ => Command::Verify,
    };
    let mut a = Args {
        command,
        fail_tick: None,
        evidence_out: None,
        items_evidence_out: None,
        source: Source::JetStream,
        zone: None,
        epoch: None,
        latest: false,
        live: false,
        session: None,
        file: None,
        out: None,
        nats: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_owned()),
    };
    while let Some(flag) = it.next() {
        if flag == "--live" {
            a.live = true;
            continue;
        }
        if flag == "--latest" {
            a.latest = true;
            continue;
        }
        let value = it.next().with_context(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--fail-tick" => a.fail_tick = Some(value.parse().context("--fail-tick")?),
            "--zone" => a.zone = Some(ZoneId(value.parse().context("--zone")?)),
            "--epoch" => a.epoch = Some(value.parse().context("--epoch")?),
            "--session" => a.session = Some(value.parse().context("--session")?),
            "--source" => {
                a.source = match value.as_str() {
                    "jetstream" => Source::JetStream,
                    "file" => Source::File,
                    other => bail!("--source must be jetstream or file, not {other}"),
                };
            },
            "--file" => a.file = Some(value.into()),
            "--evidence-out" => a.evidence_out = Some(value.into()),
            "--items-evidence-out" => a.items_evidence_out = Some(value.into()),
            "--out" => a.out = Some(value.into()),
            "--nats" => a.nats = value,
            other => bail!("unknown flag {other}"),
        }
    }
    if a.epoch.is_some() && a.latest {
        bail!("--epoch and --latest are exclusive");
    }
    if a.file.is_some() {
        a.source = Source::File;
    }
    if a.live && a.command != Command::Export {
        bail!("--live is only valid for export");
    }
    if a.command == Command::Coverage && (a.source != Source::File || a.session.is_some()) {
        bail!("coverage requires --file PATH and counts the whole recording (no --session)");
    }
    if a.fail_tick.is_some() && a.command != Command::Trace {
        bail!("--fail-tick is only valid for trace");
    }
    if a.command == Command::Frames
        && (a.source != Source::File || a.session.is_none() || a.out.is_none())
    {
        bail!("frames requires --file PATH, --session ENTITY and --out FILE");
    }
    if a.command == Command::Trace && (a.file.is_none() || a.out.is_none()) {
        bail!("trace requires --file PATH and --out FILE");
    }
    if a.evidence_out.is_some() && a.items_evidence_out.is_some() {
        bail!("choose one evidence format per export");
    }
    let evidence_path = a.evidence_out.as_ref().or(a.items_evidence_out.as_ref());
    if evidence_path.is_some() && a.command != Command::Export {
        bail!("--evidence-out is only valid for export");
    }
    if evidence_path
        .is_some_and(|path| Some(path) == a.out.as_ref() || Some(path) == a.file.as_ref())
    {
        bail!("evidence destination must differ from recording paths");
    }
    match (a.command, a.source) {
        (Command::Export, Source::File) if evidence_path.is_none() => {
            bail!("file export requires --evidence-out FILE or --items-evidence-out FILE");
        },
        (Command::Export, Source::JetStream) if a.out.is_none() => bail!("export needs --out FILE"),
        (_, Source::File) if a.file.is_none() => bail!("--source file needs --file PATH"),
        (_, Source::JetStream) if a.zone.is_none() => bail!("--zone is required"),
        (_, Source::JetStream) if a.epoch.is_none() && !a.latest => {
            bail!("--epoch N or --latest is required")
        },
        _ => {},
    }
    Ok(a)
}

/// The log to read and the epoch to read from it.
async fn source(a: &Args) -> anyhow::Result<(Box<dyn EventLog>, ZoneId, u64)> {
    if let (Source::File, Some(path)) = (a.source, &a.file) {
        let rec = Recording::read(path)?;
        let (zone, epoch) = (rec.snapshot.seed.zone, rec.snapshot.seed.epoch);
        if a.zone.is_some_and(|z| z != zone) || a.epoch.is_some_and(|e| e != epoch) {
            bail!("{} holds zone {} epoch {epoch}", path.display(), zone.0);
        }
        return Ok((Box::new(rec.into_log().await?), zone, epoch));
    }
    let zone = a.zone.context("--zone is required")?;
    let client = async_nats::connect(&a.nats)
        .await
        .with_context(|| format!("connect to {}", a.nats))?;
    let log = JetStreamEventLog::connect(client, Metrics::detached()).await?;
    let epoch = match a.epoch {
        Some(e) => e,
        None => log
            .latest_epoch(zone)
            .await?
            .with_context(|| format!("zone {} has no epoch in the log", zone.0))?,
    };
    Ok((Box::new(log), zone, epoch))
}

/// Opens the epoch, mapping an incomplete one to its exit code.
async fn open(log: &dyn EventLog, zone: ZoneId, epoch: u64) -> Result<RecordedEpoch, u8> {
    match open_epoch(log, zone, epoch).await {
        Ok(e) => Ok(e),
        Err(ReplayError::Incomplete { last_tick, .. }) => {
            let last = last_tick.map_or_else(|| "none".to_owned(), |t| t.0.to_string());
            eprintln!(
                "zone {} epoch {epoch} is incomplete: no completion watermark (last tick in the \
                 log: {last}). The server is still running or did not shut down cleanly; \
                 replay refuses partial recordings.",
                zone.0
            );
            Err(INCOMPLETE)
        },
        Err(e) => {
            eprintln!("error: {e}");
            Err(ERROR)
        },
    }
}

fn warn_on_provenance(meta: &SnapshotMeta) {
    let ours = SnapshotMeta::legacy();
    if meta.build_id != ours.build_id {
        eprintln!("warning: recorded by build {}, replaying with {}", meta.build_id, ours.build_id);
    }
}

fn dump(dir: &Path, d: &nightfall_api::application::replay::Divergence) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        std::fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))
    };
    write("divergence.txt", d.to_string().as_bytes())?;
    write("recorded-record.pb", &d.recorded.encode())?;
    write("produced-record.pb", &d.produced.encode())?;
    if let Some(o) = d.recorded_output() {
        write("recorded-output.pb", &o.bytes)?;
    }
    if let Some(o) = d.produced_output() {
        write("produced-output.pb", &o.bytes)?;
    }
    if let Some(s) = &d.state_before {
        write("state-before.json", &encode_snapshot(s)?)?;
    }
    Ok(())
}

async fn verify(a: &Args) -> anyhow::Result<u8> {
    let (log, zone, epoch) = source(a).await?;
    let opened = match open(log.as_ref(), zone, epoch).await {
        Ok(e) => e,
        Err(code) => return Ok(code),
    };
    warn_on_provenance(&opened.snapshot.snapshot.meta);
    let records = opened.watermark.records;
    let opts = VerifyOptions {
        session: a.session,
        keep_state_before: a.out.is_some(),
    };
    let started = Instant::now();
    match verify_epoch(opened, opts).await {
        Ok(report) => {
            let elapsed = started.elapsed();
            if let (Some(s), 0) = (a.session, report.players) {
                eprintln!("session {s} has no output in zone {} epoch {epoch}", zone.0);
                return Ok(ERROR);
            }
            println!(
                "zone {} epoch {epoch}: match. {} ticks replayed ({records} recorded), {} players, \
                 {} outputs, {} bytes compared, {} digest-only, in {:.1} ms",
                zone.0,
                report.ticks,
                report.players,
                report.outputs,
                report.bytes_compared,
                report.digest_only,
                elapsed.as_secs_f64() * 1e3,
            );
            Ok(MATCH)
        },
        Err(VerifyError::Diverged(d)) => {
            println!("zone {} epoch {epoch}: {d}", zone.0);
            if let Some(dir) = &a.out {
                dump(dir, &d)?;
                println!("divergence written to {}", dir.display());
            }
            Ok(DIVERGED)
        },
        Err(e) => {
            eprintln!("error: {e}");
            Ok(ERROR)
        },
    }
}

async fn export(a: &Args) -> anyhow::Result<u8> {
    if let Some(path) = a.evidence_out.as_ref().or(a.items_evidence_out.as_ref()) {
        if path.exists() {
            bail!("evidence destination must be fresh: {}", path.display());
        }
    }
    let (rec, bytes) = if let Some(path) = &a.file {
        if a.out.is_some() || a.live {
            bail!("file evidence export does not take --out or --live");
        }
        let (rec, bytes) = Recording::read_with_bytes(path)?;
        if a.zone.is_some_and(|z| z != rec.snapshot.seed.zone)
            || a.epoch.is_some_and(|e| e != rec.snapshot.seed.epoch)
        {
            bail!("file zone/epoch does not match selection");
        }
        (rec, bytes)
    } else {
        let (log, zone, epoch) = source(a).await?;
        let rec = if a.live {
            Recording::export_live(log.as_ref(), zone, epoch).await?
        } else {
            if let Err(code) = open(log.as_ref(), zone, epoch).await {
                return Ok(code);
            }
            Recording::export(log.as_ref(), zone, epoch).await?
        };
        let bytes = rec.to_bytes()?;
        (rec, bytes)
    };
    let seed = rec.snapshot.seed;
    if let Some(session) = a.session {
        let present =
            rec.snapshot.entities.iter().any(|e| {
                e.id == session && e.kind == nightfall_api::domain::zone::EntityKind::Player
            }) || rec
                .records
                .iter()
                .any(|r| r.outputs.iter().any(|o| o.entity == session));
        if !present {
            bail!("session {session} has no output in zone {} epoch {}", seed.zone.0, seed.epoch);
        }
    }
    let evidence = a
        .evidence_out
        .as_ref()
        .map(|_| evidence::collect(&rec, &bytes))
        .transpose()?;
    let item_evidence = a
        .items_evidence_out
        .as_ref()
        .map(|_| items_evidence::collect(&rec, &bytes))
        .transpose()?;
    if let Some(out) = &a.out {
        std::fs::write(out, &bytes)?;
        println!(
            "zone {} epoch {}: {} records written to {}",
            seed.zone.0,
            seed.epoch,
            rec.records.len(),
            out.display()
        );
    }
    if let (Some(path), Some(bytes)) = (&a.evidence_out, evidence) {
        evidence::write(path, &bytes)?;
        println!("verified evidence written to {}", path.display());
    }
    if let (Some(path), Some(bytes)) = (&a.items_evidence_out, item_evidence) {
        evidence::write(path, &bytes)?;
        println!("verified item evidence written to {}", path.display());
    }
    Ok(MATCH)
}

async fn coverage(a: &Args) -> anyhow::Result<u8> {
    let (log, zone, epoch) = source(a).await?;
    let rec = Recording::export(log.as_ref(), zone, epoch).await?;
    let report = coverage::collect(&rec)?;
    let out = a
        .out
        .as_deref()
        .unwrap_or_else(|| Path::new("coverage.transitions.json"));
    std::fs::write(out, serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("write {}", out.display()))?;
    report.print();
    println!("coverage written to {}", out.display());
    Ok(MATCH)
}

async fn trace_page(a: &Args) -> anyhow::Result<u8> {
    let (log, zone, epoch) = source(a).await?;
    let rec = Recording::export(log.as_ref(), zone, epoch).await?;
    let page = trace::render(&rec, a.session, a.fail_tick)?;
    let out = a.out.as_deref().context("trace needs --out FILE")?;
    std::fs::write(out, &page).with_context(|| format!("write {}", out.display()))?;
    println!("trace written to {} ({} bytes)", out.display(), page.len());
    Ok(MATCH)
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n{USAGE}");
            return ExitCode::from(ERROR);
        },
    };
    // V6 epochs recorded over a RULES_DIR overlay reference its composed bundle by hash;
    // the same overlay must be supplied to resolve them (the embedded bundle needs nothing).
    // It is composed lazily, only when a recorded hash misses the embedded bundle.
    if let Some(dir) = std::env::var_os("RULES_DIR").filter(|dir| !dir.is_empty()) {
        match nightfall_api::infrastructure::skill_data::Phase3DataSource::from_dir(
            std::path::Path::new(&dir),
        ) {
            Ok(source) => nightfall_api::infrastructure::item_data::register_phase4_source(source),
            Err(error) => {
                eprintln!("error: RULES_DIR V6 bundle: {error}");
                return ExitCode::from(ERROR);
            },
        }
    }
    let result = match args.command {
        Command::Verify => verify(&args).await,
        Command::Export => export(&args).await,
        Command::Coverage => coverage(&args).await,
        Command::Trace => trace_page(&args).await,
        Command::Frames => match (&args.file, args.session, &args.out) {
            (Some(file), Some(session), Some(out)) => frames::write(file, session, out).await,
            _ => Ok(ERROR),
        },
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(ERROR)
        },
    }
}

#[cfg(test)]
mod test_support {
    use nightfall_api::domain::character_progression::{
        CharacterAppearance, CharacterIdentity, FrozenTransferResult, SuccessfulTransferReceipt,
    };
    use nightfall_api::domain::class::ClassId;
    use nightfall_api::domain::zone::{EntityId, Tick, ZoneEvent};
    use nightfall_api::domain::{AccountId, BaseStats, CharacterId, CharacterName, Race};

    #[allow(clippy::unwrap_used)]
    pub(super) fn transfer_effect(entity: EntityId, tick: Tick) -> ZoneEvent {
        ZoneEvent::ClassTransfer {
            entity,
            tick,
            old_class_id: ClassId(0),
            receipt: Box::new(SuccessfulTransferReceipt {
                key: uuid::Uuid::from_u128(10),
                target_class_id: ClassId(1),
                result: FrozenTransferResult {
                    character_id: CharacterId::from_uuid(entity.as_uuid()),
                    identity: CharacterIdentity {
                        account_id: AccountId::from_uuid(uuid::Uuid::nil()),
                        race: Race::Human,
                        base_class_id: ClassId(0),
                        appearance: CharacterAppearance::default(),
                    },
                    name: CharacterName::new("Owner").unwrap(),
                    current_class_id: ClassId(1),
                    level: 20,
                    xp: 0,
                    sp: 765_432,
                    stats: BaseStats {
                        str: 20,
                        dex: 20,
                        con: 20,
                        int: 20,
                        wit: 20,
                        men: 20,
                    },
                    position_millitiles: [126_000, 126_000],
                    hp: 1,
                    mp: 0,
                    cp: 987_654,
                    max_hp: 2,
                    max_mp: 2,
                    max_cp: 999_999,
                    token_tier_1_count: 17,
                    token_tier_2_count: 19,
                    granted_skill_keys: vec!["PRIVATE_GRANTED_SKILL_SENTINEL".into()],
                },
            }),
        }
    }
}
