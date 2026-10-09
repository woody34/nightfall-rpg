//! WebSocket helpers: seed a player, get a play ticket through gRPC, connect to `/ws` with a
//! real `tokio-tungstenite` client, and read `ServerMessage`s.

#![allow(
    dead_code,
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects
)]

use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use nightfall_api::domain::{AccountId, Character, CharacterId, CharacterName, Position, Race};
use nightfall_api::interface::grpc::pb::{
    self, client_message, server_message, world_event, ClientMessage, IssuePlayTicketRequest,
    ServerMessage,
};
use prost::Message as _;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

use super::TestApp;

/// How long a test waits for any one expected frame.
pub const WAIT: Duration = Duration::from_secs(5);

/// A character owned by its own fresh account.
#[derive(Debug, Clone)]
pub struct Player {
    pub account: Uuid,
    pub character: CharacterId,
    pub name: String,
}

impl Player {
    /// The entity id on the wire (`EntitySpawn.entity_id`).
    pub fn entity_id(&self) -> String {
        self.character.to_string()
    }
}

/// Seeds a character at tile `(x, y)` for a new account.
pub fn seed_player(app: &TestApp, name: &str, x: f32, y: f32) -> Player {
    let account = Uuid::now_v7();
    let mut c = Character::create(
        AccountId::from_uuid(account),
        CharacterName::new(name).unwrap(),
        Race::Human,
    );
    c.position = Position { x, y };
    app.characters.insert_for_test(c.clone());
    Player {
        account,
        character: c.id,
        name: name.to_owned(),
    }
}

/// A fresh play ticket for `p` (new idempotency key every call).
pub async fn ticket(app: &TestApp, p: &Player) -> String {
    app.session_as(p.account)
        .issue_play_ticket(IssuePlayTicketRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            character_id: p.character.to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .ticket
}

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Opens `/ws` with `authorization` (if any). `Err` carries the HTTP status of a refused
/// handshake.
pub async fn connect_with(app: &TestApp, authorization: Option<&str>) -> Result<Ws, u16> {
    let mut req = app.ws_url.as_str().into_client_request().unwrap();
    if let Some(a) = authorization {
        req.headers_mut()
            .insert("authorization", a.parse().unwrap());
    }
    match tokio_tungstenite::connect_async(req).await {
        Ok((socket, _)) => Ok(Ws { socket }),
        Err(tungstenite::Error::Http(res)) => Err(res.status().as_u16()),
        Err(e) => panic!("handshake failed without an HTTP status: {e}"),
    }
}

/// Opens `/ws` with `Authorization: Bearer <ticket>`.
pub async fn connect(app: &TestApp, ticket: &str) -> Result<Ws, u16> {
    connect_with(app, Some(&format!("Bearer {ticket}"))).await
}

/// Seeds nothing: issues a ticket for `p` and connects, panicking on refusal.
pub async fn join(app: &TestApp, p: &Player) -> Ws {
    let t = ticket(app, p).await;
    connect(app, &t).await.expect("handshake refused")
}

/// What arrived on a socket.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)] // test helper; one value at a time
pub enum Received {
    Message(ServerMessage, Vec<u8>),
    Closed(Option<u16>),
}

pub struct Ws {
    pub socket: Socket,
}

impl Ws {
    pub async fn send(&mut self, msg: &ClientMessage) {
        self.send_raw(msg.encode_to_vec()).await;
    }

    pub async fn send_raw(&mut self, bytes: Vec<u8>) {
        self.socket
            .send(Message::Binary(bytes.into()))
            .await
            .unwrap();
    }

    /// Next data frame or close, ignoring pings. `None` on timeout.
    pub async fn recv(&mut self, wait: Duration) -> Option<Received> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let next = tokio::time::timeout_at(deadline, self.socket.next())
                .await
                .ok()?;
            match next {
                Some(Ok(Message::Binary(b))) => {
                    let msg = ServerMessage::decode(b.as_ref()).expect("not a ServerMessage");
                    return Some(Received::Message(msg, b.to_vec()));
                },
                Some(Ok(Message::Close(frame))) => {
                    return Some(Received::Closed(frame.map(|f| u16::from(f.code))));
                },
                Some(Ok(_)) => {},
                Some(Err(_)) | None => return Some(Received::Closed(None)),
            }
        }
    }

    /// Next `ServerMessage`; panics on close or timeout.
    pub async fn next_msg(&mut self) -> ServerMessage {
        match self.recv(WAIT).await {
            Some(Received::Message(m, _)) => m,
            other => panic!("expected a message, got {other:?}"),
        }
    }

    /// Reads until `pred` matches, returning everything read (the match last).
    pub async fn until(
        &mut self,
        mut pred: impl FnMut(&ServerMessage) -> bool,
    ) -> Vec<ServerMessage> {
        let mut seen = Vec::new();
        loop {
            match self.recv(WAIT).await {
                Some(Received::Message(m, _)) => {
                    let done = pred(&m);
                    seen.push(m);
                    if done {
                        return seen;
                    }
                },
                other => panic!("expected a matching message, got {other:?} after {seen:?}"),
            }
        }
    }

    /// Reads (discarding messages) until the server closes; returns the close code.
    pub async fn closed(&mut self, wait: Duration) -> Option<u16> {
        loop {
            match self.recv(wait).await {
                Some(Received::Message(..)) => {},
                Some(Received::Closed(code)) => return code,
                None => panic!("socket still open after {wait:?}"),
            }
        }
    }

    /// Everything that arrives within `wait` (stops early on close).
    pub async fn drain(&mut self, wait: Duration) -> Vec<ServerMessage> {
        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + wait;
        while let Some(Received::Message(m, _)) = self
            .recv(deadline.saturating_duration_since(tokio::time::Instant::now()))
            .await
        {
            seen.push(m);
        }
        seen
    }

    pub async fn close(mut self) {
        let _ = self
            .socket
            .close(Some(tungstenite::protocol::CloseFrame {
                code: CloseCode::Normal,
                reason: "bye".into(),
            }))
            .await;
    }
}

pub fn move_to(seq: u32, x: f32, y: f32) -> ClientMessage {
    ClientMessage {
        seq,
        intent: Some(client_message::Intent::MoveTo(pb::MoveToRequest {
            destination: Some(pb::Position { x, y }),
        })),
    }
}

pub fn stop_move(seq: u32) -> ClientMessage {
    ClientMessage {
        seq,
        intent: Some(client_message::Intent::StopMove(pb::StopMoveRequest {})),
    }
}

pub fn ack(m: &ServerMessage) -> Option<&pb::Ack> {
    match &m.payload {
        Some(server_message::Payload::Ack(a)) => Some(a),
        _ => None,
    }
}

pub fn rejected(m: &ServerMessage) -> Option<&pb::IntentRejected> {
    match &m.payload {
        Some(server_message::Payload::Rejected(r)) => Some(r),
        _ => None,
    }
}

pub fn spawn_of(m: &ServerMessage) -> Option<&pb::EntitySpawn> {
    match &m.payload {
        Some(server_message::Payload::Event(pb::WorldEvent {
            event: Some(world_event::Event::Spawn(s)),
        })) => Some(s),
        _ => None,
    }
}

pub fn move_of(m: &ServerMessage) -> Option<&pb::EntityMove> {
    match &m.payload {
        Some(server_message::Payload::Event(pb::WorldEvent {
            event: Some(world_event::Event::Move(mv)),
        })) => Some(mv),
        _ => None,
    }
}

pub fn despawn_of(m: &ServerMessage) -> Option<&pb::EntityDespawn> {
    match &m.payload {
        Some(server_message::Payload::Event(pb::WorldEvent {
            event: Some(world_event::Event::Despawn(d)),
        })) => Some(d),
        _ => None,
    }
}

/// Entity ids that appear in any event of `msgs`.
pub fn entities_in(msgs: &[ServerMessage]) -> Vec<String> {
    let mut ids: Vec<String> = msgs
        .iter()
        .filter_map(|m| {
            spawn_of(m)
                .map(|s| s.entity_id.clone())
                .or_else(|| move_of(m).map(|mv| mv.entity_id.clone()))
                .or_else(|| despawn_of(m).map(|d| d.entity_id.clone()))
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}
