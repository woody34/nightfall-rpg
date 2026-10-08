//! In-memory [`SessionAudit`]: keeps every frame, per session, in order. Tests and dev.

use std::collections::BTreeMap;

use bytes::Bytes;
use parking_lot::Mutex;

use crate::application::SessionAudit;
use crate::domain::SessionId;

/// One recorded frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditedFrame {
    /// Client to server, with its decoded seq if it had one.
    In {
        /// `ClientMessage.seq`, `None` if undecodable.
        seq: Option<u32>,
        /// The frame as received.
        frame: Bytes,
    },
    /// Server to client.
    Out {
        /// The frame as queued for the socket.
        frame: Bytes,
    },
}

/// Map-backed audit log. Unbounded: only for tests and short dev runs.
#[derive(Debug, Default)]
pub struct InMemorySessionAudit {
    sessions: Mutex<BTreeMap<SessionId, Vec<AuditedFrame>>>,
}

impl InMemorySessionAudit {
    /// Every frame of `session`, inbound and outbound interleaved in recording order.
    #[must_use]
    pub fn frames(&self, session: SessionId) -> Vec<AuditedFrame> {
        self.sessions
            .lock()
            .get(&session)
            .cloned()
            .unwrap_or_default()
    }

    /// Sessions seen so far, in id (creation) order.
    #[must_use]
    pub fn sessions(&self) -> Vec<SessionId> {
        self.sessions.lock().keys().copied().collect()
    }

    fn push(&self, session: SessionId, frame: AuditedFrame) {
        self.sessions.lock().entry(session).or_default().push(frame);
    }
}

impl SessionAudit for InMemorySessionAudit {
    fn record_in(&self, session: SessionId, seq: Option<u32>, frame: &Bytes) {
        self.push(
            session,
            AuditedFrame::In {
                seq,
                frame: frame.clone(),
            },
        );
    }

    fn record_out(&self, session: SessionId, frame: &Bytes) {
        self.push(
            session,
            AuditedFrame::Out {
                frame: frame.clone(),
            },
        );
    }
}

/// Records nothing. What the server runs with until Story 3.2's `JetStream` adapter exists:
/// keeping every frame in memory would grow without bound.
#[derive(Debug, Default, Clone, Copy)]
pub struct DiscardSessionAudit;

impl SessionAudit for DiscardSessionAudit {
    fn record_in(&self, _session: SessionId, _seq: Option<u32>, _frame: &Bytes) {}

    fn record_out(&self, _session: SessionId, _frame: &Bytes) {}
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_kept_per_session_in_order() {
        let audit = InMemorySessionAudit::default();
        let (a, b) = (SessionId::new(), SessionId::new());
        audit.record_in(a, Some(1), &Bytes::from_static(b"in"));
        audit.record_out(b, &Bytes::from_static(b"other"));
        audit.record_out(a, &Bytes::from_static(b"out"));
        assert_eq!(
            audit.frames(a),
            vec![
                AuditedFrame::In {
                    seq: Some(1),
                    frame: Bytes::from_static(b"in")
                },
                AuditedFrame::Out {
                    frame: Bytes::from_static(b"out")
                },
            ]
        );
        let mut both = vec![a, b];
        both.sort();
        assert_eq!(audit.sessions(), both);
    }
}
