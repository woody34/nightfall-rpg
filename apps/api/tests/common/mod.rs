//! Shared harness: boots the real HTTP and gRPC servers on ephemeral ports with in-memory
//! adapters, and hands back clients. Every endpoint test goes through a real socket.

#![allow(
    dead_code,
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

pub mod pg;

use std::sync::Arc;

use nightfall_api::infrastructure::memory::{InMemoryCharacterRepository, InMemoryEventBus};
use nightfall_api::infrastructure::SystemClock;
use nightfall_api::interface::grpc::pb::game_service_client::GameServiceClient;
use nightfall_api::{bind, build_game_service, serve_grpc, serve_http, Dependencies};
use tonic::transport::Channel;

pub struct TestApp {
    pub http_base: String,
    pub grpc: GameServiceClient<Channel>,
    pub characters: Arc<InMemoryCharacterRepository>,
    pub bus: Arc<InMemoryEventBus>,
    /// Server tasks; aborted when the app is dropped.
    http_task: tokio::task::JoinHandle<anyhow::Result<()>>,
    grpc_task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl TestApp {
    pub async fn spawn() -> Self {
        let characters = Arc::new(InMemoryCharacterRepository::default());
        let bus = Arc::new(InMemoryEventBus::default());
        let deps = Dependencies {
            characters: characters.clone(),
            bus: bus.clone(),
            clock: Arc::new(SystemClock),
        };
        let service = build_game_service(&deps);

        let (http, grpc) = bind("127.0.0.1:0".parse().unwrap(), "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let http_addr = http.local_addr().unwrap();
        let grpc_addr = grpc.local_addr().unwrap();

        let http_task = tokio::spawn(serve_http(http));
        let grpc_task = tokio::spawn(serve_grpc(grpc, service));

        let grpc = GameServiceClient::connect(format!("http://{grpc_addr}"))
            .await
            .unwrap();
        Self {
            http_base: format!("http://{http_addr}"),
            grpc,
            characters,
            bus,
            http_task,
            grpc_task,
        }
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        self.http_task.abort();
        self.grpc_task.abort();
    }
}

pub const KEY_A: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8e";
pub const KEY_B: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8f";
pub const ACCOUNT: &str = "00000000-0000-0000-0000-000000000001";
