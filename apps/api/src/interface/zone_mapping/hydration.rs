//! Wire projections for the coordinated V5 hydration carriers. The zone supplies already
//! filtered endpoints; these functions preserve their paired presence and original times.

use super::{pb, swing_to_pb};
use crate::domain::zone::effects::TargetBinding;
use crate::domain::zone::{EntityId, PvpState, Swing, Tick};

/// A separately delivered V5 swing, after the observer's complete identity batch.
#[must_use]
pub fn phase3_swing_to_pb(
    attacker: TargetBinding,
    swing: &Swing,
    target_visible: bool,
    baseline: Option<Tick>,
) -> pb::AttackStarted {
    let mut result = swing_to_pb(attacker.entity, swing);
    result.attacker_incarnation = Some(attacker.incarnation);
    result.baseline = baseline.is_some();
    result.baseline_tick = baseline.map_or(0, |t| t.0);
    if !target_visible {
        result.target.clear();
        result.target_incarnation = 0;
    }
    result
}

/// Final owner selection, including an explicit empty admission/replacement baseline.
#[must_use]
pub fn phase3_selection_to_pb(
    entity: EntityId,
    tick: Tick,
    target: Option<TargetBinding>,
    baseline: Option<Tick>,
) -> pb::TargetChanged {
    pb::TargetChanged {
        entity: entity.to_string(),
        target: target.map(|t| t.entity.to_string()).unwrap_or_default(),
        target_incarnation: target.map(|t| t.incarnation),
        tick: tick.0,
        baseline: baseline.is_some(),
        baseline_tick: baseline.map_or(0, |t| t.0),
    }
}

/// Public player counters; bootstrap is explicitly accepted at the identity baseline.
#[must_use]
pub fn pvp_state_to_pb(
    subject: TargetBinding,
    tick: Tick,
    pvp: &PvpState,
    baseline: Option<Tick>,
) -> pb::PvpStateChanged {
    pb::PvpStateChanged {
        entity: subject.entity.to_string(),
        incarnation: subject.incarnation,
        tick: tick.0,
        flag_until_tick: pvp.flag_until.map_or(0, |t| t.0),
        karma: pvp.karma,
        pk_count: pvp.pk_count,
        pvp_count: pvp.pvp_count,
        baseline: baseline.is_some(),
        baseline_tick: baseline.map_or(0, |t| t.0),
    }
}

/// Apply only nonreferential public V5 values to an identity spawn. Pending swing/cast
/// and effect state follow in separate hydration events after all identities exist.
pub fn phase3_identity_to_pb(spawn: &mut pb::EntitySpawn, flag_until: Option<Tick>, karma: u32) {
    spawn.flag_until_tick = flag_until.map_or(0, |t| t.0);
    spawn.karma = karma;
    spawn.pending_swing = None;
    spawn.pending_cast = None;
    spawn.active_effects.clear();
}
