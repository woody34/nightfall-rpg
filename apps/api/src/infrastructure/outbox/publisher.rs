use async_nats::jetstream::{self, message::PublishMessage, stream};
use async_trait::async_trait;
use bytes::Bytes;

/// Name of the stream that holds domain events.
pub const EVENTS_STREAM: &str = "NF_EVENTS";

/// Where the relay sends staged rows. Returns only once the broker has durably accepted the
/// message, so the relay can mark the row published.
#[async_trait]
pub trait OutboxPublisher: Send + Sync {
    /// Publishes `payload` on `subject`. `msg_id` is the outbox row id; implementations must
    /// pass it to the broker for deduplication so a retry after a crash is harmless.
    async fn publish(&self, subject: &str, msg_id: i64, payload: Bytes) -> anyhow::Result<()>;
}

/// Acknowledged `JetStream` publishing with `Nats-Msg-Id` deduplication.
#[derive(Clone)]
pub struct JetStreamPublisher {
    context: jetstream::Context,
}

impl JetStreamPublisher {
    /// Creates the `NF_EVENTS` stream (subjects `nightfall.>`) if it does not exist.
    pub async fn connect(client: async_nats::Client) -> anyhow::Result<Self> {
        let context = jetstream::new(client);
        context
            .get_or_create_stream(stream::Config {
                name: EVENTS_STREAM.to_owned(),
                subjects: vec!["nightfall.>".to_owned()],
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow::anyhow!("create stream {EVENTS_STREAM}: {e}"))?;
        Ok(Self { context })
    }
}

#[async_trait]
impl OutboxPublisher for JetStreamPublisher {
    async fn publish(&self, subject: &str, msg_id: i64, payload: Bytes) -> anyhow::Result<()> {
        let ack = self
            .context
            .send_publish(
                subject.to_owned(),
                PublishMessage::build()
                    .payload(payload)
                    .message_id(msg_id.to_string()),
            )
            .await?
            .await?;
        if ack.duplicate {
            tracing::debug!(msg_id, "broker deduplicated outbox publish");
        }
        Ok(())
    }
}
