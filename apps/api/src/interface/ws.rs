//! `GET /ws`: the real-time channel's handshake (Story 4.1) and its wire adapters.
//!
//! Everything that can be refused is refused **before** the upgrade, as an HTTP status (plan
//! §8 #17): not a WebSocket request (400/426 from axum), too many sessions from this IP (429),
//! missing or bad play ticket (401), superseded ticket (409), character gone (404). Only an
//! admitted socket is upgraded; from then on errors are close codes, decided by
//! `application::session`.
//!
//! The ticket is read from `Authorization: Bearer <ticket>` (§8 #7). It is never logged and
//! never put on a span: the HTTP span records the path only.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt as _, StreamExt as _};
use parking_lot::Mutex;
use prost::Message as _;

use super::grpc::pb;
use super::zone_mapping::{command_from_pb, encode_observer_outputs, MappingError};
use crate::application::session::{
    run_session, DecodedIntent, FrameSink, FrameSource, InboundFrame, OutboundFrame, SessionCodec,
    SessionContext, SessionReject, SessionStart, SinkClosed, Undecodable,
};
use crate::application::use_cases::{ConsumePlayTicket, GetCharacter};
use crate::domain::zone::{EntityId, ObserverOutput};
use crate::domain::SessionId;

/// Inbound messages larger than this break the connection inside the WebSocket codec. It is
/// well above the 4096-byte protocol limit so that oversize frames up to it reach the session
/// and get a proper `IntentRejected{INVALID}`.
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// What the `/ws` route needs.
#[derive(Clone)]
pub struct WsState {
    consume: Arc<ConsumePlayTicket>,
    characters: Arc<GetCharacter>,
    sessions: Arc<SessionContext>,
    ips: IpLimiter,
}

impl WsState {
    /// Builds the route state.
    #[must_use]
    pub fn new(
        consume: ConsumePlayTicket,
        characters: GetCharacter,
        sessions: Arc<SessionContext>,
    ) -> Self {
        let ips = IpLimiter::new(sessions.limits.max_sessions_per_ip);
        Self {
            consume: Arc::new(consume),
            characters: Arc::new(characters),
            sessions,
            ips,
        }
    }
}

/// The `/ws` route. The router must be served with
/// `into_make_service_with_connect_info::<SocketAddr>()` (the per-IP limit needs the peer).
pub fn routes(state: WsState) -> Router {
    Router::new().route("/ws", get(upgrade)).with_state(state)
}

async fn upgrade(
    State(st): State<WsState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(permit) = st.ips.try_acquire(peer.ip()) else {
        return (StatusCode::TOO_MANY_REQUESTS, "too many sessions from this address")
            .into_response();
    };
    let Some(ticket) = bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, "play ticket required").into_response();
    };
    let admission = match st.consume.execute(ticket).await {
        Ok(a) => a,
        Err(rejection) => return rejection.into_response(),
    };
    // Ownership is re-checked against the ticket's account, like any other read.
    let character = match st
        .characters
        .execute(admission.account_id, &admission.character_id.to_string())
        .await
    {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let player = match super::zone_mapping::player_spawn(&character) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "stored character position is not a tile position");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        },
    };
    let start = SessionStart {
        id: SessionId::new(),
        generation: crate::application::session::zone_generation(admission.generation),
        player,
    };
    tracing::info!(session_id = %start.id, "play ticket accepted; upgrading");
    let sessions = st.sessions.clone();
    ws.max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            let (sink, stream) = socket.split();
            run_session(sessions, start, AxumSource(stream), AxumSink(sink)).await;
            drop(permit);
        })
}

/// The token after `Bearer ` in `Authorization`, if any.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token.trim())
}

/// Counts live sessions (and upgrades in progress) per client IP.
#[derive(Clone)]
struct IpLimiter {
    max: usize,
    live: Arc<Mutex<HashMap<IpAddr, usize>>>,
}

impl IpLimiter {
    fn new(max: usize) -> Self {
        Self {
            max,
            live: Arc::default(),
        }
    }

    fn try_acquire(&self, ip: IpAddr) -> Option<IpPermit> {
        let mut live = self.live.lock();
        let n = live.entry(ip).or_default();
        if *n >= self.max {
            return None;
        }
        *n = n.saturating_add(1);
        Some(IpPermit {
            ip,
            live: self.live.clone(),
        })
    }
}

/// One slot of an IP's quota, released on drop (refused handshake or session end).
struct IpPermit {
    ip: IpAddr,
    live: Arc<Mutex<HashMap<IpAddr, usize>>>,
}

impl Drop for IpPermit {
    fn drop(&mut self) {
        let mut live = self.live.lock();
        if let Some(n) = live.get_mut(&self.ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                live.remove(&self.ip);
            }
        }
    }
}

/// The reading half of an axum socket.
struct AxumSource(SplitStream<WebSocket>);

impl FrameSource for AxumSource {
    async fn recv(&mut self) -> Option<InboundFrame> {
        match self.0.next().await? {
            Ok(Message::Binary(bytes)) => Some(InboundFrame::Data {
                bytes,
                binary: true,
            }),
            Ok(Message::Text(text)) => Some(InboundFrame::Data {
                bytes: Bytes::copy_from_slice(text.as_bytes()),
                binary: false,
            }),
            Ok(Message::Ping(_) | Message::Pong(_)) => Some(InboundFrame::Control),
            Ok(Message::Close(_)) => Some(InboundFrame::Close),
            Err(e) => {
                tracing::debug!(error = %e, "socket read failed");
                None
            },
        }
    }
}

/// The writing half of an axum socket.
struct AxumSink(SplitSink<WebSocket, Message>);

impl FrameSink for AxumSink {
    async fn send(&mut self, frames: Vec<OutboundFrame>) -> Result<(), SinkClosed> {
        for frame in frames {
            let msg = match frame {
                OutboundFrame::Binary(b) => Message::Binary(b),
                OutboundFrame::Close { code, reason } => Message::Close(Some(CloseFrame {
                    code,
                    reason: reason.into(),
                })),
            };
            self.0.feed(msg).await.map_err(|_| SinkClosed)?;
        }
        self.0.flush().await.map_err(|_| SinkClosed)
    }
}

/// world.proto over prost.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProstCodec;

impl SessionCodec for ProstCodec {
    fn decode(&self, entity: EntityId, frame: &[u8]) -> Result<DecodedIntent, Undecodable> {
        let msg = pb::ClientMessage::decode(frame).map_err(|_| Undecodable)?;
        let command = command_from_pb(entity, msg.intent.as_ref()).map_err(|e| {
            let reason = match e {
                MappingError::InvalidCoordinate => SessionReject::OutOfBounds,
                MappingError::InvalidTarget
                | MappingError::MissingIntent
                | MappingError::MissingDestination => SessionReject::Invalid,
            };
            (reason, mapping_detail(e))
        });
        Ok(DecodedIntent {
            seq: msg.seq,
            command,
        })
    }

    fn rejected(&self, seq: u32, reason: SessionReject, detail: &str) -> Bytes {
        let reason = match reason {
            SessionReject::Invalid => pb::RejectReason::Invalid,
            SessionReject::OutOfBounds => pb::RejectReason::OutOfBounds,
            SessionReject::RateLimited => pb::RejectReason::RateLimited,
            SessionReject::Overloaded => pb::RejectReason::Overloaded,
        };
        let msg = pb::ServerMessage {
            payload: Some(pb::server_message::Payload::Rejected(pb::IntentRejected {
                seq,
                reason: reason.into(),
                detail: detail.to_owned(),
            })),
        };
        Bytes::from(msg.encode_to_vec())
    }

    fn outputs(&self, outputs: &[ObserverOutput], server_time_ms: i64) -> Vec<Bytes> {
        encode_observer_outputs(outputs, server_time_ms)
    }
}

const fn mapping_detail(e: MappingError) -> &'static str {
    match e {
        MappingError::InvalidTarget => "target must be a UUID or empty",
        MappingError::MissingIntent => "intent is required",
        MappingError::MissingDestination => "destination is required",
        MappingError::InvalidCoordinate => "coordinate is not a finite tile position",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use axum::http::HeaderValue;
    use uuid::Uuid;

    use super::*;
    use crate::domain::zone::ZoneCommand;

    fn headers(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_str(v).unwrap());
        h
    }

    #[test]
    fn bearer_takes_the_token_and_ignores_other_schemes() {
        assert_eq!(bearer(&headers("Bearer abc")), Some("abc"));
        assert_eq!(bearer(&headers("bearer abc")), Some("abc"));
        assert_eq!(bearer(&headers("Basic abc")), None);
        assert_eq!(bearer(&headers("Bearer ")), None);
        assert_eq!(bearer(&HeaderMap::new()), None);
    }

    #[test]
    fn ip_limiter_caps_per_address_and_releases_on_drop() {
        let l = IpLimiter::new(2);
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let b: IpAddr = "10.0.0.2".parse().unwrap();
        let p1 = l.try_acquire(a).unwrap();
        let _p2 = l.try_acquire(a).unwrap();
        assert!(l.try_acquire(a).is_none());
        assert!(l.try_acquire(b).is_some(), "other addresses are unaffected");
        drop(p1);
        assert!(l.try_acquire(a).is_some());
    }

    #[test]
    fn codec_decodes_intents_and_maps_bad_ones_to_reasons() {
        let e = EntityId::from_uuid(Uuid::from_u128(5));
        let frame = pb::ClientMessage {
            seq: 3,
            intent: Some(pb::client_message::Intent::StopMove(pb::StopMoveRequest {})),
        }
        .encode_to_vec();
        let d = ProstCodec.decode(e, &frame).unwrap();
        assert_eq!(d.seq, 3);
        assert_eq!(d.command, Ok(ZoneCommand::StopMove { entity: e }));

        let empty = pb::ClientMessage {
            seq: 4,
            intent: None,
        }
        .encode_to_vec();
        let d = ProstCodec.decode(e, &empty).unwrap();
        assert_eq!(d.command, Err((SessionReject::Invalid, "intent is required")));

        let nan = pb::ClientMessage {
            seq: 5,
            intent: Some(pb::client_message::Intent::MoveTo(pb::MoveToRequest {
                destination: Some(pb::Position {
                    x: f32::NAN,
                    y: 0.0,
                }),
            })),
        }
        .encode_to_vec();
        let d = ProstCodec.decode(e, &nan).unwrap();
        assert_eq!(d.command.unwrap_err().0, SessionReject::OutOfBounds);

        assert_eq!(ProstCodec.decode(e, &[0xff, 0xff, 0xff]), Err(Undecodable));
    }

    #[test]
    fn rejections_encode_as_intent_rejected() {
        let b = ProstCodec.rejected(9, SessionReject::RateLimited, "slow down");
        let m = pb::ServerMessage::decode(b.as_ref()).unwrap();
        assert_eq!(
            m.payload,
            Some(pb::server_message::Payload::Rejected(pb::IntentRejected {
                seq: 9,
                reason: pb::RejectReason::RateLimited.into(),
                detail: "slow down".to_owned(),
            }))
        );
    }
}
