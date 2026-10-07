//! Interface layer: inbound adapters (HTTP, gRPC). Thin by design: parse the wire request,
//! call one use case, map the result or error back to the wire. No business rules here.

pub mod grpc;
pub mod http;
