//! Load test for the real-time channel (Story 4.4): N sessions random-walking against a running
//! server. Reports the zone's tick duration (p50/p99/max bucket, from the server's
//! `nightfall_tick_duration_seconds` histogram), frames per second both ways, and dropped
//! frames.
//!
//! The server must accept dev tokens and allow N sessions from one address:
//!
//! ```bash
//! AUTH_DEV_TOKENS=1 WS_MAX_SESSIONS_PER_IP=1000 cargo run --release -p nightfall-api --bin nightfall-api
//! cargo run --release -p nightfall-api --example ws_load -- --sessions 200 --seconds 60
//! ```
//!
//! Flags: `--sessions N` (200), `--seconds S` (60), `--http URL` (`http://127.0.0.1:3000`),
//! `--grpc URL` (`http://127.0.0.1:50051`), `--interval-ms MS` between moves per session
//! (1000), `--step TILES` largest random step (24).

#![allow(
    missing_docs,
    clippy::print_stdout, // a CLI report is the point of this binary
    clippy::print_stderr,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines
)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context as _};
use futures_util::{SinkExt as _, StreamExt as _};
use nightfall_api::infrastructure::memory::TestTokenVerifier;
use nightfall_api::interface::grpc::pb::game_service_client::GameServiceClient;
use nightfall_api::interface::grpc::pb::session_service_client::SessionServiceClient;
use nightfall_api::interface::grpc::pb::{
    self, client_message, server_message, world_event, ClientMessage, CreateCharacterRequest,
    IssuePlayTicketRequest, ServerMessage,
};
use prost::Message as _;
use rand_chacha::rand_core::{Rng as _, SeedableRng as _};
use rand_chacha::ChaCha12Rng;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::Message;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use uuid::Uuid;

const ZONE_TILES: f32 = 256.0;

#[derive(Debug, Clone)]
struct Args {
    sessions: usize,
    seconds: u64,
    http: String,
    grpc: String,
    interval_ms: u64,
    step: f32,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut a = Args {
        sessions: 200,
        seconds: 60,
        http: "http://127.0.0.1:3000".into(),
        grpc: "http://127.0.0.1:50051".into(),
        interval_ms: 1000,
        step: 24.0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let value = it.next().with_context(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--sessions" => a.sessions = value.parse()?,
            "--seconds" => a.seconds = value.parse()?,
            "--http" => a.http = value,
            "--grpc" => a.grpc = value,
            "--interval-ms" => a.interval_ms = value.parse()?,
            "--step" => a.step = value.parse()?,
            other => bail!("unknown flag {other}"),
        }
    }
    Ok(a)
}

/// Client-side counters, shared by every session task.
#[derive(Default)]
struct Counters {
    sent: AtomicU64,
    received: AtomicU64,
    acks: AtomicU64,
    rejected: AtomicU64,
    moves: AtomicU64,
}

/// A name of 3-16 ASCII letters, unique per run and session.
fn name(run: &str, i: usize) -> String {
    let mut n = i;
    let mut suffix = String::new();
    for _ in 0..4 {
        suffix.push(char::from(b'a' + (n % 26) as u8));
        n /= 26;
    }
    format!("Ld{run}{suffix}")
}

/// Account, character and play ticket for one session, through gRPC like a real client.
async fn admit(channel: &Channel, run: &str, i: usize) -> anyhow::Result<(String, String)> {
    let account = Uuid::now_v7();
    let bearer: MetadataValue<_> =
        format!("Bearer {}", TestTokenVerifier::token_for(account)).parse()?;
    let auth = move |mut req: tonic::Request<()>| {
        req.metadata_mut().insert("authorization", bearer.clone());
        Ok(req)
    };
    let mut game = GameServiceClient::with_interceptor(channel.clone(), auth.clone());
    let character = game
        .create_character(CreateCharacterRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            name: name(run, i),
            race: pb::Race::Human.into(),
            ..Default::default()
        })
        .await?
        .into_inner();
    let mut session = SessionServiceClient::with_interceptor(channel.clone(), auth);
    let ticket = session
        .issue_play_ticket(IssuePlayTicketRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            character_id: character.id.clone(),
        })
        .await?
        .into_inner();
    Ok((character.id, ticket.ticket))
}

/// One random-walking session until `deadline`. Returns the close code if the server closed it.
/// One session's parameters.
struct Walker {
    args: Args,
    ws_url: String,
    entity: String,
    ticket: String,
    seed: u64,
    deadline: tokio::time::Instant,
    counters: Arc<Counters>,
}

async fn walk(w: Walker) -> anyhow::Result<Option<u16>> {
    let Walker {
        args,
        ws_url,
        entity,
        ticket,
        seed,
        deadline,
        counters,
    } = w;
    let mut req = ws_url.into_client_request()?;
    req.headers_mut()
        .insert("authorization", format!("Bearer {ticket}").parse()?);
    let (socket, _) = tokio_tungstenite::connect_async(req).await?;
    let (mut tx, mut rx) = socket.split();
    let mut rng = ChaCha12Rng::seed_from_u64(seed);
    let mut pos = (0.0_f32, 0.0_f32);
    let mut seq = 0_u32;
    // Stagger the first move so sessions do not all act on the same tick.
    let first = Duration::from_millis(u64::from(rng.next_u32()) % args.interval_ms.max(1));
    let mut next_move = tokio::time::Instant::now() + first;
    loop {
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => {
                let _ = tx.close().await;
                return Ok(None);
            },
            () = tokio::time::sleep_until(next_move) => {
                next_move += Duration::from_millis(args.interval_ms);
                seq += 1;
                let jitter = |r: &mut ChaCha12Rng| {
                    (r.next_u32() as f32 / u32::MAX as f32 * 2.0 - 1.0) * args.step
                };
                let dest = (
                    (pos.0 + jitter(&mut rng)).clamp(0.0, ZONE_TILES),
                    (pos.1 + jitter(&mut rng)).clamp(0.0, ZONE_TILES),
                );
                let msg = ClientMessage {
                    seq,
                    intent: Some(client_message::Intent::MoveTo(pb::MoveToRequest {
                        destination: Some(pb::Position { x: dest.0, y: dest.1 }),
                    })),
                };
                tx.send(Message::Binary(msg.encode_to_vec().into())).await?;
                counters.sent.fetch_add(1, Ordering::Relaxed);
            },
            frame = rx.next() => match frame {
                Some(Ok(Message::Binary(b))) => {
                    counters.received.fetch_add(1, Ordering::Relaxed);
                    match ServerMessage::decode(b.as_ref())?.payload {
                        Some(server_message::Payload::Ack(_)) => {
                            counters.acks.fetch_add(1, Ordering::Relaxed);
                        },
                        Some(server_message::Payload::Rejected(_)) => {
                            counters.rejected.fetch_add(1, Ordering::Relaxed);
                        },
                        Some(server_message::Payload::Event(pb::WorldEvent {
                            event: Some(world_event::Event::Move(m)),
                        })) => {
                            counters.moves.fetch_add(1, Ordering::Relaxed);
                            if m.entity_id == entity {
                                if let Some(p) = m.position {
                                    pos = (p.x, p.y);
                                }
                            }
                        },
                        _ => {},
                    }
                },
                Some(Ok(Message::Close(f))) => return Ok(Some(f.map_or(1005, |f| u16::from(f.code)))),
                Some(Ok(_)) => {},
                Some(Err(e)) => return Err(e.into()),
                None => return Ok(Some(1006)),
            },
        }
    }
}

/// `(le bound, cumulative count)` buckets of a Prometheus histogram, plus its count.
fn histogram(text: &str, name: &str) -> (Vec<(f64, f64)>, f64) {
    let mut buckets = Vec::new();
    let mut count = 0.0;
    for line in text.lines() {
        let Some((series, value)) = line.rsplit_once(' ') else {
            continue;
        };
        let Ok(value) = value.parse::<f64>() else {
            continue;
        };
        if series.starts_with(&format!("{name}_bucket")) {
            let le = series
                .split("le=\"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .map_or(f64::INFINITY, |le| le.parse().unwrap_or(f64::INFINITY));
            buckets.push((le, value));
        } else if series.starts_with(&format!("{name}_count")) {
            count = value;
        }
    }
    buckets.sort_by(|a, b| a.0.total_cmp(&b.0));
    (buckets, count)
}

fn counter(text: &str, name: &str, label: &str) -> f64 {
    text.lines()
        .filter(|l| l.starts_with(name) && l.contains(label))
        .filter_map(|l| l.rsplit(' ').next()?.parse::<f64>().ok())
        .sum()
}

/// `after - before` per bucket.
fn delta(before: &[(f64, f64)], after: &[(f64, f64)]) -> Vec<(f64, f64)> {
    after
        .iter()
        .map(|(le, c)| {
            let b = before
                .iter()
                .find(|(l, _)| l.total_cmp(le).is_eq())
                .map_or(0.0, |(_, c)| *c);
            (*le, c - b)
        })
        .collect()
}

/// Quantile `q` of the delta between two histogram scrapes, linearly interpolated inside the
/// bucket it falls in. Returns `(estimate, bucket upper bound)` in seconds.
fn quantile(before: &[(f64, f64)], after: &[(f64, f64)], q: f64) -> Option<(f64, f64)> {
    let delta = delta(before, after);
    let total = delta.last()?.1;
    if total <= 0.0 {
        return None;
    }
    let target = q * total;
    let mut prev = (0.0, 0.0);
    for (le, cum) in delta {
        if cum >= target {
            if le.is_infinite() {
                return Some((prev.0, le));
            }
            let span = cum - prev.1;
            let frac = if span > 0.0 {
                (target - prev.1) / span
            } else {
                1.0
            };
            return Some((prev.0 + (le - prev.0) * frac, le));
        }
        prev = (le, cum);
    }
    None
}

async fn scrape(http: &reqwest::Client, base: &str) -> anyhow::Result<String> {
    Ok(http
        .get(format!("{base}/metrics"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    let ws_url = format!("{}/ws", args.http.replacen("http", "ws", 1));
    let http = reqwest::Client::new();
    let channel = Channel::from_shared(args.grpc.clone())?.connect().await?;
    let run: String = {
        let mut r = ChaCha12Rng::seed_from_u64(
            u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or_default(),
        );
        (0..4)
            .map(|_| char::from(b'a' + (r.next_u32() % 26) as u8))
            .collect()
    };

    println!("admitting {} sessions (run {run})...", args.sessions);
    let mut admitted = Vec::with_capacity(args.sessions);
    for i in 0..args.sessions {
        admitted.push(
            admit(&channel, &run, i)
                .await
                .with_context(|| format!("session {i}"))?,
        );
    }

    let before = scrape(&http, &args.http).await?;
    let started = Instant::now();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(args.seconds);
    let counters = Arc::new(Counters::default());
    let mut tasks = Vec::with_capacity(args.sessions);
    for (i, (entity, ticket)) in admitted.into_iter().enumerate() {
        tasks.push(tokio::spawn(walk(Walker {
            args: args.clone(),
            ws_url: ws_url.clone(),
            entity,
            ticket,
            seed: i as u64,
            deadline,
            counters: counters.clone(),
        })));
    }
    let mut closes: BTreeMap<String, usize> = BTreeMap::new();
    for t in tasks {
        let key = match t.await? {
            Ok(None) => "ran to the end".to_owned(),
            Ok(Some(code)) => format!("closed by server ({code})"),
            Err(e) => format!("error: {e}"),
        };
        *closes.entry(key).or_default() += 1;
    }
    let elapsed = started.elapsed().as_secs_f64();
    // Let the last ticks land in the histogram.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after = scrape(&http, &args.http).await?;

    let (hb, _) = histogram(&before, "nightfall_tick_duration_seconds");
    let (ha, count_after) = histogram(&after, "nightfall_tick_duration_seconds");
    let (_, count_before) = histogram(&before, "nightfall_tick_duration_seconds");
    let ticks = count_after - count_before;
    let fmt_q = |q: f64| {
        quantile(&hb, &ha, q).map_or_else(
            || "n/a".to_owned(),
            |(est, le)| format!("{:.3} ms (bucket <= {} ms)", est * 1e3, le * 1e3),
        )
    };
    let max_bucket = {
        delta(&hb, &ha)
            .iter()
            .find(|(_, c)| (*c - ticks).abs() < 0.5)
            .map_or_else(|| "n/a".to_owned(), |(le, _)| format!("<= {} ms", le * 1e3))
    };
    let frames = |text: &str, dir: &str| {
        counter(text, "nightfall_ws_frames_total", &format!("direction=\"{dir}\""))
    };
    let dropped = counter(&after, "nightfall_ws_dropped_frames_total", "")
        - counter(&before, "nightfall_ws_dropped_frames_total", "");

    println!();
    println!("ws_load: {} sessions for {elapsed:.1} s", args.sessions);
    println!("  ticks observed         {ticks:.0}");
    println!("  tick p50               {}", fmt_q(0.50));
    println!("  tick p99               {}", fmt_q(0.99));
    println!("  tick max               {max_bucket}");
    println!(
        "  server frames/s        in {:.0}, out {:.0}",
        (frames(&after, "in") - frames(&before, "in")) / elapsed,
        (frames(&after, "out") - frames(&before, "out")) / elapsed
    );
    println!(
        "  client frames/s        sent {:.0}, received {:.0}",
        counters.sent.load(Ordering::Relaxed) as f64 / elapsed,
        counters.received.load(Ordering::Relaxed) as f64 / elapsed
    );
    println!(
        "  client totals          acks {}, rejections {}, entity moves {}",
        counters.acks.load(Ordering::Relaxed),
        counters.rejected.load(Ordering::Relaxed),
        counters.moves.load(Ordering::Relaxed)
    );
    println!("  dropped frames         {dropped:.0}");
    for (outcome, n) in &closes {
        println!("  sessions {outcome:<14}{n}");
    }
    if dropped > 0.0 {
        eprintln!("dropped frames > 0");
    }
    Ok(())
}
