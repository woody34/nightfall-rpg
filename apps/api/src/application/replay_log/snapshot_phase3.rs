//! NFSNAP9 body v1: legacy/base JSON once, full rules once keyed by their content hash,
//! typed protobuf sidecars, packed UUIDs and first-use canonical effect-header dedupe.
//! Header identity includes every frozen handler coefficient; live pulse phase stays per
//! application. No restoration hooks, timer filtering, current-data lookup or truncation.
use std::collections::BTreeMap;
use std::sync::Arc;

use prost::Message;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::record::{CodecError, MAX_SNAPSHOT_JSON};
use super::snapshot_value::{error, pack, unpack, PbValue};
use crate::domain::zone::effects::{
    ActiveEffect, ActiveHandler, ActivePeriodic, EffectFlags, EffectId, EffectList, EffectPool,
    EffectSource, EffectSourceKind, PeriodicPower, PeriodicResource, PulseSchedule, TargetBinding,
};
use crate::domain::zone::skill::SkillRef;
use crate::domain::zone::{
    Phase3Entity, Phase3Rules, Phase3Snapshot, RestoredModifierSource as Source, Tick, ZoneSnapshot,
};

#[derive(Clone, PartialEq, Message)]
struct Snapshot {
    #[prost(uint32, tag = "1")]
    format: u32,
    #[prost(bytes = "vec", tag = "2")]
    base_json: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    rules_hash: Vec<u8>,
    #[prost(message, optional, tag = "4")]
    rules: Option<super::snapshot_registry::Registry>,
    #[prost(uint64, repeated, packed = "true", tag = "5")]
    counters: Vec<u64>,
    #[prost(message, repeated, tag = "6")]
    headers: Vec<Header>,
    #[prost(message, repeated, tag = "7")]
    entities: Vec<Entity>,
    // Fixed-width source IDs in effect traversal order. Keeping high-entropy bytes together
    // avoids paying a compressor reset/header cost for every random departed identity.
    #[prost(bytes = "vec", tag = "8")]
    source_ids: Vec<u8>,
    #[prost(message, optional, tag = "9")]
    base_entities: Option<PbValue>,
}
#[derive(Clone, PartialEq, Message)]
struct Entity {
    #[prost(uint32, tag = "1")]
    base_index: u32,
    #[prost(message, optional, tag = "2")]
    metadata: Option<PbValue>,
    #[prost(message, repeated, tag = "3")]
    effects: Vec<Effect>,
    #[prost(message, repeated, tag = "4")]
    modifiers: Vec<Modifier>,
    #[prost(uint64, tag = "5")]
    effect_base: u64,
}
#[derive(Clone, PartialEq, Message)]
struct Modifier {
    #[prost(uint64, tag = "1")]
    sequence: u64,
    #[prost(uint64, optional, tag = "2")]
    effect: Option<u64>,
    #[prost(message, optional, tag = "3")]
    passive: Option<PbSkill>,
    #[prost(bool, tag = "4")]
    death_penalty: bool,
}
#[derive(Clone, Copy, PartialEq, Message)]
struct PbSkill {
    #[prost(uint32, tag = "1")]
    id: u32,
    #[prost(uint32, tag = "2")]
    rank: u32,
}
#[derive(Clone, PartialEq, Message)]
struct Header {
    #[prost(message, optional, tag = "1")]
    skill: Option<PbSkill>,
    #[prost(uint32, tag = "2")]
    pool: u32,
    #[prost(string, tag = "3")]
    abnormal_type: String,
    #[prost(uint32, tag = "4")]
    abnormal_level: u32,
    #[prost(uint32, tag = "5")]
    flags: u32,
    #[prost(message, repeated, tag = "6")]
    handlers: Vec<Handler>,
}
#[derive(Clone, PartialEq, Message)]
struct Handler {
    #[prost(uint32, tag = "1")]
    xml_ordinal: u32,
    #[prost(message, optional, tag = "2")]
    periodic: Option<PbValue>,
}
#[derive(Clone, PartialEq, Message)]
struct Effect {
    #[prost(uint64, tag = "1")]
    id: u64,
    #[prost(uint32, tag = "2")]
    header: u32,
    #[prost(uint32, tag = "3")]
    source_kind: u32,
    #[prost(uint32, optional, tag = "5")]
    source_incarnation: Option<u32>,
    #[prost(uint32, tag = "6")]
    target_incarnation: u32,
    #[prost(uint64, tag = "7")]
    applied_at: u64,
    #[prost(uint64, optional, tag = "8")]
    expires_at: Option<u64>,
    #[prost(message, repeated, tag = "9")]
    phases: Vec<Phase>,
}
#[derive(Clone, PartialEq, Message)]
struct Phase {
    #[prost(uint64, tag = "1")]
    anchor: u64,
    #[prost(uint64, tag = "2")]
    next_ideal_offset_ms: u64,
    #[prost(uint64, tag = "3")]
    pulse_index: u64,
    #[prost(uint32, tag = "4")]
    interval_ms: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Periodic {
    resource: PeriodicResource,
    power: PeriodicPower,
}
// Rare/small fields use the lossless protobuf DTO vocabulary. Explicitly enumerate them
// so future additions cannot silently disappear from the versioned binary codec.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    ordinal: crate::domain::zone::EntityOrdinal,
    cast: crate::domain::zone::CastState,
    reuse: BTreeMap<crate::domain::zone::skill::SkillId, Tick>,
    regen: crate::domain::zone::state_regen::RegenSchedule,
    fractions: crate::domain::zone::ResourceFractions,
    player: Option<crate::domain::zone::Phase3Player>,
}

fn required<T>(value: Option<T>) -> Result<T, CodecError> {
    value.ok_or_else(|| error("missing required field"))
}
fn entity_id(bytes: &[u8]) -> Result<crate::domain::zone::EntityId, CodecError> {
    Ok(crate::domain::zone::EntityId::from_uuid(
        Uuid::from_slice(bytes).map_err(error)?,
    ))
}
fn skill(value: SkillRef) -> PbSkill {
    PbSkill {
        id: value.id.0,
        rank: value.rank,
    }
}
fn read_skill(value: PbSkill) -> SkillRef {
    SkillRef {
        id: crate::domain::zone::skill::SkillId(value.id),
        rank: value.rank,
    }
}
fn header(effect: &ActiveEffect) -> Result<Header, CodecError> {
    let flags = effect.flags;
    Ok(Header {
        skill: Some(skill(effect.skill)),
        pool: match effect.pool {
            EffectPool::Buff => 0,
            EffectPool::SongDance => 1,
            EffectPool::Debuff => 2,
            EffectPool::System => 3,
        },
        abnormal_type: effect.abnormal_type.clone(),
        abnormal_level: effect.abnormal_level,
        flags: u32::from(flags.stay_after_death)
            | (u32::from(flags.stay_on_subclass_change) << 1)
            | (u32::from(flags.removed_on_damage) << 2)
            | (u32::from(flags.removed_on_action_except_move) << 3)
            | (u32::from(flags.irreplaceable) << 4),
        handlers: effect
            .handlers
            .iter()
            .map(|h| {
                Ok(Handler {
                    xml_ordinal: h.xml_ordinal,
                    periodic: h
                        .periodic
                        .as_ref()
                        .map(|p| {
                            pack(&Periodic {
                                resource: p.resource,
                                power: p.power,
                            })
                        })
                        .transpose()?,
                })
            })
            .collect::<Result<_, CodecError>>()?,
    })
}

#[allow(clippy::too_many_lines)] // Canonical table traversal and first-use allocation stay together.
pub(super) fn encode(snapshot: &ZoneSnapshot) -> Result<Vec<u8>, CodecError> {
    let phase3 = snapshot
        .phase3
        .as_ref()
        .ok_or_else(|| error("missing phase3"))?;
    let mut base = snapshot.clone();
    base.phase3 = None;
    base.entities.clear();
    base.aoi.clear();
    let mut message = Snapshot {
        format: 1,
        base_json: super::record::bounded_snapshot_json(&base)?,
        rules_hash: phase3.rules_hash.to_vec(),
        rules: Some(super::snapshot_registry::Registry::pack(phase3.rules.as_ref())?),
        counters: vec![
            phase3.next_entity_ordinal,
            phase3.next_cast_id,
            phase3.next_effect_id,
            phase3.next_offer_id,
            phase3.next_install_sequence,
        ],
        headers: Vec::new(),
        entities: Vec::new(),
        source_ids: Vec::new(),
        base_entities: Some(pack(&(&snapshot.entities, &snapshot.aoi))?),
    };
    let mut headers = BTreeMap::new();
    let base_indices = snapshot
        .entities
        .iter()
        .enumerate()
        .map(|(index, e)| Ok((e.id, u32::try_from(index).map_err(error)?)))
        .collect::<Result<BTreeMap<_, _>, CodecError>>()?;
    for (id, state) in &phase3.entities {
        let mut effects = Vec::new();
        let effect_base = state
            .effects
            .entries()
            .first_key_value()
            .map_or(0, |(id, _)| id.0);
        let mut previous_effect = effect_base;
        for effect in state.effects.entries().values() {
            let h = header(effect)?;
            let key = h.encode_to_vec();
            let index = if let Some(index) = headers.get(&key) {
                *index
            } else {
                let index = u32::try_from(message.headers.len()).map_err(error)?;
                headers.insert(key, index);
                message.headers.push(h);
                index
            };
            if let Some(binding) = effect.source.binding {
                message
                    .source_ids
                    .extend_from_slice(binding.entity.as_uuid().as_bytes());
            }
            effects.push(Effect {
                id: effect
                    .id
                    .0
                    .checked_sub(previous_effect)
                    .ok_or_else(|| error("effect order"))?,
                header: index,
                source_kind: match effect.source.kind {
                    EffectSourceKind::Player => 0,
                    EffectSourceKind::Npc => 1,
                    EffectSourceKind::System => 2,
                },
                source_incarnation: effect.source.binding.map(|b| b.incarnation),
                target_incarnation: effect.target_incarnation,
                applied_at: effect.applied_at.0,
                expires_at: effect.expires_at.map(|t| t.0),
                phases: effect
                    .handlers
                    .iter()
                    .filter_map(|h| h.periodic.as_ref())
                    .map(|p| Phase {
                        anchor: p.schedule.anchor.0,
                        next_ideal_offset_ms: p.schedule.next_ideal_offset_ms,
                        pulse_index: p.schedule.pulse_index,
                        interval_ms: p.schedule.interval_ms,
                    })
                    .collect(),
            });
            previous_effect = effect.id.0;
        }
        let mut previous_sequence = 0;
        message.entities.push(Entity {
            base_index: *base_indices
                .get(id)
                .ok_or_else(|| error("sidecar lacks base entity"))?,
            metadata: Some(pack(&Metadata {
                ordinal: state.ordinal,
                cast: state.cast.clone(),
                reuse: state.reuse.clone(),
                regen: state.regen,
                fractions: state.fractions,
                player: state.player.clone(),
            })?),
            effects,
            effect_base,
            modifiers: state
                .modifier_install_order
                .iter()
                .map(|(sequence, source)| {
                    let mut m = Modifier {
                        sequence: sequence
                            .checked_sub(previous_sequence)
                            .ok_or_else(|| error("modifier order"))?,
                        effect: None,
                        passive: None,
                        death_penalty: false,
                    };
                    match source {
                        Source::Passive(s) => m.passive = Some(skill(*s)),
                        Source::Effect(e) => {
                            m.effect = Some(
                                e.0.checked_sub(effect_base)
                                    .ok_or_else(|| error("modifier effect before base"))?,
                            );
                        },
                        Source::DeathPenalty => m.death_penalty = true,
                    }
                    previous_sequence = *sequence;
                    Ok(m)
                })
                .collect::<Result<_, CodecError>>()?,
        });
    }
    if message.encoded_len() > MAX_SNAPSHOT_JSON {
        return Err(error("binary expansion exceeds 32 MiB"));
    }
    validate_expansion(&message)?;
    Ok(message.encode_to_vec())
}

// Dedupe must not turn a small transport message into unbounded cloned handler state.
// Charge the inline protobuf size of every referenced header before materializing it.
fn validate_expansion(message: &Snapshot) -> Result<(), CodecError> {
    let mut expanded = message.encoded_len();
    for entity in &message.entities {
        if entity.effects.len() > 44 {
            return Err(error("effect pools exceed source maximum 24+12+8"));
        }
        for effect in &entity.effects {
            let header = message
                .headers
                .get(usize::try_from(effect.header).map_err(error)?)
                .ok_or_else(|| error("effect header reference"))?;
            expanded = expanded
                .checked_add(header.encoded_len())
                .ok_or_else(|| error("header expansion overflow"))?;
            if expanded > MAX_SNAPSHOT_JSON {
                return Err(error("inline header expansion exceeds 32 MiB"));
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)] // Validate ordered references before constructing the canonical sidecar.
pub(super) fn decode(bytes: &[u8]) -> Result<ZoneSnapshot, CodecError> {
    let message = Snapshot::decode(bytes).map_err(error)?;
    // Unknown/duplicate fields, alternate integer encodings and field order are refused.
    if message.encode_to_vec() != bytes || message.format != 1 {
        return Err(error("noncanonical body or unsupported format"));
    }
    validate_expansion(&message)?;
    let mut snapshot: ZoneSnapshot = serde_json::from_slice(&message.base_json).map_err(error)?;
    if snapshot.phase3.is_some()
        || !snapshot.entities.is_empty()
        || !snapshot.aoi.is_empty()
        || snapshot.meta.schema_version != 9
        || snapshot.meta.digest_version != crate::domain::zone::StateDigestVersion::BinaryV5
    {
        return Err(error("invalid base policy or duplicated phase3"));
    }
    let rules: Phase3Rules = required(message.rules)?.unpack()?;
    (snapshot.entities, snapshot.aoi) = unpack(required(message.base_entities)?)?;
    let rules_hash: [u8; 32] = message
        .rules_hash
        .try_into()
        .map_err(|_| error("rules hash length"))?;
    if rules.hash() != rules_hash {
        return Err(error("rules hash mismatch"));
    }
    let [next_entity_ordinal, next_cast_id, next_effect_id, next_offer_id, next_install_sequence]: [u64; 5] =
        message.counters.try_into().map_err(|_| error("allocation counter cardinality"))?;
    let mut entities = BTreeMap::new();
    let (source_ids, remainder) = message.source_ids.as_chunks::<16>();
    let mut source_ids = source_ids.iter();
    if !remainder.is_empty() {
        return Err(error("source UUID slab length"));
    }
    for entity in message.entities {
        let id = snapshot
            .entities
            .get(usize::try_from(entity.base_index).map_err(error)?)
            .ok_or_else(|| error("base entity reference"))?
            .id;
        if entities
            .last_key_value()
            .is_some_and(|(previous, _)| *previous >= id)
        {
            return Err(error("entity order/duplicate"));
        }
        let metadata: Metadata = unpack(required(entity.metadata)?)?;
        let mut effects = BTreeMap::new();
        let mut previous_effect = entity.effect_base;
        for mut effect in entity.effects {
            effect.id = effect
                .id
                .checked_add(previous_effect)
                .ok_or_else(|| error("effect delta overflow"))?;
            previous_effect = effect.id;
            if effects
                .last_key_value()
                .is_some_and(|(previous, _)| *previous >= EffectId(effect.id))
            {
                return Err(error("effect order/duplicate"));
            }
            let h = message
                .headers
                .get(usize::try_from(effect.header).map_err(error)?)
                .ok_or_else(|| error("effect header reference"))?;
            let source_id = if effect.source_incarnation.is_some() {
                Some(entity_id(
                    source_ids
                        .next()
                        .ok_or_else(|| error("missing source UUID"))?,
                )?)
            } else {
                None
            };
            let decoded = read_effect(effect, h, source_id)?;
            effects.insert(decoded.id, decoded);
        }
        let mut modifiers = BTreeMap::new();
        let mut previous_sequence = 0;
        for mut m in entity.modifiers {
            m.sequence = m
                .sequence
                .checked_add(previous_sequence)
                .ok_or_else(|| error("modifier delta overflow"))?;
            previous_sequence = m.sequence;
            if modifiers
                .last_key_value()
                .is_some_and(|(previous, _)| *previous >= m.sequence)
            {
                return Err(error("modifier order/duplicate"));
            }
            let source = match (m.effect, m.passive, m.death_penalty) {
                (Some(e), None, false) => Source::Effect(EffectId(
                    e.checked_add(entity.effect_base)
                        .ok_or_else(|| error("modifier effect overflow"))?,
                )),
                (None, Some(s), false) => Source::Passive(read_skill(s)),
                (None, None, true) => Source::DeathPenalty,
                _ => return Err(error("invalid modifier union")),
            };
            modifiers.insert(m.sequence, source);
        }
        entities.insert(
            id,
            Phase3Entity {
                ordinal: metadata.ordinal,
                cast: metadata.cast,
                reuse: metadata.reuse,
                regen: metadata.regen,
                fractions: metadata.fractions,
                player: metadata.player,
                effects: EffectList::from_entries(effects).map_err(error)?,
                modifier_install_order: modifiers,
            },
        );
    }
    if source_ids.next().is_some() {
        return Err(error("unused source UUID"));
    }
    snapshot.phase3 = Some(Phase3Snapshot {
        rules: Arc::new(rules),
        rules_hash,
        next_entity_ordinal,
        next_cast_id,
        next_effect_id,
        next_offer_id,
        next_install_sequence,
        entities,
    });
    // Also enforces first-use header ordering, dedupe, no unused headers and DTO key forms.
    if encode(&snapshot)? != bytes {
        return Err(error("noncanonical snapshot tables"));
    }
    Ok(snapshot)
}
fn read_effect(
    effect: Effect,
    h: &Header,
    source_id: Option<crate::domain::zone::EntityId>,
) -> Result<ActiveEffect, CodecError> {
    let binding = match (source_id, effect.source_incarnation) {
        (None, None) => None,
        (Some(id), Some(incarnation)) => Some(TargetBinding {
            entity: id,
            incarnation,
        }),
        _ => return Err(error("partial source binding")),
    };
    if h.flags > 31 {
        return Err(error("unknown effect flags"));
    }
    let mut phases = effect.phases.into_iter();
    let handlers = h
        .handlers
        .iter()
        .map(|h| {
            Ok(ActiveHandler {
                xml_ordinal: h.xml_ordinal,
                periodic: if let Some(periodic) = &h.periodic {
                    let p: Periodic = unpack(periodic.clone())?;
                    let phase = phases
                        .next()
                        .ok_or_else(|| error("missing periodic phase"))?;
                    Some(ActivePeriodic {
                        resource: p.resource,
                        power: p.power,
                        schedule: PulseSchedule {
                            anchor: Tick(phase.anchor),
                            next_ideal_offset_ms: phase.next_ideal_offset_ms,
                            pulse_index: phase.pulse_index,
                            interval_ms: phase.interval_ms,
                        },
                    })
                } else {
                    None
                },
            })
        })
        .collect::<Result<_, CodecError>>()?;
    if phases.next().is_some() {
        return Err(error("extra periodic phase"));
    }
    Ok(ActiveEffect {
        id: EffectId(effect.id),
        skill: read_skill(required(h.skill)?),
        source: EffectSource {
            binding,
            kind: match effect.source_kind {
                0 => EffectSourceKind::Player,
                1 => EffectSourceKind::Npc,
                2 => EffectSourceKind::System,
                _ => return Err(error("invalid source kind")),
            },
        },
        target_incarnation: effect.target_incarnation,
        applied_at: Tick(effect.applied_at),
        expires_at: effect.expires_at.map(Tick),
        pool: match h.pool {
            0 => EffectPool::Buff,
            1 => EffectPool::SongDance,
            2 => EffectPool::Debuff,
            3 => EffectPool::System,
            _ => return Err(error("invalid pool")),
        },
        abnormal_type: h.abnormal_type.clone(),
        abnormal_level: h.abnormal_level,
        flags: EffectFlags {
            stay_after_death: h.flags & 1 != 0,
            stay_on_subclass_change: h.flags & 2 != 0,
            removed_on_damage: h.flags & 4 != 0,
            removed_on_action_except_move: h.flags & 8 != 0,
            irreplaceable: h.flags & 16 != 0,
        },
        handlers,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::domain::zone::*;
    use crate::infrastructure::{class_data, rules_data, skill_data};

    fn snapshot() -> ZoneSnapshot {
        let mut snapshot = ZoneState::new_phase3(
            ZoneSeed {
                zone: ZoneId(1),
                epoch: 1,
            },
            ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
            0,
            rules_data::load_rules(&rules_data::RulesSource::embedded())
                .unwrap()
                .rules,
            class_data::load_classes(&class_data::ClassSource::embedded())
                .unwrap()
                .registry,
            skill_data::load_phase3_rules(&skill_data::Phase3DataSource::embedded(), &[]).unwrap(),
        )
        .unwrap()
        .snapshot();
        snapshot.entities = (1..=2)
            .map(|index| crate::domain::zone::Entity {
                id: EntityId::from_uuid(Uuid::from_u128(index)),
                kind: EntityKind::Npc,
                name: "Codec fixture".into(),
                pos: Vec2Fixed::from_tiles(10, 10),
                dest: None,
                speed: Speed::DEFAULT,
                generation: SessionGeneration(0),
                targeting: TargetingState::default(),
                combat: None,
                ai: None,
            })
            .collect();
        let phase3 = snapshot.phase3.as_mut().unwrap();
        // Representation fixture: two entities share one complete effect header but keep
        // different identities, source bindings, target lives, absolute times and phases.
        for index in 1..=2 {
            let id = EntityId::from_uuid(Uuid::from_u128(index));
            let effect = ActiveEffect {
                id: EffectId(u64::try_from(index).unwrap()),
                skill: SkillRef {
                    id: skill::SkillId(1040),
                    rank: 1,
                },
                source: EffectSource {
                    kind: EffectSourceKind::Player,
                    binding: Some(TargetBinding {
                        entity: id,
                        incarnation: 3,
                    }),
                },
                target_incarnation: 2,
                applied_at: Tick(4),
                expires_at: Some(Tick(1000)),
                pool: EffectPool::Buff,
                abnormal_type: "pDef".into(),
                abnormal_level: 1,
                flags: EffectFlags {
                    stay_after_death: true,
                    stay_on_subclass_change: false,
                    removed_on_damage: true,
                    removed_on_action_except_move: false,
                    irreplaceable: true,
                },
                handlers: vec![ActiveHandler {
                    xml_ordinal: 3,
                    periodic: Some(ActivePeriodic {
                        resource: PeriodicResource::Hp,
                        power: PeriodicPower::Per {
                            power: effects::FrozenRatio::new(
                                skill_math::ExactRatio::new(-1, 3).unwrap(),
                            ),
                            multiplier: effects::FrozenRatio::new(
                                skill_math::ExactRatio::new(333, 500).unwrap(),
                            ),
                        },
                        schedule: PulseSchedule {
                            anchor: Tick(4),
                            next_ideal_offset_ms: 1332,
                            pulse_index: u64::try_from(index).unwrap(),
                            interval_ms: 666,
                        },
                    }),
                }],
            };
            phase3.entities.insert(
                id,
                Phase3Entity {
                    ordinal: EntityOrdinal(u64::try_from(index).unwrap()),
                    cast: CastState::Idle,
                    reuse: BTreeMap::from([(skill::SkillId(1040), Tick(u64::MAX))]),
                    modifier_install_order: BTreeMap::from([(
                        u64::try_from(index).unwrap(),
                        RestoredModifierSource::Effect(effect.id),
                    )]),
                    effects: EffectList::from_entries(BTreeMap::from([(effect.id, effect)]))
                        .unwrap(),
                    regen: state_regen::RegenSchedule {
                        next_at: Some(Tick(30)),
                    },
                    fractions: ResourceFractions {
                        hp: 1,
                        mp: 999_999,
                        cp: 0,
                    },
                    player: None,
                },
            );
        }
        phase3.next_entity_ordinal = 3;
        phase3.next_effect_id = 3;
        phase3.next_install_sequence = 3;
        snapshot
    }
    #[test]
    fn compact_headers_preserve_every_canonical_field_and_share_rules() {
        let snapshot = snapshot();
        let bytes = encode(&snapshot).unwrap();
        let pb = Snapshot::decode(bytes.as_slice()).unwrap();
        assert_eq!(pb.headers.len(), 1);
        assert_eq!(pb.entities.len(), 2);
        assert_eq!(decode(&bytes).unwrap(), snapshot);
        assert_eq!(encode(&decode(&bytes).unwrap()).unwrap(), bytes);
        let transport = crate::application::replay_log::encode_snapshot(&snapshot).unwrap();
        assert_eq!(crate::application::replay_log::decode_snapshot(&transport).unwrap(), snapshot);
    }
    #[test]
    fn malformed_or_ambiguous_binary_tables_are_refused() {
        let original = Snapshot::decode(encode(&snapshot()).unwrap().as_slice()).unwrap();
        for mutation in 0..12 {
            let mut bad = original.clone();
            match mutation {
                0 => bad.rules_hash[0] ^= 1,
                1 => bad.headers.push(bad.headers[0].clone()),
                2 => bad.entities[0].effects[0].header = 99,
                3 => {
                    bad.source_ids.pop();
                },
                4 => bad.entities[0].effects[0].source_incarnation = None,
                5 => bad.entities[0].effects[0].phases.clear(),
                6 => bad.entities[0].effects[0].phases.push(Phase::default()),
                7 => bad.headers[0].flags |= 32,
                8 => bad.entities.swap(0, 1),
                9 => bad.entities[0].modifiers[0].death_penalty = true,
                10 => {
                    let duplicate = bad.entities[0].effects[0].clone();
                    bad.entities[0].effects.push(duplicate);
                },
                _ => bad.counters.pop().map(|_| ()).unwrap(),
            }
            assert!(decode(&bad.encode_to_vec()).is_err(), "mutation {mutation}");
        }
        let mut unknown = original.encode_to_vec();
        unknown.extend_from_slice(&[0x78, 1]);
        assert!(decode(&unknown).is_err());
    }
}
