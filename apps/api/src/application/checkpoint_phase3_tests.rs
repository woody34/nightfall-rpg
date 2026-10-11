//! Canonical critical-save detection; these equality probes do not execute gameplay policy.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::retained_phase3_changed;
use crate::domain::zone::effects::{
    ActiveEffect, ActiveHandler, ActivePeriodic, EffectFlags, EffectId, EffectList, EffectPool,
    EffectSource, EffectSourceKind, FrozenRatio, PeriodicPower, PeriodicResource, PulseSchedule,
};
use crate::domain::zone::skill::{SkillId, SkillRef};
use crate::domain::zone::skill_math::ExactRatio;
use crate::domain::zone::state_regen::RegenSchedule;
use crate::domain::zone::{
    CastState, DeathRecord, EntityOrdinal, Phase3Entity, Phase3Player, PvpState, ResourceFractions,
    Tick,
};
use std::collections::BTreeMap;

fn side() -> Phase3Entity {
    Phase3Entity {
        ordinal: EntityOrdinal(1),
        cast: CastState::Idle,
        reuse: BTreeMap::from([(SkillId(3), Tick(600))]),
        effects: EffectList::default(),
        modifier_install_order: BTreeMap::new(),
        regen: RegenSchedule {
            next_at: Some(Tick(30)),
        },
        fractions: ResourceFractions::default(),
        player: Some(Phase3Player {
            pvp: PvpState {
                flag_until: Some(Tick(1200)),
                karma: 0,
                pk_count: 0,
                pvp_count: 0,
            },
            death_penalty_level: 0,
            last_death: None,
            resurrect_offer: None,
        }),
    }
}
#[test]
fn only_canonical_retained_changes_are_critical_not_clock_or_nonretained_schedules() {
    let before = side();
    assert!(!retained_phase3_changed(Some(&before), Some(&before)));
    let mut later = before.clone();
    later.regen.next_at = Some(Tick(60));
    later.ordinal = EntityOrdinal(2);
    assert!(!retained_phase3_changed(Some(&before), Some(&later)));
    assert!(!retained_phase3_changed(None, None));
    assert!(retained_phase3_changed(None, Some(&before)));
    assert!(retained_phase3_changed(Some(&before), None));
    for change in 0..9 {
        let mut after = before.clone();
        let player = after.player.as_mut().unwrap();
        match change {
            0 => after.fractions.mp = 1,
            1 => {
                after.reuse.insert(SkillId(3), Tick(601));
            },
            2 => player.pvp.flag_until = Some(Tick(1201)),
            3 => player.pvp.karma = 1,
            4 => player.pvp.pk_count = 1,
            5 => player.death_penalty_level = 1,
            6 => player.pvp.pvp_count = 1,
            7 => {
                after
                    .modifier_install_order
                    .insert(1, crate::domain::zone::RestoredModifierSource::DeathPenalty);
            },
            _ => {
                player.last_death = Some(DeathRecord {
                    incarnation: 1,
                    pre_death_xp: 100,
                    recoverable_death_xp_loss: 10,
                });
            },
        }
        assert!(retained_phase3_changed(Some(&before), Some(&after)));
    }
}
#[test]
fn initialized_effect_pulse_phase_is_critical_without_waiting_for_whole_hp_to_change() {
    let mut before = side();
    let effect = ActiveEffect {
        id: EffectId(1),
        skill: SkillRef {
            id: SkillId(110),
            rank: 1,
        },
        source: EffectSource {
            binding: None,
            kind: EffectSourceKind::System,
        },
        target_incarnation: 1,
        applied_at: Tick(0),
        expires_at: Some(Tick(1000)),
        pool: EffectPool::Buff,
        abnormal_type: "NONE".into(),
        abnormal_level: 0,
        flags: EffectFlags {
            stay_after_death: false,
            stay_on_subclass_change: false,
            removed_on_damage: false,
            removed_on_action_except_move: false,
            irreplaceable: false,
        },
        handlers: vec![ActiveHandler {
            xml_ordinal: 0,
            periodic: Some(ActivePeriodic {
                resource: PeriodicResource::Hp,
                power: PeriodicPower::Diff {
                    amount: FrozenRatio::new(ExactRatio::whole(1)),
                },
                schedule: PulseSchedule::new(Tick(0), 666).unwrap(),
            }),
        }],
    };
    before.effects = EffectList::from_entries(BTreeMap::from([(effect.id, effect)])).unwrap();
    let mut after = before.clone();
    after.effects.advance_pulse(EffectId(1), 0).unwrap();
    assert!(retained_phase3_changed(Some(&before), Some(&after)));
    assert_eq!(before.fractions, after.fractions);
}
