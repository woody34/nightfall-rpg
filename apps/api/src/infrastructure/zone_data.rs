//! Zone definitions from `packages/data/zones/*.toml` (data pipeline:
//! docs/planning/00-foundations.md §3.4). Parsed into serde structs, validated, and turned into
//! the application's [`ZoneDefinition`]. The definition's hash (SHA-256 of the file bytes) is
//! the `config_hash` every snapshot of the zone records.

use std::path::Path;

use anyhow::Context as _;
use serde::Deserialize;

use super::data_hash::DataHash;
use super::npc_data::{parse_template, resolve_slots, SlotFile, EMBEDDED_TEMPLATES};
use crate::application::zone_bootstrap::{NpcSpawn, ZoneDefinition};
use crate::domain::zone::{NpcTemplate, Speed, Vec2Fixed, ZoneBounds, ZoneId};

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
    safe_point: PointFile,
    #[serde(default)]
    spawn_slots: Vec<SlotFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PointFile {
    pos: [i32; 2],
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
    /// Fixtures are never attackable; attackable NPCs are spawn-slot templates.
    #[serde(default)]
    attackable: bool,
}

/// Parses a zone with the embedded NPC templates.
pub fn parse_zone(toml_src: &str) -> anyhow::Result<ZoneDefinition> {
    parse_zone_with(toml_src, &EMBEDDED_TEMPLATES)
}

/// Parses and validates a zone and its NPC templates (`(file stem, source)` pairs). Every
/// problem found in the templates and spawn slots is reported together.
pub fn parse_zone_with(
    toml_src: &str,
    template_srcs: &[(&str, &str)],
) -> anyhow::Result<ZoneDefinition> {
    let file: ZoneFile = toml::from_str(toml_src).context("parse zone TOML")?;
    if file.id.is_empty() || file.name.is_empty() {
        anyhow::bail!("zone id and name must not be empty");
    }
    let [min_x, min_y] = file.bounds.min;
    let [max_x, max_y] = file.bounds.max;
    let bounds =
        ZoneBounds::new(Vec2Fixed::from_tiles(min_x, min_y), Vec2Fixed::from_tiles(max_x, max_y))
            .with_context(|| format!("zone {}: invalid bounds", file.id))?;
    let mut errors = Vec::new();
    let npcs = file
        .npcs
        .into_iter()
        .filter_map(|n| {
            let [x, y] = n.pos;
            let pos = Vec2Fixed::from_tiles(x, y);
            if n.attackable {
                errors.push(format!(
                    "zone {}: NPC {} is attackable; attackable NPCs must be templates",
                    file.id, n.name
                ));
            }
            if !bounds.contains(pos) {
                errors.push(format!(
                    "zone {}: NPC {} at {x},{y} is outside the bounds",
                    file.id, n.name
                ));
                return None;
            }
            Some(NpcSpawn {
                name: n.name,
                pos,
                speed: Speed::from_milli_tiles_per_tick(n.speed),
            })
        })
        .collect::<Vec<_>>();

    let mut sorted: Vec<(&str, &str)> = template_srcs.to_vec();
    sorted.sort_by_key(|(stem, _)| *stem);
    let mut npc_templates: Vec<NpcTemplate> = sorted
        .iter()
        .filter_map(|(stem, src)| parse_template(stem, src, &mut errors))
        .collect();
    npc_templates.sort_by(|a, b| a.id.cmp(&b.id));
    let spawn_slots =
        resolve_slots(&file.id, file.spawn_slots, &npc_templates, bounds, &mut errors);
    let [sx, sy] = file.safe_point.pos;
    let safe_point = Vec2Fixed::from_tiles(sx, sy);
    if !bounds.contains(safe_point) {
        errors.push(format!("zone {}: safe point {sx},{sy} is outside the bounds", file.id));
    }
    if !errors.is_empty() {
        anyhow::bail!("invalid data ({} errors):\n  {}", errors.len(), errors.join("\n  "));
    }

    let hash = sorted
        .iter()
        .fold(DataHash::new().part("zone", toml_src.as_bytes()), |h, (stem, src)| {
            h.part(&format!("npc:{stem}"), src.as_bytes())
        });
    Ok(ZoneDefinition {
        zone: ZoneId(file.zone_id),
        name: file.name,
        bounds,
        npcs,
        npc_templates,
        spawn_slots,
        safe_point,
        config_hash: hash.finish(),
    })
}

/// Reads and parses a zone file plus the templates in the sibling `../npcs/` directory.
pub fn load_zone(path: &Path) -> anyhow::Result<ZoneDefinition> {
    let src = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let dir = path.parent().and_then(Path::parent).map(|d| d.join("npcs"));
    let mut templates = Vec::new();
    if let Some(dir) = dir.filter(|d| d.is_dir()) {
        for entry in std::fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
            let p = entry?.path();
            if p.extension().is_some_and(|e| e == "toml") {
                let stem = p
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let text =
                    std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
                templates.push((stem, text));
            }
        }
    }
    let refs: Vec<(&str, &str)> = templates
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    parse_zone_with(&src, &refs).with_context(|| format!("load {}", path.display()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::infrastructure::npc_data::KELTIR_TOML;

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

    fn errors_of(zone: &str) -> String {
        format!("{:#}", parse_zone(zone).unwrap_err())
    }

    #[test]
    fn the_fixture_zone_boots_with_its_spawn_slots_and_safe_point() {
        let def = parse_zone(TEST_ZONE_TOML).unwrap();
        assert_eq!(def.npc_templates.len(), 1);
        assert_eq!(def.spawn_slots.len(), 2);
        assert_eq!(def.spawn_slots[0].count, 2);
        // Slot overrides win; omitted fields take the template's.
        assert_eq!(def.spawn_slots[0].respawn_delay_secs, 30);
        assert_eq!(def.spawn_slots[1].respawn_delay_secs, 20);
        assert_eq!(def.spawn_slots[1].respawn_random_secs, 5);
        assert_eq!(def.safe_point, Vec2Fixed::from_tiles(126, 126));
        // The two slots are within the template's clan help range of each other.
        let t = &def.npc_templates[0];
        assert!(def.spawn_slots[0]
            .home
            .within(def.spawn_slots[1].home, t.clan_help_range));
    }

    #[test]
    fn the_loaded_snapshot_is_deterministic() {
        let a = parse_zone(TEST_ZONE_TOML).unwrap();
        let b = parse_zone(TEST_ZONE_TOML).unwrap();
        assert_eq!(a, b);
        let c = parse_zone_with(
            TEST_ZONE_TOML,
            &[("keltir", &KELTIR_TOML.replace("max_hp = 44", "max_hp = 45"))],
        )
        .unwrap();
        assert_ne!(a.config_hash, c.config_hash, "template content feeds the hash");
    }

    #[test]
    fn a_slot_naming_an_unknown_template_is_rejected() {
        let e = errors_of(&TEST_ZONE_TOML.replace("template = \"keltir\"", "template = \"wolf\""));
        assert!(e.contains("unknown template"), "{e}");
    }

    #[test]
    fn a_slot_home_outside_the_zone_is_rejected() {
        let e = errors_of(&TEST_ZONE_TOML.replace("home = [100, 100]", "home = [300, 100]"));
        assert!(e.contains("outside the zone bounds"), "{e}");
    }

    #[test]
    fn a_negative_slot_delay_is_rejected() {
        let e = errors_of(
            &TEST_ZONE_TOML.replace("respawn_delay_secs = 20", "respawn_delay_secs = -1"),
        );
        assert!(e.contains("respawn_delay_secs"), "{e}");
    }

    #[test]
    fn a_float_in_a_template_is_rejected() {
        let t = KELTIR_TOML.replace("move_speed = 400", "move_speed = 0.4");
        let e = format!("{:#}", parse_zone_with(TEST_ZONE_TOML, &[("keltir", &t)]).unwrap_err());
        assert!(e.contains("keltir"), "{e}");
    }

    #[test]
    fn all_errors_are_reported_together() {
        let z = TEST_ZONE_TOML
            .replace("template = \"keltir\"", "template = \"wolf\"")
            .replace("home = [100, 100]", "home = [300, 100]");
        let e = errors_of(&z);
        assert!(e.contains("(3 errors)"), "{e}");
    }

    #[test]
    fn an_attackable_fixture_npc_is_rejected() {
        let e = errors_of(&TEST_ZONE_TOML.replacen("attackable = false", "attackable = true", 1));
        assert!(e.contains("attackable"), "{e}");
    }
}
