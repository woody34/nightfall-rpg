//! Event bus over NATS. Core publish for now; `JetStream` once consumers need replay.

use async_trait::async_trait;

use crate::application::EventBus;
use crate::domain::DomainEvent;

/// `host:port` of each server in a (possibly comma-separated) NATS URL, with userinfo, scheme,
/// path and query dropped, so it is safe to log. Unparseable entries become `<invalid>`.
#[must_use]
pub fn redact_servers(url: &str) -> String {
    url.split(',')
        .map(|part| {
            part.trim()
                .parse::<async_nats::ServerAddr>()
                .map_or_else(|_| "<invalid>".to_owned(), |a| format!("{}:{}", a.host(), a.port()))
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Connects a client from a (possibly comma-separated) NATS URL, taking credentials out of the
/// URL first. `async_nats` logs the server addresses it dials at debug level, `Debug` output
/// included, so a password left in the address would reach any debug-level log; credentials
/// are handed over through `ConnectOptions` instead and the addresses it sees carry none.
pub async fn connect_client(url: &str) -> anyhow::Result<async_nats::Client> {
    let mut servers = Vec::new();
    let mut options = async_nats::ConnectOptions::new();
    let mut have_auth = false;
    for part in url.split(',') {
        let addr: async_nats::ServerAddr = part
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid NATS server address (credentials not shown)"))?;
        if !have_auth {
            match (addr.username(), addr.password()) {
                (Some(user), Some(pass)) => {
                    options = options.user_and_password(user.to_owned(), pass.to_owned());
                    have_auth = true;
                },
                (Some(token), None) => {
                    options = options.token(token.to_owned());
                    have_auth = true;
                },
                _ => {},
            }
        }
        servers.push(
            format!("{}://{}:{}", addr.scheme(), addr.host(), addr.port())
                .parse::<async_nats::ServerAddr>()?,
        );
    }
    Ok(options.connect(servers).await?)
}

/// Publishes domain events as JSON on `nightfall.<aggregate>.<event>` subjects.
#[derive(Clone)]
pub struct NatsEventBus {
    client: async_nats::Client,
}

impl NatsEventBus {
    /// Connects to `url` (e.g. `nats://localhost:4222`).
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let client = connect_client(url).await?;
        tracing::info!(servers = %redact_servers(url), "connected to NATS");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_keeps_only_host_and_port() {
        assert_eq!(redact_servers("nats://user:s3cret@nats.internal:4223"), "nats.internal:4223");
        assert_eq!(redact_servers("nats://tok3n@localhost"), "localhost:4222");
        assert_eq!(redact_servers("nats://a:b@one:1,nats://c:d@two:2"), "one:1,two:2");
        assert_eq!(redact_servers("not a url ://"), "<invalid>");
    }
}
