//! Drives the fixed two-player session recorded as `fixtures/sessions/two-players-v1.nfr`
//! (Story 3.3): both join, walk, one sends an out-of-bounds `MoveTo` (rejected), the other
//! stops mid-walk with `StopMove`, both disconnect. Re-record after a deliberate change to
//! zone behaviour (docs/engineering/architecture.md §2.5, "Replay tool"):
//!
//! ```bash
//! AUTH_DEV_TOKENS=1 cargo run -p nightfall-api --bin nightfall-api     # leave running
//! cargo run -p nightfall-api --example record_session
//! # stop the server with ctrl-c (writes the watermark), then:
//! cargo run -p nightfall-api --bin nightfall-replay -- export --zone 1 --latest \
//!   --out apps/api/fixtures/sessions/two-players-v1.nfr
//! ```
//!
//! Flags: `--http URL` (`http://127.0.0.1:3000`), `--grpc URL` (`http://127.0.0.1:50051`).

#![allow(
    missing_docs,
    clippy::print_stdout, // a CLI report is the point of this binary
    clippy::print_stderr,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation
)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context as _};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt as _, StreamExt as _};
use nightfall_api::infrastructure::memory::TestTokenVerifier;
use nightfall_api::interface::grpc::pb::game_service_client::GameServiceClient;
use nightfall_api::interface::grpc::pb::session_service_client::SessionServiceClient;
use nightfall_api::interface::grpc::pb::{
    self, client_message, server_message, ClientMessage, CreateCharacterRequest,
    IssuePlayTicketRequest, ServerMessage,
};
use prost::Message as _;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use uuid::Uuid;

type Tx = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

#[derive(Default)]
struct Seen {
    frames: AtomicU64,
    acks: AtomicU64,
    rejected: AtomicU64,
}

/// Account, character and play ticket, through gRPC like a real client.
async fn admit(channel: &Channel, name: &str) -> anyhow::Result<(String, String)> {
    let bearer: MetadataValue<_> =
        format!("Bearer {}", TestTokenVerifier::token_for(Uuid::now_v7())).parse()?;
    let auth = move |mut req: tonic::Request<()>| {
        req.metadata_mut().insert("authorization", bearer.clone());
        Ok(req)
    };
    let mut game = GameServiceClient::with_interceptor(channel.clone(), auth.clone());
    let character = game
        .create_character(CreateCharacterRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            name: name.to_owned(),
            race: pb::Race::Human.into(),
            ..Default::default()
        })
        .await?
        .into_inner();
    let ticket = SessionServiceClient::with_interceptor(channel.clone(), auth)
        .issue_play_ticket(IssuePlayTicketRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            character_id: character.id.clone(),
        })
        .await?
        .into_inner();
    Ok((character.id, ticket.ticket))
}

/// Connects and drains the socket in the background, counting what arrives.
async fn connect(ws_url: &str, ticket: &str, seen: Arc<Seen>) -> anyhow::Result<Tx> {
    let mut req = ws_url.into_client_request()?;
    req.headers_mut()
        .insert("authorization", format!("Bearer {ticket}").parse()?);
    let (socket, _) = tokio_tungstenite::connect_async(req).await?;
    let (tx, mut rx) = socket.split();
    tokio::spawn(async move {
        while let Some(Ok(Message::Binary(b))) = rx.next().await {
            seen.frames.fetch_add(1, Ordering::Relaxed);
            match ServerMessage::decode(b.as_ref()).map(|m| m.payload) {
                Ok(Some(server_message::Payload::Ack(_))) => {
                    seen.acks.fetch_add(1, Ordering::Relaxed);
                },
                Ok(Some(server_message::Payload::Rejected(_))) => {
                    seen.rejected.fetch_add(1, Ordering::Relaxed);
                },
                _ => {},
            }
        }
    });
    Ok(tx)
}

async fn move_to(tx: &mut Tx, seq: u32, x: f32, y: f32) -> anyhow::Result<()> {
    send(
        tx,
        seq,
        client_message::Intent::MoveTo(pb::MoveToRequest {
            destination: Some(pb::Position { x, y }),
        }),
    )
    .await
}

async fn stop(tx: &mut Tx, seq: u32) -> anyhow::Result<()> {
    send(tx, seq, client_message::Intent::StopMove(pb::StopMoveRequest {})).await
}

async fn send(tx: &mut Tx, seq: u32, intent: client_message::Intent) -> anyhow::Result<()> {
    let msg = ClientMessage {
        seq,
        intent: Some(intent),
    };
    tx.send(Message::Binary(msg.encode_to_vec().into())).await?;
    Ok(())
}

async fn pause(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut http = "http://127.0.0.1:3000".to_owned();
    let mut grpc = "http://127.0.0.1:50051".to_owned();
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let value = it.next().with_context(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--http" => http = value,
            "--grpc" => grpc = value,
            other => bail!("unknown flag {other}"),
        }
    }
    let ws_url = format!("{}/ws", http.replacen("http", "ws", 1));
    let channel = Channel::from_shared(grpc)?.connect().await?;
    // Names must be unique per database; the suffix keeps re-recording possible.
    let run = &Uuid::now_v7().simple().to_string()[24..];
    let run: String = run
        .chars()
        .map(|c| char::from(b'a' + (c as u8) % 26))
        .collect();
    let (a_id, a_ticket) = admit(&channel, &format!("Ra{run}")).await?;
    let (b_id, b_ticket) = admit(&channel, &format!("Rb{run}")).await?;
    let (sa, sb) = (Arc::new(Seen::default()), Arc::new(Seen::default()));

    // Both spawn at their saved position (0, 0) and see each other.
    let mut a = connect(&ws_url, &a_ticket, sa.clone()).await?;
    pause(300).await;
    let mut b = connect(&ws_url, &b_ticket, sb.clone()).await?;
    pause(300).await;
    move_to(&mut a, 1, 4.0, 3.0).await?;
    pause(200).await;
    move_to(&mut b, 1, 2.0, 6.0).await?;
    pause(400).await;
    move_to(&mut a, 2, 300.0, 10.0).await?; // outside the 256x256 zone: OUT_OF_BOUNDS
    pause(300).await;
    move_to(&mut b, 2, 20.0, 20.0).await?;
    pause(500).await;
    stop(&mut b, 3).await?; // mid-walk
    pause(200).await;
    move_to(&mut a, 3, 1.5, 1.25).await?;
    pause(1500).await;
    a.close().await?;
    pause(300).await;
    b.close().await?;
    pause(300).await;

    for (who, id, s) in [("a", a_id, sa), ("b", b_id, sb)] {
        println!(
            "player {who} {id}: {} frames, {} acks, {} rejected",
            s.frames.load(Ordering::Relaxed),
            s.acks.load(Ordering::Relaxed),
            s.rejected.load(Ordering::Relaxed)
        );
    }
    println!("done; stop the server (ctrl-c) to close the epoch, then export it");
    Ok(())
}
