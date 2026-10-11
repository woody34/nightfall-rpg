//! Synthetic full-pool periodic stress fixture. Canonical source validation is real;
//! no claim that shipped learning grants Enlarge or these distinct song/debuff identities.
#![allow(clippy::too_many_lines, clippy::indexing_slicing)]
use nightfall_api::domain::zone::{
    effects::{
        compile_periodic, ActiveEffect, ActiveHandler, ActivePeriodic, EffectFlags, EffectId,
        EffectList, EffectPool, EffectSource, EffectSourceKind, PulseSchedule,
    },
    skill::{SkillId, SkillRef, SkillRegistry},
    Phase3Rules, RestoredModifierSource, Tick, ZoneSnapshot,
};
use std::{collections::BTreeMap, sync::Arc};

pub(super) fn fill(snapshot: &mut ZoneSnapshot, count: u32) {
    let phase = snapshot.phase3.as_mut().unwrap();
    let base = &phase.rules;
    let mut skills = base.skills().skills().clone();
    let template = skills[&SkillId(1095)].clone();
    for index in 0..count {
        let mut skill = template.clone();
        // Synthetic healing callbacks exercise live scheduling without lethal stress or
        // requiring a currently present hostile source in every observer cell.
        skill.effects[0].params.insert(
            "power".into(),
            nightfall_api::domain::zone::skill::RankValue::Scalar("1".into()),
        );
        skill.id = SkillId(1_000_000 + index);
        skill.key = format!("l2.skill.{}", skill.id.0);
        skill.provenance.skill_id = skill.id.0;
        skill.is_magic = if (24..36).contains(&index) { 3 } else { 1 };
        let abnormal = skill.abnormal.as_mut().unwrap();
        abnormal.r#type = "NONE".into();
        abnormal.is_debuff = index >= 36;
        abnormal.time_s.fill(1200);
        skills.insert(skill.id, skill);
    }
    phase.rules = Arc::new(
        Phase3Rules::try_new(
            SkillRegistry::try_new(skills).unwrap(),
            base.pvp().clone(),
            base.regen().clone(),
            base.constants().clone(),
            base.profiles().clone(),
            base.geometry().clone(),
            base.npc().clone(),
        )
        .unwrap(),
    );
    phase.rules_hash = phase.rules.hash();
    for state in phase.entities.values_mut() {
        let mut entries = BTreeMap::new();
        for index in 0..count {
            let skill = phase
                .rules
                .skills()
                .get(SkillId(1_000_000 + index))
                .unwrap();
            let (resource, power, interval) =
                compile_periodic(&skill.effects[0], 1, skill.max_level)
                    .unwrap()
                    .unwrap();
            let effect = ActiveEffect {
                id: EffectId(phase.next_effect_id),
                skill: SkillRef {
                    id: skill.id,
                    rank: 1,
                },
                source: EffectSource {
                    binding: None,
                    kind: EffectSourceKind::System,
                },
                target_incarnation: 1,
                applied_at: Tick(0),
                expires_at: Some(Tick(12_000)),
                pool: if index < 24 {
                    EffectPool::Buff
                } else if index < 36 {
                    EffectPool::SongDance
                } else {
                    EffectPool::Debuff
                },
                abnormal_type: "NONE".into(),
                abnormal_level: skill.abnormal.as_ref().unwrap().level[0],
                flags: EffectFlags {
                    stay_after_death: *skill.stay_after_death.at(1, skill.max_level).unwrap(),
                    stay_on_subclass_change: *skill
                        .stay_on_subclass_change
                        .at(1, skill.max_level)
                        .unwrap(),
                    removed_on_damage: *skill.removed_on_damage.at(1, skill.max_level).unwrap(),
                    removed_on_action_except_move: *skill
                        .removed_on_action_except_move
                        .at(1, skill.max_level)
                        .unwrap(),
                    irreplaceable: *skill.irreplaceable_buff.at(1, skill.max_level).unwrap(),
                },
                handlers: vec![ActiveHandler {
                    xml_ordinal: 0,
                    periodic: Some(ActivePeriodic {
                        resource,
                        power,
                        schedule: PulseSchedule::new(Tick(0), interval).unwrap(),
                    }),
                }],
            };
            effect.validate_source(phase.rules.skills()).unwrap();
            state
                .modifier_install_order
                .insert(phase.next_install_sequence, RestoredModifierSource::Effect(effect.id));
            entries.insert(effect.id, effect);
            phase.next_effect_id += 1;
            phase.next_install_sequence += 1;
        }
        state.effects = EffectList::from_entries(entries).unwrap();
    }
    let contexts = snapshot
        .entities
        .iter()
        .map(|e| {
            let c = e.combat.as_ref().unwrap();
            let learned = match &c.role {
                nightfall_api::domain::zone::CombatRole::Player {
                    progression: Some(p),
                    ..
                } => p.class_state.learned_skills.as_slice(),
                nightfall_api::domain::zone::CombatRole::Player {
                    progression: None, ..
                }
                | nightfall_api::domain::zone::CombatRole::Npc { .. } => &[],
            };
            (
                e.id,
                nightfall_api::domain::zone::Phase3EntityContext {
                    next_tick: snapshot.tick,
                    incarnation: c.incarnation,
                    alive: c.hp > 0,
                    player: e.kind == nightfall_api::domain::zone::EntityKind::Player,
                    learned,
                },
            )
        })
        .collect();
    phase.validate_live(&contexts).unwrap();
}
