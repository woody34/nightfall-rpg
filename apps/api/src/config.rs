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
    /// Token issuer, e.g. `http://localhost:8080/realms/nightfall`. `None` disables
    /// authentication: every RPC except `Ping` is refused.
    pub oidc_issuer: Option<String>,
    /// Required token audience.
    pub oidc_audience: String,
    /// Accept `test:<account_uuid>` tokens instead of verifying JWTs. Local development only;
    /// opt in with `AUTH_DEV_TOKENS=1`.
    pub auth_dev_tokens: bool,
    /// Public WebSocket URL returned with each play ticket.
    pub ws_public_url: String,
    /// Concurrent WebSocket sessions per client IP. Default 10 (world.proto); raised only for
    /// load tests that run every client from one address.
    pub ws_max_sessions_per_ip: usize,
}

impl Config {
    /// Reads `HTTP_ADDR`, `GRPC_ADDR`, `DATABASE_URL`, `NATS_URL`, `OIDC_ISSUER`,
    /// `OIDC_AUDIENCE`, `AUTH_DEV_TOKENS`, `WS_PUBLIC_URL`, `WS_MAX_SESSIONS_PER_IP`.
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
            oidc_issuer: std::env::var("OIDC_ISSUER").ok().filter(|s| !s.is_empty()),
            oidc_audience: std::env::var("OIDC_AUDIENCE")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "nightfall-api".into()),
            auth_dev_tokens: std::env::var("AUTH_DEV_TOKENS").is_ok_and(|v| v == "1"),
            ws_public_url: std::env::var("WS_PUBLIC_URL")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| crate::DEFAULT_WS_PUBLIC_URL.into()),
            ws_max_sessions_per_ip: match std::env::var("WS_MAX_SESSIONS_PER_IP") {
                Ok(v) if !v.is_empty() => v.parse()?,
                _ => crate::application::session::MAX_SESSIONS_PER_IP,
            },
        })
    }
}
