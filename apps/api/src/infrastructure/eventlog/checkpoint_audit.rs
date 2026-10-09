//! Non-blocking checkpoint acknowledgements on the session audit stream.
use std::sync::Arc;

use async_nats::jetstream::{self, message::PublishMessage};
use bytes::Bytes;
use tokio::sync::mpsc;

use crate::application::checkpoint::CheckpointAck;
use crate::application::replay_log::ReplayLogMetrics;
use crate::application::{SessionAudit, SessionAuditContext};
use crate::domain::SessionId;

/// Adds `.checkpoint` records to a session audit adapter. Wire-frame recording remains with
/// the wrapped adapter; save acknowledgements are JSON, never protobuf socket output.
pub struct CheckpointAudit {
    inner: Arc<dyn SessionAudit>,
    tx: mpsc::Sender<(SessionId, CheckpointAck)>,
    metrics: Arc<dyn ReplayLogMetrics>,
}

impl CheckpointAudit {
    /// Starts a bounded writer. The zone bootstrap creates `NF_SESSIONS` before any saves.
    pub fn spawn(
        client: async_nats::Client,
        inner: Arc<dyn SessionAudit>,
        metrics: Arc<dyn ReplayLogMetrics>,
    ) -> Self {
        let (tx, mut rx) = mpsc::channel::<(SessionId, CheckpointAck)>(1024);
        let reporter = metrics.clone();
        tokio::spawn(async move {
            let context = jetstream::new(client);
            while let Some((session, ack)) = rx.recv().await {
                let result = async {
                    let payload = Bytes::from(serde_json::to_vec(&ack)?);
                    context
                        .send_publish(
                            format!("nightfall.session.{session}.checkpoint"),
                            PublishMessage::build()
                                .payload(payload)
                                .message_id(format!("checkpoint:{session}:{}", ack.key)),
                        )
                        .await?
                        .await?;
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                if let Err(e) = result {
                    reporter.audit_dropped(1);
                    tracing::warn!(error = %e, "checkpoint audit acknowledgement dropped");
                }
            }
        });
        Self { inner, tx, metrics }
    }
}
impl SessionAudit for CheckpointAudit {
    fn record_in(&self, session: SessionId, seq: Option<u32>, frame: &Bytes) {
        self.inner.record_in(session, seq, frame);
    }
    fn record_out(&self, session: SessionId, frame: &Bytes) {
        self.inner.record_out(session, frame);
    }
    fn record_in_context(
        &self,
        session: SessionId,
        seq: Option<u32>,
        frame: &Bytes,
        context: SessionAuditContext,
    ) {
        self.inner.record_in_context(session, seq, frame, context);
    }
    fn record_out_context(&self, session: SessionId, frame: &Bytes, context: SessionAuditContext) {
        self.inner.record_out_context(session, frame, context);
    }
    fn record_checkpoint(&self, session: SessionId, ack: &CheckpointAck) {
        self.inner.record_checkpoint(session, ack);
        if self.tx.try_send((session, ack.clone())).is_err() {
            self.metrics.audit_dropped(1);
        }
    }
}
