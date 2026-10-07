//! gRPC surface generated from packages/proto.

use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use tonic::{transport::Server, Request, Response, Status};

pub mod pb {
    tonic::include_proto!("nightfall.v1");
}

use pb::game_service_server::{GameService, GameServiceServer};
use pb::{
    BaseStats, Character, GetCharacterRequest, PingRequest, PingResponse, Position, Race,
};

#[derive(Default)]
pub struct GameServiceImpl;

#[tonic::async_trait]
impl GameService for GameServiceImpl {
    async fn ping(&self, req: Request<PingRequest>) -> Result<Response<PingResponse>, Status> {
        tracing::debug!(client_version = %req.get_ref().client_version, "ping");
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or_default();

        Ok(Response::new(PingResponse {
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            server_time_ms: now_ms,
        }))
    }

    async fn get_character(
        &self,
        req: Request<GetCharacterRequest>,
    ) -> Result<Response<Character>, Status> {
        // Stub: returns a fixed Human fighter until persistence exists.
        let id = req.into_inner().character_id;
        Ok(Response::new(Character {
            id,
            name: "Adventurer".into(),
            race: Race::Human as i32,
            level: 1,
            stats: Some(BaseStats {
                str: 40,
                dex: 30,
                con: 43,
                int: 21,
                wit: 11,
                men: 25,
            }),
            position: Some(Position { x: 0.0, y: 0.0 }),
        }))
    }
}

pub async fn serve(addr: SocketAddr) -> anyhow::Result<()> {
    Server::builder()
        .add_service(GameServiceServer::new(GameServiceImpl))
        .serve(addr)
        .await?;
    Ok(())
}
