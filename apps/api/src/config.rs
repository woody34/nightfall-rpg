//! Process configuration, read once from the environment at startup.

use std::net::SocketAddr;

/// Everything `main` needs to wire the process.
#[derive(Debug, Clone)]
pub struct Config {
    /// axum listener (REST, WebSocket).
    pub http_addr: SocketAddr,
    /// tonic listener.
    pub grpc_addr: SocketAddr,
    /// Postgres URL. `None` selects in-memory persistence (dev only).
    pub database_url: Option<String>,
    /// NATS URL. `None` selects the in-memory bus (dev only).
    pub nats_url: Option<String>,
}

impl Config {
    /// Reads `HTTP_ADDR`, `GRPC_ADDR`, `DATABASE_URL`, `NATS_URL`.
    pub fn from_env() -> anyhow::Result<Self> {
        let http_addr = std::env::var("HTTP_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:3000".into())
            .parse()?;
        let grpc_addr = std::env::var("GRPC_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:50051".into())
            .parse()?;
        Ok(Self {
            http_addr,
            grpc_addr,
            database_url: std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty()),
            nats_url: std::env::var("NATS_URL").ok().filter(|s| !s.is_empty()),
        })
    }
}
