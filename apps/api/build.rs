//! Compiles the shared .proto contracts into Rust at build time.
//! Uses `protox` (a pure-Rust protobuf compiler) so no system `protoc` is required.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    embed_phase4_skills()?;
    let proto_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/proto");
    let files = [
        proto_root.join("nightfall/v1/game.proto"),
        proto_root.join("nightfall/v1/world.proto"),
        proto_root.join("nightfall/v1/session.proto"),
    ];

    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
    }

    let fds = protox::compile(&files, [&proto_root])?;

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true) // clients are used by the integration tests
        .compile_fds(fds)?;

    Ok(())
}

fn embed_phase4_skills() -> anyhow::Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/data/skills-phase4");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut paths = std::fs::read_dir(&root)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "toml")
    });
    paths.sort();
    let mut generated = String::from("const EMBEDDED_PHASE4_SKILLS: &[(&str, &str)] = &[\n");
    for path in paths {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("invalid Phase4 skill filename"))?;
        let source_name = format!("skills/{name}");
        let path = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF8 skill path"))?;
        // Debug strings are escaped Rust literals in the generated source.
        writeln!(generated, "({source_name:?}, include_str!({path:?})),")?;
    }
    generated.push_str("];\n");
    std::fs::write(
        PathBuf::from(
            std::env::var_os("OUT_DIR").ok_or_else(|| anyhow::anyhow!("missing OUT_DIR"))?,
        )
        .join("phase4_skills.rs"),
        generated,
    )?;
    Ok(())
}
