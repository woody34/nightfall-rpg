//! Zone definitions from `packages/data/zones/*.toml` (data pipeline:
//! docs/planning/00-foundations.md §3.4). Parsed into serde structs, validated, and turned into
//! the application's [`ZoneDefinition`]. The definition's hash (SHA-256 of the file bytes) is
//! the `config_hash` every snapshot of the zone records.

use std::path::Path;

use anyhow::Context as _;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::application::zone_bootstrap::{NpcSpawn, ZoneDefinition};
use crate::domain::zone::{Speed, Vec2Fixed, ZoneBounds, ZoneId};

/// The Phase 0b fixture zone, compiled in so the server starts without a data directory.
pub const TEST_ZONE_TOML: &str = include_str!("../../../../packages/data/zones/test_zone.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ZoneFile {
    id: String,
    zone_id: u32,
    name: String,
    bounds: BoundsFile,
    #[serde(default)]
    npcs: Vec<NpcFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundsFile {
    min: [i32; 2],
    max: [i32; 2],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NpcFile {
    name: String,
    pos: [i32; 2],
    speed: u32,
}

/// Parses and validates one zone file's contents.
pub fn parse_zone(toml_src: &str) -> anyhow::Result<ZoneDefinition> {
    let file: ZoneFile = toml::from_str(toml_src).context("parse zone TOML")?;
    if file.id.is_empty() || file.name.is_empty() {
        anyhow::bail!("zone id and name must not be empty");
    }
    let [min_x, min_y] = file.bounds.min;
    let [max_x, max_y] = file.bounds.max;
    let bounds =
        ZoneBounds::new(Vec2Fixed::from_tiles(min_x, min_y), Vec2Fixed::from_tiles(max_x, max_y))
            .with_context(|| format!("zone {}: invalid bounds", file.id))?;
    let npcs = file
        .npcs
        .into_iter()
        .map(|n| {
            let [x, y] = n.pos;
            let pos = Vec2Fixed::from_tiles(x, y);
            if !bounds.contains(pos) {
                anyhow::bail!("zone {}: NPC {} at {x},{y} is outside the bounds", file.id, n.name);
            }
            Ok(NpcSpawn {
                name: n.name,
                pos,
                speed: Speed::from_milli_tiles_per_tick(n.speed),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let hash = Sha256::digest(toml_src.as_bytes());
    Ok(ZoneDefinition {
        zone: ZoneId(file.zone_id),
        name: file.name,
        bounds,
        npcs,
        config_hash: format!("sha256:{}", hex(&hash)),
    })
}

/// Reads and parses a zone file.
pub fn load_zone(path: &Path) -> anyhow::Result<ZoneDefinition> {
    let src = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    parse_zone(&src).with_context(|| format!("load {}", path.display()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len().saturating_mul(2)), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_zone_is_256_tiles_square_with_its_npcs() {
        let def = parse_zone(TEST_ZONE_TOML).unwrap();
        assert_eq!(def.zone, ZoneId(1));
        assert_eq!(def.bounds.max(), Vec2Fixed::from_tiles(256, 256));
        assert_eq!(def.npcs.len(), 2);
        assert_eq!(def.npcs[0].name, "Gatekeeper");
        assert!(def.config_hash.starts_with("sha256:"));
        assert_eq!(def.config_hash.len(), "sha256:".len() + 64);
    }

    #[test]
    fn an_npc_outside_the_bounds_is_rejected() {
        let src = TEST_ZONE_TOML.replace("pos = [128, 128]", "pos = [300, 1]");
        assert!(parse_zone(&src).is_err());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let src = format!("{TEST_ZONE_TOML}\nweather = \"rain\"\n");
        assert!(parse_zone(&src).is_err());
    }

    #[test]
    fn the_hash_changes_with_the_content() {
        let a = parse_zone(TEST_ZONE_TOML).unwrap();
        let b = parse_zone(&TEST_ZONE_TOML.replace("Test Zone", "Other Zone")).unwrap();
        assert_ne!(a.config_hash, b.config_hash);
    }
}
