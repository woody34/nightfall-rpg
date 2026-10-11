//! Capacity preflight: every shipped rule plus synthetic, independently stackable skills
//! exercising the maximum 24/12/8 pools. This is a codec/representation bound, not evidence
//! that the current gameplay corpus grants Enlarge or twelve executable songs.
#![allow(clippy::too_many_lines, clippy::indexing_slicing)] // Fixed-size, validated fixture construction.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::sync::Arc;

use nightfall_api::application::replay_log::{decode_snapshot, encode_snapshot};
use nightfall_api::domain::zone::effects::{
    compile_periodic, ActiveEffect, ActiveHandler, ActivePeriodic, EffectCapacities, EffectFlags,
    EffectId, EffectList, EffectPool, EffectSource, EffectSourceKind, PulseSchedule, TargetBinding,
};
use nightfall_api::domain::zone::skill::{RankValue, SkillId, SkillRef, SkillRegistry};
use nightfall_api::domain::zone::{
    AoiIndex, EntityId, EntityOrdinal, NpcCombat, Phase3EntityContext, Phase3Rules,
    RestoredModifierSource, Tick, Vec2Fixed, ZoneBounds, ZoneCommand, ZoneId, ZoneInput, ZoneSeed,
    ZoneState,
};
use nightfall_api::infrastructure::{
    class_data::{load_classes, ClassSource},
    rules_data::{load_rules, RulesSource},
    skill_data::{load_phase3_rules, Phase3DataSource},
    zone_data::{parse_zone, TEST_ZONE_TOML},
};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

pub(super) fn run() -> bool {
    assert!(realistic());
    let base = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
    let original_skills = base.skills().skills().len();
    let shipped_rule_bytes = serde_json::to_vec(&base).unwrap().len();
    let mut skills = base.skills().skills().clone();
    // A pure periodic source avoids changing physical movement inputs when duplicating
    // the source-valid canonical fixture; the full decoder can therefore cold-restore it.
    let template = skills[&SkillId(1095)].clone();
    for index in 0..44_u32 {
        let mut skill = template.clone();
        skill.id = SkillId(1_000_000 + index);
        skill.key = format!("l2.skill.{}", skill.id.0);
        skill.provenance.skill_id = skill.id.0;
        skill.l2_ref = format!("Capacity fixture {index}");
        skill.is_magic = if (24..36).contains(&index) { 3 } else { 1 };
        let abnormal = skill.abnormal.as_mut().unwrap();
        abnormal.r#type = "NONE".into();
        abnormal.is_debuff = index >= 36;
        abnormal.time_s.fill(1200);
        // Preserve the complete source handler shape, normalizing its rank parameters.
        for value in skill.effects[0].params.values_mut() {
            *value = RankValue::Scalar(value.at(1, skill.max_level).unwrap().clone());
        }
        skills.insert(skill.id, skill);
    }
    let rules = Phase3Rules::try_new(
        SkillRegistry::try_new(skills).unwrap(),
        base.pvp().clone(),
        base.regen().clone(),
        base.constants().clone(),
        base.profiles().clone(),
        base.geometry().clone(),
        base.npc().clone(),
    )
    .unwrap();
    let stat_rules = load_rules(&RulesSource::embedded()).unwrap().rules;
    let templates = parse_zone(TEST_ZONE_TOML).unwrap().npc_templates;
    let mut zone = ZoneState::new_phase3(
        ZoneSeed {
            zone: ZoneId(1),
            epoch: 1,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        stat_rules.clone(),
        load_classes(&ClassSource::embedded()).unwrap().registry,
        rules,
    )
    .unwrap();
    let applied = zone
        .run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::SpawnNpc {
            name: "Capacity keltir".into(),
            pos: Vec2Fixed::from_tiles(10, 10),
            speed: templates[0].move_speed,
            combat: Some(Box::new(NpcCombat::from_template(&stat_rules, &templates[0]).unwrap())),
        })]))
        .unwrap();
    assert!(applied.dispositions.is_empty());
    let mut snapshot = zone.snapshot();
    let model = snapshot.entities[0].clone();
    let phase = snapshot.phase3.as_mut().unwrap();
    let side = phase.entities.values().next().unwrap().clone();
    phase.entities.clear();
    snapshot.entities.clear();
    let mut next_effect = 1_u64;
    for index in 0..1000_u64 {
        let hash = Sha256::digest(index.to_le_bytes());
        let id = EntityId::from_uuid(Uuid::from_bytes(hash[..16].try_into().unwrap()));
        let mut entity = model.clone();
        entity.id = id;
        snapshot.entities.push(entity);
        let mut state = side.clone();
        state.ordinal = EntityOrdinal(index + 1);
        let mut entries = BTreeMap::new();
        for offset in 0..44_u32 {
            let skill = phase
                .rules
                .skills()
                .get(SkillId(1_000_000 + offset))
                .unwrap();
            let abnormal = skill.abnormal.as_ref().unwrap();
            let (resource, power, interval) =
                compile_periodic(&skill.effects[0], 1, skill.max_level)
                    .unwrap()
                    .unwrap();
            let effect = ActiveEffect {
                id: EffectId(next_effect),
                skill: SkillRef {
                    id: skill.id,
                    rank: 1,
                },
                source: EffectSource {
                    binding: Some(TargetBinding {
                        entity: EntityId::from_uuid(Uuid::from_bytes(
                            Sha256::digest(next_effect.to_be_bytes())[..16]
                                .try_into()
                                .unwrap(),
                        )),
                        incarnation: 1,
                    }),
                    kind: EffectSourceKind::Player,
                },
                target_incarnation: 1,
                applied_at: Tick(0),
                expires_at: Some(Tick(12_000)),
                pool: if offset < 24 {
                    EffectPool::Buff
                } else if offset < 36 {
                    EffectPool::SongDance
                } else {
                    EffectPool::Debuff
                },
                abnormal_type: "NONE".into(),
                abnormal_level: abnormal.level[0],
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
                .insert(next_effect, RestoredModifierSource::Effect(effect.id));
            entries.insert(effect.id, effect);
            next_effect += 1;
        }
        state.effects = EffectList::from_entries(entries).unwrap();
        state
            .effects
            .validate_capacity(EffectCapacities::for_enlarge(4))
            .unwrap();
        phase.entities.insert(id, state);
    }
    snapshot.entities.sort_by_key(|entity| entity.id);
    let mut aoi = AoiIndex::default();
    for entity in &snapshot.entities {
        aoi.insert(entity.id, entity.pos);
    }
    snapshot.aoi = aoi.to_cells();
    phase.next_entity_ordinal = 1001;
    phase.next_effect_id = next_effect;
    phase.next_install_sequence = next_effect;
    let contexts = snapshot
        .entities
        .iter()
        .map(|e| {
            (
                e.id,
                Phase3EntityContext {
                    next_tick: snapshot.tick,
                    incarnation: 1,
                    alive: true,
                    player: false,
                    learned: &[],
                },
            )
        })
        .collect();
    phase.validate_live(&contexts).unwrap();
    assert_eq!(phase.entities.len(), 1000);
    assert_eq!(
        phase
            .entities
            .values()
            .map(|e| e.effects.entries().len())
            .sum::<usize>(),
        44_000
    );
    assert_eq!(phase.rules.skills().skills().len(), original_skills + 44);
    let rule_bytes = serde_json::to_vec(Arc::as_ref(&phase.rules)).unwrap().len();
    snapshot.validate_version().unwrap();
    let json = serde_json::to_vec(&snapshot).unwrap();
    let mut compressor =
        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    compressor.write_all(&json).unwrap();
    let compressed = compressor.finish().unwrap().len() + 12;
    // These diagnostic labels mirror record.rs's private bounds. encode_snapshot below is
    // the authoritative gate and deliberately runs with its existing bounded allocator.
    println!("shipped_rules_bytes={shipped_rule_bytes} shipped_skill_roots={original_skills} synthetic_skill_roots=44");
    println!("full_rules_bytes={rule_bytes} entities=1000 effects=44000 legacy_json_decoded_bytes={} decoded_limit={} legacy_json_encoded_bytes={compressed} encoded_limit={}",json.len(),32*1024*1024,1024*1024-1024);
    let (binary_decoded, binary_encoded) =
        nightfall_api::application::replay_log::phase3_snapshot_size(&snapshot).unwrap();
    println!("binary_decoded_bytes={binary_decoded} binary_encoded_bytes={binary_encoded}");
    match encode_snapshot(&snapshot) {
        Ok(encoded) => {
            let decoded = u32::from_le_bytes(encoded[8..12].try_into().unwrap());
            println!("binary_decoded_bytes={decoded} decoded_limit={} binary_encoded_bytes={} encoded_limit={}", 32*1024*1024, encoded.len(), 1024*1024-1024);
            let restored = decode_snapshot(&encoded).unwrap();
            assert_eq!(serde_json::to_vec(&restored).unwrap(), json);
            let mut cold = ZoneState::from_snapshot(restored).unwrap();
            cold.run_tick(cold.draft(Vec::new())).unwrap();
            println!(
                "codec_capacity=PASS canonical_roundtrip=PASS cold_restore_and_tick=PASS shipped_full_pool_admission=NOT_CLAIMED"
            );
            true
        },
        Err(error) => {
            println!("codec_capacity=FAIL {error}; live_gameplay_gate=NOT_MEASURED");
            false
        },
    }
}

/// Source-valid workload shape for the eight-player soak: the shipped zone's fixtures and
/// spawn slots plus eight admitted players, all casting Shield. This is an actual tick and
/// restore/suffix check; it is not a packaged client or database/broker soak.
fn realistic() -> bool {
    use nightfall_api::domain::character_progression::{
        CharacterAppearance, CharacterIdentity, ClassState,
    };
    use nightfall_api::domain::class::ClassId;
    use nightfall_api::domain::subclass::LearnedSkill;
    use nightfall_api::domain::zone as z;
    use nightfall_api::domain::{AccountId, Race};
    let def = parse_zone(TEST_ZONE_TOML).unwrap();
    let stat_rules = load_rules(&RulesSource::embedded()).unwrap().rules;
    let slots = nightfall_api::application::zone_bootstrap::slot_specs(&stat_rules, &def).unwrap();
    let mut zone = ZoneState::new_phase3(
        ZoneSeed {
            zone: ZoneId(1),
            epoch: 9,
        },
        def.bounds,
        0,
        load_rules(&RulesSource::embedded()).unwrap().rules,
        load_classes(&ClassSource::embedded()).unwrap().registry,
        load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap(),
    )
    .unwrap()
    .with_spawn_slots(slots);
    let mut inputs = def
        .npcs
        .into_iter()
        .map(|npc| {
            ZoneInput::system(ZoneCommand::SpawnNpc {
                name: npc.name,
                pos: npc.pos,
                speed: npc.speed,
                combat: None,
            })
        })
        .collect::<Vec<_>>();
    let players = (1..=8_u128)
        .map(|n| EntityId::from_uuid(Uuid::from_u128(n)))
        .collect::<Vec<_>>();
    for player in &players {
        let mut ledger = ClassState::new(ClassId(10));
        ledger.learned_skills = vec![LearnedSkill {
            key: "l2.skill.1040".into(),
            level: 1,
        }];
        let mut load = z::PlayerLoad::fresh("human_mystic");
        load.progression = Some(Box::new(z::PlayerProgression {
            identity: CharacterIdentity {
                account_id: AccountId::from_uuid(player.as_uuid()),
                race: Race::Human,
                base_class_id: ClassId(10),
                appearance: CharacterAppearance::default(),
            },
            class_state: ledger,
            max_cp: 0,
        }));
        inputs.push(ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: *player,
            name: "Caster".into(),
            pos: Vec2Fixed::from_tiles(126, 126),
            speed: z::Speed::DEFAULT,
            generation: z::SessionGeneration(1),
            load: Some(Box::new(load)),
        }));
    }
    let admitted = zone.run_tick(zone.draft(inputs)).unwrap();
    assert!(admitted.dispositions.is_empty(), "{:?}", admitted.dispositions);
    let cast = zone
        .run_tick(
            zone.draft(
                players
                    .iter()
                    .map(|id| ZoneInput {
                        source: z::CommandSource::Session {
                            entity: *id,
                            generation: z::SessionGeneration(1),
                        },
                        seq: Some(1),
                        command: ZoneCommand::CastSkill {
                            entity: *id,
                            skill_id: SkillId(1040),
                            target: Some(*id),
                        },
                    })
                    .collect(),
            ),
        )
        .unwrap();
    assert!(cast.dispositions.is_empty());
    for _ in 0..40 {
        zone.run_tick(zone.draft(vec![])).unwrap();
    }
    let snapshot = zone.snapshot();
    let effects = snapshot
        .phase3
        .as_ref()
        .unwrap()
        .entities
        .values()
        .map(|e| e.effects.entries().len())
        .sum::<usize>();
    assert_eq!(effects, 8);
    let (decoded, encoded) =
        nightfall_api::application::replay_log::phase3_snapshot_size(&snapshot).unwrap();
    let bytes = encode_snapshot(&snapshot).unwrap();
    assert_eq!(encoded, bytes.len());
    let restored_snapshot = decode_snapshot(&bytes).unwrap();
    assert_eq!(restored_snapshot, snapshot);
    let mut restored = ZoneState::from_snapshot(restored_snapshot).unwrap();
    for _ in 0..35 {
        let draft = zone.draft(vec![]);
        assert_eq!(zone.run_tick(draft.clone()).unwrap(), restored.run_tick(draft).unwrap());
    }
    println!("realistic_players=8 zone_entities={} active_effects={effects} binary_decoded_bytes={decoded} binary_encoded_bytes={encoded} codec_capacity=PASS live_restore_suffix=PASS packaged_soak=NOT_RUN", snapshot.entities.len());
    true
}
