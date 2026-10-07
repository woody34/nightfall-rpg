//! Event bus over NATS. Core publish for now; `JetStream` once consumers need replay.

use async_trait::async_trait;

use crate::application::EventBus;
use crate::domain::DomainEvent;

/// Publishes domain events as JSON on `nightfall.<aggregate>.<event>` subjects.
#[derive(Clone)]
pub struct NatsEventBus {
    client: async_nats::Client,
}

impl NatsEventBus {
    /// Connects to `url` (e.g. `nats://localhost:4222`).
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let client = async_nats::connect(url).await?;
        tracing::info!(url, "connected to NATS");
        Ok(Self { client })
    }

    /// Wraps an existing client.
    #[must_use]
    pub fn new(client: async_nats::Client) -> Self {
        Self { client }
    }

    /// The underlying client, for subscribers.
    #[must_use]
    pub fn client(&self) -> &async_nats::Client {
        &self.client
    }
}

#[async_trait]
impl EventBus for NatsEventBus {
    async fn publish(&self, event: &DomainEvent) -> anyhow::Result<()> {
        let payload = serde_json::to_vec(event)?;
        self.client.publish(event.subject(), payload.into()).await?;
        Ok(())
    }
}
