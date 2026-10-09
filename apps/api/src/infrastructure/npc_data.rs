//! NPC templates (`packages/data/npcs/<id>.toml`) and zone spawn slots. Parsing is exact: only
//! integers and decimal strings, floats are rejected. Validation collects every problem so a bad
//! data set is reported in full at startup.

use super::exact_decimal::parse_decimal;

use std::collections::BTreeSet;
use std::fmt;

use serde::de::{self, Deserializer, Visitor};
use serde::Deserialize;

use crate::domain::zone::{
    Fixed, NpcTemplate, NpcTemplateId, SpawnSlot, Speed, Vec2Fixed, ZoneBounds, Q,
};

/// The Phase 1 starter monster, compiled in so the server starts without a data directory.
pub const KELTIR_TOML: &str = include_str!("../../../../packages/data/npcs/keltir.toml");

/// Embedded templates as `(file stem, source)`.
pub const EMBEDDED_TEMPLATES: [(&str, &str); 1] = [("keltir", KELTIR_TOML)];

/// An integer or a decimal string exactly representable as Q units (1e-6).
#[derive(Debug, Clone, Copy)]
struct QDecimal(i64);

impl<'de> Deserialize<'de> for QDecimal {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = QDecimal;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an integer or a decimal string such as \"8.8\"")
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<QDecimal, E> {
                v.checked_mul(Q)
                    .map(QDecimal)
                    .ok_or_else(|| E::custom("value too large"))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<QDecimal, E> {
                i64::try_from(v)
                    .map_err(|_| E::custom("value too large"))
                    .and_then(|v| self.visit_i64(v))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<QDecimal, E> {
                Err(E::custom("float literals are not allowed; use an integer or a decimal string"))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<QDecimal, E> {
                parse_decimal(s)
                    .map(|v| QDecimal(v.raw()))
                    .map_err(E::custom)
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateFile {
    id: String,
    name: String,
    attackable: bool,
    level: u32,
    p_atk: QDecimal,
    p_def: QDecimal,
    max_hp: u32,
    max_mp: u32,
    attack_speed: u32,
    attack_range: u32,
    move_speed: u32,
    collision_radius: u32,
    xp_reward: u64,
    aggressive: bool,
    aggro_range: u32,
    #[serde(default)]
    clan_id: Option<String>,
    clan_help_range: u32,
    leash_radius: u32,
    corpse_decay_ticks: u32,
    respawn_delay_secs: i64,
    respawn_random_secs: i64,
}

/// A spawn slot as written in a zone file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SlotFile {
    id: String,
    template: String,
    home: [i32; 2],
    count: u32,
    #[serde(default)]
    respawn_delay_secs: Option<i64>,
    #[serde(default)]
    respawn_random_secs: Option<i64>,
}

fn to_fixed(v: u32) -> Option<Fixed> {
    i32::try_from(v).ok().map(Fixed::from_raw)
}

/// Parses and validates one template; `stem` is its file name without `.toml`. All problems
/// found are pushed to `errors`; `None` when any exist.
pub fn parse_template(stem: &str, src: &str, errors: &mut Vec<String>) -> Option<NpcTemplate> {
    let f: TemplateFile = match toml::from_str(src) {
        Ok(f) => f,
        Err(e) => {
            errors.push(format!("npc template {stem}: {}", e.message()));
            return None;
        },
    };
    let before = errors.len();
    let mut err = |m: &str| errors.push(format!("npc template {stem}: {m}"));
    if f.id != stem {
        err(&format!("id {:?} must equal the file name", f.id));
    }
    if f.name.is_empty() {
        err("name must not be empty");
    }
    if !f.attackable {
        err("attackable must be true (non-combat fixtures are zone [[npcs]] entries)");
    }
    let level = u16::try_from(f.level).ok().filter(|l| *l >= 1);
    if level.is_none() {
        err("level must be 1..=65535");
    }
    for (name, v) in [
        ("max_hp", f.max_hp),
        ("attack_speed", f.attack_speed),
        ("attack_range", f.attack_range),
        ("move_speed", f.move_speed),
        ("collision_radius", f.collision_radius),
        ("leash_radius", f.leash_radius),
    ] {
        if v == 0 {
            err(&format!("{name} must be positive"));
        }
    }
    if f.p_atk.0 < 0 || f.p_def.0 < 0 {
        err("p_atk and p_def must not be negative");
    }
    if f.aggressive && f.aggro_range == 0 {
        err("an aggressive template needs a positive aggro_range");
    }
    if f.leash_radius < f.aggro_range {
        err("leash_radius must be at least aggro_range");
    }
    if f.clan_id.as_deref() == Some("") {
        err("clan_id must not be empty (omit it for no clan)");
    }
    if f.clan_help_range > 0 && f.clan_id.is_none() {
        err("clan_help_range needs a clan_id");
    }
    if f.respawn_delay_secs < 1 {
        err("respawn_delay_secs must be at least 1");
    }
    if f.respawn_random_secs < 0 {
        err("respawn_random_secs must not be negative");
    }
    let dist = |name: &str, v: u32, err: &mut dyn FnMut(&str)| {
        let r = to_fixed(v);
        if r.is_none() {
            err(&format!("{name} is out of range"));
        }
        r.unwrap_or(Fixed::ZERO)
    };
    let attack_range = dist("attack_range", f.attack_range, &mut err);
    let collision_radius = dist("collision_radius", f.collision_radius, &mut err);
    let aggro_range = dist("aggro_range", f.aggro_range, &mut err);
    let clan_help_range = dist("clan_help_range", f.clan_help_range, &mut err);
    let leash_radius = dist("leash_radius", f.leash_radius, &mut err);
    let delay = u32::try_from(f.respawn_delay_secs).unwrap_or(0);
    let random = u32::try_from(f.respawn_random_secs).unwrap_or(0);
    if errors.len() > before {
        return None;
    }
    Some(NpcTemplate {
        id: NpcTemplateId(f.id),
        name: f.name,
        level: level?,
        p_atk_q: f.p_atk.0,
        p_def_q: f.p_def.0,
        max_hp: f.max_hp,
        max_mp: f.max_mp,
        attack_speed: f.attack_speed,
        attack_range,
        move_speed: Speed::from_milli_tiles_per_tick(f.move_speed),
        collision_radius,
        xp_reward: f.xp_reward,
        aggressive: f.aggressive,
        aggro_range,
        clan_id: f.clan_id,
        clan_help_range,
        leash_radius,
        corpse_decay_ticks: f.corpse_decay_ticks,
        respawn_delay_secs: delay,
        respawn_random_secs: random,
    })
}

/// Resolves and validates spawn slots against `templates` and `bounds`, pushing every problem.
pub(super) fn resolve_slots(
    zone: &str,
    slots: Vec<SlotFile>,
    templates: &[NpcTemplate],
    bounds: ZoneBounds,
    errors: &mut Vec<String>,
) -> Vec<SpawnSlot> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for s in slots {
        let mut err = |m: &str| errors.push(format!("zone {zone}: spawn slot {}: {m}", s.id));
        let mut ok = true;
        if !seen.insert(s.id.clone()) {
            err("duplicate slot id");
            ok = false;
        }
        let template = templates.iter().find(|t| t.id.0 == s.template);
        if template.is_none() {
            err(&format!("unknown template {:?}", s.template));
            ok = false;
        }
        let home = Vec2Fixed::from_tiles(s.home[0], s.home[1]);
        if !bounds.contains(home) {
            err(&format!("home {},{} is outside the zone bounds", s.home[0], s.home[1]));
            ok = false;
        }
        let count = u16::try_from(s.count)
            .ok()
            .filter(|c| (1..=256).contains(c));
        if count.is_none() {
            err("count must be 1..=256");
            ok = false;
        }
        if s.respawn_delay_secs.is_some_and(|d| d < 1) {
            err("respawn_delay_secs must be at least 1");
            ok = false;
        }
        if s.respawn_random_secs.is_some_and(|d| d < 0) {
            err("respawn_random_secs must not be negative");
            ok = false;
        }
        if let (true, Some(t), Some(count)) = (ok, template, count) {
            let pick = |o: Option<i64>, default: u32| {
                o.map_or(default, |v| u32::try_from(v).unwrap_or(default))
            };
            out.push(SpawnSlot {
                id: s.id.clone(),
                template: t.id.clone(),
                home,
                count,
                respawn_delay_secs: pick(s.respawn_delay_secs, t.respawn_delay_secs),
                respawn_random_secs: pick(s.respawn_random_secs, t.respawn_random_secs),
            });
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Result<NpcTemplate, Vec<String>> {
        let mut errors = Vec::new();
        parse_template("keltir", src, &mut errors).ok_or(errors)
    }

    #[test]
    fn the_keltir_template_parses_to_exact_values() {
        let t = parse(KELTIR_TOML).unwrap();
        assert_eq!(t.p_atk_q, 8_800_000);
        assert_eq!(t.p_def_q, 26_900_000);
        assert_eq!(t.level, 1);
        assert_eq!(t.respawn_delay_secs, 30);
    }

    #[test]
    fn a_float_literal_is_rejected() {
        let e = parse(&KELTIR_TOML.replace("p_atk = \"8.8\"", "p_atk = 8.8")).unwrap_err();
        assert!(e.iter().any(|m| m.contains("float")), "{e:?}");
        let e = parse(&KELTIR_TOML.replace("max_hp = 44", "max_hp = 44.5")).unwrap_err();
        assert!(!e.is_empty());
    }

    #[test]
    fn every_problem_is_reported_not_just_the_first() {
        let src = KELTIR_TOML
            .replace("respawn_delay_secs = 30", "respawn_delay_secs = -5")
            .replace("max_hp = 44", "max_hp = 0")
            .replace("level = 1", "level = 0");
        assert_eq!(parse(&src).unwrap_err().len(), 3);
    }

    #[test]
    fn the_id_must_match_the_file_name() {
        let mut errors = Vec::new();
        assert!(parse_template("wolf", KELTIR_TOML, &mut errors).is_none());
        assert_eq!(errors.len(), 1);
    }
}
