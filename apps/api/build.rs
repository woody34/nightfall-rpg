//! Compiles the shared .proto contracts into Rust at build time.
//! Uses `protox` (a pure-Rust protobuf compiler) so no system `protoc` is required.

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let proto_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/proto");
    let files = [proto_root.join("nightfall/v1/game.proto")];

    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
    }

    let fds = protox::compile(&files, [&proto_root])?;

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_fds(fds)?;

    Ok(())
}
