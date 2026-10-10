//! V5 hostile-action classification and canonical player `PvP` mutations. The tick driver
//! checks this at start, impact and each eligible pulse, before costs/rolls/flag changes.
//! Counter precedence is architecture D-R4: flagged wins, chaotic-unflagged awards neither.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::effects::TargetBinding;
use super::skill_math::{karma_gain, karma_lost, ExactRatio};
use super::skill_rules::PvpRules;
use super::{EntityKind, Fixed, PvpState, StatError, Tick, Vec2Fixed, ZoneBounds};

/// Additive `zones/test_zone_phase3.toml` policy, hashed separately from the legacy zone.
/// It is architect-set configuration (R7/D-R11), not an inferred source constant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Phase3Geometry {
    pub schema_version: u32,
    pub id: String,
    pub peace_zone: PeaceZoneConfig,
    pub provenance: GeometryProvenance,
}

/// Inclusive square in canonical input units. Only this shape is supported in Phase3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct PeaceZoneConfig {
    pub shape: String,
    pub centre: [i32; 2],
    pub half_width_millitiles: i32,
    pub inclusive: bool,
}

/// Provenance remains part of the normalized rules hash, including the legacy safe point
/// reference. This configuration has no fictitious L2J revision or source claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct GeometryProvenance {
    pub kind: String,
    pub decision: String,
    pub safe_point: String,
}

impl Phase3Geometry {
    /// Validate the supported schema and all coordinate operations without zone context.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.id != "test_zone"
            || self.peace_zone.shape != "square"
            || !self.peace_zone.inclusive
            || self.peace_zone.half_width_millitiles <= 0
            || self.provenance.kind != "Nightfall configuration"
            || self.provenance.decision != "architecture-decisions.md §6 R7 / §7 D-R11"
            || self.provenance.safe_point != "packages/data/zones/test_zone.toml:33"
        {
            return Err("unsupported Phase3 peace geometry/schema/provenance".into());
        }
        let center = self.center().map_err(|e| e.to_string())?;
        for coordinate in [center.x.raw(), center.y.raw()] {
            coordinate
                .checked_sub(self.peace_zone.half_width_millitiles)
                .and_then(|_| coordinate.checked_add(self.peace_zone.half_width_millitiles))
                .ok_or_else(|| "Phase3 peace geometry coordinate overflow".to_owned())?;
        }
        Ok(())
    }
    /// Checked whole-tile to milli-tile conversion, not saturating coordinate arithmetic.
    pub fn center(&self) -> Result<Vec2Fixed, StatError> {
        let [x, y] = self.peace_zone.centre;
        let fixed = |n: i32| {
            n.checked_mul(super::UNITS_PER_TILE)
                .map(Fixed::from_raw)
                .ok_or(StatError::Overflow)
        };
        Ok(Vec2Fixed::new(fixed(x)?, fixed(y)?))
    }
    /// The whole square must fit the epoch bounds, not merely its center.
    pub fn peace_square(&self, bounds: ZoneBounds) -> Result<PeaceSquare, StatError> {
        self.validate().map_err(|_| StatError::Overflow)?;
        PeaceSquare::new(
            self.center()?,
            Fixed::from_raw(self.peace_zone.half_width_millitiles),
            bounds,
        )
    }
}

/// Validated inclusive square; construction never assumes a default town or width.
/// Geometry must be supplied from the immutable epoch policy, not a live config lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeaceSquare {
    center: Vec2Fixed,
    half_width: Fixed,
}
impl PeaceSquare {
    /// Reject negative/overflow/out-of-zone geometry at policy admission.
    pub fn new(
        center: Vec2Fixed,
        half_width: Fixed,
        bounds: ZoneBounds,
    ) -> Result<Self, StatError> {
        if half_width.raw() < 0 {
            return Err(StatError::Overflow);
        }
        let point = |sign: i32| -> Result<Vec2Fixed, StatError> {
            let width = half_width
                .raw()
                .checked_mul(sign)
                .ok_or(StatError::Overflow)?;
            Ok(Vec2Fixed::new(
                Fixed::from_raw(
                    center
                        .x
                        .raw()
                        .checked_add(width)
                        .ok_or(StatError::Overflow)?,
                ),
                Fixed::from_raw(
                    center
                        .y
                        .raw()
                        .checked_add(width)
                        .ok_or(StatError::Overflow)?,
                ),
            ))
        };
        if !bounds.contains(point(-1)?) || !bounds.contains(point(1)?) {
            return Err(StatError::Overflow);
        }
        Ok(Self { center, half_width })
    }
    /// Inclusive on all four edges and corners; this is not a radial distance test.
    pub fn contains(self, point: Vec2Fixed) -> bool {
        let width = i64::from(self.half_width.raw());
        i64::from(point.x.raw()).abs_diff(i64::from(self.center.x.raw())) <= width.unsigned_abs()
            && i64::from(point.y.raw()).abs_diff(i64::from(self.center.y.raw()))
                <= width.unsigned_abs()
    }
}

/// Read-only endpoint captured from the current authoritative entity and combat block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct HostilityEndpoint {
    pub binding: TargetBinding,
    pub kind: EntityKind,
    pub pos: Vec2Fixed,
    pub alive: bool,
    pub attackable: bool,
    pub protected: bool,
}

/// Refusals shared by skill and auto-attack callers; they map to the pinned domain/wire
/// reasons when the coordinated event/mapper contract is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[allow(missing_docs)]
pub enum HostilityError {
    #[error("stale target/source incarnation")]
    StaleLife,
    #[error("dead source")]
    DeadActor,
    #[error("target cannot receive this hostile action")]
    InvalidTarget,
    #[error("hostile endpoint is inside peace")]
    PeaceZone,
    #[error("target has spawn protection")]
    Protected,
}

/// Capture BEFORE hostility/kill mutations. The deadline is evaluated at impact, never at
/// cast start; flagged+chaotic is Flagged, following the accepted truth table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VictimPvp {
    /// Unflagged with zero karma: one PK and pre-increment karma gain.
    Neutral,
    /// Positive remaining flag: one `PvP` even when also chaotic.
    Flagged,
    /// Positive karma without a live flag: neither counter nor new flag refresh.
    Chaotic,
}
impl VictimPvp {
    /// Classify at the exact hostility/impact tick; deadline equality is expired.
    pub fn at(state: &PvpState, now: Tick) -> Self {
        if state.flag_until.is_some_and(|until| until > now) {
            Self::Flagged
        } else if state.karma != 0 {
            Self::Chaotic
        } else {
            Self::Neutral
        }
    }
}

/// Explicit input to `apply_damage_v5`: source attribution determines CP absorption.
/// Caller separately preserves the impact-time victim classification for kill credit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostilityClassification {
    /// Player-sourced damage to a player removes CP before HP (including periodic damage).
    pub absorb_cp: bool,
}

/// Validate ordinary live hostile starts/impacts. Selection alone does not call this.
/// AOI/range, handler target kind and NPC intention remain caller-specific checks.
pub fn hostility_allowed(
    source: HostilityEndpoint,
    target: HostilityEndpoint,
    expected_source: TargetBinding,
    expected_target: TargetBinding,
    peace: PeaceSquare,
) -> Result<HostilityClassification, HostilityError> {
    if source.binding != expected_source
        || target.binding != expected_target
        || source.binding.incarnation == 0
        || target.binding.incarnation == 0
    {
        return Err(HostilityError::StaleLife);
    }
    if !source.alive {
        return Err(HostilityError::DeadActor);
    }
    if source.binding.entity == target.binding.entity
        || !target.alive
        || (target.kind == EntityKind::Npc && !target.attackable)
        || (source.kind == EntityKind::Npc && target.kind == EntityKind::Npc)
    {
        return Err(HostilityError::InvalidTarget);
    }
    if peace.contains(source.pos) || peace.contains(target.pos) {
        return Err(HostilityError::PeaceZone);
    }
    if target.protected {
        return Err(HostilityError::Protected);
    }
    Ok(HostilityClassification {
        absorb_cp: source.kind == EntityKind::Player && target.kind == EntityKind::Player,
    })
}

/// Actual saturating counter/karma changes, never the requested amount at a saturated cap.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct PvpKillDelta {
    pub karma_gained: u32,
    pub pk_added: u32,
    pub pvp_added: u32,
}

impl PvpState {
    /// Accepted hostile starts, eligible pulses and impacts refresh only the attacker.
    /// Repeated activity never shortens an existing longer flag; being hit does not call it.
    pub fn note_hostile_act(
        &mut self,
        victim: VictimPvp,
        now: Tick,
        rules: &PvpRules,
    ) -> Result<bool, StatError> {
        let duration = match victim {
            VictimPvp::Neutral => rules.flags.pvp_flag_ticks,
            VictimPvp::Flagged => rules.flags.pvp_flag_low_ticks,
            VictimPvp::Chaotic => return Ok(false),
        };
        let deadline = Tick(now.0.checked_add(duration).ok_or(StatError::Overflow)?);
        if self.flag_until.is_none_or(|old| old < deadline) {
            self.flag_until = Some(deadline);
            Ok(true)
        } else {
            Ok(false)
        }
    }
    /// Called in the expiry phase, even when chaotic color hides the independent flag.
    pub fn expire_flag(&mut self, now: Tick) -> bool {
        if self.flag_until.is_some_and(|until| until <= now) {
            self.flag_until = None;
            true
        } else {
            false
        }
    }
    /// Living-player lethal credit takes one branch using victim state captured at impact.
    /// Formulas.java:1830 uses the killer's PK count BEFORE the increment.
    pub fn apply_player_kill(&mut self, victim: VictimPvp) -> PvpKillDelta {
        match victim {
            VictimPvp::Neutral => {
                let before = self.karma;
                self.karma = self.karma.saturating_add(karma_gain(self.pk_count));
                let pk = self.pk_count;
                self.pk_count = self.pk_count.saturating_add(1);
                PvpKillDelta {
                    karma_gained: self.karma.saturating_sub(before),
                    pk_added: self.pk_count.saturating_sub(pk),
                    pvp_added: 0,
                }
            },
            VictimPvp::Flagged => {
                let pvp = self.pvp_count;
                self.pvp_count = self.pvp_count.saturating_add(1);
                PvpKillDelta {
                    pvp_added: self.pvp_count.saturating_sub(pvp),
                    ..PvpKillDelta::default()
                }
            },
            VictimPvp::Chaotic => PvpKillDelta::default(),
        }
    }
    /// Only actual monster XP credited, using the level BEFORE that award. No time, death,
    /// healer or raw-XP decay path exists. Exact source literals are not rounded to Q.
    pub fn decay_karma(
        &mut self,
        credited_xp: u64,
        pre_award_level: u32,
        rules: &PvpRules,
    ) -> Result<u32, StatError> {
        let index = pre_award_level
            .checked_sub(1)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or(StatError::LevelOutOfRange(pre_award_level))?;
        let multiplier = rules
            .karma_lost
            .multipliers
            .get(index)
            .ok_or(StatError::LevelOutOfRange(pre_award_level))?;
        let lost = karma_lost(
            credited_xp,
            ExactRatio::parse_literal(multiplier)?,
            ExactRatio::whole(i64::from(rules.karma_lost.rate_karma_lost)),
        )?;
        let lost =
            u32::try_from(lost.min(u64::from(self.karma))).map_err(|_| StatError::Overflow)?;
        self.karma = self.karma.saturating_sub(lost);
        Ok(lost)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::{EntityId, ZoneId, ZoneSeed, ZoneState};
    use super::*;
    use uuid::Uuid;
    fn state(karma: u32, until: Option<u64>) -> PvpState {
        PvpState {
            karma,
            flag_until: until.map(Tick),
            pk_count: 0,
            pvp_count: 0,
        }
    }
    fn rules() -> PvpRules {
        toml::from_str(include_str!("../../../../../packages/data/tables/pvp.toml")).unwrap()
    }
    fn bounds() -> ZoneBounds {
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap()
    }
    fn peace() -> PeaceSquare {
        PeaceSquare::new(Vec2Fixed::from_tiles(126, 126), Fixed::from_tiles(10), bounds()).unwrap()
    }
    fn endpoint(n: u128, kind: EntityKind) -> HostilityEndpoint {
        HostilityEndpoint {
            binding: TargetBinding {
                entity: EntityId::from_uuid(Uuid::from_u128(n)),
                incarnation: 1,
            },
            kind,
            pos: Vec2Fixed::from_tiles(100, 100),
            alive: true,
            attackable: true,
            protected: false,
        }
    }

    #[test]
    fn flag_and_kill_truth_table_covers_all_attacker_and_victim_colors() {
        let rules = rules();
        for initial in [state(0, None), state(0, Some(2000)), state(300, Some(2000))] {
            for (victim, class, duration, pk, pvp) in [
                (state(0, None), VictimPvp::Neutral, Some(1300), 1, 0),
                (state(0, Some(101)), VictimPvp::Flagged, Some(700), 0, 1),
                (state(5, None), VictimPvp::Chaotic, None, 0, 0),
                (state(5, Some(101)), VictimPvp::Flagged, Some(700), 0, 1),
            ] {
                let mut attacker = initial.clone();
                let before = attacker.clone();
                let frozen = VictimPvp::at(&victim, Tick(100));
                assert_eq!(frozen, class);
                attacker
                    .note_hostile_act(frozen, Tick(100), &rules)
                    .unwrap();
                let expected = match (before.flag_until, duration.map(Tick)) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, None) => a,
                    (None, b) => b,
                };
                assert_eq!(attacker.flag_until, expected);
                let delta = attacker.apply_player_kill(frozen);
                assert_eq!((delta.pk_added, delta.pvp_added), (pk, pvp));
                assert_eq!(
                    attacker.karma,
                    if pk == 1 {
                        before.karma.saturating_add(720)
                    } else {
                        before.karma
                    }
                );
                assert_eq!(victim.pk_count, 0); // Attacker mutation never changes the victim.
            }
        }
        assert_eq!(VictimPvp::at(&state(0, Some(100)), Tick(100)), VictimPvp::Neutral);
    }
    #[test]
    fn caps_expiry_preincrement_gain_and_failed_deadline_are_explicit() {
        let mut p = state(u32::MAX - 3, None);
        p.pk_count = 99;
        p.pvp_count = u32::MAX;
        assert_eq!(
            p.apply_player_kill(VictimPvp::Neutral),
            PvpKillDelta {
                karma_gained: 3,
                pk_added: 1,
                pvp_added: 0
            }
        );
        assert_eq!(p.pk_count, 100);
        assert_eq!(p.apply_player_kill(VictimPvp::Flagged), PvpKillDelta::default());
        let mut p = state(0, None);
        p.pk_count = 99;
        assert_eq!(p.apply_player_kill(VictimPvp::Neutral).karma_gained, 36_090);
        let before = p.clone();
        assert!(p
            .note_hostile_act(VictimPvp::Neutral, Tick(u64::MAX), &rules())
            .is_err());
        assert_eq!(p, before);
        p.flag_until = Some(Tick(100));
        assert!(!p.expire_flag(Tick(99)));
        assert!(p.expire_flag(Tick(100)));
        assert!(!p.expire_flag(Tick(100)));
    }
    #[test]
    fn hostility_checks_both_peace_endpoints_lives_protection_and_cp_source() {
        let source = endpoint(1, EntityKind::Player);
        let target = endpoint(2, EntityKind::Player);
        let check = |s: HostilityEndpoint, t: HostilityEndpoint| {
            hostility_allowed(s, t, source.binding, target.binding, peace())
        };
        assert!(check(source, target).unwrap().absorb_cp);
        let mut npc = source;
        npc.kind = EntityKind::Npc;
        assert!(!check(npc, target).unwrap().absorb_cp);
        for pos in [
            Vec2Fixed::from_tiles(116, 116),
            Vec2Fixed::from_tiles(136, 136),
            Vec2Fixed::from_tiles(126, 136),
        ] {
            let mut t = target;
            t.pos = pos;
            assert_eq!(check(source, t), Err(HostilityError::PeaceZone));
            let mut s = source;
            s.pos = pos;
            assert_eq!(check(s, target), Err(HostilityError::PeaceZone));
        }
        let mut t = target;
        t.pos = Vec2Fixed::new(Fixed::from_raw(136_001), Fixed::from_tiles(126));
        assert!(check(source, t).is_ok());
        t = target;
        t.binding.incarnation = 2;
        assert_eq!(check(source, t), Err(HostilityError::StaleLife));
        t = target;
        t.protected = true;
        assert_eq!(check(source, t), Err(HostilityError::Protected));
        t = target;
        t.alive = false;
        assert_eq!(check(source, t), Err(HostilityError::InvalidTarget));
        // Validation itself is read-only and does not draw from any zone generator.
        let z = ZoneState::new(
            ZoneSeed {
                zone: ZoneId(1),
                epoch: 1,
            },
            bounds(),
            0,
        );
        let before = z.snapshot();
        check(source, target).unwrap();
        assert_eq!(z.snapshot(), before);
    }
    #[test]
    fn karma_decay_uses_actual_credit_and_exact_preadvance_level() {
        let rules = rules();
        let mut p = state(1000, None);
        assert_eq!(p.decay_karma(0, 1, &rules).unwrap(), 0);
        assert_eq!(p.decay_karma(28, 1, &rules).unwrap(), 1);
        assert_eq!(p.karma, 999);
        let before = p.clone();
        assert!(p.decay_karma(28, 0, &rules).is_err());
        assert_eq!(p, before);
        assert_eq!(p.decay_karma(u64::MAX, 1, &rules).unwrap(), 999);
        assert_eq!(p.karma, 0);
    }
}
