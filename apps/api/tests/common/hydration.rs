//! Optional export of actual server-produced protobuf frames for client projection tests.
use nightfall_api::interface::grpc::pb;

/// Explicit fixture generation only; ordinary tests never rewrite committed artifacts.
pub(crate) fn write(name: &str, frames: &[pb::ServerMessage]) -> anyhow::Result<()> {
    use base64::Engine as _;
    use prost::Message as _;
    if let Some(directory) = std::env::var_os("NIGHTFALL_HYDRATION_FIXTURE_DIR") {
        let encoded: Vec<_> = frames
            .iter()
            .map(|m| base64::engine::general_purpose::STANDARD.encode(m.encode_to_vec()))
            .collect();
        let fixture = serde_json::json!({
            "format": "nightfall.v1.ServerMessage/base64",
            "producer": name,
            "frames": encoded,
        });
        std::fs::write(
            std::path::Path::new(&directory).join(format!("{name}.json")),
            serde_json::to_vec_pretty(&fixture)?,
        )?;
    }
    Ok(())
}
